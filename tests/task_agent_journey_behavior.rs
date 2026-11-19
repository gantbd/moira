use moira_core::{ChronosTaskRefV1, agent_participation::ProfileIdV1, task_agent_journey as j};

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
