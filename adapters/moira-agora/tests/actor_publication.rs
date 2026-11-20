use agora_actor_publication_port::{
    ActorPublicationAcceptedV1, ActorPublicationObservationRefusalV1,
    ActorPublicationObservationV1, ActorPublicationProvenanceObservationV1,
    ActorPublicationProvenanceV1, ActorPublicationProvenanceWireClientV1,
    ActorPublicationRefusalV1, ActorPublicationRequestV1,
    ActorPublicationSubjectProvenanceObservationV1, ActorPublicationSubjectProvenanceV1,
    ActorPublicationSubjectProvenanceWireClientV1, ActorPublicationWireClientV1,
};
use moira_agora::{
    PublicationIntentV1, PublicationLimitsV1,
    actor::{
        ActorObservationReplyV1, ActorProvenanceObservationReplyV1, ActorPublicationReplyV1,
        ActorPublisherErrorV1, ActorResponsePublisher, ActorSubjectProvenanceObservationReplyV1,
        retain_actor_participation_relationship,
    },
};
use moira_core::{
    WorkflowId,
    agent_participation::{AgentSessionIdV1, AttemptV1, BindingV1, ParticipationIdV1, ProfileIdV1},
    deliberation::{
        DeliberationIdV1, EnsembleMemberIdV1, GroundingRefV1, MethodRefV1, PersonaLensV1,
        SurfaceDepthV1,
    },
    task_agent_journey as journey,
};
use moira_host::{
    TaskAgentJourneyApplyErrorV1, TaskAgentJourneyApplyReceiptV1, TaskAgentJourneyApplyV1,
    TaskAgentJourneyCensusV1, TaskAgentJourneyMutationV1, TaskAgentJourneyStore,
};
use moira_protocol::EffectIdV1;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Error {
    LostReply,
}

#[derive(Clone)]
struct Client {
    retained: Arc<Mutex<Option<(ActorPublicationRequestV1, ActorPublicationAcceptedV1)>>>,
    lose_publish_reply: bool,
}

impl ActorPublicationWireClientV1 for Client {
    type Error = Error;

    fn publish_actor_wire(
        &mut self,
        request: ActorPublicationRequestV1,
    ) -> Result<Result<ActorPublicationAcceptedV1, ActorPublicationRefusalV1>, Self::Error> {
        let mut retained = self.retained.lock().unwrap();
        let accepted = ActorPublicationAcceptedV1 {
            revision: 19,
            message_id: [0x44; 16],
            message_sequence: 7,
        };
        match retained.as_ref() {
            Some((prior, prior_accepted)) if prior == &request => return Ok(Ok(*prior_accepted)),
            Some(_) => return Ok(Err(ActorPublicationRefusalV1::Conflict)),
            None => *retained = Some((request, accepted)),
        }
        if self.lose_publish_reply {
            Err(Error::LostReply)
        } else {
            Ok(Ok(accepted))
        }
    }

    fn observe_actor_wire(
        &mut self,
        request: ActorPublicationRequestV1,
    ) -> Result<
        Result<ActorPublicationObservationV1, ActorPublicationObservationRefusalV1>,
        Self::Error,
    > {
        Ok(match self.retained.lock().unwrap().as_ref() {
            Some((prior, accepted)) if prior == &request => {
                Ok(ActorPublicationObservationV1::Accepted(*accepted))
            }
            Some(_) => Err(ActorPublicationObservationRefusalV1::Conflict),
            None => Ok(ActorPublicationObservationV1::Absent),
        })
    }
}

impl ActorPublicationProvenanceWireClientV1 for Client {
    type Error = Error;

    fn observe_actor_provenance_wire(
        &mut self,
        request: ActorPublicationRequestV1,
    ) -> Result<
        Result<ActorPublicationProvenanceObservationV1, ActorPublicationObservationRefusalV1>,
        Self::Error,
    > {
        Ok(match self.retained.lock().unwrap().as_ref() {
            Some((prior, accepted)) if prior == &request => Ok(
                ActorPublicationProvenanceObservationV1::Accepted(ActorPublicationProvenanceV1 {
                    accepted: *accepted,
                    actor: "agent".into(),
                }),
            ),
            Some(_) => Err(ActorPublicationObservationRefusalV1::Conflict),
            None => Ok(ActorPublicationProvenanceObservationV1::Absent),
        })
    }
}

impl ActorPublicationSubjectProvenanceWireClientV1 for Client {
    type Error = Error;

    fn observe_actor_subject_provenance_wire(
        &mut self,
        request: ActorPublicationRequestV1,
    ) -> Result<
        Result<
            ActorPublicationSubjectProvenanceObservationV1,
            ActorPublicationObservationRefusalV1,
        >,
        Self::Error,
    > {
        Ok(match self.retained.lock().unwrap().as_ref() {
            Some((prior, accepted)) if prior == &request => {
                Ok(ActorPublicationSubjectProvenanceObservationV1::Accepted(
                    ActorPublicationSubjectProvenanceV1 {
                        accepted: *accepted,
                        actor: "agent".into(),
                        subject: [0x45; 16],
                    },
                ))
            }
            Some(_) => Err(ActorPublicationObservationRefusalV1::Conflict),
            None => Ok(ActorPublicationSubjectProvenanceObservationV1::Absent),
        })
    }
}

fn intent() -> PublicationIntentV1 {
    PublicationIntentV1 {
        effect: EffectIdV1([0x31; 16]),
        source: [0x32; 16],
        room_sequence: 5,
        room_id: "secret-room".into(),
        expected_agora_revision: 18,
        logical_time: 23,
        body: "answer".into(),
        addressed_to: vec!["human".into()],
    }
}

#[test]
fn lost_publish_reply_is_observed_after_independent_reconnect() {
    let retained = Arc::new(Mutex::new(None));
    let first = Client {
        retained: retained.clone(),
        lose_publish_reply: true,
    };
    let mut publisher = ActorResponsePublisher::new(first, PublicationLimitsV1::default()).unwrap();
    assert_eq!(
        publisher.publish(&intent()),
        Err(ActorPublisherErrorV1::Transport(Error::LostReply))
    );

    let reconnected = Client {
        retained,
        lose_publish_reply: false,
    };
    let mut publisher =
        ActorResponsePublisher::new(reconnected, PublicationLimitsV1::default()).unwrap();
    let ActorObservationReplyV1::Published(observed) = publisher.observe(&intent()).unwrap() else {
        panic!("retained publication expected")
    };
    assert_eq!(observed.receipt.effect, intent().effect);
    assert_eq!(observed.receipt.message.message_sequence, 7);
    assert_eq!(observed.receipt.agora_revision, 19);
    assert_eq!(observed.message_id.into_bytes(), [0x44; 16]);
    let ActorProvenanceObservationReplyV1::Published(provenance) =
        publisher.observe_provenance(&intent()).unwrap()
    else {
        panic!("retained publication provenance expected")
    };
    assert_eq!(provenance.published, observed);
    assert_eq!(provenance.actor, "agent");
    assert_eq!(provenance.room_id, "secret-room");
    assert!(!format!("{provenance:?}").contains("agent"));
    assert!(!format!("{provenance:?}").contains("secret-room"));
    let ActorSubjectProvenanceObservationReplyV1::Published(subject_provenance) =
        publisher.observe_subject_provenance(&intent()).unwrap()
    else {
        panic!("retained publication subject provenance expected")
    };
    assert_eq!(subject_provenance.published, observed);
    assert_eq!(subject_provenance.actor, "agent");
    assert_eq!(subject_provenance.subject, [0x45; 16]);
    assert_eq!(subject_provenance.room_id, "secret-room");
    let debug = format!("{subject_provenance:?}");
    assert!(!debug.contains("agent"));
    assert!(!debug.contains("45, 45"));
    assert!(!debug.contains("secret-room"));
    assert!(matches!(
        publisher.publish(&intent()),
        Ok(ActorPublicationReplyV1::Published(replayed)) if replayed == observed
    ));
}

#[test]
fn absence_and_conflict_never_fall_through_to_publication() {
    let retained = Arc::new(Mutex::new(None));
    let client = Client {
        retained,
        lose_publish_reply: false,
    };
    let mut publisher =
        ActorResponsePublisher::new(client, PublicationLimitsV1::default()).unwrap();
    assert_eq!(
        publisher.observe(&intent()).unwrap(),
        ActorObservationReplyV1::Absent
    );
    assert_eq!(
        publisher.observe_provenance(&intent()).unwrap(),
        ActorProvenanceObservationReplyV1::Absent
    );
    assert_eq!(
        publisher.observe_subject_provenance(&intent()).unwrap(),
        ActorSubjectProvenanceObservationReplyV1::Absent
    );
    assert!(matches!(
        publisher.publish(&intent()).unwrap(),
        ActorPublicationReplyV1::Published(_)
    ));
    let mut changed = intent();
    changed.body = "changed".into();
    assert_eq!(
        publisher.observe(&changed).unwrap(),
        ActorObservationReplyV1::Refused(ActorPublicationObservationRefusalV1::Conflict)
    );
}

#[test]
fn subject_provenance_is_consumed_only_through_private_journey_mutation() {
    let journey_id = journey::JourneyIdV1 {
        owner: journey::OwnerIdV1([0x32; 16]),
        sequence: 5,
    };
    let provenance = moira_agora::actor::ActorPublishedSubjectProvenanceV1 {
        published: moira_agora::PublishedV1 {
            receipt: moira_protocol::PublicationReceiptV1 {
                effect: EffectIdV1([0x31; 16]),
                message: moira_protocol::MessageRefV1 {
                    source: [0x32; 16],
                    room_sequence: 5,
                    message_sequence: 7,
                },
                agora_revision: 19,
            },
            message_id: agora_protocol::v2::MessageIdV1::new([0x44; 16]),
        },
        actor: "agent".into(),
        subject: [0x45; 16],
        room_id: "secret-room".into(),
    };
    let mut store = RecordingStore::new(journey_id);
    let retained =
        retain_actor_participation_relationship(&mut store, Some(8), journey_id, provenance)
            .unwrap();
    assert_eq!(retained.storage_revision, 9);
    assert_eq!(retained.receipt.relationship.publication.effect, [0x31; 16]);
    assert_eq!(retained.receipt.relationship.publication.source, [0x32; 16]);
    assert_eq!(retained.receipt.relationship.publication.room_sequence, 5);
    assert_eq!(
        retained.receipt.relationship.publication.message_sequence,
        7
    );
    assert_eq!(
        retained.receipt.relationship.publication.message_id,
        [0x44; 16]
    );
    assert_eq!(retained.receipt.relationship.publication.agora_revision, 19);
    assert_eq!(retained.receipt.relationship.publication.actor, "agent");
    assert_eq!(
        retained.receipt.relationship.publication.subject,
        [0x45; 16]
    );
    assert_eq!(store.expected_revision, Some(8));
}

struct RecordingStore {
    journey: journey::JourneyIdV1,
    expected_revision: Option<u64>,
}

impl RecordingStore {
    fn new(journey: journey::JourneyIdV1) -> Self {
        Self {
            journey,
            expected_revision: None,
        }
    }
}

impl TaskAgentJourneyStore for RecordingStore {
    type Error = ();

    fn owner(&self) -> journey::OwnerIdV1 {
        self.journey.owner
    }

    fn owner_revision(&self) -> u64 {
        1
    }

    fn storage_revision(&self) -> u64 {
        8
    }

    fn needs_reopen(&self) -> bool {
        false
    }

    fn catalogue(&self) -> Vec<journey::TemplateV1> {
        Vec::new()
    }

    fn census(&self) -> TaskAgentJourneyCensusV1 {
        TaskAgentJourneyCensusV1 {
            templates: 0,
            previews: 0,
            journeys: 1,
        }
    }

    fn preview(&self, _id: journey::PreviewIdV1) -> Option<journey::PreviewSnapshotV1> {
        None
    }

    fn journey(&self, _id: journey::JourneyIdV1) -> Option<journey::JourneySnapshotV1> {
        None
    }

    fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        mutation: TaskAgentJourneyMutationV1,
    ) -> Result<TaskAgentJourneyApplyReceiptV1, TaskAgentJourneyApplyErrorV1<Self::Error>> {
        self.expected_revision = expected_storage_revision;
        let TaskAgentJourneyMutationV1::BindActorParticipationRelationship {
            journey,
            publication,
        } = mutation
        else {
            panic!("private relationship mutation expected")
        };
        let authority = journey::ParticipationAdmissionAuthorityV1 {
            journey,
            participation: ParticipationIdV1([1; 16]),
            decision: [2; 16],
            principal: publication.subject,
            epoch: 3,
            expires_at: 4,
        };
        Ok(TaskAgentJourneyApplyReceiptV1 {
            storage_revision: 9,
            outcome: TaskAgentJourneyApplyV1::ActorParticipationRelationship(
                journey::ActorParticipationRelationshipReceiptV1 {
                    relationship: journey::ActorParticipationRelationshipV1 {
                        journey,
                        journey_revision: 3,
                        participation: BindingV1 {
                            participation: authority.participation,
                            workflow: WorkflowId::new([5; 16]),
                            deliberation: DeliberationIdV1([6; 16]),
                            member: EnsembleMemberIdV1([7; 16]),
                            session: AgentSessionIdV1([8; 16]),
                            profile: ProfileIdV1([9; 16]),
                            attempt: AttemptV1 {
                                task_key: [10; 16],
                                incarnation: 11,
                                attempt: 12,
                            },
                            method: MethodRefV1 {
                                machine_digest: [13; 32],
                                state_digest: [14; 32],
                            },
                            grounding: GroundingRefV1 {
                                authority_digest: [15; 32],
                                surface_digest: [16; 32],
                                depth: SurfaceDepthV1::Activated,
                            },
                            persona: PersonaLensV1::Researcher,
                        },
                        participation_owner_revision: 0,
                        authority,
                        publication,
                    },
                    replayed: false,
                },
            ),
        })
    }
}
