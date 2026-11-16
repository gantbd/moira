use moira_core::tool_continuation::{ToolCapabilityIdV1, ToolExecutionIdV1};
use moira_host::ToolOutputStoreV1;
use moira_sqlite::tool_output::{
    ToolOutputSqliteStore, ToolOutputStoreErrorV1, ToolOutputStoreLimitsV1,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "moira-tool-output-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn limits() -> ToolOutputStoreLimitsV1 {
    ToolOutputStoreLimitsV1 {
        max_entries: 1,
        max_output_bytes: 64,
        max_database_pages: 64,
    }
}

#[test]
fn exact_output_replays_reopens_and_conflicts_or_capacity_refuse() {
    let path = path();
    let owner = [1; 16];
    let record = {
        let mut store = ToolOutputSqliteStore::open(&path, owner, limits()).unwrap();
        let first = store
            .put(
                ToolExecutionIdV1([2; 16]),
                ToolCapabilityIdV1([3; 16]),
                [4; 32],
                b"result",
            )
            .unwrap();
        assert!(!first.replayed);
        assert!(
            store
                .put(
                    ToolExecutionIdV1([2; 16]),
                    ToolCapabilityIdV1([3; 16]),
                    [4; 32],
                    b"result",
                )
                .unwrap()
                .replayed
        );
        assert!(matches!(
            store.put(
                ToolExecutionIdV1([2; 16]),
                ToolCapabilityIdV1([3; 16]),
                [4; 32],
                b"different",
            ),
            Err(ToolOutputStoreErrorV1::Conflict)
        ));
        assert!(matches!(
            store.put(
                ToolExecutionIdV1([5; 16]),
                ToolCapabilityIdV1([3; 16]),
                [4; 32],
                b"second",
            ),
            Err(ToolOutputStoreErrorV1::Capacity)
        ));
        first.record
    };
    let mut reopened = ToolOutputSqliteStore::open(&path, owner, limits()).unwrap();
    assert_eq!(
        reopened.get(record.output_digest).unwrap().unwrap(),
        b"result"
    );
    assert_eq!(
        reopened
            .find(ToolExecutionIdV1([2; 16]))
            .unwrap()
            .unwrap()
            .0,
        record
    );
    drop(reopened);
    assert!(matches!(
        ToolOutputSqliteStore::open(&path, [9; 16], limits()),
        Err(ToolOutputStoreErrorV1::OwnerMismatch)
    ));
    let mut drift = limits();
    drift.max_output_bytes += 1;
    assert!(matches!(
        ToolOutputSqliteStore::open(&path, owner, drift),
        Err(ToolOutputStoreErrorV1::ProfileMismatch)
    ));
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(format!("{}.owner.lock", path.display()));
}
