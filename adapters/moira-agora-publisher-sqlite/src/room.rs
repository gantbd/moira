//! SQLite owner for one exact task-agent Agora room checkpoint.

use super::{PublisherStoreErrorV1, decode_u64, map_resource, refusal, refusal_dto, schema_names};
use agora_client::{
    room_command::{CreateRoomReceiptV1, CreateRoomRequestV1},
    v2::ClientStateV2,
};
use agora_protocol::v2::{AdmissionPolicyV2, ClientIdV2, CommandV2, RequestIdV2, RequestV2};
use moira_agora::room::{
    RetainedRoomV1, RoomCheckpointV1, RoomIntentV1, RoomLimitsV1, RoomRecordV1, RoomReplyV1,
    RoomStateStoreV1,
};
use moira_core::task_agent_journey::{JourneyIdV1, OwnerIdV1};
use moira_sqlite::{OwnedResource, validate_sqlite_integrity};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, limits::Limit, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4d415253;
const SCHEMA: &str = "CREATE TABLE agora_room_meta (slot INTEGER PRIMARY KEY CHECK(slot=1), owner BLOB NOT NULL CHECK(length(owner)=16), profile BLOB NOT NULL CHECK(length(profile)=32), revision BLOB NOT NULL CHECK(length(revision)=8), payload BLOB NOT NULL);";

pub type RoomStoreErrorV1 = PublisherStoreErrorV1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoomStoreLimitsV1 {
    pub room: RoomLimitsV1,
    pub max_checkpoint_bytes: usize,
    pub max_database_pages: u32,
}

impl Default for RoomStoreLimitsV1 {
    fn default() -> Self {
        Self {
            room: RoomLimitsV1::default(),
            max_checkpoint_bytes: 16 * 1_024,
            max_database_pages: 32,
        }
    }
}

pub struct RoomSqliteStore {
    connection: Connection,
    resource: OwnedResource,
    expected_owner: [u8; 16],
    expected_client: ClientIdV2,
    limits: RoomStoreLimitsV1,
}

impl RoomSqliteStore {
    pub fn open(
        path: impl AsRef<Path>,
        owner: [u8; 16],
        client: ClientIdV2,
        limits: RoomStoreLimitsV1,
    ) -> Result<Self, RoomStoreErrorV1> {
        validate_limits(limits)?;
        if owner == [0; 16] {
            return Err(RoomStoreErrorV1::InvalidConfig);
        }
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
            .ok_or(RoomStoreErrorV1::InvalidConfig)?;
        connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, sqlite_limit)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON;")?;
        validate_sqlite_integrity(&connection).map_err(map_resource)?;
        let application: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let names = schema_names(&connection)?;
        let fresh = application == 0 && version == 0 && names.is_empty();
        if !fresh && (application != APPLICATION_ID || version != 1 || names != ["agora_room_meta"])
        {
            return Err(RoomStoreErrorV1::ForeignDatabase);
        }
        let profile = profile(limits);
        if fresh {
            let checkpoint = RoomCheckpointV1::initial(client);
            let payload = encode(&checkpoint, owner, limits)?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute(
                "INSERT INTO agora_room_meta VALUES (1,?1,?2,?3,?4)",
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
                .map_err(|_| RoomStoreErrorV1::Indeterminate)?;
            resource.sync_parent().map_err(map_resource)?;
        } else {
            let (retained_owner, retained_profile): (Vec<u8>, Vec<u8>) = connection.query_row(
                "SELECT owner,profile FROM agora_room_meta WHERE slot=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if retained_owner.as_slice() != owner {
                return Err(RoomStoreErrorV1::OwnerMismatch);
            }
            if retained_profile.as_slice() != profile {
                return Err(RoomStoreErrorV1::ProfileMismatch);
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
            return Err(RoomStoreErrorV1::InvalidConfig);
        }
        let mut store = Self {
            connection,
            resource,
            expected_owner: owner,
            expected_client: client,
            limits,
        };
        store.load()?;
        Ok(store)
    }

    fn check_resource(&self) -> Result<(), RoomStoreErrorV1> {
        self.resource.check().map_err(map_resource)
    }
}

impl RoomStateStoreV1 for RoomSqliteStore {
    type Error = RoomStoreErrorV1;

    fn load(&mut self) -> Result<RoomRecordV1, Self::Error> {
        self.check_resource()?;
        let (revision, payload): (Vec<u8>, Vec<u8>) = self.connection.query_row(
            "SELECT revision,payload FROM agora_room_meta WHERE slot=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let revision = decode_u64(&revision)?;
        let checkpoint = decode(&payload, self.expected_owner, self.limits)?;
        if checkpoint.client.client != self.expected_client {
            return Err(RoomStoreErrorV1::ClientMismatch);
        }
        Ok(RoomRecordV1 {
            revision,
            checkpoint,
        })
    }

    fn compare_exchange(
        &mut self,
        expected_revision: u64,
        checkpoint: &RoomCheckpointV1,
    ) -> Result<RoomRecordV1, Self::Error> {
        self.check_resource()?;
        if checkpoint.client.client != self.expected_client {
            return Err(RoomStoreErrorV1::ClientMismatch);
        }
        let current = self.load()?;
        if current.revision != expected_revision {
            return Err(RoomStoreErrorV1::RevisionConflict);
        }
        let revision = expected_revision
            .checked_add(1)
            .ok_or(RoomStoreErrorV1::RevisionExhausted)?;
        let payload = encode(checkpoint, self.expected_owner, self.limits)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE agora_room_meta SET revision=?1,payload=?2 WHERE slot=1 AND revision=?3",
            params![
                revision.to_be_bytes().as_slice(),
                payload,
                expected_revision.to_be_bytes().as_slice()
            ],
        )?;
        if changed != 1 {
            return Err(RoomStoreErrorV1::RevisionConflict);
        }
        transaction
            .commit()
            .map_err(|_| RoomStoreErrorV1::Indeterminate)?;
        Ok(RoomRecordV1 {
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
    journey_owner: [u8; 16],
    journey_sequence: u64,
    expected_journey_revision: u64,
    source: [u8; 16],
    room_sequence: u64,
    room_id: String,
    name: String,
    admission: u8,
    expected_agora_revision: u64,
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
    Created { room_id: String, revision: u64 },
    Refused { code: u8, value: Option<u64> },
}

fn encode(
    checkpoint: &RoomCheckpointV1,
    owner: [u8; 16],
    limits: RoomStoreLimitsV1,
) -> Result<Vec<u8>, RoomStoreErrorV1> {
    validate_checkpoint(checkpoint, owner, limits)?;
    let payload = serde_json::to_vec(&dto(checkpoint)).map_err(|_| RoomStoreErrorV1::Integrity)?;
    if payload.len() > limits.max_checkpoint_bytes {
        return Err(RoomStoreErrorV1::Capacity);
    }
    Ok(payload)
}

fn decode(
    payload: &[u8],
    owner: [u8; 16],
    limits: RoomStoreLimitsV1,
) -> Result<RoomCheckpointV1, RoomStoreErrorV1> {
    if payload.len() > limits.max_checkpoint_bytes {
        return Err(RoomStoreErrorV1::Capacity);
    }
    let dto: CheckpointDto =
        serde_json::from_slice(payload).map_err(|_| RoomStoreErrorV1::Integrity)?;
    if dto.version != 1 {
        return Err(RoomStoreErrorV1::Integrity);
    }
    let checkpoint = checkpoint(dto)?;
    validate_checkpoint(&checkpoint, owner, limits)?;
    Ok(checkpoint)
}

fn dto(value: &RoomCheckpointV1) -> CheckpointDto {
    CheckpointDto {
        version: 1,
        client: value.client.client.0,
        next_request: value.client.next_request,
        pending: value.pending_intent.as_ref().map(intent_dto),
        last: value.last.as_ref().map(|last| LastDto {
            intent: intent_dto(&last.intent),
            reply: match &last.reply {
                RoomReplyV1::Created(receipt) => ReplyDto::Created {
                    room_id: receipt.room_id.clone(),
                    revision: receipt.revision,
                },
                RoomReplyV1::Refused(value) => {
                    let (code, value) = refusal_dto(*value);
                    ReplyDto::Refused { code, value }
                }
            },
        }),
    }
}

fn checkpoint(value: CheckpointDto) -> Result<RoomCheckpointV1, RoomStoreErrorV1> {
    let pending_intent = value.pending.map(intent).transpose()?;
    let pending = pending_intent
        .as_ref()
        .map(|intent| {
            Ok::<_, RoomStoreErrorV1>(RequestV2 {
                request_id: RequestIdV2(
                    value
                        .next_request
                        .checked_sub(1)
                        .ok_or(RoomStoreErrorV1::Integrity)?,
                ),
                command: command(intent),
            })
        })
        .transpose()?;
    let last = value
        .last
        .map(|last| {
            let intent = intent(last.intent)?;
            let reply = match last.reply {
                ReplyDto::Created { room_id, revision } => {
                    RoomReplyV1::Created(CreateRoomReceiptV1 { room_id, revision })
                }
                ReplyDto::Refused { code, value } => RoomReplyV1::Refused(refusal(code, value)?),
            };
            Ok::<_, RoomStoreErrorV1>(RetainedRoomV1 { intent, reply })
        })
        .transpose()?;
    Ok(RoomCheckpointV1 {
        client: ClientStateV2 {
            client: ClientIdV2(value.client),
            next_request: value.next_request,
            pending,
        },
        pending_intent,
        last,
    })
}

fn intent_dto(value: &RoomIntentV1) -> IntentDto {
    IntentDto {
        journey_owner: value.journey.owner.0,
        journey_sequence: value.journey.sequence,
        expected_journey_revision: value.expected_journey_revision,
        source: value.source,
        room_sequence: value.room_sequence,
        room_id: value.request.room_id.clone(),
        name: value.request.name.clone(),
        admission: match value.request.admission {
            AdmissionPolicyV2::Open => 0,
            AdmissionPolicyV2::InviteOnly => 1,
        },
        expected_agora_revision: value.request.expected_revision,
    }
}

fn intent(value: IntentDto) -> Result<RoomIntentV1, RoomStoreErrorV1> {
    Ok(RoomIntentV1 {
        journey: JourneyIdV1 {
            owner: OwnerIdV1(value.journey_owner),
            sequence: value.journey_sequence,
        },
        expected_journey_revision: value.expected_journey_revision,
        source: value.source,
        room_sequence: value.room_sequence,
        request: CreateRoomRequestV1 {
            room_id: value.room_id,
            name: value.name,
            admission: match value.admission {
                0 => AdmissionPolicyV2::Open,
                1 => AdmissionPolicyV2::InviteOnly,
                _ => return Err(RoomStoreErrorV1::Integrity),
            },
            expected_revision: value.expected_agora_revision,
        },
    })
}

fn command(value: &RoomIntentV1) -> CommandV2 {
    CommandV2::CreateRoom {
        room_id: value.request.room_id.clone(),
        name: value.request.name.clone(),
        admission: value.request.admission,
        expected_revision: value.request.expected_revision,
    }
}

fn validate_checkpoint(
    value: &RoomCheckpointV1,
    owner: [u8; 16],
    limits: RoomStoreLimitsV1,
) -> Result<(), RoomStoreErrorV1> {
    match (&value.pending_intent, &value.client.pending) {
        (None, None) => {}
        (Some(intent), Some(request))
            if valid_intent(intent, owner, limits.room)
                && value.client.next_request.checked_sub(1) == Some(request.request_id.0)
                && request.command == command(intent) => {}
        _ => return Err(RoomStoreErrorV1::Integrity),
    }
    if value.last.as_ref().is_some_and(|last| {
        !valid_intent(&last.intent, owner, limits.room) || !reply_matches(&last.intent, &last.reply)
    }) {
        return Err(RoomStoreErrorV1::Integrity);
    }
    if value.pending_intent.is_some() && value.last.is_some() {
        return Err(RoomStoreErrorV1::Integrity);
    }
    Ok(())
}

fn valid_intent(value: &RoomIntentV1, owner: [u8; 16], limits: RoomLimitsV1) -> bool {
    value.journey.owner.0 == owner
        && value.source == owner
        && value.expected_journey_revision > 0
        && value.room_sequence == value.journey.sequence
        && !value.request.room_id.is_empty()
        && value.request.room_id.len() <= limits.max_room_id_bytes
        && !value.request.name.is_empty()
        && value.request.name.len() <= limits.max_name_bytes
}

fn reply_matches(intent: &RoomIntentV1, reply: &RoomReplyV1) -> bool {
    match reply {
        RoomReplyV1::Created(receipt) => {
            receipt.room_id == intent.request.room_id
                && receipt.revision > intent.request.expected_revision
        }
        RoomReplyV1::Refused(_) => true,
    }
}

fn validate_limits(value: RoomStoreLimitsV1) -> Result<(), RoomStoreErrorV1> {
    if value.room.max_room_id_bytes == 0
        || value.room.max_name_bytes == 0
        || value.max_checkpoint_bytes < 256
        || value.max_database_pages < 8
    {
        Err(RoomStoreErrorV1::InvalidConfig)
    } else {
        Ok(())
    }
}

fn profile(value: RoomStoreLimitsV1) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRAARS1");
    hash.update(value.room.max_room_id_bytes.to_be_bytes());
    hash.update(value.room.max_name_bytes.to_be_bytes());
    hash.update(value.max_checkpoint_bytes.to_be_bytes());
    hash.update(value.max_database_pages.to_be_bytes());
    hash.finalize().into()
}
