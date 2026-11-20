use moira_core::{
    ChronosAttemptReceiptV1, ChronosTaskRefV1, agent_participation::ProfileIdV1,
    task_agent_journey as j,
};

#[test]
fn canonical_checkpoint_round_trips_and_reopens_exact_journey() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let admitted = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap()
        .journey;
    let task_ready = engine
        .bind_task_attempt(
            admitted.id,
            ChronosAttemptReceiptV1::new(preview.request.task.task, 2, 12),
        )
        .unwrap()
        .journey;
    let ready = engine
        .bind_room(
            admitted.id,
            j::AgoraRoomReceiptV1 {
                source: task_ready.workflow.owner.0,
                room_sequence: task_ready.workflow.sequence,
                agora_revision: 8,
            },
        )
        .unwrap()
        .journey;
    let limits = j::persistence::CodecLimitsV1::default();
    let bytes = j::persistence::encode(&engine.checkpoint(), limits).unwrap();
    let checkpoint = j::persistence::decode(&bytes, limits).unwrap();
    let reopened = j::Engine::recover(checkpoint).unwrap();
    assert_eq!(reopened.journey(admitted.id), Some(&ready));
    assert_eq!(
        j::persistence::encode(&reopened.checkpoint(), limits).unwrap(),
        bytes
    );
}

#[test]
fn legacy_admitted_checkpoint_decodes_and_upgrades_canonically() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let admitted = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap()
        .journey;
    let limits = j::persistence::CodecLimitsV1::default();
    let mut legacy = j::persistence::encode(&engine.checkpoint(), limits).unwrap();
    legacy[8] = 1;
    for _ in 0..19 {
        assert_eq!(legacy.pop(), Some(0));
    }

    let checkpoint = j::persistence::decode(&legacy, limits).unwrap();
    assert_eq!(checkpoint.journeys, vec![admitted]);
    let upgraded = j::persistence::encode(&checkpoint, limits).unwrap();
    assert_eq!(upgraded[8], 5);
    assert_eq!(
        j::persistence::decode(&upgraded, limits).unwrap(),
        checkpoint
    );
}

#[test]
fn version_two_task_attempt_checkpoint_decodes_and_upgrades_canonically() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    let admitted = engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap()
        .journey;
    let task_ready = engine
        .bind_task_attempt(
            admitted.id,
            ChronosAttemptReceiptV1::new(preview.request.task.task, 2, 12),
        )
        .unwrap()
        .journey;
    let limits = j::persistence::CodecLimitsV1::default();
    let mut version_two = j::persistence::encode(&engine.checkpoint(), limits).unwrap();
    version_two[8] = 2;
    for _ in 0..18 {
        assert_eq!(version_two.pop(), Some(0));
    }

    let checkpoint = j::persistence::decode(&version_two, limits).unwrap();
    assert_eq!(checkpoint.journeys, vec![task_ready]);
    let upgraded = j::persistence::encode(&checkpoint, limits).unwrap();
    assert_eq!(upgraded[8], 5);
    assert_eq!(
        j::persistence::decode(&upgraded, limits).unwrap(),
        checkpoint
    );
}

#[test]
fn truncation_tail_mutation_and_host_limit_refuse() {
    let mut engine = engine();
    let preview = engine.preview(preview_request(), 1_000).unwrap().preview;
    engine
        .instantiate(instantiate_request(preview), 1_002)
        .unwrap();
    let limits = j::persistence::CodecLimitsV1::default();
    let bytes = j::persistence::encode(&engine.checkpoint(), limits).unwrap();
    for end in 0..bytes.len() {
        assert!(j::persistence::decode(&bytes[..end], limits).is_err());
    }
    let mut tailed = bytes.clone();
    tailed.push(0);
    assert!(j::persistence::decode(&tailed, limits).is_err());
    let mut changed = bytes.clone();
    changed[0] ^= 1;
    assert!(j::persistence::decode(&changed, limits).is_err());
    let constrained = j::persistence::CodecLimitsV1 {
        max_bytes: bytes.len() - 1,
        ..limits
    };
    assert_eq!(
        j::persistence::decode(&bytes, constrained),
        Err(j::persistence::CodecErrorV1::Limit)
    );
}

fn engine() -> j::Engine {
    j::Engine::try_new(j::OwnerIdV1([1; 16]), j::LimitsV1::default(), [template()]).unwrap()
}

fn template() -> j::TemplateV1 {
    j::TemplateV1 {
        id: j::TemplateIdV1([2; 16]),
        version: 3,
        digest: [3; 32],
        work_mode: j::WorkModeV1::ProductManager,
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
