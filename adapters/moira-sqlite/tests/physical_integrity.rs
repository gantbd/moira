use moira_core::{
    persistence::CodecLimits,
    saga::{SagaLimitsV1, persistence::SagaCodecLimitsV1},
};
use moira_sqlite::{
    SqliteStore, StoreError, StoreLimits, Timing,
    saga::{SagaSqliteStore, SagaStoreLimitsV1},
};
use std::{
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "moira-physical-integrity-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self(directory)
    }

    fn database(&self) -> PathBuf {
        self.0.join("store.db")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["store.db", "store.db-journal", "store.db.owner.lock"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}

fn corrupt_last_page_header(path: &Path) -> Vec<u8> {
    let mut bytes = fs::read(path).unwrap();
    assert!(bytes.len() >= 8_192);
    assert_eq!(bytes.len() % 4_096, 0);
    let offset = bytes.len() - 4_096;
    bytes[offset] ^= 0xff;
    let mut file = OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(offset as u64)).unwrap();
    file.write_all(&bytes[offset..=offset]).unwrap();
    file.sync_all().unwrap();
    bytes
}

fn legacy_limits() -> StoreLimits {
    StoreLimits {
        codec: CodecLimits {
            max_bytes: 16_384,
            max_workflows: 8,
        },
        max_database_pages: 64,
        timing: Timing::Off,
    }
}

fn saga_limits() -> SagaStoreLimitsV1 {
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

#[test]
fn legacy_store_refuses_structural_page_damage_without_rewriting_the_database() {
    let fixture = Fixture::new("legacy");
    let path = fixture.database();
    drop(SqliteStore::open(&path, [1; 16], legacy_limits()).unwrap());
    let corrupted = corrupt_last_page_header(&path);

    assert!(matches!(
        SqliteStore::open(&path, [1; 16], legacy_limits()),
        Err(StoreError::Schema | StoreError::Sqlite(_))
    ));
    assert_eq!(fs::read(path).unwrap(), corrupted);
}

#[test]
fn saga_store_refuses_structural_page_damage_without_rewriting_the_database() {
    let fixture = Fixture::new("saga");
    let path = fixture.database();
    drop(SagaSqliteStore::open(&path, [2; 16], saga_limits()).unwrap());
    let corrupted = corrupt_last_page_header(&path);

    assert!(matches!(
        SagaSqliteStore::open(&path, [2; 16], saga_limits()),
        Err(StoreError::Schema | StoreError::Sqlite(_))
    ));
    assert_eq!(fs::read(path).unwrap(), corrupted);
}
