use daimon_client::ModelServicePort;
use daimon_host::{
    AssembledModelInputV1, AttemptRefusalV1, AttemptValidator, AuthenticatedSessionV1,
    AuthorityGrantV1, AuthorityRefusalV1, ContextAssembler, ContextRefusalV1, ExecutionAuthorizer,
    ModelProvider, ModelProviderOutcomeV1, ProfileRefusalV1, ProviderProfileResolver,
};
use daimon_protocol as daimon;
use daimon_service::{ServiceLimitsV1, model::ModelService};
use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, DeliberationPlanV1, EnsembleMemberIdV1, EnsembleMemberV1, GroundingRefV1,
    MaybeValueV1, MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1, ReportIdV1, ResolutionV1,
    SurfaceDepthV1,
    registry::{DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryRevisionV1},
};
use moira_daimon::{
    CompletedToolResultV1, ExecutionIntentV1, ModelExecutionAdapter, ModelExecutionReplyV1,
    ModelProposalV1,
    ensemble::{
        self, EnsembleRefusalV1, MemberContinuationV1, MemberOutcomeV1, MemberResponseLimitsV1,
        MemberResponseRefusalV1, MemberRunV1,
    },
};
use moira_protocol::{DispositionV1, EffectIdV1, TaskRefV1};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    convert::Infallible,
    rc::Rc,
};

const MEMBER_RESPONSE: &[u8] = br#"{"schema":"moira.member-response.v1","claims":[{"kind":"empirical","statement":"local model completed","evidence_digests":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"dissent_digests":["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]}]}"#;

struct Allow;

impl ExecutionAuthorizer for Allow {
    fn authorize(
        &mut self,
        _session: &AuthenticatedSessionV1,
        request: &daimon::ExecuteRequestV1,
        _now: u64,
    ) -> Result<AuthorityGrantV1, AuthorityRefusalV1> {
        Ok(AuthorityGrantV1 {
            decision_id: [20; 16],
            expires_at: 100,
            tools: request.tools.clone(),
        })
    }
}

struct Current;

impl AttemptValidator for Current {
    fn validate(
        &mut self,
        _session: &AuthenticatedSessionV1,
        _request: &daimon::ExecuteRequestV1,
    ) -> Result<(), AttemptRefusalV1> {
        Ok(())
    }
}

struct Profile;

impl ProviderProfileResolver for Profile {
    fn resolve_profile(
        &mut self,
        _session: &AuthenticatedSessionV1,
        request: &daimon::ExecuteRequestV1,
    ) -> Result<daimon::ProviderProfileSnapshotV1, ProfileRefusalV1> {
        Ok(daimon::ProviderProfileSnapshotV1 {
            provider: daimon::ProviderProfileIdV1::new([21; 16]),
            harness_profile: request.profile,
            wire: daimon::ProviderWireProfileV1::OpenAiChatCompletions,
            model: b"local".to_vec(),
            structured_tools: true,
            document_digest: daimon::ContentDigestV1::new([22; 32]),
        })
    }
}

struct Context;

impl ContextAssembler for Context {
    fn assemble(
        &mut self,
        _session: &AuthenticatedSessionV1,
        request: &daimon::ExecuteRequestV1,
        _profile: &daimon::ProviderProfileSnapshotV1,
    ) -> Result<AssembledModelInputV1, ContextRefusalV1> {
        Ok(AssembledModelInputV1 {
            context: vec![daimon::ContextEvidenceV1 {
                reference: request.context[0],
                kind: daimon::ContextKindV1::AgoraMessage,
                bytes: b"message".to_vec(),
                source_digest: daimon::ContentDigestV1::new([23; 32]),
                grounding: None,
            }],
            messages: vec![daimon::ModelMessageV1 {
                role: daimon::MessageRoleV1::User,
                content: b"message".to_vec(),
            }],
            estimated_input_tokens: 2,
        })
    }
}

struct Provider {
    calls: Rc<Cell<usize>>,
    output: Vec<u8>,
}

impl ModelProvider for Provider {
    fn execute_model(&mut self, invocation: &daimon::ModelInvocationV1) -> ModelProviderOutcomeV1 {
        self.calls.set(self.calls.get() + 1);
        ModelProviderOutcomeV1::Receipt(daimon::ModelProviderReceiptV1 {
            execution_id: invocation.execution_id,
            provider_document_digest: invocation.provider.document_digest,
            evidence_digest: daimon::ContentDigestV1::new([24; 32]),
            disposition: daimon::ExecutionDispositionV1::Completed,
            input_tokens: 2,
            output_tokens: 3,
            output: self.output.clone(),
            proposals: {
                let mut proposals = vec![
                    daimon::OutputProposalV1::AgoraMessage(daimon::AgoraMessageProposalV1 {
                        room: [25; 16],
                        reply_to: [26; 16],
                        body: b"answer".to_vec(),
                    }),
                    daimon::OutputProposalV1::VellumEdit(daimon::VellumEditProposalV1 {
                        workspace: [27; 16],
                        document: [28; 16],
                        expected_revision: 4,
                        replacement: b"edit".to_vec(),
                    }),
                ];
                if invocation.execution_id.into_bytes() == [2; 16]
                    || invocation.execution_id.into_bytes() == [40; 16]
                {
                    proposals.push(daimon::OutputProposalV1::ToolCall(
                        daimon::ToolCallProposalV1 {
                            request: [29; 16],
                            capability: daimon::ToolCapabilityIdV1::new([30; 16]),
                            provider_correlation: b"call-29".to_vec(),
                            arguments: br#"{"path":"README.md"}"#.to_vec(),
                        },
                    ));
                }
                proposals
            },
        })
    }
}

struct Port(ModelService<Allow, Current, Profile, Context, Provider>);

struct ToolPort(Rc<RefCell<Vec<moira_protocol::tool_continuation::CommandV1>>>);

impl moira_client::ToolContinuationServicePort for ToolPort {
    type Error = Infallible;

    fn execute(
        &mut self,
        command: moira_protocol::tool_continuation::CommandV1,
    ) -> Result<moira_protocol::tool_continuation::ReplyV1, Self::Error> {
        self.0.borrow_mut().push(command);
        Ok(moira_protocol::tool_continuation::ReplyV1::Refused(
            moira_protocol::tool_continuation::RefusalV1::NotAuthorized,
        ))
    }
}

impl ModelServicePort for Port {
    type Error = Infallible;

    fn execute_model(
        &mut self,
        session: daimon::SessionHandleV1,
        command: daimon::ModelCommandV1,
    ) -> Result<daimon::ModelReplyV1, Self::Error> {
        Ok(self.0.execute(session, command))
    }
}

fn intent() -> ExecutionIntentV1 {
    ExecutionIntentV1 {
        effect: EffectIdV1([1; 16]),
        execution_id: [2; 16],
        task: TaskRefV1 {
            external_task_key: [3; 16],
            incarnation: 4,
        },
        attempt_number: 1,
        principal: [5; 16],
        profile: [6; 16],
        idempotency_key: [7; 16],
        context: vec![[8; 16]],
        tools: vec![[30; 16]],
        budget: daimon::ExecutionBudgetV1 {
            max_elapsed_millis: 1_000,
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 1,
            max_context_bytes: 1_024,
            max_output_bytes: 1_024,
        },
    }
}

fn ensemble_plan() -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([30; 16]),
        question_digest: [29; 32],
        method: MethodRefV1 {
            machine_digest: [31; 32],
            state_digest: [32; 32],
        },
        claims: vec![ClaimKindV1::Empirical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([33; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([34; 16]),
            scores: vec![1],
        }],
        members: vec![
            EnsembleMemberV1 {
                id: EnsembleMemberIdV1([35; 16]),
                lens: PersonaLensV1::Researcher,
                concern_root_digest: [36; 32],
                grounding: GroundingRefV1 {
                    authority_digest: [37; 32],
                    surface_digest: [38; 32],
                    depth: SurfaceDepthV1::Ambient,
                },
            },
            EnsembleMemberV1 {
                id: EnsembleMemberIdV1([39; 16]),
                lens: PersonaLensV1::RigorReviewer,
                concern_root_digest: [40; 32],
                grounding: GroundingRefV1 {
                    authority_digest: [41; 32],
                    surface_digest: [42; 32],
                    depth: SurfaceDepthV1::Activated,
                },
            },
        ],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [43; 32],
            result_digest: [44; 32],
            all_orthogonal: true,
        },
        fuel: 3,
    }
}

fn member_run(member: EnsembleMemberIdV1, seed: u8) -> MemberRunV1 {
    let mut execution = intent();
    execution.effect = EffectIdV1([seed; 16]);
    execution.execution_id = [seed + 1; 16];
    execution.idempotency_key = [seed + 2; 16];
    execution.context = vec![[seed + 3; 16]];
    MemberRunV1 {
        member,
        report: ReportIdV1([seed + 4; 16]),
        execution,
    }
}

#[test]
fn model_result_retains_moira_effect_attempt_and_typed_proposals_on_replay() {
    let calls = Rc::new(Cell::new(0));
    let mut service = ModelService::new(
        daimon_harness::Limits::default(),
        ServiceLimitsV1::default(),
        Allow,
        Current,
        Profile,
        Context,
        Provider {
            calls: Rc::clone(&calls),
            output: MEMBER_RESPONSE.to_vec(),
        },
    )
    .unwrap();
    let session = service
        .open_session(AuthenticatedSessionV1 {
            principal: [5; 16],
            authority_context: [9; 16],
        })
        .unwrap();
    let mut adapter = ModelExecutionAdapter::new(session, Port(service));
    let first = adapter.execute(intent(), 10).unwrap();
    assert_eq!(adapter.execute(intent(), 10).unwrap(), first);
    assert_eq!(calls.get(), 1);
    let ModelExecutionReplyV1::Result(result) = first else {
        panic!("model result expected")
    };
    assert_eq!(result.receipt.effect, EffectIdV1([1; 16]));
    assert_eq!(result.receipt.task.incarnation, 4);
    assert_eq!(result.receipt.disposition, DispositionV1::Completed);
    assert_eq!(result.proposals.len(), 3);
    assert!(matches!(
        &result.proposals[0],
        ModelProposalV1::AgoraMessage { room, .. } if *room == [25; 16]
    ));
    let calls = moira_daimon::tool_calls(
        &result,
        moira_protocol::tool_continuation::ToolBudgetV1 {
            max_elapsed_millis: 250,
            max_output_bytes: 4_096,
        },
    );
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].request.0, [29; 16]);
    assert_eq!(calls[0].budget.max_elapsed_millis, 250);
    let captured = Rc::new(RefCell::new(Vec::new()));
    let mut continuation =
        moira_client::ToolContinuationMutationClient::new(ToolPort(Rc::clone(&captured)));
    let reply = moira_daimon::admit_tool_continuation(
        &mut continuation,
        Some(7),
        moira_protocol::WorkflowIdV1([31; 16]),
        result.receipt.effect,
        &result,
        moira_protocol::tool_continuation::ToolBudgetV1 {
            max_elapsed_millis: 250,
            max_output_bytes: 4_096,
        },
    )
    .unwrap();
    assert!(matches!(
        reply,
        Some(moira_protocol::tool_continuation::ReplyV1::Refused(
            moira_protocol::tool_continuation::RefusalV1::NotAuthorized
        ))
    ));
    assert!(matches!(
        captured.borrow().as_slice(),
        [moira_protocol::tool_continuation::CommandV1::Apply {
            expected_storage_revision: Some(7),
            mutation: moira_protocol::tool_continuation::MutationV1::Admit {
                workflow,
                source_execution,
                calls,
                ..
            }
        }] if workflow.0 == [31; 16]
            && source_execution.0 == [1; 16]
            && calls.len() == 1
    ));
    let authority = moira_protocol::tool_continuation::AuthorityEvidenceV1 {
        decision: moira_protocol::tool_continuation::AuthorityDecisionIdV1([35; 16]),
        subject: moira_protocol::tool_continuation::AuthoritySubjectIdV1([36; 16]),
        capability: moira_protocol::tool_continuation::ToolCapabilityIdV1([30; 16]),
        epoch: 1,
    };
    let attempt = moira_protocol::tool_continuation::AttemptEvidenceV1 {
        task_key: [3; 16],
        incarnation: 4,
        attempt: 1,
    };
    let _ = moira_daimon::authorize_tool_effect(
        &mut continuation,
        Some(8),
        moira_protocol::WorkflowIdV1([31; 16]),
        EffectIdV1([37; 16]),
        authority,
        attempt,
    )
    .unwrap();
    assert!(matches!(
        captured.borrow().last(),
        Some(moira_protocol::tool_continuation::CommandV1::Apply {
            expected_storage_revision: Some(8),
            mutation: moira_protocol::tool_continuation::MutationV1::Authorize {
                workflow,
                effect,
                gate
            }
        }) if workflow.0 == [31; 16]
            && effect.0 == [37; 16]
            && gate.authority == authority
            && gate.attempt == attempt
    ));
    assert!(matches!(
        &result.proposals[1],
        ModelProposalV1::VellumEdit { document, expected_revision, .. }
            if *document == [28; 16] && *expected_revision == 4
    ));
    assert!(matches!(
        &result.proposals[2],
        ModelProposalV1::ToolCall { request, capability, provider_correlation, arguments }
            if *request == [29; 16]
                && *capability == [30; 16]
                && provider_correlation == b"call-29"
                && arguments == br#"{"path":"README.md"}"#
    ));
}

#[test]
fn completed_tool_output_resumes_the_same_task_through_daimons_native_contract() {
    let calls = Rc::new(Cell::new(0));
    let service = ModelService::new(
        daimon_harness::Limits::default(),
        ServiceLimitsV1 {
            max_sessions: 2,
            max_executions: 4,
        },
        Allow,
        Current,
        Profile,
        Context,
        Provider {
            calls: Rc::clone(&calls),
            output: MEMBER_RESPONSE.to_vec(),
        },
    )
    .unwrap();
    let mut service = service;
    let session = service
        .open_session(AuthenticatedSessionV1 {
            principal: [5; 16],
            authority_context: [19; 16],
        })
        .unwrap();
    let mut adapter = ModelExecutionAdapter::new(session, Port(service));
    let source = intent();
    assert!(matches!(
        adapter.execute(source.clone(), 10).unwrap(),
        ModelExecutionReplyV1::Result(_)
    ));
    let output = b"README contents".to_vec();
    let mut resumed = source;
    resumed.execution_id = [40; 16];
    resumed.idempotency_key = [41; 16];
    let completed_results = vec![CompletedToolResultV1 {
        request: [29; 16],
        evidence_digest: [42; 32],
        output_digest: Sha256::digest(&output).into(),
        output,
    }];
    let plan = ensemble_plan();
    let outcome = ensemble::resume_member(
        &mut adapter,
        &plan,
        MemberRunV1 {
            member: plan.members[0].id,
            report: ReportIdV1([43; 16]),
            execution: resumed,
        },
        MemberContinuationV1 {
            member: plan.members[0].id,
            report: ReportIdV1([43; 16]),
            source_execution: [2; 16],
        },
        completed_results.clone(),
        11,
    )
    .unwrap();
    let MemberOutcomeV1::ToolContinuation { continuation, .. } = outcome else {
        panic!("second tool continuation expected")
    };

    let second_output = b"second tool result".to_vec();
    let second_results = vec![CompletedToolResultV1 {
        request: [29; 16],
        evidence_digest: [44; 32],
        output_digest: Sha256::digest(&second_output).into(),
        output: second_output,
    }];
    let mut terminal = intent();
    terminal.execution_id = [50; 16];
    terminal.idempotency_key = [51; 16];
    let terminal_for_replay = terminal.clone();
    let outcome = ensemble::resume_member(
        &mut adapter,
        &plan,
        MemberRunV1 {
            member: plan.members[0].id,
            report: ReportIdV1([43; 16]),
            execution: terminal,
        },
        continuation,
        second_results.clone(),
        12,
    )
    .unwrap();
    assert!(matches!(outcome, MemberOutcomeV1::Report { .. }));
    assert_eq!(calls.get(), 3);

    let Port(service) = adapter.into_port();
    let (authorizer, attempts, profiles, context, provider, store) = service.into_parts();
    let mut reopened = ModelService::open(
        daimon_harness::Limits::default(),
        ServiceLimitsV1 {
            max_sessions: 2,
            max_executions: 4,
        },
        authorizer,
        attempts,
        profiles,
        context,
        provider,
        store,
    )
    .unwrap();
    let session = reopened
        .open_session(AuthenticatedSessionV1 {
            principal: [5; 16],
            authority_context: [19; 16],
        })
        .unwrap();
    let mut reopened = ModelExecutionAdapter::new(session, Port(reopened));
    assert!(matches!(
        reopened
            .resume(terminal_for_replay, [40; 16], second_results, 13,)
            .unwrap(),
        ModelExecutionReplyV1::Result(_)
    ));
    assert_eq!(calls.get(), 3);
}

#[test]
fn orthogonal_members_execute_with_distinct_identities_and_become_attributed_reports() {
    let calls = Rc::new(Cell::new(0));
    let mut service = ModelService::new(
        daimon_harness::Limits::default(),
        ServiceLimitsV1::default(),
        Allow,
        Current,
        Profile,
        Context,
        Provider {
            calls: Rc::clone(&calls),
            output: MEMBER_RESPONSE.to_vec(),
        },
    )
    .unwrap();
    let session = service
        .open_session(AuthenticatedSessionV1 {
            principal: [5; 16],
            authority_context: [9; 16],
        })
        .unwrap();
    let mut adapter = ModelExecutionAdapter::new(session, Port(service));
    let plan = ensemble_plan();
    let outcomes = ensemble::execute(
        &mut adapter,
        &plan,
        vec![
            member_run(plan.members[0].id, 50),
            member_run(plan.members[1].id, 60),
        ],
        10,
    )
    .unwrap();
    assert_eq!(calls.get(), 2);
    assert_eq!(outcomes.len(), 2);
    let MemberOutcomeV1::Report {
        report, response, ..
    } = &outcomes[0]
    else {
        panic!("first report expected")
    };
    assert_eq!(report.member, plan.members[0].id);
    assert_eq!(report.evidence_digests[0], plan.question_digest);
    assert_eq!(report.evidence_digests[1], plan.method.machine_digest);
    assert_eq!(
        report.evidence_digests[4],
        plan.members[0].grounding.surface_digest
    );
    assert_eq!(report.evidence_digests[5], [24; 32]);
    assert_eq!(report.evidence_digests[6], [0xaa; 32]);
    assert_eq!(report.dissent_digests, vec![[0xbb; 32]]);
    assert_eq!(
        response.claims[0].statement_digest,
        <[u8; 32]>::from(Sha256::digest(b"local model completed"))
    );

    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(plan.clone()),
        )
        .unwrap();
    for outcome in outcomes {
        let MemberOutcomeV1::Report { report, .. } = outcome else {
            panic!("complete cohort report expected")
        };
        registry
            .apply(
                registry.revision(),
                RegistryCommandV1::SubmitReport {
                    deliberation: plan.id,
                    report,
                },
            )
            .unwrap();
    }
    registry
        .apply(
            RegistryRevisionV1(3),
            RegistryCommandV1::Resolve {
                deliberation: plan.id,
                resolution: ResolutionV1::Maybe(MaybeValueV1::Both),
            },
        )
        .unwrap();
    assert_eq!(registry.revision(), RegistryRevisionV1(4));
}

#[test]
fn duplicate_execution_identity_refuses_the_whole_cohort_before_dispatch() {
    let calls = Rc::new(Cell::new(0));
    let mut service = ModelService::new(
        daimon_harness::Limits::default(),
        ServiceLimitsV1::default(),
        Allow,
        Current,
        Profile,
        Context,
        Provider {
            calls: Rc::clone(&calls),
            output: MEMBER_RESPONSE.to_vec(),
        },
    )
    .unwrap();
    let session = service
        .open_session(AuthenticatedSessionV1 {
            principal: [5; 16],
            authority_context: [9; 16],
        })
        .unwrap();
    let mut adapter = ModelExecutionAdapter::new(session, Port(service));
    let plan = ensemble_plan();
    let first = member_run(plan.members[0].id, 50);
    let mut second = member_run(plan.members[1].id, 60);
    second.execution.execution_id = first.execution.execution_id;
    assert_eq!(
        ensemble::execute(&mut adapter, &plan, vec![first, second], 10),
        Err(EnsembleRefusalV1::DuplicateExecution)
    );
    assert_eq!(calls.get(), 0);
}

#[test]
fn completed_prose_is_attributed_but_never_admitted_as_a_report() {
    let calls = Rc::new(Cell::new(0));
    let mut service = ModelService::new(
        daimon_harness::Limits::default(),
        ServiceLimitsV1::default(),
        Allow,
        Current,
        Profile,
        Context,
        Provider {
            calls: Rc::clone(&calls),
            output: b"plausible prose is not structured evidence".to_vec(),
        },
    )
    .unwrap();
    let session = service
        .open_session(AuthenticatedSessionV1 {
            principal: [5; 16],
            authority_context: [9; 16],
        })
        .unwrap();
    let mut adapter = ModelExecutionAdapter::new(session, Port(service));
    let plan = ensemble_plan();
    let outcomes = ensemble::execute(
        &mut adapter,
        &plan,
        vec![
            member_run(plan.members[0].id, 50),
            member_run(plan.members[1].id, 60),
        ],
        10,
    )
    .unwrap();
    assert_eq!(calls.get(), 2);
    assert!(outcomes.iter().all(|outcome| matches!(
        outcome,
        MemberOutcomeV1::InvalidReport {
            refusal: MemberResponseRefusalV1::InvalidJson,
            ..
        }
    )));
}

#[test]
fn member_response_is_closed_to_admitted_claim_kinds_and_exact_digest_shape() {
    let plan = ensemble_plan();
    let limits = MemberResponseLimitsV1::default();
    assert!(ensemble::parse_member_response(&plan, MEMBER_RESPONSE, limits).is_ok());

    let unknown_field = br#"{"schema":"moira.member-response.v1","claims":[],"answer":true}"#;
    assert_eq!(
        ensemble::parse_member_response(&plan, unknown_field, limits),
        Err(MemberResponseRefusalV1::InvalidJson)
    );

    let unadmitted = br#"{"schema":"moira.member-response.v1","claims":[{"kind":"formal","statement":"local model completed","evidence_digests":[],"dissent_digests":[]}]}"#;
    assert_eq!(
        ensemble::parse_member_response(&plan, unadmitted, limits),
        Err(MemberResponseRefusalV1::UnadmittedClaimKind)
    );

    let uppercase_digest = br#"{"schema":"moira.member-response.v1","claims":[{"kind":"empirical","statement":"local model completed","evidence_digests":["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"],"dissent_digests":[]}]}"#;
    assert_eq!(
        ensemble::parse_member_response(&plan, uppercase_digest, limits),
        Err(MemberResponseRefusalV1::InvalidDigest)
    );
}
