//! Single active local owner. No connector dispatch or distributed transaction.
#![forbid(unsafe_code)]
#![cfg(unix)]

pub mod agent_output;
pub mod agent_participation;
pub mod agent_prompt;
mod backup;
pub mod deliberation;
pub mod model_saga;
mod resource;
pub mod saga;
pub mod tool_continuation;
pub mod tool_output;
pub use backup::{OnlineBackupLimitsV1, OnlineBackupReceiptV1};
#[doc(hidden)]
pub use resource::OwnedResource;

use moira_core::persistence::{CodecError, CodecLimits, decode_checkpoint, encode_checkpoint};
use moira_core::saga::{SagaRefusalV1, persistence::SagaCodecErrorV1};
use moira_core::{
    CheckpointV1, CommandV1, Engine, Limits, RefusalV1, TransitionV1, WorkflowRevision,
};
use moira_host::{ApplyErrorV1, ApplyReceiptV1, CoordinatorStore};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, limits::Limit, params,
};
use std::{
    io::ErrorKind,
    path::Path,
    time::{Duration, Instant},
};

const APPLICATION_ID: i64 = 0x4d4f4952;
const SCHEMA: &str = "CREATE TABLE moira_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), initialized INTEGER NOT NULL CHECK(initialized IN (0,1)));
CREATE TABLE moira_state (slot INTEGER PRIMARY KEY CHECK(slot=1), revision BLOB NOT NULL CHECK(length(revision)=8), payload BLOB NOT NULL);";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Timing {
    Off,
    Process,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreLimits {
    pub codec: CodecLimits,
    pub max_database_pages: u32,
    pub timing: Timing,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreRevision(pub u64);

#[derive(Debug)]
pub enum StoreError {
    InvalidConfig,
    Busy,
    ForeignDatabase,
    OwnerMismatch,
    Schema,
    NeedsReopen,
    LostResource,
    Io(ErrorKind),
    Sqlite(ErrorCode),
    Codec(CodecError),
    SagaCodec(SagaCodecErrorV1),
    Refusal(RefusalV1),
    SagaRefusal(SagaRefusalV1),
    Conflict {
        expected: Option<StoreRevision>,
        actual: Option<StoreRevision>,
    },
    RevisionExhausted,
    Indeterminate,
    BackupExists,
    BackupIncomplete,
    BackupPublicationUncertain,
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Moira SQLite: {self:?}")
    }
}
impl std::error::Error for StoreError {}
impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        // Never propagate SQLite message strings, parameters, paths or contents.
        match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            Some(code) => Self::Sqlite(code),
            None => Self::Schema,
        }
    }
}
impl From<CodecError> for StoreError {
    fn from(e: CodecError) -> Self {
        Self::Codec(e)
    }
}
impl From<SagaCodecErrorV1> for StoreError {
    fn from(error: SagaCodecErrorV1) -> Self {
        Self::SagaCodec(error)
    }
}
fn io(error: std::io::Error) -> StoreError {
    StoreError::Io(error.kind())
}

/// Selected reopen integrity profile: SQLite's bounded quick check must report
/// exactly `ok` before any existing domain state is trusted or persistent
/// connection pragmas are changed. This detects structural page/index damage;
/// it is not an arbitrary-bit corruption or storage-device certification.
pub fn validate_sqlite_integrity(connection: &Connection) -> Result<(), StoreError> {
    let result: String = connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    if result != "ok" {
        return Err(StoreError::Schema);
    }
    Ok(())
}

#[derive(Debug)]
pub struct StoredCheckpoint {
    pub revision: StoreRevision,
    pub checkpoint: CheckpointV1,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Applied {
    pub revision: StoreRevision,
    pub transition: TransitionV1,
    /// Observation of an earlier committed transition, never fresh dispatch.
    pub replayed: bool,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub command_offers: u64,
    pub committed: u64,
    pub replayed: u64,
    pub refused: u64,
    pub conflicts: u64,
    pub indeterminate: u64,
    pub load_offers: u64,
    pub loaded: u64,
    pub load_refused: u64,
    pub commit_timed_samples: u64,
    pub commit_untimed_samples: u64,
    pub commit_service_total: Option<Duration>,
    pub max_commit_service: Option<Duration>,
    pub overflowed: bool,
}
#[doc(hidden)]
pub fn count(value: &mut u64, overflow: &mut bool) {
    if let Some(next) = value.checked_add(1) {
        *value = next;
    } else {
        *overflow = true;
    }
}

/// Database must be a regular file in a trusted local directory. The advisory
/// sidecar lock coordinates cooperating hosts, not malicious processes or remote
/// filesystems. First qualification target: the current macOS host.
pub struct SqliteStore {
    // Field order keeps SQLite closed before active ownership is released.
    connection: Connection,
    resource: OwnedResource,
    owner: [u8; 16],
    limits: StoreLimits,
    engine: Engine,
    store_revision: Option<StoreRevision>,
    healthy: bool,
    usage: Usage,
    #[cfg(test)]
    fault: Option<Fault>,
}
impl std::fmt::Debug for SqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SqliteStore(<redacted>)")
    }
}

impl SqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        limits: StoreLimits,
    ) -> Result<Self, StoreError> {
        let initial = Engine::try_new(Limits {
            max_workflows: limits.codec.max_workflows,
        })
        .map_err(|_| StoreError::InvalidConfig)?;
        encode_checkpoint(&initial.checkpoint(), limits.codec)
            .map_err(|_| StoreError::InvalidConfig)?;
        let record_limit: i32 = limits
            .codec
            .max_bytes
            .checked_add(1024)
            .and_then(|n| n.try_into().ok())
            .ok_or(StoreError::InvalidConfig)?;
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
            let mut stmt = connection.prepare("SELECT name FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 3")?;
            stmt.query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let fresh = app == 0 && version == 0 && names.is_empty();
        if !fresh
            && (app != APPLICATION_ID || version != 1 || names != ["moira_meta", "moira_state"])
        {
            return Err(StoreError::ForeignDatabase);
        }
        // Check ownership before making persistent pragma changes to an existing DB.
        if !fresh {
            read_state(&connection, owner, limits.codec)?;
        }
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
            return Err(StoreError::InvalidConfig);
        }
        if fresh {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA)?;
            tx.execute("INSERT INTO moira_meta VALUES (1,?1,0)", [owner.as_slice()])?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
            resource.sync_parent()?;
        }
        let loaded = read_state(&connection, owner, limits.codec)?;
        let (engine, store_revision) = match loaded {
            Some(state) => (
                Engine::recover(state.checkpoint)
                    .map_err(|e| StoreError::Codec(CodecError::Recovery(e)))?,
                Some(state.revision),
            ),
            None => (initial, None),
        };
        let store = Self {
            connection,
            resource,
            owner,
            limits,
            engine,
            store_revision,
            healthy: true,
            usage: Usage::default(),
            #[cfg(test)]
            fault: None,
        };
        store.check_identity()?;
        Ok(store)
    }

    /// Last confirmed domain revision; does not un-fence an indeterminate owner.
    pub fn revision(&self) -> WorkflowRevision {
        self.engine.revision()
    }
    pub fn needs_reopen(&self) -> bool {
        !self.healthy
    }
    pub fn usage(&self) -> Usage {
        self.usage
    }
    /// Publishes a bounded integrity-checked SQLite online snapshot while this
    /// single owner remains open. The destination must not already exist.
    pub fn backup_online(
        &mut self,
        destination: impl AsRef<Path>,
        limits: OnlineBackupLimitsV1,
    ) -> Result<OnlineBackupReceiptV1, StoreError> {
        self.guard()?;
        let result = backup::publish_online_backup(
            &self.connection,
            &self.resource,
            destination.as_ref(),
            self.limits.max_database_pages,
            limits,
        );
        if let Err(error) = self.check_identity() {
            self.healthy = false;
            return Err(error);
        }
        Ok(OnlineBackupReceiptV1 {
            storage_revision: self.store_revision,
            page_count: result?,
        })
    }
    fn check_identity(&self) -> Result<(), StoreError> {
        self.resource.check()
    }
    fn guard(&mut self) -> Result<(), StoreError> {
        if !self.healthy {
            return Err(StoreError::NeedsReopen);
        }
        if let Err(e) = self.check_identity() {
            self.healthy = false;
            return Err(e);
        }
        Ok(())
    }
    pub fn load(&mut self) -> Result<Option<StoredCheckpoint>, StoreError> {
        let result = (|| {
            self.guard()?;
            let loaded =
                read_state(&self.connection, self.owner, self.limits.codec).inspect_err(|_| {
                    self.healthy = false;
                })?;
            let actual = loaded.as_ref().map(|s| s.revision);
            if actual != self.store_revision {
                self.healthy = false;
                return Err(StoreError::Conflict {
                    expected: self.store_revision,
                    actual,
                });
            }
            Ok(loaded)
        })();
        count(&mut self.usage.load_offers, &mut self.usage.overflowed);
        let counter = if result.is_ok() {
            &mut self.usage.loaded
        } else {
            &mut self.usage.load_refused
        };
        count(counter, &mut self.usage.overflowed);
        result
    }
    pub fn apply(
        &mut self,
        expected: WorkflowRevision,
        command: CommandV1,
    ) -> Result<Applied, StoreError> {
        let result = self.apply_inner(expected, command);
        let u = &mut self.usage;
        count(&mut u.command_offers, &mut u.overflowed);
        match &result {
            Ok(result) if result.replayed => count(&mut u.replayed, &mut u.overflowed),
            Ok(_) => count(&mut u.committed, &mut u.overflowed),
            Err(StoreError::Indeterminate) => count(&mut u.indeterminate, &mut u.overflowed),
            Err(error) => {
                count(&mut u.refused, &mut u.overflowed);
                if matches!(error, StoreError::Conflict { .. }) {
                    count(&mut u.conflicts, &mut u.overflowed);
                }
            }
        }
        result
    }
    fn apply_inner(
        &mut self,
        expected: WorkflowRevision,
        command: CommandV1,
    ) -> Result<Applied, StoreError> {
        self.guard()?;
        let mut candidate = Engine::recover(self.engine.checkpoint())
            .map_err(|e| StoreError::Codec(CodecError::Recovery(e)))?;
        let transition = candidate
            .apply(expected, command)
            .map_err(StoreError::Refusal)?;
        let actual = read_revision(&self.connection).inspect_err(|_| {
            self.healthy = false;
        })?;
        if actual != self.store_revision {
            self.healthy = false;
            return Err(StoreError::Conflict {
                expected: self.store_revision,
                actual,
            });
        }
        if candidate.revision() == self.engine.revision() {
            return Ok(Applied {
                revision: actual.ok_or(StoreError::Schema)?,
                transition,
                replayed: true,
            });
        }
        let bytes = encode_checkpoint(&candidate.checkpoint(), self.limits.codec)?;
        let next = StoreRevision(match self.store_revision {
            Some(revision) => revision
                .0
                .checked_add(1)
                .ok_or(StoreError::RevisionExhausted)?,
            None => 0,
        });
        let started = match self.limits.timing {
            Timing::Off => None,
            Timing::Process => std::panic::catch_unwind(Instant::now).ok(),
        };
        let result = self.commit_bytes(next, bytes.as_bytes());
        record_timing(&mut self.usage, started);
        result?;
        self.engine = candidate;
        self.store_revision = Some(next);
        Ok(Applied {
            revision: next,
            transition,
            replayed: false,
        })
    }
    fn commit_bytes(&mut self, next: StoreRevision, bytes: &[u8]) -> Result<(), StoreError> {
        // A storage failure fences the connection unless an explicit rollback
        // has acknowledged that this attempt left the previous state intact.
        self.healthy = false;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual = read_revision(&tx)?;
        if actual != self.store_revision {
            self.healthy = false;
            return Err(StoreError::Conflict {
                expected: self.store_revision,
                actual,
            });
        }
        tx.execute("INSERT INTO moira_state VALUES (1,?1,?2) ON CONFLICT(slot) DO UPDATE SET revision=excluded.revision,payload=excluded.payload", params![next.0.to_be_bytes().as_slice(), bytes])?;
        tx.execute("UPDATE moira_meta SET initialized=1 WHERE slot=1", [])?;
        #[cfg(test)]
        if self.fault == Some(Fault::BeforeCommit) {
            self.fault = None;
            tx.rollback()?;
            self.healthy = true;
            return Err(StoreError::Io(ErrorKind::Interrupted));
        }
        self.healthy = false;
        tx.commit().map_err(|_| StoreError::Indeterminate)?;
        #[cfg(test)]
        if self.fault == Some(Fault::AfterCommit) {
            self.fault = None;
            return Err(StoreError::Indeterminate);
        }
        self.check_identity()
            .map_err(|_| StoreError::Indeterminate)?;
        self.healthy = true;
        Ok(())
    }
}

impl CoordinatorStore for SqliteStore {
    type Error = StoreError;

    fn revision(&self) -> WorkflowRevision {
        self.revision()
    }

    fn needs_reopen(&self) -> bool {
        self.needs_reopen()
    }

    fn workflow(&self, workflow: moira_core::WorkflowId) -> Option<moira_core::WorkflowSnapshotV1> {
        self.engine.workflow(&workflow).copied()
    }

    fn apply(
        &mut self,
        expected: WorkflowRevision,
        command: CommandV1,
    ) -> Result<ApplyReceiptV1, ApplyErrorV1<Self::Error>> {
        match SqliteStore::apply(self, expected, command) {
            Ok(applied) => Ok(ApplyReceiptV1 {
                storage_revision: applied.revision.0,
                transition: applied.transition,
                replayed: applied.replayed,
            }),
            Err(StoreError::Refusal(refusal)) => Err(ApplyErrorV1::Domain(refusal)),
            Err(error) => Err(ApplyErrorV1::Backend(error)),
        }
    }
}

impl moira_host::AttemptReceiptStore for SqliteStore {
    fn workflow_for_attempt(
        &self,
        task: moira_core::ChronosTaskRefV1,
        attempt: u32,
    ) -> Result<Option<moira_core::WorkflowSnapshotV1>, moira_core::AttemptReceiptConflict> {
        self.engine.workflow_for_attempt(task, attempt)
    }
}

fn read_revision(connection: &Connection) -> Result<Option<StoreRevision>, StoreError> {
    let bytes: Option<Vec<u8>> = connection
        .query_row("SELECT revision FROM moira_state WHERE slot=1", [], |r| {
            r.get(0)
        })
        .optional()?;
    bytes
        .map(|bytes| {
            Ok(StoreRevision(u64::from_be_bytes(
                bytes.try_into().map_err(|_| StoreError::Schema)?,
            )))
        })
        .transpose()
}
fn read_state(
    connection: &Connection,
    owner: [u8; 16],
    limits: CodecLimits,
) -> Result<Option<StoredCheckpoint>, StoreError> {
    let (retained, initialized): (Vec<u8>, i64) = connection.query_row(
        "SELECT owner,initialized FROM moira_meta WHERE slot=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if retained != owner {
        return Err(StoreError::OwnerMismatch);
    }
    let revision = read_revision(connection)?;
    if !matches!(initialized, 0 | 1) || (initialized == 1) != revision.is_some() {
        return Err(StoreError::Schema);
    }
    let Some(revision) = revision else {
        return Ok(None);
    };
    let mut statement = connection.prepare("SELECT payload FROM moira_state WHERE slot=1")?;
    let mut rows = statement.query([])?;
    let row = rows.next()?.ok_or(StoreError::Schema)?;
    let value = row.get_ref(0)?.as_blob().map_err(|_| StoreError::Schema)?;
    let checkpoint = decode_checkpoint(value, limits)?;
    Ok(Some(StoredCheckpoint {
        revision,
        checkpoint,
    }))
}
#[doc(hidden)]
pub fn record_timing(usage: &mut Usage, started: Option<Instant>) {
    let elapsed = started.and_then(|start| {
        std::panic::catch_unwind(Instant::now)
            .ok()?
            .checked_duration_since(start)
    });
    if let Some(elapsed) = elapsed {
        count(&mut usage.commit_timed_samples, &mut usage.overflowed);
        let previous = usage.commit_service_total.unwrap_or_default();
        usage.commit_service_total = Some(previous.checked_add(elapsed).unwrap_or_else(|| {
            usage.overflowed = true;
            Duration::MAX
        }));
        usage.max_commit_service = Some(usage.max_commit_service.unwrap_or_default().max(elapsed));
    } else {
        count(&mut usage.commit_untimed_samples, &mut usage.overflowed);
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum Fault {
    BeforeCommit,
    AfterCommit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use moira_core::{AgoraMessageRefV1, ParticipantRefV1};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::{
        fs::{self, File},
        path::PathBuf,
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "moira-sqlite-unit-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> PathBuf {
            self.0.join("state.db")
        }
        fn open(&self, timing: Timing) -> SqliteStore {
            SqliteStore::open(
                self.path(),
                [7; 16],
                StoreLimits {
                    codec: CodecLimits {
                        max_bytes: 8192,
                        max_workflows: 2,
                    },
                    max_database_pages: 64,
                    timing,
                },
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for name in [
                "state.db",
                "state.db-journal",
                "state.db.owner.lock",
                "old.db",
                "old.lock",
            ] {
                let _ = fs::remove_file(self.0.join(name));
            }
            let _ = fs::remove_dir(&self.0);
        }
    }
    fn command() -> CommandV1 {
        CommandV1::AdmitMessage {
            message: AgoraMessageRefV1::new([1; 16], 2, 3),
            recipient: ParticipantRefV1::new([4; 16]),
        }
    }

    #[test]
    fn acknowledged_rollback_keeps_identity_available_but_lost_commit_reply_fences() {
        let f = Fixture::new();
        let mut s = f.open(Timing::Off);
        s.fault = Some(Fault::BeforeCommit);
        assert!(matches!(
            s.apply(WorkflowRevision(0), command()),
            Err(StoreError::Io(ErrorKind::Interrupted))
        ));
        assert!(!s.needs_reopen());
        assert!(s.load().unwrap().is_none());
        assert_eq!(s.revision(), WorkflowRevision(0));
        s.fault = Some(Fault::AfterCommit);
        assert!(matches!(
            s.apply(WorkflowRevision(0), command()),
            Err(StoreError::Indeterminate)
        ));
        assert!(s.needs_reopen());
        assert_eq!(s.revision(), WorkflowRevision(0));
        assert!(matches!(s.load(), Err(StoreError::NeedsReopen)));
        assert!(matches!(
            s.apply(WorkflowRevision(0), command()),
            Err(StoreError::NeedsReopen)
        ));
        assert_eq!(s.usage().committed, 0);
        assert_eq!(s.usage().indeterminate, 1);
        assert_eq!(s.usage().commit_untimed_samples, 2);
        drop(s);
        let mut s = f.open(Timing::Off);
        assert_eq!(s.revision(), WorkflowRevision(1));
        let replay = s.apply(s.revision(), command()).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.revision, StoreRevision(0));
    }

    #[test]
    fn resource_or_owner_lock_replacement_fences_without_writing() {
        for lock in [false, true] {
            let f = Fixture::new();
            let mut s = f.open(Timing::Off);
            let target = if lock {
                s.resource.lock_path.clone()
            } else {
                f.path()
            };
            let old = f.0.join(if lock { "old.lock" } else { "old.db" });
            fs::rename(&target, old).unwrap();
            File::create(&target).unwrap();
            assert!(matches!(
                s.apply(WorkflowRevision(0), command()),
                Err(StoreError::LostResource)
            ));
            assert!(s.needs_reopen());
            assert_eq!(s.revision(), WorkflowRevision(0));
            assert_eq!(fs::metadata(target).unwrap().len(), 0);
        }
    }

    #[test]
    fn lost_state_row_is_not_a_new_store_and_invalid_load_fences() {
        let f = Fixture::new();
        let mut s = f.open(Timing::Off);
        s.apply(WorkflowRevision(0), command()).unwrap();
        s.connection.execute("DELETE FROM moira_state", []).unwrap();
        assert!(matches!(s.load(), Err(StoreError::Schema)));
        assert!(s.needs_reopen());
        assert!(matches!(
            s.apply(s.revision(), command()),
            Err(StoreError::NeedsReopen)
        ));
        drop(s);
        assert!(matches!(
            SqliteStore::open(
                f.path(),
                [7; 16],
                StoreLimits {
                    codec: CodecLimits {
                        max_bytes: 8192,
                        max_workflows: 2
                    },
                    max_database_pages: 64,
                    timing: Timing::Off,
                }
            ),
            Err(StoreError::Schema)
        ));
    }

    #[test]
    fn instrumentation_is_optional_content_free_and_saturating() {
        let f = Fixture::new();
        let mut s = f.open(Timing::Process);
        s.apply(WorkflowRevision(0), command()).unwrap();
        let before = s.usage();
        assert_eq!(before.commit_timed_samples, 1);
        assert!(before.commit_service_total.is_some());
        assert!(before.max_commit_service.is_some());
        s.usage.command_offers = u64::MAX;
        assert!(s.apply(s.revision(), command()).unwrap().replayed);
        assert_eq!(s.usage().command_offers, u64::MAX);
        assert!(s.usage().overflowed);
        assert_eq!(s.usage().commit_timed_samples, 1);
        assert!(!format!("{:?}", s.usage()).contains("[1, 1"));
        assert_eq!(format!("{s:?}"), "SqliteStore(<redacted>)");
    }
}
