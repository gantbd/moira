#![cfg(unix)]

use daimon_acp::{AcpLaunchProfileV1, AcpLimitsV1, AcpProvider, AcpWorkspaceV1};
use daimon_agent_router::{AgentProviderRouteV1, AgentProviderRouter, StaticAgentProfileRouter};
use daimon_claude::{ClaudeLaunchProfileV1, ClaudeLimitsV1, ClaudeProvider, ClaudeWorkspaceV1};
use daimon_harness::Limits as DaimonCoreLimits;
use daimon_host::{
    AttemptRefusalV1, AttemptValidator, AuthenticatedSessionV1, AuthorityGrantV1,
    AuthorityRefusalV1, ExecutionAuthorizer, agent::AgentContextAssembler,
};
use daimon_local_daemon::unix::{AgentDaemon, Authenticator as DaimonAuthenticator};
use daimon_protocol::{self as dp, agent::*};
use daimon_service::agent::{AgentService, AgentServiceLimitsV1, VolatileAgentStore};
use daimon_wire::{LimitsV1 as DaimonLimitsV1, agent::AgentWireLimitsV1};
use moira_agent_local::{
    ConfigV1, ExecutionBudgetConfigV1, OutputConfigV1, PolicyConfigV1, SessionBudgetConfigV1,
    TransportConfigV1,
};
use moira_core::{
    agent_participation::{LimitsV1 as ParticipationDomainLimits, persistence::CodecLimitsV1},
    deliberation::{
        CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
        DeliberationCheckpointV1, DeliberationIdV1, DeliberationPlanV1, EnsembleMemberIdV1,
        EnsembleMemberV1, GroundingRefV1, MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1,
        SurfaceDepthV1,
        registry::{
            DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryReceiptV1,
            RegistryRevisionV1,
        },
    },
};
use moira_daimon::ensemble::MemberResponseLimitsV1;
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};
use moira_local_daemon::unix::{AgentParticipationAuthenticator, AgentParticipationDaemon};
use moira_local_harness::agent_report::admit_terminal_output;
use moira_protocol::{WorkflowIdV1, agent_participation as p, deliberation as d};
use moira_service::{AccessV1, agent_participation::AgentParticipationService};
use moira_sqlite::{
    agent_output::{AgentOutputSqliteStore, AgentOutputStoreLimitsV1},
    agent_participation::{AgentParticipationSqliteStore, AgentParticipationStoreLimitsV1},
    agent_prompt::{AgentPromptSqliteStore, AgentPromptStoreLimitsV1},
};
use moira_wire::LimitsV1 as MoiraLimitsV1;
use std::{
    convert::Infallible,
    ffi::OsString,
    fs,
    os::unix::net::UnixStream,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

struct ParticipationAuthentication;

impl AgentParticipationAuthenticator for ParticipationAuthentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AccessV1, Self::Error> {
        Ok(AccessV1 {
            read: true,
            mutate: true,
        })
    }

    fn authorize_permission(
        &mut self,
        _: p::PermissionSelectionV1,
    ) -> Result<Option<moira_service::agent_participation::PermissionAccessV1>, Self::Error> {
        Ok(None)
    }

    fn authorize_prompt_materialization(
        &mut self,
        _: p::PromptMaterializationRequestV1,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

struct DaimonAuthentication;

impl DaimonAuthenticator for DaimonAuthentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AuthenticatedSessionV1, Self::Error> {
        Ok(AuthenticatedSessionV1 {
            principal: [20; 16],
            authority_context: [40; 16],
        })
    }
}

struct Allow;

impl ExecutionAuthorizer for Allow {
    fn authorize(
        &mut self,
        _: &AuthenticatedSessionV1,
        request: &dp::ExecuteRequestV1,
        _: u64,
    ) -> Result<AuthorityGrantV1, AuthorityRefusalV1> {
        Ok(AuthorityGrantV1 {
            decision_id: request.execution_id.into_bytes(),
            expires_at: 1_000,
            tools: request.tools.clone(),
        })
    }
}

struct Current;

impl AttemptValidator for Current {
    fn validate(
        &mut self,
        _: &AuthenticatedSessionV1,
        _: &dp::ExecuteRequestV1,
    ) -> Result<(), AttemptRefusalV1> {
        Ok(())
    }
}

struct ExactContext;

impl AgentContextAssembler for ExactContext {
    fn assemble_agent_context(
        &mut self,
        _: &AuthenticatedSessionV1,
        request: &AgentOpenRequestV1,
        _: &AgentProfileSnapshotV1,
    ) -> Result<Vec<dp::ContextEvidenceV1>, daimon_host::ContextRefusalV1> {
        context(&request.execution)
    }

    fn assemble_agent_turn_context(
        &mut self,
        _: &AuthenticatedSessionV1,
        request: &AgentTurnRequestV1,
        _: &AgentProfileSnapshotV1,
    ) -> Result<Vec<dp::ContextEvidenceV1>, daimon_host::ContextRefusalV1> {
        context(&request.execution)
    }
}

fn context(
    request: &dp::ExecuteRequestV1,
) -> Result<Vec<dp::ContextEvidenceV1>, daimon_host::ContextRefusalV1> {
    request
        .context
        .iter()
        .map(|reference| {
            Ok(dp::ContextEvidenceV1 {
                reference: *reference,
                kind: dp::ContextKindV1::ChronosWork,
                bytes: b"retained orthogonal theorem context".to_vec(),
                source_digest: dp::ContentDigestV1::new([90; 32]),
                grounding: None,
            })
        })
        .collect()
}

struct RegistryStore(DeliberationRegistryV1);

impl DeliberationStore for RegistryStore {
    type Error = Infallible;

    fn revision(&self) -> RegistryRevisionV1 {
        self.0.revision()
    }

    fn needs_reopen(&self) -> bool {
        false
    }

    fn deliberation(&self, id: DeliberationIdV1) -> Option<DeliberationCheckpointV1> {
        self.0.deliberation(id)
    }

    fn apply(
        &mut self,
        expected_revision: RegistryRevisionV1,
        command: RegistryCommandV1,
    ) -> Result<RegistryReceiptV1, DeliberationApplyErrorV1<Self::Error>> {
        self.0
            .apply(expected_revision, command)
            .map_err(DeliberationApplyErrorV1::Domain)
    }
}

#[test]
fn acp_and_claude_terminal_outputs_become_one_orthogonal_moira_ensemble() {
    let fixture = Fixture::new();
    let acp_binding = binding(0);
    let claude_binding = binding(10);
    let mut participation_service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(
            &fixture.participation_database,
            [1; 16],
            participation_limits(),
        )
        .unwrap(),
    );
    let mut storage_revision = None;
    for binding in [acp_binding, claude_binding] {
        apply_tracked(
            &mut participation_service,
            &mut storage_revision,
            p::MutationV1::Admit(binding),
        );
        apply_tracked(
            &mut participation_service,
            &mut storage_revision,
            p::MutationV1::Prepare {
                participation: binding.participation,
                expected_revision: 0,
                operation: p::OperationV1::Open {
                    key: p::IdempotencyKeyV1([binding.participation.0[0] + 60; 16]),
                    execution: p::ExecutionIdV1([binding.participation.0[0] + 61; 16]),
                },
            },
        );
    }

    let mut daimon_daemon = AgentDaemon::bind_new(
        &fixture.daimon_socket,
        daimon_service(&fixture),
        DaimonAuthentication,
        DaimonLimitsV1::default(),
        AgentWireLimitsV1::default(),
        Duration::from_secs(2),
        8,
    )
    .unwrap();
    let open_daemon = AgentParticipationDaemon::bind_new(
        &fixture.participation_socket,
        participation_service,
        ParticipationAuthentication,
        MoiraLimitsV1::default(),
        Duration::from_secs(2),
        8,
    )
    .unwrap();
    let open_server = thread::spawn(move || {
        let mut daemon = open_daemon;
        for _ in 0..4 {
            daemon.serve_next().unwrap();
        }
        daemon.into_parts().0
    });
    for binding in [acp_binding, claude_binding] {
        thread::scope(|scope| {
            let worker = scope.spawn(|| run_binary(&fixture, binding));
            daimon_daemon.serve_next().unwrap();
            assert_eq!(worker.join().unwrap()["outcome"], "observed");
        });
    }
    let mut participation_service = open_server.join().unwrap();

    let mut prompts = AgentPromptSqliteStore::open(
        &fixture.prompt_database,
        [60; 16],
        AgentPromptStoreLimitsV1 {
            max_entries: 4,
            max_prompt_bytes: 4_096,
            max_database_pages: 64,
        },
    )
    .unwrap();
    for binding in [acp_binding, claude_binding] {
        assert!(matches!(
            participation_service
                .execute_prompt_authorized(
                    AccessV1 {
                        read: true,
                        mutate: true,
                    },
                    p::PromptSubmissionV1 {
                        participation: binding.participation,
                        prompt: b"produce the closed retained report".to_vec(),
                    },
                    &mut prompts,
                )
                .unwrap(),
            p::ReplyV1::Accepted(_)
        ));
    }
    let prompt_daemon = AgentParticipationDaemon::bind_new_with_prompt_store(
        &fixture.participation_socket,
        participation_service,
        ParticipationAuthentication,
        prompts,
        MoiraLimitsV1::default(),
        Duration::from_secs(2),
        8,
    )
    .unwrap();
    let prompt_server = thread::spawn(move || {
        let mut daemon = prompt_daemon;
        for _ in 0..4 {
            daemon.serve_next().unwrap();
        }
        let (mut service, _, _) = daemon.into_prompt_parts();
        [
            participation(&mut service, acp_binding.participation),
            participation(&mut service, claude_binding.participation),
        ]
    });
    for binding in [acp_binding, claude_binding] {
        thread::scope(|scope| {
            let worker = scope.spawn(|| run_binary(&fixture, binding));
            daimon_daemon.serve_next().unwrap();
            assert_eq!(worker.join().unwrap()["outcome"], "observed");
        });
    }
    let participations = prompt_server.join().unwrap();
    assert!(
        participations
            .iter()
            .all(|value| value.phase == p::PhaseV1::Terminal)
    );

    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan())),
        )
        .unwrap();
    let mut registry = RegistryStore(registry);
    let mut outputs =
        AgentOutputSqliteStore::open(&fixture.output_database, [50; 16], output_limits()).unwrap();
    let acp = admit_terminal_output(
        &mut registry,
        &mut outputs,
        &participations[0],
        MemberResponseLimitsV1::default(),
    )
    .unwrap();
    let claude = admit_terminal_output(
        &mut registry,
        &mut outputs,
        &participations[1],
        MemberResponseLimitsV1::default(),
    )
    .unwrap();

    assert_eq!(
        acp.response.claims[0].statement,
        "ACP historian retained the chronology"
    );
    assert_eq!(
        claude.response.claims[0].statement,
        "Claude rigor reviewer retained the constraint"
    );
    assert_ne!(acp.report.member, claude.report.member);
    assert_ne!(acp.report.output_digest, claude.report.output_digest);
    let acp_provider = participations[0].terminal.unwrap().provider_evidence_digest;
    let claude_provider = participations[1].terminal.unwrap().provider_evidence_digest;
    assert_ne!(acp_provider, claude_provider);
    assert!(acp.report.evidence_digests.contains(&acp_provider));
    assert!(claude.report.evidence_digests.contains(&claude_provider));
    let retained = registry.0.deliberation(DeliberationIdV1([4; 16])).unwrap();
    assert!(retained.plan.orthogonality.all_orthogonal);
    assert_eq!(retained.reports.len(), 2);
    assert_eq!(registry.revision(), RegistryRevisionV1(3));
}

fn daimon_service(
    fixture: &Fixture,
) -> AgentService<
    Allow,
    Current,
    StaticAgentProfileRouter,
    ExactContext,
    AgentProviderRouter,
    VolatileAgentStore,
> {
    let acp = profile(0);
    let claude = profile(10);
    let daimon_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../daimon")
        .canonicalize()
        .unwrap();
    let acp_fixture = daimon_root.join("adapters/daimon-acp/tests/support/fake_acp_agent.sh");
    let claude_fixture =
        daimon_root.join("adapters/daimon-claude/tests/support/fake_claude_agent.sh");
    let claude_workspace = daimon_root.join("adapters/daimon-claude");
    let acp_provider = AcpProvider::try_new(
        AcpLimitsV1 {
            slice_wait: Duration::from_millis(20),
            ..AcpLimitsV1::default()
        },
        vec![AcpLaunchProfileV1 {
            snapshot: acp.clone(),
            program: PathBuf::from("/bin/sh"),
            arguments: vec![acp_fixture.into_os_string(), OsString::from("report_acp")],
            inherited_environment_names: Vec::new(),
            agent_authentication_method: None,
            isolation: daimon_acp::ProcessIsolationV1::Direct,
        }],
        vec![AcpWorkspaceV1 {
            identity: [21; 16],
            path: fixture.root.clone(),
        }],
    )
    .unwrap();
    let claude_provider = ClaudeProvider::try_new(
        ClaudeLimitsV1 {
            slice_wait: Duration::from_millis(20),
            ..ClaudeLimitsV1::default()
        },
        vec![ClaudeLaunchProfileV1 {
            snapshot: claude.clone(),
            program: claude_fixture,
            arguments: vec![
                OsString::from("report_claude"),
                claude_workspace.clone().into_os_string(),
            ],
            inherited_environment_names: Vec::new(),
            isolation: daimon_acp::ProcessIsolationV1::Direct,
        }],
        vec![ClaudeWorkspaceV1 {
            identity: [21; 16],
            path: claude_workspace,
        }],
    )
    .unwrap();
    let provider = AgentProviderRouter::try_new(
        2,
        vec![
            AgentProviderRouteV1 {
                profile_documents: vec![acp.document_digest],
                provider: Box::new(acp_provider),
            },
            AgentProviderRouteV1 {
                profile_documents: vec![claude.document_digest],
                provider: Box::new(claude_provider),
            },
        ],
    )
    .unwrap();
    AgentService::new(
        DaimonCoreLimits::default(),
        AgentServiceLimitsV1::default(),
        Allow,
        Current,
        StaticAgentProfileRouter::try_new(2, vec![acp, claude]).unwrap(),
        ExactContext,
        provider,
    )
    .unwrap()
}

fn profile(offset: u8) -> AgentProfileSnapshotV1 {
    AgentProfileSnapshotV1 {
        provider: dp::ProviderProfileIdV1::new([offset + 1; 16]),
        harness_profile: dp::HarnessProfileIdV1::new([offset + 7; 16]),
        wire: if offset == 0 {
            AgentWireProfileV1::AgentClientProtocol
        } else {
            AgentWireProfileV1::ClaudeAgentStreamJson
        },
        command_digest: dp::ContentDigestV1::new([offset + 27; 32]),
        policy_digest: dp::ContentDigestV1::new([offset + 28; 32]),
        document_digest: dp::ContentDigestV1::new([offset + 30; 32]),
    }
}

fn apply_tracked(
    service: &mut AgentParticipationService<AgentParticipationSqliteStore>,
    storage_revision: &mut Option<u64>,
    mutation: p::MutationV1,
) {
    let p::ReplyV1::Accepted(value) = service
        .execute(p::CommandV1::Apply {
            expected_storage_revision: *storage_revision,
            mutation: Box::new(mutation),
        })
        .unwrap()
    else {
        panic!("participation mutation expected")
    };
    let p::OutcomeV1::Applied {
        storage_revision: next,
        ..
    } = *value
    else {
        panic!("applied participation expected")
    };
    *storage_revision = Some(next);
}

fn participation(
    service: &mut AgentParticipationService<AgentParticipationSqliteStore>,
    id: p::ParticipationIdV1,
) -> p::ParticipationViewV1 {
    let p::ReplyV1::Accepted(value) = service
        .execute(p::CommandV1::Participation { participation: id })
        .unwrap()
    else {
        panic!("participation expected")
    };
    let p::OutcomeV1::Participation(Some(value)) = *value else {
        panic!("retained participation expected")
    };
    value
}

fn run_binary(fixture: &Fixture, binding: p::BindingV1) -> serde_json::Value {
    let path = fixture
        .root
        .join(format!("agent-{}.json", binding.participation.0[0]));
    let config = config(fixture, binding);
    fs::write(&path, serde_json::to_vec(&config_json(&config)).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_moira-agent-local"))
        .arg(&path)
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn config(fixture: &Fixture, binding: p::BindingV1) -> ConfigV1 {
    ConfigV1 {
        participation_socket: fixture.participation_socket.clone(),
        daimon_agent_socket: fixture.daimon_socket.clone(),
        participation_id: hex(binding.participation.0),
        output: OutputConfigV1 {
            path: fixture.output_database.clone(),
            owner: hex([50; 16]),
            max_entries: 4,
            max_output_bytes: 4_096,
            max_page_assistant_bytes: 4_096,
            max_database_pages: 64,
        },
        policy: PolicyConfigV1 {
            principal: hex([20; 16]),
            workspace: hex([21; 16]),
            context: vec![hex([40; 16])],
            tools: Vec::new(),
            execution_budget: ExecutionBudgetConfigV1 {
                max_elapsed_millis: 1_000,
                max_input_tokens: 100,
                max_output_tokens: 100,
                max_tool_calls: 1,
                max_context_bytes: 4_096,
                max_output_bytes: 4_096,
            },
            session_budget: SessionBudgetConfigV1 {
                max_elapsed_millis: 10_000,
                max_turns: 4,
                max_events: 32,
                max_event_bytes: 4_096,
                max_prompt_bytes: 4_096,
                max_permission_requests: 2,
            },
        },
        transport: TransportConfigV1 {
            participation_max_frame_bytes: 65_536,
            participation_max_requests: 8,
            daimon_handshake_max_frame_bytes: 1_048_576,
            agent_max_frame_bytes: 1_048_576,
            timeout_ms: 2_000,
        },
        now: 7,
        max_event_observations: 32,
    }
}

fn config_json(config: &ConfigV1) -> serde_json::Value {
    serde_json::json!({
        "participation_socket": config.participation_socket,
        "daimon_agent_socket": config.daimon_agent_socket,
        "participation_id": config.participation_id,
        "output": {
            "path": config.output.path,
            "owner": config.output.owner,
            "max_entries": config.output.max_entries,
            "max_output_bytes": config.output.max_output_bytes,
            "max_page_assistant_bytes": config.output.max_page_assistant_bytes,
            "max_database_pages": config.output.max_database_pages
        },
        "policy": {
            "principal": config.policy.principal,
            "workspace": config.policy.workspace,
            "context": config.policy.context,
            "tools": config.policy.tools,
            "execution_budget": {
                "max_elapsed_millis": config.policy.execution_budget.max_elapsed_millis,
                "max_input_tokens": config.policy.execution_budget.max_input_tokens,
                "max_output_tokens": config.policy.execution_budget.max_output_tokens,
                "max_tool_calls": config.policy.execution_budget.max_tool_calls,
                "max_context_bytes": config.policy.execution_budget.max_context_bytes,
                "max_output_bytes": config.policy.execution_budget.max_output_bytes
            },
            "session_budget": {
                "max_elapsed_millis": config.policy.session_budget.max_elapsed_millis,
                "max_turns": config.policy.session_budget.max_turns,
                "max_events": config.policy.session_budget.max_events,
                "max_event_bytes": config.policy.session_budget.max_event_bytes,
                "max_prompt_bytes": config.policy.session_budget.max_prompt_bytes,
                "max_permission_requests": config.policy.session_budget.max_permission_requests
            }
        },
        "transport": {
            "participation_max_frame_bytes": config.transport.participation_max_frame_bytes,
            "participation_max_requests": config.transport.participation_max_requests,
            "daimon_handshake_max_frame_bytes": config.transport.daimon_handshake_max_frame_bytes,
            "agent_max_frame_bytes": config.transport.agent_max_frame_bytes,
            "timeout_ms": config.transport.timeout_ms
        },
        "now": config.now,
        "max_event_observations": config.max_event_observations
    })
}

fn binding(offset: u8) -> p::BindingV1 {
    p::BindingV1 {
        participation: p::ParticipationIdV1([offset + 2; 16]),
        workflow: WorkflowIdV1([3; 16]),
        deliberation: d::DeliberationIdV1([4; 16]),
        member: d::EnsembleMemberIdV1([offset + 5; 16]),
        session: p::AgentSessionIdV1([offset + 6; 16]),
        profile: p::ProfileIdV1([offset + 7; 16]),
        attempt: p::AttemptV1 {
            task_key: [offset + 8; 16],
            incarnation: 1,
            attempt: 1,
        },
        method: d::MethodRefV1 {
            machine_digest: [9; 32],
            state_digest: [10; 32],
        },
        grounding: d::GroundingRefV1 {
            authority_digest: [offset + 11; 32],
            surface_digest: [offset + 12; 32],
            depth: d::SurfaceDepthV1::Activated,
        },
        persona: if offset == 0 {
            d::PersonaLensV1::Historian
        } else {
            d::PersonaLensV1::RigorReviewer
        },
    }
}

fn plan() -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([4; 16]),
        question_digest: [1; 32],
        method: MethodRefV1 {
            machine_digest: [9; 32],
            state_digest: [10; 32],
        },
        claims: vec![ClaimKindV1::Empirical, ClaimKindV1::Normative],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([2; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([3; 16]),
            scores: vec![1],
        }],
        admission: moira_core::deliberation::EnsembleAdmissionV1::affirmed(
            [72; 32], [73; 32], [74; 32], [75; 32], [76; 16],
        ),
        members: vec![member(0), member(10)],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [70; 32],
            result_digest: [71; 32],
            all_orthogonal: true,
        },
        fuel: 2,
    }
}

fn member(offset: u8) -> EnsembleMemberV1 {
    EnsembleMemberV1 {
        id: EnsembleMemberIdV1([offset + 5; 16]),
        stakeholder: moira_core::deliberation::StakeholderRefV1::technical_sme(
            [offset + 5; 16],
            [offset + 80; 32],
        ),
        lens: if offset == 0 {
            PersonaLensV1::Historian
        } else {
            PersonaLensV1::RigorReviewer
        },
        concern_root_digest: [offset + 80; 32],
        grounding: GroundingRefV1 {
            authority_digest: [offset + 11; 32],
            surface_digest: [offset + 12; 32],
            depth: SurfaceDepthV1::Activated,
        },
    }
}

fn participation_limits() -> AgentParticipationStoreLimitsV1 {
    AgentParticipationStoreLimitsV1 {
        codec: CodecLimitsV1 {
            max_bytes: 131_072,
            domain: ParticipationDomainLimits {
                max_participations: 4,
            },
        },
        max_database_pages: 128,
    }
}

fn output_limits() -> AgentOutputStoreLimitsV1 {
    AgentOutputStoreLimitsV1 {
        max_entries: 4,
        max_output_bytes: 4_096,
        max_page_assistant_bytes: 4_096,
        max_database_pages: 64,
    }
}

fn hex(bytes: [u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Fixture {
    root: PathBuf,
    participation_database: PathBuf,
    output_database: PathBuf,
    prompt_database: PathBuf,
    participation_socket: PathBuf,
    daimon_socket: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "moira-mixed-agent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self {
            participation_database: root.join("participation.db"),
            output_database: root.join("output.db"),
            prompt_database: root.join("prompt.db"),
            participation_socket: root.join("participation.sock"),
            daimon_socket: root.join("daimon.sock"),
            root,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for path in [
            &self.participation_database,
            &self.output_database,
            &self.prompt_database,
        ] {
            for suffix in ["", "-journal", ".owner.lock"] {
                let _ = fs::remove_file(format!("{}{suffix}", path.display()));
            }
        }
        let _ = fs::remove_file(&self.participation_socket);
        let _ = fs::remove_file(&self.daimon_socket);
        let _ = fs::remove_dir(&self.root);
    }
}
