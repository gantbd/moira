#![cfg(unix)]

use moira_core::{
    AgoraMessageRefV1, CommandV1, ParticipantRefV1, WorkflowRevision,
    persistence::CodecLimits,
    saga::{SagaCommandV1, SagaLimitsV1, SagaTransitionV1, persistence::SagaCodecLimitsV1},
};
use moira_sqlite::{
    SqliteStore, StoreLimits, Timing,
    saga::{SagaSqliteStore, SagaStoreLimitsV1},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "moira-offline-backup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(&self.0);
    }
}

fn offline_copy(source: &Path, backup: &Path, restored: &Path) {
    fs::copy(source, backup).unwrap();
    assert_eq!(fs::read(source).unwrap(), fs::read(backup).unwrap());
    fs::copy(backup, restored).unwrap();
    assert_eq!(fs::read(backup).unwrap(), fs::read(restored).unwrap());
}

#[test]
fn stopped_legacy_store_backup_restores_exact_replay_without_copying_owner_lock() {
    let directory = Directory::new();
    let source = directory.path("legacy.db");
    let backup = directory.path("legacy.backup");
    let restored = directory.path("legacy-restored.db");
    let limits = StoreLimits {
        codec: CodecLimits {
            max_bytes: 8_192,
            max_workflows: 2,
        },
        max_database_pages: 64,
        timing: Timing::Off,
    };
    let command = CommandV1::AdmitMessage {
        message: AgoraMessageRefV1::new([1; 16], 2, 3),
        recipient: ParticipantRefV1::new([4; 16]),
    };
    let original = {
        let mut store = SqliteStore::open(&source, [7; 16], limits).unwrap();
        store.apply(WorkflowRevision(0), command).unwrap()
    };
    drop(SqliteStore::open(&source, [7; 16], limits).unwrap());

    offline_copy(&source, &backup, &restored);
    assert!(directory.path("legacy.db.owner.lock").exists());
    assert!(!directory.path("legacy.backup.owner.lock").exists());
    assert!(!directory.path("legacy-restored.db.owner.lock").exists());

    let mut recovered = SqliteStore::open(&restored, [7; 16], limits).unwrap();
    let replay = recovered.apply(WorkflowRevision(1), command).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.transition, original.transition);
    assert_eq!(replay.revision, original.revision);
}

#[test]
fn stopped_saga_store_backup_restores_effect_identity_and_exact_replay() {
    let directory = Directory::new();
    let source = directory.path("saga.db");
    let backup = directory.path("saga.backup");
    let restored = directory.path("saga-restored.db");
    let limits = SagaStoreLimitsV1 {
        codec: SagaCodecLimitsV1 {
            max_bytes: 8_192,
            max_sagas: 4,
            max_operator_actions: 8,
        },
        saga: SagaLimitsV1 {
            max_sagas: 4,
            max_retries_per_saga: 2,
            max_inspection_page: 2,
            max_operator_actions: 8,
        },
        max_database_pages: 64,
    };
    let command = SagaCommandV1::Admit {
        source: [7; 16],
        now: 10,
        deadline: 20,
        retry_budget: 2,
    };
    let original = {
        let mut store = SagaSqliteStore::open(&source, [9; 16], limits).unwrap();
        store.apply(0, command).unwrap()
    };
    let SagaTransitionV1::Admitted { saga, effect, .. } = original.transition else {
        panic!("admission expected")
    };
    drop(SagaSqliteStore::open(&source, [9; 16], limits).unwrap());

    offline_copy(&source, &backup, &restored);
    assert!(directory.path("saga.db.owner.lock").exists());
    assert!(!directory.path("saga.backup.owner.lock").exists());
    assert!(!directory.path("saga-restored.db.owner.lock").exists());

    let mut recovered = SagaSqliteStore::open(&restored, [9; 16], limits).unwrap();
    let replay = recovered.apply(1, command).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.transition, original.transition);
    let snapshot = recovered.saga(saga).unwrap();
    assert_eq!(snapshot.current_effect, Some(effect));
}
