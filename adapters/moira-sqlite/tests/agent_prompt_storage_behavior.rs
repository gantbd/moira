use moira_host::AgentPromptStoreV1;
use moira_sqlite::agent_prompt::{
    AgentPromptSqliteStore, AgentPromptStoreErrorV1, AgentPromptStoreLimitsV1,
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
        "moira-agent-prompt-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn limits() -> AgentPromptStoreLimitsV1 {
    AgentPromptStoreLimitsV1 {
        max_entries: 1,
        max_prompt_bytes: 64,
        max_database_pages: 64,
    }
}

#[test]
fn exact_prompt_replays_reopens_and_capacity_refuses() {
    let path = temp_path();
    let owner = [1; 16];
    let record = {
        let mut store = AgentPromptSqliteStore::open(&path, owner, limits()).unwrap();
        let first = store.put(b"ground this answer").unwrap();
        assert!(!first.replayed);
        assert_eq!(first.record.prompt_bytes, 18);
        assert_eq!(
            first.record.prompt_digest,
            <[u8; 32]>::from(Sha256::digest(b"ground this answer"))
        );
        assert!(store.put(b"ground this answer").unwrap().replayed);
        assert!(matches!(
            store.put(b"another prompt"),
            Err(AgentPromptStoreErrorV1::Capacity)
        ));
        first.record
    };
    let mut reopened = AgentPromptSqliteStore::open(&path, owner, limits()).unwrap();
    assert_eq!(
        reopened.get(record.prompt_digest).unwrap().unwrap(),
        b"ground this answer"
    );
    assert_eq!(reopened.get([7; 32]).unwrap(), None);
    assert!(matches!(
        reopened.get([0; 32]),
        Err(AgentPromptStoreErrorV1::Integrity)
    ));
    drop(reopened);
    assert!(matches!(
        AgentPromptSqliteStore::open(&path, [9; 16], limits()),
        Err(AgentPromptStoreErrorV1::OwnerMismatch)
    ));
    let mut drift = limits();
    drift.max_prompt_bytes += 1;
    assert!(matches!(
        AgentPromptSqliteStore::open(&path, owner, drift),
        Err(AgentPromptStoreErrorV1::ProfileMismatch)
    ));
    cleanup(&path);
}

#[test]
fn empty_oversize_and_foreign_database_are_refused() {
    let path = temp_path();
    let owner = [2; 16];
    let mut store = AgentPromptSqliteStore::open(&path, owner, limits()).unwrap();
    assert!(matches!(
        store.put(b""),
        Err(AgentPromptStoreErrorV1::InvalidConfig)
    ));
    assert!(matches!(
        store.put(&[1; 65]),
        Err(AgentPromptStoreErrorV1::InvalidConfig)
    ));
    drop(store);
    cleanup(&path);

    let path = temp_path();
    fs::write(&path, b"not sqlite").unwrap();
    assert!(AgentPromptSqliteStore::open(&path, owner, limits()).is_err());
    cleanup(&path);
}

fn cleanup(path: &PathBuf) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(format!("{}.owner.lock", path.display()));
}
