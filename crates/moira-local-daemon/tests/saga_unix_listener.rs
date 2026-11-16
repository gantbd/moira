#![cfg(unix)]

use moira_core::saga::{SagaLimitsV1, persistence::SagaCodecLimitsV1};
use moira_local_daemon::unix::{SagaAuthenticator, SagaDaemon};
use moira_protocol::{
    OperatorActionIdV1, OperatorDispositionV1, OperatorOperationV1, SagaCommandV1,
    SagaEffectOutcomeV1, SagaMutationV1, SagaOutcomeV1, SagaRefusalV1, SagaReplyV1, SagaSourceV1,
    SagaTransitionV1,
};
use moira_service::{
    AccessV1,
    saga::{OperatorAccessV1, SagaService},
};
use moira_sqlite::saga::{SagaSqliteStore, SagaStoreLimitsV1};
use moira_wire::{LimitsV1, decode_ready, decode_saga_reply, encode_saga_request};
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

struct Authentication {
    allow_operator: bool,
}

impl SagaAuthenticator for Authentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AccessV1, Self::Error> {
        Ok(AccessV1 {
            read: true,
            mutate: true,
        })
    }

    fn authorize_operator(
        &mut self,
        _: OperatorOperationV1,
    ) -> Result<Option<OperatorAccessV1>, Self::Error> {
        Ok(self.allow_operator.then_some(OperatorAccessV1 {
            subject: [3; 16],
            authority_decision: 4,
            authority_epoch: 5,
        }))
    }
}

fn path(kind: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "moira-saga-{kind}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn limits() -> SagaStoreLimitsV1 {
    SagaStoreLimitsV1 {
        codec: SagaCodecLimitsV1 {
            max_bytes: 16_384,
            max_sagas: 8,
            max_operator_actions: 16,
        },
        saga: SagaLimitsV1 {
            max_sagas: 8,
            max_retries_per_saga: 2,
            max_inspection_page: 4,
            max_operator_actions: 16,
        },
        max_database_pages: 64,
    }
}

fn frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut header = [0; 4];
    stream.read_exact(&mut header).unwrap();
    let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut bytes).unwrap();
    bytes
}

fn send(stream: &mut UnixStream, command: SagaCommandV1) -> SagaReplyV1 {
    let bytes = encode_saga_request(&command, LimitsV1::default()).unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
    decode_saga_reply(&frame(stream), LimitsV1::default()).unwrap()
}

#[test]
fn private_saga_socket_reconciles_and_replays_after_sqlite_daemon_reopen() {
    let database = path("service.db");
    let socket = path("service.sock");
    let owner = [8; 16];
    let admission = SagaMutationV1::Admit {
        source: SagaSourceV1([2; 16]),
        now: 10,
        deadline: 20,
        retry_budget: 2,
    };
    let store = SagaSqliteStore::open(&database, owner, limits()).unwrap();
    let mut daemon = SagaDaemon::bind_new(
        &socket,
        SagaService::new(store),
        Authentication {
            allow_operator: true,
        },
        LimitsV1::default(),
        Duration::from_secs(2),
        8,
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let action = OperatorActionIdV1([6; 16]);
    let client_socket = socket.clone();
    let first = thread::spawn(move || {
        let mut stream = UnixStream::connect(client_socket).unwrap();
        assert_eq!(
            decode_ready(&frame(&mut stream), LimitsV1::default())
                .unwrap()
                .protocol_version,
            1
        );
        let SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            transition: SagaTransitionV1::Admitted { saga, effect, .. },
            replayed: false,
            ..
        }) = send(
            &mut stream,
            SagaCommandV1::Apply {
                expected_revision: 0,
                mutation: admission,
            },
        )
        else {
            panic!("admission expected")
        };
        send(
            &mut stream,
            SagaCommandV1::Apply {
                expected_revision: 1,
                mutation: SagaMutationV1::AcceptEffect {
                    saga,
                    effect,
                    outcome: SagaEffectOutcomeV1::Uncertain,
                    observed_at: 11,
                },
            },
        );
        let operation = OperatorOperationV1 {
            action,
            saga,
            disposition: OperatorDispositionV1::KeepUncertain,
        };
        let reconciled = send(
            &mut stream,
            SagaCommandV1::Reconcile {
                expected_revision: 2,
                operation,
            },
        );
        let exported = send(&mut stream, SagaCommandV1::OperatorReceipt { action });
        drop(stream);
        (operation, reconciled, exported)
    });
    daemon.serve_next().unwrap();
    let (operation, reconciled, exported) = first.join().unwrap();
    assert!(matches!(
        reconciled,
        SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            replayed: false,
            transition: SagaTransitionV1::Reconciled { .. },
            ..
        })
    ));
    assert!(matches!(
        exported,
        SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipt(Some(receipt)))
            if receipt.authority_decision == 4 && receipt.authority_epoch == 5
    ));
    assert_eq!(daemon.health().accepted_connections, 1);
    drop(daemon);
    assert!(!socket.exists());

    let reopened = SagaSqliteStore::open(&database, owner, limits()).unwrap();
    let mut daemon = SagaDaemon::bind_new(
        &socket,
        SagaService::new(reopened),
        Authentication {
            allow_operator: true,
        },
        LimitsV1::default(),
        Duration::from_secs(2),
        2,
    )
    .unwrap();
    let replay_socket = socket.clone();
    let replay = thread::spawn(move || {
        let mut stream = UnixStream::connect(replay_socket).unwrap();
        decode_ready(&frame(&mut stream), LimitsV1::default()).unwrap();
        let reply = send(
            &mut stream,
            SagaCommandV1::Reconcile {
                expected_revision: 2,
                operation,
            },
        );
        drop(stream);
        reply
    });
    daemon.serve_next().unwrap();
    assert!(matches!(
        replay.join().unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Applied { replayed: true, .. })
    ));
    drop(daemon);

    let reopened = SagaSqliteStore::open(&database, owner, limits()).unwrap();
    let mut daemon = SagaDaemon::bind_new(
        &socket,
        SagaService::new(reopened),
        Authentication {
            allow_operator: false,
        },
        LimitsV1::default(),
        Duration::from_secs(2),
        2,
    )
    .unwrap();
    let denied_socket = socket.clone();
    let denied = thread::spawn(move || {
        let mut stream = UnixStream::connect(denied_socket).unwrap();
        decode_ready(&frame(&mut stream), LimitsV1::default()).unwrap();
        let reply = send(
            &mut stream,
            SagaCommandV1::Reconcile {
                expected_revision: 3,
                operation: OperatorOperationV1 {
                    action: OperatorActionIdV1([7; 16]),
                    ..operation
                },
            },
        );
        let status = send(&mut stream, SagaCommandV1::Status);
        drop(stream);
        (reply, status)
    });
    daemon.serve_next().unwrap();
    let (denied, status) = denied.join().unwrap();
    assert_eq!(denied, SagaReplyV1::Refused(SagaRefusalV1::NotAuthorized));
    assert!(matches!(
        status,
        SagaReplyV1::Accepted(SagaOutcomeV1::Status(value)) if value.revision == 3
    ));
    drop(daemon);
    assert!(!socket.exists());
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", database.display(), suffix));
    }
}
