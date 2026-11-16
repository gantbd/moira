use moira_core::{
    EffectId, WorkflowId,
    tool_continuation::{
        AttemptEvidenceV1, AuthorityDecisionIdV1, AuthorityEvidenceV1, AuthoritySubjectIdV1,
        Engine, LimitsV1, PhaseV1, RefusalV1, ToolBudgetV1, ToolCallV1, ToolCapabilityIdV1,
        ToolEffectStateV1, ToolExecutionIdV1, ToolFailureV1, ToolGateV1, ToolReceiptV1,
        ToolRequestIdV1,
    },
};

fn calls() -> Vec<ToolCallV1> {
    vec![
        ToolCallV1 {
            request: ToolRequestIdV1([1; 16]),
            capability: ToolCapabilityIdV1([2; 16]),
            provider_correlation: b"call-1".to_vec(),
            arguments: br#"{"path":"one"}"#.to_vec(),
            budget: ToolBudgetV1 {
                max_elapsed_millis: 100,
                max_output_bytes: 1_024,
            },
        },
        ToolCallV1 {
            request: ToolRequestIdV1([3; 16]),
            capability: ToolCapabilityIdV1([4; 16]),
            provider_correlation: b"call-2".to_vec(),
            arguments: br#"{"path":"two"}"#.to_vec(),
            budget: ToolBudgetV1 {
                max_elapsed_millis: 100,
                max_output_bytes: 1_024,
            },
        },
    ]
}

fn gate() -> ToolGateV1 {
    ToolGateV1 {
        authority: AuthorityEvidenceV1 {
            decision: AuthorityDecisionIdV1([5; 16]),
            subject: AuthoritySubjectIdV1([6; 16]),
            capability: ToolCapabilityIdV1([2; 16]),
            epoch: 7,
        },
        attempt: AttemptEvidenceV1 {
            task_key: [8; 16],
            incarnation: 9,
            attempt: 1,
        },
    }
}

#[test]
fn each_native_call_gets_a_distinct_sequential_effect_and_exact_gate() {
    let workflow = WorkflowId::new([10; 16]);
    let source = EffectId::new([11; 16]);
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    let admitted = engine
        .admit(workflow, source, gate().attempt, calls())
        .unwrap();
    assert_ne!(admitted.effects[0].effect, admitted.effects[1].effect);
    assert_eq!(
        admitted.effects[0].state,
        ToolEffectStateV1::PendingAuthorization
    );
    assert_eq!(admitted.effects[1].state, ToolEffectStateV1::Queued);

    let first = admitted.effects[0].effect;
    let second = admitted.effects[1].effect;
    assert_eq!(
        engine.dispatch(workflow, first, ToolExecutionIdV1([12; 16])),
        Err(RefusalV1::NotReady)
    );
    let mut wrong_gate = gate();
    wrong_gate.authority.capability = ToolCapabilityIdV1([99; 16]);
    assert_eq!(
        engine.authorize(workflow, first, wrong_gate),
        Err(RefusalV1::InvalidEvidence)
    );
    let mut wrong_attempt = gate();
    wrong_attempt.attempt.attempt = 2;
    assert_eq!(
        engine.authorize(workflow, first, wrong_attempt),
        Err(RefusalV1::InvalidEvidence)
    );
    engine.authorize(workflow, first, gate()).unwrap();
    engine
        .dispatch(workflow, first, ToolExecutionIdV1([12; 16]))
        .unwrap();
    assert_eq!(
        engine.accept(
            workflow,
            ToolReceiptV1 {
                effect: first,
                capability: ToolCapabilityIdV1([2; 16]),
                execution: ToolExecutionIdV1([12; 16]),
                evidence_digest: [13; 32],
                output_digest: [14; 32],
                output_bytes: 1_025,
            }
        ),
        Err(RefusalV1::ReceiptMismatch)
    );
    engine
        .accept(
            workflow,
            ToolReceiptV1 {
                effect: first,
                capability: ToolCapabilityIdV1([2; 16]),
                execution: ToolExecutionIdV1([12; 16]),
                evidence_digest: [13; 32],
                output_digest: [14; 32],
                output_bytes: 16,
            },
        )
        .unwrap();
    assert_eq!(
        engine.saga(workflow).unwrap().effects[1].state,
        ToolEffectStateV1::PendingAuthorization
    );
    let mut second_gate = gate();
    second_gate.authority.capability = ToolCapabilityIdV1([4; 16]);
    assert_eq!(
        engine
            .authorize(workflow, second, second_gate)
            .unwrap()
            .phase,
        PhaseV1::Pending
    );
}

#[test]
fn mismatched_receipt_and_unknown_outcome_cannot_advance_the_tail() {
    let workflow = WorkflowId::new([20; 16]);
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    let first = engine
        .admit(workflow, EffectId::new([21; 16]), gate().attempt, calls())
        .unwrap()
        .effects[0]
        .effect;
    engine.authorize(workflow, first, gate()).unwrap();
    engine
        .dispatch(workflow, first, ToolExecutionIdV1([22; 16]))
        .unwrap();
    assert_eq!(
        engine.accept(
            workflow,
            ToolReceiptV1 {
                effect: first,
                capability: ToolCapabilityIdV1([99; 16]),
                execution: ToolExecutionIdV1([22; 16]),
                evidence_digest: [23; 32],
                output_digest: [24; 32],
                output_bytes: 16,
            }
        ),
        Err(RefusalV1::ReceiptMismatch)
    );
    let fenced = engine.mark_uncertain(workflow, first).unwrap();
    assert_eq!(fenced.phase, PhaseV1::FencedUncertain { effect: first });
    assert_eq!(fenced.effects[1].state, ToolEffectStateV1::Queued);
}

#[test]
fn duplicate_model_request_identity_refuses_before_effect_allocation() {
    let workflow = WorkflowId::new([30; 16]);
    let mut missing_correlation = calls();
    missing_correlation[0].provider_correlation.clear();
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    assert_eq!(
        engine.admit(
            workflow,
            EffectId::new([31; 16]),
            gate().attempt,
            missing_correlation
        ),
        Err(RefusalV1::Limit)
    );
    assert!(engine.saga(workflow).is_none());

    let mut duplicated = calls();
    duplicated[1].request = duplicated[0].request;
    assert_eq!(
        engine.admit(
            workflow,
            EffectId::new([31; 16]),
            gate().attempt,
            duplicated
        ),
        Err(RefusalV1::DuplicateRequest)
    );
    assert!(engine.saga(workflow).is_none());
    let admitted_effect = engine
        .admit(workflow, EffectId::new([31; 16]), gate().attempt, calls())
        .unwrap()
        .effects[0]
        .effect;
    let mut fresh = Engine::try_new(LimitsV1::default()).unwrap();
    let fresh_effect = fresh
        .admit(workflow, EffectId::new([31; 16]), gate().attempt, calls())
        .unwrap()
        .effects[0]
        .effect;
    assert_eq!(admitted_effect, fresh_effect);
}

#[test]
fn semantic_checkpoint_recovery_preserves_gate_and_rejects_mutated_history() {
    let workflow = WorkflowId::new([40; 16]);
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    let admitted = engine
        .admit(workflow, EffectId::new([41; 16]), gate().attempt, calls())
        .unwrap();
    let first = admitted.effects[0].effect;
    let second = admitted.effects[1].effect;
    engine.authorize(workflow, first, gate()).unwrap();
    engine
        .dispatch(workflow, first, ToolExecutionIdV1([42; 16]))
        .unwrap();
    engine
        .accept(
            workflow,
            ToolReceiptV1 {
                effect: first,
                capability: ToolCapabilityIdV1([2; 16]),
                execution: ToolExecutionIdV1([42; 16]),
                evidence_digest: [43; 32],
                output_digest: [44; 32],
                output_bytes: 16,
            },
        )
        .unwrap();
    let mut second_gate = gate();
    second_gate.authority.capability = ToolCapabilityIdV1([4; 16]);
    engine.authorize(workflow, second, second_gate).unwrap();
    engine
        .dispatch(workflow, second, ToolExecutionIdV1([45; 16]))
        .unwrap();
    let checkpoint = engine.checkpoint();
    assert_eq!(
        Engine::recover(checkpoint.clone()).unwrap().checkpoint(),
        checkpoint
    );

    let mut skipped = checkpoint.clone();
    skipped.next_effect += 1;
    assert!(matches!(
        Engine::recover(skipped),
        Err(RefusalV1::InvalidCheckpoint)
    ));

    let mut false_completion = checkpoint.clone();
    false_completion.sagas[0].phase = PhaseV1::Completed;
    assert!(matches!(
        Engine::recover(false_completion),
        Err(RefusalV1::InvalidCheckpoint)
    ));

    let mut substituted_authority = checkpoint;
    let ToolEffectStateV1::InFlight { gate, .. } =
        &mut substituted_authority.sagas[0].effects[1].state
    else {
        panic!("second call must be in flight")
    };
    gate.authority.capability = ToolCapabilityIdV1([99; 16]);
    assert!(matches!(
        Engine::recover(substituted_authority),
        Err(RefusalV1::InvalidCheckpoint)
    ));
}

#[test]
fn attributed_failure_is_terminal_and_replay_safe() {
    let workflow = WorkflowId::new([50; 16]);
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    let effect = engine
        .admit(workflow, EffectId::new([51; 16]), gate().attempt, calls())
        .unwrap()
        .effects[0]
        .effect;
    engine.authorize(workflow, effect, gate()).unwrap();
    engine
        .dispatch(workflow, effect, ToolExecutionIdV1([52; 16]))
        .unwrap();
    let failure = ToolFailureV1 {
        effect,
        capability: ToolCapabilityIdV1([2; 16]),
        execution: ToolExecutionIdV1([52; 16]),
        evidence_digest: [53; 32],
    };
    let failed_phase = engine.fail(workflow, failure).unwrap().phase;
    assert_eq!(failed_phase, PhaseV1::Failed { effect });
    assert_eq!(engine.fail(workflow, failure).unwrap().phase, failed_phase);
    assert_eq!(
        Engine::recover(engine.checkpoint()).unwrap().checkpoint(),
        engine.checkpoint()
    );
}
