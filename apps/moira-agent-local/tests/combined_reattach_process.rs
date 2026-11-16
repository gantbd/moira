#![cfg(unix)]

use aegis_authority::{EngineId, Limits as AegisLimits, PrincipalId, SessionId};
use aegis_local_daemon::unix::{Authenticator as AegisAuthenticator, Daemon as AegisDaemon};
use aegis_service::{
    AuthenticatedSessionV1 as AegisSessionV1, Service as AegisService,
    ServiceLimitsV1 as AegisServiceLimitsV1,
};
use daimon_client::{AgentReadOnlyClient, AgentSessionClient};
use daimon_protocol::{self as dp, agent::*};
use moira_core::agent_participation::{LimitsV1 as DomainLimits, persistence::CodecLimitsV1};
use moira_local_daemon::unix::{AgentParticipationAuthenticator, AgentParticipationDaemon};
use moira_protocol::{WorkflowIdV1, agent_participation as p, deliberation as d};
use moira_service::{AccessV1, agent_participation::AgentParticipationService};
use moira_sqlite::agent_participation::{
    AgentParticipationSqliteStore, AgentParticipationStoreLimitsV1,
};
use moira_wire::LimitsV1 as MoiraLimitsV1;
use std::{
    convert::Infallible,
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct AegisAuthentication;

impl AegisAuthenticator for AegisAuthentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AegisSessionV1, Self::Error> {
        Ok(AegisSessionV1 {
            principal: PrincipalId::new([1; 16]),
            session: SessionId::new([9; 16]),
            administrator: true,
        })
    }
}

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

struct Fixture {
    root: PathBuf,
    aegis_socket: PathBuf,
    chronos_socket: PathBuf,
    daimon_socket: PathBuf,
    participation_socket: PathBuf,
    daimon_store: PathBuf,
    participation_database: PathBuf,
    output_database: PathBuf,
    daimon_config: PathBuf,
    worker_config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "moira-daimon-reattach-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self {
            aegis_socket: root.join("aegis.sock"),
            chronos_socket: root.join("chronos.sock"),
            daimon_socket: root.join("daimon.sock"),
            participation_socket: root.join("participation.sock"),
            daimon_store: root.join("daimon-store"),
            participation_database: root.join("participation.db"),
            output_database: root.join("output.db"),
            daimon_config: root.join("daimon.json"),
            worker_config: root.join("worker.json"),
            root,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
#[ignore = "requires the separately built real daimon-local-agent executable"]
fn moira_recovers_through_real_daimon_process_kill_and_explicit_confirmation() {
    let fixture = Fixture::new();
    let aegis = start_aegis(&fixture, 2);
    let chronos = start_chronos(&fixture, 2);
    write_daimon_config(&fixture);
    write_worker_config(&fixture);
    let mut first = start_daimon(&fixture);

    let session = dp::AgentSessionIdV1::new([6; 16]);
    let (service_session, port) = connect_daimon(&fixture);
    let mut client = AgentSessionClient::new(service_session, port);
    let open_reply = client
        .open(
            AgentOpenRequestV1 {
                session,
                execution: execution(12, 13),
                workspace: [6; 16],
                budget: AgentSessionBudgetV1 {
                    max_elapsed_millis: 10_000,
                    max_turns: 2,
                    max_events: 32,
                    max_event_bytes: 4_096,
                    max_prompt_bytes: 4_096,
                    max_permission_requests: 1,
                },
            },
            10,
        )
        .unwrap();
    let AgentReplyV1::Accepted(AgentOutcomeV1::Session(opened)) = open_reply else {
        panic!("real Daimon open expected, received {open_reply:?}")
    };
    assert_eq!(opened.phase, AgentSessionPhaseV1::Ready);
    assert!(opened.capabilities.resume_session);
    drop(client);
    prime_ready(&fixture, &opened);

    first.kill().unwrap();
    first.wait().unwrap();
    let _ = fs::remove_file(&fixture.daimon_socket);
    let mut second = start_daimon(&fixture);

    let service = serve_worker(&fixture, open_service(&fixture));
    let first_worker = run_worker(&fixture);
    assert!(first_worker.status.success(), "{first_worker:?}");
    let mut service = service.join().unwrap();
    let challenge = match service
        .execute(p::CommandV1::ReattachConfirmation {
            participation: p::ParticipationIdV1([2; 16]),
        })
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::ReattachConfirmation(Some(value)) => value,
            other => panic!("reattach challenge expected, received {other:?}"),
        },
        other => panic!("accepted challenge expected, received {other:?}"),
    };
    assert!(matches!(
        service
            .execute(p::CommandV1::ConfirmReattach(p::ConfirmedReattachV1 {
                participation: p::ParticipationIdV1([2; 16]),
                confirmation_digest: challenge.confirmation_digest,
            }))
            .unwrap(),
        p::ReplyV1::Accepted(value)
            if matches!(*value, p::OutcomeV1::Applied { replayed: false, .. })
    ));

    let service = serve_worker(&fixture, service);
    let second_worker = run_worker(&fixture);
    assert!(second_worker.status.success(), "{second_worker:?}");
    let mut service = service.join().unwrap();
    let view = participation(&mut service);
    assert_eq!(view.phase, p::PhaseV1::Ready);
    assert_eq!(view.session_revision, Some(2));
    assert!(view.pending.is_none());

    let (service_session, port) = connect_daimon(&fixture);
    let mut reader = AgentReadOnlyClient::new(service_session, port);
    let detached = reader.current_view(session).unwrap();
    assert_eq!(detached.phase, AgentSessionPhaseV1::Ready);
    assert_eq!(detached.revision, 2);
    drop(reader);
    second.kill().unwrap();
    second.wait().unwrap();
    chronos.join().unwrap();
    aegis.join().unwrap();
}

fn start_aegis(fixture: &Fixture, count: usize) -> thread::JoinHandle<()> {
    let id = EngineId::new([8; 16]);
    let authority =
        aegis_local::create_new(fixture.root.join("authority"), id, AegisLimits::default())
            .unwrap();
    let service = AegisService::new(authority, id, AegisServiceLimitsV1::default()).unwrap();
    let mut daemon = AegisDaemon::bind_new(
        &fixture.aegis_socket,
        service,
        AegisAuthentication,
        aegis_wire::LimitsV1::default(),
        Duration::from_secs(3),
        16,
    )
    .unwrap();
    thread::spawn(move || {
        for _ in 0..count {
            daemon.serve_next().unwrap();
        }
    })
}

fn start_chronos(fixture: &Fixture, count: usize) -> thread::JoinHandle<()> {
    let listener = UnixListener::bind(&fixture.chronos_socket).unwrap();
    let limits = chronos_protocol::Limits {
        max_frame_bytes: 1_048_576,
        max_id_bytes: 256,
        max_graph_tasks: 1024,
        max_dependencies_per_task: 128,
        max_capabilities_per_task: 64,
        max_capability_bytes: 256,
        max_page_entries: 256,
    };
    thread::spawn(move || {
        for _ in 0..count {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            BufReader::new(stream.try_clone().unwrap())
                .take((limits.max_frame_bytes + 1) as u64)
                .read_until(b'\n', &mut request)
                .unwrap();
            assert_eq!(request.pop(), Some(b'\n'));
            let request = chronos_wire::decode_request(&request, limits).unwrap();
            assert!(
                matches!(request.command, chronos_protocol::Command::Snapshot { ref task } if task == "local-agent-task")
            );
            let response = chronos_protocol::Response {
                version: 1,
                outcome: chronos_protocol::Outcome::Snapshot {
                    value: Some(chronos_protocol::Entry {
                        reference: chronos_protocol::Reference {
                            task: "local-agent-task".into(),
                            incarnation: "1".into(),
                        },
                        snapshot: chronos_protocol::Snapshot {
                            phase: chronos_protocol::Phase::Running,
                            attempt_number: Some(1),
                            dependency_count: 0,
                            required_capability_count: 0,
                            priority: 0,
                            max_attempts: 1,
                        },
                    }),
                    revision: Some(chronos_protocol::Revision::Finite("1".into())),
                },
            };
            stream
                .write_all(&chronos_wire::encode(&response, limits.max_frame_bytes).unwrap())
                .unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
        }
    })
}

fn write_daimon_config(f: &Fixture) {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../daimon/adapters/daimon-acp/tests/support/fake_acp_agent.sh")
        .canonicalize()
        .unwrap();
    let value = serde_json::json!({
        "principal": hex([1;16]), "aegis_socket": f.aegis_socket, "aegis_expected_epoch": 0,
        "aegis_grant_now": 1, "aegis_grant_expires_at": 1000, "chronos_socket": f.chronos_socket,
        "daimon_socket": f.daimon_socket, "store_directory": f.daimon_store,
        "providers": [{"provider_id":hex([2;16]),"harness_profile_id":hex([3;16]),"policy_digest":format!("sha256:{}","22".repeat(32)),"program":"/bin/sh","arguments":[script,"resume"],"inherited_environment_names":[],"isolation":{"kind":"direct"},"transport":"acp"}],
        "workspaces":[{"identity":hex([6;16]),"path":f.root}],
        "task_bindings":[{"external_task_key":hex([7;16]),"chronos_task_id":"local-agent-task"}],
        "tool_capabilities":[],"context":[],"io_timeout_ms":3000,"max_agent_sessions":8,"max_service_sessions":8,"max_requests_per_connection":16,"max_store_bytes":1048576,"max_context_bytes":4096
    });
    fs::write(&f.daimon_config, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn write_worker_config(f: &Fixture) {
    let value = serde_json::json!({
        "participation_socket":f.participation_socket,"daimon_agent_socket":f.daimon_socket,"participation_id":hex([2;16]),
        "output":{"path":f.output_database,"owner":hex([50;16]),"max_entries":4,"max_output_bytes":4096,"max_page_assistant_bytes":1024,"max_database_pages":64},
        "policy":{"principal":hex([1;16]),"workspace":hex([6;16]),"context":[],"tools":[],"execution_budget":{"max_elapsed_millis":1000,"max_input_tokens":100,"max_output_tokens":100,"max_tool_calls":1,"max_context_bytes":1024,"max_output_bytes":1024},"session_budget":{"max_elapsed_millis":10000,"max_turns":2,"max_events":32,"max_event_bytes":4096,"max_prompt_bytes":4096,"max_permission_requests":1}},
        "transport":{"participation_max_frame_bytes":65536,"participation_max_requests":8,"daimon_handshake_max_frame_bytes":1048576,"agent_max_frame_bytes":1048576,"timeout_ms":3000},"now":20,"max_event_observations":32
    });
    fs::write(&f.worker_config, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn start_daimon(f: &Fixture) -> Child {
    let program =
        std::env::var_os("DAIMON_LOCAL_AGENT_PROGRAM").expect("real Daimon program required");
    let child = Command::new(program)
        .arg(&f.daimon_config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    wait_for_socket(&f.daimon_socket);
    child
}

fn run_worker(f: &Fixture) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_moira-agent-local"))
        .arg(&f.worker_config)
        .output()
        .unwrap()
}

fn connect_daimon(f: &Fixture) -> (dp::SessionHandleV1, daimon_client::unix::AgentUnixPort) {
    daimon_client::unix::connect_agent(
        &f.daimon_socket,
        daimon_wire::LimitsV1::default(),
        daimon_wire::agent::AgentWireLimitsV1::default(),
        Duration::from_secs(3),
    )
    .unwrap()
}

fn wait_for_socket(path: &Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(10));
    }
}

fn open_service(f: &Fixture) -> AgentParticipationService<AgentParticipationSqliteStore> {
    AgentParticipationService::new(
        AgentParticipationSqliteStore::open(
            &f.participation_database,
            [1; 16],
            participation_limits(),
        )
        .unwrap(),
    )
}

fn serve_worker(
    f: &Fixture,
    service: AgentParticipationService<AgentParticipationSqliteStore>,
) -> thread::JoinHandle<AgentParticipationService<AgentParticipationSqliteStore>> {
    let daemon = AgentParticipationDaemon::bind_new(
        &f.participation_socket,
        service,
        ParticipationAuthentication,
        MoiraLimitsV1::default(),
        Duration::from_secs(3),
        8,
    )
    .unwrap();
    thread::spawn(move || {
        let mut daemon = daemon;
        daemon.serve_next().unwrap();
        daemon.serve_next().unwrap();
        daemon.into_parts().0
    })
}

fn prime_ready(f: &Fixture, opened: &AgentSessionViewV1) {
    let mut service = open_service(f);
    apply(&mut service, None, p::MutationV1::Admit(binding()));
    apply(
        &mut service,
        Some(0),
        p::MutationV1::Prepare {
            participation: p::ParticipationIdV1([2; 16]),
            expected_revision: 0,
            operation: p::OperationV1::Open {
                key: p::IdempotencyKeyV1([13; 16]),
                execution: p::ExecutionIdV1([12; 16]),
            },
        },
    );
    apply(
        &mut service,
        Some(1),
        p::MutationV1::Observe {
            participation: p::ParticipationIdV1([2; 16]),
            expected_revision: 1,
            key: p::IdempotencyKeyV1([13; 16]),
            observation: p::ObservationV1 {
                phase: p::PhaseV1::Ready,
                session_revision: opened.revision,
                active_turn: None,
                last_event_cursor: None,
                profile_document_digest: opened.profile_document_digest.into_bytes(),
                capabilities: p::CapabilitiesV1 {
                    load_session: opened.capabilities.load_session,
                    resume_session: opened.capabilities.resume_session,
                    permissions: opened.capabilities.permissions,
                    filesystem_read: opened.capabilities.filesystem_read,
                    filesystem_write: opened.capabilities.filesystem_write,
                    terminal: opened.capabilities.terminal,
                    session_modes: opened.capabilities.session_modes,
                    session_models: opened.capabilities.session_models,
                },
                permission: None,
                terminal: None,
            },
        },
    );
}

fn apply(
    s: &mut AgentParticipationService<AgentParticipationSqliteStore>,
    r: Option<u64>,
    m: p::MutationV1,
) {
    assert!(matches!(
        s.execute(p::CommandV1::Apply {
            expected_storage_revision: r,
            mutation: Box::new(m)
        })
        .unwrap(),
        p::ReplyV1::Accepted(_)
    ));
}
fn participation(
    s: &mut AgentParticipationService<AgentParticipationSqliteStore>,
) -> p::ParticipationViewV1 {
    match s
        .execute(p::CommandV1::Participation {
            participation: p::ParticipationIdV1([2; 16]),
        })
        .unwrap()
    {
        p::ReplyV1::Accepted(v) => match *v {
            p::OutcomeV1::Participation(Some(v)) => v,
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
}

fn binding() -> p::BindingV1 {
    p::BindingV1 {
        participation: p::ParticipationIdV1([2; 16]),
        workflow: WorkflowIdV1([3; 16]),
        deliberation: d::DeliberationIdV1([4; 16]),
        member: d::EnsembleMemberIdV1([5; 16]),
        session: p::AgentSessionIdV1([6; 16]),
        profile: p::ProfileIdV1([3; 16]),
        attempt: p::AttemptV1 {
            task_key: [7; 16],
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
            max_bytes: 65536,
            domain: DomainLimits {
                max_participations: 4,
            },
        },
        max_database_pages: 64,
    }
}
fn execution(seed: u8, key: u8) -> dp::ExecuteRequestV1 {
    dp::ExecuteRequestV1 {
        execution_id: dp::ExecutionIdV1::new([seed; 16]),
        task_attempt: dp::TaskAttemptRefV1 {
            task_key: [7; 16],
            task_incarnation: 1,
            attempt_number: 1,
        },
        principal: dp::PrincipalIdV1::new([1; 16]),
        profile: dp::HarnessProfileIdV1::new([3; 16]),
        idempotency_key: dp::IdempotencyKeyV1::new([key; 16]),
        context: Vec::new(),
        tools: Vec::new(),
        budget: dp::ExecutionBudgetV1 {
            max_elapsed_millis: 5000,
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 1,
            max_context_bytes: 4096,
            max_output_bytes: 4096,
        },
    }
}
fn hex<const N: usize>(bytes: [u8; N]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
