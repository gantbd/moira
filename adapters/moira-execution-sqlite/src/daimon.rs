//! Durable fake-provider receiver. No model, tool, process or network invocation.
//! Aegis gates the profile and every tool with current Execute authority.
use crate::chronos::ChronosReceiver;
pub use crate::daimon_codec::{CodecError, FakeReply};
use crate::daimon_codec::{decode_intent, decode_result, encode_intent, encode_result};
use daimon_harness::{
    AuthorityEvidenceV1, Engine, ExecutionDispositionV1, HarnessRequestV1, HarnessResultV1, Limits,
    LogicalTime, ProviderReceiptV1, RefusalV1,
};
use moira_core::{ChronosTaskRefV1, DaimonDispositionV1, DaimonResultReceiptV1, EffectId};
use moira_sqlite::{
    OwnedResource, StoreError, Timing, Usage, count, record_timing, validate_sqlite_integrity,
};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, TransactionBehavior, limits::Limit, params,
};
use std::{
    path::Path,
    time::{Duration, Instant, SystemTime},
};
const APPLICATION_ID: i64 = 0x4d444149;
const SCHEMA:&str="CREATE TABLE fake_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), entries INTEGER NOT NULL CHECK(entries>=0));
CREATE TABLE fake_executions (effect BLOB PRIMARY KEY CHECK(length(effect)=16), execution BLOB NOT NULL UNIQUE CHECK(length(execution)=16), replay_key BLOB NOT NULL UNIQUE CHECK(length(replay_key)=16), intent BLOB NOT NULL, phase INTEGER NOT NULL CHECK(phase IN(1,2,3)), result BLOB, CHECK ((phase=2 AND result IS NOT NULL) OR (phase!=2 AND result IS NULL)));";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FakeReceiverLimits {
    pub harness: Limits,
    pub max_entries: usize,
    pub max_record_bytes: usize,
    pub max_database_pages: u32,
    pub timing: Timing,
}
#[derive(Debug)]
pub enum FakeReceiverError {
    Storage(StoreError),
    Codec(CodecError),
    InvalidConfig,
    InvalidState,
    IntentConflict,
    Capacity,
    AttemptRefused,
    AuthorityRefused,
    HarnessRefused(RefusalV1),
}
impl std::fmt::Display for FakeReceiverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fake Daimon receiver: {self:?}")
    }
}
impl std::error::Error for FakeReceiverError {}
impl From<StoreError> for FakeReceiverError {
    fn from(e: StoreError) -> Self {
        Self::Storage(e)
    }
}
impl From<rusqlite::Error> for FakeReceiverError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}
impl From<CodecError> for FakeReceiverError {
    fn from(e: CodecError) -> Self {
        Self::Codec(e)
    }
}

pub struct ExecutionAttempt<'a> {
    pub chronos: &'a mut ChronosReceiver,
    pub submission: EffectId,
    pub now: SystemTime,
}
/// Host binds authenticated subject/session and supplies a live Aegis owner.
/// This borrowed authority is never checkpointed or revived by result replay.
pub struct ExecutionAuthority<'a> {
    pub engine: &'a mut aegis_authority::Engine,
    pub grant: &'a aegis_authority::GrantV1,
    pub subject: aegis_authority::PrincipalId,
    pub session: Option<aegis_authority::SessionId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionEvidence {
    ProviderResult(HarnessResultV1),
    /// Dispatch was durably offered, but no result was committed. This is not
    /// a fabricated provider receipt or evidence of completed external work.
    RecoveryUnknown,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Execution {
    pub receipt: DaimonResultReceiptV1,
    pub evidence: ExecutionEvidence,
    pub replayed: bool,
    pub settlement: SettlementEvidence,
}
/// Receipt fact minted only by this durable receiver; not authority to mutate
/// Chronos. Fields stay sealed outside this adapter crate.
#[derive(Clone, Eq, PartialEq)]
pub struct SettlementEvidence {
    pub(crate) source_owner: [u8; 16],
    pub(crate) effect: EffectId,
    pub(crate) execution: [u8; 16],
    pub(crate) attempt: daimon_harness::TaskAttemptRefV1,
    pub(crate) disposition: DaimonDispositionV1,
    pub(crate) recovery_unknown: bool,
}
impl std::fmt::Debug for SettlementEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SettlementEvidence(<redacted>)")
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutionUsage {
    pub offers: u64,
    pub new_results: u64,
    pub replays: u64,
    pub unknown_replays: u64,
    pub refused: u64,
    pub authority_refused: u64,
    pub attempt_refused: u64,
    pub conflicts: u64,
    pub provider_calls: u64,
    pub markers: u64,
    pub indeterminate: u64,
    pub overflowed: bool,
    /// Actual SQL transaction populations, not execution counts or audit truth.
    pub transactions: Usage,
}
struct Stored {
    intent: Vec<u8>,
    phase: i64,
    result: Option<Vec<u8>>,
}
pub struct FakeDaimonReceiver {
    connection: Connection,
    resource: OwnedResource,
    limits: FakeReceiverLimits,
    entries: usize,
    healthy: bool,
    usage: ExecutionUsage,
    recovered_unknown: usize,
    owner: [u8; 16],
    #[cfg(test)]
    fault: Option<Fault>,
}
impl std::fmt::Debug for FakeDaimonReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FakeDaimonReceiver(<redacted>)")
    }
}
impl FakeDaimonReceiver {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        limits: FakeReceiverLimits,
    ) -> Result<Self, FakeReceiverError> {
        if limits.max_entries == 0
            || limits.max_entries > i64::MAX as usize
            || limits.max_record_bytes == 0
        {
            return Err(FakeReceiverError::InvalidConfig);
        }
        Engine::try_new(limits.harness).map_err(|_| FakeReceiverError::InvalidConfig)?;
        let record_limit: i32 = limits
            .max_record_bytes
            .checked_mul(2)
            .and_then(|n| n.checked_add(2048))
            .and_then(|n| n.try_into().ok())
            .ok_or(FakeReceiverError::InvalidConfig)?;
        let resource = OwnedResource::open(path, limits.max_database_pages)?;
        let mut connection = Connection::open_with_flags(
            resource.path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::ZERO)?;
        connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, record_limit)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON;")?;
        validate_sqlite_integrity(&connection)?;
        let app: i64 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let names = {
            let mut q=connection.prepare("SELECT name FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 3")?;
            q.query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let fresh = app == 0 && version == 0 && names.is_empty();
        if !fresh
            && (app != APPLICATION_ID || version != 1 || names != ["fake_executions", "fake_meta"])
        {
            return Err(StoreError::ForeignDatabase.into());
        }
        let entries = if fresh {
            0
        } else {
            validate_store(&connection, owner, limits)?
        };
        let journal: String =
            connection.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
        connection.execute_batch(
            "PRAGMA synchronous=EXTRA; PRAGMA fullfsync=ON; PRAGMA temp_store=MEMORY;",
        )?;
        connection.pragma_update(None, "max_page_count", limits.max_database_pages)?;
        let page: i64 = connection.pragma_query_value(None, "page_size", |r| r.get(0))?;
        let pages: u32 = connection.pragma_query_value(None, "max_page_count", |r| r.get(0))?;
        let sync: i64 = connection.pragma_query_value(None, "synchronous", |r| r.get(0))?;
        let full: i64 = connection.pragma_query_value(None, "fullfsync", |r| r.get(0))?;
        if journal != "delete"
            || page != 4096
            || pages > limits.max_database_pages
            || sync != 3
            || full != 1
        {
            return Err(FakeReceiverError::InvalidConfig);
        }
        if fresh {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA)?;
            tx.execute("INSERT INTO fake_meta VALUES (1,?1,0)", [owner.as_slice()])?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
            resource.sync_parent()?;
        }
        let recovered_unknown: i64 = connection.query_row(
            "SELECT count(*) FROM fake_executions WHERE phase=1",
            [],
            |r| r.get(0),
        )?;
        let recovered_unknown: usize = recovered_unknown
            .try_into()
            .map_err(|_| FakeReceiverError::InvalidState)?;
        if recovered_unknown > 0 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute("UPDATE fake_executions SET phase=3 WHERE phase=1", [])?;
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
        }
        resource.check()?;
        Ok(Self {
            connection,
            resource,
            limits,
            entries,
            healthy: true,
            usage: ExecutionUsage::default(),
            recovered_unknown,
            owner,
            #[cfg(test)]
            fault: None,
        })
    }
    pub fn needs_reopen(&self) -> bool {
        !self.healthy
    }
    pub fn usage(&self) -> ExecutionUsage {
        self.usage
    }
    pub fn recovered_unknown(&self) -> usize {
        self.recovered_unknown
    }
    fn guard(&mut self) -> Result<(), FakeReceiverError> {
        if !self.healthy {
            return Err(StoreError::NeedsReopen.into());
        }
        self.resource
            .check()
            .inspect_err(|_| self.healthy = false)?;
        Ok(())
    }
    /// Fresh Aegis Execute checks cover the profile and every requested tool,
    /// including for retained result delivery. Hosts still own authentication.
    /// FakeReply is part of this deterministic fake profile's canonical input;
    /// production provider protocols must not reuse that simulation contract.
    pub fn execute(
        &mut self,
        effect: EffectId,
        request: HarnessRequestV1,
        reply: FakeReply,
        authority: ExecutionAuthority<'_>,
        time: LogicalTime,
        attempt: ExecutionAttempt<'_>,
    ) -> Result<Execution, FakeReceiverError> {
        let result = self.execute_inner(effect, request, reply, authority, time, attempt);
        let u = &mut self.usage;
        count(&mut u.offers, &mut u.overflowed);
        match &result {
            Ok(result) if result.replayed => {
                count(&mut u.replays, &mut u.overflowed);
                if matches!(result.evidence, ExecutionEvidence::RecoveryUnknown) {
                    count(&mut u.unknown_replays, &mut u.overflowed)
                }
            }
            Ok(_) => count(&mut u.new_results, &mut u.overflowed),
            Err(FakeReceiverError::Storage(StoreError::Indeterminate)) => {
                count(&mut u.indeterminate, &mut u.overflowed)
            }
            Err(e) => {
                count(&mut u.refused, &mut u.overflowed);
                if matches!(e, FakeReceiverError::AuthorityRefused) {
                    count(&mut u.authority_refused, &mut u.overflowed)
                }
                if matches!(e, FakeReceiverError::AttemptRefused) {
                    count(&mut u.attempt_refused, &mut u.overflowed)
                }
                if matches!(e, FakeReceiverError::IntentConflict) {
                    count(&mut u.conflicts, &mut u.overflowed)
                }
            }
        }
        result
    }
    fn execute_inner(
        &mut self,
        effect: EffectId,
        request: HarnessRequestV1,
        reply: FakeReply,
        authority: ExecutionAuthority<'_>,
        time: LogicalTime,
        attempt: ExecutionAttempt<'_>,
    ) -> Result<Execution, FakeReceiverError> {
        self.guard()?;
        let bytes = encode_intent(
            &request,
            &reply,
            self.limits.max_record_bytes,
            self.limits.harness,
        )?;
        // A fresh volatile preparation verifies current host evidence even for
        // retained-result delivery. It dispatches nothing and revives no grant.
        let mut engine =
            Engine::try_new(self.limits.harness).map_err(|_| FakeReceiverError::InvalidConfig)?;
        let authority = current_authority(&request, authority, time)?;
        let prepared = engine
            .prepare(request.clone(), &authority, time)
            .map_err(FakeReceiverError::HarnessRefused)?;
        let retained: Option<Stored> = self
            .connection
            .query_row(
                "SELECT intent,phase,result FROM fake_executions WHERE effect=?1",
                [effect.into_bytes().as_slice()],
                |r| {
                    Ok(Stored {
                        intent: r.get(0)?,
                        phase: r.get(1)?,
                        result: r.get(2)?,
                    })
                },
            )
            .optional()
            .inspect_err(|_| self.healthy = false)?;
        if let Some(retained) = retained {
            if retained.intent != bytes {
                return Err(FakeReceiverError::IntentConflict);
            }
            let evidence = match retained.phase {
                2 => ExecutionEvidence::ProviderResult(decode_result(
                    retained
                        .result
                        .as_deref()
                        .ok_or(FakeReceiverError::InvalidState)?,
                    self.limits.max_record_bytes,
                    self.limits.harness,
                )?),
                3 if retained.result.is_none() => ExecutionEvidence::RecoveryUnknown,
                _ => {
                    self.healthy = false;
                    return Err(StoreError::NeedsReopen.into());
                }
            };
            return Ok(execution(self.owner, effect, &request, evidence, true));
        }
        let aliased: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM fake_executions WHERE execution=?1 OR replay_key=?2)",
            params![
                request.execution_id.into_bytes().as_slice(),
                request.idempotency_key.into_bytes().as_slice()
            ],
            |r| r.get(0),
        )?;
        if aliased {
            return Err(FakeReceiverError::IntentConflict);
        }
        if self.entries >= self.limits.max_entries {
            return Err(FakeReceiverError::Capacity);
        }
        attempt
            .chronos
            .verify_execution(attempt.submission, &request.task_attempt, attempt.now)
            .map_err(|_| FakeReceiverError::AttemptRefused)?;
        self.healthy = false;
        let started = start(self.limits.timing);
        let marker = (|| -> Result<(), FakeReceiverError> {
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let actual: i64 =
                tx.query_row("SELECT entries FROM fake_meta WHERE slot=1", [], |r| {
                    r.get(0)
                })?;
            if actual < 0 || actual as usize != self.entries {
                return Err(FakeReceiverError::InvalidState);
            }
            tx.execute(
                "INSERT INTO fake_executions VALUES (?1,?2,?3,?4,1,NULL)",
                params![
                    effect.into_bytes().as_slice(),
                    request.execution_id.into_bytes().as_slice(),
                    request.idempotency_key.into_bytes().as_slice(),
                    bytes
                ],
            )?;
            tx.execute("UPDATE fake_meta SET entries=entries+1 WHERE slot=1", [])?;
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
            self.resource
                .check()
                .map_err(|_| StoreError::Indeterminate)?;
            Ok(())
        })();
        self.observe_transaction(&marker, started);
        marker?;
        self.entries += 1;
        count(&mut self.usage.markers, &mut self.usage.overflowed);
        #[cfg(test)]
        if self.fault == Some(Fault::AfterMarker) {
            self.fault = None;
            return Err(StoreError::Indeterminate.into());
        }
        count(&mut self.usage.provider_calls, &mut self.usage.overflowed);
        let result = engine
            .dispatch(
                prepared,
                ProviderReceiptV1::new(
                    request.execution_id,
                    reply.disposition,
                    reply.output_bytes,
                    reply.artifacts,
                ),
            )
            .map_err(FakeReceiverError::HarnessRefused)?;
        let encoded = encode_result(&result, self.limits.max_record_bytes, self.limits.harness)?;
        let started = start(self.limits.timing);
        let commit = (|| -> Result<(), FakeReceiverError> {
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let updated = tx.execute(
                "UPDATE fake_executions SET phase=2,result=?1 WHERE effect=?2 AND phase=1",
                params![encoded, effect.into_bytes().as_slice()],
            )?;
            if updated != 1 {
                return Err(FakeReceiverError::InvalidState);
            }
            #[cfg(test)]
            if self.fault == Some(Fault::BeforeResult) {
                self.fault = None;
                tx.rollback()?;
                return Err(StoreError::Io(std::io::ErrorKind::Interrupted).into());
            }
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
            #[cfg(test)]
            if self.fault == Some(Fault::AfterResult) {
                self.fault = None;
                return Err(StoreError::Indeterminate.into());
            }
            self.resource
                .check()
                .map_err(|_| StoreError::Indeterminate)?;
            Ok(())
        })();
        self.observe_transaction(&commit, started);
        commit?;
        self.healthy = true;
        Ok(execution(
            self.owner,
            effect,
            &request,
            ExecutionEvidence::ProviderResult(result),
            false,
        ))
    }
    fn observe_transaction(
        &mut self,
        result: &Result<(), FakeReceiverError>,
        started: Option<Instant>,
    ) {
        let u = &mut self.usage.transactions;
        count(&mut u.command_offers, &mut u.overflowed);
        record_timing(u, started);
        match result {
            Ok(()) => count(&mut u.committed, &mut u.overflowed),
            Err(FakeReceiverError::Storage(StoreError::Indeterminate)) => {
                count(&mut u.indeterminate, &mut u.overflowed)
            }
            Err(_) => count(&mut u.refused, &mut u.overflowed),
        }
    }
}
fn current_authority(
    request: &HarnessRequestV1,
    authority: ExecutionAuthority<'_>,
    time: LogicalTime,
) -> Result<AuthorityEvidenceV1, FakeReceiverError> {
    use aegis_authority::{
        ActionV1, AuthorizationRequestV1, EngineNamespaceV1, LogicalTime as AegisTime, PrincipalId,
        ResourceId,
    };
    if authority.subject != PrincipalId::new(request.principal.into_bytes()) {
        return Err(FakeReceiverError::AuthorityRefused);
    }
    let check = |resource| {
        AuthorizationRequestV1::new(
            authority.subject,
            authority.session,
            EngineNamespaceV1::Daimon,
            ResourceId::new(resource),
            ActionV1::Execute,
        )
    };
    let profile = authority
        .engine
        .validate(
            authority.grant,
            &check(request.profile.into_bytes()),
            AegisTime(time.0),
        )
        .map_err(|_| FakeReceiverError::AuthorityRefused)?;
    for tool in &request.tools {
        authority
            .engine
            .validate(
                authority.grant,
                &check(tool.into_bytes()),
                AegisTime(time.0),
            )
            .map_err(|_| FakeReceiverError::AuthorityRefused)?;
    }
    Ok(AuthorityEvidenceV1::new(
        profile.decision_id_bytes(),
        request.principal,
        request.tools.iter().copied(),
        LogicalTime(profile.expires_at().0),
    ))
}
fn start(timing: Timing) -> Option<Instant> {
    match timing {
        Timing::Off => None,
        Timing::Process => std::panic::catch_unwind(Instant::now).ok(),
    }
}
fn execution(
    owner: [u8; 16],
    effect: EffectId,
    request: &HarnessRequestV1,
    evidence: ExecutionEvidence,
    replayed: bool,
) -> Execution {
    let disposition = match &evidence {
        ExecutionEvidence::RecoveryUnknown => DaimonDispositionV1::Uncertain,
        ExecutionEvidence::ProviderResult(result) => match result.disposition {
            ExecutionDispositionV1::Completed => DaimonDispositionV1::Completed,
            ExecutionDispositionV1::Failed => DaimonDispositionV1::Failed,
            ExecutionDispositionV1::Cancelled => DaimonDispositionV1::Cancelled,
            ExecutionDispositionV1::TimedOut => DaimonDispositionV1::TimedOut,
            ExecutionDispositionV1::Uncertain => DaimonDispositionV1::Uncertain,
        },
    };
    Execution {
        settlement: SettlementEvidence {
            source_owner: owner,
            effect,
            execution: request.execution_id.into_bytes(),
            attempt: request.task_attempt,
            disposition,
            recovery_unknown: matches!(evidence, ExecutionEvidence::RecoveryUnknown),
        },
        receipt: DaimonResultReceiptV1::new(
            effect,
            request.execution_id.into_bytes(),
            ChronosTaskRefV1::new(
                request.task_attempt.task_key(),
                request.task_attempt.task_incarnation(),
            ),
            request.task_attempt.attempt_number(),
            disposition,
        ),
        evidence,
        replayed,
    }
}
fn validate_store(
    c: &Connection,
    owner: [u8; 16],
    limits: FakeReceiverLimits,
) -> Result<usize, FakeReceiverError> {
    let (retained, entries): (Vec<u8>, i64) = c.query_row(
        "SELECT owner,entries FROM fake_meta WHERE slot=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if retained != owner {
        return Err(StoreError::OwnerMismatch.into());
    }
    if entries < 0 || entries as usize > limits.max_entries {
        return Err(FakeReceiverError::Capacity);
    }
    let mut q=c.prepare("SELECT effect,execution,replay_key,intent,phase,result FROM fake_executions ORDER BY effect")?;
    let mut rows = q.query([])?;
    let mut count = 0usize;
    while let Some(row) = rows.next()? {
        count = count
            .checked_add(1)
            .filter(|n| *n <= limits.max_entries)
            .ok_or(FakeReceiverError::Capacity)?;
        let blob = |i| {
            row.get_ref(i)?
                .as_blob()
                .map_err(|_| rusqlite::Error::InvalidQuery)
        };
        if blob(0)?.len() != 16 {
            return Err(FakeReceiverError::InvalidState);
        }
        let intent = decode_intent(blob(3)?, limits.max_record_bytes, limits.harness)?;
        if blob(1)? != intent.request.execution_id.into_bytes()
            || blob(2)? != intent.request.idempotency_key.into_bytes()
        {
            return Err(FakeReceiverError::InvalidState);
        }
        let phase: i64 = row.get(4)?;
        match (phase, row.get_ref(5)?) {
            (1 | 3, rusqlite::types::ValueRef::Null) => {}
            (2, rusqlite::types::ValueRef::Blob(bytes)) => {
                let result = decode_result(bytes, limits.max_record_bytes, limits.harness)?;
                if result.execution_id != intent.request.execution_id
                    || result.disposition != intent.reply.disposition
                    || result.output_bytes != intent.reply.output_bytes
                    || result.produced_artifacts != intent.reply.artifacts
                {
                    return Err(FakeReceiverError::InvalidState);
                }
            }
            _ => return Err(FakeReceiverError::InvalidState),
        }
    }
    if count != entries as usize {
        return Err(FakeReceiverError::InvalidState);
    }
    Ok(count)
}
#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum Fault {
    AfterMarker,
    BeforeResult,
    AfterResult,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chronos_engine::{Agent, EngineLimits, StateRevision, TaskSpecV2};
    use daimon_harness::{
        ExecutionBudgetV1, ExecutionId, HarnessProfileId, IdempotencyKey, PrincipalId,
        TaskAttemptRefV1, ToolCapabilityId,
    };
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "moira-fake-fault-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for name in [
                "fake.db",
                "fake.db-journal",
                "fake.db.owner.lock",
                "chronos.db",
                "chronos.db-journal",
                "chronos.db.owner.lock",
            ] {
                let _ = fs::remove_file(self.0.join(name));
            }
            let _ = fs::remove_dir(&self.0);
        }
    }
    fn config() -> FakeReceiverLimits {
        FakeReceiverLimits {
            harness: Limits::default(),
            max_entries: 2,
            max_record_bytes: 4096,
            max_database_pages: 64,
            timing: Timing::Process,
        }
    }
    fn setup(path: &Path) -> (ChronosReceiver, FakeDaimonReceiver) {
        let mut c = ChronosReceiver::open(
            path.join("chronos.db"),
            [7; 16],
            crate::chronos::ReceiverLimits {
                engine: EngineLimits {
                    max_tasks: 2,
                    ..EngineLimits::default()
                },
                max_receipts: 2,
                max_snapshot_bytes: 16384,
                max_intent_bytes: 2048,
                max_database_pages: 64,
                timing: Timing::Off,
            },
        )
        .unwrap();
        let submission = EffectId::new([1; 16]);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        c.admit(submission, StateRevision::ZERO, TaskSpecV2::new("work"))
            .unwrap();
        let grant = c
            .authorize_worker(Agent::new("worker"), now, Duration::from_secs(60), 1)
            .unwrap();
        c.claim(
            submission,
            c.revision(),
            &grant,
            now,
            Duration::from_secs(30),
        )
        .unwrap();
        let r = FakeDaimonReceiver::open(path.join("fake.db"), [7; 16], config()).unwrap();
        (c, r)
    }
    fn execute(
        c: &mut ChronosReceiver,
        r: &mut FakeDaimonReceiver,
    ) -> Result<Execution, FakeReceiverError> {
        use aegis_authority::{
            ActionSetV1, ActionV1, AuthorityEpoch, Engine, EngineId, EngineNamespaceV1,
            GrantDescriptorV1, Limits, LogicalTime as AegisTime, PrincipalId as AegisPrincipal,
            ResourceId, ResourceScopeV1,
        };
        let id = EngineId::new([8; 16]);
        let mut aegis = Engine::try_new(id, Limits::default()).unwrap();
        let subject = AegisPrincipal::new([3; 16]);
        let scopes = [4, 6]
            .into_iter()
            .map(|n| {
                ResourceScopeV1::new(
                    EngineNamespaceV1::Daimon,
                    ResourceId::new([n; 16]),
                    ActionSetV1::singleton(ActionV1::Execute),
                )
            })
            .collect();
        let grant = aegis
            .issue(
                GrantDescriptorV1::new(subject, None, id, AuthorityEpoch(0), AegisTime(30), scopes),
                AegisTime(1),
            )
            .unwrap();
        let request = HarnessRequestV1::new(
            ExecutionId::new([2; 16]),
            TaskAttemptRefV1::new([1; 16], 0, 1),
            PrincipalId::new([3; 16]),
            HarnessProfileId::new([4; 16]),
            IdempotencyKey::new([5; 16]),
            vec![],
            vec![ToolCapabilityId::new([6; 16])],
            ExecutionBudgetV1 {
                max_elapsed_millis: 100,
                max_input_tokens: 100,
                max_output_tokens: 100,
                max_tool_calls: 1,
                max_context_bytes: 100,
                max_output_bytes: 100,
            },
        );
        r.execute(
            EffectId::new([2; 16]),
            request,
            FakeReply {
                disposition: ExecutionDispositionV1::Completed,
                output_bytes: 0,
                artifacts: vec![],
            },
            ExecutionAuthority {
                engine: &mut aegis,
                grant: &grant,
                subject,
                session: None,
            },
            LogicalTime(10),
            ExecutionAttempt {
                chronos: c,
                submission: EffectId::new([1; 16]),
                now: SystemTime::UNIX_EPOCH + Duration::from_secs(10),
            },
        )
    }
    #[test]
    fn uncertain_dispatch_never_reexecutes_but_committed_result_replays_exactly() {
        for fault in [Fault::AfterMarker, Fault::BeforeResult, Fault::AfterResult] {
            let f = Fixture::new();
            let (mut c, mut r) = setup(&f.0);
            r.fault = Some(fault);
            assert!(execute(&mut c, &mut r).is_err());
            assert!(r.needs_reopen());
            assert_eq!(
                r.usage().provider_calls,
                if fault == Fault::AfterMarker { 0 } else { 1 }
            );
            assert_eq!(
                r.usage().transactions.commit_timed_samples,
                if fault == Fault::AfterMarker { 1 } else { 2 }
            );
            assert!(matches!(
                execute(&mut c, &mut r),
                Err(FakeReceiverError::Storage(StoreError::NeedsReopen))
            ));
            drop(r);
            drop(c);
            let (mut c, mut r) = setup(&f.0);
            let result = execute(&mut c, &mut r).unwrap();
            assert!(result.replayed);
            assert_eq!(
                matches!(result.evidence, ExecutionEvidence::RecoveryUnknown),
                fault != Fault::AfterResult
            );
            assert_eq!(
                r.recovered_unknown(),
                if fault == Fault::AfterResult { 0 } else { 1 }
            );
            assert_eq!(r.usage().provider_calls, 0);
            let action = if fault == Fault::AfterResult {
                crate::chronos::SettlementAction::Complete
            } else {
                crate::chronos::SettlementAction::KeepUncertain
            };
            let settled = c
                .settle(
                    EffectId::new([1; 16]),
                    c.revision(),
                    &result.settlement,
                    action,
                    crate::chronos::SettlementContext {
                        source_owner: [7; 16],
                        now: SystemTime::UNIX_EPOCH + Duration::from_secs(10),
                    },
                )
                .unwrap();
            assert_eq!(
                settled.phase,
                if fault == Fault::AfterResult {
                    chronos_engine::TaskPhase::Succeeded
                } else {
                    chronos_engine::TaskPhase::Uncertain
                }
            );
        }
    }
    #[test]
    fn deletion_of_a_retained_execution_refuses_reopen_instead_of_executing_again() {
        let f = Fixture::new();
        let (mut c, mut r) = setup(&f.0);
        execute(&mut c, &mut r).unwrap();
        r.connection
            .execute("DELETE FROM fake_executions", [])
            .unwrap();
        drop(r);
        assert!(matches!(
            FakeDaimonReceiver::open(f.0.join("fake.db"), [7; 16], config()),
            Err(FakeReceiverError::InvalidState)
        ));
    }
    #[test]
    fn process_loss_after_durable_marker_exposes_unknown_without_dispatch() {
        let f = Fixture::new();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "daimon::tests::child_exits_after_marker"])
            .env("MOIRA_FAKE_MARKER_DB", &f.0)
            .output()
            .unwrap();
        assert!(out.status.success(), "fake marker child failed");
        let (mut c, mut r) = setup(&f.0);
        let result = execute(&mut c, &mut r).unwrap();
        assert!(matches!(
            result.evidence,
            ExecutionEvidence::RecoveryUnknown
        ));
        assert_eq!(r.usage().provider_calls, 0);
    }
    #[test]
    fn child_exits_after_marker() {
        let Some(path) = std::env::var_os("MOIRA_FAKE_MARKER_DB") else {
            return;
        };
        let (mut c, mut r) = setup(&PathBuf::from(path));
        r.fault = Some(Fault::AfterMarker);
        assert!(execute(&mut c, &mut r).is_err());
        std::process::exit(0);
    }
}
