#![cfg(unix)]

use chronos_client::WireClientV1;
use chronos_local::LocalClient;
use chronos_wire::{Command, Outcome, Phase, Request, RetrySafety, Revision, TaskSpec};
use moira_chronos_worker::{
    ClaimAndBindErrorV1, ClaimIntentV1, ConfigurationV1, ErrorV1, JourneyBindingErrorV1,
    ResultBindingV1, SettlementV1, TrustedWorkerV1,
};
use moira_core::{
    ChronosTaskRefV1, agent_participation::ProfileIdV1, task_agent_journey as journey,
};
use moira_host::{
    TaskAgentJourneyApplyErrorV1, TaskAgentJourneyApplyV1, TaskAgentJourneyMutationV1,
    TaskAgentJourneyStore,
};
use moira_protocol::{DispositionV1, EffectIdV1, ResultReceiptV1};
use moira_sqlite::task_agent_journey::{
    TaskAgentJourneySqliteStore, TaskAgentJourneyStoreLimitsV1,
};
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

fn journey_template() -> journey::TemplateV1 {
    journey::TemplateV1 {
        id: journey::TemplateIdV1([2; 16]),
        version: 1,
        digest: [3; 32],
        work_mode: journey::WorkModeV1::Collaborator,
        profile: ProfileIdV1([4; 16]),
        room_policy: journey::RoomPolicyV1::SharedContext,
        budget: journey::BudgetV1 {
            max_context_bytes: 4_096,
            max_output_tokens: 512,
            max_elapsed_millis: 10_000,
        },
        grounding_policy_digest: [5; 32],
        authority_policy_digest: [6; 32],
        consequence_digest: [7; 32],
        retired: false,
    }
}

fn journey_store(directory: &Directory) -> TaskAgentJourneySqliteStore {
    let domain = journey::LimitsV1 {
        max_templates: 2,
        max_previews: 4,
        max_journeys: 4,
        max_preview_ttl: 60,
    };
    TaskAgentJourneySqliteStore::open(
        directory.0.join("task-agent.db"),
        journey::OwnerIdV1([1; 16]),
        TaskAgentJourneyStoreLimitsV1 {
            codec: journey::persistence::CodecLimitsV1 {
                max_bytes: 65_536,
                domain,
            },
            domain,
            max_database_pages: 64,
        },
        [journey_template()],
    )
    .unwrap()
}

fn admitted_journey(
    directory: &Directory,
    external_task_key: [u8; 16],
) -> (TaskAgentJourneySqliteStore, journey::JourneySnapshotV1) {
    let mut store = journey_store(directory);
    let template = journey_template();
    let preview = match store
        .apply(
            Some(0),
            TaskAgentJourneyMutationV1::Preview {
                request: journey::PreviewRequestV1 {
                    operation: journey::PreviewOperationIdV1([8; 16]),
                    template: journey::TemplateRefV1 {
                        id: template.id,
                        version: template.version,
                        digest: template.digest,
                    },
                    task: journey::TaskAdmissionV1 {
                        task: ChronosTaskRefV1::new(external_task_key, 0),
                        chronos_revision: 1,
                        evidence_digest: [9; 32],
                    },
                    profile: template.profile,
                    room_policy: template.room_policy,
                    initiating_principal: journey::PrincipalIdV1([10; 16]),
                    accountable_human: journey::PrincipalIdV1([11; 16]),
                    objective_digest: [12; 32],
                    acceptance_digest: [13; 32],
                    current_evidence_digest: [14; 32],
                    requested_ttl: 30,
                },
                observed_at: 100,
            },
        )
        .unwrap()
        .outcome
    {
        TaskAgentJourneyApplyV1::Preview(receipt) => receipt.preview,
        _ => panic!("preview expected"),
    };
    let admitted = match store
        .apply(
            Some(1),
            TaskAgentJourneyMutationV1::Instantiate {
                request: journey::InstantiateRequestV1 {
                    operation: journey::InstantiateOperationIdV1([15; 16]),
                    preview: preview.id,
                    expected_owner_revision: preview.owner_revision,
                    confirmation: journey::HumanConfirmationV1 {
                        preview: preview.id,
                        accountable_human: preview.request.accountable_human,
                        consequence_digest: preview.consequence_digest,
                    },
                },
                observed_at: 101,
            },
        )
        .unwrap()
        .outcome
    {
        TaskAgentJourneyApplyV1::Instantiate(receipt) => receipt.journey,
        _ => panic!("admission expected"),
    };
    (store, admitted)
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

    let (mut store, admitted) = admitted_journey(&directory, [8; 16]);
    let mut worker = TrustedWorkerV1::new(handle, configuration()).unwrap();
    let ready = worker
        .claim_and_bind(
            &mut store,
            Some(2),
            admitted.id,
            ClaimIntentV1 {
                task_id: "j1-owned-task".into(),
                external_task_key: [8; 16],
                expected_revision: 1,
                now: SystemTime::UNIX_EPOCH,
            },
        )
        .unwrap();
    let claimed = ready.claim;
    assert_eq!(ready.storage_revision, 3);
    assert_eq!(ready.journey.phase, journey::PhaseV1::TaskAttemptReady);
    assert_eq!(ready.journey.task_attempt.unwrap().attempt_number, 1);
    drop(store);
    let reopened = journey_store(&directory);
    assert_eq!(TaskAgentJourneyStore::storage_revision(&reopened), 3);
    assert_eq!(reopened.journey(admitted.id), Some(ready.journey));
    drop(reopened);
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
fn storage_conflict_retains_the_claim_for_bind_only_recovery() {
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
    submit(&mut client, "j1-bind-recovery");
    let (mut store, admitted) = admitted_journey(&directory, [18; 16]);
    let mut worker = TrustedWorkerV1::new(handle, configuration()).unwrap();

    let error = worker
        .claim_and_bind(
            &mut store,
            Some(1),
            admitted.id,
            ClaimIntentV1 {
                task_id: "j1-bind-recovery".into(),
                external_task_key: [18; 16],
                expected_revision: 1,
                now: SystemTime::UNIX_EPOCH,
            },
        )
        .unwrap_err();
    let ClaimAndBindErrorV1::Bind(JourneyBindingErrorV1::Store { claim, error }) = error else {
        panic!("retained-claim storage conflict expected")
    };
    assert!(matches!(
        error,
        TaskAgentJourneyApplyErrorV1::Conflict {
            expected: Some(1),
            actual: 2
        }
    ));
    assert_eq!(worker.active(), Some(claim));
    assert_eq!(TaskAgentJourneyStore::storage_revision(&store), 2);
    assert_eq!(store.journey(admitted.id), Some(admitted));

    let ready = worker
        .bind_active_journey(&mut store, Some(2), admitted.id)
        .unwrap();
    assert_eq!(ready.claim, claim);
    assert_eq!(ready.storage_revision, 3);
    assert!(!ready.replayed);
    assert!(
        worker
            .bind_active_journey(&mut store, Some(3), admitted.id)
            .unwrap()
            .replayed
    );
    assert!(matches!(
        snapshot(&mut client, "j1-bind-recovery"),
        Phase::Running
    ));

    drop(worker);
    drop(store);
    let server = owner.stop();
    assert_eq!(server.signals().worker_requests, 3);
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
