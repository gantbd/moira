//! Publishes Moira response intents through Agora's public V2 client.
//!
//! Agora owns rooms, participants, messages and request replay. Moira owns the
//! workflow effect and the deployment's stable source/room correspondence.

#![forbid(unsafe_code)]

use agora_client::v2::{ClientErrorV2, ClientStateV2, ClientV2, ServicePortV2};
use agora_protocol::v2::{
    ClientIdV2, CommandV2, MessageIdV1, OutcomeV2, RefusalV2, ReplyV2, RequestV2,
};
use moira_protocol::{EffectIdV1, MessageRefV1, PublicationReceiptV1};

pub mod durable;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicationLimitsV1 {
    pub max_room_id_bytes: usize,
    pub max_body_bytes: usize,
    pub max_addressed_participants: usize,
    pub max_participant_id_bytes: usize,
}

impl Default for PublicationLimitsV1 {
    fn default() -> Self {
        Self {
            max_room_id_bytes: 128,
            max_body_bytes: 16 * 1_024,
            max_addressed_participants: 64,
            max_participant_id_bytes: 128,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct PublicationIntentV1 {
    pub effect: EffectIdV1,
    pub source: [u8; 16],
    pub room_sequence: u64,
    pub room_id: String,
    pub expected_agora_revision: u64,
    pub logical_time: u64,
    pub body: String,
    pub addressed_to: Vec<String>,
}

impl std::fmt::Debug for PublicationIntentV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PublicationIntentV1")
            .field("effect", &self.effect)
            .field("source", &"<opaque>")
            .field("room_sequence", &self.room_sequence)
            .field("room_id_bytes", &self.room_id.len())
            .field("expected_agora_revision", &self.expected_agora_revision)
            .field("logical_time", &self.logical_time)
            .field("body_bytes", &self.body.len())
            .field("addressed_count", &self.addressed_to.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublishedV1 {
    pub receipt: PublicationReceiptV1,
    pub message_id: MessageIdV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationReplyV1 {
    Published(PublishedV1),
    Refused(RefusalV2),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublishErrorV1<E> {
    InvalidLimits,
    InvalidInput,
    InvalidState,
    UnresolvedPublication,
    Client(ClientErrorV2<E>),
    UnexpectedReply,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ResponsePublisherStateV1 {
    pub client: ClientStateV2,
    pub pending_intent: Option<PublicationIntentV1>,
}

impl std::fmt::Debug for ResponsePublisherStateV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResponsePublisherStateV1")
            .field("client", &self.client)
            .field("has_pending_intent", &self.pending_intent.is_some())
            .finish()
    }
}

pub struct ResponsePublisher<P> {
    client: ClientV2<P>,
    limits: PublicationLimitsV1,
    pending: Option<PublicationIntentV1>,
}

impl<P: ServicePortV2> ResponsePublisher<P> {
    pub fn new(
        client: ClientIdV2,
        port: P,
        limits: PublicationLimitsV1,
    ) -> Result<Self, PublishErrorV1<P::Error>> {
        if limits.max_room_id_bytes == 0
            || limits.max_body_bytes == 0
            || limits.max_addressed_participants == 0
            || limits.max_participant_id_bytes == 0
        {
            return Err(PublishErrorV1::InvalidLimits);
        }
        Ok(Self {
            client: ClientV2::new(client, port),
            limits,
            pending: None,
        })
    }

    pub fn restore(
        port: P,
        limits: PublicationLimitsV1,
        state: ResponsePublisherStateV1,
    ) -> Result<Self, PublishErrorV1<P::Error>> {
        validate_limits(limits)?;
        match (&state.pending_intent, &state.client.pending) {
            (None, None) => {}
            (Some(intent), Some(request))
                if valid_intent(limits, intent)
                    && request.command == publication_command(intent) => {}
            _ => return Err(PublishErrorV1::InvalidState),
        }
        let client =
            ClientV2::restore(state.client, port).map_err(|_| PublishErrorV1::InvalidState)?;
        Ok(Self {
            client,
            limits,
            pending: state.pending_intent,
        })
    }

    pub fn publish(
        &mut self,
        intent: PublicationIntentV1,
    ) -> Result<PublicationReplyV1, PublishErrorV1<P::Error>> {
        if !self.valid(&intent) {
            return Err(PublishErrorV1::InvalidInput);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending != &intent)
        {
            return Err(PublishErrorV1::UnresolvedPublication);
        }
        let command = publication_command(&intent);
        let reply = match self.client.execute(command) {
            Ok(reply) => reply,
            Err(error @ ClientErrorV2::OutcomeUnknown(_)) => {
                self.pending = Some(intent);
                return Err(PublishErrorV1::Client(error));
            }
            Err(error) => return Err(PublishErrorV1::Client(error)),
        };
        self.pending = None;
        match reply {
            ReplyV2::Accepted(OutcomeV2::IdentifiedMessageAccepted {
                revision,
                message_id,
                sequence,
            }) => Ok(PublicationReplyV1::Published(PublishedV1 {
                receipt: PublicationReceiptV1 {
                    effect: intent.effect,
                    message: MessageRefV1 {
                        source: intent.source,
                        room_sequence: intent.room_sequence,
                        message_sequence: sequence,
                    },
                    agora_revision: revision,
                },
                message_id,
            })),
            ReplyV2::Refused(refusal) => Ok(PublicationReplyV1::Refused(refusal)),
            ReplyV2::Accepted(_) => Err(PublishErrorV1::UnexpectedReply),
        }
    }

    pub fn pending(&self) -> Option<&PublicationIntentV1> {
        self.pending.as_ref()
    }

    pub fn state(&self) -> ResponsePublisherStateV1 {
        ResponsePublisherStateV1 {
            client: self.client.state(),
            pending_intent: self.pending.clone(),
        }
    }

    /// Explicitly abandons local uncertainty without claiming that Agora did
    /// not publish. Both Moira identity and exact Agora request are returned.
    pub fn abandon_uncertainty(&mut self) -> Option<(PublicationIntentV1, RequestV2)> {
        let intent = self.pending.take()?;
        let request = self.client.abandon_uncertainty()?;
        Some((intent, request))
    }

    pub fn into_parts(self) -> (P, Option<PublicationIntentV1>, Option<RequestV2>) {
        let (port, request) = self.client.into_parts();
        (port, self.pending, request)
    }

    fn valid(&self, value: &PublicationIntentV1) -> bool {
        valid_intent(self.limits, value)
    }
}

fn validate_limits<E>(limits: PublicationLimitsV1) -> Result<(), PublishErrorV1<E>> {
    if limits.max_room_id_bytes == 0
        || limits.max_body_bytes == 0
        || limits.max_addressed_participants == 0
        || limits.max_participant_id_bytes == 0
    {
        return Err(PublishErrorV1::InvalidLimits);
    }
    Ok(())
}

fn valid_intent(limits: PublicationLimitsV1, value: &PublicationIntentV1) -> bool {
    !value.room_id.is_empty()
        && value.room_id.len() <= limits.max_room_id_bytes
        && !value.body.is_empty()
        && value.body.len() <= limits.max_body_bytes
        && value.addressed_to.len() <= limits.max_addressed_participants
        && value.addressed_to.iter().all(|participant| {
            !participant.is_empty() && participant.len() <= limits.max_participant_id_bytes
        })
}

fn publication_command(intent: &PublicationIntentV1) -> CommandV2 {
    CommandV2::SpeakIdentified {
        room_id: intent.room_id.clone(),
        expected_revision: intent.expected_agora_revision,
        logical_time: intent.logical_time,
        body: intent.body.clone(),
        addressed_to: intent.addressed_to.clone(),
    }
}
