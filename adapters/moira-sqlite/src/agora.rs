//! Dedicated Agora state/publication receiver. Authentication stays with hosts;
//! participant/room authority, retention, replay and outbox semantics stay in Agora.
use crate::{
    StoreError, StoreRevision, Timing, Usage, count, record_timing, resource::OwnedResource,
};
use agora_engine::persistence::{CheckpointError, RecoveryLimits};
use agora_engine::*;
use moira_core::{AgoraMessageRefV1, AgoraPublicationReceiptV1, EffectId};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, limits::Limit, params};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};
const APPLICATION_ID: i64 = 0x4d41474f;
const SCHEMA:&str="CREATE TABLE agora_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), source BLOB NOT NULL CHECK(length(source)=16), generation BLOB NOT NULL CHECK(length(generation)=8), next_room BLOB NOT NULL CHECK(length(next_room)=8), checkpoint BLOB NOT NULL);
CREATE TABLE agora_rooms (room TEXT PRIMARY KEY, ordinal BLOB NOT NULL UNIQUE CHECK(length(ordinal)=8));";

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AgoraIdentity {
    pub owner: [u8; 16],
    pub source: [u8; 16],
}
impl std::fmt::Debug for AgoraIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgoraIdentity(<redacted>)")
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgoraReceiverLimits {
    pub engine: Limits,
    pub max_snapshot_bytes: usize,
    pub max_database_pages: u32,
    pub timing: Timing,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalClass {
    RevisionConflict,
    OutboxFull,
    Capacity,
    UnknownResource,
    Authority,
    InvalidInput,
    Duplicate,
    Exhausted,
}
#[derive(Debug)]
pub enum AgoraReceiverError {
    Storage(StoreError),
    Checkpoint(CheckpointError),
    InvalidConfig,
    InvalidState,
    ScopeMismatch,
    CommandRefused(RefusalClass),
    PublicationRefused(RefusalClass),
    PublicationConflict,
    PublicationCapacity,
    AcknowledgementRefused,
    UnretainedMessage,
}
impl std::fmt::Display for AgoraReceiverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Agora receiver: {self:?}")
    }
}
impl std::error::Error for AgoraReceiverError {}
impl From<StoreError> for AgoraReceiverError {
    fn from(e: StoreError) -> Self {
        Self::Storage(e)
    }
}
impl From<rusqlite::Error> for AgoraReceiverError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}
impl From<CheckpointError> for AgoraReceiverError {
    fn from(e: CheckpointError) -> Self {
        Self::Checkpoint(e)
    }
}
#[derive(Clone, Eq, PartialEq)]
pub struct ReplyMessage {
    pub room_id: RoomId,
    pub author: ParticipantId,
    pub at: LogicalTime,
    pub body: String,
}
impl std::fmt::Debug for ReplyMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplyMessage")
            .field("body_bytes", &self.body.len())
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Published {
    pub transition: TransitionV2,
    pub receipt: AgoraPublicationReceiptV1,
    pub replayed: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Acknowledged {
    pub removed: usize,
    pub replayed: bool,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecoveryUsage {
    pub snapshot_bytes: usize,
    pub rooms: usize,
    pub open_service: Option<Duration>,
}
struct Loaded {
    engine: Engine,
    generation: u64,
    next_room: u64,
    catalog: BTreeMap<RoomId, u64>,
    bytes: usize,
}
pub struct AgoraReceiver {
    connection: Connection,
    resource: OwnedResource,
    identity: AgoraIdentity,
    limits: AgoraReceiverLimits,
    engine: Engine,
    catalog: BTreeMap<RoomId, u64>,
    generation: u64,
    next_room: u64,
    confirmed_revision: Revision,
    healthy: bool,
    command_usage: Usage,
    publication_usage: Usage,
    acknowledgement_usage: Usage,
    read_usage: Usage,
    recovery_usage: RecoveryUsage,
    #[cfg(test)]
    fault: Option<crate::Fault>,
}
impl std::fmt::Debug for AgoraReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgoraReceiver(<redacted>)")
    }
}
impl AgoraReceiver {
    pub fn open(
        path: impl AsRef<Path>,
        identity: AgoraIdentity,
        limits: AgoraReceiverLimits,
    ) -> Result<Self, AgoraReceiverError> {
        let started = start(limits.timing);
        let record_limit: i32 = limits
            .max_snapshot_bytes
            .checked_add(2048)
            .and_then(|n| n.try_into().ok())
            .ok_or(AgoraReceiverError::InvalidConfig)?;
        let initial = Engine::with_limits(limits.engine);
        let cp = initial.prepare_checkpoint(limits.max_snapshot_bytes)?;
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
        let app: i64 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let names = {
            let mut q=connection.prepare("SELECT name FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 3")?;
            q.query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let fresh = app == 0 && version == 0 && names.is_empty();
        if !fresh
            && (app != APPLICATION_ID || version != 1 || names != ["agora_meta", "agora_rooms"])
        {
            return Err(StoreError::ForeignDatabase.into());
        }
        let loaded = if fresh {
            Loaded {
                engine: initial,
                generation: 0,
                next_room: 0,
                catalog: BTreeMap::new(),
                bytes: cp.bytes().len(),
            }
        } else {
            load(&connection, identity, limits)?
        };
        let journal: String =
            connection.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
        connection.execute_batch(
            "PRAGMA synchronous=EXTRA; PRAGMA fullfsync=ON; PRAGMA temp_store=MEMORY;",
        )?;
        connection.pragma_update(None, "max_page_count", limits.max_database_pages)?;
        let page: i64 = connection.pragma_query_value(None, "page_size", |r| r.get(0))?;
        let pages: u32 = connection.pragma_query_value(None, "max_page_count", |r| r.get(0))?;
        let sync: i64 = connection.pragma_query_value(None, "synchronous", |r| r.get(0))?;
        let full: i64 = connection.pragma_query_value(None, "fullfsync", |r| r.get(0))?;
        if journal != "delete"
            || page != 4096
            || pages > limits.max_database_pages
            || sync != 3
            || full != 1
        {
            return Err(AgoraReceiverError::InvalidConfig);
        }
        if fresh {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA)?;
            tx.execute(
                "INSERT INTO agora_meta VALUES(1,?1,?2,?3,?3,?4)",
                params![
                    identity.owner.as_slice(),
                    identity.source.as_slice(),
                    0u64.to_be_bytes().as_slice(),
                    cp.bytes()
                ],
            )?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
            resource.sync_parent()?;
        }
        resource.check()?;
        let mut timing = Usage::default();
        record_timing(&mut timing, started);
        Ok(Self {
            connection,
            resource,
            identity,
            limits,
            confirmed_revision: loaded.engine.revision(),
            recovery_usage: RecoveryUsage {
                snapshot_bytes: loaded.bytes,
                rooms: loaded.catalog.len(),
                open_service: timing.max_commit_service,
            },
            engine: loaded.engine,
            catalog: loaded.catalog,
            generation: loaded.generation,
            next_room: loaded.next_room,
            healthy: true,
            command_usage: Usage::default(),
            publication_usage: Usage::default(),
            acknowledgement_usage: Usage::default(),
            read_usage: Usage::default(),
            #[cfg(test)]
            fault: None,
        })
    }
    pub fn revision(&self) -> Revision {
        self.confirmed_revision
    }
    pub fn store_revision(&self) -> StoreRevision {
        StoreRevision(self.generation)
    }
    pub fn needs_reopen(&self) -> bool {
        !self.healthy
    }
    pub fn command_usage(&self) -> Usage {
        self.command_usage
    }
    pub fn publication_usage(&self) -> Usage {
        self.publication_usage
    }
    pub fn acknowledgement_usage(&self) -> Usage {
        self.acknowledgement_usage
    }
    pub fn read_usage(&self) -> Usage {
        self.read_usage
    }
    pub fn recovery_usage(&self) -> RecoveryUsage {
        self.recovery_usage
    }
    fn guard(&mut self) -> Result<(), AgoraReceiverError> {
        if !self.healthy {
            return Err(StoreError::NeedsReopen.into());
        }
        self.resource
            .check()
            .inspect_err(|_| self.healthy = false)?;
        Ok(())
    }
    pub fn apply(
        &mut self,
        command: RevisionedCommandV2,
    ) -> Result<TransitionV2, AgoraReceiverError> {
        let result = (|| {
            self.guard()?;
            let result = self
                .engine
                .apply_revisioned(command)
                .map_err(|e| AgoraReceiverError::CommandRefused(classify(e)))?;
            self.healthy = false;
            let room = match &result.outcome {
                OutcomeV2::RoomCreated(id) => Some(id.clone()),
                _ => None,
            };
            self.persist(room, Operation::Command)?;
            Ok(result)
        })();
        observe(&mut self.command_usage, &result, false);
        result
    }
    pub fn publish(
        &mut self,
        effect: EffectId,
        expected: Revision,
        message: ReplyMessage,
    ) -> Result<Published, AgoraReceiverError> {
        let result = (|| {
            self.guard()?;
            let before = self.engine.revision();
            let transition = self
                .engine
                .publish(PublicationCommandV2::new(
                    expected,
                    PublicationKeyV2::new(effect.into_bytes()),
                    message.room_id,
                    message.author,
                    message.at,
                    message.body,
                ))
                .map_err(|e| match e {
                    PublicationRefusalV2::IdempotencyConflict => {
                        AgoraReceiverError::PublicationConflict
                    }
                    PublicationRefusalV2::PublicationReceiptLimit => {
                        AgoraReceiverError::PublicationCapacity
                    }
                    PublicationRefusalV2::Core(e) => {
                        AgoraReceiverError::PublicationRefused(classify(e))
                    }
                })?;
            let replayed = self.engine.revision() == before;
            if !replayed {
                self.healthy = false;
                self.persist(None, Operation::Publication)?;
            }
            let OutcomeV2::MessageAccepted { room_id, sequence } = &transition.outcome else {
                self.healthy = false;
                return Err(AgoraReceiverError::InvalidState);
            };
            let ordinal = *self
                .catalog
                .get(room_id)
                .ok_or(AgoraReceiverError::InvalidState)?;
            Ok(Published {
                receipt: AgoraPublicationReceiptV1::new(
                    effect,
                    AgoraMessageRefV1::new(self.identity.source, ordinal, *sequence),
                    transition.revision.0,
                ),
                transition,
                replayed,
            })
        })();
        let replayed = result.as_ref().is_ok_and(|r| r.replayed);
        observe(&mut self.publication_usage, &result, replayed);
        result
    }
    pub fn acknowledge_events(
        &mut self,
        sequence: EventSequence,
    ) -> Result<Acknowledged, AgoraReceiverError> {
        let result = (|| {
            self.guard()?;
            let before = self.engine.acknowledged_through();
            let removed = self
                .engine
                .acknowledge_events(sequence)
                .map_err(|_| AgoraReceiverError::AcknowledgementRefused)?;
            let replayed = self.engine.acknowledged_through() == before;
            if !replayed {
                self.healthy = false;
                self.persist(None, Operation::Acknowledgement)?;
            }
            Ok(Acknowledged { removed, replayed })
        })();
        let replayed = result.as_ref().is_ok_and(|r| r.replayed);
        observe(&mut self.acknowledgement_usage, &result, replayed);
        result
    }
    pub fn room(&mut self, id: &RoomId) -> Result<Option<RoomView>, AgoraReceiverError> {
        let result = (|| {
            self.guard()?;
            Ok(self.engine.room(id))
        })();
        observe_read(&mut self.read_usage, &result);
        result
    }
    pub fn pending_events(&mut self) -> Result<Vec<EventEnvelopeV2>, AgoraReceiverError> {
        let result = (|| {
            self.guard()?;
            Ok(self.engine.pending_events().cloned().collect())
        })();
        observe_read(&mut self.read_usage, &result);
        result
    }
    pub fn message_ref(
        &mut self,
        id: &RoomId,
        sequence: u64,
    ) -> Result<AgoraMessageRefV1, AgoraReceiverError> {
        let result = (|| {
            self.guard()?;
            self.engine
                .message(id, sequence)
                .ok_or(AgoraReceiverError::UnretainedMessage)?;
            let ordinal = *self
                .catalog
                .get(id)
                .ok_or(AgoraReceiverError::InvalidState)?;
            Ok(AgoraMessageRefV1::new(
                self.identity.source,
                ordinal,
                sequence,
            ))
        })();
        observe_read(&mut self.read_usage, &result);
        result
    }
    fn persist(
        &mut self,
        room: Option<RoomId>,
        operation: Operation,
    ) -> Result<(), AgoraReceiverError> {
        let next = self
            .generation
            .checked_add(1)
            .ok_or(StoreError::RevisionExhausted)?;
        let next_room = if room.is_some() {
            self.next_room
                .checked_add(1)
                .ok_or(StoreError::RevisionExhausted)?
        } else {
            self.next_room
        };
        let cp = self
            .engine
            .prepare_checkpoint(self.limits.max_snapshot_bytes)?;
        self.resource.check()?;
        let started = start(self.limits.timing);
        let result = (|| -> Result<(), AgoraReceiverError> {
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let actual: Vec<u8> =
                tx.query_row("SELECT generation FROM agora_meta WHERE slot=1", [], |r| {
                    r.get(0)
                })?;
            if number(&actual)? != self.generation {
                return Err(AgoraReceiverError::InvalidState);
            }
            if let Some(id) = &room {
                tx.execute(
                    "INSERT INTO agora_rooms VALUES (?1,?2)",
                    params![id.as_str(), self.next_room.to_be_bytes().as_slice()],
                )?;
            }
            tx.execute(
                "UPDATE agora_meta SET generation=?1,next_room=?2,checkpoint=?3 WHERE slot=1",
                params![
                    next.to_be_bytes().as_slice(),
                    next_room.to_be_bytes().as_slice(),
                    cp.bytes()
                ],
            )?;
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
            match operation {
                Operation::Command => &mut self.command_usage,
                Operation::Publication => &mut self.publication_usage,
                Operation::Acknowledgement => &mut self.acknowledgement_usage,
            },
            started,
        );
        result?;
        if let Some(id) = room {
            self.catalog.insert(id, self.next_room);
        }
        self.next_room = next_room;
        self.generation = next;
        self.confirmed_revision = self.engine.revision();
        self.healthy = true;
        Ok(())
    }
}
enum Operation {
    Command,
    Publication,
    Acknowledgement,
}
fn start(timing: Timing) -> Option<Instant> {
    match timing {
        Timing::Off => None,
        Timing::Process => std::panic::catch_unwind(Instant::now).ok(),
    }
}
fn number(bytes: &[u8]) -> Result<u64, AgoraReceiverError> {
    Ok(u64::from_be_bytes(
        bytes
            .try_into()
            .map_err(|_| AgoraReceiverError::InvalidState)?,
    ))
}
fn load(
    c: &Connection,
    identity: AgoraIdentity,
    limits: AgoraReceiverLimits,
) -> Result<Loaded, AgoraReceiverError> {
    let mut q = c.prepare(
        "SELECT owner,source,generation,next_room,checkpoint FROM agora_meta WHERE slot=1",
    )?;
    let mut rows = q.query([])?;
    let row = rows.next()?.ok_or(AgoraReceiverError::InvalidState)?;
    let blob = |i| {
        row.get_ref(i)?
            .as_blob()
            .map_err(|_| rusqlite::Error::InvalidQuery)
    };
    if blob(0)? != identity.owner {
        return Err(StoreError::OwnerMismatch.into());
    }
    if blob(1)? != identity.source {
        return Err(AgoraReceiverError::ScopeMismatch);
    }
    let generation = number(blob(2)?)?;
    let next_room = number(blob(3)?)?;
    let bytes = blob(4)?;
    let engine = Engine::recover_checkpoint(
        bytes,
        RecoveryLimits {
            max_snapshot_bytes: limits.max_snapshot_bytes,
            maximum_engine_limits: limits.engine,
        },
    )?;
    if next_room != engine.room_ids().len() as u64 || generation < engine.revision().0 {
        return Err(AgoraReceiverError::InvalidState);
    }
    let mut catalog = BTreeMap::new();
    let mut q = c.prepare("SELECT room,ordinal FROM agora_rooms ORDER BY ordinal")?;
    let mut records = q.query([])?;
    while let Some(row) = records.next()? {
        if catalog.len() >= engine.limits().max_rooms {
            return Err(AgoraReceiverError::InvalidState);
        }
        let id = row
            .get_ref(0)?
            .as_str()
            .map_err(|_| AgoraReceiverError::InvalidState)?;
        if id.len() > engine.limits().max_room_id_bytes {
            return Err(AgoraReceiverError::InvalidState);
        }
        let ordinal = number(
            row.get_ref(1)?
                .as_blob()
                .map_err(|_| AgoraReceiverError::InvalidState)?,
        )?;
        if ordinal != catalog.len() as u64 {
            return Err(AgoraReceiverError::InvalidState);
        }
        if catalog.insert(RoomId::new(id), ordinal).is_some() {
            return Err(AgoraReceiverError::InvalidState);
        }
    }
    if catalog.len() as u64 != next_room || engine.room_ids().any(|id| !catalog.contains_key(id)) {
        return Err(AgoraReceiverError::InvalidState);
    }
    Ok(Loaded {
        engine,
        generation,
        next_room,
        catalog,
        bytes: bytes.len(),
    })
}
fn observe<T>(u: &mut Usage, result: &Result<T, AgoraReceiverError>, replayed: bool) {
    count(&mut u.command_offers, &mut u.overflowed);
    match result {
        Ok(_) if replayed => count(&mut u.replayed, &mut u.overflowed),
        Ok(_) => count(&mut u.committed, &mut u.overflowed),
        Err(AgoraReceiverError::Storage(StoreError::Indeterminate)) => {
            count(&mut u.indeterminate, &mut u.overflowed)
        }
        Err(e) => {
            count(&mut u.refused, &mut u.overflowed);
            if matches!(e, AgoraReceiverError::PublicationConflict) {
                count(&mut u.conflicts, &mut u.overflowed)
            }
        }
    }
}
fn observe_read<T>(u: &mut Usage, result: &Result<T, AgoraReceiverError>) {
    count(&mut u.load_offers, &mut u.overflowed);
    count(
        if result.is_ok() {
            &mut u.loaded
        } else {
            &mut u.load_refused
        },
        &mut u.overflowed,
    )
}
fn classify(e: Refusal) -> RefusalClass {
    match e {
        Refusal::RevisionConflict { .. } => RefusalClass::RevisionConflict,
        Refusal::EventOutboxFull => RefusalClass::OutboxFull,
        Refusal::ParticipantLimit
        | Refusal::RoomLimit
        | Refusal::MemberLimit
        | Refusal::InvitationLimit
        | Refusal::MessageRetentionDisabled
        | Refusal::AddressedParticipantLimit => RefusalClass::Capacity,
        Refusal::UnknownParticipant(_) | Refusal::UnknownRoom(_) => RefusalClass::UnknownResource,
        Refusal::InvitationRequired(_)
        | Refusal::NotAuthorized(_)
        | Refusal::OwnerCannotLeave
        | Refusal::NotMember(_) => RefusalClass::Authority,
        Refusal::DuplicateParticipant(_)
        | Refusal::DuplicateRoom(_)
        | Refusal::AlreadyMember(_) => RefusalClass::Duplicate,
        Refusal::InvalidParticipantId
        | Refusal::InvalidRoomId
        | Refusal::InvalidDisplayName
        | Refusal::InvalidRoomName
        | Refusal::InvalidMessageBody
        | Refusal::NonCanonicalAddressing
        | Refusal::InvalidRoleChange => RefusalClass::InvalidInput,
        Refusal::MessageSequenceExhausted
        | Refusal::RevisionExhausted
        | Refusal::EventSequenceExhausted => RefusalClass::Exhausted,
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
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "moira-agora-fault-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> PathBuf {
            self.0.join("agora.db")
        }
        fn open(&self) -> AgoraReceiver {
            AgoraReceiver::open(
                self.path(),
                AgoraIdentity {
                    owner: [7; 16],
                    source: [10; 16],
                },
                limits(),
            )
            .unwrap()
        }
        fn configured(&self) -> AgoraReceiver {
            let mut r = self.open();
            r.apply(RevisionedCommandV2 {
                expected_revision: r.revision(),
                command: CommandV2::RegisterParticipant(Participant::new(
                    "owner",
                    "Owner",
                    ParticipantKind::Human,
                )),
            })
            .unwrap();
            r.apply(RevisionedCommandV2 {
                expected_revision: r.revision(),
                command: CommandV2::CreateRoom {
                    room_id: "room".into(),
                    name: "Room".into(),
                    owner: "owner".into(),
                    admission: AdmissionPolicy::Open,
                },
            })
            .unwrap();
            r
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for name in ["agora.db", "agora.db-journal", "agora.db.owner.lock"] {
                let _ = fs::remove_file(self.0.join(name));
            }
            let _ = fs::remove_dir(&self.0);
        }
    }
    fn limits() -> AgoraReceiverLimits {
        AgoraReceiverLimits {
            engine: Limits::default(),
            max_snapshot_bytes: 65536,
            max_database_pages: 64,
            timing: Timing::Process,
        }
    }
    fn message() -> ReplyMessage {
        ReplyMessage {
            room_id: "room".into(),
            author: "owner".into(),
            at: LogicalTime(10),
            body: "answer".into(),
        }
    }
    #[test]
    fn publication_commit_contains_both_message_and_receipt_or_neither() {
        for fault in [crate::Fault::BeforeCommit, crate::Fault::AfterCommit] {
            let f = Fixture::new();
            let mut r = f.configured();
            let revision = r.revision();
            let store = r.store_revision();
            r.fault = Some(fault);
            assert!(
                r.publish(EffectId::new([1; 16]), revision, message())
                    .is_err()
            );
            assert!(r.needs_reopen());
            assert_eq!(r.revision(), revision);
            assert_eq!(r.store_revision(), store);
            assert!(matches!(
                r.room(&"room".into()),
                Err(AgoraReceiverError::Storage(StoreError::NeedsReopen))
            ));
            assert_eq!(r.publication_usage().commit_timed_samples, 1);
            assert_eq!(r.publication_usage().committed, 0);
            drop(r);
            let mut r = f.open();
            let result = r
                .publish(EffectId::new([1; 16]), revision, message())
                .unwrap();
            assert_eq!(result.replayed, fault == crate::Fault::AfterCommit);
            assert_eq!(r.room(&"room".into()).unwrap().unwrap().messages.len(), 1);
        }
    }
    #[test]
    fn catalogue_loss_refuses_reopen_and_observation_counters_saturate_without_payloads() {
        let f = Fixture::new();
        let mut r = f.configured();
        r.publication_usage.command_offers = u64::MAX;
        r.publish(EffectId::new([1; 16]), r.revision(), message())
            .unwrap();
        assert_eq!(r.publication_usage().command_offers, u64::MAX);
        assert!(r.publication_usage().overflowed);
        assert_eq!(format!("{r:?}"), "AgoraReceiver(<redacted>)");
        assert!(!format!("{:?}", message()).contains("answer"));
        r.connection.execute("DELETE FROM agora_rooms", []).unwrap();
        drop(r);
        assert!(matches!(
            AgoraReceiver::open(
                f.path(),
                AgoraIdentity {
                    owner: [7; 16],
                    source: [10; 16]
                },
                limits()
            ),
            Err(AgoraReceiverError::InvalidState)
        ));
    }
    #[test]
    fn acknowledgement_commit_loss_recovers_the_exact_prefix_without_claiming_delivery() {
        let f = Fixture::new();
        let mut r = f.configured();
        r.fault = Some(crate::Fault::AfterCommit);
        assert!(matches!(
            r.acknowledge_events(EventSequence(0)),
            Err(AgoraReceiverError::Storage(StoreError::Indeterminate))
        ));
        drop(r);
        let mut r = f.open();
        let result = r.acknowledge_events(EventSequence(0)).unwrap();
        assert!(result.replayed);
        assert_eq!(result.removed, 0);
        assert_eq!(
            r.pending_events().unwrap().first().unwrap().sequence,
            EventSequence(1)
        );
        assert_eq!(r.acknowledgement_usage().commit_timed_samples, 0);
    }
}
