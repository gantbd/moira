use moira_client::AgentParticipationServicePort;
use moira_host::{AgentPromptCommitV1, AgentPromptRecordV1, AgentPromptStoreV1};
use moira_local_harness::agent_runtime::{
    AgentCommandMaterializerV1, AgentCommandTemplateV1, FixedAgentCommandPolicyV1,
    FixedAgentCommandTemplateErrorV1, FixedAgentCommandTemplateV1, PreparedParticipationV1,
    PromptMaterializationLoadErrorV1, RetainedPromptMaterializerV1,
    StoredPromptMaterializerErrorV1, StoredPromptMaterializerV1, materialize_pending_prompt,
};
use moira_protocol::{WorkflowIdV1, agent, agent_participation as p, deliberation as d};
use sha2::{Digest, Sha256};

struct PromptStore {
    value: Result<Option<Vec<u8>>, &'static str>,
    gets: usize,
}

impl AgentPromptStoreV1 for PromptStore {
    type Error = &'static str;

    fn put(&mut self, prompt: &[u8]) -> Result<AgentPromptCommitV1, Self::Error> {
        Ok(AgentPromptCommitV1 {
            record: AgentPromptRecordV1 {
                prompt_digest: Sha256::digest(prompt).into(),
                prompt_bytes: prompt.len(),
            },
            replayed: false,
        })
    }

    fn get(&mut self, _prompt_digest: [u8; 32]) -> Result<Option<Vec<u8>>, Self::Error> {
        self.gets += 1;
        self.value.clone()
    }
}

#[derive(Default)]
struct Template {
    prompt: Option<Vec<u8>>,
}

impl AgentCommandTemplateV1 for Template {
    type Error = &'static str;

    fn materialize(
        &mut self,
        participation: &p::ParticipationViewV1,
        _operation: p::OperationV1,
        prompt: Option<&[u8]>,
        _now: u64,
    ) -> Result<agent::CommandV1, Self::Error> {
        self.prompt = prompt.map(<[u8]>::to_vec);
        Ok(agent::CommandV1::Inspect {
            session: agent::SessionIdV1(participation.binding.session.0),
        })
    }
}

struct ParticipationPort {
    reply: p::ReplyV1,
    command: Option<p::CommandV1>,
}

impl AgentParticipationServicePort for ParticipationPort {
    type Error = &'static str;

    fn execute_participation(&mut self, command: p::CommandV1) -> Result<p::ReplyV1, Self::Error> {
        self.command = Some(command);
        Ok(self.reply.clone())
    }
}

#[test]
fn daemon_materialization_is_correlated_to_the_exact_prepared_prompt() {
    let bytes = b"daemon-owned grounded prompt".to_vec();
    let digest = Sha256::digest(&bytes).into();
    let prepared = prepared_prompt(digest);
    let mut port = ParticipationPort {
        reply: p::ReplyV1::Accepted(Box::new(p::OutcomeV1::PromptMaterialization(
            p::PromptMaterializationV1 {
                participation: prepared.participation.binding.participation,
                participation_revision: prepared.participation.revision,
                prompt_digest: digest,
                prompt: bytes.clone(),
            },
        ))),
        command: None,
    };
    assert_eq!(
        materialize_pending_prompt(&mut port, &prepared),
        Ok(Some(bytes))
    );
    assert_eq!(
        port.command,
        Some(p::CommandV1::MaterializePrompt(
            p::PromptMaterializationRequestV1 {
                participation: prepared.participation.binding.participation,
                expected_revision: prepared.participation.revision,
                prompt_digest: digest,
            }
        ))
    );
}

#[test]
fn daemon_materialization_refusal_and_mismatched_reply_fail_closed() {
    let bytes = b"daemon-owned grounded prompt".to_vec();
    let digest = Sha256::digest(&bytes).into();
    let prepared = prepared_prompt(digest);
    let mut refused = ParticipationPort {
        reply: p::ReplyV1::Refused(p::RefusalV1::NotAuthorized),
        command: None,
    };
    assert_eq!(
        materialize_pending_prompt(&mut refused, &prepared),
        Err(PromptMaterializationLoadErrorV1::Refused(
            p::RefusalV1::NotAuthorized
        ))
    );

    let mut mismatched = ParticipationPort {
        reply: p::ReplyV1::Accepted(Box::new(p::OutcomeV1::PromptMaterialization(
            p::PromptMaterializationV1 {
                participation: prepared.participation.binding.participation,
                participation_revision: prepared.participation.revision + 1,
                prompt_digest: digest,
                prompt: bytes,
            },
        ))),
        command: None,
    };
    assert_eq!(
        materialize_pending_prompt(&mut mismatched, &prepared),
        Err(PromptMaterializationLoadErrorV1::Integrity)
    );
}

#[test]
fn retained_prompt_is_rechecked_immediately_before_template_materialization() {
    let bytes = b"retained exact prompt".to_vec();
    let digest = Sha256::digest(&bytes).into();
    let mut exact = RetainedPromptMaterializerV1::new(Some(bytes.clone()), Template::default());
    assert!(matches!(
        exact.materialize(&view(), prompt_operation(digest), 9),
        Ok(agent::CommandV1::Inspect { .. })
    ));
    assert_eq!(exact.into_parts().1.prompt, Some(bytes));

    let mut corrupt =
        RetainedPromptMaterializerV1::new(Some(b"wrong".to_vec()), Template::default());
    assert_eq!(
        corrupt.materialize(&view(), prompt_operation(digest), 9),
        Err(StoredPromptMaterializerErrorV1::Integrity)
    );
    assert_eq!(corrupt.into_parts().1.prompt, None);
}

#[test]
fn exact_retained_prompt_is_the_only_content_given_to_the_template() {
    let bytes = b"retained grounded prompt".to_vec();
    let digest = Sha256::digest(&bytes).into();
    let operation = prompt_operation(digest);
    let mut materializer = StoredPromptMaterializerV1::new(
        PromptStore {
            value: Ok(Some(bytes.clone())),
            gets: 0,
        },
        Template::default(),
    );
    assert!(matches!(
        materializer.materialize(&view(), operation, 9),
        Ok(agent::CommandV1::Inspect { .. })
    ));
    let (store, template) = materializer.into_parts();
    assert_eq!(store.gets, 1);
    assert_eq!(template.prompt, Some(bytes));
}

#[test]
fn missing_corrupt_or_backend_prompt_content_fails_before_the_template() {
    let digest = Sha256::digest(b"expected").into();
    for (value, expected) in [
        (Ok(None), StoredPromptMaterializerErrorV1::Missing),
        (
            Ok(Some(b"wrong".to_vec())),
            StoredPromptMaterializerErrorV1::Integrity,
        ),
        (
            Err("store failed"),
            StoredPromptMaterializerErrorV1::Store("store failed"),
        ),
    ] {
        let mut materializer =
            StoredPromptMaterializerV1::new(PromptStore { value, gets: 0 }, Template::default());
        assert_eq!(
            materializer.materialize(&view(), prompt_operation(digest), 9),
            Err(expected)
        );
        assert_eq!(materializer.into_parts().1.prompt, None);
    }
}

#[test]
fn fixed_template_derives_exact_prompt_identity_from_retained_participation() {
    let bytes = b"retained grounded prompt";
    let digest = Sha256::digest(bytes).into();
    let operation = prompt_operation(digest);
    let mut template = FixedAgentCommandTemplateV1::new(policy()).unwrap();
    let agent::CommandV1::Prompt { request, now } = template
        .materialize(&view(), operation, Some(bytes), 19)
        .unwrap()
    else {
        panic!("prompt command expected")
    };
    assert_eq!(now, 19);
    assert_eq!(request.session, agent::SessionIdV1([5; 16]));
    assert_eq!(request.turn, agent::TurnIdV1([16; 16]));
    assert_eq!(request.expected_revision, 15);
    assert_eq!(request.execution.id, agent::ExecutionIdV1([17; 16]));
    assert_eq!(
        request.execution.idempotency_key,
        agent::IdempotencyKeyV1([15; 16])
    );
    assert_eq!(request.execution.profile, agent::ProfileIdV1([6; 16]));
    assert_eq!(request.execution.attempt.attempt, 9);
    assert_eq!(request.prompt, bytes);
}

#[test]
fn fixed_template_refuses_missing_prompt_duplicate_policy_and_revision() {
    let mut duplicate = policy();
    duplicate.tools.push(duplicate.tools[0]);
    assert!(matches!(
        FixedAgentCommandTemplateV1::new(duplicate),
        Err(FixedAgentCommandTemplateErrorV1::InvalidPolicy)
    ));
    let mut oversized = policy();
    oversized.context = (0_u8..65)
        .map(|value| agent::ContextIdV1([value; 16]))
        .collect();
    assert!(matches!(
        FixedAgentCommandTemplateV1::new(oversized),
        Err(FixedAgentCommandTemplateErrorV1::InvalidPolicy)
    ));
    let mut template = FixedAgentCommandTemplateV1::new(policy()).unwrap();
    assert_eq!(
        template.materialize(&view(), prompt_operation([1; 32]), None, 19),
        Err(FixedAgentCommandTemplateErrorV1::MissingPrompt)
    );
    let mut without_revision = view();
    without_revision.session_revision = None;
    assert_eq!(
        template.materialize(
            &without_revision,
            p::OperationV1::Advance {
                key: p::IdempotencyKeyV1([20; 16]),
                turn: p::AgentTurnIdV1([21; 16]),
            },
            None,
            19,
        ),
        Err(FixedAgentCommandTemplateErrorV1::MissingSessionRevision)
    );
}

fn policy() -> FixedAgentCommandPolicyV1 {
    FixedAgentCommandPolicyV1 {
        principal: agent::PrincipalIdV1([30; 16]),
        workspace: [31; 16],
        context: vec![agent::ContextIdV1([32; 16])],
        tools: vec![agent::ToolIdV1([33; 16])],
        execution_budget: agent::ExecutionBudgetV1 {
            max_elapsed_millis: 1_000,
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 1,
            max_context_bytes: 1_024,
            max_output_bytes: 1_024,
        },
        session_budget: agent::SessionBudgetV1 {
            max_elapsed_millis: 10_000,
            max_turns: 4,
            max_events: 32,
            max_event_bytes: 4_096,
            max_prompt_bytes: 4_096,
            max_permission_requests: 4,
        },
    }
}

fn prompt_operation(prompt_digest: [u8; 32]) -> p::OperationV1 {
    p::OperationV1::Prompt {
        key: p::IdempotencyKeyV1([15; 16]),
        turn: p::AgentTurnIdV1([16; 16]),
        execution: p::ExecutionIdV1([17; 16]),
        prompt_digest,
        attempt: attempt(),
    }
}

fn attempt() -> p::AttemptV1 {
    p::AttemptV1 {
        task_key: [7; 16],
        incarnation: 8,
        attempt: 9,
    }
}

fn view() -> p::ParticipationViewV1 {
    p::ParticipationViewV1 {
        binding: p::BindingV1 {
            participation: p::ParticipationIdV1([1; 16]),
            workflow: WorkflowIdV1([2; 16]),
            deliberation: d::DeliberationIdV1([3; 16]),
            member: d::EnsembleMemberIdV1([4; 16]),
            session: p::AgentSessionIdV1([5; 16]),
            profile: p::ProfileIdV1([6; 16]),
            attempt: attempt(),
            method: d::MethodRefV1 {
                machine_digest: [10; 32],
                state_digest: [11; 32],
            },
            grounding: d::GroundingRefV1 {
                authority_digest: [12; 32],
                surface_digest: [13; 32],
                depth: d::SurfaceDepthV1::Reference,
            },
            persona: d::PersonaLensV1::RigorReviewer,
        },
        revision: 14,
        phase: p::PhaseV1::Ready,
        session_revision: Some(15),
        last_event_cursor: None,
        profile_document_digest: Some([18; 32]),
        capabilities: p::CapabilitiesV1::default(),
        active_turn: None,
        permission: None,
        terminal: None,
        pending: None,
    }
}

fn prepared_prompt(prompt_digest: [u8; 32]) -> PreparedParticipationV1 {
    let mut participation = view();
    participation.pending = Some(prompt_operation(prompt_digest));
    PreparedParticipationV1 {
        storage_revision: 22,
        participation,
    }
}
