//! Separately owned SQLite checkpoints for Agora effect replay.

#![forbid(unsafe_code)]
#![cfg(unix)]

pub mod room;

use agora_client::v2::ClientStateV2;
use agora_protocol::v2::{ClientIdV2, RefusalV2, RequestIdV2, RequestV2};
use moira_agora::{
    PublicationIntentV1, PublicationLimitsV1, PublicationReplyV1, PublishedV1,
    ResponsePublisherStateV1,
    durable::{
        PublisherCheckpointV1, PublisherRecordV1, PublisherStateStoreV1, RetainedPublicationV1,
    },
};
use moira_sqlite::{OwnedResource, validate_sqlite_integrity};
use rusqlite::{Connection, ErrorCode, OpenFlags, TransactionBehavior, limits::Limit, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::ErrorKind, path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4d415053;
const SCHEMA: &str = "CREATE TABLE agora_publisher_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), profile BLOB NOT NULL CHECK(length(profile)=32), revision BLOB NOT NULL CHECK(length(revision)=8), payload BLOB NOT NULL);";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublisherStoreLimitsV1 {
    pub publication: PublicationLimitsV1,
    pub max_checkpoint_bytes: usize,
    pub max_database_pages: u32,
}

impl Default for PublisherStoreLimitsV1 {
    fn default() -> Self {
        Self {
            publication: PublicationLimitsV1::default(),
            max_checkpoint_bytes: 64 * 1_024,
            max_database_pages: 64,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublisherStoreErrorV1 {
    InvalidConfig,
    Busy,
    ForeignDatabase,
    OwnerMismatch,
    ProfileMismatch,
    ClientMismatch,
    RevisionConflict,
    RevisionExhausted,
    Integrity,
    Capacity,
    Io(ErrorKind),
    Sqlite(ErrorCode),
    Indeterminate,
}

impl std::fmt::Display for PublisherStoreErrorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Moira Agora checkpoint SQLite: {self:?}")
    }
}

impl std::error::Error for PublisherStoreErrorV1 {}

impl From<rusqlite::Error> for PublisherStoreErrorV1 {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            Some(code) => Self::Sqlite(code),
            None => Self::Integrity,
        }
    }
}

pub struct PublisherSqliteStore {
    connection: Connection,
    resource: OwnedResource,
    expected_client: ClientIdV2,
    limits: PublisherStoreLimitsV1,
}

impl PublisherSqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        client: ClientIdV2,
        limits: PublisherStoreLimitsV1,
    ) -> Result<Self, PublisherStoreErrorV1> {
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
        let sqlite_limit = limits
            .max_checkpoint_bytes
            .checked_add(1_024)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or(PublisherStoreErrorV1::InvalidConfig)?;
        connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, sqlite_limit)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON;")?;
        validate_sqlite_integrity(&connection).map_err(map_resource)?;
        let application: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let names = schema_names(&connection)?;
        let fresh = application == 0 && version == 0 && names.is_empty();
        if !fresh
            && (application != APPLICATION_ID || version != 1 || names != ["agora_publisher_meta"])
        {
            return Err(PublisherStoreErrorV1::ForeignDatabase);
        }
        let profile = profile(limits);
        if fresh {
            let checkpoint = PublisherCheckpointV1::initial(client);
            let payload = encode(&checkpoint, limits)?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute(
                "INSERT INTO agora_publisher_meta VALUES (1,?1,?2,?3,?4)",
                params![
                    owner.as_slice(),
                    profile.as_slice(),
                    0_u64.to_be_bytes().as_slice(),
                    payload
                ],
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction
                .commit()
                .map_err(|_| PublisherStoreErrorV1::Indeterminate)?;
            resource.sync_parent().map_err(map_resource)?;
        } else {
            let (retained_owner, retained_profile): (Vec<u8>, Vec<u8>) = connection.query_row(
                "SELECT owner,profile FROM agora_publisher_meta WHERE slot=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if retained_owner.as_slice() != owner {
                return Err(PublisherStoreErrorV1::OwnerMismatch);
            }
            if retained_profile.as_slice() != profile {
                return Err(PublisherStoreErrorV1::ProfileMismatch);
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
            return Err(PublisherStoreErrorV1::InvalidConfig);
        }
        let mut store = Self {
            connection,
            resource,
            expected_client: client,
            limits,
        };
        store.load()?;
        Ok(store)
    }

    fn check_resource(&self) -> Result<(), PublisherStoreErrorV1> {
        self.resource.check().map_err(map_resource)
    }
}

impl PublisherStateStoreV1 for PublisherSqliteStore {
    type Error = PublisherStoreErrorV1;

    fn load(&mut self) -> Result<PublisherRecordV1, Self::Error> {
        self.check_resource()?;
        let (revision, payload): (Vec<u8>, Vec<u8>) = self.connection.query_row(
            "SELECT revision,payload FROM agora_publisher_meta WHERE slot=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let revision = decode_u64(&revision)?;
        let checkpoint = decode(&payload, self.limits)?;
        if checkpoint.client.client.client != self.expected_client {
            return Err(PublisherStoreErrorV1::ClientMismatch);
        }
        Ok(PublisherRecordV1 {
            revision,
            checkpoint,
        })
    }

    fn compare_exchange(
        &mut self,
        expected_revision: u64,
        checkpoint: &PublisherCheckpointV1,
    ) -> Result<PublisherRecordV1, Self::Error> {
        self.check_resource()?;
        if checkpoint.client.client.client != self.expected_client {
            return Err(PublisherStoreErrorV1::ClientMismatch);
        }
        let current = self.load()?;
        if current.revision != expected_revision {
            return Err(PublisherStoreErrorV1::RevisionConflict);
        }
        let revision = expected_revision
            .checked_add(1)
            .ok_or(PublisherStoreErrorV1::RevisionExhausted)?;
        let payload = encode(checkpoint, self.limits)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE agora_publisher_meta SET revision=?1,payload=?2 WHERE slot=1 AND revision=?3",
            params![
                revision.to_be_bytes().as_slice(),
                payload,
                expected_revision.to_be_bytes().as_slice()
            ],
        )?;
        if changed != 1 {
            return Err(PublisherStoreErrorV1::RevisionConflict);
        }
        transaction
            .commit()
            .map_err(|_| PublisherStoreErrorV1::Indeterminate)?;
        Ok(PublisherRecordV1 {
            revision,
            checkpoint: checkpoint.clone(),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointDto {
    version: u8,
    client: u64,
    next_request: u64,
    pending: Option<IntentDto>,
    last: Option<LastDto>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentDto {
    effect: [u8; 16],
    source: [u8; 16],
    room_sequence: u64,
    room_id: String,
    expected_agora_revision: u64,
    logical_time: u64,
    body: String,
    addressed_to: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LastDto {
    intent: IntentDto,
    reply: ReplyDto,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReplyDto {
    Published {
        message: [u8; 16],
        message_sequence: u64,
        agora_revision: u64,
    },
    Refused {
        code: u8,
        value: Option<u64>,
    },
}

fn encode(
    checkpoint: &PublisherCheckpointV1,
    limits: PublisherStoreLimitsV1,
) -> Result<Vec<u8>, PublisherStoreErrorV1> {
    validate_checkpoint(checkpoint, limits)?;
    let dto = dto(checkpoint);
    let payload = serde_json::to_vec(&dto).map_err(|_| PublisherStoreErrorV1::Integrity)?;
    if payload.len() > limits.max_checkpoint_bytes {
        return Err(PublisherStoreErrorV1::Capacity);
    }
    Ok(payload)
}

fn decode(
    payload: &[u8],
    limits: PublisherStoreLimitsV1,
) -> Result<PublisherCheckpointV1, PublisherStoreErrorV1> {
    if payload.len() > limits.max_checkpoint_bytes {
        return Err(PublisherStoreErrorV1::Capacity);
    }
    let dto: CheckpointDto =
        serde_json::from_slice(payload).map_err(|_| PublisherStoreErrorV1::Integrity)?;
    if dto.version != 1 {
        return Err(PublisherStoreErrorV1::Integrity);
    }
    let checkpoint = checkpoint(dto)?;
    validate_checkpoint(&checkpoint, limits)?;
    Ok(checkpoint)
}

fn dto(value: &PublisherCheckpointV1) -> CheckpointDto {
    CheckpointDto {
        version: 1,
        client: value.client.client.client.0,
        next_request: value.client.client.next_request,
        pending: value.client.pending_intent.as_ref().map(intent_dto),
        last: value.checkpoint_last_dto(),
    }
}

trait LastDtoExt {
    fn checkpoint_last_dto(&self) -> Option<LastDto>;
}

impl LastDtoExt for PublisherCheckpointV1 {
    fn checkpoint_last_dto(&self) -> Option<LastDto> {
        self.last.as_ref().map(|last| LastDto {
            intent: intent_dto(&last.intent),
            reply: match last.reply {
                PublicationReplyV1::Published(published) => ReplyDto::Published {
                    message: published.message_id.into_bytes(),
                    message_sequence: published.receipt.message.message_sequence,
                    agora_revision: published.receipt.agora_revision,
                },
                PublicationReplyV1::Refused(refusal) => {
                    let (code, value) = refusal_dto(refusal);
                    ReplyDto::Refused { code, value }
                }
            },
        })
    }
}

fn checkpoint(value: CheckpointDto) -> Result<PublisherCheckpointV1, PublisherStoreErrorV1> {
    let pending_intent = value.pending.map(intent);
    let pending = pending_intent
        .as_ref()
        .map(|intent| {
            Ok::<_, PublisherStoreErrorV1>(RequestV2 {
                request_id: RequestIdV2(
                    value
                        .next_request
                        .checked_sub(1)
                        .ok_or(PublisherStoreErrorV1::Integrity)?,
                ),
                command: command(intent),
            })
        })
        .transpose()?;
    let last = value
        .last
        .map(|last| {
            let intent = intent(last.intent);
            let reply = match last.reply {
                ReplyDto::Published {
                    message,
                    message_sequence,
                    agora_revision,
                } => PublicationReplyV1::Published(PublishedV1 {
                    receipt: moira_protocol::PublicationReceiptV1 {
                        effect: intent.effect,
                        message: moira_protocol::MessageRefV1 {
                            source: intent.source,
                            room_sequence: intent.room_sequence,
                            message_sequence,
                        },
                        agora_revision,
                    },
                    message_id: agora_protocol::v2::MessageIdV1::new(message),
                }),
                ReplyDto::Refused { code, value } => {
                    PublicationReplyV1::Refused(refusal(code, value)?)
                }
            };
            Ok::<_, PublisherStoreErrorV1>(RetainedPublicationV1 { intent, reply })
        })
        .transpose()?;
    Ok(PublisherCheckpointV1 {
        client: ResponsePublisherStateV1 {
            client: ClientStateV2 {
                client: ClientIdV2(value.client),
                next_request: value.next_request,
                pending,
            },
            pending_intent,
        },
        last,
    })
}

fn intent_dto(value: &PublicationIntentV1) -> IntentDto {
    IntentDto {
        effect: value.effect.0,
        source: value.source,
        room_sequence: value.room_sequence,
        room_id: value.room_id.clone(),
        expected_agora_revision: value.expected_agora_revision,
        logical_time: value.logical_time,
        body: value.body.clone(),
        addressed_to: value.addressed_to.clone(),
    }
}

fn intent(value: IntentDto) -> PublicationIntentV1 {
    PublicationIntentV1 {
        effect: moira_protocol::EffectIdV1(value.effect),
        source: value.source,
        room_sequence: value.room_sequence,
        room_id: value.room_id,
        expected_agora_revision: value.expected_agora_revision,
        logical_time: value.logical_time,
        body: value.body,
        addressed_to: value.addressed_to,
    }
}

fn command(value: &PublicationIntentV1) -> agora_protocol::v2::CommandV2 {
    agora_protocol::v2::CommandV2::SpeakIdentified {
        room_id: value.room_id.clone(),
        expected_revision: value.expected_agora_revision,
        logical_time: value.logical_time,
        body: value.body.clone(),
        addressed_to: value.addressed_to.clone(),
    }
}

fn validate_checkpoint(
    value: &PublisherCheckpointV1,
    limits: PublisherStoreLimitsV1,
) -> Result<(), PublisherStoreErrorV1> {
    match (&value.client.pending_intent, &value.client.client.pending) {
        (None, None) => {}
        (Some(intent), Some(request))
            if valid_intent(intent, limits.publication)
                && value.client.client.next_request.checked_sub(1)
                    == Some(request.request_id.0)
                && request.command == command(intent) => {}
        _ => return Err(PublisherStoreErrorV1::Integrity),
    }
    if value.last.as_ref().is_some_and(|last| {
        !valid_intent(&last.intent, limits.publication) || !reply_matches(&last.intent, last.reply)
    }) {
        return Err(PublisherStoreErrorV1::Integrity);
    }
    Ok(())
}

fn reply_matches(intent: &PublicationIntentV1, reply: PublicationReplyV1) -> bool {
    match reply {
        PublicationReplyV1::Published(published) => {
            published.receipt.effect == intent.effect
                && published.receipt.message.source == intent.source
                && published.receipt.message.room_sequence == intent.room_sequence
        }
        PublicationReplyV1::Refused(_) => true,
    }
}

fn valid_intent(value: &PublicationIntentV1, limits: PublicationLimitsV1) -> bool {
    !value.room_id.is_empty()
        && value.room_id.len() <= limits.max_room_id_bytes
        && !value.body.is_empty()
        && value.body.len() <= limits.max_body_bytes
        && value.addressed_to.len() <= limits.max_addressed_participants
        && value.addressed_to.iter().all(|participant| {
            !participant.is_empty() && participant.len() <= limits.max_participant_id_bytes
        })
}

fn refusal_dto(value: RefusalV2) -> (u8, Option<u64>) {
    use RefusalV2::*;
    match value {
        Stale { actual_revision } => (5, Some(actual_revision)),
        MessageThrottled { retry_after_ms } => (41, Some(retry_after_ms)),
        other => (unit_refusal_code(other), None),
    }
}

fn unit_refusal_code(value: RefusalV2) -> u8 {
    use RefusalV2::*;
    match value {
        UnknownClient => 0,
        UnknownParticipant => 1,
        UnknownRoom => 2,
        NotMember => 3,
        NotAuthorized => 4,
        RequestConflict => 6,
        ClientLimit => 7,
        RequestLimit => 8,
        CursorLimit => 9,
        InvalidCursor => 10,
        InvalidInput => 11,
        Capacity => 12,
        Exhausted => 13,
        Unavailable => 14,
        Banned => 15,
        ThreadArchived => 16,
        UnknownTopic => 17,
        TopicResolved => 18,
        InvalidTopicSupersession => 19,
        TopicMustBeResolved => 20,
        TopicAliasLimit => 21,
        UnknownTopicAlias => 22,
        InvalidTopicTag => 23,
        DuplicateTopicTag => 24,
        TopicTagLimit => 25,
        UnknownTopicTag => 26,
        InvalidReadScope => 27,
        InvalidReadTarget => 28,
        ReadPositionRegression => 29,
        ImmutableConversationCohort => 30,
        InvalidAttentionScope => 31,
        InvalidPinChange => 32,
        InvalidSaveChange => 33,
        InvalidPoll => 34,
        PollLimit => 35,
        UnknownPoll => 36,
        NotPollEligible => 37,
        PollClosed => 38,
        InvalidPollChange => 39,
        PostingRestricted => 40,
        InvalidMessageReport => 42,
        UnknownMessageReport => 43,
        MessageReportLimit => 44,
        InvalidMessageReportChange => 45,
        Stale { .. } | MessageThrottled { .. } => unreachable!("payload refusal"),
    }
}

fn refusal(code: u8, value: Option<u64>) -> Result<RefusalV2, PublisherStoreErrorV1> {
    use RefusalV2::*;
    Ok(match (code, value) {
        (0, None) => UnknownClient,
        (1, None) => UnknownParticipant,
        (2, None) => UnknownRoom,
        (3, None) => NotMember,
        (4, None) => NotAuthorized,
        (5, Some(actual_revision)) => Stale { actual_revision },
        (6, None) => RequestConflict,
        (7, None) => ClientLimit,
        (8, None) => RequestLimit,
        (9, None) => CursorLimit,
        (10, None) => InvalidCursor,
        (11, None) => InvalidInput,
        (12, None) => Capacity,
        (13, None) => Exhausted,
        (14, None) => Unavailable,
        (15, None) => Banned,
        (16, None) => ThreadArchived,
        (17, None) => UnknownTopic,
        (18, None) => TopicResolved,
        (19, None) => InvalidTopicSupersession,
        (20, None) => TopicMustBeResolved,
        (21, None) => TopicAliasLimit,
        (22, None) => UnknownTopicAlias,
        (23, None) => InvalidTopicTag,
        (24, None) => DuplicateTopicTag,
        (25, None) => TopicTagLimit,
        (26, None) => UnknownTopicTag,
        (27, None) => InvalidReadScope,
        (28, None) => InvalidReadTarget,
        (29, None) => ReadPositionRegression,
        (30, None) => ImmutableConversationCohort,
        (31, None) => InvalidAttentionScope,
        (32, None) => InvalidPinChange,
        (33, None) => InvalidSaveChange,
        (34, None) => InvalidPoll,
        (35, None) => PollLimit,
        (36, None) => UnknownPoll,
        (37, None) => NotPollEligible,
        (38, None) => PollClosed,
        (39, None) => InvalidPollChange,
        (40, None) => PostingRestricted,
        (41, Some(retry_after_ms)) => MessageThrottled { retry_after_ms },
        (42, None) => InvalidMessageReport,
        (43, None) => UnknownMessageReport,
        (44, None) => MessageReportLimit,
        (45, None) => InvalidMessageReportChange,
        _ => return Err(PublisherStoreErrorV1::Integrity),
    })
}

fn validate_limits(value: PublisherStoreLimitsV1) -> Result<(), PublisherStoreErrorV1> {
    let publication = value.publication;
    if publication.max_room_id_bytes == 0
        || publication.max_body_bytes == 0
        || publication.max_addressed_participants == 0
        || publication.max_participant_id_bytes == 0
        || value.max_checkpoint_bytes < 256
        || value.max_database_pages < 8
    {
        Err(PublisherStoreErrorV1::InvalidConfig)
    } else {
        Ok(())
    }
}

fn profile(value: PublisherStoreLimitsV1) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRAAPS1");
    for bound in [
        value.publication.max_room_id_bytes,
        value.publication.max_body_bytes,
        value.publication.max_addressed_participants,
        value.publication.max_participant_id_bytes,
        value.max_checkpoint_bytes,
    ] {
        hash.update(u64::try_from(bound).unwrap_or(u64::MAX).to_be_bytes());
    }
    hash.update(value.max_database_pages.to_be_bytes());
    hash.finalize().into()
}

fn decode_u64(value: &[u8]) -> Result<u64, PublisherStoreErrorV1> {
    Ok(u64::from_be_bytes(
        value
            .try_into()
            .map_err(|_| PublisherStoreErrorV1::Integrity)?,
    ))
}

fn schema_names(connection: &Connection) -> Result<Vec<String>, PublisherStoreErrorV1> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type IN ('table','index') AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?)
}

fn map_resource(error: moira_sqlite::StoreError) -> PublisherStoreErrorV1 {
    use moira_sqlite::StoreError;
    match error {
        StoreError::Busy => PublisherStoreErrorV1::Busy,
        StoreError::Io(kind) => PublisherStoreErrorV1::Io(kind),
        StoreError::LostResource => PublisherStoreErrorV1::Integrity,
        _ => PublisherStoreErrorV1::InvalidConfig,
    }
}
