//! Separate durable owner for the closed saga profile.
//!
//! This profile deliberately uses its own database and schema. Opening a legacy
//! coordinator database as a saga store is a foreign-database refusal, not a
//! migration.

use crate::{
    OnlineBackupLimitsV1, OnlineBackupReceiptV1, StoreError, StoreRevision,
    resource::OwnedResource, validate_sqlite_integrity,
};
use moira_core::saga::{
    EffectEvidencePageV1, SagaCensusV1, SagaCommandV1, SagaEffectIdV1, SagaEngineV1, SagaLimitsV1,
    SagaPageV1, SagaReconciliationViewV1, SagaRefusalV1, SagaSnapshotV1, SagaTransitionV1,
    persistence::{SagaCodecLimitsV1, decode_saga_checkpoint, encode_saga_checkpoint},
};
use moira_host::{SagaApplyErrorV1, SagaApplyReceiptV1, SagaStore};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, TransactionBehavior, limits::Limit, params,
};
use std::{path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4d534147;
const SCHEMA: &str = "CREATE TABLE saga_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), profile BLOB NOT NULL CHECK(length(profile)=25), initialized INTEGER NOT NULL CHECK(initialized IN (0,1)));
CREATE TABLE saga_state (slot INTEGER PRIMARY KEY CHECK(slot=1), storage_revision BLOB NOT NULL CHECK(length(storage_revision)=8), domain_revision BLOB NOT NULL CHECK(length(domain_revision)=8), payload BLOB NOT NULL);";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SagaStoreLimitsV1 {
    pub codec: SagaCodecLimitsV1,
    pub saga: SagaLimitsV1,
    pub max_database_pages: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SagaAppliedV1 {
    pub storage_revision: StoreRevision,
    pub transition: SagaTransitionV1,
    pub replayed: bool,
}

pub struct SagaSqliteStore {
    connection: Connection,
    resource: OwnedResource,
    limits: SagaStoreLimitsV1,
    engine: SagaEngineV1,
    storage_revision: Option<StoreRevision>,
    healthy: bool,
}

impl std::fmt::Debug for SagaSqliteStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SagaSqliteStore(<redacted>)")
    }
}

impl SagaSqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        limits: SagaStoreLimitsV1,
    ) -> Result<Self, StoreError> {
        let initial = SagaEngineV1::try_new(limits.saga).map_err(StoreError::SagaRefusal)?;
        encode_saga_checkpoint(&initial.checkpoint(), limits.codec)
            .map_err(|_| StoreError::InvalidConfig)?;
        let record_limit: i32 = limits
            .codec
            .max_bytes
            .checked_add(1_024)
            .and_then(|value| value.try_into().ok())
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
                || names != ["saga_meta", "saga_state"])
        {
            return Err(StoreError::ForeignDatabase);
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
            return Err(StoreError::InvalidConfig);
        }
        if fresh {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute(
                "INSERT INTO saga_meta VALUES (1,?1,?2,0)",
                params![owner.as_slice(), profile(limits.saga)],
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction
                .commit()
                .map_err(|_| StoreError::Indeterminate)?;
            resource.sync_parent()?;
        }
        let loaded = read_state(&connection, owner, limits)?;
        let (engine, storage_revision) = if let Some((revision, checkpoint)) = loaded {
            (
                SagaEngineV1::recover(checkpoint).map_err(StoreError::SagaRefusal)?,
                Some(revision),
            )
        } else {
            (initial, None)
        };
        let store = Self {
            connection,
            resource,
            limits,
            engine,
            storage_revision,
            healthy: true,
        };
        store.resource.check()?;
        Ok(store)
    }

    pub const fn revision(&self) -> u64 {
        self.engine.revision()
    }

    pub const fn needs_reopen(&self) -> bool {
        !self.healthy
    }

    /// Publishes a bounded integrity-checked SQLite online snapshot while this
    /// single owner remains open. The destination must not already exist.
    pub fn backup_online(
        &mut self,
        destination: impl AsRef<Path>,
        limits: OnlineBackupLimitsV1,
    ) -> Result<OnlineBackupReceiptV1, StoreError> {
        self.guard()?;
        let result = crate::backup::publish_online_backup(
            &self.connection,
            &self.resource,
            destination.as_ref(),
            self.limits.max_database_pages,
            limits,
        );
        if let Err(error) = self.resource.check() {
            self.healthy = false;
            return Err(error);
        }
        Ok(OnlineBackupReceiptV1 {
            storage_revision: self.storage_revision,
            page_count: result?,
        })
    }

    pub fn saga(&self, saga: moira_core::saga::SagaIdV1) -> Option<SagaSnapshotV1> {
        self.engine.saga(saga)
    }

    pub fn inspect_page(
        &self,
        after: Option<moira_core::saga::SagaIdV1>,
        limit: usize,
    ) -> Result<SagaPageV1, SagaRefusalV1> {
        self.engine.inspect_page(after, limit)
    }

    pub fn census(&self) -> SagaCensusV1 {
        self.engine.census()
    }

    pub fn reconciliation_view(
        &self,
        saga: moira_core::saga::SagaIdV1,
    ) -> Option<SagaReconciliationViewV1> {
        self.engine.reconciliation_view(saga)
    }

    pub fn evidence_page(
        &self,
        saga: moira_core::saga::SagaIdV1,
        after: Option<SagaEffectIdV1>,
        limit: usize,
    ) -> Result<EffectEvidencePageV1, SagaRefusalV1> {
        self.engine.evidence_page(saga, after, limit)
    }

    pub fn operator_receipt(
        &self,
        action: moira_core::saga::OperatorActionIdV1,
    ) -> Option<moira_core::saga::OperatorReceiptSummaryV1> {
        self.engine.operator_receipt(action)
    }

    pub fn operator_receipt_page(
        &self,
        after: Option<moira_core::saga::OperatorActionIdV1>,
        limit: usize,
    ) -> Result<moira_core::saga::OperatorReceiptPageV1, SagaRefusalV1> {
        self.engine.operator_receipt_page(after, limit)
    }

    pub fn apply(
        &mut self,
        expected_revision: u64,
        command: SagaCommandV1,
    ) -> Result<SagaAppliedV1, StoreError> {
        self.guard()?;
        let mut candidate =
            SagaEngineV1::recover(self.engine.checkpoint()).map_err(StoreError::SagaRefusal)?;
        let transition = candidate
            .apply(expected_revision, command)
            .map_err(StoreError::SagaRefusal)?;
        let actual = read_revision(&self.connection)?;
        if actual != self.storage_revision {
            self.healthy = false;
            return Err(StoreError::Conflict {
                expected: self.storage_revision,
                actual,
            });
        }
        if candidate.revision() == self.engine.revision() {
            return Ok(SagaAppliedV1 {
                storage_revision: actual.ok_or(StoreError::Schema)?,
                transition,
                replayed: true,
            });
        }
        let encoded = encode_saga_checkpoint(&candidate.checkpoint(), self.limits.codec)?;
        let next = StoreRevision(match self.storage_revision {
            Some(revision) => revision
                .0
                .checked_add(1)
                .ok_or(StoreError::RevisionExhausted)?,
            None => 0,
        });
        self.healthy = false;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let retained = read_revision(&transaction)?;
        if retained != self.storage_revision {
            return Err(StoreError::Conflict {
                expected: self.storage_revision,
                actual: retained,
            });
        }
        transaction.execute(
            "INSERT INTO saga_state VALUES (1,?1,?2,?3) ON CONFLICT(slot) DO UPDATE SET storage_revision=excluded.storage_revision,domain_revision=excluded.domain_revision,payload=excluded.payload",
            params![
                next.0.to_be_bytes().as_slice(),
                candidate.revision().to_be_bytes().as_slice(),
                encoded.as_bytes()
            ],
        )?;
        transaction.execute("UPDATE saga_meta SET initialized=1 WHERE slot=1", [])?;
        transaction
            .commit()
            .map_err(|_| StoreError::Indeterminate)?;
        self.resource
            .check()
            .map_err(|_| StoreError::Indeterminate)?;
        self.healthy = true;
        self.engine = candidate;
        self.storage_revision = Some(next);
        Ok(SagaAppliedV1 {
            storage_revision: next,
            transition,
            replayed: false,
        })
    }

    fn guard(&mut self) -> Result<(), StoreError> {
        if !self.healthy {
            return Err(StoreError::NeedsReopen);
        }
        if let Err(error) = self.resource.check() {
            self.healthy = false;
            return Err(error);
        }
        Ok(())
    }
}

impl SagaStore for SagaSqliteStore {
    type Error = StoreError;

    fn revision(&self) -> u64 {
        self.revision()
    }

    fn needs_reopen(&self) -> bool {
        self.needs_reopen()
    }

    fn saga(&self, saga: moira_core::saga::SagaIdV1) -> Option<SagaSnapshotV1> {
        self.saga(saga)
    }

    fn inspect_page(
        &self,
        after: Option<moira_core::saga::SagaIdV1>,
        limit: usize,
    ) -> Result<SagaPageV1, SagaRefusalV1> {
        self.inspect_page(after, limit)
    }

    fn census(&self) -> SagaCensusV1 {
        self.census()
    }

    fn reconciliation_view(
        &self,
        saga: moira_core::saga::SagaIdV1,
    ) -> Option<SagaReconciliationViewV1> {
        self.reconciliation_view(saga)
    }

    fn evidence_page(
        &self,
        saga: moira_core::saga::SagaIdV1,
        after: Option<SagaEffectIdV1>,
        limit: usize,
    ) -> Result<EffectEvidencePageV1, SagaRefusalV1> {
        self.evidence_page(saga, after, limit)
    }

    fn operator_receipt(
        &self,
        action: moira_core::saga::OperatorActionIdV1,
    ) -> Option<moira_core::saga::OperatorReceiptSummaryV1> {
        self.operator_receipt(action)
    }

    fn operator_receipt_page(
        &self,
        after: Option<moira_core::saga::OperatorActionIdV1>,
        limit: usize,
    ) -> Result<moira_core::saga::OperatorReceiptPageV1, SagaRefusalV1> {
        self.operator_receipt_page(after, limit)
    }

    fn apply(
        &mut self,
        expected_revision: u64,
        command: SagaCommandV1,
    ) -> Result<SagaApplyReceiptV1, SagaApplyErrorV1<Self::Error>> {
        match SagaSqliteStore::apply(self, expected_revision, command) {
            Ok(applied) => Ok(SagaApplyReceiptV1 {
                storage_revision: applied.storage_revision.0,
                transition: applied.transition,
                replayed: applied.replayed,
            }),
            Err(StoreError::SagaRefusal(refusal)) => Err(SagaApplyErrorV1::Domain(refusal)),
            Err(error) => Err(SagaApplyErrorV1::Backend(error)),
        }
    }
}

fn read_revision(connection: &Connection) -> Result<Option<StoreRevision>, StoreError> {
    let bytes: Option<Vec<u8>> = connection
        .query_row(
            "SELECT storage_revision FROM saga_state WHERE slot=1",
            [],
            |row| row.get(0),
        )
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
    limits: SagaStoreLimitsV1,
) -> Result<Option<(StoreRevision, moira_core::saga::SagaCheckpointV1)>, StoreError> {
    let (retained_owner, retained_profile, initialized): (Vec<u8>, Vec<u8>, i64) = connection
        .query_row(
            "SELECT owner,profile,initialized FROM saga_meta WHERE slot=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    if retained_owner != owner {
        return Err(StoreError::OwnerMismatch);
    }
    if retained_profile != profile(limits.saga) {
        return Err(StoreError::InvalidConfig);
    }
    let revision = read_revision(connection)?;
    if !matches!(initialized, 0 | 1) || (initialized == 1) != revision.is_some() {
        return Err(StoreError::Schema);
    }
    let Some(revision) = revision else {
        return Ok(None);
    };
    let (domain_revision, payload): (Vec<u8>, Vec<u8>) = connection.query_row(
        "SELECT domain_revision,payload FROM saga_state WHERE slot=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let domain_revision =
        u64::from_be_bytes(domain_revision.try_into().map_err(|_| StoreError::Schema)?);
    let checkpoint = decode_saga_checkpoint(&payload, limits.codec)?;
    if checkpoint.limits() != limits.saga {
        return Err(StoreError::InvalidConfig);
    }
    let engine = SagaEngineV1::recover(checkpoint.clone()).map_err(StoreError::SagaRefusal)?;
    if engine.revision() != domain_revision {
        return Err(StoreError::Schema);
    }
    Ok(Some((revision, checkpoint)))
}

fn profile(limits: SagaLimitsV1) -> Vec<u8> {
    let mut value = Vec::with_capacity(25);
    value.extend_from_slice(&(limits.max_sagas as u64).to_be_bytes());
    value.push(limits.max_retries_per_saga);
    value.extend_from_slice(&(limits.max_inspection_page as u64).to_be_bytes());
    value.extend_from_slice(&(limits.max_operator_actions as u64).to_be_bytes());
    value
}
