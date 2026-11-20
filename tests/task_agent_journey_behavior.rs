use moira_core::{
    ChronosAttemptReceiptV1, ChronosTaskRefV1, WorkflowId,
    agent_participation::{
        AgentSessionIdV1, AttemptV1, BindingV1 as AgentParticipationBindingV1, ParticipationIdV1,
        ProfileIdV1,
    },
    deliberation::{
        DeliberationIdV1, EnsembleAdmissionV1, EnsembleFitnessV1, EnsembleMemberIdV1,
        GroundingRefV1, MethodRefV1, PersonaLensV1, SurfaceDepthV1,
    },
    task_agent_journey as j,
};

#[test]
fn exact_preview_and_human_confirmation_admit_one_owner_journey() {
    let mut engine = engine();
    let preview_request = preview_request();
    let preview = engine.preview(preview_request, 1_000).unwrap();
    assert!(!preview.replayed);
    assert_eq!(preview.preview.id.owner, j::OwnerIdV1([1; 16]));
    assert_eq!(preview.preview.id.sequence, 0);
    assert!(engine.preview(preview_request, 1_001).unwrap().replayed);

    let request = instantiate_request(preview.preview);
    let admitted = engine.instantiate(request, 1_002).unwrap();
    assert!(!admitted.replayed);
    assert_eq!(admitted.journey.phase, j::PhaseV1::Admitted);
    assert_eq!(admitted.journey.task_attempt, None);
    assert_eq!(admitted.journey.room, None);
    assert_eq!(admitted.journey.id.owner, j::OwnerIdV1([1; 16]));
    assert_eq!(admitted.journey.workflow.owner, j::OwnerIdV1([1; 16]));
    assert_eq!(
        admitted.journey.id.sequence,
        admitted.journey.workflow.sequence
    );
    assert!(engine.instantiate(request, 1_003).unwrap().replayed);

    let recovered = j::Engine::recover(engine.checkpoint()).unwrap();
    assert_eq!(
        recovered.journey(admitted.journey.id),
        Some(&admitted.journey)
    );
}

#[test]
fn stale_mismatched_or_reused_confirmation_cannot_create_a_journey() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let mut request = instantiate_request(preview);
    assert_eq!(
        engine.instantiate(request, preview.expires_at),
        Err(j::RefusalV1::PreviewExpired)
    );

    request = instantiate_request(preview);
    request.confirmation.accountable_human = j::PrincipalIdV1([99; 16]);
    assert_eq!(
        engine.instantiate(request, 1_002),
        Err(j::RefusalV1::InvalidConfirmation)
    );

    request = instantiate_request(preview);
    request.confirmation.consequence_digest = [98; 32];
    assert_eq!(
        engine.instantiate(request, 1_002),
        Err(j::RefusalV1::InvalidConfirmation)
    );

    let admitted = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap();
    let mut second = instantiate_request(preview);
    second.operation = j::InstantiateOperationIdV1([97; 16]);
    assert_eq!(
        engine.instantiate(second, 1_003),
        Err(j::RefusalV1::PreviewAlreadyInstantiated)
    );
    assert_eq!(engine.journey(admitted.journey.id), Some(&admitted.journey));
}

#[test]
fn operation_replay_conflicts_and_template_drift_refuse_without_allocation() {
    let mut engine = engine();
    let original = preview_request();
    let first = engine.preview(original, 1_000).unwrap().preview;
    let mut conflict = original;
    conflict.acceptance_digest = [77; 32];
    assert_eq!(
        engine.preview(conflict, 1_000),
        Err(j::RefusalV1::PreviewOperationConflict)
    );

    let mut drift = original;
    drift.operation = j::PreviewOperationIdV1([76; 16]);
    drift.template.version += 1;
    assert_eq!(
        engine.preview(drift, 1_000),
        Err(j::RefusalV1::TemplateMismatch)
    );

    let admitted = engine
        .instantiate(instantiate_request(first), 1_002)
        .unwrap();
    let mut changed_replay = instantiate_request(first);
    changed_replay.confirmation.consequence_digest = [99; 32];
    assert_eq!(
        engine.instantiate(changed_replay, 1_003),
        Err(j::RefusalV1::InstantiateOperationConflict)
    );
    assert_eq!(admitted.journey.id.sequence, 0);
}

#[test]
fn invalid_checkpoint_cannot_restyle_foreign_owner_identity() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap();
    let mut checkpoint = engine.checkpoint();
    checkpoint.journeys[0].id.owner = j::OwnerIdV1([55; 16]);
    assert!(matches!(
        j::Engine::recover(checkpoint),
        Err(j::RefusalV1::InvalidCheckpoint)
    ));
}

#[test]
fn trusted_exact_chronos_claim_derives_one_task_attempt_binding() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let journey = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap()
        .journey;
    let receipt = ChronosAttemptReceiptV1::new(preview.request.task.task, 3, 12);
    assert_eq!(
        j::derive_task_attempt_binding(&journey, receipt),
        Ok(j::TaskAttemptBindingV1 {
            journey: journey.id,
            expected_journey_revision: 0,
            task: preview.request.task.task,
            attempt_number: 3,
            chronos_revision: 12,
        })
    );
}

#[test]
fn trusted_exact_chronos_claim_persists_one_replayable_ready_transition() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let admitted = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap()
        .journey;
    let receipt = ChronosAttemptReceiptV1::new(preview.request.task.task, 3, 12);

    let ready = engine.bind_task_attempt(admitted.id, receipt).unwrap();
    assert!(!ready.replayed);
    assert_eq!(ready.journey.phase, j::PhaseV1::TaskAttemptReady);
    assert_eq!(ready.journey.revision, admitted.revision + 1);
    assert_eq!(
        ready.journey.task_attempt,
        Some(j::TaskAttemptBindingV1 {
            journey: admitted.id,
            expected_journey_revision: admitted.revision,
            task: preview.request.task.task,
            attempt_number: 3,
            chronos_revision: 12,
        })
    );
    assert!(
        engine
            .bind_task_attempt(admitted.id, receipt)
            .unwrap()
            .replayed
    );

    let recovered = j::Engine::recover(engine.checkpoint()).unwrap();
    assert_eq!(recovered.journey(admitted.id), Some(&ready.journey));
}

#[test]
fn persisted_task_attempt_refuses_forks_without_changing_the_journey() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let admitted = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap()
        .journey;
    let exact = ChronosAttemptReceiptV1::new(preview.request.task.task, 3, 12);
    let ready = engine
        .bind_task_attempt(admitted.id, exact)
        .unwrap()
        .journey;

    assert_eq!(
        engine.bind_task_attempt(
            admitted.id,
            ChronosAttemptReceiptV1::new(preview.request.task.task, 4, 13),
        ),
        Err(j::RefusalV1::TaskAttemptConflict)
    );
    assert_eq!(engine.journey(admitted.id), Some(&ready));
}

#[test]
fn exact_agora_room_receipt_persists_one_replayable_ready_transition() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let admitted = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap()
        .journey;
    let task_ready = engine
        .bind_task_attempt(
            admitted.id,
            ChronosAttemptReceiptV1::new(preview.request.task.task, 3, 12),
        )
        .unwrap()
        .journey;
    let receipt = j::AgoraRoomReceiptV1 {
        source: task_ready.workflow.owner.0,
        room_sequence: task_ready.workflow.sequence,
        agora_revision: 8,
    };

    assert_eq!(
        j::derive_room_binding(&task_ready, receipt),
        Ok(j::RoomBindingV1 {
            journey: task_ready.id,
            expected_journey_revision: task_ready.revision,
            source: task_ready.workflow.owner.0,
            room_sequence: task_ready.workflow.sequence,
            agora_revision: 8,
        })
    );
    let ready = engine.bind_room(task_ready.id, receipt).unwrap();
    assert!(!ready.replayed);
    assert_eq!(ready.journey.phase, j::PhaseV1::RoomReady);
    assert_eq!(ready.journey.revision, task_ready.revision + 1);
    assert_eq!(
        ready.journey.room,
        j::derive_room_binding(&task_ready, receipt).ok()
    );
    assert!(engine.bind_room(task_ready.id, receipt).unwrap().replayed);
    assert_eq!(
        j::Engine::recover(engine.checkpoint())
            .unwrap()
            .journey(task_ready.id),
        Some(&ready.journey)
    );
}

#[test]
fn room_binding_refuses_early_foreign_zero_and_forked_receipts() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let admitted = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap()
        .journey;
    let exact = j::AgoraRoomReceiptV1 {
        source: admitted.workflow.owner.0,
        room_sequence: admitted.workflow.sequence,
        agora_revision: 8,
    };
    assert_eq!(
        engine.bind_room(admitted.id, exact),
        Err(j::RefusalV1::InvalidRoom)
    );
    let task_ready = engine
        .bind_task_attempt(
            admitted.id,
            ChronosAttemptReceiptV1::new(preview.request.task.task, 3, 12),
        )
        .unwrap()
        .journey;
    for (receipt, refusal) in [
        (
            j::AgoraRoomReceiptV1 {
                source: [99; 16],
                ..exact
            },
            j::RefusalV1::RoomMismatch,
        ),
        (
            j::AgoraRoomReceiptV1 {
                room_sequence: exact.room_sequence + 1,
                ..exact
            },
            j::RefusalV1::RoomMismatch,
        ),
        (
            j::AgoraRoomReceiptV1 {
                agora_revision: 0,
                ..exact
            },
            j::RefusalV1::InvalidRoom,
        ),
    ] {
        assert_eq!(engine.bind_room(task_ready.id, receipt), Err(refusal));
    }
    let ready = engine.bind_room(task_ready.id, exact).unwrap().journey;
    assert_eq!(
        engine.bind_room(
            task_ready.id,
            j::AgoraRoomReceiptV1 {
                agora_revision: 9,
                ..exact
            }
        ),
        Err(j::RefusalV1::RoomConflict)
    );
    assert_eq!(engine.journey(task_ready.id), Some(&ready));
}

#[test]
fn participation_blueprint_reconciles_exact_owner_facts_without_allocating_them() {
    let (ready, workflow, deliberation, binding) = participation_facts();
    assert_eq!(
        j::derive_participation_blueprint(&ready, workflow, deliberation, binding),
        Ok(j::ParticipationBlueprintV1 {
            journey: ready.id,
            expected_journey_revision: ready.revision,
            workflow,
            deliberation,
            binding,
        })
    );
    assert_eq!(workflow.relationship_revision, 0);
    assert_eq!(deliberation.owner_revision, 0);
}

#[test]
fn participation_blueprint_refuses_invented_or_substituted_owner_facts() {
    let (ready, workflow, deliberation, binding) = participation_facts();
    let mut early = ready;
    early.phase = j::PhaseV1::TaskAttemptReady;
    early.room = None;
    assert_eq!(
        j::derive_participation_blueprint(&early, workflow, deliberation, binding),
        Err(j::ParticipationBlueprintRefusalV1::RoomNotReady)
    );

    let mut changed_workflow = workflow;
    changed_workflow.source = [99; 16];
    assert_eq!(
        j::derive_participation_blueprint(&ready, changed_workflow, deliberation, binding),
        Err(j::ParticipationBlueprintRefusalV1::WorkflowMismatch)
    );

    let mut changed_binding = binding;
    changed_binding.participation = ParticipationIdV1([0; 16]);
    assert_eq!(
        j::derive_participation_blueprint(&ready, workflow, deliberation, changed_binding),
        Err(j::ParticipationBlueprintRefusalV1::InvalidOwnerEvidence)
    );
    let mut changed_deliberation = deliberation;
    changed_deliberation.grounding.surface_digest = [0; 32];
    assert_eq!(
        j::derive_participation_blueprint(&ready, workflow, changed_deliberation, binding),
        Err(j::ParticipationBlueprintRefusalV1::InvalidOwnerEvidence)
    );
    changed_deliberation = deliberation;
    changed_deliberation.ensemble_admission.fitness = EnsembleFitnessV1::Contested;
    assert_eq!(
        j::derive_participation_blueprint(&ready, workflow, changed_deliberation, binding),
        Err(j::ParticipationBlueprintRefusalV1::EnsembleNotAdmitted)
    );
    changed_binding = binding;
    changed_binding.attempt.attempt += 1;
    assert_eq!(
        j::derive_participation_blueprint(&ready, workflow, deliberation, changed_binding),
        Err(j::ParticipationBlueprintRefusalV1::AttemptMismatch)
    );
    changed_binding = binding;
    changed_binding.profile = ProfileIdV1([99; 16]);
    assert_eq!(
        j::derive_participation_blueprint(&ready, workflow, deliberation, changed_binding),
        Err(j::ParticipationBlueprintRefusalV1::ProfileMismatch)
    );
    changed_binding = binding;
    changed_binding.method.state_digest = [99; 32];
    assert_eq!(
        j::derive_participation_blueprint(&ready, workflow, deliberation, changed_binding),
        Err(j::ParticipationBlueprintRefusalV1::DeliberationMismatch)
    );
    changed_binding = binding;
    changed_binding.persona = PersonaLensV1::Historian;
    assert_eq!(
        j::derive_participation_blueprint(&ready, workflow, deliberation, changed_binding),
        Err(j::ParticipationBlueprintRefusalV1::MemberMismatch)
    );
}

fn participation_facts() -> (
    j::JourneySnapshotV1,
    j::ParticipationWorkflowReceiptV1,
    j::DeliberationMemberReceiptV1,
    AgentParticipationBindingV1,
) {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let admitted = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap()
        .journey;
    let task_ready = engine
        .bind_task_attempt(
            admitted.id,
            ChronosAttemptReceiptV1::new(preview.request.task.task, 3, 12),
        )
        .unwrap()
        .journey;
    let ready = engine
        .bind_room(
            task_ready.id,
            j::AgoraRoomReceiptV1 {
                source: task_ready.workflow.owner.0,
                room_sequence: task_ready.workflow.sequence,
                agora_revision: 8,
            },
        )
        .unwrap()
        .journey;
    let workflow = j::ParticipationWorkflowReceiptV1 {
        source: ready.workflow.owner.0,
        workflow_sequence: ready.workflow.sequence,
        participation_workflow: WorkflowId::new([20; 16]),
        relationship_revision: 0,
    };
    let deliberation = j::DeliberationMemberReceiptV1 {
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
            [29; 32], [30; 32], [31; 32], [32; 32], [33; 16],
        ),
        owner_revision: 0,
    };
    let binding = AgentParticipationBindingV1 {
        participation: ParticipationIdV1([27; 16]),
        workflow: workflow.participation_workflow,
        deliberation: deliberation.deliberation,
        member: deliberation.member,
        session: AgentSessionIdV1([28; 16]),
        profile: ready.preview.request.profile,
        attempt: AttemptV1 {
            task_key: ready.task_attempt.unwrap().task.external_task_key(),
            incarnation: ready.task_attempt.unwrap().task.incarnation(),
            attempt: ready.task_attempt.unwrap().attempt_number,
        },
        method: deliberation.method,
        grounding: deliberation.grounding,
        persona: deliberation.persona,
    };
    (ready, workflow, deliberation, binding)
}

#[test]
fn task_attempt_binding_refuses_alias_zero_and_stale_receipts() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let journey = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap()
        .journey;
    let exact_task = preview.request.task.task;
    for (receipt, refusal) in [
        (
            ChronosAttemptReceiptV1::new(ChronosTaskRefV1::new([99; 16], 10), 3, 12),
            j::TaskAttemptBindingRefusalV1::TaskMismatch,
        ),
        (
            ChronosAttemptReceiptV1::new(ChronosTaskRefV1::new([9; 16], 99), 3, 12),
            j::TaskAttemptBindingRefusalV1::TaskMismatch,
        ),
        (
            ChronosAttemptReceiptV1::new(exact_task, 0, 12),
            j::TaskAttemptBindingRefusalV1::InvalidReceipt,
        ),
        (
            ChronosAttemptReceiptV1::new(exact_task, 3, 11),
            j::TaskAttemptBindingRefusalV1::StaleChronosRevision,
        ),
        (
            ChronosAttemptReceiptV1::new(exact_task, 3, 10),
            j::TaskAttemptBindingRefusalV1::StaleChronosRevision,
        ),
    ] {
        assert_eq!(
            j::derive_task_attempt_binding(&journey, receipt),
            Err(refusal)
        );
    }
}

fn engine() -> j::Engine {
    j::Engine::try_new(j::OwnerIdV1([1; 16]), j::LimitsV1::default(), [template()]).unwrap()
}

fn template() -> j::TemplateV1 {
    j::TemplateV1 {
        id: j::TemplateIdV1([2; 16]),
        version: 3,
        digest: [3; 32],
        work_mode: j::WorkModeV1::Collaborator,
        profile: ProfileIdV1([4; 16]),
        room_policy: j::RoomPolicyV1::SharedContext,
        budget: j::BudgetV1 {
            max_context_bytes: 32 * 1_024,
            max_output_tokens: 4_096,
            max_elapsed_millis: 120_000,
        },
        grounding_policy_digest: [5; 32],
        authority_policy_digest: [6; 32],
        consequence_digest: [7; 32],
        retired: false,
    }
}

fn preview_request() -> j::PreviewRequestV1 {
    let template = template();
    j::PreviewRequestV1 {
        operation: j::PreviewOperationIdV1([8; 16]),
        template: j::TemplateRefV1 {
            id: template.id,
            version: template.version,
            digest: template.digest,
        },
        task: j::TaskAdmissionV1 {
            task: ChronosTaskRefV1::new([9; 16], 10),
            chronos_revision: 11,
            evidence_digest: [12; 32],
        },
        profile: template.profile,
        room_policy: template.room_policy,
        initiating_principal: j::PrincipalIdV1([13; 16]),
        accountable_human: j::PrincipalIdV1([14; 16]),
        objective_digest: [15; 32],
        acceptance_digest: [16; 32],
        current_evidence_digest: [17; 32],
        requested_ttl: 300,
    }
}

fn instantiate_request(preview: j::PreviewSnapshotV1) -> j::InstantiateRequestV1 {
    j::InstantiateRequestV1 {
        operation: j::InstantiateOperationIdV1([18; 16]),
        preview: preview.id,
        expected_owner_revision: preview.owner_revision,
        confirmation: j::HumanConfirmationV1 {
            preview: preview.id,
            accountable_human: preview.request.accountable_human,
            consequence_digest: preview.consequence_digest,
        },
    }
}
