use agent::persistence::{CodecErrorV1, CodecLimitsV1, decode, encode};
use moira_core::{WorkflowId, agent_participation as agent, deliberation};

#[test]
fn checkpoint_is_canonical_bounded_and_mutation_checked() {
    let mut engine = agent::Engine::try_new(agent::LimitsV1 {
        max_participations: 4,
    })
    .unwrap();
    engine
        .admit(agent::BindingV1 {
            participation: agent::ParticipationIdV1([1; 16]),
            workflow: WorkflowId::new([2; 16]),
            deliberation: deliberation::DeliberationIdV1([3; 16]),
            member: deliberation::EnsembleMemberIdV1([4; 16]),
            session: agent::AgentSessionIdV1([5; 16]),
            profile: agent::ProfileIdV1([6; 16]),
            attempt: agent::AttemptV1 {
                task_key: [7; 16],
                incarnation: 1,
                attempt: 1,
            },
            method: deliberation::MethodRefV1 {
                machine_digest: [8; 32],
                state_digest: [9; 32],
            },
            grounding: deliberation::GroundingRefV1 {
                authority_digest: [10; 32],
                surface_digest: [11; 32],
                depth: deliberation::SurfaceDepthV1::Reference,
            },
            persona: deliberation::PersonaLensV1::Historian,
        })
        .unwrap();
    let operation = agent::OperationV1::Open {
        key: agent::IdempotencyKeyV1([12; 16]),
        execution: agent::ExecutionIdV1([13; 16]),
    };
    engine
        .prepare(agent::ParticipationIdV1([1; 16]), 0, operation)
        .unwrap();
    engine
        .fence_unknown(agent::ParticipationIdV1([1; 16]), 1, operation.key())
        .unwrap();
    let limits = CodecLimitsV1 {
        max_bytes: 4096,
        domain: agent::LimitsV1 {
            max_participations: 4,
        },
    };
    let bytes = encode(&engine.checkpoint(), limits).unwrap();
    assert_eq!(decode(&bytes, limits), Ok(engine.checkpoint()));
    assert_eq!(
        encode(&decode(&bytes, limits).unwrap(), limits).unwrap(),
        bytes
    );
    assert_eq!(
        encode(
            &engine.checkpoint(),
            CodecLimitsV1 {
                max_bytes: 8,
                domain: limits.domain
            }
        ),
        Err(CodecErrorV1::Limit)
    );
    let mut rejected = 0;
    for index in 0..bytes.len() {
        let mut candidate = bytes.clone();
        candidate[index] ^= 1;
        match decode(&candidate, limits) {
            Ok(value) => assert_eq!(encode(&value, limits).unwrap(), candidate),
            Err(_) => rejected += 1,
        }
    }
    assert!(rejected > 0);
}
