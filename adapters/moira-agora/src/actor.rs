//! Reconnect-safe response publication through Agora's actor-owned profile.
//!
//! Moira supplies one stable effect identity and exact response semantics.
//! Agora derives the author from its authenticated session, retains the
//! semantic result, and exposes lookup through an independently reconnectable
//! public client. This adapter retains no Agora session or request identity.

use crate::{PublicationIntentV1, PublicationLimitsV1, PublishedV1, valid_intent};
use agora_actor_publication_port::{
    ActorPublicationIdV1, ActorPublicationInputRefusalV1, ActorPublicationIntentV1,
    ActorPublicationObservationRefusalV1, ActorPublicationObservationV1,
    ActorPublicationProvenanceObservationV1, ActorPublicationProvenanceWireClientV1,
    ActorPublicationRefusalV1, ActorPublicationRequestV1,
    ActorPublicationSubjectProvenanceObservationV1, ActorPublicationSubjectProvenanceWireClientV1,
    ActorPublicationWireClientV1,
};
use agora_protocol::v2::MessageIdV1;
use moira_core::task_agent_journey::{
    ActorParticipationRelationshipReceiptV1, ActorPublicationEvidenceV1, JourneyIdV1,
};
use moira_host::{
    TaskAgentJourneyApplyErrorV1, TaskAgentJourneyApplyV1, TaskAgentJourneyMutationV1,
    TaskAgentJourneyStore,
};
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

#[derive(Clone, Eq, PartialEq)]
pub struct ActorPublishedProvenanceV1 {
    pub published: PublishedV1,
    pub actor: String,
    pub room_id: String,
}

impl std::fmt::Debug for ActorPublishedProvenanceV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActorPublishedProvenanceV1")
            .field("published", &self.published)
            .field("actor_bytes", &self.actor.len())
            .field("room_id_bytes", &self.room_id.len())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActorProvenanceObservationReplyV1 {
    Published(ActorPublishedProvenanceV1),
    Absent,
    Refused(ActorPublicationObservationRefusalV1),
}

#[derive(Clone, Eq, PartialEq)]
pub struct ActorPublishedSubjectProvenanceV1 {
    pub published: PublishedV1,
    pub actor: String,
    pub subject: [u8; 16],
    pub room_id: String,
}

impl std::fmt::Debug for ActorPublishedSubjectProvenanceV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActorPublishedSubjectProvenanceV1")
            .field("published", &self.published)
            .field("actor_bytes", &self.actor.len())
            .field("subject", &"<redacted>")
            .field("room_id_bytes", &self.room_id.len())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActorSubjectProvenanceObservationReplyV1 {
    Published(ActorPublishedSubjectProvenanceV1),
    Absent,
    Refused(ActorPublicationObservationRefusalV1),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorParticipationRelationshipOutcomeV1 {
    pub storage_revision: u64,
    pub receipt: ActorParticipationRelationshipReceiptV1,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ActorParticipationRelationshipErrorV1<E> {
    Apply(TaskAgentJourneyApplyErrorV1<E>),
    UnexpectedOutcome,
}

/// Consume Agora's exact Host-derived actor and subject observation into the
/// private Moira task-agent owner. The public task-agent protocol deliberately
/// exposes no corresponding mutation command.
pub fn retain_actor_participation_relationship<S: TaskAgentJourneyStore>(
    store: &mut S,
    expected_storage_revision: Option<u64>,
    journey: JourneyIdV1,
    provenance: ActorPublishedSubjectProvenanceV1,
) -> Result<ActorParticipationRelationshipOutcomeV1, ActorParticipationRelationshipErrorV1<S::Error>>
{
    let publication = ActorPublicationEvidenceV1 {
        effect: provenance.published.receipt.effect.0,
        source: provenance.published.receipt.message.source,
        room_sequence: provenance.published.receipt.message.room_sequence,
        room_id: provenance.room_id,
        message_sequence: provenance.published.receipt.message.message_sequence,
        message_id: provenance.published.message_id.into_bytes(),
        agora_revision: provenance.published.receipt.agora_revision,
        actor: provenance.actor,
        subject: provenance.subject,
    };
    let applied = store
        .apply(
            expected_storage_revision,
            TaskAgentJourneyMutationV1::BindActorParticipationRelationship {
                journey,
                publication,
            },
        )
        .map_err(ActorParticipationRelationshipErrorV1::Apply)?;
    let TaskAgentJourneyApplyV1::ActorParticipationRelationship(receipt) = applied.outcome else {
        return Err(ActorParticipationRelationshipErrorV1::UnexpectedOutcome);
    };
    Ok(ActorParticipationRelationshipOutcomeV1 {
        storage_revision: applied.storage_revision,
        receipt,
    })
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

impl<P> ActorResponsePublisher<P>
where
    P: ActorPublicationWireClientV1
        + ActorPublicationProvenanceWireClientV1<Error = <P as ActorPublicationWireClientV1>::Error>,
{
    /// Exact historical observation retaining Agora's Host-derived actor next
    /// to Moira's effect/source/room projection. This proves Agora authorship,
    /// not workflow identity, task linkage, acceptance, or completion.
    pub fn observe_provenance(
        &mut self,
        intent: &PublicationIntentV1,
    ) -> Result<
        ActorProvenanceObservationReplyV1,
        ActorPublisherErrorV1<<P as ActorPublicationWireClientV1>::Error>,
    > {
        let request = request(self.limits, intent)?;
        match self
            .client
            .observe_actor_provenance_wire(request)
            .map_err(ActorPublisherErrorV1::Transport)?
        {
            Ok(ActorPublicationProvenanceObservationV1::Accepted(provenance)) => Ok(
                ActorProvenanceObservationReplyV1::Published(ActorPublishedProvenanceV1 {
                    published: published(intent, provenance.accepted),
                    actor: provenance.actor,
                    room_id: intent.room_id.clone(),
                }),
            ),
            Ok(ActorPublicationProvenanceObservationV1::Absent) => {
                Ok(ActorProvenanceObservationReplyV1::Absent)
            }
            Err(refusal) => Ok(ActorProvenanceObservationReplyV1::Refused(refusal)),
        }
    }
}

impl<P> ActorResponsePublisher<P>
where
    P: ActorPublicationWireClientV1
        + ActorPublicationSubjectProvenanceWireClientV1<
            Error = <P as ActorPublicationWireClientV1>::Error,
        >,
{
    /// Exact historical observation retaining Agora's Host-derived actor and
    /// verified subject. It proves publication attribution only; workflow
    /// identity requires a separate consuming-owner relationship.
    pub fn observe_subject_provenance(
        &mut self,
        intent: &PublicationIntentV1,
    ) -> Result<
        ActorSubjectProvenanceObservationReplyV1,
        ActorPublisherErrorV1<<P as ActorPublicationWireClientV1>::Error>,
    > {
        let request = request(self.limits, intent)?;
        match self
            .client
            .observe_actor_subject_provenance_wire(request)
            .map_err(ActorPublisherErrorV1::Transport)?
        {
            Ok(ActorPublicationSubjectProvenanceObservationV1::Accepted(provenance)) => {
                Ok(ActorSubjectProvenanceObservationReplyV1::Published(
                    ActorPublishedSubjectProvenanceV1 {
                        published: published(intent, provenance.accepted),
                        actor: provenance.actor,
                        subject: provenance.subject,
                        room_id: intent.room_id.clone(),
                    },
                ))
            }
            Ok(ActorPublicationSubjectProvenanceObservationV1::Absent) => {
                Ok(ActorSubjectProvenanceObservationReplyV1::Absent)
            }
            Err(refusal) => Ok(ActorSubjectProvenanceObservationReplyV1::Refused(refusal)),
        }
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
