#![cfg(unix)]

use aegis_client::AuthorityClient;
use aegis_protocol::{ActionV1, EngineNamespaceV1, OutcomeV1, ReplyV1, ScopeV1};
use chronos_client::WireClientV1;
use daimon_aegis::{AegisAuthorizer, grant_context};
use daimon_chronos::{ChronosAttemptValidator, TaskBindingV1};
use daimon_client::ServicePort;
use daimon_harness::Limits as HarnessLimits;
use daimon_host::{AuthenticatedSessionV1, Provider, ProviderOutcomeV1, ProviderReceiptV1};
use daimon_local_daemon::unix::{Authenticator, Daemon, connect as connect_daimon};
use daimon_protocol as daimon;
use daimon_service::{Service, ServiceLimitsV1};
use daimon_wire::{
    LimitsV1 as DaimonWireLimits, RequestFrameV1, decode_session_opened, encode_request,
};
use hermes_aegis::{AegisAuthorizer as HermesAegisAuthorizer, AuthorizationTime};
use hermes_client::ServicePort as HermesServicePort;
use hermes_ledger::{Limits as HermesLimits, persistence::CheckpointBudgetV1};
use hermes_local_daemon::unix::{Authenticator as HermesAuthenticator, Daemon as HermesDaemon};
use hermes_protocol::{
    AppendOutcomeV1, CommandV1 as HermesCommandV1, EventSequenceV1, OutcomeV1 as HermesOutcomeV1,
    ProducerIdV1, ReplyV1 as HermesReplyV1, SchemaIdV1, SessionHandleV1 as HermesSessionHandleV1,
    StreamIdV1,
};
use hermes_service::{
    AuthenticatedSessionV1 as HermesAuthenticatedSessionV1, Service as HermesService,
    ServiceLimitsV1 as HermesServiceLimitsV1,
};
use hermes_wire::{
    LimitsV1 as HermesWireLimitsV1, decode_ready as decode_hermes_ready,
    decode_reply as decode_hermes_reply, encode_request as encode_hermes_request,
};
use moira_client::{MutationClient, ServicePort as MoiraServicePort};
use moira_core::persistence::CodecLimits;
use moira_daimon::{AdapterErrorV1, ExecutionAdapter, ExecutionIntentV1, ExecutionReplyV1};
use moira_hermes::{
    PublicationV1, WorkflowEventIdV1, WorkflowEventPublisher, encode_transition_v1,
};
use moira_local_daemon::unix::{Authenticator as MoiraAuthenticator, Daemon as MoiraDaemon};
use moira_protocol::{
    AdmissionReceiptV1, AttemptReceiptV1, CommandV1 as MoiraCommandV1, DispositionV1, EffectIdV1,
    MessageRefV1, MutationV1, OutcomeV1 as MoiraOutcomeV1, ParticipantRefV1,
    ReplyV1 as MoiraReplyV1, ResultReceiptV1, TaskRefV1, TransitionV1, WorkflowIdV1,
};
use moira_service::{AccessV1, Service as MoiraService};
use moira_sqlite::{SqliteStore as MoiraSqliteStore, StoreLimits as MoiraStoreLimits, Timing};
use moira_wire::{
    LimitsV1 as MoiraWireLimitsV1, decode_ready as decode_moira_ready,
    decode_reply as decode_moira_reply, encode_request as encode_moira_request,
};
use std::{
    convert::Infallible,
    fs,
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

const PRINCIPAL: [u8; 16] = [5; 16];
const PROFILE: [u8; 16] = [6; 16];
const TOOL: [u8; 16] = [10; 16];
const TASK_KEY: [u8; 16] = [2; 16];
const HERMES_STREAM: StreamIdV1 = StreamIdV1([0x52; 16]);
const WORKFLOW_EVENT: WorkflowEventIdV1 = WorkflowEventIdV1([0x54; 16]);
const MOIRA_OWNER: [u8; 16] = [0x61; 16];
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
enum ChronosError {
    Io,
    Wire,
}

struct ChronosPort(PathBuf);

impl WireClientV1 for ChronosPort {
    type Error = ChronosError;

    fn request(
        &mut self,
        request: chronos_protocol::Request,
    ) -> Result<chronos_protocol::Response, Self::Error> {
        let limits = chronos_limits();
        let bytes = chronos_wire::encode(&request, limits.max_frame_bytes)
            .map_err(|_| ChronosError::Wire)?;
        let mut stream = UnixStream::connect(&self.0).map_err(|_| ChronosError::Io)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(5))))
            .map_err(|_| ChronosError::Io)?;
        stream.write_all(&bytes).map_err(|_| ChronosError::Io)?;
        stream.write_all(b"\n").map_err(|_| ChronosError::Io)?;
        stream.flush().map_err(|_| ChronosError::Io)?;
        stream
            .shutdown(Shutdown::Write)
            .map_err(|_| ChronosError::Io)?;
        let mut reply = Vec::new();
        stream
            .take((limits.max_frame_bytes + 1) as u64)
            .read_to_end(&mut reply)
            .map_err(|_| ChronosError::Io)?;
        if reply.last() == Some(&b'\n') {
            reply.pop();
        }
        chronos_wire::decode_response(&reply, limits.max_frame_bytes)
            .map_err(|_| ChronosError::Wire)
    }
}

fn chronos_limits() -> chronos_protocol::Limits {
    chronos_protocol::Limits {
        max_frame_bytes: 4_096,
        max_id_bytes: 64,
        max_graph_tasks: 4,
        max_dependencies_per_task: 4,
        max_capabilities_per_task: 4,
        max_capability_bytes: 64,
        max_page_entries: 4,
    }
}

struct Authentication {
    authority_context: [u8; 16],
}

impl Authenticator for Authentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AuthenticatedSessionV1, Self::Error> {
        Ok(AuthenticatedSessionV1 {
            principal: PRINCIPAL,
            authority_context: self.authority_context,
        })
    }
}

struct HermesAuthentication;

impl HermesAuthenticator for HermesAuthentication {
    type Error = Infallible;

    fn authenticate(
        &mut self,
        _: &UnixStream,
    ) -> Result<HermesAuthenticatedSessionV1, Self::Error> {
        Ok(HermesAuthenticatedSessionV1 { subject: PRINCIPAL })
    }
}

struct HermesAuthorizationTime;

impl AuthorizationTime for HermesAuthorizationTime {
    fn now(&mut self) -> u64 {
        12
    }
}

struct MoiraAuthentication;

impl MoiraAuthenticator for MoiraAuthentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AccessV1, Self::Error> {
        Ok(AccessV1 {
            read: true,
            mutate: true,
        })
    }
}

struct ProviderOnce(PathBuf);

impl Provider for ProviderOnce {
    fn execute(&mut self, request: &daimon::ExecuteRequestV1) -> ProviderOutcomeV1 {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.0)
            .expect("provider must execute exactly once");
        ProviderOutcomeV1::Receipt(ProviderReceiptV1 {
            execution_id: request.execution_id,
            disposition: daimon::ExecutionDispositionV1::Completed,
            output_bytes: 12,
            artifacts: vec![],
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SocketError {
    Io,
    Wire,
    LostReply,
}

struct LostReplyPort {
    stream: UnixStream,
}

struct HermesSocketPort {
    stream: UnixStream,
}

struct MoiraSocketPort {
    stream: UnixStream,
}

impl MoiraServicePort for MoiraSocketPort {
    type Error = SocketError;

    fn execute(&mut self, command: MoiraCommandV1) -> Result<MoiraReplyV1, Self::Error> {
        let bytes = encode_moira_request(&command, MoiraWireLimitsV1::default())
            .map_err(|_| SocketError::Wire)?;
        self.stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .and_then(|()| self.stream.write_all(&bytes))
            .and_then(|()| self.stream.flush())
            .map_err(|_| SocketError::Io)?;
        decode_moira_reply(
            &read_frame(&mut self.stream).map_err(|_| SocketError::Io)?,
            MoiraWireLimitsV1::default(),
        )
        .map_err(|_| SocketError::Wire)
    }
}

impl HermesServicePort for HermesSocketPort {
    type Error = SocketError;

    fn execute(
        &mut self,
        _: HermesSessionHandleV1,
        command: HermesCommandV1,
    ) -> Result<HermesReplyV1, Self::Error> {
        let bytes = encode_hermes_request(&command, HermesWireLimitsV1::default())
            .map_err(|_| SocketError::Wire)?;
        self.stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .and_then(|()| self.stream.write_all(&bytes))
            .and_then(|()| self.stream.flush())
            .map_err(|_| SocketError::Io)?;
        decode_hermes_reply(
            &read_frame(&mut self.stream).map_err(|_| SocketError::Io)?,
            HermesWireLimitsV1::default(),
        )
        .map_err(|_| SocketError::Wire)
    }
}

impl ServicePort for LostReplyPort {
    type Error = SocketError;

    fn execute(
        &mut self,
        session: daimon::SessionHandleV1,
        command: daimon::CommandV1,
    ) -> Result<daimon::ReplyV1, Self::Error> {
        let bytes = encode_request(
            &RequestFrameV1 { session, command },
            DaimonWireLimits::default(),
        )
        .map_err(|_| SocketError::Wire)?;
        self.stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .and_then(|()| self.stream.write_all(&bytes))
            .and_then(|()| self.stream.flush())
            .map_err(|_| SocketError::Io)?;
        self.stream
            .shutdown(Shutdown::Both)
            .map_err(|_| SocketError::Io)?;
        Err(SocketError::LostReply)
    }
}

fn read_frame(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn lost_reply_port(path: &Path) -> (daimon::SessionHandleV1, LostReplyPort) {
    let mut stream = connect_when_ready(path);
    let session = decode_session_opened(
        &read_frame(&mut stream).unwrap(),
        DaimonWireLimits::default(),
    )
    .unwrap();
    (session, LostReplyPort { stream })
}

fn connect_when_ready(path: &Path) -> UnixStream {
    let mut last = None;
    for _ in 0..1_000 {
        match UnixStream::connect(path) {
            Ok(stream) => return stream,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                last = Some(error);
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("failed to connect to {}: {error}", path.display()),
        }
    }
    panic!(
        "endpoint {} was not connectable within five seconds: {}",
        path.display(),
        last.unwrap()
    )
}

fn store_limits() -> daimon_checkpoint::StoreLimitsV1 {
    daimon_checkpoint::StoreLimitsV1 {
        max_bytes: 16_384,
        max_executions: 4,
        max_context_references: 4,
        max_tools: 4,
        max_artifacts: 4,
    }
}

fn intent(effect: EffectIdV1) -> ExecutionIntentV1 {
    ExecutionIntentV1 {
        effect,
        execution_id: [3; 16],
        task: TaskRefV1 {
            external_task_key: TASK_KEY,
            incarnation: 0,
        },
        attempt_number: 1,
        principal: PRINCIPAL,
        profile: PROFILE,
        idempotency_key: [7; 16],
        context: vec![],
        tools: vec![TOOL],
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

#[test]
fn daimon_chain_helper() {
    if std::env::var_os("MOIRA_LIVE_DAIMON_HELPER").is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("MOIRA_LIVE_ROOT").unwrap());
    let (aegis_session, aegis_port) = aegis_local_daemon::unix::connect(
        root.join("aegis.sock"),
        aegis_wire_limits(),
        Duration::from_secs(5),
    )
    .unwrap();
    let mut authority = AuthorityClient::new(aegis_session, aegis_port);
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(grant)) = authority
        .issue(
            0,
            1,
            100,
            vec![
                ScopeV1 {
                    engine: EngineNamespaceV1::Daimon,
                    resource: PROFILE,
                    actions: 1 << ActionV1::Execute as u8,
                },
                ScopeV1 {
                    engine: EngineNamespaceV1::Daimon,
                    resource: TOOL,
                    actions: 1 << ActionV1::Execute as u8,
                },
            ],
        )
        .unwrap()
    else {
        panic!("Aegis grant expected")
    };
    let context = grant_context(grant.handle);
    let authorizer = AegisAuthorizer::new(authority, grant.handle, PRINCIPAL);
    let attempts = ChronosAttemptValidator::new(
        ChronosPort(root.join("chronos.sock")),
        chronos_limits(),
        1,
        vec![TaskBindingV1 {
            external_task_key: TASK_KEY,
            chronos_task_id: "moira-chain-task".to_owned(),
        }],
    )
    .unwrap();
    let store = match std::env::var("MOIRA_LIVE_DAIMON_MODE").unwrap().as_str() {
        "create" => daimon_local::create_new(root.join("daimon-store"), store_limits()).unwrap(),
        "open" => daimon_local::open_existing(root.join("daimon-store"), store_limits()).unwrap(),
        mode => panic!("unknown Daimon helper mode {mode}"),
    };
    let service = Service::open(
        HarnessLimits::default(),
        ServiceLimitsV1 {
            max_sessions: 2,
            max_executions: 2,
        },
        authorizer,
        attempts,
        ProviderOnce(root.join("provider-called")),
        store,
    )
    .unwrap();
    let mut daemon = Daemon::bind_new(
        root.join("daimon.sock"),
        service,
        Authentication {
            authority_context: context,
        },
        DaimonWireLimits::default(),
        Duration::from_secs(5),
        2,
    )
    .unwrap();
    let _ = daemon.serve_next();
}

#[test]
fn hermes_chain_helper() {
    if std::env::var_os("MOIRA_LIVE_HERMES_HELPER").is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("MOIRA_LIVE_ROOT").unwrap());
    let (aegis_session, aegis_port) = aegis_local_daemon::unix::connect(
        root.join("aegis.sock"),
        aegis_wire_limits(),
        Duration::from_secs(5),
    )
    .unwrap();
    let mut authority = AuthorityClient::new(aegis_session, aegis_port);
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(grant)) = authority
        .issue(
            0,
            11,
            100,
            vec![ScopeV1 {
                engine: EngineNamespaceV1::Hermes,
                resource: HERMES_STREAM.0,
                actions: (1 << ActionV1::Submit as u8) | (1 << ActionV1::Observe as u8),
            }],
        )
        .unwrap()
    else {
        panic!("Hermes Aegis grant expected")
    };
    let authorizer =
        HermesAegisAuthorizer::new(authority, grant.handle, PRINCIPAL, HermesAuthorizationTime);
    let ledger = hermes_local::create_new(
        root.join("hermes-store"),
        HermesLimits::default(),
        CheckpointBudgetV1::default(),
    )
    .unwrap();
    let service = HermesService::new(ledger, authorizer, HermesServiceLimitsV1::default()).unwrap();
    let mut daemon = HermesDaemon::bind_new(
        root.join("hermes.sock"),
        service,
        HermesAuthentication,
        HermesWireLimitsV1::default(),
        Duration::from_secs(5),
        2,
    )
    .unwrap();
    daemon.serve_next().unwrap();
}

fn moira_store_limits() -> MoiraStoreLimits {
    MoiraStoreLimits {
        codec: CodecLimits {
            max_bytes: 16_384,
            max_workflows: 4,
        },
        max_database_pages: 64,
        timing: Timing::Off,
    }
}

#[test]
fn moira_chain_helper() {
    if std::env::var_os("MOIRA_LIVE_MOIRA_HELPER").is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("MOIRA_LIVE_ROOT").unwrap());
    let store =
        MoiraSqliteStore::open(root.join("moira.db"), MOIRA_OWNER, moira_store_limits()).unwrap();
    let service = MoiraService::new(store);
    let mut daemon = MoiraDaemon::bind_new(
        root.join("moira.sock"),
        service,
        MoiraAuthentication,
        MoiraWireLimitsV1::default(),
        Duration::from_secs(5),
        6,
    )
    .unwrap();
    daemon.serve_next().unwrap();
}

fn aegis_wire_limits() -> aegis_wire::LimitsV1 {
    aegis_wire::LimitsV1::default()
}

fn spawn_aegis(binary: &Path, root: &Path, connections: usize) -> Child {
    Command::new(binary)
        .env("AEGIS_CHAIN_ROOT", root)
        .env("AEGIS_CHAIN_CONNECTIONS", connections.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn spawn_daimon(root: &Path, mode: &str) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("daimon_chain_helper")
        .arg("--exact")
        .arg("--nocapture")
        .env("MOIRA_LIVE_DAIMON_HELPER", "1")
        .env("MOIRA_LIVE_ROOT", root)
        .env("MOIRA_LIVE_DAIMON_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn spawn_hermes(root: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("hermes_chain_helper")
        .arg("--exact")
        .arg("--nocapture")
        .env("MOIRA_LIVE_HERMES_HELPER", "1")
        .env("MOIRA_LIVE_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn spawn_moira(root: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("moira_chain_helper")
        .arg("--exact")
        .arg("--nocapture")
        .env("MOIRA_LIVE_MOIRA_HELPER", "1")
        .env("MOIRA_LIVE_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for(path: &Path, child: &mut Child, label: &str) {
    for _ in 0..36_000 {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "{label} exited before publishing {}: {status}",
                path.display()
            );
        }
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "{label} did not publish {} within 180 seconds",
        path.display()
    );
}

fn assert_child(child: Child, label: &str) {
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{label} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn connect_moira(root: &Path) -> MutationClient<MoiraSocketPort> {
    let mut stream = connect_when_ready(&root.join("moira.sock"));
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(5))))
        .unwrap();
    let ready = decode_moira_ready(
        &read_frame(&mut stream).unwrap(),
        MoiraWireLimitsV1::default(),
    )
    .unwrap();
    assert_eq!(ready.protocol_version, 1);
    MutationClient::new(MoiraSocketPort { stream })
}

fn prepare_execution(
    client: &mut MutationClient<MoiraSocketPort>,
) -> (WorkflowIdV1, ExecutionIntentV1) {
    let MoiraReplyV1::Accepted(MoiraOutcomeV1::Applied {
        transition: TransitionV1::ChronosSubmissionPrepared { workflow, effect },
        replayed: false,
        ..
    }) = client
        .apply(
            0,
            MutationV1::AdmitMessage {
                message: MessageRefV1 {
                    source: [0x62; 16],
                    room_sequence: 1,
                    message_sequence: 1,
                },
                recipient: ParticipantRefV1([0x63; 16]),
            },
        )
        .unwrap()
    else {
        panic!("Moira admission expected")
    };
    let MoiraReplyV1::Accepted(MoiraOutcomeV1::Applied {
        transition: TransitionV1::ChronosSubmissionAccepted { .. },
        replayed: false,
        ..
    }) = client
        .apply(
            1,
            MutationV1::AcceptChronosAdmission {
                workflow,
                receipt: AdmissionReceiptV1 {
                    effect,
                    task: TaskRefV1 {
                        external_task_key: TASK_KEY,
                        incarnation: 0,
                    },
                    chronos_revision: 1,
                },
            },
        )
        .unwrap()
    else {
        panic!("Moira Chronos admission expected")
    };
    let MoiraReplyV1::Accepted(MoiraOutcomeV1::Applied {
        transition:
            TransitionV1::DaimonExecutionPrepared {
                effect,
                task,
                attempt_number,
                ..
            },
        replayed: false,
        ..
    }) = client
        .apply(
            2,
            MutationV1::AcceptChronosAttempt {
                workflow,
                receipt: AttemptReceiptV1 {
                    task: TaskRefV1 {
                        external_task_key: TASK_KEY,
                        incarnation: 0,
                    },
                    attempt_number: 1,
                    chronos_revision: 2,
                },
            },
        )
        .unwrap()
    else {
        panic!("Moira Daimon preparation expected")
    };
    let mut prepared = intent(effect);
    prepared.task = task;
    prepared.attempt_number = attempt_number;
    (workflow, prepared)
}

fn accept_execution_result(
    client: &mut MutationClient<MoiraSocketPort>,
    workflow: WorkflowIdV1,
    receipt: ResultReceiptV1,
) -> TransitionV1 {
    let MoiraReplyV1::Accepted(MoiraOutcomeV1::Applied {
        transition,
        replayed: false,
        ..
    }) = client
        .apply(3, MutationV1::AcceptDaimonResult { workflow, receipt })
        .unwrap()
    else {
        panic!("Moira Daimon result acceptance expected")
    };
    assert_eq!(
        transition,
        TransitionV1::DaimonResultAccepted {
            workflow,
            disposition: DispositionV1::Completed,
        }
    );
    transition
}

fn publish_execution_result(root: &Path, workflow: WorkflowIdV1, transition: TransitionV1) {
    let mut stream = connect_when_ready(&root.join("hermes.sock"));
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(5))))
        .unwrap();
    let ready = decode_hermes_ready(
        &read_frame(&mut stream).unwrap(),
        HermesWireLimitsV1::default(),
    )
    .unwrap();
    assert_eq!(ready.protocol_version, 1);

    let mut publisher =
        WorkflowEventPublisher::new(HermesSessionHandleV1(0), HermesSocketPort { stream });
    assert_eq!(
        publisher
            .publish(PublicationV1 {
                stream: HERMES_STREAM,
                expected_predecessor: None,
                producer: ProducerIdV1([0x55; 16]),
                schema: SchemaIdV1([0x56; 16]),
                event: WORKFLOW_EVENT,
                workflow,
                causation: None,
                transition,
            })
            .unwrap(),
        HermesReplyV1::Accepted(HermesOutcomeV1::Append(AppendOutcomeV1::Appended(
            EventSequenceV1(0)
        )))
    );
    let mut port = publisher.into_port();
    let reply = port
        .execute(
            HermesSessionHandleV1(0),
            HermesCommandV1::Read {
                stream: HERMES_STREAM,
                start: EventSequenceV1(0),
                limit: 2,
            },
        )
        .unwrap();
    assert!(matches!(
        reply,
        HermesReplyV1::Accepted(HermesOutcomeV1::Read(batch))
            if batch.events.len() == 1
                && batch.events[0].sequence == EventSequenceV1(0)
                && batch.events[0].envelope.idempotency_key.0 == WORKFLOW_EVENT.0
                && batch.events[0].envelope.correlation.0 == workflow.0
                && batch.events[0].envelope.payload == encode_transition_v1(transition)
    ));
}

fn run_live_execution_cycle(recover_lost_reply: bool) {
    let root = std::env::temp_dir().join(format!(
        "moira-live-chain-{}-{}",
        std::process::id(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let aegis_binary = PathBuf::from(std::env::var_os("AEGIS_CHAIN_HELPER").unwrap());
    let chronos_binary = PathBuf::from(std::env::var_os("CHRONOS_CHAIN_HELPER").unwrap());

    let mut moira_process = spawn_moira(&root);
    wait_for(
        &root.join("moira.sock"),
        &mut moira_process,
        "Moira process",
    );
    let mut moira = connect_moira(&root);
    let (workflow, execution_intent) = prepare_execution(&mut moira);
    drop(moira);
    assert_child(moira_process, "preparation Moira process");
    assert!(!root.join("moira.sock").exists());

    let mut aegis = spawn_aegis(&aegis_binary, &root, if recover_lost_reply { 3 } else { 2 });
    let mut chronos = Command::new(chronos_binary)
        .env("CHRONOS_CHAIN_SOCKET", root.join("chronos.sock"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(&root.join("aegis.sock"), &mut aegis, "Aegis process");
    wait_for(&root.join("chronos.sock"), &mut chronos, "Chronos process");

    let mut daimon = spawn_daimon(&root, "create");
    wait_for(&root.join("daimon.sock"), &mut daimon, "Daimon process");
    let receipt = if recover_lost_reply {
        let (session, port) = lost_reply_port(&root.join("daimon.sock"));
        let mut adapter = ExecutionAdapter::new(session, port);
        assert_eq!(
            adapter.execute(execution_intent.clone(), 10),
            Err(AdapterErrorV1::OutcomeUnknown(SocketError::LostReply))
        );
        drop(adapter);
        assert_child(daimon, "first Daimon process");
        assert!(root.join("provider-called").exists());
        assert!(!root.join("daimon.sock").exists());

        let mut recovered = spawn_daimon(&root, "open");
        wait_for(
            &root.join("daimon.sock"),
            &mut recovered,
            "recovered Daimon process",
        );
        let (session, port) = connect_daimon(
            root.join("daimon.sock"),
            DaimonWireLimits::default(),
            Duration::from_secs(5),
        )
        .unwrap();
        let mut adapter = ExecutionAdapter::new(session, port);
        let receipt = assert_completed(adapter.execute(execution_intent.clone(), 11).unwrap());
        drop(adapter);
        assert_child(recovered, "recovered Daimon process");
        receipt
    } else {
        let (session, port) = connect_daimon(
            root.join("daimon.sock"),
            DaimonWireLimits::default(),
            Duration::from_secs(5),
        )
        .unwrap();
        let mut adapter = ExecutionAdapter::new(session, port);
        let receipt = assert_completed(adapter.execute(execution_intent, 10).unwrap());
        drop(adapter);
        assert_child(daimon, "Daimon process");
        receipt
    };

    let mut moira_process = spawn_moira(&root);
    wait_for(
        &root.join("moira.sock"),
        &mut moira_process,
        "recovered Moira process",
    );
    let mut moira = connect_moira(&root);
    let transition = accept_execution_result(&mut moira, workflow, receipt);

    let mut hermes = spawn_hermes(&root);
    wait_for(&root.join("hermes.sock"), &mut hermes, "Hermes process");
    publish_execution_result(&root, workflow, transition);
    assert_child(hermes, "Hermes process");

    drop(moira);
    assert_child(moira_process, "recovered Moira process");

    assert_child(chronos, "Chronos process");
    assert_child(aegis, "Aegis process");
    assert!(root.join("provider-called").exists());
    assert!(!root.join("aegis.sock").exists());
    assert!(!root.join("chronos.sock").exists());
    assert!(!root.join("daimon.sock").exists());
    assert!(!root.join("hermes.sock").exists());
    assert!(!root.join("moira.sock").exists());
    fs::remove_dir_all(root).unwrap();
}

fn assert_completed(reply: ExecutionReplyV1) -> ResultReceiptV1 {
    let ExecutionReplyV1::Result(receipt) = reply else {
        panic!("completed Moira receipt expected")
    };
    assert_eq!(receipt.disposition, DispositionV1::Completed);
    receipt
}

#[test]
fn moira_executes_through_live_peers_then_publishes_one_hermes_event() {
    run_live_execution_cycle(false);
}

#[test]
fn lost_reply_recovers_without_redispatch_then_publishes_one_hermes_event() {
    run_live_execution_cycle(true);
}

#[test]
#[ignore = "opt-in multi-process assurance campaign"]
fn sixteen_live_execution_process_cycles() {
    for _ in 0..16 {
        run_live_execution_cycle(true);
    }
}
