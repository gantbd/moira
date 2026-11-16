#![cfg(unix)]

use moira_core::{
    persistence::CodecLimits,
    saga::{SagaLimitsV1, persistence::SagaCodecLimitsV1},
};
use moira_local_daemon::unix::{Daemon, DaemonErrorV1, SagaDaemon};
use moira_protocol::{
    CommandV1, OperatorActionIdV1, OperatorDispositionV1, OperatorOperationV1, OutcomeV1, ReplyV1,
    SagaCommandV1, SagaIdV1, SagaRefusalV1, SagaReplyV1,
};
use moira_service::{
    AccessV1, Service,
    saga::{OperatorAuthorizer, SagaService},
};
use moira_sqlite::{
    SqliteStore, StoreLimits, Timing,
    saga::{SagaSqliteStore, SagaStoreLimitsV1},
};
use moira_unix_peer::{
    AuthenticationErrorV1, MappingV1, UnixPeerAuthenticatorV1, UnixSagaAuthenticatorV1,
};
use moira_wire::{
    LimitsV1, decode_ready, decode_reply, decode_saga_reply, encode_request, encode_saga_request,
};
use std::{
    convert::Infallible,
    fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

struct DenyOperators;

impl OperatorAuthorizer for DenyOperators {
    type Error = Infallible;

    fn authorize_operator(
        &mut self,
        _: OperatorOperationV1,
    ) -> Result<Option<moira_service::saga::OperatorAccessV1>, Self::Error> {
        Ok(None)
    }
}

fn path(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "moira-peer-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn mapping(uid_offset: u32) -> MappingV1 {
    MappingV1 {
        effective_uid: nix::unistd::geteuid().as_raw().wrapping_add(uid_offset),
        effective_gid: nix::unistd::getegid().as_raw(),
        access: AccessV1 {
            read: true,
            mutate: true,
        },
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

fn send_saga(stream: &mut UnixStream, command: SagaCommandV1) -> SagaReplyV1 {
    let bytes = encode_saga_request(&command, LimitsV1::default()).unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
    decode_saga_reply(&frame(stream), LimitsV1::default()).unwrap()
}

fn coordinator_limits() -> StoreLimits {
    StoreLimits {
        codec: CodecLimits {
            max_bytes: 8192,
            max_workflows: 2,
        },
        max_database_pages: 64,
        timing: Timing::Off,
    }
}

fn saga_limits() -> SagaStoreLimitsV1 {
    SagaStoreLimitsV1 {
        codec: SagaCodecLimitsV1 {
            max_bytes: 8192,
            max_sagas: 2,
            max_operator_actions: 2,
        },
        saga: SagaLimitsV1 {
            max_sagas: 2,
            max_retries_per_saga: 1,
            max_inspection_page: 2,
            max_operator_actions: 2,
        },
        max_database_pages: 64,
    }
}

fn cleanup(database: &Path) {
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", database.display(), suffix));
    }
}

#[test]
fn legacy_daemon_accepts_mapped_kernel_peer_and_refuses_unknown_before_ready() {
    let database = path("legacy.db");
    let socket = path("legacy.sock");
    let store = SqliteStore::open(&database, [1; 16], coordinator_limits()).unwrap();
    let auth = UnixPeerAuthenticatorV1::new(1, vec![mapping(0)]).unwrap();
    let mut daemon = Daemon::bind_new(
        &socket,
        Service::new(store),
        auth,
        LimitsV1::default(),
        Duration::from_secs(2),
        1,
    )
    .unwrap();
    let client_socket = socket.clone();
    let client = thread::spawn(move || {
        let mut stream = UnixStream::connect(client_socket).unwrap();
        decode_ready(&frame(&mut stream), LimitsV1::default()).unwrap();
        send(&mut stream, CommandV1::Status)
    });
    daemon.serve_next().unwrap();
    assert!(matches!(
        client.join().unwrap(),
        ReplyV1::Accepted(OutcomeV1::Status(_))
    ));
    drop(daemon);

    let store = SqliteStore::open(&database, [1; 16], coordinator_limits()).unwrap();
    let auth = UnixPeerAuthenticatorV1::new(1, vec![mapping(1)]).unwrap();
    let mut daemon = Daemon::bind_new(
        &socket,
        Service::new(store),
        auth,
        LimitsV1::default(),
        Duration::from_secs(2),
        1,
    )
    .unwrap();
    let client_socket = socket.clone();
    let client = thread::spawn(move || {
        let mut stream = UnixStream::connect(client_socket).unwrap();
        let mut byte = [0];
        stream.read(&mut byte).unwrap() == 0
    });
    assert!(matches!(
        daemon.serve_next(),
        Err(DaemonErrorV1::Authentication(
            AuthenticationErrorV1::UnknownPeer
        ))
    ));
    assert!(client.join().unwrap());
    drop(daemon);
    cleanup(&database);
}

#[test]
fn saga_daemon_uses_peer_access_but_keeps_operator_authority_separate() {
    let database = path("saga.db");
    let socket = path("saga.sock");
    let store = SagaSqliteStore::open(&database, [2; 16], saga_limits()).unwrap();
    let peer = UnixPeerAuthenticatorV1::new(1, vec![mapping(0)]).unwrap();
    let auth = UnixSagaAuthenticatorV1::new(peer, DenyOperators);
    let mut daemon = SagaDaemon::bind_new(
        &socket,
        SagaService::new(store),
        auth,
        LimitsV1::default(),
        Duration::from_secs(2),
        2,
    )
    .unwrap();
    let client_socket = socket.clone();
    let client = thread::spawn(move || {
        let mut stream = UnixStream::connect(client_socket).unwrap();
        decode_ready(&frame(&mut stream), LimitsV1::default()).unwrap();
        let status = send_saga(&mut stream, SagaCommandV1::Status);
        let reconcile = send_saga(
            &mut stream,
            SagaCommandV1::Reconcile {
                expected_revision: 0,
                operation: OperatorOperationV1 {
                    action: OperatorActionIdV1([3; 16]),
                    saga: SagaIdV1(1),
                    disposition: OperatorDispositionV1::KeepUncertain,
                },
            },
        );
        (status, reconcile)
    });
    daemon.serve_next().unwrap();
    let (status, reconcile) = client.join().unwrap();
    assert!(matches!(status, SagaReplyV1::Accepted(_)));
    assert_eq!(
        reconcile,
        SagaReplyV1::Refused(SagaRefusalV1::NotAuthorized)
    );
    drop(daemon);
    cleanup(&database);
}
