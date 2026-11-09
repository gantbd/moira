use moira_core::persistence::{CodecLimits, decode_checkpoint, encode_checkpoint};
use moira_core::*;

#[test]
fn canonical_checkpoint_bytes_round_trip_and_preserve_original_replay() {
    let limits = CodecLimits {
        max_bytes: 8192,
        max_workflows: 2,
    };
    let mut engine = Engine::try_new(Limits { max_workflows: 2 }).unwrap();
    let command = CommandV1::AdmitMessage {
        message: AgoraMessageRefV1::new([1; 16], 2, 3),
        recipient: ParticipantRefV1::new([4; 16]),
    };
    let first = engine.apply(engine.revision(), command).unwrap();
    let bytes = encode_checkpoint(&engine.checkpoint(), limits).unwrap();
    assert!(!format!("{bytes:?}").contains("[1, 1"));
    let checkpoint = decode_checkpoint(bytes.as_bytes(), limits).unwrap();
    assert_eq!(
        encode_checkpoint(&checkpoint, limits).unwrap().as_bytes(),
        bytes.as_bytes()
    );
    let mut recovered = Engine::recover(checkpoint).unwrap();
    assert_eq!(
        recovered.apply(recovered.revision(), command).unwrap(),
        first
    );
    assert_eq!(recovered.revision(), WorkflowRevision(1));
}

#[test]
fn truncated_trailing_unknown_profile_and_limits_refuse_without_partial_state() {
    let limits = CodecLimits {
        max_bytes: 8192,
        max_workflows: 2,
    };
    let engine = Engine::try_new(Limits { max_workflows: 2 }).unwrap();
    let bytes = encode_checkpoint(&engine.checkpoint(), limits).unwrap();
    let raw = bytes.as_bytes();
    for end in 0..raw.len() {
        assert!(decode_checkpoint(&raw[..end], limits).is_err());
    }
    let mut trailing = raw.to_vec();
    trailing.push(0);
    assert!(decode_checkpoint(&trailing, limits).is_err());
    let mut unknown = raw.to_vec();
    unknown[4] ^= 255;
    assert!(decode_checkpoint(&unknown, limits).is_err());
    assert!(
        decode_checkpoint(
            raw,
            CodecLimits {
                max_bytes: 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        decode_checkpoint(
            raw,
            CodecLimits {
                max_workflows: 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        encode_checkpoint(
            &engine.checkpoint(),
            CodecLimits {
                max_bytes: 1,
                ..limits
            }
        )
        .is_err()
    );
}
