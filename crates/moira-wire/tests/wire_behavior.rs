mod support;

use moira_protocol::*;
use moira_wire::{
    CodecErrorV1, LimitsV1, ReadyV1, decode_ready, decode_reply, decode_request, encode_ready,
    encode_reply, encode_request,
};
use support::assert_canonical_mutations;

fn message() -> MessageRefV1 {
    MessageRefV1 {
        source: [1; 16],
        room_sequence: 2,
        message_sequence: 3,
    }
}
fn task() -> TaskRefV1 {
    TaskRefV1 {
        external_task_key: [2; 16],
        incarnation: 4,
    }
}
fn attempt() -> AttemptReceiptV1 {
    AttemptReceiptV1 {
        task: task(),
        attempt_number: 1,
        chronos_revision: 5,
    }
}

#[test]
fn every_request_roundtrips_and_truncation_or_trailing_data_refuses() {
    let workflow = WorkflowIdV1([3; 16]);
    let effect = EffectIdV1([4; 16]);
    let commands = [
        CommandV1::Apply {
            expected_revision: 0,
            mutation: MutationV1::AdmitMessage {
                message: message(),
                recipient: ParticipantRefV1([5; 16]),
            },
        },
        CommandV1::Apply {
            expected_revision: 1,
            mutation: MutationV1::AcceptChronosAdmission {
                workflow,
                receipt: AdmissionReceiptV1 {
                    effect,
                    task: task(),
                    chronos_revision: 6,
                },
            },
        },
        CommandV1::Apply {
            expected_revision: 2,
            mutation: MutationV1::AcceptChronosAttempt {
                workflow,
                receipt: attempt(),
            },
        },
        CommandV1::Apply {
            expected_revision: 3,
            mutation: MutationV1::AcceptDaimonResult {
                workflow,
                receipt: ResultReceiptV1 {
                    effect,
                    execution_id: [6; 16],
                    task: task(),
                    attempt_number: 1,
                    disposition: DispositionV1::Uncertain,
                },
            },
        },
        CommandV1::Apply {
            expected_revision: 4,
            mutation: MutationV1::PrepareAgoraResponse { workflow },
        },
        CommandV1::Apply {
            expected_revision: 5,
            mutation: MutationV1::AcceptAgoraPublication {
                workflow,
                receipt: PublicationReceiptV1 {
                    effect,
                    message: message(),
                    agora_revision: 7,
                },
            },
        },
        CommandV1::Status,
        CommandV1::Workflow { workflow },
    ];
    for command in commands {
        let bytes = encode_request(&command, LimitsV1::default()).unwrap();
        assert_eq!(decode_request(&bytes, LimitsV1::default()), Ok(command));
        assert_canonical_mutations(
            &bytes,
            |candidate| decode_request(candidate, LimitsV1::default()),
            |value| encode_request(value, LimitsV1::default()),
        );
        for end in 0..bytes.len() {
            assert!(decode_request(&bytes[..end], LimitsV1::default()).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert_eq!(
            decode_request(&trailing, LimitsV1::default()),
            Err(CodecErrorV1::Malformed)
        );
    }
}

#[test]
fn replies_roundtrip_with_bounded_read_models_and_all_transition_shapes() {
    let workflow = WorkflowIdV1([3; 16]);
    let effect = EffectIdV1([4; 16]);
    let transitions = [
        TransitionV1::ChronosSubmissionPrepared { workflow, effect },
        TransitionV1::ChronosSubmissionAccepted {
            workflow,
            task: task(),
        },
        TransitionV1::DaimonExecutionPrepared {
            workflow,
            effect,
            task: task(),
            attempt_number: 1,
        },
        TransitionV1::DaimonResultAccepted {
            workflow,
            disposition: DispositionV1::Failed,
        },
        TransitionV1::AgoraResponsePrepared { workflow, effect },
        TransitionV1::AgoraPublicationAccepted {
            workflow,
            message: message(),
        },
    ];
    for transition in transitions {
        let reply = ReplyV1::Accepted(OutcomeV1::Applied {
            storage_revision: 8,
            transition,
            replayed: false,
        });
        let bytes = encode_reply(&reply, LimitsV1::default()).unwrap();
        assert_eq!(decode_reply(&bytes, LimitsV1::default()), Ok(reply));
        assert_canonical_mutations(
            &bytes,
            |candidate| decode_reply(candidate, LimitsV1::default()),
            |value| encode_reply(value, LimitsV1::default()),
        );
    }
    let view = WorkflowViewV1 {
        workflow,
        revision: 4,
        phase: PhaseV1::FencedUncertain,
        cause: message(),
        recipient: ParticipantRefV1([5; 16]),
        pending_effect: Some(effect),
        task: Some(task()),
        attempt: Some(attempt()),
        result_disposition: Some(DispositionV1::Uncertain),
        publication: Some(message()),
    };
    for reply in [
        ReplyV1::Accepted(OutcomeV1::Status(StatusV1 {
            revision: 4,
            needs_reopen: true,
        })),
        ReplyV1::Accepted(OutcomeV1::Workflow(None)),
        ReplyV1::Accepted(OutcomeV1::Workflow(Some(view))),
        ReplyV1::Refused(RefusalV1::RevisionConflict {
            expected: 3,
            actual: 4,
        }),
        ReplyV1::Refused(RefusalV1::NotAuthorized),
        ReplyV1::Refused(RefusalV1::AttemptMismatch),
    ] {
        let bytes = encode_reply(&reply, LimitsV1::default()).unwrap();
        assert_eq!(decode_reply(&bytes, LimitsV1::default()), Ok(reply));
        assert_canonical_mutations(
            &bytes,
            |candidate| decode_reply(candidate, LimitsV1::default()),
            |value| encode_reply(value, LimitsV1::default()),
        );
    }
    assert!(!format!("{view:?}").contains("1, 1, 1"));
}

#[test]
fn frame_limits_apply_before_growth() {
    let command = CommandV1::Status;
    assert_eq!(
        encode_request(&command, LimitsV1 { max_frame_bytes: 8 }),
        Err(CodecErrorV1::Limit)
    );
    let bytes = encode_request(&command, LimitsV1::default()).unwrap();
    assert_eq!(
        decode_request(
            &bytes,
            LimitsV1 {
                max_frame_bytes: bytes.len() - 1,
            }
        ),
        Err(CodecErrorV1::Limit)
    );
}

#[test]
fn ready_handshake_is_closed_and_bounded() {
    let ready = ReadyV1 {
        protocol_version: 1,
        max_frame_bytes: 4096,
        max_requests: 8,
    };
    let bytes = encode_ready(ready, LimitsV1::default()).unwrap();
    assert_eq!(decode_ready(&bytes, LimitsV1::default()), Ok(ready));
    assert_canonical_mutations(
        &bytes,
        |candidate| decode_ready(candidate, LimitsV1::default()),
        |value| encode_ready(*value, LimitsV1::default()),
    );
    let mut trailing = bytes;
    trailing.push(0);
    assert_eq!(
        decode_ready(&trailing, LimitsV1::default()),
        Err(CodecErrorV1::Malformed)
    );
}
