#![cfg(unix)]

use moira_core::{
    AgoraMessageRefV1, CommandV1, Engine, ParticipantRefV1,
    persistence::CodecLimits,
    saga::{
        EffectOutcomeV1, SagaCommandV1, SagaLimitsV1, SagaTransitionV1,
        persistence::SagaCodecLimitsV1,
    },
};
use moira_sqlite::{
    SqliteStore, StoreLimits, Timing,
    saga::{SagaSqliteStore, SagaStoreLimitsV1},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn from_sql(name: &str, source: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "moira-compatibility-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join(name);
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch(source).unwrap();
        drop(connection);
        Self { directory, path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-journal", ".owner.lock"] {
            let mut path = self.path.as_os_str().to_os_string();
            path.push(suffix);
            let _ = fs::remove_file(PathBuf::from(path));
        }
        let _ = fs::remove_dir(&self.directory);
    }
}

#[test]
fn external_legacy_database_fixture_reopens_and_replays_exact_admission() {
    let fixture = Fixture::from_sql(
        "legacy.db",
        include_str!("../../../tests/fixtures/sqlite-legacy-v1-live.sql"),
    );
    let mut store = SqliteStore::open(
        &fixture.path,
        [7; 16],
        StoreLimits {
            codec: CodecLimits {
                max_bytes: 8_192,
                max_workflows: 2,
            },
            max_database_pages: 64,
            timing: Timing::Off,
        },
    )
    .unwrap();
    let retained = store.load().unwrap().unwrap();
    assert_eq!(retained.revision.0, 0);
    let mut engine = Engine::recover(retained.checkpoint).unwrap();
    let command = CommandV1::AdmitMessage {
        message: AgoraMessageRefV1::new([1; 16], 2, 3),
        recipient: ParticipantRefV1::new([4; 16]),
    };
    let transition = engine.apply(engine.revision(), command).unwrap();
    let replay = store.apply(store.revision(), command).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.transition, transition);
}

#[test]
fn external_saga_database_fixture_reopens_and_replays_exact_history() {
    let fixture = Fixture::from_sql(
        "saga.db",
        include_str!("../../../tests/fixtures/sqlite-saga-v1-live.sql"),
    );
    let limits = SagaLimitsV1 {
        max_sagas: 4,
        max_retries_per_saga: 2,
        max_inspection_page: 2,
        max_operator_actions: 8,
    };
    let mut store = SagaSqliteStore::open(
        &fixture.path,
        [9; 16],
        SagaStoreLimitsV1 {
            codec: SagaCodecLimitsV1 {
                max_bytes: 8_192,
                max_sagas: 4,
                max_operator_actions: 8,
            },
            saga: limits,
            max_database_pages: 64,
        },
    )
    .unwrap();
    assert_eq!(store.revision(), 3);

    let command = SagaCommandV1::Admit {
        source: [7; 16],
        now: 10,
        deadline: 20,
        retry_budget: 2,
    };
    let replay = store.apply(3, command).unwrap();
    assert!(replay.replayed);
    let SagaTransitionV1::Admitted { saga, effect, .. } = replay.transition else {
        panic!("admission replay expected")
    };
    let failed = SagaCommandV1::AcceptEffect {
        saga,
        effect,
        outcome: EffectOutcomeV1::Failed,
        observed_at: 11,
    };
    assert!(store.apply(3, failed).unwrap().replayed);
    assert!(
        store
            .apply(
                3,
                SagaCommandV1::Retry {
                    saga,
                    failed_effect: effect,
                    now: 12,
                },
            )
            .unwrap()
            .replayed
    );

    let checkpoint = store.saga(saga).unwrap();
    assert_eq!(checkpoint.retries_used, 1);
}
