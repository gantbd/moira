#![cfg(unix)]

use moira_core::persistence::CodecLimits;
use moira_local_daemon::unix::{Authenticator, Daemon};
use moira_protocol::{CommandV1, MessageRefV1, MutationV1, OutcomeV1, ParticipantRefV1, ReplyV1};
use moira_service::{AccessV1, Service};
use moira_sqlite::{SqliteStore, StoreLimits, Timing};
use moira_wire::{LimitsV1, decode_ready, decode_reply, encode_request};
use std::{
    convert::Infallible,
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

struct Authentication;
impl Authenticator for Authentication {
    type Error = Infallible;
    fn authenticate(&mut self, _: &UnixStream) -> Result<AccessV1, Self::Error> {
        Ok(AccessV1 {
            read: true,
            mutate: true,
        })
    }
}

fn frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut header = [0; 4];
    stream.read_exact(&mut header).unwrap();
    let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut bytes).unwrap();
    bytes
}
fn send(stream: &mut UnixStream, command: CommandV1) -> ReplyV1 {
    let bytes = encode_request(&command, LimitsV1::default()).unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
    decode_reply(&frame(stream), LimitsV1::default()).unwrap()
}
fn path(kind: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "moira-{kind}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn store_limits() -> StoreLimits {
    StoreLimits {
        codec: CodecLimits {
            max_bytes: 8192,
            max_workflows: 8,
        },
        max_database_pages: 64,
        timing: Timing::Off,
    }
}

fn admission(expected_revision: u64) -> CommandV1 {
    CommandV1::Apply {
        expected_revision,
        mutation: MutationV1::AdmitMessage {
            message: MessageRefV1 {
                source: [1; 16],
                room_sequence: 2,
                message_sequence: 3,
            },
            recipient: ParticipantRefV1([4; 16]),
        },
    }
}

#[test]
fn private_socket_authenticates_and_runs_mutation_then_read_on_one_connection() {
    let database = path("service.db");
    let socket = path("service.sock");
    let store = SqliteStore::open(&database, [7; 16], store_limits()).unwrap();
    let mut daemon = Daemon::bind_new(
        &socket,
        Service::new(store),
        Authentication,
        LimitsV1::default(),
        Duration::from_secs(2),
        8,
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let client_socket = socket.clone();
    let client = thread::spawn(move || {
        let mut stream = UnixStream::connect(client_socket).unwrap();
        let ready = decode_ready(&frame(&mut stream), LimitsV1::default()).unwrap();
        assert_eq!(ready.protocol_version, 1);
        let accepted = send(&mut stream, admission(0));
        let status = send(&mut stream, CommandV1::Status);
        drop(stream);
        (accepted, status)
    });
    daemon.serve_next().unwrap();
    let (accepted, status) = client.join().unwrap();
    assert!(matches!(
        accepted,
        ReplyV1::Accepted(OutcomeV1::Applied {
            replayed: false,
            ..
        })
    ));
    assert!(matches!(
        status,
        ReplyV1::Accepted(OutcomeV1::Status(value)) if value.revision == 1
    ));
    assert_eq!(daemon.health().accepted_connections, 1);
    drop(daemon);
    assert!(!socket.exists());

    let reopened = SqliteStore::open(&database, [7; 16], store_limits()).unwrap();
    let mut daemon = Daemon::bind_new(
        &socket,
        Service::new(reopened),
        Authentication,
        LimitsV1::default(),
        Duration::from_secs(2),
        2,
    )
    .unwrap();
    let replay_socket = socket.clone();
    let replay = thread::spawn(move || {
        let mut stream = UnixStream::connect(replay_socket).unwrap();
        decode_ready(&frame(&mut stream), LimitsV1::default()).unwrap();
        let reply = send(&mut stream, admission(1));
        drop(stream);
        reply
    });
    daemon.serve_next().unwrap();
    assert!(matches!(
        replay.join().unwrap(),
        ReplyV1::Accepted(OutcomeV1::Applied { replayed: true, .. })
    ));
    drop(daemon);
    assert!(!socket.exists());
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", database.display(), suffix));
    }
}

#[test]
fn exact_attempt_socket_returns_retained_receipt_and_enforces_disclosure() {
    use moira_core::ChronosTaskRefV1;
    use moira_protocol::{
        AdmissionReceiptV1, AttemptReceiptV1, TaskRefV1, TransitionV1, WorkflowIdV1,
        attempt_lookup::{AttemptLookup, AttemptLookupRequestV1, ExecutionEvidence},
    };
    use moira_service::attempt_lookup::{AttemptLookupPolicy, ExactAttempt};
    use moira_wire::{decode_attempt_reply, encode_attempt_request};

    struct Policy(bool);
    impl AttemptLookupPolicy for Policy {
        fn allows_lookup(&mut self, _: &ExactAttempt<'_>) -> bool {
            true
        }
        fn bind_task(&mut self, exact: &ExactAttempt<'_>) -> Option<ChronosTaskRefV1> {
            (exact.service_scope == "atlas" && exact.task == "review")
                .then(|| ChronosTaskRefV1::new([3; 16], exact.incarnation))
        }
        fn allows_result(&mut self, _: &ExactAttempt<'_>, _: WorkflowIdV1) -> bool {
            self.0
        }
    }

    let database = path("lookup.db");
    let socket = path("lookup.sock");
    let owner = [7; 16];
    let mut service = Service::new(SqliteStore::open(&database, owner, store_limits()).unwrap());
    let ReplyV1::Accepted(OutcomeV1::Applied {
        transition: TransitionV1::ChronosSubmissionPrepared { workflow, effect },
        ..
    }) = service.execute(admission(0)).unwrap()
    else {
        panic!("expected submission");
    };
    let task = TaskRefV1 {
        external_task_key: [3; 16],
        incarnation: 4,
    };
    for (expected_revision, mutation) in [
        (
            1,
            MutationV1::AcceptChronosAdmission {
                workflow,
                receipt: AdmissionReceiptV1 {
                    effect,
                    task,
                    chronos_revision: 1,
                },
            },
        ),
        (
            2,
            MutationV1::AcceptChronosAttempt {
                workflow,
                receipt: AttemptReceiptV1 {
                    task,
                    attempt_number: 1,
                    chronos_revision: 2,
                },
            },
        ),
    ] {
        assert!(matches!(
            service
                .execute(CommandV1::Apply {
                    expected_revision,
                    mutation
                })
                .unwrap(),
            ReplyV1::Accepted(OutcomeV1::Applied { .. })
        ));
    }
    drop(service);
    let mut daemon = Daemon::bind_new(
        &socket,
        Service::new(SqliteStore::open(&database, owner, store_limits()).unwrap()),
        Authentication,
        LimitsV1::default(),
        Duration::from_secs(2),
        3,
    )
    .unwrap();
    let mut requests = 0;
    daemon.set_attempt_lookup_policy(move |_| {
        requests += 1;
        Policy(requests != 2)
    });
    let client = thread::spawn(move || {
        let limits = LimitsV1::default();
        let mut stream = UnixStream::connect(socket).unwrap();
        decode_ready(&frame(&mut stream), limits).unwrap();
        for (attempt, expected) in [
            (
                1,
                AttemptLookup::Found {
                    workflow,
                    execution: ExecutionEvidence::ReceiptPending,
                },
            ),
            (1, AttemptLookup::Denied),
            (2, AttemptLookup::Unresolved),
        ] {
            let bytes = encode_attempt_request(
                &AttemptLookupRequestV1 {
                    service_scope: "atlas".into(),
                    task: "review".into(),
                    incarnation: 4,
                    attempt,
                },
                limits,
            )
            .unwrap();
            stream
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .unwrap();
            stream.write_all(&bytes).unwrap();
            assert_eq!(
                decode_attempt_reply(&frame(&mut stream), limits),
                Ok(expected)
            );
        }
    });
    daemon.serve_next().unwrap();
    client.join().unwrap();
    drop(daemon);
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", database.display(), suffix));
    }
}
