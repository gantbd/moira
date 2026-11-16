#![cfg(unix)]

use daimon_client::ServicePort;
use daimon_harness::Limits;
use daimon_host::{
    AttemptRefusalV1, AttemptValidator, AuthenticatedSessionV1, AuthorityGrantV1,
    AuthorityRefusalV1, ExecutionAuthorizer, Provider, ProviderOutcomeV1, ProviderReceiptV1,
};
use daimon_local_daemon::unix::{Authenticator, Daemon, connect};
use daimon_protocol as daimon;
use daimon_service::{Service, ServiceLimitsV1};
use daimon_wire::{LimitsV1, RequestFrameV1, decode_session_opened, encode_request};
use moira_daimon::{AdapterErrorV1, ExecutionAdapter, ExecutionIntentV1, ExecutionReplyV1};
use moira_protocol::{DispositionV1, EffectIdV1, TaskRefV1};
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

struct Authentication;
impl Authenticator for Authentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AuthenticatedSessionV1, Self::Error> {
        Ok(AuthenticatedSessionV1 {
            principal: PRINCIPAL,
            authority_context: [11; 16],
        })
    }
}

struct Authorizer;
impl ExecutionAuthorizer for Authorizer {
    fn authorize(
        &mut self,
        _: &AuthenticatedSessionV1,
        request: &daimon::ExecuteRequestV1,
        _: u64,
    ) -> Result<AuthorityGrantV1, AuthorityRefusalV1> {
        Ok(AuthorityGrantV1 {
            decision_id: [8; 16],
            expires_at: 20,
            tools: request.tools.clone(),
        })
    }
}

struct Attempts;
impl AttemptValidator for Attempts {
    fn validate(
        &mut self,
        _: &AuthenticatedSessionV1,
        _: &daimon::ExecuteRequestV1,
    ) -> Result<(), AttemptRefusalV1> {
        Ok(())
    }
}

struct ProviderOnce(PathBuf);
impl Provider for ProviderOnce {
    fn execute(&mut self, request: &daimon::ExecuteRequestV1) -> ProviderOutcomeV1 {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.0)
            .expect("provider must not execute twice");
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

impl ServicePort for LostReplyPort {
    type Error = SocketError;

    fn execute(
        &mut self,
        session: daimon::SessionHandleV1,
        command: daimon::CommandV1,
    ) -> Result<daimon::ReplyV1, Self::Error> {
        let bytes = encode_request(&RequestFrameV1 { session, command }, LimitsV1::default())
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
    let mut stream = UnixStream::connect(path).unwrap();
    let session =
        decode_session_opened(&read_frame(&mut stream).unwrap(), LimitsV1::default()).unwrap();
    (session, LostReplyPort { stream })
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
        principal: PRINCIPAL,
        profile: [6; 16],
        idempotency_key: [7; 16],
        context: vec![[9; 16]],
        tools: vec![[10; 16]],
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

fn store_limits() -> daimon_checkpoint::StoreLimitsV1 {
    daimon_checkpoint::StoreLimitsV1 {
        max_bytes: 16_384,
        max_executions: 4,
        max_context_references: 4,
        max_tools: 4,
        max_artifacts: 4,
    }
}

fn wait_for_path(path: &Path) {
    for _ in 0..400 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("Daimon endpoint was not published: {}", path.display());
}

fn spawn_helper(mode: &str, root: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("daimon_process_helper")
        .arg("--exact")
        .arg("--nocapture")
        .env("MOIRA_DAIMON_HELPER", "1")
        .env("MOIRA_DAIMON_MODE", mode)
        .env("MOIRA_DAIMON_ROOT", root)
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
fn daimon_process_helper() {
    if std::env::var_os("MOIRA_DAIMON_HELPER").is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("MOIRA_DAIMON_ROOT").unwrap());
    let store = match std::env::var("MOIRA_DAIMON_MODE").unwrap().as_str() {
        "create" => daimon_local::create_new(root.join("execution"), store_limits()).unwrap(),
        "open" => daimon_local::open_existing(root.join("execution"), store_limits()).unwrap(),
        mode => panic!("unknown Daimon helper mode {mode}"),
    };
    let service = Service::open(
        Limits::default(),
        ServiceLimitsV1 {
            max_sessions: 4,
            max_executions: 4,
        },
        Authorizer,
        Attempts,
        ProviderOnce(root.join("provider-called")),
        store,
    )
    .unwrap();
    let mut daemon = Daemon::bind_new(
        root.join("daimon.sock"),
        service,
        Authentication,
        LimitsV1::default(),
        Duration::from_secs(3),
        2,
    )
    .unwrap();
    let _ = daemon.serve_next();
}

#[test]
fn moira_reconciles_lost_execution_reply_after_daimon_process_restart() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "moira-daimon-process-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();

    let first = spawn_helper("create", &root);
    wait_for_path(&root.join("daimon.sock"));
    let (session, port) = lost_reply_port(&root.join("daimon.sock"));
    let mut adapter = ExecutionAdapter::new(session, port);
    assert_eq!(
        adapter.execute(intent(), 10),
        Err(AdapterErrorV1::OutcomeUnknown(SocketError::LostReply))
    );
    drop(adapter);
    assert_child(first, "first Daimon");
    assert!(root.join("provider-called").exists());
    assert!(!root.join("daimon.sock").exists());

    let recovered = spawn_helper("open", &root);
    wait_for_path(&root.join("daimon.sock"));
    let (session, port) = connect(
        root.join("daimon.sock"),
        LimitsV1::default(),
        Duration::from_secs(3),
    )
    .unwrap();
    let mut adapter = ExecutionAdapter::new(session, port);
    let ExecutionReplyV1::Result(receipt) = adapter.execute(intent(), 11).unwrap() else {
        panic!("exact Moira result receipt expected")
    };
    assert_eq!(receipt.effect, EffectIdV1([1; 16]));
    assert_eq!(receipt.execution_id, [2; 16]);
    assert_eq!(receipt.disposition, DispositionV1::Completed);
    drop(adapter);
    assert_child(recovered, "recovered Daimon");
    assert!(!root.join("daimon.sock").exists());
    fs::remove_dir_all(root).unwrap();
}
