#![cfg(unix)]

use hermes_client::ServicePort;
use hermes_ledger::{Limits, persistence::CheckpointBudgetV1};
use hermes_local_daemon::unix::{Authenticator, Daemon};
use hermes_protocol::{
    AppendOutcomeV1, CommandV1, EventSequenceV1, OutcomeV1, ProducerIdV1, ReplyV1, SchemaIdV1,
    SessionHandleV1, StreamIdV1,
};
use hermes_service::{
    AuthenticatedSessionV1, AuthorizationActionV1, AuthorizationResourceV1, Authorizer, Service,
    ServiceLimitsV1,
};
use hermes_wire::{LimitsV1, decode_ready, decode_reply, encode_request};
use moira_hermes::{
    PublicationV1, PublishErrorV1, WorkflowEventIdV1, WorkflowEventPublisher, encode_transition_v1,
};
use moira_protocol::{EffectIdV1, TransitionV1, WorkflowIdV1};
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

const SUBJECT: [u8; 16] = [0x21; 16];
const STREAM: StreamIdV1 = StreamIdV1([0x31; 16]);
const WORKFLOW: WorkflowIdV1 = WorkflowIdV1([0x41; 16]);
const EVENT: WorkflowEventIdV1 = WorkflowEventIdV1([0x51; 16]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SocketError {
    Io,
    Wire,
    LostReply,
}

struct HermesSocketPort {
    stream: UnixStream,
    lose_next_reply: bool,
}

impl ServicePort for HermesSocketPort {
    type Error = SocketError;

    fn execute(&mut self, _: SessionHandleV1, command: CommandV1) -> Result<ReplyV1, Self::Error> {
        let bytes = encode_request(&command, LimitsV1::default()).map_err(|_| SocketError::Wire)?;
        self.stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .and_then(|()| self.stream.write_all(&bytes))
            .and_then(|()| self.stream.flush())
            .map_err(|_| SocketError::Io)?;
        if self.lose_next_reply {
            self.lose_next_reply = false;
            self.stream
                .shutdown(Shutdown::Both)
                .map_err(|_| SocketError::Io)?;
            return Err(SocketError::LostReply);
        }
        decode_reply(
            &read_frame(&mut self.stream).map_err(|_| SocketError::Io)?,
            LimitsV1::default(),
        )
        .map_err(|_| SocketError::Wire)
    }
}

struct Authentication;

impl Authenticator for Authentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AuthenticatedSessionV1, Self::Error> {
        Ok(AuthenticatedSessionV1 { subject: SUBJECT })
    }
}

struct Policy;

impl Authorizer for Policy {
    type Error = Infallible;

    fn authorize(
        &mut self,
        subject: [u8; 16],
        _: AuthorizationActionV1,
        _: AuthorizationResourceV1,
    ) -> Result<bool, Self::Error> {
        Ok(subject == SUBJECT)
    }
}

fn read_frame(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn connect(socket: &Path, lose_next_reply: bool) -> HermesSocketPort {
    let mut stream = UnixStream::connect(socket).unwrap();
    let ready = decode_ready(&read_frame(&mut stream).unwrap(), LimitsV1::default()).unwrap();
    assert_eq!(ready.protocol_version, 1);
    assert!(ready.max_requests >= 1);
    HermesSocketPort {
        stream,
        lose_next_reply,
    }
}

fn transition() -> TransitionV1 {
    TransitionV1::ChronosSubmissionPrepared {
        workflow: WORKFLOW,
        effect: EffectIdV1(EVENT.0),
    }
}

fn publication() -> PublicationV1 {
    PublicationV1 {
        stream: STREAM,
        expected_predecessor: None,
        producer: ProducerIdV1([0x61; 16]),
        schema: SchemaIdV1([0x71; 16]),
        event: EVENT,
        workflow: WORKFLOW,
        causation: None,
        transition: transition(),
    }
}

fn wait_for_path(path: &Path) {
    for _ in 0..400 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("Hermes endpoint was not published: {}", path.display());
}

fn spawn_helper(mode: &str, root: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("hermes_process_helper")
        .arg("--exact")
        .arg("--nocapture")
        .env("MOIRA_HERMES_HELPER", "1")
        .env("MOIRA_HERMES_MODE", mode)
        .env("MOIRA_HERMES_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn assert_child(child: Child, label: &str) {
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{label} process failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn hermes_process_helper() {
    if std::env::var_os("MOIRA_HERMES_HELPER").is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("MOIRA_HERMES_ROOT").unwrap());
    let limits = Limits::default();
    let budget = CheckpointBudgetV1::default();
    let ledger = match std::env::var("MOIRA_HERMES_MODE").unwrap().as_str() {
        "create" => hermes_local::create_new(root.join("ledger"), limits, budget).unwrap(),
        "open" => hermes_local::open_existing(root.join("ledger"), limits, budget).unwrap(),
        mode => panic!("unknown Hermes helper mode {mode}"),
    };
    let service = Service::new(ledger, Policy, ServiceLimitsV1::default()).unwrap();
    let mut daemon = Daemon::bind_new(
        root.join("hermes.sock"),
        service,
        Authentication,
        LimitsV1::default(),
        Duration::from_secs(3),
        2,
    )
    .unwrap();
    // A deliberately discarded reply may surface as transmission uncertainty
    // after the append has committed. Either transport outcome is acceptable.
    let _ = daemon.serve_next();
}

fn run_lost_reply_restart_cycle() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "moira-hermes-process-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();

    let first = spawn_helper("create", &root);
    wait_for_path(&root.join("hermes.sock"));
    let port = connect(&root.join("hermes.sock"), true);
    let mut publisher = WorkflowEventPublisher::new(SessionHandleV1(0), port);
    assert_eq!(
        publisher.publish(publication()),
        Err(PublishErrorV1::Transport(SocketError::LostReply))
    );
    drop(publisher);
    assert_child(first, "first Hermes");
    assert!(!root.join("hermes.sock").exists());

    let recovered = spawn_helper("open", &root);
    wait_for_path(&root.join("hermes.sock"));
    let port = connect(&root.join("hermes.sock"), false);
    let mut publisher = WorkflowEventPublisher::new(SessionHandleV1(0), port);
    assert_eq!(
        publisher.publish(publication()).unwrap(),
        ReplyV1::Accepted(OutcomeV1::Append(AppendOutcomeV1::Replayed(
            EventSequenceV1(0)
        )))
    );
    let mut port = publisher.into_port();
    let read = port
        .execute(
            SessionHandleV1(0),
            CommandV1::Read {
                stream: STREAM,
                start: EventSequenceV1(0),
                limit: 2,
            },
        )
        .unwrap();
    assert!(matches!(
        read,
        ReplyV1::Accepted(OutcomeV1::Read(batch))
            if batch.events.len() == 1
                && batch.events[0].sequence == EventSequenceV1(0)
                && batch.events[0].envelope.idempotency_key.0 == EVENT.0
                && batch.events[0].envelope.correlation.0 == WORKFLOW.0
                && batch.events[0].envelope.payload == encode_transition_v1(transition())
    ));
    drop(port);
    assert_child(recovered, "recovered Hermes");
    assert!(!root.join("hermes.sock").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn lost_moira_event_reply_replays_once_after_hermes_process_restart() {
    run_lost_reply_restart_cycle();
}

#[test]
#[ignore = "opt-in process soak"]
fn sixteen_hermes_delivery_process_cycles() {
    for _ in 0..16 {
        run_lost_reply_restart_cycle();
    }
}
