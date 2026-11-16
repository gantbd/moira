//! Separate durable owner for the model-proposal saga profile.
//!
//! This store has its own SQLite application id and schema. It never opens or
//! migrates the legacy coordinator or closed-saga databases.

use crate::{StoreRevision, resource::OwnedResource, validate_sqlite_integrity};
use moira_core::WorkflowId;
use moira_core::model_saga::{
    CheckpointV1, CommandV1, Engine, LimitsV1, RefusalV1, SagaSnapshotV1, TransitionV1,
    persistence::{CodecErrorV1, CodecLimitsV1, decode, encode},
};
use moira_host::{ModelSagaApplyErrorV1, ModelSagaApplyReceiptV1, ModelSagaStore};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, limits::Limit, params,
};
use std::{io::ErrorKind, path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4d4d5053;
const SCHEMA: &str = "CREATE TABLE model_saga_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), profile BLOB NOT NULL CHECK(length(profile)=24), initialized INTEGER NOT NULL CHECK(initialized IN (0,1)));
CREATE TABLE model_saga_state (slot INTEGER PRIMARY KEY CHECK(slot=1), storage_revision BLOB NOT NULL CHECK(length(storage_revision)=8), payload BLOB NOT NULL);";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelSagaStoreLimitsV1 {
    pub codec: CodecLimitsV1,
    pub saga: LimitsV1,
    pub max_database_pages: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSagaAppliedV1 {
    pub storage_revision: StoreRevision,
    pub transition: TransitionV1,
    pub replayed: bool,
}

#[derive(Debug)]
pub enum ModelSagaStoreErrorV1 {
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
        actual: Option<StoreRevision>,
    },
    RevisionExhausted,
    Indeterminate,
}

impl std::fmt::Display for ModelSagaStoreErrorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Moira model saga SQLite: {self:?}")
    }
}

impl std::error::Error for ModelSagaStoreErrorV1 {}

impl From<rusqlite::Error> for ModelSagaStoreErrorV1 {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            Some(code) => Self::Sqlite(code),
            None => Self::Schema,
        }
    }
}

impl From<CodecErrorV1> for ModelSagaStoreErrorV1 {
    fn from(error: CodecErrorV1) -> Self {
        Self::Codec(error)
    }
}

impl From<RefusalV1> for ModelSagaStoreErrorV1 {
    fn from(error: RefusalV1) -> Self {
        Self::Refusal(error)
    }
}

pub struct ModelSagaSqliteStore {
    connection: Connection,
    resource: OwnedResource,
    limits: ModelSagaStoreLimitsV1,
    engine: Engine,
    storage_revision: Option<StoreRevision>,
    healthy: bool,
}

impl std::fmt::Debug for ModelSagaSqliteStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ModelSagaSqliteStore(<redacted>)")
    }
}

impl ModelSagaSqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        limits: ModelSagaStoreLimitsV1,
    ) -> Result<Self, ModelSagaStoreErrorV1> {
        let initial = Engine::try_new(limits.saga)?;
        encode(&initial.checkpoint(), limits.codec)
            .map_err(|_| ModelSagaStoreErrorV1::InvalidConfig)?;
        let record_limit: i32 = limits
            .codec
            .max_bytes
            .checked_add(1_024)
            .and_then(|value| value.try_into().ok())
            .ok_or(ModelSagaStoreErrorV1::InvalidConfig)?;
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
        let names = {
            let mut statement = connection.prepare(
                "SELECT name FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 3",
            )?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let fresh = application == 0 && version == 0 && names.is_empty();
        if !fresh
            && (application != APPLICATION_ID
                || version != 1
                || names != ["model_saga_meta", "model_saga_state"])
        {
            return Err(ModelSagaStoreErrorV1::ForeignDatabase);
        }
        if !fresh {
            read_state(&connection, owner, limits)?;
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
            return Err(ModelSagaStoreErrorV1::InvalidConfig);
        }
        if fresh {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute(
                "INSERT INTO model_saga_meta VALUES (1,?1,?2,0)",
                params![owner.as_slice(), profile(limits.saga)],
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction
                .commit()
                .map_err(|_| ModelSagaStoreErrorV1::Indeterminate)?;
            resource.sync_parent().map_err(map_resource)?;
        }
        let loaded = read_state(&connection, owner, limits)?;
        let (engine, storage_revision) = if let Some((revision, checkpoint)) = loaded {
            (Engine::recover(checkpoint)?, Some(revision))
        } else {
            (initial, None)
        };
        resource.check().map_err(map_resource)?;
        Ok(Self {
            connection,
            resource,
            limits,
            engine,
            storage_revision,
            healthy: true,
        })
    }

    pub const fn storage_revision(&self) -> Option<StoreRevision> {
        self.storage_revision
    }

    pub const fn needs_reopen(&self) -> bool {
        !self.healthy
    }

    pub fn saga(&self, workflow: WorkflowId) -> Option<&SagaSnapshotV1> {
        self.engine.saga(workflow)
    }

    pub fn apply(
        &mut self,
        expected: Option<StoreRevision>,
        command: CommandV1,
    ) -> Result<ModelSagaAppliedV1, ModelSagaStoreErrorV1> {
        self.guard()?;
        if expected != self.storage_revision {
            return Err(ModelSagaStoreErrorV1::Conflict {
                expected,
                actual: self.storage_revision,
            });
        }
        let mut candidate = Engine::recover(self.engine.checkpoint())?;
        let transition = candidate.apply(command)?;
        let changed = candidate.checkpoint() != self.engine.checkpoint();
        let actual = read_revision(&self.connection)?;
        if actual != self.storage_revision {
            self.healthy = false;
            return Err(ModelSagaStoreErrorV1::Conflict {
                expected: self.storage_revision,
                actual,
            });
        }
        if !changed {
            return Ok(ModelSagaAppliedV1 {
                storage_revision: actual.ok_or(ModelSagaStoreErrorV1::Schema)?,
                transition,
                replayed: true,
            });
        }
        let encoded = encode(&candidate.checkpoint(), self.limits.codec)?;
        let next = StoreRevision(match self.storage_revision {
            Some(revision) => revision
                .0
                .checked_add(1)
                .ok_or(ModelSagaStoreErrorV1::RevisionExhausted)?,
            None => 0,
        });
        self.healthy = false;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let retained = read_revision(&transaction)?;
        if retained != self.storage_revision {
            return Err(ModelSagaStoreErrorV1::Conflict {
                expected: self.storage_revision,
                actual: retained,
            });
        }
        transaction.execute(
            "INSERT INTO model_saga_state VALUES (1,?1,?2) ON CONFLICT(slot) DO UPDATE SET storage_revision=excluded.storage_revision,payload=excluded.payload",
            params![next.0.to_be_bytes().as_slice(), encoded.as_slice()],
        )?;
        transaction.execute("UPDATE model_saga_meta SET initialized=1 WHERE slot=1", [])?;
        transaction
            .commit()
            .map_err(|_| ModelSagaStoreErrorV1::Indeterminate)?;
        self.resource
            .check()
            .map_err(|_| ModelSagaStoreErrorV1::Indeterminate)?;
        self.healthy = true;
        self.engine = candidate;
        self.storage_revision = Some(next);
        Ok(ModelSagaAppliedV1 {
            storage_revision: next,
            transition,
            replayed: false,
        })
    }

    fn guard(&mut self) -> Result<(), ModelSagaStoreErrorV1> {
        if !self.healthy {
            return Err(ModelSagaStoreErrorV1::NeedsReopen);
        }
        if let Err(error) = self.resource.check() {
            self.healthy = false;
            return Err(map_resource(error));
        }
        Ok(())
    }
}

impl ModelSagaStore for ModelSagaSqliteStore {
    type Error = ModelSagaStoreErrorV1;

    fn storage_revision(&self) -> Option<u64> {
        self.storage_revision().map(|revision| revision.0)
    }

    fn needs_reopen(&self) -> bool {
        self.needs_reopen()
    }

    fn saga(&self, workflow: WorkflowId) -> Option<SagaSnapshotV1> {
        self.saga(workflow).cloned()
    }

    fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        command: CommandV1,
    ) -> Result<ModelSagaApplyReceiptV1, ModelSagaApplyErrorV1<Self::Error>> {
        match ModelSagaSqliteStore::apply(
            self,
            expected_storage_revision.map(StoreRevision),
            command,
        ) {
            Ok(receipt) => Ok(ModelSagaApplyReceiptV1 {
                storage_revision: receipt.storage_revision.0,
                transition: receipt.transition,
                replayed: receipt.replayed,
            }),
            Err(ModelSagaStoreErrorV1::Refusal(value)) => Err(ModelSagaApplyErrorV1::Domain(value)),
            Err(ModelSagaStoreErrorV1::Conflict { expected, actual }) => {
                Err(ModelSagaApplyErrorV1::Conflict {
                    expected: expected.map(|revision| revision.0),
                    actual: actual.map(|revision| revision.0),
                })
            }
            Err(error) => Err(ModelSagaApplyErrorV1::Backend(error)),
        }
    }
}

fn read_revision(connection: &Connection) -> Result<Option<StoreRevision>, ModelSagaStoreErrorV1> {
    let bytes: Option<Vec<u8>> = connection
        .query_row(
            "SELECT storage_revision FROM model_saga_state WHERE slot=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    bytes
        .map(|bytes| {
            Ok(StoreRevision(u64::from_be_bytes(
                bytes
                    .try_into()
                    .map_err(|_| ModelSagaStoreErrorV1::Schema)?,
            )))
        })
        .transpose()
}

fn read_state(
    connection: &Connection,
    owner: [u8; 16],
    limits: ModelSagaStoreLimitsV1,
) -> Result<Option<(StoreRevision, CheckpointV1)>, ModelSagaStoreErrorV1> {
    let (retained_owner, retained_profile, initialized): (Vec<u8>, Vec<u8>, i64) = connection
        .query_row(
            "SELECT owner,profile,initialized FROM model_saga_meta WHERE slot=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    if retained_owner != owner {
        return Err(ModelSagaStoreErrorV1::OwnerMismatch);
    }
    if retained_profile != profile(limits.saga) {
        return Err(ModelSagaStoreErrorV1::InvalidConfig);
    }
    let revision = read_revision(connection)?;
    if !matches!(initialized, 0 | 1) || (initialized == 1) != revision.is_some() {
        return Err(ModelSagaStoreErrorV1::Schema);
    }
    let Some(revision) = revision else {
        return Ok(None);
    };
    let payload: Vec<u8> = connection.query_row(
        "SELECT payload FROM model_saga_state WHERE slot=1",
        [],
        |row| row.get(0),
    )?;
    let checkpoint = decode(&payload, limits.codec)?;
    if checkpoint != Engine::recover(checkpoint.clone())?.checkpoint()
        || encode(&checkpoint, limits.codec)? != payload
    {
        return Err(ModelSagaStoreErrorV1::Schema);
    }
    Ok(Some((revision, checkpoint)))
}

fn profile(limits: LimitsV1) -> Vec<u8> {
    let mut value = Vec::with_capacity(24);
    value.extend_from_slice(&(limits.max_sagas as u64).to_be_bytes());
    value.extend_from_slice(&(limits.max_proposals_per_saga as u64).to_be_bytes());
    value.extend_from_slice(&(limits.max_proposal_bytes as u64).to_be_bytes());
    value
}

fn map_resource(error: crate::StoreError) -> ModelSagaStoreErrorV1 {
    match error {
        crate::StoreError::Busy => ModelSagaStoreErrorV1::Busy,
        crate::StoreError::ForeignDatabase => ModelSagaStoreErrorV1::ForeignDatabase,
        crate::StoreError::OwnerMismatch => ModelSagaStoreErrorV1::OwnerMismatch,
        crate::StoreError::NeedsReopen => ModelSagaStoreErrorV1::NeedsReopen,
        crate::StoreError::LostResource => ModelSagaStoreErrorV1::LostResource,
        crate::StoreError::Io(kind) => ModelSagaStoreErrorV1::Io(kind),
        crate::StoreError::Sqlite(code) => ModelSagaStoreErrorV1::Sqlite(code),
        _ => ModelSagaStoreErrorV1::Schema,
    }
}
