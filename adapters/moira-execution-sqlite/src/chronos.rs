//! Dedicated single-owner Chronos receiver. Task scheduling stays in Chronos;
//! this adapter owns the durable canonical-intent/admission-receipt inbox.
use chronos_engine::{
    Agent, CapabilityGrant, Checkpoint, Engine, EngineLimits, IntegrationEffectIdV1,
    RecoveryLimits, RetrySafetyV1, SnapshotRevision, StateRevision, TaskId, TaskPhase, TaskSpecV2,
    TaskState,
};
use moira_core::{ChronosAdmissionReceiptV1, ChronosAttemptReceiptV1, ChronosTaskRefV1, EffectId};
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

const APPLICATION_ID: i64 = 0x4d434852;
mod settlement;
pub use settlement::{Settlement, SettlementAction, SettlementContext};
const SCHEMA: &str = "CREATE TABLE receiver_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), revision BLOB NOT NULL CHECK(length(revision)=8), checkpoint BLOB NOT NULL, settlements INTEGER NOT NULL DEFAULT 0 CHECK(settlements>=0));
CREATE TABLE receiver_receipts (effect BLOB PRIMARY KEY CHECK(length(effect)=16), intent BLOB NOT NULL, task_id TEXT NOT NULL UNIQUE, incarnation BLOB NOT NULL CHECK(length(incarnation)=8), revision BLOB NOT NULL CHECK(length(revision)=8));";
const ATTEMPTS: &str = "CREATE TABLE receiver_attempts (effect BLOB PRIMARY KEY CHECK(length(effect)=16) REFERENCES receiver_receipts(effect), number INTEGER NOT NULL CHECK(number>0 AND number<=4294967295), revision BLOB NOT NULL CHECK(length(revision)=8), fenced INTEGER NOT NULL CHECK(fenced IN(0,1)));";
const SETTLEMENTS: &str = "CREATE TABLE receiver_settlements (effect BLOB PRIMARY KEY CHECK(length(effect)=16) REFERENCES receiver_attempts(effect), evidence BLOB NOT NULL CHECK(length(evidence)=88), revision BLOB NOT NULL CHECK(length(revision)=8), conflicting BLOB CHECK(conflicting IS NULL OR length(conflicting)=88));";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiverLimits {
    pub engine: EngineLimits,
    pub max_snapshot_bytes: usize,
    pub max_intent_bytes: usize,
    pub max_receipts: usize,
    pub max_database_pages: u32,
    pub timing: Timing,
}

#[derive(Debug)]
pub enum ReceiverError {
    Storage(StoreError),
    InvalidConfig,
    Capacity,
    IntentConflict,
    AdmissionRefused,
    GrantRefused,
    ClaimRefused,
    SettlementRefused,
    SettlementConflict,
    ReconciliationRequired,
    EvidenceConflict,
    Checkpoint,
    InvalidState,
}
impl std::fmt::Display for ReceiverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Chronos receiver: {self:?}")
    }
}
impl std::error::Error for ReceiverError {}
impl From<StoreError> for ReceiverError {
    fn from(e: StoreError) -> Self {
        Self::Storage(e)
    }
}
impl From<rusqlite::Error> for ReceiverError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Admission {
    pub receipt: ChronosAdmissionReceiptV1,
    pub incarnation: u64,
    /// Retained receipt, not permission to dispatch a second task.
    pub replayed: bool,
}

/// Exact owner-retained namespace binding for one admitted task.
///
/// This is read-only provenance. It does not identify an ambient workspace,
/// grant a lease, qualify a task-conversation link, or authorize settlement.
#[derive(Clone, Eq, PartialEq)]
pub struct TaskBindingProvenanceV1 {
    pub external_task_key: [u8; 16],
    pub chronos_task_id: String,
    pub incarnation: u64,
    pub chronos_revision: u64,
}

impl std::fmt::Debug for TaskBindingProvenanceV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaskBindingProvenanceV1")
            .field("external_task_key", &"<opaque>")
            .field("chronos_task_id_bytes", &self.chronos_task_id.len())
            .field("incarnation", &self.incarnation)
            .field("chronos_revision", &self.chronos_revision)
            .finish()
    }
}

/// Authority-free attribution plus current phase, never a lease. Replayed
/// Running is an observation of the original claim, not fresh dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Attempt {
    pub receipt: ChronosAttemptReceiptV1,
    pub number: u32,
    pub phase: TaskPhase,
    pub replayed: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkerUsage {
    pub offers: u64,
    pub issued: u64,
    pub refused: u64,
    pub overflowed: bool,
}

/// Startup observation only; an open failure is reported through its Result.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecoveryUsage {
    pub newly_fenced_attempts: usize,
    /// SQL recovery-fence transaction only, not total startup latency.
    pub commit_service: Option<Duration>,
}

enum PendingWrite<'a> {
    Admission {
        effect: EffectId,
        intent: &'a [u8],
        task_id: &'a str,
        incarnation: u64,
        revision: u64,
    },
    Claim {
        effect: EffectId,
        number: u32,
        revision: u64,
    },
    Settlement {
        effect: EffectId,
        evidence: &'a [u8; 88],
        revision: u64,
        fenced: bool,
    },
}

struct RetainedAdmission {
    intent: Vec<u8>,
    task_id: String,
    incarnation: Vec<u8>,
    revision: Vec<u8>,
}

pub struct ChronosReceiver {
    // SQLite must close before releasing the cooperative owner lock.
    connection: Connection,
    resource: OwnedResource,
    engine: Engine,
    confirmed_revision: StateRevision,
    confirmed_task_count: usize,
    limits: ReceiverLimits,
    healthy: bool,
    usage: Usage,
    claim_usage: Usage,
    settlement_usage: Usage,
    evidence_conflict: bool,
    worker_usage: WorkerUsage,
    recovery_usage: RecoveryUsage,
    #[cfg(test)]
    fault: Option<crate::Fault>,
}
impl std::fmt::Debug for ChronosReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChronosReceiver(<redacted>)")
    }
}
impl ChronosReceiver {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        limits: ReceiverLimits,
    ) -> Result<Self, ReceiverError> {
        if limits.max_receipts == 0
            || limits.max_receipts > limits.engine.max_tasks
            || limits.max_intent_bytes == 0
            || limits.max_snapshot_bytes == 0
        {
            return Err(ReceiverError::InvalidConfig);
        }
        let record_limit: i32 = limits
            .max_intent_bytes
            .checked_add(limits.engine.max_task_id_bytes)
            .map(|v| v.max(limits.max_snapshot_bytes))
            .and_then(|v| v.checked_add(2048))
            .and_then(|v| v.try_into().ok())
            .ok_or(ReceiverError::InvalidConfig)?;
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
            let mut q=connection.prepare("SELECT name FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 5")?;
            q.query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let fresh = app == 0 && version == 0 && names.is_empty();
        if !fresh
            && (app != APPLICATION_ID
                || !matches!(version, 1..=3)
                || (version == 1 && names != ["receiver_meta", "receiver_receipts"])
                || (version == 2
                    && names != ["receiver_attempts", "receiver_meta", "receiver_receipts"])
                || (version == 3
                    && names
                        != [
                            "receiver_attempts",
                            "receiver_meta",
                            "receiver_receipts",
                            "receiver_settlements",
                        ]))
        {
            return Err(StoreError::ForeignDatabase.into());
        }
        // Validate an existing owner/state before persistent pragma changes.
        let (mut engine, recovered) = if fresh {
            (Engine::with_limits(limits.engine), Vec::new())
        } else {
            load(&connection, owner, limits, version >= 2, version >= 3)?
        };
        let journal: String =
            connection.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
        connection.execute_batch(
            "PRAGMA synchronous=EXTRA; PRAGMA fullfsync=ON; PRAGMA temp_store=MEMORY;",
        )?;
        connection.pragma_update(None, "max_page_count", limits.max_database_pages)?;
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |r| r.get(0))?;
        let pages: u32 = connection.pragma_query_value(None, "max_page_count", |r| r.get(0))?;
        let sync: i64 = connection.pragma_query_value(None, "synchronous", |r| r.get(0))?;
        let full_sync: i64 = connection.pragma_query_value(None, "fullfsync", |r| r.get(0))?;
        if journal != "delete"
            || sync != 3
            || full_sync != 1
            || page_size != 4096
            || pages > limits.max_database_pages
        {
            return Err(ReceiverError::InvalidConfig);
        }
        if fresh {
            let cp = engine
                .prepare_checkpoint(limits.max_snapshot_bytes)
                .map_err(|_| ReceiverError::Checkpoint)?;
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA)?;
            tx.execute_batch(ATTEMPTS)?;
            tx.execute_batch(SETTLEMENTS)?;
            tx.execute(
                "INSERT INTO receiver_meta (slot,owner,revision,checkpoint) VALUES (1,?1,?2,?3)",
                params![
                    owner.as_slice(),
                    cp.revision().get().to_be_bytes().as_slice(),
                    cp.bytes()
                ],
            )?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", 3)?;
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
            resource.sync_parent()?;
        }
        if !fresh && version < 3 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if version == 1 {
                tx.execute_batch(ATTEMPTS)?;
            }
            tx.execute_batch("ALTER TABLE receiver_meta ADD COLUMN settlements INTEGER NOT NULL DEFAULT 0 CHECK(settlements>=0);")?;
            tx.execute_batch(SETTLEMENTS)?;
            tx.pragma_update(None, "user_version", 3)?;
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
        }
        let mut recovery_usage = RecoveryUsage {
            newly_fenced_attempts: recovered.len(),
            commit_service: None,
        };
        if !recovered.is_empty() {
            // Chronos has already revoked recovered leases in memory. Commit
            // that fence before publishing any post-recovery observation.
            let cp = engine
                .prepare_checkpoint(limits.max_snapshot_bytes)
                .map_err(|_| ReceiverError::Checkpoint)?;
            let started = match limits.timing {
                Timing::Off => None,
                Timing::Process => std::panic::catch_unwind(Instant::now).ok(),
            };
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let actual: Vec<u8> =
                tx.query_row("SELECT revision FROM receiver_meta WHERE slot=1", [], |r| {
                    r.get(0)
                })?;
            if number(&actual)? != cp.expected_revision().get() {
                return Err(ReceiverError::InvalidState);
            }
            tx.execute(
                "UPDATE receiver_meta SET revision=?1,checkpoint=?2 WHERE slot=1",
                params![cp.revision().get().to_be_bytes().as_slice(), cp.bytes()],
            )?;
            for effect in recovered {
                tx.execute(
                    "UPDATE receiver_attempts SET fenced=1 WHERE effect=?1",
                    [effect.as_slice()],
                )?;
            }
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
            let mut timing = Usage::default();
            record_timing(&mut timing, started);
            recovery_usage.commit_service = timing.max_commit_service;
        }
        resource.check()?;
        let evidence_conflict: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM receiver_settlements WHERE conflicting IS NOT NULL)",
            [],
            |r| r.get(0),
        )?;
        Ok(Self {
            confirmed_revision: engine.task_state_revision(),
            confirmed_task_count: engine.tasks().count(),
            connection,
            resource,
            engine,
            limits,
            healthy: true,
            usage: Usage::default(),
            claim_usage: Usage::default(),
            settlement_usage: Usage::default(),
            evidence_conflict,
            worker_usage: WorkerUsage::default(),
            recovery_usage,
            #[cfg(test)]
            fault: None,
        })
    }
    pub fn needs_reopen(&self) -> bool {
        !self.healthy
    }
    pub fn revision(&self) -> StateRevision {
        self.confirmed_revision
    }
    pub fn task_count(&self) -> usize {
        self.confirmed_task_count
    }
    pub fn usage(&self) -> Usage {
        self.usage
    }
    pub fn claim_usage(&self) -> Usage {
        self.claim_usage
    }
    pub fn worker_usage(&self) -> WorkerUsage {
        self.worker_usage
    }
    pub fn recovery_usage(&self) -> RecoveryUsage {
        self.recovery_usage
    }
    /// Host-authenticated assertion; this adapter does not authenticate agents.
    /// Grants are bounded, volatile and foreign after recovery. Leases never
    /// leave the receiver. No grant is serialized as durable authority.
    pub fn authorize_worker(
        &mut self,
        agent: Agent,
        now: SystemTime,
        validity: Duration,
        claims: u64,
    ) -> Result<CapabilityGrant, ReceiverError> {
        let result = (|| {
            self.guard()?;
            let time = self
                .engine
                .advance_time(now)
                .map_err(|_| ReceiverError::GrantRefused)?;
            self.engine
                .issue_grant_with_claim_budget_at_time(agent, &time, validity, claims)
                .map_err(|_| ReceiverError::GrantRefused)
        })();
        let u = &mut self.worker_usage;
        count(&mut u.offers, &mut u.overflowed);
        count(
            if result.is_ok() {
                &mut u.issued
            } else {
                &mut u.refused
            },
            &mut u.overflowed,
        );
        result
    }
    /// Commit an exact targeted claim before exposing authority-free attribution.
    /// Matching replay observes the original attempt and its current phase only.
    pub fn claim(
        &mut self,
        effect: EffectId,
        expected: StateRevision,
        grant: &CapabilityGrant,
        now: SystemTime,
        duration: Duration,
    ) -> Result<Attempt, ReceiverError> {
        let result = self.claim_inner(effect, expected, grant, now, duration);
        let u = &mut self.claim_usage;
        count(&mut u.command_offers, &mut u.overflowed);
        match &result {
            Ok(a) if a.replayed => count(&mut u.replayed, &mut u.overflowed),
            Ok(_) => count(&mut u.committed, &mut u.overflowed),
            Err(ReceiverError::Storage(StoreError::Indeterminate)) => {
                count(&mut u.indeterminate, &mut u.overflowed)
            }
            Err(_) => count(&mut u.refused, &mut u.overflowed),
        }
        result
    }
    fn claim_inner(
        &mut self,
        effect: EffectId,
        expected: StateRevision,
        grant: &CapabilityGrant,
        now: SystemTime,
        duration: Duration,
    ) -> Result<Attempt, ReceiverError> {
        self.guard()?;
        let binding: Option<(String, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT task_id,incarnation FROM receiver_receipts WHERE effect=?1",
                [effect.into_bytes().as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .inspect_err(|_| self.healthy = false)?;
        let (id, incarnation) = binding.ok_or(ReceiverError::ClaimRefused)?;
        let incarnation = number(&incarnation).inspect_err(|_| self.healthy = false)?;
        let id = chronos_engine::TaskId::new(id);
        let retained: Option<(u32, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT number,revision FROM receiver_attempts WHERE effect=?1",
                [effect.into_bytes().as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .inspect_err(|_| self.healthy = false)?;
        if let Some((attempt, revision)) = retained {
            let task = self.engine.task(&id).ok_or(ReceiverError::InvalidState)?;
            let phase = match &task.state {
                TaskState::Running(claim) if claim.attempt_number == attempt => {
                    if claim.is_expired_at(now) {
                        return Err(ReceiverError::ClaimRefused);
                    }
                    TaskPhase::Running
                }
                TaskState::Uncertain { attempt_number, .. } if *attempt_number == attempt => {
                    TaskPhase::Uncertain
                }
                TaskState::Succeeded if task.attempts == attempt => TaskPhase::Succeeded,
                TaskState::Failed { .. } if task.attempts == attempt => TaskPhase::Failed,
                TaskState::Cancelled if task.attempts == attempt => TaskPhase::Cancelled,
                _ => {
                    self.healthy = false;
                    return Err(ReceiverError::InvalidState);
                }
            };
            return Ok(attempt_result(
                effect,
                incarnation,
                attempt,
                number(&revision).inspect_err(|_| self.healthy = false)?,
                phase,
                true,
            ));
        }
        if self.engine.task_state_revision() != expected {
            return Err(ReceiverError::ClaimRefused);
        }
        let time = self
            .engine
            .advance_time(now)
            .map_err(|_| ReceiverError::ClaimRefused)?;
        let task = self
            .engine
            .claim_task_guarded(expected, grant, &id, &time, duration)
            .map_err(|_| ReceiverError::ClaimRefused)?;
        self.healthy = false;
        let TaskState::Running(claim) = task.state else {
            return Err(ReceiverError::InvalidState);
        };
        let attempt = claim.attempt_number;
        let StateRevision::Finite(revision) = self.engine.task_state_revision() else {
            return Err(ReceiverError::InvalidState);
        };
        let cp = self
            .engine
            .prepare_checkpoint(self.limits.max_snapshot_bytes)
            .map_err(|_| ReceiverError::Checkpoint)?;
        self.commit(
            &cp,
            PendingWrite::Claim {
                effect,
                number: attempt,
                revision,
            },
        )?;
        Ok(attempt_result(
            effect,
            incarnation,
            attempt,
            revision,
            TaskPhase::Running,
            false,
        ))
    }
    fn guard(&mut self) -> Result<(), ReceiverError> {
        if !self.healthy {
            return Err(StoreError::NeedsReopen.into());
        }
        if self.evidence_conflict {
            return Err(ReceiverError::EvidenceConflict);
        }
        self.resource
            .check()
            .inspect_err(|_| self.healthy = false)?;
        Ok(())
    }
    pub(crate) fn verify_execution(
        &mut self,
        submission: EffectId,
        attempt: &daimon_harness::TaskAttemptRefV1,
        now: SystemTime,
    ) -> Result<(), ReceiverError> {
        self.guard()?;
        if attempt.task_key() != submission.into_bytes() {
            return Err(ReceiverError::ClaimRefused);
        }
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT task_id FROM receiver_receipts WHERE effect=?1",
                [submission.into_bytes().as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        let id = chronos_engine::TaskId::new(id.ok_or(ReceiverError::ClaimRefused)?);
        self.engine
            .advance_time(now)
            .map_err(|_| ReceiverError::ClaimRefused)?;
        let task = self.engine.task(&id).ok_or(ReceiverError::ClaimRefused)?;
        if task.reference().map(|r| r.incarnation().get()) != Some(attempt.task_incarnation()) {
            return Err(ReceiverError::ClaimRefused);
        }
        match &task.state {
            TaskState::Running(claim)
                if claim.attempt_number == attempt.attempt_number()
                    && !claim.is_expired_at(now) =>
            {
                Ok(())
            }
            _ => Err(ReceiverError::ClaimRefused),
        }
    }
    /// Exactly one minimized task per workflow effect in this receiver profile.
    /// The expected revision guards new admission, not observation of replay.
    pub fn admit(
        &mut self,
        effect: EffectId,
        expected: StateRevision,
        task: TaskSpecV2,
    ) -> Result<Admission, ReceiverError> {
        let result = self.admit_inner(effect, expected, task);
        let u = &mut self.usage;
        count(&mut u.command_offers, &mut u.overflowed);
        match &result {
            Ok(a) if a.replayed => count(&mut u.replayed, &mut u.overflowed),
            Ok(_) => count(&mut u.committed, &mut u.overflowed),
            Err(ReceiverError::Storage(StoreError::Indeterminate)) => {
                count(&mut u.indeterminate, &mut u.overflowed)
            }
            Err(e) => {
                count(&mut u.refused, &mut u.overflowed);
                if matches!(e, ReceiverError::IntentConflict) {
                    count(&mut u.conflicts, &mut u.overflowed)
                }
            }
        }
        result
    }

    /// Observes the exact task namespace retained by this receiver for an
    /// admission effect. `None` is authoritative only for this open receiver.
    pub fn observe_task_binding(
        &mut self,
        effect: EffectId,
    ) -> Result<Option<TaskBindingProvenanceV1>, ReceiverError> {
        self.guard()?;
        let retained: Option<(String, Vec<u8>, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT task_id,incarnation,revision FROM receiver_receipts WHERE effect=?1",
                [effect.into_bytes().as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .inspect_err(|_| self.healthy = false)?;
        let Some((chronos_task_id, incarnation, chronos_revision)) = retained else {
            return Ok(None);
        };
        let incarnation = number(&incarnation).inspect_err(|_| self.healthy = false)?;
        let chronos_revision = number(&chronos_revision).inspect_err(|_| self.healthy = false)?;
        let task_id = TaskId::new(chronos_task_id.clone());
        let task = self.engine.task(&task_id).ok_or_else(|| {
            self.healthy = false;
            ReceiverError::InvalidState
        })?;
        if task
            .reference()
            .map(|reference| reference.incarnation().get())
            != Some(incarnation)
        {
            self.healthy = false;
            return Err(ReceiverError::InvalidState);
        }
        Ok(Some(TaskBindingProvenanceV1 {
            external_task_key: effect.into_bytes(),
            chronos_task_id,
            incarnation,
            chronos_revision,
        }))
    }

    fn admit_inner(
        &mut self,
        effect: EffectId,
        expected: StateRevision,
        task: TaskSpecV2,
    ) -> Result<Admission, ReceiverError> {
        self.guard()?;
        let intent = canonical_intent(&task, self.limits)?;
        let retained: Option<RetainedAdmission> = self
            .connection
            .query_row(
                "SELECT intent,task_id,incarnation,revision FROM receiver_receipts WHERE effect=?1",
                [effect.into_bytes().as_slice()],
                |r| {
                    Ok(RetainedAdmission {
                        intent: r.get(0)?,
                        task_id: r.get(1)?,
                        incarnation: r.get(2)?,
                        revision: r.get(3)?,
                    })
                },
            )
            .optional()
            .inspect_err(|_| self.healthy = false)?;
        if let Some(retained) = retained {
            if retained.intent != intent {
                return Err(ReceiverError::IntentConflict);
            }
            if retained.task_id != task.id.as_str() {
                self.healthy = false;
                return Err(ReceiverError::InvalidState);
            }
            let incarnation =
                number(&retained.incarnation).inspect_err(|_| self.healthy = false)?;
            let revision = number(&retained.revision).inspect_err(|_| self.healthy = false)?;
            return Ok(admission(effect, incarnation, revision, true));
        }
        let retained: i64 =
            self.connection
                .query_row("SELECT count(*) FROM receiver_receipts", [], |r| r.get(0))?;
        if retained < 0 || retained as u64 >= self.limits.max_receipts as u64 {
            return Err(ReceiverError::Capacity);
        }
        let task_id = task.id.clone();
        let receipt = self
            .engine
            .submit_coordination_graph_with_receipt(
                expected,
                IntegrationEffectIdV1::new(effect.into_bytes()),
                vec![task],
            )
            .map_err(|_| ReceiverError::AdmissionRefused)?;
        // From this point any failure requires recovery of the prior or newly
        // committed checkpoint; the mutated candidate cannot escape to callers.
        self.healthy = false;
        let StateRevision::Finite(revision) = receipt.revision() else {
            return Err(ReceiverError::InvalidState);
        };
        let [task_ref] = receipt.tasks() else {
            return Err(ReceiverError::InvalidState);
        };
        let incarnation = task_ref.incarnation().get();
        let cp = self
            .engine
            .prepare_checkpoint(self.limits.max_snapshot_bytes)
            .map_err(|_| ReceiverError::Checkpoint)?;
        self.commit(
            &cp,
            PendingWrite::Admission {
                effect,
                intent: &intent,
                task_id: task_id.as_str(),
                incarnation,
                revision,
            },
        )?;
        Ok(admission(effect, incarnation, revision, false))
    }
    fn commit(&mut self, cp: &Checkpoint, write: PendingWrite<'_>) -> Result<(), ReceiverError> {
        let is_claim = matches!(write, PendingWrite::Claim { .. });
        let is_settlement = matches!(write, PendingWrite::Settlement { .. });
        let started = match self.limits.timing {
            Timing::Off => None,
            Timing::Process => std::panic::catch_unwind(Instant::now).ok(),
        };
        let result = (|| -> Result<(), ReceiverError> {
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let actual: Vec<u8> =
                tx.query_row("SELECT revision FROM receiver_meta WHERE slot=1", [], |r| {
                    r.get(0)
                })?;
            if number(&actual)? != cp.expected_revision().get() {
                return Err(ReceiverError::InvalidState);
            }
            tx.execute(
                "UPDATE receiver_meta SET revision=?1,checkpoint=?2 WHERE slot=1",
                params![cp.revision().get().to_be_bytes().as_slice(), cp.bytes()],
            )?;
            match write {
                PendingWrite::Admission {
                    effect,
                    intent,
                    task_id,
                    incarnation,
                    revision,
                } => {
                    tx.execute(
                        "INSERT INTO receiver_receipts VALUES (?1,?2,?3,?4,?5)",
                        params![
                            effect.into_bytes().as_slice(),
                            intent,
                            task_id,
                            incarnation.to_be_bytes().as_slice(),
                            revision.to_be_bytes().as_slice()
                        ],
                    )?;
                }
                PendingWrite::Claim {
                    effect,
                    number,
                    revision,
                } => {
                    tx.execute(
                        "INSERT INTO receiver_attempts VALUES (?1,?2,?3,0)",
                        params![
                            effect.into_bytes().as_slice(),
                            number,
                            revision.to_be_bytes().as_slice()
                        ],
                    )?;
                }
                PendingWrite::Settlement {
                    effect,
                    evidence,
                    revision,
                    fenced,
                } => {
                    tx.execute(
                        "INSERT INTO receiver_settlements VALUES (?1,?2,?3,NULL)",
                        params![
                            effect.into_bytes().as_slice(),
                            evidence.as_slice(),
                            revision.to_be_bytes().as_slice()
                        ],
                    )?;
                    tx.execute(
                        "UPDATE receiver_meta SET settlements=settlements+1 WHERE slot=1",
                        [],
                    )?;
                    if fenced {
                        tx.execute(
                            "UPDATE receiver_attempts SET fenced=1 WHERE effect=?1",
                            [effect.into_bytes().as_slice()],
                        )?;
                    }
                }
            }
            #[cfg(test)]
            if self.fault == Some(crate::Fault::BeforeCommit) {
                self.fault = None;
                tx.rollback()?;
                return Err(StoreError::Io(std::io::ErrorKind::Interrupted).into());
            }
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
            #[cfg(test)]
            if self.fault == Some(crate::Fault::AfterCommit) {
                self.fault = None;
                return Err(StoreError::Indeterminate.into());
            }
            self.resource
                .check()
                .map_err(|_| StoreError::Indeterminate)?;
            Ok(())
        })();
        record_timing(
            if is_settlement {
                &mut self.settlement_usage
            } else if is_claim {
                &mut self.claim_usage
            } else {
                &mut self.usage
            },
            started,
        );
        result?;
        self.healthy = true;
        self.confirmed_revision = self.engine.task_state_revision();
        self.confirmed_task_count = self.engine.tasks().count();
        Ok(())
    }
}

fn admission(effect: EffectId, incarnation: u64, revision: u64, replayed: bool) -> Admission {
    Admission {
        receipt: ChronosAdmissionReceiptV1::new(
            effect,
            ChronosTaskRefV1::new(effect.into_bytes(), incarnation),
            revision,
        ),
        incarnation,
        replayed,
    }
}
fn attempt_result(
    effect: EffectId,
    incarnation: u64,
    attempt: u32,
    revision: u64,
    phase: TaskPhase,
    replayed: bool,
) -> Attempt {
    Attempt {
        receipt: ChronosAttemptReceiptV1::new(
            ChronosTaskRefV1::new(effect.into_bytes(), incarnation),
            attempt,
            revision,
        ),
        number: attempt,
        phase,
        replayed,
    }
}
fn number(bytes: &[u8]) -> Result<u64, ReceiverError> {
    Ok(u64::from_be_bytes(
        bytes.try_into().map_err(|_| ReceiverError::InvalidState)?,
    ))
}
fn load(
    c: &Connection,
    owner: [u8; 16],
    limits: ReceiverLimits,
    has_attempts: bool,
    has_settlements: bool,
) -> Result<(Engine, Vec<[u8; 16]>), ReceiverError> {
    let mut q = c.prepare("SELECT owner,revision,checkpoint FROM receiver_meta WHERE slot=1")?;
    let mut rows = q.query([])?;
    let row = rows.next()?.ok_or(ReceiverError::InvalidState)?;
    let retained = row
        .get_ref(0)?
        .as_blob()
        .map_err(|_| ReceiverError::InvalidState)?;
    if retained != owner {
        return Err(StoreError::OwnerMismatch.into());
    }
    let revision = number(
        row.get_ref(1)?
            .as_blob()
            .map_err(|_| ReceiverError::InvalidState)?,
    )?;
    let engine = Engine::recover_checkpoint(
        row.get_ref(2)?
            .as_blob()
            .map_err(|_| ReceiverError::InvalidState)?,
        RecoveryLimits::new(limits.max_snapshot_bytes, limits.engine),
    )
    .map_err(|_| ReceiverError::Checkpoint)?;
    if engine.snapshot_revision() != SnapshotRevision::new(revision) {
        return Err(ReceiverError::InvalidState);
    }
    let mut q = c.prepare(
        "SELECT effect,intent,task_id,incarnation,revision FROM receiver_receipts ORDER BY effect",
    )?;
    let mut rows = q.query([])?;
    let mut count = 0usize;
    let mut attempt_count = 0i64;
    let mut settlement_count = 0i64;
    let mut recovered = Vec::new();
    while let Some(row) = rows.next()? {
        count = count
            .checked_add(1)
            .filter(|&n| n <= limits.max_receipts)
            .ok_or(ReceiverError::Capacity)?;
        let effect = row
            .get_ref(0)?
            .as_blob()
            .map_err(|_| ReceiverError::InvalidState)?;
        if effect.len() != 16 {
            return Err(ReceiverError::InvalidState);
        }
        let id = row
            .get_ref(2)?
            .as_str()
            .map_err(|_| ReceiverError::InvalidState)?;
        if id.len() > limits.engine.max_task_id_bytes {
            return Err(ReceiverError::Capacity);
        }
        let task = engine
            .task(&chronos_engine::TaskId::new(id))
            .ok_or(ReceiverError::InvalidState)?;
        let incarnation = number(
            row.get_ref(3)?
                .as_blob()
                .map_err(|_| ReceiverError::InvalidState)?,
        )?;
        if task.reference().map(|r| r.incarnation().get()) != Some(incarnation) {
            return Err(ReceiverError::InvalidState);
        }
        let accepted = number(
            row.get_ref(4)?
                .as_blob()
                .map_err(|_| ReceiverError::InvalidState)?,
        )?;
        if accepted == 0 || StateRevision::finite(accepted) > engine.task_state_revision() {
            return Err(ReceiverError::InvalidState);
        }
        let original = TaskSpecV2 {
            id: task.id.clone(),
            priority: task.priority,
            dependencies: task.dependencies.clone(),
            required_capabilities: task.required_capabilities.clone(),
            max_attempts: task.max_attempts,
            retry_safety: if task.idempotency_key.is_some() {
                RetrySafetyV1::ConnectorIdempotent
            } else {
                RetrySafetyV1::FenceUnknown
            },
        };
        if canonical_intent(&original, limits)?
            != row
                .get_ref(1)?
                .as_blob()
                .map_err(|_| ReceiverError::InvalidState)?
        {
            return Err(ReceiverError::InvalidState);
        }
        let attempt: Option<(u32, Vec<u8>, i64)> = if has_attempts {
            c.query_row(
                "SELECT number,revision,fenced FROM receiver_attempts WHERE effect=?1",
                [effect],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
        } else {
            None
        };
        match attempt {
            None if task.attempts == 0
                && matches!(
                    task.state,
                    TaskState::Ready
                        | TaskState::Blocked
                        | TaskState::Cancelled
                        | TaskState::Failed { .. }
                ) => {}
            Some((attempt, revision, fenced)) if attempt > 0 && attempt == task.attempts => {
                attempt_count += 1;
                let revision = number(&revision)?;
                if revision <= accepted
                    || StateRevision::finite(revision) > engine.task_state_revision()
                    || !matches!(fenced, 0 | 1)
                {
                    return Err(ReceiverError::InvalidState);
                }
                let settled = has_settlements
                    && settlement::validate_retained(
                        c,
                        effect,
                        task,
                        attempt,
                        revision,
                        engine.task_state_revision(),
                    )?;
                if settled {
                    settlement_count += 1;
                }
                match &task.state {
                    TaskState::Uncertain { attempt_number, .. } if *attempt_number == attempt => {
                        if settled && fenced == 0 {
                            return Err(ReceiverError::InvalidState);
                        }
                        if fenced == 0 {
                            recovered
                                .try_reserve(1)
                                .map_err(|_| ReceiverError::Capacity)?;
                            recovered
                                .push(effect.try_into().map_err(|_| ReceiverError::InvalidState)?);
                        }
                    }
                    TaskState::Succeeded | TaskState::Failed { .. } | TaskState::Cancelled
                        if settled => {}
                    _ => return Err(ReceiverError::InvalidState),
                }
            }
            _ => return Err(ReceiverError::InvalidState),
        }
    }
    // This profile admits one task per effect and exposes no retirement or
    // unrelated task mutation. A missing receipt must not become a new inbox.
    if count != engine.tasks().count() {
        return Err(ReceiverError::InvalidState);
    }
    if has_attempts {
        let actual: i64 =
            c.query_row("SELECT count(*) FROM receiver_attempts", [], |r| r.get(0))?;
        if actual != attempt_count {
            return Err(ReceiverError::InvalidState);
        }
    }
    if has_settlements {
        let actual: i64 = c.query_row("SELECT count(*) FROM receiver_settlements", [], |r| {
            r.get(0)
        })?;
        let declared: i64 = c.query_row(
            "SELECT settlements FROM receiver_meta WHERE slot=1",
            [],
            |r| r.get(0),
        )?;
        if actual != settlement_count || actual != declared {
            return Err(ReceiverError::InvalidState);
        }
    }
    Ok((engine, recovered))
}

fn canonical_intent(task: &TaskSpecV2, limits: ReceiverLimits) -> Result<Vec<u8>, ReceiverError> {
    if task.id.as_str().len() > limits.engine.max_task_id_bytes
        || task.dependencies.len() > limits.engine.max_dependency_edges
        || task.required_capabilities.len() > limits.engine.max_task_capabilities
    {
        return Err(ReceiverError::Capacity);
    }
    let mut out = IntentWriter {
        bytes: Vec::new(),
        max: limits.max_intent_bytes,
    };
    out.raw(b"MCHRINT\0\x01")?;
    out.field(task.id.as_str().as_bytes())?;
    out.raw(&task.priority.to_be_bytes())?;
    out.raw(&task.max_attempts.to_be_bytes())?;
    out.raw(&[match task.retry_safety {
        RetrySafetyV1::FenceUnknown => 0,
        RetrySafetyV1::ConnectorIdempotent => 1,
    }])?;
    out.raw(&(task.dependencies.len() as u64).to_be_bytes())?;
    for id in &task.dependencies {
        if id.as_str().len() > limits.engine.max_task_id_bytes {
            return Err(ReceiverError::Capacity);
        }
        out.field(id.as_str().as_bytes())?;
    }
    out.raw(&(task.required_capabilities.len() as u64).to_be_bytes())?;
    for name in &task.required_capabilities {
        if name.len() > limits.engine.max_capability_name_bytes {
            return Err(ReceiverError::Capacity);
        }
        out.field(name.as_bytes())?;
    }
    Ok(out.bytes)
}
struct IntentWriter {
    bytes: Vec<u8>,
    max: usize,
}
impl IntentWriter {
    fn raw(&mut self, value: &[u8]) -> Result<(), ReceiverError> {
        if self
            .bytes
            .len()
            .checked_add(value.len())
            .is_none_or(|n| n > self.max)
        {
            return Err(ReceiverError::Capacity);
        }
        self.bytes
            .try_reserve(value.len())
            .map_err(|_| ReceiverError::Capacity)?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }
    fn field(&mut self, value: &[u8]) -> Result<(), ReceiverError> {
        let len: u32 = value
            .len()
            .try_into()
            .map_err(|_| ReceiverError::Capacity)?;
        self.raw(&len.to_be_bytes())?;
        self.raw(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    pub(super) struct Fixture(PathBuf);
    impl Fixture {
        pub(super) fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "moira-chronos-unit-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        pub(super) fn path(&self) -> PathBuf {
            self.0.join("peer.db")
        }
        pub(super) fn open(&self) -> ChronosReceiver {
            ChronosReceiver::open(self.path(), [7; 16], limits()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for name in ["peer.db", "peer.db-journal", "peer.db.owner.lock"] {
                let _ = fs::remove_file(self.0.join(name));
            }
            let _ = fs::remove_dir(&self.0);
        }
    }
    pub(super) fn limits() -> ReceiverLimits {
        ReceiverLimits {
            engine: EngineLimits {
                max_tasks: 2,
                ..EngineLimits::default()
            },
            max_receipts: 2,
            max_snapshot_bytes: 16384,
            max_intent_bytes: 2048,
            max_database_pages: 64,
            timing: Timing::Process,
        }
    }
    #[test]
    fn task_and_receipt_commit_together_and_uncertain_result_withholds_candidate() {
        for fault in [crate::Fault::BeforeCommit, crate::Fault::AfterCommit] {
            let f = Fixture::new();
            let mut r = f.open();
            r.fault = Some(fault);
            let result = r.admit(
                EffectId::new([1; 16]),
                StateRevision::ZERO,
                TaskSpecV2::new("work"),
            );
            match fault {
                crate::Fault::BeforeCommit => assert!(matches!(
                    result,
                    Err(ReceiverError::Storage(StoreError::Io(_)))
                )),
                crate::Fault::AfterCommit => assert!(matches!(
                    result,
                    Err(ReceiverError::Storage(StoreError::Indeterminate))
                )),
            }
            assert!(r.needs_reopen());
            assert_eq!(r.revision(), StateRevision::ZERO);
            assert_eq!(r.task_count(), 0);
            assert_eq!(r.usage().committed, 0);
            assert_eq!(r.usage().commit_timed_samples, 1);
            assert!(matches!(
                r.admit(
                    EffectId::new([1; 16]),
                    StateRevision::ZERO,
                    TaskSpecV2::new("work")
                ),
                Err(ReceiverError::Storage(StoreError::NeedsReopen))
            ));
            drop(r);
            let mut r = f.open();
            let result = r
                .admit(
                    EffectId::new([1; 16]),
                    StateRevision::ZERO,
                    TaskSpecV2::new("work"),
                )
                .unwrap();
            assert_eq!(result.replayed, fault == crate::Fault::AfterCommit);
            assert_eq!(result.incarnation, 0);
            assert_eq!(r.task_count(), 1);
        }
    }
    #[test]
    fn missing_receipt_or_changed_intent_refuses_recovery_instead_of_resubmitting() {
        for sql in [
            "DELETE FROM receiver_receipts",
            "UPDATE receiver_receipts SET intent=x'00'",
            "UPDATE receiver_receipts SET incarnation=x'ffffffffffffffff'",
        ] {
            let f = Fixture::new();
            let mut r = f.open();
            r.admit(
                EffectId::new([1; 16]),
                StateRevision::ZERO,
                TaskSpecV2::new("work"),
            )
            .unwrap();
            r.connection.execute_batch(sql).unwrap();
            drop(r);
            assert!(matches!(
                ChronosReceiver::open(f.path(), [7; 16], limits()),
                Err(ReceiverError::InvalidState)
            ));
        }
    }
    #[test]
    fn bounded_intent_refusal_does_not_mutate_and_owner_is_exclusive() {
        let f = Fixture::new();
        let mut config = limits();
        config.max_intent_bytes = 8;
        let mut r = ChronosReceiver::open(f.path(), [7; 16], config).unwrap();
        assert!(matches!(
            r.admit(
                EffectId::new([1; 16]),
                StateRevision::ZERO,
                TaskSpecV2::new("work")
            ),
            Err(ReceiverError::Capacity)
        ));
        assert_eq!(r.task_count(), 0);
        assert!(!r.needs_reopen());
        assert!(matches!(
            ChronosReceiver::open(f.path(), [7; 16], limits()),
            Err(ReceiverError::Storage(StoreError::Busy))
        ));
        drop(r);
        assert!(matches!(
            ChronosReceiver::open(f.path(), [8; 16], limits()),
            Err(ReceiverError::Storage(StoreError::OwnerMismatch))
        ));
    }

    #[test]
    fn claim_and_attempt_record_are_atomic_and_lost_commit_reply_cannot_reclaim() {
        for fault in [crate::Fault::BeforeCommit, crate::Fault::AfterCommit] {
            let f = Fixture::new();
            let mut r = f.open();
            let effect = EffectId::new([1; 16]);
            r.admit(effect, StateRevision::ZERO, TaskSpecV2::new("work"))
                .unwrap();
            let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
            let grant = r
                .authorize_worker(Agent::new("worker"), now, Duration::from_secs(60), 1)
                .unwrap();
            let before = r.revision();
            r.fault = Some(fault);
            assert!(
                r.claim(effect, before, &grant, now, Duration::from_secs(30))
                    .is_err()
            );
            assert!(r.needs_reopen());
            assert_eq!(r.revision(), before);
            assert_eq!(r.claim_usage().committed, 0);
            drop(r);
            let mut r = f.open();
            let grant = r
                .authorize_worker(Agent::new("worker"), now, Duration::from_secs(60), 1)
                .unwrap();
            let observed = r
                .claim(effect, r.revision(), &grant, now, Duration::from_secs(30))
                .unwrap();
            assert_eq!(observed.number, 1);
            assert_eq!(observed.replayed, fault == crate::Fault::AfterCommit);
            assert_eq!(
                observed.phase,
                if fault == crate::Fault::AfterCommit {
                    TaskPhase::Uncertain
                } else {
                    TaskPhase::Running
                }
            );
        }
    }

    #[test]
    fn admission_only_v1_upgrade_retains_receipts_and_missing_attempts_refuse_recovery() {
        let f = Fixture::new();
        let mut r = f.open();
        let effect = EffectId::new([1; 16]);
        let first = r
            .admit(effect, StateRevision::ZERO, TaskSpecV2::new("work"))
            .unwrap();
        r.connection
            .execute_batch("DROP TABLE receiver_settlements; ALTER TABLE receiver_meta DROP COLUMN settlements; DROP TABLE receiver_attempts; PRAGMA user_version=1;")
            .unwrap();
        drop(r);
        let mut r = f.open();
        assert_eq!(
            r.connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            r.admit(effect, StateRevision::ZERO, TaskSpecV2::new("work"))
                .unwrap()
                .receipt,
            first.receipt
        );
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let grant = r
            .authorize_worker(Agent::new("worker"), now, Duration::from_secs(60), 1)
            .unwrap();
        r.claim(effect, r.revision(), &grant, now, Duration::from_secs(30))
            .unwrap();
        r.connection
            .execute("DELETE FROM receiver_attempts", [])
            .unwrap();
        drop(r);
        assert!(matches!(
            ChronosReceiver::open(f.path(), [7; 16], limits()),
            Err(ReceiverError::InvalidState)
        ));
    }
}
