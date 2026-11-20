//! Durable prepare/outcome coordination for one task-agent Agora room.
//!
//! The exact revision-bound request is retained before dispatch. An uncertain
//! delivery can therefore resume only that request identity; divergent room
//! work remains fenced until the owner outcome is reconciled.

use agora_client::{
    room_command::{
        CreateRoomErrorV1, CreateRoomReceiptV1, CreateRoomRequestV1, create_room,
        required_create_confirmation,
    },
    v2::{ClientErrorV2, ClientStateV2, ClientV2, ServicePortV2},
};
use agora_protocol::v2::{ClientIdV2, CommandV2, RefusalV2, RequestIdV2, RequestV2};
use moira_core::task_agent_journey::{AgoraRoomReceiptV1, JourneyIdV1, JourneySnapshotV1, PhaseV1};
use moira_host::{
    TaskAgentJourneyApplyErrorV1, TaskAgentJourneyApplyV1, TaskAgentJourneyMutationV1,
    TaskAgentJourneyStore,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoomLimitsV1 {
    pub max_room_id_bytes: usize,
    pub max_name_bytes: usize,
}

impl Default for RoomLimitsV1 {
    fn default() -> Self {
        Self {
            max_room_id_bytes: 128,
            max_name_bytes: 256,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomIntentV1 {
    pub journey: JourneyIdV1,
    pub expected_journey_revision: u64,
    pub source: [u8; 16],
    pub room_sequence: u64,
    pub request: CreateRoomRequestV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoomReplyV1 {
    Created(CreateRoomReceiptV1),
    Refused(RefusalV2),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedRoomV1 {
    pub intent: RoomIntentV1,
    pub reply: RoomReplyV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomCheckpointV1 {
    pub client: ClientStateV2,
    pub pending_intent: Option<RoomIntentV1>,
    pub last: Option<RetainedRoomV1>,
}

impl RoomCheckpointV1 {
    pub const fn initial(client: ClientIdV2) -> Self {
        Self {
            client: ClientStateV2 {
                client,
                next_request: 0,
                pending: None,
            },
            pending_intent: None,
            last: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomRecordV1 {
    pub revision: u64,
    pub checkpoint: RoomCheckpointV1,
}

/// A single-owner compare/exchange store. An error may be indeterminate, so a
/// caller must reload instead of assuming the proposed checkpoint was refused.
pub trait RoomStateStoreV1 {
    type Error;

    fn load(&mut self) -> Result<RoomRecordV1, Self::Error>;

    fn compare_exchange(
        &mut self,
        expected_revision: u64,
        checkpoint: &RoomCheckpointV1,
    ) -> Result<RoomRecordV1, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableRoomV1 {
    pub reply: RoomReplyV1,
    pub replayed: bool,
    pub storage_revision: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub enum DurableRoomErrorV1<StoreError, PortError> {
    Store(StoreError),
    InvalidCheckpoint,
    InvalidInput,
    RequestIdExhausted,
    Client(ClientErrorV2<PortError>),
    UnexpectedReply,
}

pub struct DurableRoomProvisioner<S, P> {
    store: S,
    port: Option<P>,
    limits: RoomLimitsV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadyJourneyRoomV1 {
    pub room: DurableRoomV1,
    pub journey_storage_revision: u64,
    pub journey: JourneySnapshotV1,
    pub journey_replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JourneyRoomProgressV1 {
    Ready(Box<ReadyJourneyRoomV1>),
    Refused(DurableRoomV1),
}

#[derive(Debug)]
pub enum JourneyRoomErrorV1<RoomStoreError, PortError, JourneyStoreError> {
    JourneyStoreNeedsReopen,
    JourneyNotFound,
    JourneyNotReady,
    JourneyMismatch,
    StorageRevisionConflict {
        expected: u64,
        actual: u64,
    },
    Provision(DurableRoomErrorV1<RoomStoreError, PortError>),
    Bind {
        room: DurableRoomV1,
        error: TaskAgentJourneyApplyErrorV1<JourneyStoreError>,
    },
    UnexpectedOutcome {
        room: DurableRoomV1,
    },
}

pub type JourneyRoomResultV1<RoomStoreError, PortError, JourneyStoreError> =
    Result<JourneyRoomProgressV1, JourneyRoomErrorV1<RoomStoreError, PortError, JourneyStoreError>>;

/// Provision the exact owner-derived room and retain its relationship receipt
/// in the journey store. A successful Agora result is returned inside every
/// later bind failure; calling this again with the same intent replays the
/// durable room receipt and performs no second Agora mutation.
pub fn provision_and_bind_journey_room<J, S, P>(
    journey_store: &mut J,
    provisioner: &mut DurableRoomProvisioner<S, P>,
    expected_journey_storage_revision: u64,
    intent: RoomIntentV1,
) -> JourneyRoomResultV1<S::Error, P::Error, J::Error>
where
    J: TaskAgentJourneyStore,
    S: RoomStateStoreV1,
    P: ServicePortV2,
{
    if journey_store.needs_reopen() {
        return Err(JourneyRoomErrorV1::JourneyStoreNeedsReopen);
    }
    let actual = journey_store.storage_revision();
    if actual != expected_journey_storage_revision {
        return Err(JourneyRoomErrorV1::StorageRevisionConflict {
            expected: expected_journey_storage_revision,
            actual,
        });
    }
    let journey = journey_store
        .journey(intent.journey)
        .ok_or(JourneyRoomErrorV1::JourneyNotFound)?;
    if intent.source != journey.workflow.owner.0
        || intent.room_sequence != journey.workflow.sequence
    {
        return Err(JourneyRoomErrorV1::JourneyMismatch);
    }
    if journey.phase == PhaseV1::RoomReady {
        let Some(binding) = journey.room else {
            return Err(JourneyRoomErrorV1::JourneyMismatch);
        };
        if binding.expected_journey_revision != intent.expected_journey_revision
            || binding.source != intent.source
            || binding.room_sequence != intent.room_sequence
        {
            return Err(JourneyRoomErrorV1::JourneyMismatch);
        }
        let room = provisioner
            .provision(intent)
            .map_err(JourneyRoomErrorV1::Provision)?;
        if !matches!(room.reply, RoomReplyV1::Created(_)) {
            return Err(JourneyRoomErrorV1::UnexpectedOutcome { room });
        }
        return Ok(JourneyRoomProgressV1::Ready(Box::new(ReadyJourneyRoomV1 {
            room,
            journey_storage_revision: actual,
            journey,
            journey_replayed: true,
        })));
    }
    if journey.phase != PhaseV1::TaskAttemptReady
        || journey.task_attempt.is_none()
        || journey.revision != intent.expected_journey_revision
    {
        return Err(JourneyRoomErrorV1::JourneyNotReady);
    }

    let room = provisioner
        .provision(intent.clone())
        .map_err(JourneyRoomErrorV1::Provision)?;
    let RoomReplyV1::Created(receipt) = &room.reply else {
        return Ok(JourneyRoomProgressV1::Refused(room));
    };
    let applied = journey_store
        .apply(
            Some(expected_journey_storage_revision),
            TaskAgentJourneyMutationV1::BindRoom {
                journey: intent.journey,
                receipt: AgoraRoomReceiptV1 {
                    source: intent.source,
                    room_sequence: intent.room_sequence,
                    agora_revision: receipt.revision,
                },
            },
        )
        .map_err(|error| JourneyRoomErrorV1::Bind {
            room: room.clone(),
            error,
        })?;
    let TaskAgentJourneyApplyV1::Room(bound) = applied.outcome else {
        return Err(JourneyRoomErrorV1::UnexpectedOutcome { room });
    };
    let exact = bound.journey.id == intent.journey
        && bound.journey.room.is_some_and(|binding| {
            binding.source == intent.source
                && binding.room_sequence == intent.room_sequence
                && binding.agora_revision == receipt.revision
        });
    if !exact {
        return Err(JourneyRoomErrorV1::UnexpectedOutcome { room });
    }
    Ok(JourneyRoomProgressV1::Ready(Box::new(ReadyJourneyRoomV1 {
        room,
        journey_storage_revision: applied.storage_revision,
        journey: bound.journey,
        journey_replayed: bound.replayed,
    })))
}

impl<S, P> DurableRoomProvisioner<S, P>
where
    S: RoomStateStoreV1,
    P: ServicePortV2,
{
    pub fn new(
        store: S,
        port: P,
        limits: RoomLimitsV1,
    ) -> Result<Self, DurableRoomErrorV1<S::Error, P::Error>> {
        validate_limits(limits)?;
        Ok(Self {
            store,
            port: Some(port),
            limits,
        })
    }

    pub fn provision(
        &mut self,
        intent: RoomIntentV1,
    ) -> Result<DurableRoomV1, DurableRoomErrorV1<S::Error, P::Error>> {
        if !valid_intent(self.limits, &intent) {
            return Err(DurableRoomErrorV1::InvalidInput);
        }
        let mut retained = self.store.load().map_err(DurableRoomErrorV1::Store)?;
        validate_checkpoint(self.limits, &retained.checkpoint)?;
        if let Some(last) = &retained.checkpoint.last {
            if last.intent == intent {
                return Ok(DurableRoomV1 {
                    reply: last.reply.clone(),
                    replayed: true,
                    storage_revision: retained.revision,
                });
            }
            return Err(DurableRoomErrorV1::InvalidCheckpoint);
        }

        if retained.checkpoint.pending_intent.is_none() {
            let next = retained
                .checkpoint
                .client
                .next_request
                .checked_add(1)
                .ok_or(DurableRoomErrorV1::RequestIdExhausted)?;
            let request = RequestV2 {
                request_id: RequestIdV2(retained.checkpoint.client.next_request),
                command: room_command(&intent),
            };
            let mut prepared = retained.checkpoint.clone();
            prepared.client.next_request = next;
            prepared.client.pending = Some(request);
            prepared.pending_intent = Some(intent.clone());
            retained = self
                .store
                .compare_exchange(retained.revision, &prepared)
                .map_err(DurableRoomErrorV1::Store)?;
            if retained.checkpoint != prepared {
                return Err(DurableRoomErrorV1::InvalidCheckpoint);
            }
        } else if retained.checkpoint.pending_intent.as_ref() != Some(&intent) {
            return Err(DurableRoomErrorV1::InvalidCheckpoint);
        }

        validate_checkpoint(self.limits, &retained.checkpoint)?;
        let port = self
            .port
            .take()
            .ok_or(DurableRoomErrorV1::InvalidCheckpoint)?;
        let mut client = ClientV2::restore(retained.checkpoint.client.clone(), port)
            .map_err(|_| DurableRoomErrorV1::InvalidCheckpoint)?;
        let confirmation = required_create_confirmation(&intent.request);
        let outcome = create_room(&mut client, intent.request.clone(), &confirmation);
        let state = client.state();
        let (port, _) = client.into_parts();
        self.port = Some(port);
        let reply = match outcome {
            Ok(receipt) => RoomReplyV1::Created(receipt),
            Err(CreateRoomErrorV1::Refused(refusal)) => RoomReplyV1::Refused(refusal),
            Err(CreateRoomErrorV1::Client(error)) => {
                return Err(DurableRoomErrorV1::Client(error));
            }
            Err(CreateRoomErrorV1::ConfirmationRefused { .. }) => {
                return Err(DurableRoomErrorV1::InvalidCheckpoint);
            }
            Err(CreateRoomErrorV1::UnexpectedReply) => {
                return Err(DurableRoomErrorV1::UnexpectedReply);
            }
        };
        let completed = RoomCheckpointV1 {
            client: state,
            pending_intent: None,
            last: Some(RetainedRoomV1 {
                intent,
                reply: reply.clone(),
            }),
        };
        let committed = self
            .store
            .compare_exchange(retained.revision, &completed)
            .map_err(DurableRoomErrorV1::Store)?;
        if committed.checkpoint != completed {
            return Err(DurableRoomErrorV1::InvalidCheckpoint);
        }
        Ok(DurableRoomV1 {
            reply,
            replayed: false,
            storage_revision: committed.revision,
        })
    }

    pub fn into_parts(self) -> (S, Option<P>) {
        (self.store, self.port)
    }
}

fn validate_limits<StoreError, PortError>(
    limits: RoomLimitsV1,
) -> Result<(), DurableRoomErrorV1<StoreError, PortError>> {
    if limits.max_room_id_bytes == 0 || limits.max_name_bytes == 0 {
        Err(DurableRoomErrorV1::InvalidInput)
    } else {
        Ok(())
    }
}

fn valid_intent(limits: RoomLimitsV1, intent: &RoomIntentV1) -> bool {
    intent.journey.owner.0 != [0; 16]
        && intent.expected_journey_revision > 0
        && intent.source == intent.journey.owner.0
        && intent.room_sequence == intent.journey.sequence
        && !intent.request.room_id.is_empty()
        && intent.request.room_id.len() <= limits.max_room_id_bytes
        && !intent.request.name.is_empty()
        && intent.request.name.len() <= limits.max_name_bytes
}

fn room_command(intent: &RoomIntentV1) -> CommandV2 {
    CommandV2::CreateRoom {
        room_id: intent.request.room_id.clone(),
        name: intent.request.name.clone(),
        admission: intent.request.admission,
        expected_revision: intent.request.expected_revision,
    }
}

fn validate_checkpoint<StoreError, PortError>(
    limits: RoomLimitsV1,
    checkpoint: &RoomCheckpointV1,
) -> Result<(), DurableRoomErrorV1<StoreError, PortError>> {
    match (&checkpoint.pending_intent, &checkpoint.client.pending) {
        (None, None) => {}
        (Some(intent), Some(request))
            if valid_intent(limits, intent)
                && checkpoint.client.next_request.checked_sub(1) == Some(request.request_id.0)
                && request.command == room_command(intent) => {}
        _ => return Err(DurableRoomErrorV1::InvalidCheckpoint),
    }
    if checkpoint.last.as_ref().is_some_and(|last| {
        !valid_intent(limits, &last.intent) || !reply_matches(&last.intent, &last.reply)
    }) {
        return Err(DurableRoomErrorV1::InvalidCheckpoint);
    }
    Ok(())
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
