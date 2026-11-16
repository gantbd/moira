//! Separately owned durable content-addressed outputs for tool continuations.

use crate::{resource::OwnedResource, validate_sqlite_integrity};
use moira_core::tool_continuation::{ToolCapabilityIdV1, ToolExecutionIdV1};
use moira_host::{ToolOutputCommitV1, ToolOutputRecordV1, ToolOutputStoreV1};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, limits::Limit, params,
};
use sha2::{Digest, Sha256};
use std::{io::ErrorKind, path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4d544f53;
const SCHEMA: &str = "CREATE TABLE tool_output_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), profile BLOB NOT NULL CHECK(length(profile)=32));
CREATE TABLE tool_outputs (execution BLOB PRIMARY KEY CHECK(length(execution)=16), capability BLOB NOT NULL CHECK(length(capability)=16), evidence BLOB NOT NULL CHECK(length(evidence)=32), digest BLOB NOT NULL CHECK(length(digest)=32), output BLOB NOT NULL);
CREATE INDEX tool_outputs_digest ON tool_outputs(digest);";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolOutputStoreLimitsV1 {
    pub max_entries: usize,
    pub max_output_bytes: usize,
    pub max_database_pages: u32,
}

impl Default for ToolOutputStoreLimitsV1 {
    fn default() -> Self {
        Self {
            max_entries: 65_536,
            max_output_bytes: 1_048_576,
            max_database_pages: 8_192,
        }
    }
}

#[derive(Debug)]
pub enum ToolOutputStoreErrorV1 {
    InvalidConfig,
    Busy,
    ForeignDatabase,
    OwnerMismatch,
    ProfileMismatch,
    Capacity,
    Conflict,
    Integrity,
    Io(ErrorKind),
    Sqlite(ErrorCode),
    Indeterminate,
}

impl std::fmt::Display for ToolOutputStoreErrorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Moira tool output SQLite: {self:?}")
    }
}

impl std::error::Error for ToolOutputStoreErrorV1 {}

impl From<rusqlite::Error> for ToolOutputStoreErrorV1 {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            Some(code) => Self::Sqlite(code),
            None => Self::Integrity,
        }
    }
}

pub struct ToolOutputSqliteStore {
    connection: Connection,
    resource: OwnedResource,
    limits: ToolOutputStoreLimitsV1,
}

impl std::fmt::Debug for ToolOutputSqliteStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ToolOutputSqliteStore(<redacted>)")
    }
}

impl ToolOutputSqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        limits: ToolOutputStoreLimitsV1,
    ) -> Result<Self, ToolOutputStoreErrorV1> {
        validate_limits(limits)?;
        let resource =
            OwnedResource::open(path, limits.max_database_pages).map_err(map_resource)?;
        let mut connection = Connection::open_with_flags(
            resource.path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::ZERO)?;
        let record_limit: i32 = limits
            .max_output_bytes
            .checked_add(1_024)
            .and_then(|value| value.try_into().ok())
            .ok_or(ToolOutputStoreErrorV1::InvalidConfig)?;
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
                || names != ["tool_output_meta", "tool_outputs", "tool_outputs_digest"])
        {
            return Err(ToolOutputStoreErrorV1::ForeignDatabase);
        }
        let profile = profile(limits);
        if fresh {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute(
                "INSERT INTO tool_output_meta VALUES (1,?1,?2)",
                params![owner.as_slice(), profile.as_slice()],
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction
                .commit()
                .map_err(|_| ToolOutputStoreErrorV1::Indeterminate)?;
            resource.sync_parent().map_err(map_resource)?;
        } else {
            let (retained_owner, retained_profile): (Vec<u8>, Vec<u8>) = connection.query_row(
                "SELECT owner,profile FROM tool_output_meta WHERE slot=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if retained_owner.as_slice() != owner {
                return Err(ToolOutputStoreErrorV1::OwnerMismatch);
            }
            if retained_profile.as_slice() != profile {
                return Err(ToolOutputStoreErrorV1::ProfileMismatch);
            }
        }
        let journal: String =
            connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        connection.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=EXTRA; PRAGMA fullfsync=ON; PRAGMA temp_store=MEMORY;",
        )?;
        connection.pragma_update(None, "max_page_count", limits.max_database_pages)?;
        let synchronous: i64 =
            connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
        let full_sync: i64 = connection.pragma_query_value(None, "fullfsync", |row| row.get(0))?;
        if !journal.eq_ignore_ascii_case("delete") || synchronous != 3 || full_sync != 1 {
            return Err(ToolOutputStoreErrorV1::InvalidConfig);
        }
        Ok(Self {
            connection,
            resource,
            limits,
        })
    }

    fn check_resource(&self) -> Result<(), ToolOutputStoreErrorV1> {
        self.resource.check().map_err(map_resource)
    }
}

impl ToolOutputStoreV1 for ToolOutputSqliteStore {
    type Error = ToolOutputStoreErrorV1;

    fn put(
        &mut self,
        execution: ToolExecutionIdV1,
        capability: ToolCapabilityIdV1,
        evidence_digest: [u8; 32],
        output: &[u8],
    ) -> Result<ToolOutputCommitV1, Self::Error> {
        self.check_resource()?;
        if execution.0 == [0; 16]
            || capability.0 == [0; 16]
            || evidence_digest == [0; 32]
            || output.len() > self.limits.max_output_bytes
        {
            return Err(ToolOutputStoreErrorV1::InvalidConfig);
        }
        let output_digest: [u8; 32] = Sha256::digest(output).into();
        let existing = self
            .connection
            .query_row(
                "SELECT capability,evidence,digest,output FROM tool_outputs WHERE execution=?1",
                params![execution.0.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .optional()?;
        if let Some((stored_capability, stored_evidence, stored_digest, stored_output)) = existing {
            if stored_capability.as_slice() != capability.0
                || stored_evidence.as_slice() != evidence_digest
                || stored_digest.as_slice() != output_digest
                || stored_output != output
            {
                return Err(ToolOutputStoreErrorV1::Conflict);
            }
            return Ok(commit(
                execution,
                capability,
                evidence_digest,
                output_digest,
                output.len(),
                true,
            ));
        }
        let count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM tool_outputs", [], |row| row.get(0))?;
        if usize::try_from(count).map_err(|_| ToolOutputStoreErrorV1::Integrity)?
            >= self.limits.max_entries
        {
            return Err(ToolOutputStoreErrorV1::Capacity);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO tool_outputs VALUES (?1,?2,?3,?4,?5)",
            params![
                execution.0.as_slice(),
                capability.0.as_slice(),
                evidence_digest.as_slice(),
                output_digest.as_slice(),
                output,
            ],
        )?;
        transaction
            .commit()
            .map_err(|_| ToolOutputStoreErrorV1::Indeterminate)?;
        Ok(commit(
            execution,
            capability,
            evidence_digest,
            output_digest,
            output.len(),
            false,
        ))
    }

    fn get(&mut self, output_digest: [u8; 32]) -> Result<Option<Vec<u8>>, Self::Error> {
        self.check_resource()?;
        if output_digest == [0; 32] {
            return Err(ToolOutputStoreErrorV1::Integrity);
        }
        let output = self
            .connection
            .query_row(
                "SELECT output FROM tool_outputs WHERE digest=?1 LIMIT 1",
                params![output_digest.as_slice()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        if output
            .as_ref()
            .is_some_and(|bytes| <[u8; 32]>::from(Sha256::digest(bytes)) != output_digest)
        {
            return Err(ToolOutputStoreErrorV1::Integrity);
        }
        Ok(output)
    }

    fn find(
        &mut self,
        execution: ToolExecutionIdV1,
    ) -> Result<Option<(ToolOutputRecordV1, Vec<u8>)>, Self::Error> {
        self.check_resource()?;
        let value = self
            .connection
            .query_row(
                "SELECT capability,evidence,digest,output FROM tool_outputs WHERE execution=?1",
                params![execution.0.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((capability, evidence, digest, output)) = value else {
            return Ok(None);
        };
        let capability: [u8; 16] = capability
            .try_into()
            .map_err(|_| ToolOutputStoreErrorV1::Integrity)?;
        let evidence_digest: [u8; 32] = evidence
            .try_into()
            .map_err(|_| ToolOutputStoreErrorV1::Integrity)?;
        let output_digest: [u8; 32] = digest
            .try_into()
            .map_err(|_| ToolOutputStoreErrorV1::Integrity)?;
        if <[u8; 32]>::from(Sha256::digest(&output)) != output_digest {
            return Err(ToolOutputStoreErrorV1::Integrity);
        }
        Ok(Some((
            ToolOutputRecordV1 {
                execution,
                capability: ToolCapabilityIdV1(capability),
                evidence_digest,
                output_digest,
                output_bytes: output.len(),
            },
            output,
        )))
    }
}

fn commit(
    execution: ToolExecutionIdV1,
    capability: ToolCapabilityIdV1,
    evidence_digest: [u8; 32],
    output_digest: [u8; 32],
    output_bytes: usize,
    replayed: bool,
) -> ToolOutputCommitV1 {
    ToolOutputCommitV1 {
        record: ToolOutputRecordV1 {
            execution,
            capability,
            evidence_digest,
            output_digest,
            output_bytes,
        },
        replayed,
    }
}

fn validate_limits(limits: ToolOutputStoreLimitsV1) -> Result<(), ToolOutputStoreErrorV1> {
    if limits.max_entries == 0 || limits.max_output_bytes == 0 || limits.max_database_pages < 8 {
        Err(ToolOutputStoreErrorV1::InvalidConfig)
    } else {
        Ok(())
    }
}

fn profile(limits: ToolOutputStoreLimitsV1) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRATO1");
    hash.update(
        u64::try_from(limits.max_entries)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(
        u64::try_from(limits.max_output_bytes)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(limits.max_database_pages.to_be_bytes());
    hash.finalize().into()
}

fn schema_names(connection: &Connection) -> Result<Vec<String>, ToolOutputStoreErrorV1> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type IN ('table','index') AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let values = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(values)
}

fn map_resource(error: crate::StoreError) -> ToolOutputStoreErrorV1 {
    use crate::StoreError;
    match error {
        StoreError::Busy => ToolOutputStoreErrorV1::Busy,
        StoreError::Io(kind) => ToolOutputStoreErrorV1::Io(kind),
        StoreError::LostResource => ToolOutputStoreErrorV1::Integrity,
        _ => ToolOutputStoreErrorV1::InvalidConfig,
    }
}
