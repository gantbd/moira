#![cfg(unix)]

use moira_core::{
    AgoraMessageRefV1, CommandV1, ParticipantRefV1, WorkflowRevision,
    persistence::CodecLimits,
    saga::{
        EffectOutcomeV1, SagaCommandV1, SagaLimitsV1, SagaTransitionV1,
        persistence::SagaCodecLimitsV1,
    },
};
use moira_sqlite::{
    OnlineBackupLimitsV1, SqliteStore, StoreError, StoreLimits, Timing,
    saga::{SagaSqliteStore, SagaStoreLimitsV1},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "moira-online-backup-{}-{}",
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

fn backup_limits() -> OnlineBackupLimitsV1 {
    OnlineBackupLimitsV1 {
        pages_per_step: 1,
        max_steps: 128,
        retry_delay: Duration::ZERO,
    }
}

fn legacy_limits() -> StoreLimits {
    StoreLimits {
        codec: CodecLimits {
            max_bytes: 8_192,
            max_workflows: 4,
        },
        max_database_pages: 64,
        timing: Timing::Off,
    }
}

fn saga_limits() -> SagaStoreLimitsV1 {
    SagaStoreLimitsV1 {
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
    }
}

#[test]
fn live_legacy_backup_is_atomically_published_and_source_keeps_progressing() {
    let directory = Directory::new();
    let source = directory.path("legacy.db");
    let snapshot = directory.path("legacy.snapshot");
    let owner = [1; 16];
    let first = CommandV1::AdmitMessage {
        message: AgoraMessageRefV1::new([2; 16], 3, 4),
        recipient: ParticipantRefV1::new([5; 16]),
    };
    let second = CommandV1::AdmitMessage {
        message: AgoraMessageRefV1::new([6; 16], 7, 8),
        recipient: ParticipantRefV1::new([9; 16]),
    };
    let mut store = SqliteStore::open(&source, owner, legacy_limits()).unwrap();
    store.apply(WorkflowRevision(0), first).unwrap();
    let receipt = store.backup_online(&snapshot, backup_limits()).unwrap();
    assert_eq!(receipt.storage_revision.unwrap().0, 0);
    assert!(receipt.page_count > 0);
    assert!(snapshot.exists());
    assert!(!directory.path("legacy.snapshot.partial").exists());
    store.apply(WorkflowRevision(1), second).unwrap();
    assert_eq!(store.revision(), WorkflowRevision(2));

    let mut restored = SqliteStore::open(&snapshot, owner, legacy_limits()).unwrap();
    assert_eq!(restored.revision(), WorkflowRevision(1));
    assert!(restored.apply(WorkflowRevision(1), first).unwrap().replayed);
}

#[test]
fn live_saga_backup_retains_exact_effect_before_source_advances() {
    let directory = Directory::new();
    let source = directory.path("saga.db");
    let snapshot = directory.path("saga.snapshot");
    let owner = [10; 16];
    let admission = SagaCommandV1::Admit {
        source: [11; 16],
        now: 10,
        deadline: 20,
        retry_budget: 2,
    };
    let mut store = SagaSqliteStore::open(&source, owner, saga_limits()).unwrap();
    let admitted = store.apply(0, admission).unwrap();
    let SagaTransitionV1::Admitted { saga, effect, .. } = admitted.transition else {
        panic!("admission expected")
    };
    let receipt = store.backup_online(&snapshot, backup_limits()).unwrap();
    assert_eq!(receipt.storage_revision.unwrap().0, 0);
    store
        .apply(
            1,
            SagaCommandV1::AcceptEffect {
                saga,
                effect,
                outcome: EffectOutcomeV1::Succeeded,
                observed_at: 12,
            },
        )
        .unwrap();
    assert_eq!(store.revision(), 2);

    let mut restored = SagaSqliteStore::open(&snapshot, owner, saga_limits()).unwrap();
    assert_eq!(restored.revision(), 1);
    assert_eq!(restored.saga(saga).unwrap().current_effect, Some(effect));
    assert!(restored.apply(1, admission).unwrap().replayed);
}

#[test]
fn invalid_limits_and_existing_destination_fail_without_partial_publication() {
    let directory = Directory::new();
    let source = directory.path("legacy.db");
    let destination = directory.path("existing.snapshot");
    fs::write(&destination, b"operator-owned").unwrap();
    let mut store = SqliteStore::open(&source, [12; 16], legacy_limits()).unwrap();
    assert!(matches!(
        store.backup_online(&destination, backup_limits()),
        Err(StoreError::BackupExists)
    ));
    assert_eq!(fs::read(&destination).unwrap(), b"operator-owned");
    assert!(matches!(
        store.backup_online(
            directory.path("invalid.snapshot"),
            OnlineBackupLimitsV1 {
                pages_per_step: 0,
                ..backup_limits()
            }
        ),
        Err(StoreError::InvalidConfig)
    ));
    assert!(!directory.path("invalid.snapshot").exists());
    assert!(!directory.path("invalid.snapshot.partial").exists());
    assert!(matches!(
        store.backup_online(
            directory.path("incomplete.snapshot"),
            OnlineBackupLimitsV1 {
                pages_per_step: 1,
                max_steps: 1,
                retry_delay: Duration::ZERO,
            }
        ),
        Err(StoreError::BackupIncomplete)
    ));
    assert!(!directory.path("incomplete.snapshot").exists());
    assert!(!directory.path("incomplete.snapshot.partial").exists());
    assert!(!store.needs_reopen());
}
