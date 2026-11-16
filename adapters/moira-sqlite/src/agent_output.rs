//! Durable cursor-ordered assistant output for governed agent turns.

use crate::{resource::OwnedResource, validate_sqlite_integrity};
use moira_host::{AgentOutputCommitV1, AgentOutputRecordV1, AgentOutputStoreV1};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, limits::Limit, params,
};
use sha2::{Digest, Sha256};
use std::{io::ErrorKind, path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4d414f55;
const SCHEMA: &str = "CREATE TABLE agent_output_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), profile BLOB NOT NULL CHECK(length(profile)=32));
CREATE TABLE agent_outputs (participation BLOB NOT NULL CHECK(length(participation)=16), turn BLOB NOT NULL CHECK(length(turn)=16), base_after BLOB, last_cursor BLOB NOT NULL CHECK(length(last_cursor)=8), last_page_after BLOB, last_page_digest BLOB NOT NULL CHECK(length(last_page_digest)=32), last_assistant BLOB NOT NULL, output_digest BLOB NOT NULL CHECK(length(output_digest)=32), output BLOB NOT NULL, PRIMARY KEY(participation,turn));";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentOutputStoreLimitsV1 {
    pub max_entries: usize,
    pub max_output_bytes: usize,
    pub max_page_assistant_bytes: usize,
    pub max_database_pages: u32,
}

impl Default for AgentOutputStoreLimitsV1 {
    fn default() -> Self {
        Self {
            max_entries: 65_536,
            max_output_bytes: 1_048_576,
            max_page_assistant_bytes: 262_144,
            max_database_pages: 16_384,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum AgentOutputStoreErrorV1 {
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

impl std::fmt::Display for AgentOutputStoreErrorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Moira agent output SQLite: {self:?}")
    }
}

impl std::error::Error for AgentOutputStoreErrorV1 {}

impl From<rusqlite::Error> for AgentOutputStoreErrorV1 {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            Some(code) => Self::Sqlite(code),
            None => Self::Integrity,
        }
    }
}

pub struct AgentOutputSqliteStore {
    connection: Connection,
    resource: OwnedResource,
    limits: AgentOutputStoreLimitsV1,
}

impl std::fmt::Debug for AgentOutputSqliteStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentOutputSqliteStore(<redacted>)")
    }
}

impl AgentOutputSqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        limits: AgentOutputStoreLimitsV1,
    ) -> Result<Self, AgentOutputStoreErrorV1> {
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
            .checked_add(limits.max_page_assistant_bytes)
            .and_then(|value| value.checked_add(2_048))
            .and_then(|value| value.try_into().ok())
            .ok_or(AgentOutputStoreErrorV1::InvalidConfig)?;
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
                || names != ["agent_output_meta", "agent_outputs"])
        {
            return Err(AgentOutputStoreErrorV1::ForeignDatabase);
        }
        let profile = profile(limits);
        if fresh {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute(
                "INSERT INTO agent_output_meta VALUES (1,?1,?2)",
                params![owner.as_slice(), profile.as_slice()],
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction
                .commit()
                .map_err(|_| AgentOutputStoreErrorV1::Indeterminate)?;
            resource.sync_parent().map_err(map_resource)?;
        } else {
            let (retained_owner, retained_profile): (Vec<u8>, Vec<u8>) = connection.query_row(
                "SELECT owner,profile FROM agent_output_meta WHERE slot=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if retained_owner.as_slice() != owner {
                return Err(AgentOutputStoreErrorV1::OwnerMismatch);
            }
            if retained_profile.as_slice() != profile {
                return Err(AgentOutputStoreErrorV1::ProfileMismatch);
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
            return Err(AgentOutputStoreErrorV1::InvalidConfig);
        }
        Ok(Self {
            connection,
            resource,
            limits,
        })
    }

    fn check_resource(&self) -> Result<(), AgentOutputStoreErrorV1> {
        self.resource.check().map_err(map_resource)
    }
}

impl AgentOutputStoreV1 for AgentOutputSqliteStore {
    type Error = AgentOutputStoreErrorV1;

    fn append(
        &mut self,
        participation: [u8; 16],
        turn: [u8; 16],
        after: Option<u64>,
        through: u64,
        page_digest: [u8; 32],
        assistant_bytes: &[u8],
    ) -> Result<AgentOutputCommitV1, Self::Error> {
        self.check_resource()?;
        if participation == [0; 16]
            || turn == [0; 16]
            || page_digest == [0; 32]
            || after.is_some_and(|cursor| through <= cursor)
            || assistant_bytes.len() > self.limits.max_page_assistant_bytes
        {
            return Err(AgentOutputStoreErrorV1::InvalidConfig);
        }
        let existing = load(&self.connection, participation, turn)?;
        if let Some(value) = existing {
            validate_retained(&value, participation, turn, self.limits)?;
            if value.last_page_after == after
                && value.last_cursor == through
                && value.last_page_digest == page_digest
                && value.last_assistant == assistant_bytes
            {
                return Ok(commit(
                    participation,
                    turn,
                    through,
                    value.output_digest,
                    value.output.len(),
                    true,
                ));
            }
            if after != Some(value.last_cursor) {
                return Err(AgentOutputStoreErrorV1::Conflict);
            }
            let output_bytes = value
                .output
                .len()
                .checked_add(assistant_bytes.len())
                .ok_or(AgentOutputStoreErrorV1::Capacity)?;
            if output_bytes > self.limits.max_output_bytes {
                return Err(AgentOutputStoreErrorV1::Capacity);
            }
            let mut output = value.output;
            output.extend_from_slice(assistant_bytes);
            let output_digest: [u8; 32] = Sha256::digest(&output).into();
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute(
                "UPDATE agent_outputs SET last_cursor=?3,last_page_after=?4,last_page_digest=?5,last_assistant=?6,output_digest=?7,output=?8 WHERE participation=?1 AND turn=?2",
                params![participation.as_slice(), turn.as_slice(), cursor_blob(through).as_slice(), after.map(cursor_blob), page_digest.as_slice(), assistant_bytes, output_digest.as_slice(), output],
            )?;
            transaction
                .commit()
                .map_err(|_| AgentOutputStoreErrorV1::Indeterminate)?;
            return Ok(commit(
                participation,
                turn,
                through,
                output_digest,
                output_bytes,
                false,
            ));
        }
        let count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM agent_outputs", [], |row| row.get(0))?;
        if usize::try_from(count).map_err(|_| AgentOutputStoreErrorV1::Integrity)?
            >= self.limits.max_entries
        {
            return Err(AgentOutputStoreErrorV1::Capacity);
        }
        if assistant_bytes.len() > self.limits.max_output_bytes {
            return Err(AgentOutputStoreErrorV1::Capacity);
        }
        let output_digest: [u8; 32] = Sha256::digest(assistant_bytes).into();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO agent_outputs VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                participation.as_slice(),
                turn.as_slice(),
                after.map(cursor_blob),
                cursor_blob(through).as_slice(),
                after.map(cursor_blob),
                page_digest.as_slice(),
                assistant_bytes,
                output_digest.as_slice(),
                assistant_bytes
            ],
        )?;
        transaction
            .commit()
            .map_err(|_| AgentOutputStoreErrorV1::Indeterminate)?;
        Ok(commit(
            participation,
            turn,
            through,
            output_digest,
            assistant_bytes.len(),
            false,
        ))
    }

    fn get(
        &mut self,
        participation: [u8; 16],
        turn: [u8; 16],
    ) -> Result<Option<(AgentOutputRecordV1, Vec<u8>)>, Self::Error> {
        self.check_resource()?;
        if participation == [0; 16] || turn == [0; 16] {
            return Err(AgentOutputStoreErrorV1::Integrity);
        }
        let Some(value) = load(&self.connection, participation, turn)? else {
            return Ok(None);
        };
        validate_retained(&value, participation, turn, self.limits)?;
        let record = record(
            participation,
            turn,
            value.last_cursor,
            value.output_digest,
            value.output.len(),
        );
        Ok(Some((record, value.output)))
    }
}

struct Retained {
    base_after: Option<u64>,
    last_cursor: u64,
    last_page_after: Option<u64>,
    last_page_digest: [u8; 32],
    last_assistant: Vec<u8>,
    output_digest: [u8; 32],
    output: Vec<u8>,
}

fn load(
    connection: &Connection,
    participation: [u8; 16],
    turn: [u8; 16],
) -> Result<Option<Retained>, AgentOutputStoreErrorV1> {
    connection
        .query_row(
            "SELECT base_after,last_cursor,last_page_after,last_page_digest,last_assistant,output_digest,output FROM agent_outputs WHERE participation=?1 AND turn=?2",
            params![participation.as_slice(), turn.as_slice()],
            |row| {
                Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                ))
            },
        )
        .optional()?
        .map(|(base, last, page_after, page_digest, last_assistant, output_digest, output)| {
            Ok(Retained {
                base_after: optional_cursor(base)?,
                last_cursor: cursor(last)?,
                last_page_after: optional_cursor(page_after)?,
                last_page_digest: page_digest.try_into().map_err(|_| AgentOutputStoreErrorV1::Integrity)?,
                last_assistant,
                output_digest: output_digest.try_into().map_err(|_| AgentOutputStoreErrorV1::Integrity)?,
                output,
            })
        })
        .transpose()
}

fn validate_retained(
    value: &Retained,
    participation: [u8; 16],
    turn: [u8; 16],
    limits: AgentOutputStoreLimitsV1,
) -> Result<(), AgentOutputStoreErrorV1> {
    if participation == [0; 16]
        || turn == [0; 16]
        || value.last_page_digest == [0; 32]
        || value
            .base_after
            .is_some_and(|cursor| value.last_cursor <= cursor)
        || value
            .last_page_after
            .is_some_and(|cursor| value.last_cursor <= cursor)
        || value.output.len() > limits.max_output_bytes
        || value.last_assistant.len() > limits.max_page_assistant_bytes
        || !value.output.ends_with(&value.last_assistant)
        || <[u8; 32]>::from(Sha256::digest(&value.output)) != value.output_digest
    {
        Err(AgentOutputStoreErrorV1::Integrity)
    } else {
        Ok(())
    }
}

fn cursor_blob(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

fn cursor(value: Vec<u8>) -> Result<u64, AgentOutputStoreErrorV1> {
    Ok(u64::from_be_bytes(
        value
            .try_into()
            .map_err(|_| AgentOutputStoreErrorV1::Integrity)?,
    ))
}

fn optional_cursor(value: Option<Vec<u8>>) -> Result<Option<u64>, AgentOutputStoreErrorV1> {
    value.map(cursor).transpose()
}

fn record(
    participation: [u8; 16],
    turn: [u8; 16],
    last_cursor: u64,
    output_digest: [u8; 32],
    output_bytes: usize,
) -> AgentOutputRecordV1 {
    AgentOutputRecordV1 {
        participation,
        turn,
        last_cursor,
        output_digest,
        output_bytes,
    }
}

fn commit(
    participation: [u8; 16],
    turn: [u8; 16],
    last_cursor: u64,
    output_digest: [u8; 32],
    output_bytes: usize,
    replayed: bool,
) -> AgentOutputCommitV1 {
    AgentOutputCommitV1 {
        record: record(
            participation,
            turn,
            last_cursor,
            output_digest,
            output_bytes,
        ),
        replayed,
    }
}

fn validate_limits(limits: AgentOutputStoreLimitsV1) -> Result<(), AgentOutputStoreErrorV1> {
    if limits.max_entries == 0
        || limits.max_output_bytes == 0
        || limits.max_page_assistant_bytes == 0
        || limits.max_page_assistant_bytes > limits.max_output_bytes
        || limits.max_database_pages < 8
    {
        Err(AgentOutputStoreErrorV1::InvalidConfig)
    } else {
        Ok(())
    }
}

fn profile(limits: AgentOutputStoreLimitsV1) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRAAO1");
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
    hash.update(
        u64::try_from(limits.max_page_assistant_bytes)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(limits.max_database_pages.to_be_bytes());
    hash.finalize().into()
}

fn schema_names(connection: &Connection) -> Result<Vec<String>, AgentOutputStoreErrorV1> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type IN ('table','index') AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?)
}

fn map_resource(error: crate::StoreError) -> AgentOutputStoreErrorV1 {
    use crate::StoreError;
    match error {
        StoreError::Busy => AgentOutputStoreErrorV1::Busy,
        StoreError::Io(kind) => AgentOutputStoreErrorV1::Io(kind),
        StoreError::LostResource => AgentOutputStoreErrorV1::Integrity,
        _ => AgentOutputStoreErrorV1::InvalidConfig,
    }
}
