//! Separately owned durable content-addressed prompts for agent participation.

use crate::{resource::OwnedResource, validate_sqlite_integrity};
use moira_host::{AgentPromptCommitV1, AgentPromptRecordV1, AgentPromptStoreV1};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, limits::Limit, params,
};
use sha2::{Digest, Sha256};
use std::{io::ErrorKind, path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4d50524d;
const SCHEMA: &str = "CREATE TABLE agent_prompt_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), profile BLOB NOT NULL CHECK(length(profile)=32));
CREATE TABLE agent_prompts (digest BLOB PRIMARY KEY CHECK(length(digest)=32), prompt BLOB NOT NULL CHECK(length(prompt)>0));";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentPromptStoreLimitsV1 {
    pub max_entries: usize,
    pub max_prompt_bytes: usize,
    pub max_database_pages: u32,
}

impl Default for AgentPromptStoreLimitsV1 {
    fn default() -> Self {
        Self {
            max_entries: 65_536,
            max_prompt_bytes: 1_048_576,
            max_database_pages: 8_192,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum AgentPromptStoreErrorV1 {
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

impl std::fmt::Display for AgentPromptStoreErrorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Moira agent prompt SQLite: {self:?}")
    }
}

impl std::error::Error for AgentPromptStoreErrorV1 {}

impl From<rusqlite::Error> for AgentPromptStoreErrorV1 {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            Some(code) => Self::Sqlite(code),
            None => Self::Integrity,
        }
    }
}

pub struct AgentPromptSqliteStore {
    connection: Connection,
    resource: OwnedResource,
    limits: AgentPromptStoreLimitsV1,
}

impl std::fmt::Debug for AgentPromptSqliteStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentPromptSqliteStore(<redacted>)")
    }
}

impl AgentPromptSqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        limits: AgentPromptStoreLimitsV1,
    ) -> Result<Self, AgentPromptStoreErrorV1> {
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
            .max_prompt_bytes
            .checked_add(1_024)
            .and_then(|value| value.try_into().ok())
            .ok_or(AgentPromptStoreErrorV1::InvalidConfig)?;
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
                || names != ["agent_prompt_meta", "agent_prompts"])
        {
            return Err(AgentPromptStoreErrorV1::ForeignDatabase);
        }
        let profile = profile(limits);
        if fresh {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute(
                "INSERT INTO agent_prompt_meta VALUES (1,?1,?2)",
                params![owner.as_slice(), profile.as_slice()],
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction
                .commit()
                .map_err(|_| AgentPromptStoreErrorV1::Indeterminate)?;
            resource.sync_parent().map_err(map_resource)?;
        } else {
            let (retained_owner, retained_profile): (Vec<u8>, Vec<u8>) = connection.query_row(
                "SELECT owner,profile FROM agent_prompt_meta WHERE slot=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if retained_owner.as_slice() != owner {
                return Err(AgentPromptStoreErrorV1::OwnerMismatch);
            }
            if retained_profile.as_slice() != profile {
                return Err(AgentPromptStoreErrorV1::ProfileMismatch);
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
            return Err(AgentPromptStoreErrorV1::InvalidConfig);
        }
        Ok(Self {
            connection,
            resource,
            limits,
        })
    }

    fn check_resource(&self) -> Result<(), AgentPromptStoreErrorV1> {
        self.resource.check().map_err(map_resource)
    }
}

impl AgentPromptStoreV1 for AgentPromptSqliteStore {
    type Error = AgentPromptStoreErrorV1;

    fn put(&mut self, prompt: &[u8]) -> Result<AgentPromptCommitV1, Self::Error> {
        self.check_resource()?;
        if prompt.is_empty() || prompt.len() > self.limits.max_prompt_bytes {
            return Err(AgentPromptStoreErrorV1::InvalidConfig);
        }
        let prompt_digest: [u8; 32] = Sha256::digest(prompt).into();
        let existing = self
            .connection
            .query_row(
                "SELECT prompt FROM agent_prompts WHERE digest=?1",
                params![prompt_digest.as_slice()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        if let Some(stored_prompt) = existing {
            if stored_prompt != prompt {
                return Err(AgentPromptStoreErrorV1::Conflict);
            }
            return Ok(commit(prompt_digest, prompt.len(), true));
        }
        let count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM agent_prompts", [], |row| row.get(0))?;
        if usize::try_from(count).map_err(|_| AgentPromptStoreErrorV1::Integrity)?
            >= self.limits.max_entries
        {
            return Err(AgentPromptStoreErrorV1::Capacity);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO agent_prompts VALUES (?1,?2)",
            params![prompt_digest.as_slice(), prompt],
        )?;
        transaction
            .commit()
            .map_err(|_| AgentPromptStoreErrorV1::Indeterminate)?;
        Ok(commit(prompt_digest, prompt.len(), false))
    }

    fn get(&mut self, prompt_digest: [u8; 32]) -> Result<Option<Vec<u8>>, Self::Error> {
        self.check_resource()?;
        if prompt_digest == [0; 32] {
            return Err(AgentPromptStoreErrorV1::Integrity);
        }
        let prompt = self
            .connection
            .query_row(
                "SELECT prompt FROM agent_prompts WHERE digest=?1",
                params![prompt_digest.as_slice()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        if prompt
            .as_ref()
            .is_some_and(|bytes| <[u8; 32]>::from(Sha256::digest(bytes)) != prompt_digest)
        {
            return Err(AgentPromptStoreErrorV1::Integrity);
        }
        Ok(prompt)
    }
}

fn commit(prompt_digest: [u8; 32], prompt_bytes: usize, replayed: bool) -> AgentPromptCommitV1 {
    AgentPromptCommitV1 {
        record: AgentPromptRecordV1 {
            prompt_digest,
            prompt_bytes,
        },
        replayed,
    }
}

fn validate_limits(limits: AgentPromptStoreLimitsV1) -> Result<(), AgentPromptStoreErrorV1> {
    if limits.max_entries == 0 || limits.max_prompt_bytes == 0 || limits.max_database_pages < 8 {
        Err(AgentPromptStoreErrorV1::InvalidConfig)
    } else {
        Ok(())
    }
}

fn profile(limits: AgentPromptStoreLimitsV1) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRAPR1");
    hash.update(
        u64::try_from(limits.max_entries)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(
        u64::try_from(limits.max_prompt_bytes)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(limits.max_database_pages.to_be_bytes());
    hash.finalize().into()
}

fn schema_names(connection: &Connection) -> Result<Vec<String>, AgentPromptStoreErrorV1> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type IN ('table','index') AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let values = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(values)
}

fn map_resource(error: crate::StoreError) -> AgentPromptStoreErrorV1 {
    use crate::StoreError;
    match error {
        StoreError::Busy => AgentPromptStoreErrorV1::Busy,
        StoreError::Io(kind) => AgentPromptStoreErrorV1::Io(kind),
        StoreError::LostResource => AgentPromptStoreErrorV1::Integrity,
        _ => AgentPromptStoreErrorV1::InvalidConfig,
    }
}
