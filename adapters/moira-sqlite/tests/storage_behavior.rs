#![cfg(unix)]
use moira_core::persistence::CodecLimits;
use moira_core::*;
use moira_sqlite::{SqliteStore, StoreError, StoreLimits, StoreRevision, Timing};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "moira-sqlite-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> PathBuf {
        self.0.join("state.db")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["state.db", "state.db-journal", "state.db.owner.lock"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}
fn limits() -> StoreLimits {
    StoreLimits {
        codec: CodecLimits {
            max_bytes: 8192,
            max_workflows: 2,
        },
        max_database_pages: 64,
        timing: Timing::Off,
    }
}
fn command() -> CommandV1 {
    CommandV1::AdmitMessage {
        message: AgoraMessageRefV1::new([1; 16], 2, 3),
        recipient: ParticipantRefV1::new([4; 16]),
    }
}

#[test]
fn real_database_reopens_original_effect_identity_and_refuses_stale_commands() {
    let f = Fixture::new();
    let mut s = SqliteStore::open(f.path(), [7; 16], limits()).unwrap();
    assert!(s.load().unwrap().is_none());
    let first = s.apply(WorkflowRevision(0), command()).unwrap();
    assert_eq!(first.revision, StoreRevision(0));
    assert!(matches!(
        s.apply(WorkflowRevision(0), command()),
        Err(StoreError::Refusal(RefusalV1::RevisionConflict { .. }))
    ));
    let replay = s.apply(WorkflowRevision(1), command()).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.revision, first.revision);
    assert_eq!(s.usage().committed, 1);
    assert_eq!(s.usage().commit_service_total, None);
    drop(s);
    let mut s = SqliteStore::open(f.path(), [7; 16], limits()).unwrap();
    let loaded = s.load().unwrap().unwrap();
    assert_eq!(loaded.revision, StoreRevision(0));
    let mut e = Engine::recover(loaded.checkpoint).unwrap();
    assert_eq!(e.apply(e.revision(), command()).unwrap(), first.transition);
    assert_eq!(e.revision(), WorkflowRevision(1));
}

#[test]
fn one_active_owner_refuses_hardlink_aliases_and_owner_identity_survives_restart() {
    let f = Fixture::new();
    let s = SqliteStore::open(f.path(), [7; 16], limits()).unwrap();
    assert!(matches!(
        SqliteStore::open(f.path(), [7; 16], limits()),
        Err(StoreError::Busy)
    ));
    let alias = f.0.join("alias.db");
    fs::hard_link(f.path(), &alias).unwrap();
    assert!(matches!(
        SqliteStore::open(&alias, [7; 16], limits()),
        Err(StoreError::InvalidConfig)
    ));
    fs::remove_file(alias).unwrap();
    drop(s);
    assert!(matches!(
        SqliteStore::open(f.path(), [8; 16], limits()),
        Err(StoreError::OwnerMismatch)
    ));
    assert!(SqliteStore::open(f.path(), [7; 16], limits()).is_ok());
}

#[test]
fn foreign_database_is_not_adopted_or_overwritten_and_limits_preserve_prior_state() {
    let f = Fixture::new();
    let c = rusqlite::Connection::open(f.path()).unwrap();
    c.execute_batch("CREATE TABLE unrelated (value TEXT); INSERT INTO unrelated VALUES ('keep')")
        .unwrap();
    drop(c);
    let before = fs::read(f.path()).unwrap();
    assert!(matches!(
        SqliteStore::open(f.path(), [7; 16], limits()),
        Err(StoreError::ForeignDatabase)
    ));
    assert_eq!(fs::read(f.path()).unwrap(), before);
    fs::remove_file(f.path()).unwrap();
    let mut small = limits();
    small.codec.max_bytes = 64;
    let mut s = SqliteStore::open(f.path(), [7; 16], small).unwrap();
    assert!(matches!(
        s.apply(WorkflowRevision(0), command()),
        Err(StoreError::Codec(_))
    ));
    assert!(s.load().unwrap().is_none());
    assert_eq!(s.revision(), WorkflowRevision(0));
}

#[test]
fn process_exit_without_drop_preserves_a_committed_checkpoint_and_releases_ownership() {
    if std::env::var_os("MOIRA_SQLITE_CHILD_DB").is_some() {
        return;
    }
    let f = Fixture::new();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_commits_and_exits_without_drop"])
        .env("MOIRA_SQLITE_CHILD_DB", f.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "child did not commit successfully");
    let mut s = SqliteStore::open(f.path(), [7; 16], limits()).unwrap();
    let loaded = s.load().unwrap().unwrap();
    let mut e = Engine::recover(loaded.checkpoint).unwrap();
    assert!(e.apply(e.revision(), command()).is_ok());
    assert_eq!(e.revision(), WorkflowRevision(1));
}

#[test]
fn child_commits_and_exits_without_drop() {
    let Some(path) = std::env::var_os("MOIRA_SQLITE_CHILD_DB") else {
        return;
    };
    let mut s = SqliteStore::open(PathBuf::from(path), [7; 16], limits()).unwrap();
    s.apply(WorkflowRevision(0), command()).unwrap();
    std::process::exit(0);
}

fn committed_then_reopened(f: &Fixture, command: CommandV1) -> moira_sqlite::Applied {
    let mut store = SqliteStore::open(f.path(), [7; 16], limits()).unwrap();
    let result = store.apply(store.revision(), command).unwrap();
    assert!(!result.replayed);
    let domain_revision = store.revision();
    drop(store);
    let mut store = SqliteStore::open(f.path(), [7; 16], limits()).unwrap();
    assert_eq!(store.revision(), domain_revision);
    let replay = store.apply(store.revision(), command).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.transition, result.transition);
    assert_eq!(replay.revision, result.revision);
    result
}

#[test]
fn every_workflow_boundary_and_harness_disposition_survives_database_reopen() {
    for disposition in [
        DaimonDispositionV1::Completed,
        DaimonDispositionV1::Failed,
        DaimonDispositionV1::Cancelled,
        DaimonDispositionV1::TimedOut,
        DaimonDispositionV1::Uncertain,
    ] {
        let f = Fixture::new();
        let TransitionV1::ChronosSubmissionPrepared {
            workflow,
            effect_id,
        } = committed_then_reopened(&f, command()).transition
        else {
            panic!("submission")
        };
        let task = ChronosTaskRefV1::new([3; 16], 4);
        committed_then_reopened(
            &f,
            CommandV1::AcceptChronosAdmission {
                workflow,
                receipt: ChronosAdmissionReceiptV1::new(effect_id, task, 1),
            },
        );
        let TransitionV1::DaimonExecutionPrepared { effect_id, .. } = committed_then_reopened(
            &f,
            CommandV1::AcceptChronosAttempt {
                workflow,
                receipt: ChronosAttemptReceiptV1::new(task, 1, 3),
            },
        )
        .transition
        else {
            panic!("execution")
        };
        committed_then_reopened(
            &f,
            CommandV1::AcceptDaimonResult {
                workflow,
                receipt: DaimonResultReceiptV1::new(effect_id, [7; 16], task, 1, disposition),
            },
        );
        if disposition == DaimonDispositionV1::Uncertain {
            let mut s = SqliteStore::open(f.path(), [7; 16], limits()).unwrap();
            assert!(
                s.apply(s.revision(), CommandV1::PrepareAgoraResponse { workflow })
                    .is_err()
            );
            let loaded = s.load().unwrap().unwrap();
            let e = Engine::recover(loaded.checkpoint).unwrap();
            assert_eq!(
                e.workflow(&workflow).unwrap().phase(),
                WorkflowPhaseV1::FencedUncertain
            );
        } else {
            let TransitionV1::AgoraResponsePrepared { effect_id, .. } =
                committed_then_reopened(&f, CommandV1::PrepareAgoraResponse { workflow })
                    .transition
            else {
                panic!("publication")
            };
            committed_then_reopened(
                &f,
                CommandV1::AcceptAgoraPublication {
                    workflow,
                    receipt: AgoraPublicationReceiptV1::new(
                        effect_id,
                        AgoraMessageRefV1::new([8; 16], 9, 10),
                        11,
                    ),
                },
            );
            let mut s = SqliteStore::open(f.path(), [7; 16], limits()).unwrap();
            let loaded = s.load().unwrap().unwrap();
            let e = Engine::recover(loaded.checkpoint).unwrap();
            assert_eq!(
                e.workflow(&workflow).unwrap().phase(),
                WorkflowPhaseV1::Completed
            );
        }
    }
}

#[test]
fn process_loss_during_uncommitted_write_retains_previous_checkpoint() {
    let f = Fixture::new();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_exits_during_uncommitted_write"])
        .env("MOIRA_SQLITE_UNCOMMITTED_DB", f.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "uncommitted child failed");
    let mut s = SqliteStore::open(f.path(), [7; 16], limits()).unwrap();
    assert_eq!(s.revision(), WorkflowRevision(1));
    assert_eq!(s.load().unwrap().unwrap().revision, StoreRevision(0));
    assert!(s.apply(s.revision(), command()).unwrap().replayed);
}

#[test]
fn child_exits_during_uncommitted_write() {
    let Some(path) = std::env::var_os("MOIRA_SQLITE_UNCOMMITTED_DB") else {
        return;
    };
    let path = PathBuf::from(path);
    let mut s = SqliteStore::open(&path, [7; 16], limits()).unwrap();
    s.apply(WorkflowRevision(0), command()).unwrap();
    // Deliberately abandon a real SQLite transaction, bypassing Rust Drop.
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch(
        "PRAGMA synchronous=EXTRA; BEGIN IMMEDIATE; UPDATE moira_state SET payload=x'00';",
    )
    .unwrap();
    let mut journal = path.as_os_str().to_os_string();
    journal.push("-journal");
    assert!(PathBuf::from(journal).exists());
    std::process::exit(0);
}
