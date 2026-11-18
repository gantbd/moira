use daimon_client::AgentServicePort;
use daimon_protocol::{self as daimon, agent as upstream};
use moira_host::AgentGateway;
use moira_protocol::agent as public;

struct Port {
    reply: upstream::AgentReplyV1,
    seen: Option<(daimon::SessionHandleV1, upstream::AgentCommandV1)>,
}

impl AgentServicePort for Port {
    type Error = ();

    fn execute_agent(
        &mut self,
        session: daimon::SessionHandleV1,
        command: upstream::AgentCommandV1,
    ) -> Result<upstream::AgentReplyV1, Self::Error> {
        self.seen = Some((session, command));
        Ok(self.reply.clone())
    }
}

#[test]
fn prompt_binding_is_exact_and_view_is_independent() {
    let reply = upstream::AgentReplyV1::Accepted(upstream::AgentOutcomeV1::Session(
        upstream::AgentSessionViewV1 {
            session: daimon::AgentSessionIdV1::new([1; 16]),
            revision: 8,
            phase: upstream::AgentSessionPhaseV1::TurnActive,
            profile_document_digest: daimon::ContentDigestV1::new([9; 32]),
            active_turn: Some(daimon::AgentTurnIdV1::new([2; 16])),
            last_event_cursor: Some(3),
            last_context: Some(upstream::AgentContextReceiptV1 {
                execution: daimon::ExecutionIdV1::new([3; 16]),
                manifest_digest: daimon::ContentDigestV1::new([10; 32]),
                entries: vec![upstream::AgentContextManifestEntryV1 {
                    reference: daimon::ContextReferenceIdV1::new([8; 16]),
                    kind: daimon::ContextKindV1::VellumDocument,
                    source_digest: daimon::ContentDigestV1::new([11; 32]),
                    content_bytes: 21,
                    grounding: None,
                }],
            }),
            capabilities: upstream::AgentCapabilitiesV1 {
                load_session: true,
                resume_session: true,
                permissions: true,
                filesystem_read: true,
                filesystem_write: true,
                terminal: true,
                session_modes: true,
                session_models: true,
            },
        },
    ));
    let mut adapter = moira_daimon::agent::AgentAdapter::new(
        daimon::SessionHandleV1(44),
        Port { reply, seen: None },
    );
    let command = public::CommandV1::Prompt {
        request: public::PromptRequestV1 {
            session: public::SessionIdV1([1; 16]),
            turn: public::TurnIdV1([2; 16]),
            expected_revision: 7,
            execution: execution(),
            prompt_digest: public::DigestV1([8; 32]),
            prompt: b"grounded question".to_vec(),
        },
        now: 99,
    };
    let result = adapter.execute_agent(command).unwrap();
    let public::ReplyV1::Accepted(public::OutcomeV1::Session(view)) = result else {
        panic!("session view expected")
    };
    assert_eq!(view.revision, 8);
    assert_eq!(view.phase, public::PhaseV1::TurnActive);
    assert_eq!(view.active_turn, Some(public::TurnIdV1([2; 16])));
    let context = view.last_context.as_ref().expect("context receipt");
    assert_eq!(context.execution, public::ExecutionIdV1([3; 16]));
    assert_eq!(context.manifest_digest, public::DigestV1([10; 32]));
    assert_eq!(context.entries.len(), 1);
    assert_eq!(context.entries[0].reference, public::ContextIdV1([8; 16]));
    assert_eq!(
        context.entries[0].kind,
        public::ContextKindV1::VellumDocument
    );
    assert_eq!(context.entries[0].source_digest, public::DigestV1([11; 32]));
    assert_eq!(context.entries[0].content_bytes, 21);
    assert_eq!(context.entries[0].grounding, None);
    assert_eq!(
        view.capabilities,
        public::CapabilitiesV1 {
            load_session: true,
            resume_session: true,
            permissions: true,
            filesystem_read: true,
            filesystem_write: true,
            terminal: true,
            session_modes: true,
            session_models: true,
        }
    );

    let port = adapter.into_port();
    let Some((service_session, upstream::AgentCommandV1::Prompt { request, now })) = port.seen
    else {
        panic!("prompt dispatch expected")
    };
    assert_eq!(service_session, daimon::SessionHandleV1(44));
    assert_eq!(now, 99);
    assert_eq!(request.expected_revision, 7);
    assert_eq!(request.prompt, b"grounded question");
    assert_eq!(request.prompt_digest.into_bytes(), [8; 32]);
    assert_eq!(request.execution.task_attempt.attempt_number, 4);
    assert_eq!(request.execution.context.len(), 1);
    assert_eq!(request.execution.tools.len(), 1);
}

#[test]
fn permission_event_and_refusal_remain_typed() {
    let permission = upstream::AgentPermissionRequestV1 {
        request: daimon::AgentPermissionRequestIdV1::new([5; 16]),
        turn: daimon::AgentTurnIdV1::new([2; 16]),
        capability: b"terminal".to_vec(),
        description: b"run tests".to_vec(),
        options: vec![upstream::AgentPermissionOptionV1 {
            id: daimon::AgentPermissionOptionIdV1::new([6; 16]),
            kind: upstream::AgentPermissionOptionKindV1::AllowOnce,
            label: b"allow once".to_vec(),
        }],
    };
    let reply = upstream::AgentReplyV1::Accepted(upstream::AgentOutcomeV1::Events(
        upstream::AgentEventPageV1 {
            session: daimon::AgentSessionIdV1::new([1; 16]),
            entries: vec![upstream::AgentEventRecordV1 {
                cursor: 10,
                event: upstream::AgentEventV1::Permission(permission),
            }],
            next_after: Some(10),
        },
    ));
    let mut adapter = moira_daimon::agent::AgentAdapter::new(
        daimon::SessionHandleV1(44),
        Port { reply, seen: None },
    );
    let result = adapter
        .execute_agent(public::CommandV1::Events {
            session: public::SessionIdV1([1; 16]),
            after: Some(9),
            limit: 1,
        })
        .unwrap();
    let public::ReplyV1::Accepted(public::OutcomeV1::Events(page)) = result else {
        panic!("event page expected")
    };
    let public::EventV1::Permission(permission) = &page.entries[0].event else {
        panic!("permission event expected")
    };
    assert_eq!(permission.request, public::PermissionRequestIdV1([5; 16]));
    assert_eq!(
        permission.options[0].kind,
        public::PermissionOptionKindV1::AllowOnce
    );

    let refusal = upstream::AgentReplyV1::Refused(upstream::AgentRefusalV1::AttemptDenied);
    let mut adapter = moira_daimon::agent::AgentAdapter::new(
        daimon::SessionHandleV1(44),
        Port {
            reply: refusal,
            seen: None,
        },
    );
    assert_eq!(
        adapter
            .execute_agent(public::CommandV1::Inspect {
                session: public::SessionIdV1([1; 16]),
            })
            .unwrap(),
        public::ReplyV1::Refused(public::RefusalV1::AttemptDenied)
    );
}

#[test]
fn reattach_binding_and_required_phase_are_exact() {
    let reply = upstream::AgentReplyV1::Accepted(upstream::AgentOutcomeV1::Session(
        upstream::AgentSessionViewV1 {
            session: daimon::AgentSessionIdV1::new([1; 16]),
            revision: 9,
            phase: upstream::AgentSessionPhaseV1::ReattachRequired,
            profile_document_digest: daimon::ContentDigestV1::new([9; 32]),
            active_turn: None,
            last_event_cursor: Some(3),
            last_context: None,
            capabilities: upstream::AgentCapabilitiesV1 {
                resume_session: true,
                ..upstream::AgentCapabilitiesV1::default()
            },
        },
    ));
    let mut adapter = moira_daimon::agent::AgentAdapter::new(
        daimon::SessionHandleV1(44),
        Port { reply, seen: None },
    );
    let result = adapter
        .execute_agent(public::CommandV1::Reattach {
            session: public::SessionIdV1([1; 16]),
            expected_revision: 8,
            execution: execution(),
            now: 99,
        })
        .unwrap();
    let public::ReplyV1::Accepted(public::OutcomeV1::Session(view)) = result else {
        panic!("session view expected")
    };
    assert_eq!(view.phase, public::PhaseV1::ReattachRequired);

    let Some((
        service_session,
        upstream::AgentCommandV1::Reattach {
            session,
            expected_revision,
            execution,
            now,
        },
    )) = adapter.into_port().seen
    else {
        panic!("reattach dispatch expected")
    };
    assert_eq!(service_session, daimon::SessionHandleV1(44));
    assert_eq!(session, daimon::AgentSessionIdV1::new([1; 16]));
    assert_eq!(expected_revision, 8);
    assert_eq!(execution.task_attempt.attempt_number, 4);
    assert_eq!(now, 99);
}

fn execution() -> public::ExecutionV1 {
    public::ExecutionV1 {
        id: public::ExecutionIdV1([3; 16]),
        attempt: public::AttemptV1 {
            task_key: [4; 16],
            incarnation: 2,
            attempt: 4,
        },
        principal: public::PrincipalIdV1([5; 16]),
        profile: public::ProfileIdV1([6; 16]),
        idempotency_key: public::IdempotencyKeyV1([7; 16]),
        context: vec![public::ContextIdV1([8; 16])],
        tools: vec![public::ToolIdV1([9; 16])],
        budget: public::ExecutionBudgetV1 {
            max_elapsed_millis: 1_000,
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 1,
            max_context_bytes: 4_096,
            max_output_bytes: 4_096,
        },
    }
}
