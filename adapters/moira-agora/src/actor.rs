//! Reconnect-safe response publication through Agora's actor-owned profile.
//!
//! Moira supplies one stable effect identity and exact response semantics.
//! Agora derives the author from its authenticated session, retains the
//! semantic result, and exposes lookup through an independently reconnectable
//! public client. This adapter retains no Agora session or request identity.

use crate::{PublicationIntentV1, PublicationLimitsV1, PublishedV1, valid_intent};
use agora_actor_publication_port::{
    ActorPublicationIdV1, ActorPublicationInputRefusalV1, ActorPublicationIntentV1,
    ActorPublicationObservationRefusalV1, ActorPublicationObservationV1, ActorPublicationRefusalV1,
    ActorPublicationRequestV1, ActorPublicationWireClientV1,
};
use agora_protocol::v2::MessageIdV1;
use moira_protocol::{MessageRefV1, PublicationReceiptV1};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorPublicationReplyV1 {
    Published(PublishedV1),
    Refused(ActorPublicationRefusalV1),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorObservationReplyV1 {
    Published(PublishedV1),
    Absent,
    Refused(ActorPublicationObservationRefusalV1),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActorPublisherErrorV1<E> {
    InvalidLimits,
    InvalidInput,
    Transport(E),
}

pub struct ActorResponsePublisher<P> {
    client: P,
    limits: PublicationLimitsV1,
}

impl<P: ActorPublicationWireClientV1> ActorResponsePublisher<P> {
    pub fn new(
        client: P,
        limits: PublicationLimitsV1,
    ) -> Result<Self, ActorPublisherErrorV1<P::Error>> {
        if limits.max_room_id_bytes == 0
            || limits.max_body_bytes == 0
            || limits.max_addressed_participants == 0
            || limits.max_participant_id_bytes == 0
        {
            return Err(ActorPublisherErrorV1::InvalidLimits);
        }
        Ok(Self { client, limits })
    }

    pub fn publish(
        &mut self,
        intent: &PublicationIntentV1,
    ) -> Result<ActorPublicationReplyV1, ActorPublisherErrorV1<P::Error>> {
        let request = request(self.limits, intent)?;
        match self
            .client
            .publish_actor_wire(request)
            .map_err(ActorPublisherErrorV1::Transport)?
        {
            Ok(accepted) => Ok(ActorPublicationReplyV1::Published(published(
                intent, accepted,
            ))),
            Err(refusal) => Ok(ActorPublicationReplyV1::Refused(refusal)),
        }
    }

    /// Historical observation only. `Absent` never falls through to publish.
    pub fn observe(
        &mut self,
        intent: &PublicationIntentV1,
    ) -> Result<ActorObservationReplyV1, ActorPublisherErrorV1<P::Error>> {
        let request = request(self.limits, intent)?;
        match self
            .client
            .observe_actor_wire(request)
            .map_err(ActorPublisherErrorV1::Transport)?
        {
            Ok(ActorPublicationObservationV1::Accepted(accepted)) => Ok(
                ActorObservationReplyV1::Published(published(intent, accepted)),
            ),
            Ok(ActorPublicationObservationV1::Absent) => Ok(ActorObservationReplyV1::Absent),
            Err(refusal) => Ok(ActorObservationReplyV1::Refused(refusal)),
        }
    }

    pub fn into_client(self) -> P {
        self.client
    }
}

fn request<E>(
    limits: PublicationLimitsV1,
    intent: &PublicationIntentV1,
) -> Result<ActorPublicationRequestV1, ActorPublisherErrorV1<E>> {
    if !valid_intent(limits, intent) {
        return Err(ActorPublisherErrorV1::InvalidInput);
    }
    let id = ActorPublicationIdV1::new(intent.effect.0).map_err(
        |ActorPublicationInputRefusalV1::ZeroIdentity| ActorPublisherErrorV1::InvalidInput,
    )?;
    Ok(ActorPublicationRequestV1 {
        id,
        intent: ActorPublicationIntentV1::new(
            intent.room_id.clone(),
            intent.expected_agora_revision,
            intent.logical_time,
            intent.body.clone(),
            intent.addressed_to.clone(),
        ),
    })
}

fn published(
    intent: &PublicationIntentV1,
    accepted: agora_actor_publication_port::ActorPublicationAcceptedV1,
) -> PublishedV1 {
    PublishedV1 {
        receipt: PublicationReceiptV1 {
            effect: intent.effect,
            message: MessageRefV1 {
                source: intent.source,
                room_sequence: intent.room_sequence,
                message_sequence: accepted.message_sequence,
            },
            agora_revision: accepted.revision,
        },
        message_id: MessageIdV1::new(accepted.message_id),
    }
}
