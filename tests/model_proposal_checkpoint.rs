use moira_core::{
    EffectId, WorkflowId,
    model_saga::{
        Engine, LimitsV1, PeerReceiptV1, ProposalV1,
        persistence::{CodecLimitsV1, decode, encode},
    },
};

fn proposals(seed: u8) -> Vec<ProposalV1> {
    vec![
        ProposalV1::AgoraMessage {
            room: [seed; 16],
            reply_to: [seed + 1; 16],
            body: vec![seed; 8],
        },
        ProposalV1::VellumEdit {
            workspace: [seed + 2; 16],
            document: [seed + 3; 16],
            expected_revision: 4,
            expected_extent: 5,
            replacement: vec![seed + 4; 9],
        },
    ]
}

fn checkpoint() -> moira_core::model_saga::CheckpointV1 {
    let mut engine = Engine::try_new(LimitsV1::default()).unwrap();
    let first = WorkflowId::new([10; 16]);
    engine
        .admit(first, EffectId::new([11; 16]), [12; 32], proposals(1))
        .unwrap();
    let effect = engine.pending(first).unwrap().effect;
    engine
        .accept(
            first,
            PeerReceiptV1::AgoraMessage {
                effect,
                room: [1; 16],
                reply_to: [2; 16],
                message: [12; 16],
                agora_revision: 3,
            },
        )
        .unwrap();

    let second = WorkflowId::new([20; 16]);
    engine
        .admit(second, EffectId::new([21; 16]), [22; 32], proposals(6))
        .unwrap();
    let uncertain = engine.pending(second).unwrap().effect;
    engine.mark_uncertain(second, uncertain).unwrap();
    engine.checkpoint()
}

#[test]
fn canonical_checkpoint_roundtrips_every_retained_effect_and_fence() {
    let checkpoint = checkpoint();
    let limits = CodecLimitsV1::default();
    let bytes = encode(&checkpoint, limits).unwrap();
    assert_eq!(decode(&bytes, limits).unwrap(), checkpoint);
    Engine::recover(decode(&bytes, limits).unwrap()).unwrap();
    for end in 0..bytes.len() {
        assert!(decode(&bytes[..end], limits).is_err());
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert!(decode(&trailing, limits).is_err());
}

#[test]
fn accepted_mutations_are_canonical_or_refused() {
    let limits = CodecLimitsV1::default();
    let canonical = encode(&checkpoint(), limits).unwrap();
    for index in 0..canonical.len() {
        let mut mutated = canonical.clone();
        mutated[index] ^= 1;
        if let Ok(value) = decode(&mutated, limits) {
            assert_eq!(encode(&value, limits).unwrap(), mutated);
        }
    }
}

#[test]
fn prior_unbound_checkpoint_identity_is_refused_instead_of_reinterpreted() {
    let limits = CodecLimitsV1::default();
    let mut bytes = encode(&checkpoint(), limits).unwrap();
    bytes[..8].copy_from_slice(b"MOIRAMP1");
    bytes[8] = 1;
    assert!(decode(&bytes, limits).is_err());
}
