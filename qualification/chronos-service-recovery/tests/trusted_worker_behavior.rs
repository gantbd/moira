#![cfg(unix)]

use chronos_client::WireClientV1;
use chronos_local::LocalClient;
use chronos_wire::{Command, Outcome, Phase, Request, RetrySafety, Revision, TaskSpec};
use moira_chronos_worker::{
    ClaimIntentV1, ConfigurationV1, ErrorV1, ResultBindingV1, SettlementV1, TrustedWorkerV1,
};
use moira_protocol::{DispositionV1, EffectIdV1, ResultReceiptV1};
use std::{
    io::Write,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread::JoinHandle,
    time::{Duration, SystemTime},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = PathBuf::from(format!(
            "/private/tmp/moira-chronos-worker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        chronos_standalone::ownership::create_private_directory(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

struct RunningServer {
    shutdown: UnixStream,
    owner: Option<JoinHandle<chronos_standalone::Server>>,
}

impl RunningServer {
    fn start(mut server: chronos_standalone::Server) -> Self {
        let (shutdown_read, shutdown_write) = UnixStream::pair().unwrap();
        let owner = std::thread::spawn(move || {
            server.run_with_shutdown(&shutdown_read).unwrap();
            server
        });
        Self {
            shutdown: shutdown_write,
            owner: Some(owner),
        }
    }

    fn stop(mut self) -> chronos_standalone::Server {
        self.shutdown.write_all(&[1]).unwrap();
        self.owner.take().unwrap().join().unwrap()
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.shutdown.write_all(&[1]);
        if let Some(owner) = self.owner.take() {
            let _ = owner.join();
        }
    }
}

fn snapshot(client: &mut LocalClient, task: &str) -> Phase {
    let response = client
        .request(Request {
            version: 1,
            command: Command::Snapshot { task: task.into() },
        })
        .unwrap();
    let Outcome::Snapshot {
        value: Some(entry), ..
    } = response.outcome
    else {
        panic!("snapshot expected")
    };
    entry.snapshot.phase
}

fn submit(client: &mut LocalClient, task: &str) {
    assert!(matches!(
        client
            .request(Request {
                version: 1,
                command: Command::SubmitGraph {
                    expected: Revision::Finite("0".into()),
                    tasks: vec![TaskSpec {
                        id: task.into(),
                        priority: 0,
                        dependencies: vec![],
                        capabilities: vec![],
                        max_attempts: 1,
                        retry_safety: RetrySafety::FenceUnknown,
                    }],
                },
            })
            .unwrap()
            .outcome,
        Outcome::Applied {}
    ));
}

fn configuration() -> ConfigurationV1 {
    ConfigurationV1::new(
        "moira-j1-worker",
        Duration::from_secs(60),
        Duration::from_secs(30),
    )
    .unwrap()
}

#[test]
fn exact_result_settles_the_coordination_admitted_task() {
    let directory = Directory::new();
    let mut server = chronos_standalone::Server::bind_with_options(
        &directory.0,
        chronos_standalone::ServerOptions {
            allow_writes: true,
            atomic_state: true,
            ..Default::default()
        },
    )
    .unwrap();
    let handle = server.trusted_worker().unwrap();
    let owner = RunningServer::start(server);
    let mut client = LocalClient::new(&directory.0);
    submit(&mut client, "j1-owned-task");

    let mut worker = TrustedWorkerV1::new(handle, configuration()).unwrap();
    let claimed = worker
        .claim(ClaimIntentV1 {
            task_id: "j1-owned-task".into(),
            external_task_key: [8; 16],
            expected_revision: 1,
            now: SystemTime::UNIX_EPOCH,
        })
        .unwrap();
    assert_eq!(claimed.receipt.task.external_task_key, [8; 16]);
    assert_eq!(claimed.receipt.task.incarnation, 0);
    assert_eq!(claimed.receipt.attempt_number, 1);
    assert_eq!(claimed.receipt.chronos_revision, 2);
    assert_eq!(worker.active(), Some(claimed));
    assert!(matches!(
        snapshot(&mut client, "j1-owned-task"),
        Phase::Running
    ));

    let binding = ResultBindingV1 {
        effect: EffectIdV1([9; 16]),
        execution_id: [10; 16],
    };
    worker.bind_result(binding).unwrap();
    let exact = ResultReceiptV1 {
        effect: binding.effect,
        execution_id: binding.execution_id,
        task: claimed.receipt.task,
        attempt_number: claimed.receipt.attempt_number,
        disposition: DispositionV1::Completed,
    };
    let mut mismatch = exact;
    mismatch.effect = EffectIdV1([11; 16]);
    assert!(matches!(
        worker.settle(mismatch, SystemTime::UNIX_EPOCH),
        Err(ErrorV1::EvidenceMismatch)
    ));
    let mut unsupported = exact;
    unsupported.disposition = DispositionV1::Failed;
    assert!(matches!(
        worker.settle(unsupported, SystemTime::UNIX_EPOCH),
        Err(ErrorV1::UnsupportedDisposition)
    ));
    assert!(matches!(
        snapshot(&mut client, "j1-owned-task"),
        Phase::Running
    ));
    assert_eq!(
        worker.settle(exact, SystemTime::UNIX_EPOCH).unwrap(),
        SettlementV1::Completed
    );
    assert_eq!(worker.active(), None);
    assert!(matches!(
        snapshot(&mut client, "j1-owned-task"),
        Phase::Succeeded
    ));

    drop(worker);
    let server = owner.stop();
    assert_eq!(server.signals().worker_requests, 5);
}

#[test]
fn uncertain_result_fences_without_a_second_claim() {
    let directory = Directory::new();
    let mut server = chronos_standalone::Server::bind_with_options(
        &directory.0,
        chronos_standalone::ServerOptions {
            allow_writes: true,
            atomic_state: true,
            ..Default::default()
        },
    )
    .unwrap();
    let handle = server.trusted_worker().unwrap();
    let owner = RunningServer::start(server);
    let mut client = LocalClient::new(&directory.0);
    submit(&mut client, "j1-uncertain-task");
    let mut worker = TrustedWorkerV1::new(handle, configuration()).unwrap();
    let claimed = worker
        .claim(ClaimIntentV1 {
            task_id: "j1-uncertain-task".into(),
            external_task_key: [12; 16],
            expected_revision: 1,
            now: SystemTime::UNIX_EPOCH,
        })
        .unwrap();
    let binding = ResultBindingV1 {
        effect: EffectIdV1([13; 16]),
        execution_id: [14; 16],
    };
    worker.bind_result(binding).unwrap();
    assert_eq!(
        worker
            .settle(
                ResultReceiptV1 {
                    effect: binding.effect,
                    execution_id: binding.execution_id,
                    task: claimed.receipt.task,
                    attempt_number: claimed.receipt.attempt_number,
                    disposition: DispositionV1::Uncertain,
                },
                SystemTime::UNIX_EPOCH,
            )
            .unwrap(),
        SettlementV1::FencedUnknown
    );
    assert!(matches!(
        snapshot(&mut client, "j1-uncertain-task"),
        Phase::Uncertain
    ));
    assert!(matches!(
        worker.claim(ClaimIntentV1 {
            task_id: "j1-uncertain-task".into(),
            external_task_key: [12; 16],
            expected_revision: 3,
            now: SystemTime::UNIX_EPOCH,
        }),
        Err(ErrorV1::DomainRefused)
    ));
    drop(worker);
    owner.stop();
}

#[test]
fn invalid_configuration_refuses_before_owning_the_handle() {
    struct Never;
    impl chronos_host::worker::WorkerClientV1 for Never {
        type TransportError = ();
        fn call_worker(
            &mut self,
            _: chronos_host::worker::WorkerRequestV1,
        ) -> Result<chronos_host::worker::WorkerReplyV1, Self::TransportError> {
            panic!("invalid configuration must not call Chronos")
        }
    }
    let mut invalid = configuration();
    invalid.lease_duration = Duration::ZERO;
    assert!(matches!(
        TrustedWorkerV1::new(Never, invalid),
        Err(ErrorV1::InvalidConfiguration)
    ));
}
