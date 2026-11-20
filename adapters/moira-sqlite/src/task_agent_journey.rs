//! Durable single-owner store for task-to-agent journey admission.
//!
//! This adapter persists only Moira's canonical admission checkpoint. It does
//! not claim Chronos work, create rooms, open agent sessions, or dispatch peer
//! effects.

use crate::{StoreRevision, resource::OwnedResource, validate_sqlite_integrity};
use moira_core::task_agent_journey::{
    CheckpointV1, Engine, JourneyIdV1, JourneySnapshotV1, LimitsV1, OwnerIdV1,
    ParticipationBlueprintV1, PreviewIdV1, PreviewSnapshotV1, RefusalV1, TemplateV1,
    persistence::{CodecErrorV1, CodecLimitsV1, decode, encode},
};
use moira_host::{
    TaskAgentJourneyApplyErrorV1, TaskAgentJourneyApplyReceiptV1, TaskAgentJourneyApplyV1,
    TaskAgentJourneyCensusV1, TaskAgentJourneyMutationV1, TaskAgentJourneyStore,
};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, limits::Limit, params,
};
use std::{io::ErrorKind, path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4d54414a;
const SCHEMA: &str = "CREATE TABLE task_agent_journey_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16));
CREATE TABLE task_agent_journey_state (slot INTEGER PRIMARY KEY CHECK(slot=1), storage_revision BLOB NOT NULL CHECK(length(storage_revision)=8), payload BLOB NOT NULL);";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskAgentJourneyStoreLimitsV1 {
    pub codec: CodecLimitsV1,
    pub domain: LimitsV1,
    pub max_database_pages: u32,
}

#[derive(Debug)]
pub enum TaskAgentJourneyStoreErrorV1 {
    InvalidConfig,
    Busy,
    ForeignDatabase,
    OwnerMismatch,
    Schema,
    NeedsReopen,
    LostResource,
    Io(ErrorKind),
    Sqlite(ErrorCode),
    Codec(CodecErrorV1),
    Refusal(RefusalV1),
    Conflict {
        expected: Option<StoreRevision>,
        actual: StoreRevision,
    },
    RevisionExhausted,
    Indeterminate,
}

impl std::fmt::Display for TaskAgentJourneyStoreErrorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Moira task-agent journey SQLite: {self:?}")
    }
}

impl std::error::Error for TaskAgentJourneyStoreErrorV1 {}

impl From<rusqlite::Error> for TaskAgentJourneyStoreErrorV1 {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            Some(code) => Self::Sqlite(code),
            None => Self::Schema,
        }
    }
}

impl From<CodecErrorV1> for TaskAgentJourneyStoreErrorV1 {
    fn from(error: CodecErrorV1) -> Self {
        Self::Codec(error)
    }
}

impl From<RefusalV1> for TaskAgentJourneyStoreErrorV1 {
    fn from(error: RefusalV1) -> Self {
        Self::Refusal(error)
    }
}

pub struct TaskAgentJourneySqliteStore {
    connection: Connection,
    resource: OwnedResource,
    owner: OwnerIdV1,
    limits: TaskAgentJourneyStoreLimitsV1,
    engine: Engine,
    storage_revision: StoreRevision,
    healthy: bool,
}

impl std::fmt::Debug for TaskAgentJourneySqliteStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TaskAgentJourneySqliteStore(<redacted>)")
    }
}

impl TaskAgentJourneySqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: OwnerIdV1,
        limits: TaskAgentJourneyStoreLimitsV1,
        templates: impl IntoIterator<Item = TemplateV1>,
    ) -> Result<Self, TaskAgentJourneyStoreErrorV1> {
        let initial = Engine::try_new(owner, limits.domain, templates)?;
        let initial_checkpoint = initial.checkpoint();
        let initial_bytes = encode(&initial_checkpoint, limits.codec)
            .map_err(|_| TaskAgentJourneyStoreErrorV1::InvalidConfig)?;
        let record_limit: i32 = limits
            .codec
            .max_bytes
            .checked_add(1_024)
            .and_then(|value| value.try_into().ok())
            .ok_or(TaskAgentJourneyStoreErrorV1::InvalidConfig)?;
        let resource =
            OwnedResource::open(path, limits.max_database_pages).map_err(map_resource)?;
        let mut connection = Connection::open_with_flags(
            resource.path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::ZERO)?;
        connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, record_limit)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON;")?;
        validate_sqlite_integrity(&connection).map_err(map_resource)?;
        let application: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let names = schema_names(&connection)?;
        let fresh = application == 0 && version == 0 && names.is_empty();
        if !fresh
            && (application != APPLICATION_ID
                || version != 1
                || names != ["task_agent_journey_meta", "task_agent_journey_state"])
        {
            return Err(TaskAgentJourneyStoreErrorV1::ForeignDatabase);
        }
        if !fresh {
            let (_, checkpoint) = read_state(&connection, owner, limits.codec)?;
            validate_profile(&checkpoint, &initial_checkpoint)?;
        }
        let journal: String =
            connection.query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))?;
        connection.execute_batch(
            "PRAGMA synchronous=EXTRA; PRAGMA fullfsync=ON; PRAGMA temp_store=MEMORY;",
        )?;
        connection.pragma_update(None, "max_page_count", limits.max_database_pages)?;
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
        let pages: u32 = connection.pragma_query_value(None, "max_page_count", |row| row.get(0))?;
        let synchronous: i64 =
            connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
        let full_sync: i64 = connection.pragma_query_value(None, "fullfsync", |row| row.get(0))?;
        if journal != "delete"
            || synchronous != 3
            || full_sync != 1
            || page_size != 4_096
            || pages > limits.max_database_pages
        {
            return Err(TaskAgentJourneyStoreErrorV1::InvalidConfig);
        }
        if fresh {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute(
                "INSERT INTO task_agent_journey_meta VALUES (1,?1)",
                params![owner.0.as_slice()],
            )?;
            transaction.execute(
                "INSERT INTO task_agent_journey_state VALUES (1,?1,?2)",
                params![0_u64.to_be_bytes().as_slice(), initial_bytes.as_slice()],
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction
                .commit()
                .map_err(|_| TaskAgentJourneyStoreErrorV1::Indeterminate)?;
            resource.sync_parent().map_err(map_resource)?;
        }
        let (storage_revision, checkpoint) = read_state(&connection, owner, limits.codec)?;
        validate_profile(&checkpoint, &initial_checkpoint)?;
        let engine = Engine::recover(checkpoint)?;
        resource.check().map_err(map_resource)?;
        Ok(Self {
            connection,
            resource,
            owner,
            limits,
            engine,
            storage_revision,
            healthy: true,
        })
    }

    pub const fn storage_revision(&self) -> StoreRevision {
        self.storage_revision
    }

    pub const fn needs_reopen(&self) -> bool {
        !self.healthy
    }

    fn apply_mutation(
        &mut self,
        expected: Option<StoreRevision>,
        mutation: TaskAgentJourneyMutationV1,
    ) -> Result<TaskAgentJourneyApplyReceiptV1, TaskAgentJourneyStoreErrorV1> {
        self.guard()?;
        if expected.unwrap_or(StoreRevision(0)) != self.storage_revision {
            return Err(TaskAgentJourneyStoreErrorV1::Conflict {
                expected,
                actual: self.storage_revision,
            });
        }
        let mut candidate = Engine::recover(self.engine.checkpoint())?;
        let outcome = match mutation {
            TaskAgentJourneyMutationV1::Preview {
                request,
                observed_at,
            } => TaskAgentJourneyApplyV1::Preview(candidate.preview(request, observed_at)?),
            TaskAgentJourneyMutationV1::Instantiate {
                request,
                observed_at,
            } => TaskAgentJourneyApplyV1::Instantiate(candidate.instantiate(request, observed_at)?),
            TaskAgentJourneyMutationV1::BindTaskAttempt { journey, receipt } => {
                TaskAgentJourneyApplyV1::TaskAttempt(candidate.bind_task_attempt(journey, receipt)?)
            }
            TaskAgentJourneyMutationV1::BindRoom { journey, receipt } => {
                TaskAgentJourneyApplyV1::Room(candidate.bind_room(journey, receipt)?)
            }
            TaskAgentJourneyMutationV1::PrepareParticipation(preparation) => {
                TaskAgentJourneyApplyV1::ParticipationPrepared(candidate.prepare_participation(
                    preparation.journey,
                    preparation.workflow,
                    preparation.deliberation,
                    preparation.binding,
                )?)
            }
        };
        let changed = candidate.checkpoint() != self.engine.checkpoint();
        let actual = read_revision(&self.connection)?;
        if actual != self.storage_revision {
            self.healthy = false;
            return Err(TaskAgentJourneyStoreErrorV1::Conflict {
                expected: Some(self.storage_revision),
                actual,
            });
        }
        if !changed {
            return Ok(TaskAgentJourneyApplyReceiptV1 {
                storage_revision: self.storage_revision.0,
                outcome,
            });
        }
        let encoded = encode(&candidate.checkpoint(), self.limits.codec)?;
        let next = StoreRevision(
            self.storage_revision
                .0
                .checked_add(1)
                .ok_or(TaskAgentJourneyStoreErrorV1::RevisionExhausted)?,
        );
        self.healthy = false;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let retained = read_revision(&transaction)?;
        if retained != self.storage_revision {
            return Err(TaskAgentJourneyStoreErrorV1::Conflict {
                expected: Some(self.storage_revision),
                actual: retained,
            });
        }
        transaction.execute(
            "UPDATE task_agent_journey_state SET storage_revision=?1,payload=?2 WHERE slot=1",
            params![next.0.to_be_bytes().as_slice(), encoded.as_slice()],
        )?;
        transaction
            .commit()
            .map_err(|_| TaskAgentJourneyStoreErrorV1::Indeterminate)?;
        self.resource
            .check()
            .map_err(|_| TaskAgentJourneyStoreErrorV1::Indeterminate)?;
        self.engine = candidate;
        self.storage_revision = next;
        self.healthy = true;
        Ok(TaskAgentJourneyApplyReceiptV1 {
            storage_revision: next.0,
            outcome,
        })
    }

    fn guard(&mut self) -> Result<(), TaskAgentJourneyStoreErrorV1> {
        if !self.healthy {
            return Err(TaskAgentJourneyStoreErrorV1::NeedsReopen);
        }
        if let Err(error) = self.resource.check() {
            self.healthy = false;
            return Err(map_resource(error));
        }
        Ok(())
    }
}

impl TaskAgentJourneyStore for TaskAgentJourneySqliteStore {
    type Error = TaskAgentJourneyStoreErrorV1;

    fn owner(&self) -> OwnerIdV1 {
        self.owner
    }

    fn owner_revision(&self) -> u64 {
        self.engine.owner_revision()
    }

    fn storage_revision(&self) -> u64 {
        self.storage_revision.0
    }

    fn needs_reopen(&self) -> bool {
        self.needs_reopen()
    }

    fn catalogue(&self) -> Vec<TemplateV1> {
        self.engine.checkpoint().templates
    }

    fn census(&self) -> TaskAgentJourneyCensusV1 {
        let checkpoint = self.engine.checkpoint();
        TaskAgentJourneyCensusV1 {
            templates: u64::try_from(checkpoint.templates.len()).unwrap_or(u64::MAX),
            previews: u64::try_from(checkpoint.previews.len()).unwrap_or(u64::MAX),
            journeys: u64::try_from(checkpoint.journeys.len()).unwrap_or(u64::MAX),
        }
    }

    fn preview(&self, id: PreviewIdV1) -> Option<PreviewSnapshotV1> {
        self.engine.preview_snapshot(id).copied()
    }

    fn journey(&self, id: JourneyIdV1) -> Option<JourneySnapshotV1> {
        self.engine.journey(id).copied()
    }

    fn participation_blueprint(&self, journey: JourneyIdV1) -> Option<ParticipationBlueprintV1> {
        self.engine.participation_blueprint(journey).copied()
    }

    fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        mutation: TaskAgentJourneyMutationV1,
    ) -> Result<TaskAgentJourneyApplyReceiptV1, TaskAgentJourneyApplyErrorV1<Self::Error>> {
        match self.apply_mutation(expected_storage_revision.map(StoreRevision), mutation) {
            Ok(receipt) => Ok(receipt),
            Err(TaskAgentJourneyStoreErrorV1::Refusal(value)) => {
                Err(TaskAgentJourneyApplyErrorV1::Domain(value))
            }
            Err(TaskAgentJourneyStoreErrorV1::Conflict { expected, actual }) => {
                Err(TaskAgentJourneyApplyErrorV1::Conflict {
                    expected: expected.map(|revision| revision.0),
                    actual: actual.0,
                })
            }
            Err(error) => Err(TaskAgentJourneyApplyErrorV1::Backend(error)),
        }
    }
}

fn schema_names(connection: &Connection) -> Result<Vec<String>, TaskAgentJourneyStoreErrorV1> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 3",
    )?;
    Ok(statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn read_revision(connection: &Connection) -> Result<StoreRevision, TaskAgentJourneyStoreErrorV1> {
    let bytes: Option<Vec<u8>> = connection
        .query_row(
            "SELECT storage_revision FROM task_agent_journey_state WHERE slot=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let bytes = bytes.ok_or(TaskAgentJourneyStoreErrorV1::Schema)?;
    Ok(StoreRevision(u64::from_be_bytes(
        bytes
            .try_into()
            .map_err(|_| TaskAgentJourneyStoreErrorV1::Schema)?,
    )))
}

fn read_state(
    connection: &Connection,
    owner: OwnerIdV1,
    codec: CodecLimitsV1,
) -> Result<(StoreRevision, CheckpointV1), TaskAgentJourneyStoreErrorV1> {
    let retained_owner: Vec<u8> = connection.query_row(
        "SELECT owner FROM task_agent_journey_meta WHERE slot=1",
        [],
        |row| row.get(0),
    )?;
    if retained_owner != owner.0 {
        return Err(TaskAgentJourneyStoreErrorV1::OwnerMismatch);
    }
    let revision = read_revision(connection)?;
    let payload: Vec<u8> = connection.query_row(
        "SELECT payload FROM task_agent_journey_state WHERE slot=1",
        [],
        |row| row.get(0),
    )?;
    let checkpoint = decode(&payload, codec)?;
    if checkpoint != Engine::recover(checkpoint.clone())?.checkpoint() {
        return Err(TaskAgentJourneyStoreErrorV1::Schema);
    }
    Ok((revision, checkpoint))
}

fn validate_profile(
    retained: &CheckpointV1,
    configured: &CheckpointV1,
) -> Result<(), TaskAgentJourneyStoreErrorV1> {
    if retained.owner != configured.owner {
        return Err(TaskAgentJourneyStoreErrorV1::OwnerMismatch);
    }
    if retained.limits != configured.limits || retained.templates != configured.templates {
        return Err(TaskAgentJourneyStoreErrorV1::InvalidConfig);
    }
    Ok(())
}

fn map_resource(error: crate::StoreError) -> TaskAgentJourneyStoreErrorV1 {
    match error {
        crate::StoreError::Busy => TaskAgentJourneyStoreErrorV1::Busy,
        crate::StoreError::ForeignDatabase => TaskAgentJourneyStoreErrorV1::ForeignDatabase,
        crate::StoreError::OwnerMismatch => TaskAgentJourneyStoreErrorV1::OwnerMismatch,
        crate::StoreError::NeedsReopen => TaskAgentJourneyStoreErrorV1::NeedsReopen,
        crate::StoreError::LostResource => TaskAgentJourneyStoreErrorV1::LostResource,
        crate::StoreError::Io(kind) => TaskAgentJourneyStoreErrorV1::Io(kind),
        crate::StoreError::Sqlite(code) => TaskAgentJourneyStoreErrorV1::Sqlite(code),
        _ => TaskAgentJourneyStoreErrorV1::Schema,
    }
}
