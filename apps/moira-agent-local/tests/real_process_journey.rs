#![cfg(unix)]

use daimon_host::AuthenticatedSessionV1;
use daimon_local_daemon::{
    AgentServicePort,
    unix::{AgentDaemon, AgentSessionPort, Authenticator as DaimonAuthenticator},
};
use daimon_protocol::{
    AgentTurnIdV1, ContentDigestV1, ExecutionDispositionV1, SessionHandleV1,
    agent::{
        AgentCapabilitiesV1, AgentCommandV1, AgentEventPageV1, AgentEventRecordV1, AgentEventV1,
        AgentOutcomeV1, AgentReplyV1, AgentSessionPhaseV1, AgentSessionViewV1,
        AgentTurnStopReasonV1,
    },
};
use daimon_wire::{LimitsV1 as DaimonLimitsV1, agent::AgentWireLimitsV1};
use moira_agent_local::{
    ConfigV1, ExecutionBudgetConfigV1, OutputConfigV1, PolicyConfigV1, SessionBudgetConfigV1,
    SummaryV1, TransportConfigV1, execute, validate,
};
use moira_core::agent_participation::{LimitsV1 as DomainLimits, persistence::CodecLimitsV1};
use moira_host::{AgentOutputStoreV1, AgentPromptStoreV1};
use moira_local_daemon::unix::{AgentParticipationAuthenticator, AgentParticipationDaemon};
use moira_protocol::{WorkflowIdV1, agent_participation as p, deliberation as d};
use moira_service::{AccessV1, agent_participation::AgentParticipationService};
use moira_sqlite::agent_output::AgentOutputSqliteStore;
use moira_sqlite::agent_participation::{
    AgentParticipationSqliteStore, AgentParticipationStoreLimitsV1,
};
use moira_sqlite::agent_prompt::{AgentPromptSqliteStore, AgentPromptStoreLimitsV1};
use moira_wire::LimitsV1 as MoiraLimitsV1;
use std::{
    convert::Infallible,
    fs,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
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
}

struct WorkerParticipationAuthentication;

impl AgentParticipationAuthenticator for WorkerParticipationAuthentication {
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

struct AgentPort {
    commands: Arc<AtomicUsize>,
}

impl AgentServicePort for AgentPort {
    type Error = Infallible;

    fn execute_agent(
        &mut self,
        service_session: SessionHandleV1,
        command: AgentCommandV1,
    ) -> Result<AgentReplyV1, Self::Error> {
        assert_eq!(service_session, SessionHandleV1(17));
        let ordinal = self.commands.fetch_add(1, Ordering::Relaxed);
        Ok(match (ordinal, command) {
            (0, AgentCommandV1::Open { request, now }) => {
                assert_eq!(request.session.into_bytes(), [6; 16]);
                assert_eq!(request.execution.execution_id.into_bytes(), [14; 16]);
                assert_eq!(request.execution.idempotency_key.into_bytes(), [13; 16]);
                assert_eq!(request.execution.principal.into_bytes(), [20; 16]);
                assert_eq!(request.execution.profile.into_bytes(), [7; 16]);
                assert_eq!(request.execution.context[0].into_bytes(), [22; 16]);
                assert_eq!(request.execution.tools[0].into_bytes(), [23; 16]);
                assert_eq!(request.workspace, [21; 16]);
                assert_eq!(now, 7);
                AgentReplyV1::Accepted(AgentOutcomeV1::Session(AgentSessionViewV1 {
                    session: request.session,
                    revision: 1,
                    phase: AgentSessionPhaseV1::Ready,
                    profile_document_digest: ContentDigestV1::new([30; 32]),
                    active_turn: None,
                    last_event_cursor: None,
                    last_context: None,
                    capabilities: AgentCapabilitiesV1::default(),
                }))
            }
            (
                1,
                AgentCommandV1::Events {
                    session,
                    after,
                    limit,
                },
            ) => {
                assert_eq!(session.into_bytes(), [6; 16]);
                assert_eq!(after, None);
                assert_eq!(limit, 32);
                AgentReplyV1::Accepted(AgentOutcomeV1::Events(AgentEventPageV1 {
                    session,
                    entries: Vec::new(),
                    next_after: None,
                }))
            }
            other => panic!("unexpected Daimon agent command {other:?}"),
        })
    }
}

impl AgentSessionPort for AgentPort {
    fn open_agent_service_session(
        &mut self,
        facts: AuthenticatedSessionV1,
    ) -> Result<SessionHandleV1, daimon_protocol::agent::AgentRefusalV1> {
        assert_eq!(facts.principal, [20; 16]);
        Ok(SessionHandleV1(17))
    }

    fn close_agent_service_session(&mut self, session: SessionHandleV1) -> bool {
        assert_eq!(session, SessionHandleV1(17));
        true
    }
}

struct PromptAgentPort {
    commands: Arc<AtomicUsize>,
    turn: Option<AgentTurnIdV1>,
}

struct ReattachAgentPort {
    commands: Arc<AtomicUsize>,
}

impl AgentServicePort for ReattachAgentPort {
    type Error = Infallible;

    fn execute_agent(
        &mut self,
        service_session: SessionHandleV1,
        command: AgentCommandV1,
    ) -> Result<AgentReplyV1, Self::Error> {
        assert_eq!(service_session, SessionHandleV1(19));
        let ordinal = self.commands.fetch_add(1, Ordering::Relaxed);
        let capabilities = AgentCapabilitiesV1 {
            load_session: true,
            resume_session: true,
            ..AgentCapabilitiesV1::default()
        };
        Ok(match (ordinal, command) {
            (0, AgentCommandV1::Inspect { session }) => {
                assert_eq!(session.into_bytes(), [6; 16]);
                AgentReplyV1::Accepted(AgentOutcomeV1::Session(AgentSessionViewV1 {
                    session,
                    revision: 1,
                    phase: AgentSessionPhaseV1::ReattachRequired,
                    profile_document_digest: ContentDigestV1::new([30; 32]),
                    active_turn: None,
                    last_event_cursor: None,
                    last_context: None,
                    capabilities,
                }))
            }
            (
                1,
                AgentCommandV1::Reattach {
                    session,
                    expected_revision,
                    execution,
                    now,
                },
            ) => {
                assert_eq!(session.into_bytes(), [6; 16]);
                assert_eq!(expected_revision, 1);
                assert_ne!(execution.execution_id.into_bytes(), [0; 16]);
                assert_ne!(execution.idempotency_key.into_bytes(), [0; 16]);
                assert_eq!(execution.task_attempt.task_key, [8; 16]);
                assert_eq!(execution.task_attempt.task_incarnation, 1);
                assert_eq!(execution.task_attempt.attempt_number, 1);
                assert_eq!(execution.principal.into_bytes(), [20; 16]);
                assert_eq!(execution.profile.into_bytes(), [7; 16]);
                assert_eq!(now, 7);
                AgentReplyV1::Accepted(AgentOutcomeV1::Session(AgentSessionViewV1 {
                    session,
                    revision: 2,
                    phase: AgentSessionPhaseV1::Ready,
                    profile_document_digest: ContentDigestV1::new([30; 32]),
                    active_turn: None,
                    last_event_cursor: None,
                    last_context: None,
                    capabilities,
                }))
            }
            (
                2,
                AgentCommandV1::Events {
                    session,
                    after,
                    limit,
                },
            ) => {
                assert_eq!(session.into_bytes(), [6; 16]);
                assert_eq!(after, None);
                assert_eq!(limit, 32);
                AgentReplyV1::Accepted(AgentOutcomeV1::Events(AgentEventPageV1 {
                    session,
                    entries: Vec::new(),
                    next_after: None,
                }))
            }
            other => panic!("unexpected Daimon reattach command {other:?}"),
        })
    }
}

impl AgentSessionPort for ReattachAgentPort {
    fn open_agent_service_session(
        &mut self,
        facts: AuthenticatedSessionV1,
    ) -> Result<SessionHandleV1, daimon_protocol::agent::AgentRefusalV1> {
        assert_eq!(facts.principal, [20; 16]);
        Ok(SessionHandleV1(19))
    }

    fn close_agent_service_session(&mut self, session: SessionHandleV1) -> bool {
        assert_eq!(session, SessionHandleV1(19));
        true
    }
}

impl AgentServicePort for PromptAgentPort {
    type Error = Infallible;

    fn execute_agent(
        &mut self,
        service_session: SessionHandleV1,
        command: AgentCommandV1,
    ) -> Result<AgentReplyV1, Self::Error> {
        assert_eq!(service_session, SessionHandleV1(18));
        let ordinal = self.commands.fetch_add(1, Ordering::Relaxed);
        Ok(match (ordinal, command) {
            (0, AgentCommandV1::Prompt { request, now }) => {
                assert_eq!(request.session.into_bytes(), [6; 16]);
                assert_eq!(request.expected_revision, 1);
                assert_eq!(request.prompt, b"answer from the retained theorem");
                assert_eq!(
                    request.prompt_digest.into_bytes(),
                    p::agent_prompt_digest(b"answer from the retained theorem").unwrap()
                );
                assert_eq!(request.execution.principal.into_bytes(), [20; 16]);
                assert_eq!(request.execution.profile.into_bytes(), [7; 16]);
                assert_eq!(request.execution.context[0].into_bytes(), [22; 16]);
                assert_eq!(request.execution.tools[0].into_bytes(), [23; 16]);
                assert_eq!(now, 7);
                self.turn = Some(request.turn);
                AgentReplyV1::Accepted(AgentOutcomeV1::Session(AgentSessionViewV1 {
                    session: request.session,
                    revision: 2,
                    phase: AgentSessionPhaseV1::Ready,
                    profile_document_digest: ContentDigestV1::new([31; 32]),
                    active_turn: None,
                    last_event_cursor: Some(1),
                    last_context: None,
                    capabilities: AgentCapabilitiesV1::default(),
                }))
            }
            (
                1,
                AgentCommandV1::Events {
                    session,
                    after,
                    limit,
                },
            ) => {
                assert_eq!(session.into_bytes(), [6; 16]);
                assert_eq!(after, None);
                assert_eq!(limit, 32);
                let turn = self.turn.take().expect("prompt turn retained");
                AgentReplyV1::Accepted(AgentOutcomeV1::Events(AgentEventPageV1 {
                    session,
                    entries: vec![
                        AgentEventRecordV1 {
                            cursor: 0,
                            event: AgentEventV1::AssistantMessage {
                                bytes: b"grounded answer".to_vec(),
                            },
                        },
                        AgentEventRecordV1 {
                            cursor: 1,
                            event: AgentEventV1::TurnFinished {
                                turn,
                                disposition: ExecutionDispositionV1::Completed,
                                stop_reason: AgentTurnStopReasonV1::EndTurn,
                                output_digest: None,
                            },
                        },
                    ],
                    next_after: None,
                }))
            }
            other => panic!("unexpected Daimon prompt command {other:?}"),
        })
    }
}

impl AgentSessionPort for PromptAgentPort {
    fn open_agent_service_session(
        &mut self,
        facts: AuthenticatedSessionV1,
    ) -> Result<SessionHandleV1, daimon_protocol::agent::AgentRefusalV1> {
        assert_eq!(facts.principal, [20; 16]);
        Ok(SessionHandleV1(18))
    }

    fn close_agent_service_session(&mut self, session: SessionHandleV1) -> bool {
        assert_eq!(session, SessionHandleV1(18));
        true
    }
}

#[test]
fn executable_composition_crosses_both_private_sockets_and_commits_observation() {
    let fixture = Fixture::new();
    let binding = binding();
    prime_open(&fixture.participation_database, binding);

    let participation_service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(
            &fixture.participation_database,
            [1; 16],
            participation_limits(),
        )
        .unwrap(),
    );
    let participation_daemon = AgentParticipationDaemon::bind_new(
        &fixture.participation_socket,
        participation_service,
        ParticipationAuthentication,
        MoiraLimitsV1::default(),
        Duration::from_secs(2),
        8,
    )
    .unwrap();
    let participation_server = thread::spawn(move || {
        let mut daemon = participation_daemon;
        daemon.serve_next().unwrap();
        daemon.serve_next().unwrap();
        let (mut service, _) = daemon.into_parts();
        let p::ReplyV1::Accepted(value) = service
            .execute(p::CommandV1::Participation {
                participation: binding.participation,
            })
            .unwrap()
        else {
            panic!("participation reply expected")
        };
        let p::OutcomeV1::Participation(Some(view)) = *value else {
            panic!("participation view expected")
        };
        view
    });

    let commands = Arc::new(AtomicUsize::new(0));
    let daimon_daemon = AgentDaemon::bind_new(
        &fixture.daimon_socket,
        AgentPort {
            commands: Arc::clone(&commands),
        },
        DaimonAuthentication,
        DaimonLimitsV1::default(),
        AgentWireLimitsV1::default(),
        Duration::from_secs(2),
        2,
    )
    .unwrap();
    let daimon_server = thread::spawn(move || {
        let mut daemon = daimon_daemon;
        daemon.serve_next().unwrap();
    });

    let output = run_binary(&fixture);
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({
            "outcome": "observed",
            "participation_revision": 3,
        })
    );
    assert_eq!(commands.load(Ordering::Relaxed), 2);

    let retained = participation_server.join().unwrap();
    daimon_server.join().unwrap();
    assert_eq!(retained.phase, p::PhaseV1::Ready);
    assert_eq!(retained.session_revision, Some(1));
    assert_eq!(retained.profile_document_digest, Some([30; 32]));
    assert_eq!(retained.pending, None);
    assert!(!fixture.participation_socket.exists());
    assert!(!fixture.daimon_socket.exists());
}

fn run_binary(fixture: &Fixture) -> std::process::Output {
    let path = fixture.root.join("agent.json");
    let value = serde_json::json!({
        "participation_socket": fixture.participation_socket,
        "daimon_agent_socket": fixture.daimon_socket,
        "participation_id": hex([2; 16]),
        "output": {
            "path": fixture.output_database,
            "owner": hex([50; 16]),
            "max_entries": 4,
            "max_output_bytes": 4096,
            "max_page_assistant_bytes": 1024,
            "max_database_pages": 64,
        },
        "policy": {
            "principal": hex([20; 16]),
            "workspace": hex([21; 16]),
            "context": [hex([22; 16])],
            "tools": [hex([23; 16])],
            "execution_budget": {
                "max_elapsed_millis": 1000,
                "max_input_tokens": 100,
                "max_output_tokens": 100,
                "max_tool_calls": 1,
                "max_context_bytes": 1024,
                "max_output_bytes": 1024,
            },
            "session_budget": {
                "max_elapsed_millis": 10000,
                "max_turns": 4,
                "max_events": 32,
                "max_event_bytes": 4096,
                "max_prompt_bytes": 4096,
                "max_permission_requests": 4,
            },
        },
        "transport": {
            "participation_max_frame_bytes": 65536,
            "participation_max_requests": 8,
            "daimon_handshake_max_frame_bytes": 1048576,
            "agent_max_frame_bytes": 1048576,
            "timeout_ms": 2000,
        },
        "now": 7,
        "max_event_observations": 32,
    });
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_moira-agent-local"))
        .arg(&path)
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();
    output
}

#[test]
fn prompt_worker_materializes_through_daemon_and_retains_exact_terminal_output() {
    let fixture = Fixture::new();
    let binding = binding();
    let (participation_service, prompt_store) = prime_prompt(&fixture, binding);
    let participation_daemon = AgentParticipationDaemon::bind_new_with_prompt_store(
        &fixture.participation_socket,
        participation_service,
        WorkerParticipationAuthentication,
        prompt_store,
        MoiraLimitsV1::default(),
        Duration::from_secs(2),
        8,
    )
    .unwrap();
    let participation_server = thread::spawn(move || {
        let mut daemon = participation_daemon;
        daemon.serve_next().unwrap();
        daemon.serve_next().unwrap();
        let (mut service, _, mut prompts) = daemon.into_prompt_parts();
        let digest = p::agent_prompt_digest(b"answer from the retained theorem").unwrap();
        assert_eq!(
            prompts.get(digest).unwrap(),
            Some(b"answer from the retained theorem".to_vec())
        );
        let p::ReplyV1::Accepted(value) = service
            .execute(p::CommandV1::Participation {
                participation: binding.participation,
            })
            .unwrap()
        else {
            panic!("participation reply expected")
        };
        let p::OutcomeV1::Participation(Some(view)) = *value else {
            panic!("participation view expected")
        };
        view
    });

    let commands = Arc::new(AtomicUsize::new(0));
    let daimon_daemon = AgentDaemon::bind_new(
        &fixture.daimon_socket,
        PromptAgentPort {
            commands: Arc::clone(&commands),
            turn: None,
        },
        DaimonAuthentication,
        DaimonLimitsV1::default(),
        AgentWireLimitsV1::default(),
        Duration::from_secs(2),
        2,
    )
    .unwrap();
    let daimon_server = thread::spawn(move || {
        let mut daemon = daimon_daemon;
        daemon.serve_next().unwrap();
    });

    let summary = execute(validate(config(&fixture)).unwrap()).unwrap();
    assert_eq!(summary, SummaryV1::Observed { revision: 5 });
    assert_eq!(commands.load(Ordering::Relaxed), 2);

    let retained = participation_server.join().unwrap();
    daimon_server.join().unwrap();
    assert_eq!(retained.phase, p::PhaseV1::Terminal);
    assert_eq!(retained.session_revision, Some(2));
    assert_eq!(retained.last_event_cursor, Some(1));
    assert_eq!(retained.profile_document_digest, Some([31; 32]));
    assert_eq!(retained.pending, None);
    let terminal = retained.terminal.expect("terminal evidence retained");
    let mut outputs =
        AgentOutputSqliteStore::open(&fixture.output_database, [50; 16], output_limits()).unwrap();
    let (record, bytes) = outputs
        .get(binding.participation.0, terminal.turn.0)
        .unwrap()
        .expect("assistant output retained");
    assert_eq!(record.last_cursor, 1);
    assert_eq!(bytes, b"grounded answer");
    assert_eq!(record.output_digest, terminal.output_digest);
}

#[test]
fn executable_discovers_confirms_and_dispatches_governed_reattach() {
    let fixture = Fixture::new();
    let binding = binding();
    prime_ready_resume(&fixture.participation_database, binding);

    let participation_service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(
            &fixture.participation_database,
            [1; 16],
            participation_limits(),
        )
        .unwrap(),
    );
    let participation_daemon = AgentParticipationDaemon::bind_new(
        &fixture.participation_socket,
        participation_service,
        ParticipationAuthentication,
        MoiraLimitsV1::default(),
        Duration::from_secs(2),
        8,
    )
    .unwrap();
    let first_participation_server = thread::spawn(move || {
        let mut daemon = participation_daemon;
        daemon.serve_next().unwrap();
        daemon.serve_next().unwrap();
        daemon.into_parts().0
    });

    let commands = Arc::new(AtomicUsize::new(0));
    let daimon_daemon = AgentDaemon::bind_new(
        &fixture.daimon_socket,
        ReattachAgentPort {
            commands: Arc::clone(&commands),
        },
        DaimonAuthentication,
        DaimonLimitsV1::default(),
        AgentWireLimitsV1::default(),
        Duration::from_secs(2),
        2,
    )
    .unwrap();
    let daimon_server = thread::spawn(move || {
        let mut daemon = daimon_daemon;
        daemon.serve_next().unwrap();
        daemon.serve_next().unwrap();
    });

    let first = run_binary(&fixture);
    assert!(first.status.success(), "{:?}", first);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&first.stdout).unwrap(),
        serde_json::json!({
            "outcome": "observed",
            "participation_revision": 3,
        })
    );
    let mut service = first_participation_server.join().unwrap();
    let challenge = match service
        .execute(p::CommandV1::ReattachConfirmation {
            participation: binding.participation,
        })
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::ReattachConfirmation(Some(value)) => value,
            other => panic!("reattach confirmation expected, received {other:?}"),
        },
        other => panic!("reattach confirmation accepted reply expected, received {other:?}"),
    };
    let confirmed = p::ConfirmedReattachV1 {
        participation: binding.participation,
        confirmation_digest: challenge.confirmation_digest,
    };
    let prepared = match service
        .execute(p::CommandV1::ConfirmReattach(confirmed))
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::Applied {
                participation,
                replayed: false,
                ..
            } => participation,
            other => panic!("prepared reattach expected, received {other:?}"),
        },
        other => panic!("reattach confirmation accepted reply expected, received {other:?}"),
    };
    let Some(p::OperationV1::Reattach {
        key,
        execution,
        attempt,
    }) = prepared.pending
    else {
        panic!("derived reattach operation expected")
    };
    assert_ne!(key.0, [0; 16]);
    assert_ne!(execution.0, [0; 16]);
    assert_eq!(attempt, binding.attempt);

    let participation_daemon = AgentParticipationDaemon::bind_new(
        &fixture.participation_socket,
        service,
        ParticipationAuthentication,
        MoiraLimitsV1::default(),
        Duration::from_secs(2),
        8,
    )
    .unwrap();
    let second_participation_server = thread::spawn(move || {
        let mut daemon = participation_daemon;
        daemon.serve_next().unwrap();
        daemon.serve_next().unwrap();
        let (mut service, _) = daemon.into_parts();
        let p::ReplyV1::Accepted(value) = service
            .execute(p::CommandV1::Participation {
                participation: binding.participation,
            })
            .unwrap()
        else {
            panic!("participation reply expected")
        };
        let p::OutcomeV1::Participation(Some(view)) = *value else {
            panic!("participation view expected")
        };
        view
    });
    let second = run_binary(&fixture);
    assert!(second.status.success(), "{:?}", second);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&second.stdout).unwrap(),
        serde_json::json!({
            "outcome": "observed",
            "participation_revision": 6,
        })
    );

    let retained = second_participation_server.join().unwrap();
    daimon_server.join().unwrap();
    assert_eq!(commands.load(Ordering::Relaxed), 3);
    assert_eq!(retained.phase, p::PhaseV1::Ready);
    assert_eq!(retained.session_revision, Some(2));
    assert_eq!(retained.pending, None);
}

fn config(fixture: &Fixture) -> ConfigV1 {
    ConfigV1 {
        participation_socket: fixture.participation_socket.clone(),
        daimon_agent_socket: fixture.daimon_socket.clone(),
        participation_id: hex([2; 16]),
        output: OutputConfigV1 {
            path: fixture.output_database.clone(),
            owner: hex([50; 16]),
            max_entries: 4,
            max_output_bytes: 4_096,
            max_page_assistant_bytes: 1_024,
            max_database_pages: 64,
        },
        policy: PolicyConfigV1 {
            principal: hex([20; 16]),
            workspace: hex([21; 16]),
            context: vec![hex([22; 16])],
            tools: vec![hex([23; 16])],
            execution_budget: ExecutionBudgetConfigV1 {
                max_elapsed_millis: 1_000,
                max_input_tokens: 100,
                max_output_tokens: 100,
                max_tool_calls: 1,
                max_context_bytes: 1_024,
                max_output_bytes: 1_024,
            },
            session_budget: SessionBudgetConfigV1 {
                max_elapsed_millis: 10_000,
                max_turns: 4,
                max_events: 32,
                max_event_bytes: 4_096,
                max_prompt_bytes: 4_096,
                max_permission_requests: 4,
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

fn prime_open(database: &Path, binding: p::BindingV1) {
    let mut service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(database, [1; 16], participation_limits()).unwrap(),
    );
    apply(&mut service, None, p::MutationV1::Admit(binding));
    apply(
        &mut service,
        Some(0),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: 0,
            operation: p::OperationV1::Open {
                key: p::IdempotencyKeyV1([13; 16]),
                execution: p::ExecutionIdV1([14; 16]),
            },
        },
    );
}

fn prime_prompt(
    fixture: &Fixture,
    binding: p::BindingV1,
) -> (
    AgentParticipationService<AgentParticipationSqliteStore>,
    AgentPromptSqliteStore,
) {
    let mut service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(
            &fixture.participation_database,
            [1; 16],
            participation_limits(),
        )
        .unwrap(),
    );
    apply(&mut service, None, p::MutationV1::Admit(binding));
    apply(
        &mut service,
        Some(0),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: 0,
            operation: p::OperationV1::Open {
                key: p::IdempotencyKeyV1([13; 16]),
                execution: p::ExecutionIdV1([14; 16]),
            },
        },
    );
    apply(
        &mut service,
        Some(1),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 1,
            key: p::IdempotencyKeyV1([13; 16]),
            observation: p::ObservationV1 {
                phase: p::PhaseV1::Ready,
                session_revision: 1,
                active_turn: None,
                last_event_cursor: None,
                profile_document_digest: [30; 32],
                capabilities: p::CapabilitiesV1::default(),
                permission: None,
                terminal: None,
            },
        },
    );
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
    assert!(matches!(
        service
            .execute_prompt_authorized(
                AccessV1 {
                    read: true,
                    mutate: true,
                },
                p::PromptSubmissionV1 {
                    participation: binding.participation,
                    prompt: b"answer from the retained theorem".to_vec(),
                },
                &mut prompts,
            )
            .unwrap(),
        p::ReplyV1::Accepted(_)
    ));
    (service, prompts)
}

fn prime_ready_resume(database: &Path, binding: p::BindingV1) {
    let mut service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(database, [1; 16], participation_limits()).unwrap(),
    );
    apply(&mut service, None, p::MutationV1::Admit(binding));
    apply(
        &mut service,
        Some(0),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: 0,
            operation: p::OperationV1::Open {
                key: p::IdempotencyKeyV1([13; 16]),
                execution: p::ExecutionIdV1([14; 16]),
            },
        },
    );
    apply(
        &mut service,
        Some(1),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 1,
            key: p::IdempotencyKeyV1([13; 16]),
            observation: p::ObservationV1 {
                phase: p::PhaseV1::Ready,
                session_revision: 1,
                active_turn: None,
                last_event_cursor: None,
                profile_document_digest: [30; 32],
                capabilities: p::CapabilitiesV1 {
                    load_session: true,
                    resume_session: true,
                    ..p::CapabilitiesV1::default()
                },
                permission: None,
                terminal: None,
            },
        },
    );
}

fn apply(
    service: &mut AgentParticipationService<AgentParticipationSqliteStore>,
    revision: Option<u64>,
    mutation: p::MutationV1,
) {
    assert!(matches!(
        service
            .execute(p::CommandV1::Apply {
                expected_storage_revision: revision,
                mutation: Box::new(mutation),
            })
            .unwrap(),
        p::ReplyV1::Accepted(_)
    ));
}

fn binding() -> p::BindingV1 {
    p::BindingV1 {
        participation: p::ParticipationIdV1([2; 16]),
        workflow: WorkflowIdV1([3; 16]),
        deliberation: d::DeliberationIdV1([4; 16]),
        member: d::EnsembleMemberIdV1([5; 16]),
        session: p::AgentSessionIdV1([6; 16]),
        profile: p::ProfileIdV1([7; 16]),
        attempt: p::AttemptV1 {
            task_key: [8; 16],
            incarnation: 1,
            attempt: 1,
        },
        method: d::MethodRefV1 {
            machine_digest: [9; 32],
            state_digest: [10; 32],
        },
        grounding: d::GroundingRefV1 {
            authority_digest: [11; 32],
            surface_digest: [12; 32],
            depth: d::SurfaceDepthV1::Activated,
        },
        persona: d::PersonaLensV1::Researcher,
    }
}

fn participation_limits() -> AgentParticipationStoreLimitsV1 {
    AgentParticipationStoreLimitsV1 {
        codec: CodecLimitsV1 {
            max_bytes: 65_536,
            domain: DomainLimits {
                max_participations: 4,
            },
        },
        max_database_pages: 64,
    }
}

fn output_limits() -> moira_sqlite::agent_output::AgentOutputStoreLimitsV1 {
    moira_sqlite::agent_output::AgentOutputStoreLimitsV1 {
        max_entries: 4,
        max_output_bytes: 4_096,
        max_page_assistant_bytes: 1_024,
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
            "moira-agent-local-{}-{}",
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
