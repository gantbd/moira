use moira_core::{EffectId, WorkflowId, model_saga::*};

fn proposals() -> Vec<ProposalV1> {
    vec![
        ProposalV1::AgoraMessage {
            room: [1; 16],
            reply_to: [2; 16],
            body: b"answer".to_vec(),
        },
        ProposalV1::VellumEdit {
            workspace: [3; 16],
            document: [4; 16],
            expected_revision: 7,
            expected_extent: 12,
            replacement: b"replacement".to_vec(),
        },
    ]
}

#[test]
fn proposals_receive_distinct_ordered_effects_and_matching_receipts_replay() {
    let workflow = WorkflowId::new([10; 16]);
    let daimon_effect = EffectId::new([11; 16]);
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    let admitted = engine
        .admit(workflow, daimon_effect, [12; 32], proposals())
        .unwrap()
        .clone();
    assert_eq!(admitted.phase, PhaseV1::Pending);
    assert_eq!(admitted.selection_digest, [12; 32]);
    assert_ne!(admitted.effects[0].effect, admitted.effects[1].effect);
    assert_eq!(admitted.effects[0].state, EffectStateV1::Pending);
    assert_eq!(admitted.effects[1].state, EffectStateV1::Queued);
    assert_eq!(
        engine
            .admit(workflow, daimon_effect, [12; 32], proposals())
            .unwrap(),
        &admitted
    );
    assert_eq!(
        engine.admit(workflow, daimon_effect, [13; 32], proposals()),
        Err(RefusalV1::Conflict)
    );

    let agora_effect = admitted.effects[0].effect;
    let agora_receipt = PeerReceiptV1::AgoraMessage {
        effect: agora_effect,
        room: [1; 16],
        reply_to: [2; 16],
        message: [12; 16],
        agora_revision: 9,
    };
    engine.accept(workflow, agora_receipt).unwrap();
    assert_eq!(
        engine.accept(workflow, agora_receipt).unwrap().phase,
        PhaseV1::Pending
    );
    let vellum_effect = engine.pending(workflow).unwrap().effect;
    assert_eq!(vellum_effect, admitted.effects[1].effect);

    let vellum_receipt = PeerReceiptV1::VellumEdit {
        effect: vellum_effect,
        workspace: [3; 16],
        document: [4; 16],
        source_revision: 8,
        durable: true,
    };
    assert_eq!(
        engine.accept(workflow, vellum_receipt).unwrap().phase,
        PhaseV1::Completed
    );
    assert_eq!(
        engine.accept(workflow, vellum_receipt).unwrap().phase,
        PhaseV1::Completed
    );
}

#[test]
fn admission_requires_a_nonzero_action_selection_digest() {
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    assert_eq!(
        engine.admit(
            WorkflowId::new([90; 16]),
            EffectId::new([91; 16]),
            [0; 32],
            proposals(),
        ),
        Err(RefusalV1::InvalidSelection)
    );
    assert!(engine.saga(WorkflowId::new([90; 16])).is_none());
}

#[test]
fn uncertainty_fences_the_tail_and_survives_checkpoint_recovery() {
    let workflow = WorkflowId::new([20; 16]);
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    engine
        .admit(workflow, EffectId::new([21; 16]), [22; 32], proposals())
        .unwrap();
    let effect = engine.pending(workflow).unwrap().effect;
    assert_eq!(
        engine.mark_uncertain(workflow, effect).unwrap().phase,
        PhaseV1::FencedUncertain { effect }
    );
    assert!(engine.pending(workflow).is_none());
    assert_eq!(
        engine.mark_uncertain(workflow, effect).unwrap().phase,
        PhaseV1::FencedUncertain { effect }
    );

    let recovered = Engine::recover(engine.checkpoint()).unwrap();
    assert_eq!(
        recovered.saga(workflow).unwrap().phase,
        PhaseV1::FencedUncertain { effect }
    );
    assert_eq!(
        recovered.saga(workflow).unwrap().effects[1].state,
        EffectStateV1::Queued
    );
}

#[test]
fn historical_receipt_reconciles_only_the_fenced_effect_and_advances_tail() {
    let workflow = WorkflowId::new([22; 16]);
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    engine
        .admit(workflow, EffectId::new([23; 16]), [24; 32], proposals())
        .unwrap();
    let effect = engine.pending(workflow).unwrap().effect;
    engine.mark_uncertain(workflow, effect).unwrap();
    let receipt = PeerReceiptV1::AgoraMessage {
        effect,
        room: [1; 16],
        reply_to: [2; 16],
        message: [24; 16],
        agora_revision: 9,
    };
    let reconciled = engine.reconcile(workflow, receipt).unwrap().clone();
    assert_eq!(reconciled.phase, PhaseV1::Pending);
    assert_eq!(
        reconciled.effects[0].state,
        EffectStateV1::Succeeded(receipt)
    );
    assert_eq!(reconciled.effects[1].state, EffectStateV1::Pending);
    assert_eq!(engine.reconcile(workflow, receipt).unwrap(), &reconciled);

    let mut wrong = receipt;
    let PeerReceiptV1::AgoraMessage {
        ref mut message, ..
    } = wrong
    else {
        unreachable!()
    };
    *message = [99; 16];
    assert_eq!(
        engine.reconcile(workflow, wrong),
        Err(RefusalV1::ReceiptMismatch)
    );
}

#[test]
fn changed_proposal_or_misattributed_receipt_conflicts_without_advancing() {
    let workflow = WorkflowId::new([30; 16]);
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    engine
        .admit(workflow, EffectId::new([31; 16]), [32; 32], proposals())
        .unwrap();
    let mut changed = proposals();
    let ProposalV1::AgoraMessage { body, .. } = &mut changed[0] else {
        unreachable!()
    };
    *body = b"changed".to_vec();
    assert_eq!(
        engine.admit(workflow, EffectId::new([31; 16]), [32; 32], changed),
        Err(RefusalV1::Conflict)
    );

    let effect = engine.pending(workflow).unwrap().effect;
    assert_eq!(
        engine.accept(
            workflow,
            PeerReceiptV1::AgoraMessage {
                effect,
                room: [99; 16],
                reply_to: [2; 16],
                message: [32; 16],
                agora_revision: 1,
            },
        ),
        Err(RefusalV1::ReceiptMismatch)
    );
    assert_eq!(engine.pending(workflow).unwrap().effect, effect);
}
