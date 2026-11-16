#![cfg(unix)]

use moira_client::unix::{SagaClientLimitsV1, SagaUnixPort};
use moira_client::{SagaMutationClient, SagaReadOnlyClient};
use moira_protocol::{SagaMutationV1, SagaOutcomeV1, SagaReplyV1, SagaSourceV1};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

fn root() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    PathBuf::from("/private/tmp").join(format!(
        "moira-saga-owner-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn write_config(root: &Path) -> (PathBuf, PathBuf) {
    let database = root.join("sagas.db");
    let socket = root.join("sagas.sock");
    let config = root.join("owner.json");
    let document = format!(
        r#"{{
  "database": "{}",
  "socket": "{}",
  "owner": "31313131313131313131313131313131",
  "mappings": [{{
    "effective_uid": {},
    "effective_gid": {},
    "read": true,
    "mutate": true
  }}],
  "io_timeout_ms": 2000,
  "max_requests_per_connection": 8,
  "max_frame_bytes": 65536,
  "max_checkpoint_bytes": 16384,
  "max_database_pages": 64,
  "max_sagas": 8,
  "max_retries_per_saga": 2,
  "max_inspection_page": 4,
  "max_operator_actions": 16
}}"#,
        database.display(),
        socket.display(),
        nix::unistd::geteuid().as_raw(),
        nix::unistd::getegid().as_raw(),
    );
    fs::write(&config, document).unwrap();
    (config, socket)
}

fn start(config: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_moira-saga-local"))
        .arg("--supervised")
        .arg(config)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn connect(child: &mut Child, socket: &Path) -> SagaUnixPort {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(port) = SagaUnixPort::connect(socket, SagaClientLimitsV1::default()) {
            return port;
        }
        if let Some(status) = child.try_wait().unwrap() {
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("Moira saga owner exited before readiness: {status}: {stderr}");
        }
        assert!(Instant::now() < deadline, "Moira saga socket was not ready");
        thread::sleep(Duration::from_millis(10));
    }
}

fn stop(mut child: Child, socket: &Path) {
    child.stdin.take().unwrap().write_all(b"x").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "supervised owner refused shutdown: {status}"
            );
            assert!(!socket.exists(), "owner retained its socket after drain");
            return;
        }
        assert!(Instant::now() < deadline, "supervised owner did not drain");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn supervised_owner_mutates_drains_and_reopens_through_public_client() {
    let root = root();
    fs::create_dir(&root).unwrap();
    let (config, socket) = write_config(&root);

    let mut first = start(&config);
    let port = connect(&mut first, &socket);
    let mut mutation = SagaMutationClient::new(port);
    assert!(matches!(
        mutation.apply(
            0,
            SagaMutationV1::Admit {
                source: SagaSourceV1([7; 16]),
                now: 10,
                deadline: 20,
                retry_budget: 1,
            },
        ),
        Ok(SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            replayed: false,
            ..
        }))
    ));
    drop(mutation);
    stop(first, &socket);

    let mut reopened = start(&config);
    let port = connect(&mut reopened, &socket);
    let mut read = SagaReadOnlyClient::new(port);
    assert!(matches!(
        read.census(),
        Ok(SagaReplyV1::Accepted(SagaOutcomeV1::Census(census))) if census.total == 1
    ));
    drop(read);
    stop(reopened, &socket);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn non_shutdown_byte_does_not_grant_drain_and_signal_still_cleans_up() {
    let root = root();
    fs::create_dir(&root).unwrap();
    let (config, socket) = write_config(&root);
    let mut child = start(&config);
    drop(connect(&mut child, &socket));
    child.stdin.take().unwrap().write_all(b"q").unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(child.try_wait().unwrap().is_none());
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(child.id()).unwrap()),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "signal drain failed: {status}");
            break;
        }
        assert!(Instant::now() < deadline, "signal drain did not finish");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!socket.exists());
    fs::remove_dir_all(root).unwrap();
}
