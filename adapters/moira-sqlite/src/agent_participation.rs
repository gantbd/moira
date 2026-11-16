//! Exclusive SQLite owner for durable interactive-agent participation.

use crate::{StoreRevision, resource::OwnedResource, validate_sqlite_integrity};
use moira_core::agent_participation::{
    CheckpointV1, Engine, LimitsV1, ParticipationIdV1, ReceiptV1, RefusalV1, SnapshotV1,
    persistence::{CodecErrorV1, CodecLimitsV1, decode, encode},
};
use moira_host::{
    AgentParticipationApplyErrorV1, AgentParticipationApplyReceiptV1, AgentParticipationMutationV1,
    AgentParticipationStore,
};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, limits::Limit, params,
};
use std::{io::ErrorKind, path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4d415053;
const SCHEMA:&str="CREATE TABLE agent_participation_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), profile BLOB NOT NULL CHECK(length(profile)=8), initialized INTEGER NOT NULL CHECK(initialized IN (0,1)));
CREATE TABLE agent_participation_state (slot INTEGER PRIMARY KEY CHECK(slot=1), storage_revision BLOB NOT NULL CHECK(length(storage_revision)=8), payload BLOB NOT NULL);";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentParticipationStoreLimitsV1 {
    pub codec: CodecLimitsV1,
    pub max_database_pages: u32,
}
#[derive(Debug)]
pub enum AgentParticipationStoreErrorV1 {
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
impl std::fmt::Display for AgentParticipationStoreErrorV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Moira agent participation SQLite: {self:?}")
    }
}
impl std::error::Error for AgentParticipationStoreErrorV1 {}
impl From<rusqlite::Error> for AgentParticipationStoreErrorV1 {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            Some(code) => Self::Sqlite(code),
            None => Self::Schema,
        }
    }
}
impl From<CodecErrorV1> for AgentParticipationStoreErrorV1 {
    fn from(value: CodecErrorV1) -> Self {
        Self::Codec(value)
    }
}
impl From<RefusalV1> for AgentParticipationStoreErrorV1 {
    fn from(value: RefusalV1) -> Self {
        Self::Refusal(value)
    }
}

pub struct AgentParticipationSqliteStore {
    connection: Connection,
    resource: OwnedResource,
    limits: AgentParticipationStoreLimitsV1,
    engine: Engine,
    storage_revision: Option<StoreRevision>,
    healthy: bool,
}
impl std::fmt::Debug for AgentParticipationSqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentParticipationSqliteStore(<redacted>)")
    }
}

impl AgentParticipationSqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        limits: AgentParticipationStoreLimitsV1,
    ) -> Result<Self, AgentParticipationStoreErrorV1> {
        let initial = Engine::try_new(limits.codec.domain)?;
        encode(&initial.checkpoint(), limits.codec)
            .map_err(|_| AgentParticipationStoreErrorV1::InvalidConfig)?;
        let retained_profile = profile(limits.codec.domain)?;
        let record_limit: i32 = limits
            .codec
            .max_bytes
            .checked_add(1024)
            .and_then(|v| v.try_into().ok())
            .ok_or(AgentParticipationStoreErrorV1::InvalidConfig)?;
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
            connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let names = schema_names(&connection)?;
        let fresh = application == 0 && version == 0 && names.is_empty();
        if !fresh
            && (application != APPLICATION_ID
                || version != 1
                || names != ["agent_participation_meta", "agent_participation_state"])
        {
            return Err(AgentParticipationStoreErrorV1::ForeignDatabase);
        }
        if !fresh {
            read_state(&connection, owner, retained_profile, limits.codec)?;
        }
        let journal: String =
            connection.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
        connection.execute_batch(
            "PRAGMA synchronous=EXTRA; PRAGMA fullfsync=ON; PRAGMA temp_store=MEMORY;",
        )?;
        connection.pragma_update(None, "max_page_count", limits.max_database_pages)?;
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |r| r.get(0))?;
        let pages: u32 = connection.pragma_query_value(None, "max_page_count", |r| r.get(0))?;
        let synchronous: i64 = connection.pragma_query_value(None, "synchronous", |r| r.get(0))?;
        let full_sync: i64 = connection.pragma_query_value(None, "fullfsync", |r| r.get(0))?;
        if journal != "delete"
            || synchronous != 3
            || full_sync != 1
            || page_size != 4096
            || pages > limits.max_database_pages
        {
            return Err(AgentParticipationStoreErrorV1::InvalidConfig);
        }
        if fresh {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute(
                "INSERT INTO agent_participation_meta VALUES (1,?1,?2,0)",
                params![owner.as_slice(), retained_profile.as_slice()],
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction
                .commit()
                .map_err(|_| AgentParticipationStoreErrorV1::Indeterminate)?;
            resource.sync_parent().map_err(map_resource)?;
        }
        let loaded = read_state(&connection, owner, retained_profile, limits.codec)?;
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
    pub fn participation(&self, id: ParticipationIdV1) -> Option<&SnapshotV1> {
        self.engine.participation(id)
    }
    pub fn apply(
        &mut self,
        expected: Option<StoreRevision>,
        mutation: AgentParticipationMutationV1,
    ) -> Result<AgentParticipationAppliedV1, AgentParticipationStoreErrorV1> {
        self.guard()?;
        if expected != self.storage_revision {
            return Err(AgentParticipationStoreErrorV1::Conflict {
                expected,
                actual: self.storage_revision,
            });
        }
        let before = self.engine.checkpoint();
        let mut candidate = Engine::recover(before.clone())?;
        let receipt = apply_domain(&mut candidate, mutation)?;
        let participation = candidate
            .participation(receipt.participation)
            .cloned()
            .ok_or(AgentParticipationStoreErrorV1::Schema)?;
        let changed = candidate.checkpoint() != before;
        let actual = read_revision(&self.connection)?;
        if actual != self.storage_revision {
            self.healthy = false;
            return Err(AgentParticipationStoreErrorV1::Conflict {
                expected: self.storage_revision,
                actual,
            });
        }
        if !changed {
            return Ok(AgentParticipationAppliedV1 {
                storage_revision: actual.ok_or(AgentParticipationStoreErrorV1::Schema)?,
                receipt,
                participation,
                replayed: true,
            });
        }
        let encoded = encode(&candidate.checkpoint(), self.limits.codec)?;
        let next = StoreRevision(match self.storage_revision {
            Some(v) => {
                v.0.checked_add(1)
                    .ok_or(AgentParticipationStoreErrorV1::RevisionExhausted)?
            }
            None => 0,
        });
        self.healthy = false;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let retained = read_revision(&transaction)?;
        if retained != self.storage_revision {
            return Err(AgentParticipationStoreErrorV1::Conflict {
                expected: self.storage_revision,
                actual: retained,
            });
        }
        transaction.execute("INSERT INTO agent_participation_state VALUES (1,?1,?2) ON CONFLICT(slot) DO UPDATE SET storage_revision=excluded.storage_revision,payload=excluded.payload",params![next.0.to_be_bytes().as_slice(),encoded.as_slice()])?;
        transaction.execute(
            "UPDATE agent_participation_meta SET initialized=1 WHERE slot=1",
            [],
        )?;
        transaction
            .commit()
            .map_err(|_| AgentParticipationStoreErrorV1::Indeterminate)?;
        self.resource
            .check()
            .map_err(|_| AgentParticipationStoreErrorV1::Indeterminate)?;
        self.engine = candidate;
        self.storage_revision = Some(next);
        self.healthy = true;
        Ok(AgentParticipationAppliedV1 {
            storage_revision: next,
            receipt,
            participation,
            replayed: false,
        })
    }
    fn guard(&mut self) -> Result<(), AgentParticipationStoreErrorV1> {
        if !self.healthy {
            return Err(AgentParticipationStoreErrorV1::NeedsReopen);
        }
        if let Err(value) = self.resource.check() {
            self.healthy = false;
            return Err(map_resource(value));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentParticipationAppliedV1 {
    pub storage_revision: StoreRevision,
    pub receipt: ReceiptV1,
    pub participation: SnapshotV1,
    pub replayed: bool,
}
fn apply_domain(
    engine: &mut Engine,
    value: AgentParticipationMutationV1,
) -> Result<ReceiptV1, RefusalV1> {
    match value {
        AgentParticipationMutationV1::Admit(v) => engine.admit(v),
        AgentParticipationMutationV1::Prepare {
            participation,
            expected_revision,
            operation,
        } => engine.prepare(participation, expected_revision, operation),
        AgentParticipationMutationV1::Observe {
            participation,
            expected_revision,
            key,
            observation,
        } => engine.observe(participation, expected_revision, key, observation),
        AgentParticipationMutationV1::FenceUnknown {
            participation,
            expected_revision,
            key,
        } => engine.fence_unknown(participation, expected_revision, key),
        AgentParticipationMutationV1::MarkReattachRequired {
            participation,
            expected_revision,
            session_revision,
        } => engine.mark_reattach_required(participation, expected_revision, session_revision),
    }
}

impl AgentParticipationStore for AgentParticipationSqliteStore {
    type Error = AgentParticipationStoreErrorV1;
    fn storage_revision(&self) -> Option<u64> {
        self.storage_revision().map(|v| v.0)
    }
    fn needs_reopen(&self) -> bool {
        self.needs_reopen()
    }
    fn participation(&self, id: ParticipationIdV1) -> Option<SnapshotV1> {
        self.participation(id).cloned()
    }
    fn apply(
        &mut self,
        expected: Option<u64>,
        mutation: AgentParticipationMutationV1,
    ) -> Result<AgentParticipationApplyReceiptV1, AgentParticipationApplyErrorV1<Self::Error>> {
        match AgentParticipationSqliteStore::apply(self, expected.map(StoreRevision), mutation) {
            Ok(v) => Ok(AgentParticipationApplyReceiptV1 {
                storage_revision: v.storage_revision.0,
                receipt: v.receipt,
                participation: v.participation,
                replayed: v.replayed,
            }),
            Err(AgentParticipationStoreErrorV1::Refusal(v)) => {
                Err(AgentParticipationApplyErrorV1::Domain(v))
            }
            Err(AgentParticipationStoreErrorV1::Conflict { expected, actual }) => {
                Err(AgentParticipationApplyErrorV1::Conflict {
                    expected: expected.map(|v| v.0),
                    actual: actual.map(|v| v.0),
                })
            }
            Err(v) => Err(AgentParticipationApplyErrorV1::Backend(v)),
        }
    }
}

fn schema_names(c: &Connection) -> Result<Vec<String>, AgentParticipationStoreErrorV1> {
    let mut s = c.prepare(
        "SELECT name FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 3",
    )?;
    Ok(s.query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?)
}
fn read_revision(c: &Connection) -> Result<Option<StoreRevision>, AgentParticipationStoreErrorV1> {
    let bytes: Option<Vec<u8>> = c
        .query_row(
            "SELECT storage_revision FROM agent_participation_state WHERE slot=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    bytes
        .map(|v| {
            Ok(StoreRevision(u64::from_be_bytes(
                v.try_into()
                    .map_err(|_| AgentParticipationStoreErrorV1::Schema)?,
            )))
        })
        .transpose()
}
fn read_state(
    c: &Connection,
    owner: [u8; 16],
    expected_profile: [u8; 8],
    codec: CodecLimitsV1,
) -> Result<Option<(StoreRevision, CheckpointV1)>, AgentParticipationStoreErrorV1> {
    let (retained_owner, retained_profile, initialized): (Vec<u8>, Vec<u8>, i64) = c.query_row(
        "SELECT owner,profile,initialized FROM agent_participation_meta WHERE slot=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if retained_owner != owner {
        return Err(AgentParticipationStoreErrorV1::OwnerMismatch);
    }
    if retained_profile != expected_profile {
        return Err(AgentParticipationStoreErrorV1::InvalidConfig);
    }
    let revision = read_revision(c)?;
    if !matches!(initialized, 0 | 1) || (initialized == 1) != revision.is_some() {
        return Err(AgentParticipationStoreErrorV1::Schema);
    }
    let Some(revision) = revision else {
        return Ok(None);
    };
    let payload: Vec<u8> = c.query_row(
        "SELECT payload FROM agent_participation_state WHERE slot=1",
        [],
        |r| r.get(0),
    )?;
    let checkpoint = decode(&payload, codec)?;
    if checkpoint != Engine::recover(checkpoint.clone())?.checkpoint()
        || encode(&checkpoint, codec)? != payload
    {
        return Err(AgentParticipationStoreErrorV1::Schema);
    }
    Ok(Some((revision, checkpoint)))
}
fn profile(v: LimitsV1) -> Result<[u8; 8], AgentParticipationStoreErrorV1> {
    Ok(u64::try_from(v.max_participations)
        .map_err(|_| AgentParticipationStoreErrorV1::InvalidConfig)?
        .to_be_bytes())
}
fn map_resource(v: crate::StoreError) -> AgentParticipationStoreErrorV1 {
    match v {
        crate::StoreError::Busy => AgentParticipationStoreErrorV1::Busy,
        crate::StoreError::ForeignDatabase => AgentParticipationStoreErrorV1::ForeignDatabase,
        crate::StoreError::OwnerMismatch => AgentParticipationStoreErrorV1::OwnerMismatch,
        crate::StoreError::NeedsReopen => AgentParticipationStoreErrorV1::NeedsReopen,
        crate::StoreError::LostResource => AgentParticipationStoreErrorV1::LostResource,
        crate::StoreError::Io(v) => AgentParticipationStoreErrorV1::Io(v),
        crate::StoreError::Sqlite(v) => AgentParticipationStoreErrorV1::Sqlite(v),
        _ => AgentParticipationStoreErrorV1::Schema,
    }
}
