use moira_core::{
    EffectId, WorkflowId,
    tool_continuation::{
        AttemptEvidenceV1, AuthorityDecisionIdV1, AuthorityEvidenceV1, AuthoritySubjectIdV1,
        Engine, LimitsV1, ToolBudgetV1, ToolCallV1, ToolCapabilityIdV1, ToolExecutionIdV1,
        ToolGateV1, ToolReceiptV1, ToolRequestIdV1,
        persistence::{CodecErrorV1, CodecLimitsV1, decode, encode},
    },
};

fn checkpoint() -> moira_core::tool_continuation::CheckpointV1 {
    let workflow = WorkflowId::new([1; 16]);
    let calls = vec![
        ToolCallV1 {
            request: ToolRequestIdV1([2; 16]),
            capability: ToolCapabilityIdV1([3; 16]),
            provider_correlation: b"call-1".to_vec(),
            arguments: b"first".to_vec(),
            budget: ToolBudgetV1 {
                max_elapsed_millis: 100,
                max_output_bytes: 1_024,
            },
        },
        ToolCallV1 {
            request: ToolRequestIdV1([4; 16]),
            capability: ToolCapabilityIdV1([5; 16]),
            provider_correlation: b"call-2".to_vec(),
            arguments: b"second".to_vec(),
            budget: ToolBudgetV1 {
                max_elapsed_millis: 200,
                max_output_bytes: 2_048,
            },
        },
    ];
    let source_attempt = AttemptEvidenceV1 {
        task_key: [10; 16],
        incarnation: 11,
        attempt: 1,
    };
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    let admitted = engine
        .admit(workflow, EffectId::new([6; 16]), source_attempt, calls)
        .unwrap();
    let first = admitted.effects[0].effect;
    let second = admitted.effects[1].effect;
    let gate = |capability| ToolGateV1 {
        authority: AuthorityEvidenceV1 {
            decision: AuthorityDecisionIdV1([7; 16]),
            subject: AuthoritySubjectIdV1([8; 16]),
            capability,
            epoch: 9,
        },
        attempt: source_attempt,
    };
    engine
        .authorize(workflow, first, gate(ToolCapabilityIdV1([3; 16])))
        .unwrap();
    engine
        .dispatch(workflow, first, ToolExecutionIdV1([12; 16]))
        .unwrap();
    engine
        .accept(
            workflow,
            ToolReceiptV1 {
                effect: first,
                capability: ToolCapabilityIdV1([3; 16]),
                execution: ToolExecutionIdV1([12; 16]),
                evidence_digest: [13; 32],
                output_digest: [14; 32],
                output_bytes: 16,
            },
        )
        .unwrap();
    engine
        .authorize(workflow, second, gate(ToolCapabilityIdV1([5; 16])))
        .unwrap();
    engine
        .dispatch(workflow, second, ToolExecutionIdV1([15; 16]))
        .unwrap();
    engine.checkpoint()
}

#[test]
fn canonical_bytes_roundtrip_authority_attempt_and_in_flight_identity() {
    let limits = CodecLimitsV1::default();
    let checkpoint = checkpoint();
    let bytes = encode(&checkpoint, limits).unwrap();
    assert_eq!(&bytes[..8], b"MOIRATC3");
    let decoded = decode(&bytes, limits).unwrap();
    assert_eq!(decoded, checkpoint);
    assert_eq!(encode(&decoded, limits).unwrap(), bytes);
}

#[test]
fn truncation_trailing_data_old_version_and_narrow_host_bounds_refuse() {
    let limits = CodecLimitsV1::default();
    let bytes = encode(&checkpoint(), limits).unwrap();
    for end in 0..bytes.len() {
        assert!(decode(&bytes[..end], limits).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(decode(&trailing, limits), Err(CodecErrorV1::Invalid));

    let mut old = bytes.clone();
    old[..8].copy_from_slice(b"MOIRATC1");
    old[8] = 1;
    assert_eq!(decode(&old, limits), Err(CodecErrorV1::Invalid));

    let mut narrow = limits;
    narrow.domain.max_calls_per_saga = 1;
    assert_eq!(decode(&bytes, narrow), Err(CodecErrorV1::Limit));
}

#[test]
fn any_accepted_single_byte_mutation_remains_canonical() {
    let limits = CodecLimitsV1::default();
    let bytes = encode(&checkpoint(), limits).unwrap();
    for index in 0..bytes.len() {
        let mut mutated = bytes.clone();
        mutated[index] ^= 1;
        if let Ok(decoded) = decode(&mutated, limits) {
            assert_eq!(encode(&decoded, limits).unwrap(), mutated);
        }
    }
}
