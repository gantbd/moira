#![cfg(unix)]

use moira_core::{
    ChronosAttemptReceiptV1, ChronosTaskRefV1, WorkflowId,
    agent_participation::{
        AgentSessionIdV1, AttemptV1, BindingV1, Engine as ParticipationEngine,
        LimitsV1 as ParticipationLimitsV1, ParticipationIdV1, ProfileIdV1 as DomainProfileIdV1,
    },
    deliberation::{
        DeliberationIdV1, EnsembleAdmissionV1, EnsembleMemberIdV1, GroundingRefV1, MethodRefV1,
        PersonaLensV1, SurfaceDepthV1,
    },
    task_agent_journey as domain,
};
use moira_host::{
    TaskAgentJourneyApplyErrorV1, TaskAgentJourneyApplyV1, TaskAgentJourneyMutationV1,
    TaskAgentJourneyStore, TaskAgentParticipationPreparationV1,
};
use moira_protocol::{TaskRefV1, agent_participation::ProfileIdV1, task_agent_journey as public};
use moira_service::{
    AccessV1,
    task_agent_journey::{TaskAgentJourneyService, TrustedTimeV1},
};
use moira_sqlite::task_agent_journey::{
    TaskAgentJourneySqliteStore, TaskAgentJourneyStoreLimitsV1,
};
use std::{
    convert::Infallible,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        Self {
            path: std::env::temp_dir().join(format!(
                "moira-task-agent-journey-{}-{}.db",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
        }
    }

    fn store(&self) -> TaskAgentJourneySqliteStore {
        TaskAgentJourneySqliteStore::open(
            &self.path,
            domain::OwnerIdV1([1; 16]),
            limits(),
            [template()],
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-journal", ".owner.lock"] {
            let _ = fs::remove_file(format!("{}{}", self.path.display(), suffix));
        }
    }
}

struct Time(u64);

impl TrustedTimeV1 for Time {
    type Error = Infallible;

    fn now(&mut self) -> Result<u64, Self::Error> {
        let value = self.0;
        self.0 += 1;
        Ok(value)
    }
}

fn limits() -> TaskAgentJourneyStoreLimitsV1 {
    let domain = domain::LimitsV1 {
        max_templates: 2,
        max_previews: 4,
        max_journeys: 4,
        max_preview_ttl: 60,
    };
    TaskAgentJourneyStoreLimitsV1 {
        codec: domain::persistence::CodecLimitsV1 {
            max_bytes: 65_536,
            domain,
        },
        domain,
        max_database_pages: 64,
    }
}

fn template() -> domain::TemplateV1 {
    domain::TemplateV1 {
        id: domain::TemplateIdV1([2; 16]),
        version: 1,
        digest: [3; 32],
        work_mode: domain::WorkModeV1::Collaborator,
        profile: DomainProfileIdV1([4; 16]),
        room_policy: domain::RoomPolicyV1::SharedContext,
        budget: domain::BudgetV1 {
            max_context_bytes: 4_096,
            max_output_tokens: 512,
            max_elapsed_millis: 10_000,
        },
        grounding_policy_digest: [5; 32],
        authority_policy_digest: [6; 32],
        consequence_digest: [7; 32],
        retired: false,
    }
}

fn preview_request() -> public::PreviewRequestV1 {
    public::PreviewRequestV1 {
        operation: public::PreviewOperationIdV1([8; 16]),
        template: public::TemplateRefV1 {
            id: public::TemplateIdV1([2; 16]),
            version: 1,
            digest: [3; 32],
        },
        task: public::TaskAdmissionV1 {
            task: TaskRefV1 {
                external_task_key: [9; 16],
                incarnation: 1,
            },
            chronos_revision: 11,
            evidence_digest: [10; 32],
        },
        profile: ProfileIdV1([4; 16]),
        room_policy: public::RoomPolicyV1::SharedContext,
        initiating_principal: public::PrincipalIdV1([12; 16]),
        accountable_human: public::PrincipalIdV1([13; 16]),
        objective_digest: [14; 32],
        acceptance_digest: [15; 32],
        current_evidence_digest: [16; 32],
        requested_ttl: 30,
    }
}

#[test]
fn public_service_persists_preview_admission_and_exact_replay_across_reopen() {
    let fixture = Fixture::new();
    let preview_command = public::CommandV1::Preview {
        expected_storage_revision: None,
        request: preview_request(),
    };
    let mut service = TaskAgentJourneyService::new(fixture.store(), Time(100));

    assert_eq!(
        service
            .execute_authorized(
                AccessV1 {
                    read: true,
                    mutate: false,
                },
                preview_command.clone(),
            )
            .unwrap(),
        public::ReplyV1::Refused(public::RefusalV1::NotAuthorized)
    );
    let public::ReplyV1::Accepted(outcome) = service.execute(preview_command.clone()).unwrap()
    else {
        panic!("preview expected")
    };
    let public::OutcomeV1::Previewed {
        storage_revision,
        receipt,
    } = *outcome
    else {
        panic!("preview outcome expected")
    };
    assert_eq!(storage_revision, 1);
    assert!(!receipt.replayed);
    let preview = receipt.preview;

    let replay = public::CommandV1::Preview {
        expected_storage_revision: Some(1),
        request: preview_request(),
    };
    let public::ReplyV1::Accepted(outcome) = service.execute(replay.clone()).unwrap() else {
        panic!("preview replay expected")
    };
    let public::OutcomeV1::Previewed {
        storage_revision,
        receipt,
    } = *outcome
    else {
        panic!("preview replay outcome expected")
    };
    assert_eq!(storage_revision, 1);
    assert!(receipt.replayed);
    drop(service);

    let instantiate_request = public::InstantiateRequestV1 {
        operation: public::InstantiateOperationIdV1([17; 16]),
        preview: preview.id,
        expected_owner_revision: preview.owner_revision,
        confirmation: public::HumanConfirmationV1 {
            preview: preview.id,
            accountable_human: preview.request.accountable_human,
            consequence_digest: preview.consequence_digest,
        },
    };
    let instantiate = public::CommandV1::Instantiate {
        expected_storage_revision: Some(1),
        request: instantiate_request,
    };
    let mut service = TaskAgentJourneyService::new(fixture.store(), Time(101));
    assert_eq!(
        service.execute(replay).unwrap(),
        public::ReplyV1::Accepted(Box::new(public::OutcomeV1::Previewed {
            storage_revision: 1,
            receipt: public::PreviewReceiptV1 {
                preview,
                replayed: true,
            },
        }))
    );
    let public::ReplyV1::Accepted(outcome) = service.execute(instantiate.clone()).unwrap() else {
        panic!("journey admission expected")
    };
    let public::OutcomeV1::Admitted {
        storage_revision,
        receipt,
    } = *outcome
    else {
        panic!("journey outcome expected")
    };
    assert_eq!(storage_revision, 2);
    assert!(!receipt.replayed);
    let journey = receipt.journey;

    let replay = public::CommandV1::Instantiate {
        expected_storage_revision: Some(2),
        request: instantiate_request,
    };
    let public::ReplyV1::Accepted(outcome) = service.execute(replay).unwrap() else {
        panic!("journey replay expected")
    };
    let public::OutcomeV1::Admitted {
        storage_revision,
        receipt,
    } = *outcome
    else {
        panic!("journey replay outcome expected")
    };
    assert_eq!(storage_revision, 2);
    assert!(receipt.replayed);
    assert_eq!(receipt.journey, journey);
    let (mut store, _) = service.into_parts();
    let domain_journey = domain::JourneyIdV1 {
        owner: domain::OwnerIdV1(journey.id.owner.0),
        sequence: journey.id.sequence,
    };
    let attempt = ChronosAttemptReceiptV1::new(
        ChronosTaskRefV1::new(
            journey.preview.request.task.task.external_task_key,
            journey.preview.request.task.task.incarnation,
        ),
        2,
        12,
    );
    let ready = store
        .apply(
            Some(2),
            TaskAgentJourneyMutationV1::BindTaskAttemptNamespace {
                journey: domain_journey,
                receipt: attempt,
                chronos_task_id: "j1-owned-task".into(),
            },
        )
        .unwrap();
    assert_eq!(ready.storage_revision, 3);
    let TaskAgentJourneyApplyV1::TaskAttempt(ready) = ready.outcome else {
        panic!("task-attempt transition expected")
    };
    assert!(!ready.replayed);
    assert_eq!(ready.journey.phase, domain::PhaseV1::TaskAttemptReady);
    assert_eq!(
        store
            .apply(
                Some(3),
                TaskAgentJourneyMutationV1::BindTaskAttemptNamespace {
                    journey: domain_journey,
                    receipt: attempt,
                    chronos_task_id: "j1-owned-task".into(),
                },
            )
            .unwrap()
            .storage_revision,
        3
    );
    assert!(matches!(
        store.apply(
            Some(3),
            TaskAgentJourneyMutationV1::BindTaskAttempt {
                journey: domain_journey,
                receipt: ChronosAttemptReceiptV1::new(attempt.task(), 3, 13),
            },
        ),
        Err(TaskAgentJourneyApplyErrorV1::Domain(
            domain::RefusalV1::TaskAttemptConflict
        ))
    ));
    drop(store);
    let store = fixture.store();
    let mut service = TaskAgentJourneyService::new(store, Time(200));
    let public::ReplyV1::Accepted(namespace) = service
        .execute_authorized(
            AccessV1 {
                read: true,
                mutate: false,
            },
            public::CommandV1::TaskAttemptNamespace {
                journey: journey.id,
            },
        )
        .unwrap()
    else {
        panic!("task namespace expected")
    };
    let public::OutcomeV1::TaskAttemptNamespace(Some(namespace)) = *namespace else {
        panic!("task namespace outcome expected")
    };
    assert_eq!(namespace.chronos_task_id, "j1-owned-task");
    assert_eq!(namespace.task, journey.preview.request.task.task);
    assert_eq!(namespace.attempt_number, 2);
    assert!(!format!("{namespace:?}").contains("j1-owned-task"));
    let (mut store, _) = service.into_parts();
    let room = domain::AgoraRoomReceiptV1 {
        source: domain_journey.owner.0,
        room_sequence: domain_journey.sequence,
        agora_revision: 8,
    };
    let room_ready = store
        .apply(
            Some(3),
            TaskAgentJourneyMutationV1::BindRoom {
                journey: domain_journey,
                receipt: room,
            },
        )
        .unwrap();
    assert_eq!(room_ready.storage_revision, 4);
    let TaskAgentJourneyApplyV1::Room(room_ready) = room_ready.outcome else {
        panic!("room transition expected")
    };
    assert!(!room_ready.replayed);
    assert_eq!(room_ready.journey.phase, domain::PhaseV1::RoomReady);
    assert_eq!(
        store
            .apply(
                Some(4),
                TaskAgentJourneyMutationV1::BindRoom {
                    journey: domain_journey,
                    receipt: room,
                },
            )
            .unwrap()
            .storage_revision,
        4
    );
    assert!(matches!(
        store.apply(
            Some(4),
            TaskAgentJourneyMutationV1::BindRoom {
                journey: domain_journey,
                receipt: domain::AgoraRoomReceiptV1 {
                    agora_revision: 9,
                    ..room
                },
            },
        ),
        Err(TaskAgentJourneyApplyErrorV1::Domain(
            domain::RefusalV1::RoomConflict
        ))
    ));
    let workflow = domain::ParticipationWorkflowReceiptV1 {
        source: domain_journey.owner.0,
        workflow_sequence: domain_journey.sequence,
        participation_workflow: WorkflowId::new([20; 16]),
        relationship_revision: 0,
    };
    let deliberation = domain::DeliberationMemberReceiptV1 {
        deliberation: DeliberationIdV1([21; 16]),
        member: EnsembleMemberIdV1([22; 16]),
        method: MethodRefV1 {
            machine_digest: [23; 32],
            state_digest: [24; 32],
        },
        grounding: GroundingRefV1 {
            authority_digest: [25; 32],
            surface_digest: [26; 32],
            depth: SurfaceDepthV1::Activated,
        },
        persona: PersonaLensV1::Researcher,
        ensemble_admission: EnsembleAdmissionV1::affirmed(
            [27; 32], [28; 32], [29; 32], [30; 32], [31; 16],
        ),
        owner_revision: 0,
    };
    let binding = BindingV1 {
        participation: ParticipationIdV1([32; 16]),
        workflow: workflow.participation_workflow,
        deliberation: deliberation.deliberation,
        member: deliberation.member,
        session: AgentSessionIdV1([33; 16]),
        profile: DomainProfileIdV1([4; 16]),
        attempt: AttemptV1 {
            task_key: attempt.task().external_task_key(),
            incarnation: attempt.task().incarnation(),
            attempt: attempt.attempt_number(),
        },
        method: deliberation.method,
        grounding: deliberation.grounding,
        persona: deliberation.persona,
    };
    let prepared = store
        .apply(
            Some(4),
            TaskAgentJourneyMutationV1::PrepareParticipation(Box::new(
                TaskAgentParticipationPreparationV1 {
                    journey: domain_journey,
                    workflow,
                    deliberation,
                    binding,
                },
            )),
        )
        .unwrap();
    assert_eq!(prepared.storage_revision, 5);
    let TaskAgentJourneyApplyV1::ParticipationPrepared(prepared) = prepared.outcome else {
        panic!("participation preparation expected")
    };
    assert!(!prepared.replayed);
    assert_eq!(prepared.blueprint.binding, binding);
    assert_eq!(
        store
            .apply(
                Some(5),
                TaskAgentJourneyMutationV1::PrepareParticipation(Box::new(
                    TaskAgentParticipationPreparationV1 {
                        journey: domain_journey,
                        workflow,
                        deliberation,
                        binding,
                    },
                )),
            )
            .unwrap()
            .storage_revision,
        5
    );
    let mut changed_binding = binding;
    changed_binding.session = AgentSessionIdV1([99; 16]);
    assert!(matches!(
        store.apply(
            Some(5),
            TaskAgentJourneyMutationV1::PrepareParticipation(Box::new(
                TaskAgentParticipationPreparationV1 {
                    journey: domain_journey,
                    workflow,
                    deliberation,
                    binding: changed_binding,
                },
            )),
        ),
        Err(TaskAgentJourneyApplyErrorV1::Domain(
            domain::RefusalV1::ParticipationConflict
        ))
    ));
    let authority = domain::ParticipationAdmissionAuthorityV1 {
        journey: domain_journey,
        participation: binding.participation,
        decision: [34; 16],
        principal: [35; 16],
        epoch: 6,
        expires_at: 400,
    };
    let attributed = store
        .apply(
            Some(5),
            TaskAgentJourneyMutationV1::AttributeParticipation {
                journey: domain_journey,
                authority,
            },
        )
        .unwrap();
    assert_eq!(attributed.storage_revision, 6);
    let TaskAgentJourneyApplyV1::ParticipationAttributed(attributed) = attributed.outcome else {
        panic!("participation attribution expected")
    };
    assert_eq!(attributed.attribution.authority, authority);
    let mut participation_owner =
        ParticipationEngine::try_new(ParticipationLimitsV1::default()).unwrap();
    participation_owner.admit(binding).unwrap();
    let observed = participation_owner
        .participation(binding.participation)
        .unwrap()
        .clone();
    let admitted = store
        .apply(
            Some(6),
            TaskAgentJourneyMutationV1::BindParticipation {
                journey: domain_journey,
                observed: Box::new(observed),
            },
        )
        .unwrap();
    assert_eq!(admitted.storage_revision, 7);
    let TaskAgentJourneyApplyV1::ParticipationAdmitted(admitted) = admitted.outcome else {
        panic!("participation admission expected")
    };
    assert_eq!(
        admitted.journey.phase,
        domain::PhaseV1::ParticipationAdmitted
    );
    let publication = domain::ActorPublicationEvidenceV1 {
        effect: [36; 16],
        source: room.source,
        room_sequence: room.room_sequence,
        room_id: "exact-room".into(),
        message_sequence: 3,
        message_id: [37; 16],
        agora_revision: 9,
        actor: "exact-agent".into(),
        subject: authority.principal,
    };
    let relationship = store
        .apply(
            Some(7),
            TaskAgentJourneyMutationV1::BindActorParticipationRelationship {
                journey: domain_journey,
                publication: publication.clone(),
            },
        )
        .unwrap();
    assert_eq!(relationship.storage_revision, 8);
    let TaskAgentJourneyApplyV1::ActorParticipationRelationship(relationship) =
        relationship.outcome
    else {
        panic!("actor-participation relationship expected")
    };
    assert!(!relationship.replayed);
    assert_eq!(relationship.relationship.publication, publication);
    let replay = store
        .apply(
            Some(8),
            TaskAgentJourneyMutationV1::BindActorParticipationRelationship {
                journey: domain_journey,
                publication: publication.clone(),
            },
        )
        .unwrap();
    assert_eq!(replay.storage_revision, 8);
    let TaskAgentJourneyApplyV1::ActorParticipationRelationship(replay) = replay.outcome else {
        panic!("actor-participation relationship replay expected")
    };
    assert!(replay.replayed);
    let mut substituted = publication.clone();
    substituted.subject = [99; 16];
    assert!(matches!(
        store.apply(
            Some(8),
            TaskAgentJourneyMutationV1::BindActorParticipationRelationship {
                journey: domain_journey,
                publication: substituted,
            },
        ),
        Err(TaskAgentJourneyApplyErrorV1::Domain(
            domain::RefusalV1::ActorParticipationRelationshipConflict
        ))
    ));
    drop(store);

    let ready_view = public::JourneyViewV1 {
        revision: 1,
        phase: public::PhaseV1::TaskAttemptReady,
        task_attempt: Some(public::TaskAttemptBindingV1 {
            journey: journey.id,
            expected_journey_revision: 0,
            task: journey.preview.request.task.task,
            attempt_number: 2,
            chronos_revision: 12,
        }),
        ..journey
    };
    let room_ready_view = public::JourneyViewV1 {
        revision: 2,
        phase: public::PhaseV1::RoomReady,
        room: Some(public::RoomBindingV1 {
            journey: journey.id,
            expected_journey_revision: 1,
            source: journey.id.owner.0,
            room_sequence: journey.id.sequence,
            agora_revision: 8,
        }),
        ..ready_view
    };
    let participation_ready_view = public::JourneyViewV1 {
        revision: 3,
        phase: public::PhaseV1::ParticipationAdmitted,
        ..room_ready_view
    };
    let participation_view = public::ParticipationAdmissionViewV1 {
        journey: journey.id,
        expected_journey_revision: 2,
        participation: binding.participation.0,
        owner_revision: 0,
        owner_phase: moira_protocol::agent_participation::PhaseV1::Admitted,
        authority_decision: authority.decision,
        authority_principal: authority.principal,
        authority_epoch: authority.epoch,
        authority_expires_at: authority.expires_at,
    };

    let reopened = fixture.store();
    assert_eq!(
        reopened.participation_blueprint(domain_journey),
        Some(prepared.blueprint)
    );
    assert_eq!(
        reopened.participation_attribution(domain_journey),
        Some(attributed.attribution)
    );
    let mut service = TaskAgentJourneyService::new(reopened, Time(200));
    assert_eq!(
        service.execute(public::CommandV1::Status).unwrap(),
        public::ReplyV1::Accepted(Box::new(public::OutcomeV1::Status(public::StatusV1 {
            owner: public::OwnerIdV1([1; 16]),
            owner_revision: 1,
            storage_revision: 8,
            template_count: 1,
            preview_count: 1,
            journey_count: 1,
            needs_reopen: false,
        })))
    );
    assert_eq!(
        service
            .execute(public::CommandV1::Journey {
                journey: journey.id,
            })
            .unwrap(),
        public::ReplyV1::Accepted(Box::new(public::OutcomeV1::Journey(Some(
            participation_ready_view
        ))))
    );
    assert_eq!(
        service
            .execute_authorized(
                AccessV1 {
                    read: false,
                    mutate: true,
                },
                public::CommandV1::ActorParticipationRelationship {
                    journey: journey.id,
                },
            )
            .unwrap(),
        public::ReplyV1::Refused(public::RefusalV1::NotAuthorized)
    );
    let public::ReplyV1::Accepted(observed_relationship) = service
        .execute(public::CommandV1::ActorParticipationRelationship {
            journey: journey.id,
        })
        .unwrap()
    else {
        panic!("actor-participation relationship observation expected")
    };
    let public::OutcomeV1::ActorParticipationRelationship(Some(observed_relationship)) =
        *observed_relationship
    else {
        panic!("actor-participation relationship outcome expected")
    };
    assert_eq!(observed_relationship.journey, journey.id);
    assert_eq!(observed_relationship.journey_revision, 3);
    assert_eq!(
        observed_relationship.participation.session.0,
        binding.session.0
    );
    assert_eq!(
        observed_relationship.authority_principal,
        authority.principal
    );
    assert_eq!(observed_relationship.publication.effect, publication.effect);
    assert_eq!(
        observed_relationship.publication.message_id,
        publication.message_id
    );
    assert_eq!(
        observed_relationship.publication.subject,
        authority.principal
    );
    let diagnostic = format!("{observed_relationship:?}");
    assert!(!diagnostic.contains("exact-room"));
    assert!(!diagnostic.contains("exact-agent"));
    assert_eq!(
        service
            .execute(public::CommandV1::Participation {
                journey: journey.id,
            })
            .unwrap(),
        public::ReplyV1::Accepted(Box::new(public::OutcomeV1::Participation(Some(
            participation_view
        ))))
    );
    assert_eq!(
        service.execute(preview_command).unwrap(),
        public::ReplyV1::Refused(public::RefusalV1::StorageRevisionConflict)
    );
}
