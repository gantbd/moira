mod support;

use moira_protocol::agent::*;
use moira_wire::{
    CodecErrorV1, LimitsV1, decode_agent_reply, decode_agent_request, encode_agent_reply,
    encode_agent_request,
};
use support::assert_canonical_mutations;

fn round_request(value: CommandV1) {
    let limits = LimitsV1::default();
    let bytes = encode_agent_request(&value, limits).unwrap();
    assert_eq!(decode_agent_request(&bytes, limits), Ok(value));
    assert_canonical_mutations(
        &bytes,
        |candidate| decode_agent_request(candidate, limits),
        |decoded| encode_agent_request(decoded, limits),
    );
    let mut trailing = bytes;
    trailing.push(0);
    assert_eq!(
        decode_agent_request(&trailing, limits),
        Err(CodecErrorV1::Malformed)
    );
}

fn round_reply(value: ReplyV1) {
    let limits = LimitsV1::default();
    let bytes = encode_agent_reply(&value, limits).unwrap();
    assert_eq!(decode_agent_reply(&bytes, limits), Ok(value));
    assert_canonical_mutations(
        &bytes,
        |candidate| decode_agent_reply(candidate, limits),
        |decoded| encode_agent_reply(decoded, limits),
    );
}

#[test]
fn every_agent_command_roundtrips_canonically() {
    let session = SessionIdV1([1; 16]);
    let turn = TurnIdV1([2; 16]);
    let key = IdempotencyKeyV1([3; 16]);
    for command in [
        CommandV1::Open {
            request: OpenRequestV1 {
                session,
                execution: execution(),
                workspace: [4; 16],
                budget: SessionBudgetV1 {
                    max_elapsed_millis: 10,
                    max_turns: 2,
                    max_events: 20,
                    max_event_bytes: 100,
                    max_prompt_bytes: 200,
                    max_permission_requests: 3,
                },
            },
            now: 1,
        },
        CommandV1::Prompt {
            request: PromptRequestV1 {
                session,
                turn,
                expected_revision: 2,
                execution: execution(),
                prompt_digest: DigestV1([5; 32]),
                prompt: b"grounded".to_vec(),
            },
            now: 3,
        },
        CommandV1::Events {
            session,
            after: Some(4),
            limit: 5,
        },
        CommandV1::Inspect { session },
        CommandV1::DecidePermission {
            session,
            expected_revision: 6,
            execution: execution(),
            decision: PermissionDecisionV1 {
                request: PermissionRequestIdV1([6; 16]),
                option: PermissionOptionIdV1([7; 16]),
            },
            now: 7,
        },
        CommandV1::Advance {
            session,
            turn,
            expected_revision: 8,
            idempotency_key: key,
            now: 9,
        },
        CommandV1::Cancel {
            session,
            turn,
            expected_revision: 10,
            idempotency_key: key,
            now: 11,
        },
        CommandV1::Close {
            session,
            expected_revision: 12,
            idempotency_key: key,
            now: 13,
        },
    ] {
        round_request(command);
    }
}

#[test]
fn every_event_outcome_and_refusal_roundtrips_canonically() {
    let session = SessionIdV1([1; 16]);
    let turn = TurnIdV1([2; 16]);
    let permission = PermissionRequestV1 {
        request: PermissionRequestIdV1([6; 16]),
        turn,
        capability: b"terminal".to_vec(),
        description: b"run tests".to_vec(),
        options: vec![
            PermissionOptionV1 {
                id: PermissionOptionIdV1([7; 16]),
                kind: PermissionOptionKindV1::AllowOnce,
                label: b"allow".to_vec(),
            },
            PermissionOptionV1 {
                id: PermissionOptionIdV1([8; 16]),
                kind: PermissionOptionKindV1::RejectAlways,
                label: b"reject".to_vec(),
            },
        ],
    };
    let events = vec![
        EventV1::AssistantMessage {
            bytes: b"answer".to_vec(),
        },
        EventV1::Thought {
            bytes: b"thought".to_vec(),
        },
        EventV1::Plan {
            bytes: b"plan".to_vec(),
        },
        EventV1::ToolCall {
            title: b"tool".to_vec(),
            detail: b"detail".to_vec(),
        },
        EventV1::Terminal {
            stream: b"stdout".to_vec(),
            bytes: b"ok".to_vec(),
        },
        EventV1::Permission(permission),
        EventV1::FileChange {
            path: b"a.rs".to_vec(),
            content_digest: DigestV1([9; 32]),
        },
        EventV1::Usage {
            input_tokens: 10,
            output_tokens: 11,
        },
        EventV1::TurnFinished {
            turn,
            disposition: DispositionV1::Uncertain,
            stop_reason: StopReasonV1::OutcomeUnknown,
            output_digest: Some(DigestV1([10; 32])),
        },
    ];
    round_reply(ReplyV1::Accepted(OutcomeV1::Session(SessionViewV1 {
        session,
        revision: 3,
        phase: PhaseV1::AwaitingPermission,
        profile_document_digest: DigestV1([11; 32]),
        active_turn: Some(turn),
        last_event_cursor: Some(8),
        capabilities: CapabilitiesV1 {
            load_session: true,
            resume_session: true,
            permissions: true,
            filesystem_read: true,
            filesystem_write: true,
            terminal: true,
            session_modes: true,
            session_models: true,
        },
    })));
    round_reply(ReplyV1::Accepted(OutcomeV1::Events(EventPageV1 {
        session,
        entries: events
            .into_iter()
            .enumerate()
            .map(|(cursor, event)| EventRecordV1 {
                cursor: cursor as u64,
                event,
            })
            .collect(),
        next_after: Some(8),
    })));
    round_reply(ReplyV1::Accepted(OutcomeV1::OutcomeUnknown {
        session,
        idempotency_key: IdempotencyKeyV1([3; 16]),
    }));
    for refusal in [
        RefusalV1::NotAuthorized,
        RefusalV1::UnknownServiceSession,
        RefusalV1::PrincipalMismatch,
        RefusalV1::UnknownAgentSession,
        RefusalV1::UnknownTurn,
        RefusalV1::UnknownPermissionRequest,
        RefusalV1::UnknownPermissionOption,
        RefusalV1::RevisionConflict,
        RefusalV1::AuthorityDenied,
        RefusalV1::AuthorityExpired,
        RefusalV1::AttemptDenied,
        RefusalV1::IdempotencyConflict,
        RefusalV1::SessionConflict,
        RefusalV1::TurnConflict,
        RefusalV1::ProviderReceiptMismatch,
        RefusalV1::ProviderReceiptConflict,
        RefusalV1::Busy,
        RefusalV1::Capacity,
        RefusalV1::Limit,
        RefusalV1::Unsupported,
        RefusalV1::InvalidInput,
        RefusalV1::Closed,
    ] {
        round_reply(ReplyV1::Refused(refusal));
    }
}

#[test]
fn framing_and_collection_limits_fail_closed() {
    let value = CommandV1::Inspect {
        session: SessionIdV1([1; 16]),
    };
    assert_eq!(
        encode_agent_request(&value, LimitsV1 { max_frame_bytes: 8 }),
        Err(CodecErrorV1::Limit)
    );
    let mut bytes = encode_agent_request(&value, LimitsV1::default()).unwrap();
    bytes[8] = 4;
    assert_eq!(
        decode_agent_request(&bytes, LimitsV1::default()),
        Err(CodecErrorV1::Unsupported)
    );
}

fn execution() -> ExecutionV1 {
    ExecutionV1 {
        id: ExecutionIdV1([12; 16]),
        attempt: AttemptV1 {
            task_key: [13; 16],
            incarnation: 2,
            attempt: 3,
        },
        principal: PrincipalIdV1([14; 16]),
        profile: ProfileIdV1([15; 16]),
        idempotency_key: IdempotencyKeyV1([16; 16]),
        context: vec![ContextIdV1([17; 16])],
        tools: vec![ToolIdV1([18; 16])],
        budget: ExecutionBudgetV1 {
            max_elapsed_millis: 100,
            max_input_tokens: 20,
            max_output_tokens: 30,
            max_tool_calls: 1,
            max_context_bytes: 1_000,
            max_output_bytes: 2_000,
        },
    }
}
