use moira_host::AgentOutputStoreV1;
use moira_sqlite::agent_output::{
    AgentOutputSqliteStore, AgentOutputStoreErrorV1, AgentOutputStoreLimitsV1,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn temp_path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "moira-agent-output-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn limits() -> AgentOutputStoreLimitsV1 {
    AgentOutputStoreLimitsV1 {
        max_entries: 1,
        max_output_bytes: 8,
        max_page_assistant_bytes: 4,
        max_database_pages: 64,
    }
}

#[test]
fn contiguous_pages_replay_and_reopen_as_exact_composed_output() {
    let path = temp_path();
    let owner = [1; 16];
    let participation = [2; 16];
    let turn = [3; 16];
    {
        let mut store = AgentOutputSqliteStore::open(&path, owner, limits()).unwrap();
        let first = store
            .append(participation, turn, None, 2, [4; 32], b"hel")
            .unwrap();
        assert!(!first.replayed);
        assert_eq!(first.record.last_cursor, 2);
        assert_eq!(
            first.record.output_digest,
            <[u8; 32]>::from(Sha256::digest(b"hel"))
        );
        assert!(
            store
                .append(participation, turn, None, 2, [4; 32], b"hel")
                .unwrap()
                .replayed
        );
        assert!(matches!(
            store.append(participation, turn, None, 2, [4; 32], b"HEX"),
            Err(AgentOutputStoreErrorV1::Conflict)
        ));
        assert!(matches!(
            store.append(participation, turn, Some(3), 4, [5; 32], b"x"),
            Err(AgentOutputStoreErrorV1::Conflict)
        ));
        let second = store
            .append(participation, turn, Some(2), 5, [6; 32], b"lo")
            .unwrap();
        assert!(!second.replayed);
        assert_eq!(second.record.output_bytes, 5);
        assert_eq!(
            second.record.output_digest,
            <[u8; 32]>::from(Sha256::digest(b"hello"))
        );
        assert!(
            store
                .append(participation, turn, Some(2), 5, [6; 32], b"lo")
                .unwrap()
                .replayed
        );
        assert!(matches!(
            store.append(participation, turn, None, 2, [4; 32], b"hel"),
            Err(AgentOutputStoreErrorV1::Conflict)
        ));
        assert!(matches!(
            store.append(participation, [7; 16], None, 0, [8; 32], b"x"),
            Err(AgentOutputStoreErrorV1::Capacity)
        ));
        assert!(matches!(
            store.append(participation, turn, Some(5), 6, [9; 32], b"more"),
            Err(AgentOutputStoreErrorV1::Capacity)
        ));
    }
    let mut reopened = AgentOutputSqliteStore::open(&path, owner, limits()).unwrap();
    let (record, output) = reopened.get(participation, turn).unwrap().unwrap();
    assert_eq!(record.last_cursor, 5);
    assert_eq!(
        record.output_digest,
        <[u8; 32]>::from(Sha256::digest(b"hello"))
    );
    assert_eq!(output, b"hello");
    drop(reopened);
    assert!(matches!(
        AgentOutputSqliteStore::open(&path, [9; 16], limits()),
        Err(AgentOutputStoreErrorV1::OwnerMismatch)
    ));
    let mut drift = limits();
    drift.max_output_bytes += 1;
    assert!(matches!(
        AgentOutputSqliteStore::open(&path, owner, drift),
        Err(AgentOutputStoreErrorV1::ProfileMismatch)
    ));
    cleanup(&path);
}

#[test]
fn invalid_identity_cursor_and_page_bounds_fail_closed() {
    let path = temp_path();
    let mut store = AgentOutputSqliteStore::open(&path, [1; 16], limits()).unwrap();
    for result in [
        store.append([0; 16], [2; 16], None, 0, [3; 32], b""),
        store.append([1; 16], [0; 16], None, 0, [3; 32], b""),
        store.append([1; 16], [2; 16], None, 0, [0; 32], b""),
        store.append([1; 16], [2; 16], Some(1), 1, [3; 32], b""),
        store.append([1; 16], [2; 16], None, 0, [3; 32], b"12345"),
    ] {
        assert!(matches!(
            result,
            Err(AgentOutputStoreErrorV1::InvalidConfig)
        ));
    }
    assert!(matches!(
        store.get([0; 16], [2; 16]),
        Err(AgentOutputStoreErrorV1::Integrity)
    ));
    cleanup(&path);
}

#[test]
fn logically_corrupt_last_page_is_not_replayed_or_returned() {
    let path = temp_path();
    {
        let mut store = AgentOutputSqliteStore::open(&path, [1; 16], limits()).unwrap();
        store
            .append([2; 16], [3; 16], None, 0, [4; 32], b"abc")
            .unwrap();
    }
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE agent_outputs SET last_assistant=?1",
            rusqlite::params![b"xyz".as_slice()],
        )
        .unwrap();
    drop(connection);
    let mut store = AgentOutputSqliteStore::open(&path, [1; 16], limits()).unwrap();
    assert!(matches!(
        store.get([2; 16], [3; 16]),
        Err(AgentOutputStoreErrorV1::Integrity)
    ));
    cleanup(&path);
}

fn cleanup(path: &PathBuf) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(format!("{}.owner.lock", path.display()));
}
