//! Separate durable owner for methodical deliberations.
//!
//! This profile has its own application id and schema. The retained revision is
//! the domain registry revision, not a second adapter-defined sequence.

use crate::{resource::OwnedResource, validate_sqlite_integrity};
use moira_core::deliberation::{
    DeliberationCheckpointV1, DeliberationIdV1,
    registry::{
        self, DeliberationRegistryV1, RegistryCommandV1, RegistryRefusalV1, RegistryRevisionV1,
        persistence::{CodecErrorV1, CodecLimitsV1, decode, encode},
    },
};
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, limits::Limit, params,
};
use std::{io::ErrorKind, path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4d444c42;
const SCHEMA: &str = "CREATE TABLE deliberation_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), profile BLOB NOT NULL, initialized INTEGER NOT NULL CHECK(initialized IN (0,1)));
CREATE TABLE deliberation_state (slot INTEGER PRIMARY KEY CHECK(slot=1), revision BLOB NOT NULL CHECK(length(revision)=8), payload BLOB NOT NULL);";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliberationStoreLimitsV1 {
    pub codec: CodecLimitsV1,
    pub max_database_pages: u32,
}

#[derive(Debug)]
pub enum DeliberationStoreErrorV1 {
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
    Indeterminate,
}

impl std::fmt::Display for DeliberationStoreErrorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Moira deliberation SQLite: {self:?}")
    }
}

impl std::error::Error for DeliberationStoreErrorV1 {}

impl From<rusqlite::Error> for DeliberationStoreErrorV1 {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            Some(code) => Self::Sqlite(code),
            None => Self::Schema,
        }
    }
}

impl From<CodecErrorV1> for DeliberationStoreErrorV1 {
    fn from(error: CodecErrorV1) -> Self {
        Self::Codec(error)
    }
}

pub struct DeliberationSqliteStore {
    connection: Connection,
    resource: OwnedResource,
    limits: DeliberationStoreLimitsV1,
    registry: DeliberationRegistryV1,
    healthy: bool,
}

impl std::fmt::Debug for DeliberationSqliteStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DeliberationSqliteStore(<redacted>)")
    }
}

impl DeliberationSqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        limits: DeliberationStoreLimitsV1,
    ) -> Result<Self, DeliberationStoreErrorV1> {
        let initial = DeliberationRegistryV1::try_new(limits.codec.registry)
            .map_err(|_| DeliberationStoreErrorV1::InvalidConfig)?;
        encode(&initial.checkpoint(), limits.codec)
            .map_err(|_| DeliberationStoreErrorV1::InvalidConfig)?;
        let record_limit: i32 = limits
            .codec
            .max_bytes
            .checked_add(1_024)
            .and_then(|value| value.try_into().ok())
            .ok_or(DeliberationStoreErrorV1::InvalidConfig)?;
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
                || names != ["deliberation_meta", "deliberation_state"])
        {
            return Err(DeliberationStoreErrorV1::ForeignDatabase);
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
            return Err(DeliberationStoreErrorV1::InvalidConfig);
        }
        if fresh {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute(
                "INSERT INTO deliberation_meta VALUES (1,?1,?2,0)",
                params![owner.as_slice(), profile(limits.codec)],
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction
                .commit()
                .map_err(|_| DeliberationStoreErrorV1::Indeterminate)?;
            resource.sync_parent().map_err(map_resource)?;
        }
        let registry = match read_state(&connection, owner, limits)? {
            Some(checkpoint) => DeliberationRegistryV1::recover(checkpoint)
                .map_err(|_| DeliberationStoreErrorV1::Schema)?,
            None => initial,
        };
        resource.check().map_err(map_resource)?;
        Ok(Self {
            connection,
            resource,
            limits,
            registry,
            healthy: true,
        })
    }

    pub const fn revision(&self) -> RegistryRevisionV1 {
        self.registry.revision()
    }

    pub const fn needs_reopen(&self) -> bool {
        !self.healthy
    }

    pub fn deliberation(&self, id: DeliberationIdV1) -> Option<DeliberationCheckpointV1> {
        self.registry.deliberation(id)
    }

    pub fn apply(
        &mut self,
        expected: RegistryRevisionV1,
        command: RegistryCommandV1,
    ) -> Result<registry::RegistryReceiptV1, ApplyFailureV1> {
        self.guard().map_err(ApplyFailureV1::Backend)?;
        let mut candidate = DeliberationRegistryV1::recover(self.registry.checkpoint())
            .map_err(|_| ApplyFailureV1::Backend(DeliberationStoreErrorV1::Schema))?;
        let receipt = candidate
            .apply(expected, command)
            .map_err(ApplyFailureV1::Domain)?;
        let retained = read_revision(&self.connection).map_err(ApplyFailureV1::Backend)?;
        let expected_retained = persisted_revision(self.registry.revision());
        if retained != expected_retained {
            self.healthy = false;
            return Err(ApplyFailureV1::Backend(DeliberationStoreErrorV1::Schema));
        }
        if receipt.replayed {
            return Ok(receipt);
        }
        let encoded = encode(&candidate.checkpoint(), self.limits.codec)
            .map_err(DeliberationStoreErrorV1::from)
            .map_err(ApplyFailureV1::Backend)?;
        self.healthy = false;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(DeliberationStoreErrorV1::from)
            .map_err(ApplyFailureV1::Backend)?;
        let actual = read_revision(&transaction).map_err(ApplyFailureV1::Backend)?;
        if actual != expected_retained {
            return Err(ApplyFailureV1::Backend(DeliberationStoreErrorV1::Schema));
        }
        transaction
            .execute(
                "INSERT INTO deliberation_state VALUES (1,?1,?2) ON CONFLICT(slot) DO UPDATE SET revision=excluded.revision,payload=excluded.payload",
                params![receipt.revision.0.to_be_bytes().as_slice(), encoded.as_slice()],
            )
            .map_err(DeliberationStoreErrorV1::from)
            .map_err(ApplyFailureV1::Backend)?;
        transaction
            .execute(
                "UPDATE deliberation_meta SET initialized=1 WHERE slot=1",
                [],
            )
            .map_err(DeliberationStoreErrorV1::from)
            .map_err(ApplyFailureV1::Backend)?;
        transaction
            .commit()
            .map_err(|_| ApplyFailureV1::Backend(DeliberationStoreErrorV1::Indeterminate))?;
        self.resource
            .check()
            .map_err(|_| ApplyFailureV1::Backend(DeliberationStoreErrorV1::Indeterminate))?;
        self.registry = candidate;
        self.healthy = true;
        Ok(receipt)
    }

    fn guard(&mut self) -> Result<(), DeliberationStoreErrorV1> {
        if !self.healthy {
            return Err(DeliberationStoreErrorV1::NeedsReopen);
        }
        if let Err(error) = self.resource.check() {
            self.healthy = false;
            return Err(map_resource(error));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum ApplyFailureV1 {
    Domain(RegistryRefusalV1),
    Backend(DeliberationStoreErrorV1),
}

impl DeliberationStore for DeliberationSqliteStore {
    type Error = DeliberationStoreErrorV1;

    fn revision(&self) -> RegistryRevisionV1 {
        self.revision()
    }

    fn needs_reopen(&self) -> bool {
        self.needs_reopen()
    }

    fn deliberation(&self, id: DeliberationIdV1) -> Option<DeliberationCheckpointV1> {
        self.deliberation(id)
    }

    fn apply(
        &mut self,
        expected_revision: RegistryRevisionV1,
        command: RegistryCommandV1,
    ) -> Result<registry::RegistryReceiptV1, DeliberationApplyErrorV1<Self::Error>> {
        match DeliberationSqliteStore::apply(self, expected_revision, command) {
            Ok(receipt) => Ok(receipt),
            Err(ApplyFailureV1::Domain(value)) => Err(DeliberationApplyErrorV1::Domain(value)),
            Err(ApplyFailureV1::Backend(error)) => Err(DeliberationApplyErrorV1::Backend(error)),
        }
    }
}

fn persisted_revision(revision: RegistryRevisionV1) -> Option<RegistryRevisionV1> {
    (revision.0 != 0).then_some(revision)
}

fn read_revision(
    connection: &Connection,
) -> Result<Option<RegistryRevisionV1>, DeliberationStoreErrorV1> {
    let bytes: Option<Vec<u8>> = connection
        .query_row(
            "SELECT revision FROM deliberation_state WHERE slot=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    bytes
        .map(|bytes| {
            Ok(RegistryRevisionV1(u64::from_be_bytes(
                bytes
                    .try_into()
                    .map_err(|_| DeliberationStoreErrorV1::Schema)?,
            )))
        })
        .transpose()
}

fn read_state(
    connection: &Connection,
    owner: [u8; 16],
    limits: DeliberationStoreLimitsV1,
) -> Result<Option<registry::RegistryCheckpointV1>, DeliberationStoreErrorV1> {
    let (retained_owner, retained_profile, initialized): (Vec<u8>, Vec<u8>, i64) = connection
        .query_row(
            "SELECT owner,profile,initialized FROM deliberation_meta WHERE slot=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    if retained_owner != owner {
        return Err(DeliberationStoreErrorV1::OwnerMismatch);
    }
    if retained_profile != profile(limits.codec) {
        return Err(DeliberationStoreErrorV1::InvalidConfig);
    }
    let revision = read_revision(connection)?;
    if !matches!(initialized, 0 | 1) || (initialized == 1) != revision.is_some() {
        return Err(DeliberationStoreErrorV1::Schema);
    }
    let Some(revision) = revision else {
        return Ok(None);
    };
    let payload: Vec<u8> = connection.query_row(
        "SELECT payload FROM deliberation_state WHERE slot=1",
        [],
        |row| row.get(0),
    )?;
    let checkpoint = decode(&payload, limits.codec)?;
    if checkpoint.revision != revision
        || checkpoint
            != DeliberationRegistryV1::recover(checkpoint.clone())
                .map_err(|_| DeliberationStoreErrorV1::Schema)?
                .checkpoint()
        || encode(&checkpoint, limits.codec)? != payload
    {
        return Err(DeliberationStoreErrorV1::Schema);
    }
    Ok(Some(checkpoint))
}

fn profile(limits: CodecLimitsV1) -> Vec<u8> {
    let registry = limits.registry;
    let domain = registry.deliberation;
    let mut value = Vec::with_capacity(84);
    for number in [
        limits.max_bytes,
        limits.max_deliberation_bytes,
        registry.max_deliberations,
        domain.max_claims,
        domain.max_criteria,
        domain.max_candidates,
        domain.max_members,
        domain.max_evidence_per_report,
        domain.max_dissent_per_report,
    ] {
        value.extend_from_slice(&(number as u64).to_be_bytes());
    }
    value.extend_from_slice(&domain.max_fuel.to_be_bytes());
    value
}

fn map_resource(error: crate::StoreError) -> DeliberationStoreErrorV1 {
    match error {
        crate::StoreError::Busy => DeliberationStoreErrorV1::Busy,
        crate::StoreError::ForeignDatabase => DeliberationStoreErrorV1::ForeignDatabase,
        crate::StoreError::OwnerMismatch => DeliberationStoreErrorV1::OwnerMismatch,
        crate::StoreError::NeedsReopen => DeliberationStoreErrorV1::NeedsReopen,
        crate::StoreError::LostResource => DeliberationStoreErrorV1::LostResource,
        crate::StoreError::Io(kind) => DeliberationStoreErrorV1::Io(kind),
        crate::StoreError::Sqlite(code) => DeliberationStoreErrorV1::Sqlite(code),
        _ => DeliberationStoreErrorV1::Schema,
    }
}
