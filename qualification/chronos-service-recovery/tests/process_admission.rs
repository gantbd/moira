#![cfg(unix)]

use chronos_client::{WireClientV1, WireClientV2, WireClientV3, WireClientV4};
use chronos_local::{CallError, LocalClient};
use chronos_protocol as wire;
use moira_chronos::{
    AdapterErrorV1, AdmissionAdapter, AdmissionIntentV1, AdmissionReplyV1, PreparedAdmissionV1,
    RetrySafetyV1,
};
use moira_protocol::{AdmissionReceiptV1, EffectIdV1, TaskRefV1};
use std::{
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

struct ProcessPort {
    directory: PathBuf,
    lose_execute_reply: bool,
    lost_stream: Option<UnixStream>,
}

impl WireClientV4 for ProcessPort {
    type Error = CallError;
    fn request_v4(
        &mut self,
        request: wire::coordination_v4::Request,
    ) -> Result<wire::coordination_v4::Response, CallError> {
        LocalClient::new(&self.directory).request_v4(request)
    }
}

impl WireClientV3 for ProcessPort {
    type Error = CallError;

    fn request_v3(
        &mut self,
        request: wire::coordination_v3::Request,
    ) -> Result<wire::coordination_v3::Response, Self::Error> {
        if self.lose_execute_reply
            && matches!(
                request.command,
                wire::coordination_v3::Command::ExecuteInvocation { .. }
            )
        {
            self.lose_execute_reply = false;
            request
                .validate(chronos_local::wire_limits())
                .map_err(|_| CallError::NotSent)?;
            let bytes = chronos_wire::encode(&request, chronos_local::MAX_REQUEST_BYTES)
                .map_err(|_| CallError::NotSent)?;
            let socket =
                chronos_local::verify_socket(&self.directory).map_err(|_| CallError::NotSent)?;
            let mut stream = UnixStream::connect(socket).map_err(|_| CallError::NotSent)?;
            stream
                .shutdown(Shutdown::Read)
                .map_err(|_| CallError::NotSent)?;
            stream
                .write_all(&bytes)
                .and_then(|()| stream.write_all(b"\n"))
                .and_then(|()| stream.flush())
                .map_err(|_| CallError::OutcomeUnknown)?;
            stream
                .shutdown(Shutdown::Write)
                .map_err(|_| CallError::OutcomeUnknown)?;
            // Keep the half-closed connection alive until the server has
            // consumed the complete request, matching the transport's own
            // discarded-reply qualification rather than racing a full close.
            self.lost_stream = Some(stream);
            return Err(CallError::OutcomeUnknown);
        }
        LocalClient::new(&self.directory).request_v3(request)
    }
}

fn intent() -> AdmissionIntentV1 {
    AdmissionIntentV1 {
        effect: EffectIdV1([0x11; 16]),
        external_task_key: [0x21; 16],
        task_id: "moira-process-task".to_owned(),
        expected_chronos_revision: 0,
        priority: 3,
        capabilities: vec!["agent".to_owned()],
        max_attempts: 1,
        retry_safety: RetrySafetyV1::FenceUnknown,
    }
}

fn wait_for_socket(directory: &Path, child: &mut Child) {
    // Newly linked qualification binaries can be held before Rust main by the
    // host executable scanner. Keep the wait finite, but distinguish that delay
    // from a helper that has already failed instead of reporting both as a
    // missing Chronos endpoint.
    for _ in 0..36_000 {
        if chronos_local::verify_socket(directory).is_ok() {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            let mut stderr = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                pipe.read_to_string(&mut stderr).unwrap();
            }
            panic!("Chronos helper exited before endpoint publication ({status}): {stderr}");
        }
        thread::sleep(Duration::from_millis(5));
    }
    child.kill().unwrap();
    let _ = child.wait();
    panic!(
        "Chronos process endpoint was not published within 180 seconds: {}",
        directory.display()
    );
}

fn spawn_helper(directory: &Path, requests: usize) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("chronos_process_helper")
        .arg("--exact")
        .arg("--nocapture")
        .env("MOIRA_CHRONOS_HELPER", "1")
        .env("MOIRA_CHRONOS_DIRECTORY", directory)
        .env("MOIRA_CHRONOS_REQUESTS", requests.to_string())
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
fn chronos_process_helper() {
    if std::env::var_os("MOIRA_CHRONOS_HELPER").is_none() {
        return;
    }
    let directory = PathBuf::from(std::env::var_os("MOIRA_CHRONOS_DIRECTORY").unwrap());
    let requests: usize = std::env::var("MOIRA_CHRONOS_REQUESTS")
        .unwrap()
        .parse()
        .unwrap();
    let mut server = chronos_standalone::Server::bind_with_options(
        &directory,
        chronos_standalone::ServerOptions {
            allow_writes: true,
            atomic_state: true,
            observer_history: false,
            allocation_receipts: true,
        },
    )
    .unwrap();
    for _ in 0..requests {
        server.serve_one().unwrap();
    }
}

#[test]
fn lost_reply_reconciles_original_after_restart_retirement_and_name_reuse() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // Keep the Unix socket path below macOS's sockaddr_un ceiling.
    let directory = PathBuf::from(format!(
        "/private/tmp/moira-ch-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }

    let mut first = spawn_helper(&directory, 2);
    wait_for_socket(&directory, &mut first);
    let limits = chronos_local::wire_limits();
    let mut adapter = AdmissionAdapter::new(
        ProcessPort {
            directory: directory.clone(),
            lose_execute_reply: true,
            lost_stream: None,
        },
        limits,
    )
    .unwrap();
    let prepared = adapter.prepare(intent()).unwrap();
    assert_eq!(prepared.serial(), 1);
    assert!(matches!(
        adapter.execute(&prepared),
        Err(AdapterErrorV1::OutcomeUnknown(CallError::OutcomeUnknown))
    ));
    assert_child(first, "first Chronos");
    drop(adapter);
    assert!(chronos_local::verify_socket(&directory).is_err());

    let mut recovered = spawn_helper(&directory, 5);
    wait_for_socket(&directory, &mut recovered);
    let mut client = LocalClient::new(&directory);
    let original = wire::Reference {
        task: intent().task_id,
        incarnation: "0".into(),
    };
    let revision = |value: &str| wire::Revision::Finite(value.into());
    assert!(matches!(
        client
            .request(wire::Request {
                version: 1,
                command: wire::Command::Cancel {
                    expected: revision("1"),
                    task: original.clone(),
                }
            })
            .unwrap()
            .outcome,
        wire::Outcome::Applied {}
    ));
    assert!(matches!(
        client
            .request_v2(wire::coordination_v2::Request {
                version: 2,
                command: wire::coordination_v2::Command::Retire {
                    expected: revision("2"),
                    task: original
                },
            })
            .unwrap()
            .outcome,
        wire::coordination_v2::Outcome::Applied {}
    ));
    assert!(matches!(
        client
            .request(wire::Request {
                version: 1,
                command: wire::Command::SubmitGraph {
                    expected: revision("3"),
                    tasks: vec![wire::TaskSpec {
                        id: intent().task_id,
                        priority: 0,
                        dependencies: vec![],
                        capabilities: vec![],
                        max_attempts: 1,
                        retry_safety: wire::RetrySafety::FenceUnknown,
                    }],
                }
            })
            .unwrap()
            .outcome,
        wire::Outcome::Applied {}
    ));
    let restored =
        PreparedAdmissionV1::restore(prepared.intent().clone(), prepared.serial(), limits).unwrap();
    let mut adapter = AdmissionAdapter::new(
        ProcessPort {
            directory: directory.clone(),
            lose_execute_reply: false,
            lost_stream: None,
        },
        limits,
    )
    .unwrap();
    assert_eq!(
        adapter.reconcile(&restored).unwrap(),
        AdmissionReplyV1::Admitted(AdmissionReceiptV1 {
            effect: EffectIdV1([0x11; 16]),
            task: TaskRefV1 {
                external_task_key: [0x21; 16],
                incarnation: 0,
            },
            chronos_revision: 1,
        })
    );
    drop(adapter);
    assert_child(recovered, "recovered Chronos");
    assert!(chronos_local::verify_socket(&directory).is_err());
    fs::remove_dir_all(directory).unwrap();
}
