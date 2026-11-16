use moira_core::persistence::CodecLimits;
use moira_core::saga::{
    EffectOutcomeV1, SagaCommandV1, SagaLimitsV1, SagaPhaseV1, SagaTransitionV1,
    persistence::SagaCodecLimitsV1,
};
use moira_sqlite::saga::{SagaSqliteStore, SagaStoreLimitsV1};
use moira_sqlite::{SqliteStore, StoreError, StoreLimits, Timing};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "moira-saga-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self { directory }
    }

    fn path(&self) -> PathBuf {
        self.directory.join("saga.db")
    }

    fn open(&self) -> SagaSqliteStore {
        SagaSqliteStore::open(
            self.path(),
            [9; 16],
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
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["saga.db", "saga.db-journal", "saga.db.owner.lock"] {
            let _ = fs::remove_file(self.directory.join(name));
        }
        let _ = fs::remove_dir(&self.directory);
    }
}

#[test]
fn saga_store_reopens_exact_effect_evidence_without_reapplying_progress() {
    let fixture = Fixture::new();
    let admit = SagaCommandV1::Admit {
        source: [1; 16],
        now: 10,
        deadline: 50,
        retry_budget: 2,
    };
    let (saga, effect) = {
        let mut store = fixture.open();
        let admitted = store.apply(0, admit).unwrap();
        let SagaTransitionV1::Admitted { saga, effect, .. } = admitted.transition else {
            panic!("unexpected transition")
        };
        let evidence = SagaCommandV1::AcceptEffect {
            saga,
            effect,
            outcome: EffectOutcomeV1::Succeeded,
            observed_at: 11,
        };
        store.apply(1, evidence).unwrap();
        assert_eq!(store.census().awaiting_decision, 1);
        (saga, effect)
    };

    let mut reopened = fixture.open();
    assert_eq!(reopened.revision(), 2);
    assert_eq!(
        reopened.saga(saga).unwrap().phase,
        SagaPhaseV1::ForwardSucceeded
    );
    let replay = reopened
        .apply(
            2,
            SagaCommandV1::AcceptEffect {
                saga,
                effect,
                outcome: EffectOutcomeV1::Succeeded,
                observed_at: 11,
            },
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.storage_revision.0, 1);
    assert_eq!(reopened.revision(), 2);
}

#[test]
fn saga_store_refuses_profile_drift_and_legacy_coordinator_databases() {
    let fixture = Fixture::new();
    let mut original = fixture.open();
    original
        .apply(
            0,
            SagaCommandV1::Admit {
                source: [3; 16],
                now: 1,
                deadline: 2,
                retry_budget: 1,
            },
        )
        .unwrap();
    drop(original);
    let changed = SagaSqliteStore::open(
        fixture.path(),
        [9; 16],
        SagaStoreLimitsV1 {
            codec: SagaCodecLimitsV1 {
                max_bytes: 16_384,
                max_sagas: 8,
                max_operator_actions: 16,
            },
            saga: SagaLimitsV1 {
                max_sagas: 8,
                max_retries_per_saga: 1,
                max_inspection_page: 4,
                max_operator_actions: 16,
            },
            max_database_pages: 64,
        },
    );
    assert!(matches!(changed, Err(StoreError::InvalidConfig)));

    let legacy_path = fixture.directory.join("legacy.db");
    drop(
        SqliteStore::open(
            &legacy_path,
            [9; 16],
            StoreLimits {
                codec: CodecLimits {
                    max_bytes: 8_192,
                    max_workflows: 2,
                },
                max_database_pages: 64,
                timing: Timing::Off,
            },
        )
        .unwrap(),
    );
    assert!(matches!(
        SagaSqliteStore::open(
            &legacy_path,
            [9; 16],
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
            },
        ),
        Err(StoreError::ForeignDatabase)
    ));
    for name in ["legacy.db", "legacy.db-journal", "legacy.db.owner.lock"] {
        let _ = fs::remove_file(fixture.directory.join(name));
    }
}

#[test]
fn logical_database_mutations_refuse_reopen_without_reinitializing_state() {
    let mutations = [
        "UPDATE saga_meta SET owner=zeroblob(16)",
        "UPDATE saga_meta SET profile=zeroblob(25)",
        "UPDATE saga_meta SET initialized=0",
        "UPDATE saga_state SET domain_revision=zeroblob(8)",
        "UPDATE saga_state SET payload=x'00'",
        "DELETE FROM saga_state",
        "PRAGMA application_id=0",
        "PRAGMA user_version=2",
        "CREATE TABLE unexpected(value INTEGER)",
    ];

    for mutation in mutations {
        let fixture = Fixture::new();
        let mut store = fixture.open();
        store
            .apply(
                0,
                SagaCommandV1::Admit {
                    source: [4; 16],
                    now: 1,
                    deadline: 2,
                    retry_budget: 1,
                },
            )
            .unwrap();
        drop(store);

        let connection = rusqlite::Connection::open(fixture.path()).unwrap();
        connection.execute_batch(mutation).unwrap();
        drop(connection);

        assert!(
            SagaSqliteStore::open(
                fixture.path(),
                [9; 16],
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
                },
            )
            .is_err(),
            "logical database mutation reopened successfully: {mutation}"
        );
    }
}
