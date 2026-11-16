mod support;

use moira_protocol::{agent_participation as p, deliberation as d};
use moira_wire::{
    CodecErrorV1, LimitsV1, decode_agent_participation_reply, decode_agent_participation_request,
    decode_agent_request, encode_agent_participation_reply, encode_agent_participation_request,
};
use support::assert_canonical_mutations;

fn binding() -> p::BindingV1 {
    p::BindingV1 {
        participation: p::ParticipationIdV1([1; 16]),
        workflow: moira_protocol::WorkflowIdV1([2; 16]),
        deliberation: d::DeliberationIdV1([3; 16]),
        member: d::EnsembleMemberIdV1([4; 16]),
        session: p::AgentSessionIdV1([5; 16]),
        profile: p::ProfileIdV1([6; 16]),
        attempt: attempt(),
        method: d::MethodRefV1 {
            machine_digest: [7; 32],
            state_digest: [8; 32],
        },
        grounding: d::GroundingRefV1 {
            authority_digest: [9; 32],
            surface_digest: [10; 32],
            depth: d::SurfaceDepthV1::Activated,
        },
        persona: d::PersonaLensV1::RigorReviewer,
    }
}

fn attempt() -> p::AttemptV1 {
    p::AttemptV1 {
        task_key: [11; 16],
        incarnation: 12,
        attempt: 13,
    }
}

fn permission() -> p::PermissionEvidenceV1 {
    p::PermissionEvidenceV1 {
        request: p::PermissionRequestIdV1([14; 16]),
        turn: p::AgentTurnIdV1([15; 16]),
        capability_digest: [16; 32],
        options_digest: [17; 32],
        option_count: 1,
        option_ids: [
            p::PermissionOptionIdV1([19; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
        ],
    }
}

fn capabilities() -> p::CapabilitiesV1 {
    p::CapabilitiesV1 {
        load_session: true,
        resume_session: false,
        permissions: true,
        filesystem_read: true,
        filesystem_write: false,
        terminal: true,
        session_modes: false,
        session_models: true,
    }
}

fn operation() -> p::OperationV1 {
    p::OperationV1::DecidePermission {
        key: p::IdempotencyKeyV1([18; 16]),
        execution: p::ExecutionIdV1([22; 16]),
        request: permission().request,
        option: p::PermissionOptionIdV1([19; 16]),
        authority: p::PermissionAuthorityV1 {
            decision: p::AuthorityDecisionIdV1([20; 16]),
            subject: p::AuthoritySubjectIdV1([21; 16]),
            capability_digest: permission().capability_digest,
            epoch: 22,
        },
        attempt: attempt(),
    }
}

fn view() -> p::ParticipationViewV1 {
    p::ParticipationViewV1 {
        binding: binding(),
        revision: 23,
        phase: p::PhaseV1::AwaitingPermission,
        session_revision: Some(24),
        last_event_cursor: Some(25),
        profile_document_digest: Some([26; 32]),
        capabilities: capabilities(),
        active_turn: Some(permission().turn),
        permission: Some(permission()),
        terminal: None,
        pending: Some(operation()),
    }
}

fn round_request(value: p::CommandV1) {
    let limits = LimitsV1::default();
    let bytes = encode_agent_participation_request(&value, limits).unwrap();
    assert_eq!(
        decode_agent_participation_request(&bytes, limits),
        Ok(value)
    );
    assert_eq!(
        decode_agent_request(&bytes, limits),
        Err(CodecErrorV1::Unsupported)
    );
    assert_canonical_mutations(
        &bytes,
        |candidate| decode_agent_participation_request(candidate, limits),
        |decoded| encode_agent_participation_request(decoded, limits),
    );
    for end in 0..bytes.len() {
        assert!(decode_agent_participation_request(&bytes[..end], limits).is_err());
    }
}

fn round_reply(value: p::ReplyV1) {
    let limits = LimitsV1::default();
    let bytes = encode_agent_participation_reply(&value, limits).unwrap();
    assert_eq!(decode_agent_participation_reply(&bytes, limits), Ok(value));
    assert_canonical_mutations(
        &bytes,
        |candidate| decode_agent_participation_reply(candidate, limits),
        |decoded| encode_agent_participation_reply(decoded, limits),
    );
}

#[test]
fn commands_and_mutations_roundtrip_on_distinct_canonical_frames() {
    let participation = binding().participation;
    for mutation in [
        p::MutationV1::Admit(binding()),
        p::MutationV1::Prepare {
            participation,
            expected_revision: 1,
            operation: operation(),
        },
        p::MutationV1::Prepare {
            participation,
            expected_revision: 2,
            operation: p::OperationV1::Reattach {
                key: p::IdempotencyKeyV1([40; 16]),
                execution: p::ExecutionIdV1([41; 16]),
                attempt: attempt(),
            },
        },
        p::MutationV1::Observe {
            participation,
            expected_revision: 2,
            key: p::IdempotencyKeyV1([27; 16]),
            observation: p::ObservationV1 {
                phase: p::PhaseV1::Terminal,
                session_revision: 3,
                active_turn: Some(permission().turn),
                last_event_cursor: Some(4),
                profile_document_digest: [28; 32],
                capabilities: capabilities(),
                permission: None,
                terminal: Some(p::TerminalEvidenceV1 {
                    turn: permission().turn,
                    disposition: p::DispositionV1::Uncertain,
                    stop_reason: p::StopReasonV1::OutcomeUnknown,
                    output_digest: [29; 32],
                    provider_evidence_digest: [30; 32],
                }),
            },
        },
        p::MutationV1::FenceUnknown {
            participation,
            expected_revision: 3,
            key: p::IdempotencyKeyV1([31; 16]),
        },
        p::MutationV1::MarkReattachRequired {
            participation,
            expected_revision: 4,
            session_revision: 24,
        },
    ] {
        round_request(p::CommandV1::Apply {
            expected_storage_revision: Some(4),
            mutation: Box::new(mutation),
        });
    }
    round_request(p::CommandV1::Status);
    round_request(p::CommandV1::Participation { participation });
    round_request(p::CommandV1::SelectPermission(p::PermissionSelectionV1 {
        expected_storage_revision: Some(5),
        participation,
        expected_revision: 6,
        key: p::IdempotencyKeyV1([32; 16]),
        execution: p::ExecutionIdV1([22; 16]),
        request: permission().request,
        option: p::PermissionOptionIdV1([33; 16]),
        capability_digest: permission().capability_digest,
        attempt: attempt(),
    }));
    round_request(p::CommandV1::PermissionConfirmation { participation });
    round_request(p::CommandV1::ConfirmPermission(
        p::ConfirmedPermissionSelectionV1 {
            participation,
            confirmation_digest: [34; 32],
            option: p::PermissionOptionIdV1([33; 16]),
        },
    ));
    round_request(p::CommandV1::CancelConfirmation { participation });
    round_request(p::CommandV1::ConfirmCancel(p::ConfirmedCancelV1 {
        participation,
        confirmation_digest: [36; 32],
    }));
    round_request(p::CommandV1::SubmitPrompt(p::PromptSubmissionV1 {
        participation,
        prompt: b"ground this response".to_vec(),
    }));
    round_request(p::CommandV1::MaterializePrompt(
        p::PromptMaterializationRequestV1 {
            participation,
            expected_revision: 23,
            prompt_digest: [38; 32],
        },
    ));
    round_request(p::CommandV1::ReattachConfirmation { participation });
    round_request(p::CommandV1::ConfirmReattach(p::ConfirmedReattachV1 {
        participation,
        confirmation_digest: [39; 32],
    }));
}

#[test]
fn prompt_submission_obeys_the_frame_bound() {
    let value = p::CommandV1::SubmitPrompt(p::PromptSubmissionV1 {
        participation: binding().participation,
        prompt: vec![1; 64],
    });
    assert_eq!(
        encode_agent_participation_request(
            &value,
            LimitsV1 {
                max_frame_bytes: 32,
            },
        ),
        Err(CodecErrorV1::Limit)
    );
}

#[test]
fn outcomes_and_refusals_roundtrip_canonically() {
    round_reply(p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Applied {
        storage_revision: 7,
        participation: view(),
        replayed: true,
    })));
    round_reply(p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Status(
        p::StatusV1 {
            storage_revision: Some(8),
            needs_reopen: false,
        },
    ))));
    round_reply(p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Participation(
        Some(view()),
    ))));
    round_reply(p::ReplyV1::Accepted(Box::new(
        p::OutcomeV1::PermissionConfirmation(Some(p::PermissionConfirmationV1 {
            storage_revision: 9,
            participation: binding().participation,
            participation_revision: 10,
            session_revision: 11,
            request: permission().request,
            turn: permission().turn,
            capability_digest: permission().capability_digest,
            options_digest: permission().options_digest,
            option_count: permission().option_count,
            option_ids: permission().option_ids,
            confirmation_digest: [35; 32],
        })),
    )));
    round_reply(p::ReplyV1::Accepted(Box::new(
        p::OutcomeV1::CancelConfirmation(Some(p::CancelConfirmationV1 {
            storage_revision: 12,
            participation: binding().participation,
            participation_revision: 13,
            session_revision: 14,
            turn: permission().turn,
            confirmation_digest: [37; 32],
        })),
    )));
    round_reply(p::ReplyV1::Accepted(Box::new(
        p::OutcomeV1::PromptMaterialization(p::PromptMaterializationV1 {
            participation: binding().participation,
            participation_revision: 23,
            prompt_digest: [38; 32],
            prompt: b"ground this response".to_vec(),
        }),
    )));
    round_reply(p::ReplyV1::Accepted(Box::new(
        p::OutcomeV1::ReattachConfirmation(Some(p::ReattachConfirmationV1 {
            storage_revision: 24,
            participation: binding().participation,
            participation_revision: 25,
            session_revision: 26,
            confirmation_digest: [39; 32],
        })),
    )));
    for refusal in [
        p::RefusalV1::NotAuthorized,
        p::RefusalV1::StorageRevisionConflict {
            expected: Some(1),
            actual: None,
        },
        p::RefusalV1::InvalidLimits,
        p::RefusalV1::Limit,
        p::RefusalV1::InvalidBinding,
        p::RefusalV1::DuplicateParticipation,
        p::RefusalV1::UnknownParticipation,
        p::RefusalV1::RevisionConflict,
        p::RefusalV1::RevisionExhausted,
        p::RefusalV1::InvalidOperation,
        p::RefusalV1::OperationConflict,
        p::RefusalV1::NoPendingOperation,
        p::RefusalV1::InvalidObservation,
        p::RefusalV1::ObservationConflict,
        p::RefusalV1::InvalidCheckpoint,
    ] {
        round_reply(p::ReplyV1::Refused(refusal));
    }
}

#[test]
fn limits_and_noncanonical_boole_fail_closed() {
    let value = p::CommandV1::Status;
    assert_eq!(
        encode_agent_participation_request(&value, LimitsV1 { max_frame_bytes: 8 }),
        Err(CodecErrorV1::Limit)
    );
    let reply = p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Status(p::StatusV1 {
        storage_revision: None,
        needs_reopen: true,
    })));
    let mut bytes = encode_agent_participation_reply(&reply, LimitsV1::default()).unwrap();
    *bytes.last_mut().unwrap() = 2;
    assert_eq!(
        decode_agent_participation_reply(&bytes, LimitsV1::default()),
        Err(CodecErrorV1::Malformed)
    );
}
