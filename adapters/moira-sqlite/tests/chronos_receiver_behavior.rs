#![cfg(unix)]
use chronos_engine::{EngineLimits, StateRevision, TaskSpecV2};
use moira_core::EffectId;
use moira_sqlite::Timing;
use moira_sqlite::chronos::{ChronosReceiver, ReceiverError, ReceiverLimits};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "moira-chronos-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("peer.db")
    }
    fn open(&self) -> ChronosReceiver {
        ChronosReceiver::open(self.path(), [7; 16], limits()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["peer.db", "peer.db-journal", "peer.db.owner.lock"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        for name in ["origin.db", "origin.db-journal", "origin.db.owner.lock"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}
fn limits() -> ReceiverLimits {
    ReceiverLimits {
        engine: EngineLimits {
            max_tasks: 2,
            ..EngineLimits::default()
        },
        max_snapshot_bytes: 16384,
        max_intent_bytes: 2048,
        max_receipts: 2,
        max_database_pages: 64,
        timing: Timing::Off,
    }
}
fn effect() -> EffectId {
    EffectId::new([1; 16])
}

#[test]
fn lost_admission_reply_reopens_original_receipt_without_second_task() {
    let f = Fixture::new();
    let mut r = f.open();
    let first = r
        .admit(effect(), StateRevision::ZERO, TaskSpecV2::new("work"))
        .unwrap();
    assert!(!first.replayed);
    assert_eq!(r.task_count(), 1);
    drop(r);
    let mut r = f.open();
    // A retry may still carry the original optimistic guard. Retained replay
    // precedes guard validation and must not submit another Chronos command.
    let replay = r
        .admit(effect(), StateRevision::ZERO, TaskSpecV2::new("work"))
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.receipt, first.receipt);
    assert_eq!(replay.incarnation, first.incarnation);
    assert_eq!(r.task_count(), 1);
    assert_eq!(r.usage().committed, 0);
    assert_eq!(r.usage().replayed, 1);
}

#[test]
fn canonical_intent_conflict_and_stale_guard_do_not_record_an_admission() {
    let f = Fixture::new();
    let mut r = f.open();
    r.admit(effect(), StateRevision::ZERO, TaskSpecV2::new("work"))
        .unwrap();
    for changed in [
        TaskSpecV2::new("other"),
        TaskSpecV2::new("work").with_priority(1),
        TaskSpecV2::new("work").with_max_attempts(2),
        TaskSpecV2::new("work").requiring("rust"),
        TaskSpecV2::new("work").connector_idempotent(),
        TaskSpecV2::new("work").depends_on("other"),
    ] {
        assert!(matches!(
            r.admit(effect(), StateRevision::ZERO, changed),
            Err(ReceiverError::IntentConflict)
        ));
    }
    assert!(matches!(
        r.admit(
            EffectId::new([2; 16]),
            StateRevision::ZERO,
            TaskSpecV2::new("second")
        ),
        Err(ReceiverError::AdmissionRefused)
    ));
    assert_eq!(r.task_count(), 1);
    assert!(!r.needs_reopen());
    let revision = r.revision();
    assert!(
        r.admit(EffectId::new([2; 16]), revision, TaskSpecV2::new("second"))
            .is_ok()
    );
    assert_eq!(r.task_count(), 2);
}

#[test]
fn receiver_capacity_refuses_new_admission_but_keeps_retained_replay() {
    let f = Fixture::new();
    let mut config = limits();
    config.max_receipts = 1;
    let mut r = ChronosReceiver::open(f.path(), [7; 16], config).unwrap();
    r.admit(effect(), StateRevision::ZERO, TaskSpecV2::new("work"))
        .unwrap();
    assert!(matches!(
        r.admit(
            EffectId::new([2; 16]),
            r.revision(),
            TaskSpecV2::new("second")
        ),
        Err(ReceiverError::Capacity)
    ));
    assert!(
        r.admit(effect(), StateRevision::ZERO, TaskSpecV2::new("work"))
            .unwrap()
            .replayed
    );
    assert_eq!(r.task_count(), 1);
}

#[test]
fn receiver_commit_survives_child_exit_before_origin_observes_receipt() {
    let f = Fixture::new();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_commits_without_acknowledging_origin"])
        .env("MOIRA_CHRONOS_CHILD_DB", f.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "receiver child failed");
    let mut r = f.open();
    assert!(
        r.admit(effect(), StateRevision::ZERO, TaskSpecV2::new("work"))
            .unwrap()
            .replayed
    );
    assert_eq!(r.task_count(), 1);
}

#[test]
fn child_commits_without_acknowledging_origin() {
    let Some(path) = std::env::var_os("MOIRA_CHRONOS_CHILD_DB") else {
        return;
    };
    let mut r = ChronosReceiver::open(PathBuf::from(path), [7; 16], limits()).unwrap();
    r.admit(effect(), StateRevision::ZERO, TaskSpecV2::new("work"))
        .unwrap();
    std::process::exit(0);
}

#[test]
fn durable_origin_and_receiver_close_lost_reply_with_one_exact_task() {
    use moira_core::persistence::CodecLimits;
    use moira_core::{
        AgoraMessageRefV1, CommandV1, Engine, ParticipantRefV1, TransitionV1, WorkflowPhaseV1,
        WorkflowRevision,
    };
    use moira_sqlite::{SqliteStore, StoreLimits};
    let f = Fixture::new();
    let config = StoreLimits {
        codec: CodecLimits {
            max_bytes: 8192,
            max_workflows: 2,
        },
        max_database_pages: 64,
        timing: Timing::Off,
    };
    let origin_path = f.0.join("origin.db");
    let mut origin = SqliteStore::open(&origin_path, [7; 16], config).unwrap();
    let input = CommandV1::AdmitMessage {
        message: AgoraMessageRefV1::new([1; 16], 2, 3),
        recipient: ParticipantRefV1::new([4; 16]),
    };
    let TransitionV1::ChronosSubmissionPrepared {
        workflow,
        effect_id,
    } = origin.apply(WorkflowRevision(0), input).unwrap().transition
    else {
        panic!("submission")
    };
    let mut receiver = f.open();
    let first = receiver
        .admit(effect_id, StateRevision::ZERO, TaskSpecV2::new("work"))
        .unwrap();
    // Lose the reply and both in-memory engines, not the committed records.
    drop(receiver);
    drop(origin);
    let mut origin = SqliteStore::open(&origin_path, [7; 16], config).unwrap();
    let before = origin.load().unwrap().unwrap();
    let e = Engine::recover(before.checkpoint).unwrap();
    assert_eq!(e.pending_effect(&workflow), Some(effect_id));
    assert_eq!(
        e.workflow(&workflow).unwrap().phase(),
        WorkflowPhaseV1::TaskSubmissionPending
    );
    let mut receiver = f.open();
    let retained = receiver
        .admit(effect_id, StateRevision::ZERO, TaskSpecV2::new("work"))
        .unwrap();
    assert!(retained.replayed);
    assert_eq!(retained.receipt, first.receipt);
    origin
        .apply(
            origin.revision(),
            CommandV1::AcceptChronosAdmission {
                workflow,
                receipt: retained.receipt,
            },
        )
        .unwrap();
    drop(origin);
    drop(receiver);
    let mut origin = SqliteStore::open(&origin_path, [7; 16], config).unwrap();
    let e = Engine::recover(origin.load().unwrap().unwrap().checkpoint).unwrap();
    assert_eq!(
        e.workflow(&workflow).unwrap().phase(),
        WorkflowPhaseV1::TaskSubmitted
    );
    assert_eq!(e.revision(), WorkflowRevision(2));
    let mut receiver = f.open();
    assert_eq!(receiver.task_count(), 1);
    let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10);
    let grant = receiver
        .authorize_worker(
            chronos_engine::Agent::new("worker"),
            now,
            std::time::Duration::from_secs(60),
            1,
        )
        .unwrap();
    let attempt = receiver
        .claim(
            effect_id,
            receiver.revision(),
            &grant,
            now,
            std::time::Duration::from_secs(30),
        )
        .unwrap();
    let prepared = origin
        .apply(
            origin.revision(),
            CommandV1::AcceptChronosAttempt {
                workflow,
                receipt: attempt.receipt,
            },
        )
        .unwrap();
    let TransitionV1::DaimonExecutionPrepared {
        effect_id: harness_effect,
        ..
    } = prepared.transition
    else {
        panic!("harness intent")
    };
    drop(origin);
    drop(receiver);
    let mut origin = SqliteStore::open(&origin_path, [7; 16], config).unwrap();
    let e = Engine::recover(origin.load().unwrap().unwrap().checkpoint).unwrap();
    assert_eq!(
        e.workflow(&workflow).unwrap().phase(),
        WorkflowPhaseV1::HarnessPending
    );
    assert_eq!(e.pending_effect(&workflow), Some(harness_effect));
    let mut receiver = f.open();
    let observation = receiver
        .claim(
            effect_id,
            receiver.revision(),
            &grant,
            now,
            std::time::Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(observation.phase, chronos_engine::TaskPhase::Uncertain);
    assert_eq!(observation.receipt, attempt.receipt);
}

#[test]
fn committed_claim_exposes_no_lease_and_restart_retains_only_uncertain_attribution() {
    use chronos_engine::{Agent, TaskPhase};
    use std::time::{Duration, SystemTime};
    let f = Fixture::new();
    let mut r = f.open();
    r.admit(effect(), StateRevision::ZERO, TaskSpecV2::new("work"))
        .unwrap();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    let grant = r
        .authorize_worker(Agent::new("worker"), now, Duration::from_secs(60), 1)
        .unwrap();
    let first = r
        .claim(effect(), r.revision(), &grant, now, Duration::from_secs(30))
        .unwrap();
    assert!(!first.replayed);
    assert_eq!(first.phase, TaskPhase::Running);
    assert_eq!(first.number, 1);
    assert_eq!(r.claim_usage().committed, 1);
    let replay = r
        .claim(
            effect(),
            StateRevision::ZERO,
            &grant,
            now,
            Duration::from_secs(30),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.receipt, first.receipt);
    drop(r);
    let mut r = f.open();
    let recovered = r
        .claim(effect(), r.revision(), &grant, now, Duration::from_secs(30))
        .unwrap();
    assert!(recovered.replayed);
    assert_eq!(recovered.phase, TaskPhase::Uncertain);
    assert_eq!(recovered.number, 1);
    assert_eq!(recovered.receipt, first.receipt);
    assert_eq!(r.claim_usage().committed, 0);
    assert_eq!(r.recovery_usage().newly_fenced_attempts, 1);
    assert_eq!(r.recovery_usage().commit_service, None);
    let stable = r.revision();
    drop(r);
    let mut r = f.open();
    assert_eq!(r.revision(), stable);
    assert_eq!(r.recovery_usage().newly_fenced_attempts, 0);
    assert_eq!(
        r.claim(effect(), stable, &grant, now, Duration::from_secs(30))
            .unwrap()
            .phase,
        TaskPhase::Uncertain
    );
}

#[test]
fn foreign_or_expired_worker_grants_never_claim_a_task_or_consume_an_attempt() {
    use chronos_engine::Agent;
    use std::time::{Duration, SystemTime};
    let f = Fixture::new();
    let mut r = f.open();
    r.admit(effect(), StateRevision::ZERO, TaskSpecV2::new("work"))
        .unwrap();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    let old = r
        .authorize_worker(Agent::new("worker"), now, Duration::from_secs(60), 1)
        .unwrap();
    drop(r);
    let mut r = f.open();
    let revision = r.revision();
    assert!(matches!(
        r.claim(effect(), revision, &old, now, Duration::from_secs(30)),
        Err(ReceiverError::ClaimRefused)
    ));
    assert_eq!(r.revision(), revision);
    let expired = r
        .authorize_worker(Agent::new("worker"), now, Duration::from_secs(1), 1)
        .unwrap();
    assert!(matches!(
        r.claim(
            effect(),
            revision,
            &expired,
            now + Duration::from_secs(1),
            Duration::from_secs(30)
        ),
        Err(ReceiverError::ClaimRefused)
    ));
    let now = now + Duration::from_secs(1);
    let grant = r
        .authorize_worker(Agent::new("worker"), now, Duration::from_secs(60), 1)
        .unwrap();
    assert!(matches!(
        r.claim(
            effect(),
            StateRevision::ZERO,
            &grant,
            now,
            Duration::from_secs(30)
        ),
        Err(ReceiverError::ClaimRefused)
    ));
    let first = r
        .claim(effect(), revision, &grant, now, Duration::from_secs(30))
        .unwrap();
    assert_eq!(first.number, 1);
}

#[test]
fn receiver_process_loss_after_claim_returns_uncertainty_not_another_attempt() {
    use chronos_engine::{Agent, TaskPhase};
    use std::time::{Duration, SystemTime};
    let f = Fixture::new();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_exits_after_claim_commit"])
        .env("MOIRA_CHRONOS_CLAIM_DB", f.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "claim child failed");
    let mut r = f.open();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    let grant = r
        .authorize_worker(Agent::new("worker"), now, Duration::from_secs(60), 1)
        .unwrap();
    let observed = r
        .claim(effect(), r.revision(), &grant, now, Duration::from_secs(30))
        .unwrap();
    assert!(observed.replayed);
    assert_eq!(observed.phase, TaskPhase::Uncertain);
    assert_eq!(observed.number, 1);
    assert_eq!(r.claim_usage().committed, 0);
}

#[test]
fn child_exits_after_claim_commit() {
    use chronos_engine::Agent;
    use std::time::{Duration, SystemTime};
    let Some(path) = std::env::var_os("MOIRA_CHRONOS_CLAIM_DB") else {
        return;
    };
    let mut r = ChronosReceiver::open(PathBuf::from(path), [7; 16], limits()).unwrap();
    r.admit(effect(), StateRevision::ZERO, TaskSpecV2::new("work"))
        .unwrap();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    let grant = r
        .authorize_worker(Agent::new("worker"), now, Duration::from_secs(60), 1)
        .unwrap();
    r.claim(effect(), r.revision(), &grant, now, Duration::from_secs(30))
        .unwrap();
    std::process::exit(0);
}
