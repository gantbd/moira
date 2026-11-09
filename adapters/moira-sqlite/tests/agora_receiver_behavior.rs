#![cfg(unix)]
use agora_engine::*;
use moira_core::EffectId;
use moira_sqlite::{Timing, agora::*};
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
            "moira-agora-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("agora.db")
    }
    fn open(&self) -> AgoraReceiver {
        AgoraReceiver::open(self.path(), identity(), limits()).unwrap()
    }
    fn configured(&self) -> AgoraReceiver {
        let mut r = self.open();
        for (id, kind) in [
            ("human", ParticipantKind::Human),
            ("agent", ParticipantKind::Agent),
        ] {
            r.apply(RevisionedCommandV1 {
                expected_revision: r.revision(),
                command: CommandV1::RegisterParticipant(Participant::new(id, id, kind)),
            })
            .unwrap();
        }
        r.apply(RevisionedCommandV1 {
            expected_revision: r.revision(),
            command: CommandV1::CreateRoom {
                room_id: "room".into(),
                name: "Room".into(),
                owner: "human".into(),
                admission: AdmissionPolicy::Open,
            },
        })
        .unwrap();
        r.apply(RevisionedCommandV1 {
            expected_revision: r.revision(),
            command: CommandV1::Join {
                room_id: "room".into(),
                participant: "agent".into(),
            },
        })
        .unwrap();
        r
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["agora.db", "agora.db-journal", "agora.db.owner.lock"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}
fn identity() -> AgoraIdentity {
    AgoraIdentity {
        owner: [7; 16],
        source: [10; 16],
    }
}
fn limits() -> AgoraReceiverLimits {
    AgoraReceiverLimits {
        engine: Limits {
            max_messages_per_room: 1,
            ..Limits::default()
        },
        max_snapshot_bytes: 65536,
        max_database_pages: 64,
        timing: Timing::Off,
    }
}
fn message() -> ReplyMessage {
    ReplyMessage {
        room_id: "room".into(),
        author: "agent".into(),
        at: LogicalTime(10),
        body: "answer\0\r\n🇩🇪".into(),
    }
}

#[test]
fn reply_and_exact_receipt_survive_restart_even_after_message_eviction() {
    let f = Fixture::new();
    let mut r = f.configured();
    let original = r.revision();
    let first = r
        .publish(EffectId::new([1; 16]), original, message())
        .unwrap();
    assert!(!first.replayed);
    r.apply(RevisionedCommandV1 {
        expected_revision: r.revision(),
        command: CommandV1::Speak {
            room_id: "room".into(),
            author: "human".into(),
            at: LogicalTime(11),
            body: "newest".into(),
        },
    })
    .unwrap();
    let events = r.pending_events().unwrap();
    drop(r);
    let mut r = f.open();
    let replay = r
        .publish(EffectId::new([1; 16]), original, message())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.receipt, first.receipt);
    assert_eq!(replay.transition, first.transition);
    assert_eq!(r.room(&"room".into()).unwrap().unwrap().messages.len(), 1);
    assert_eq!(r.pending_events().unwrap(), events);
    assert_eq!(r.publication_usage().committed, 0);
    r.acknowledge_events(EventSequence(2)).unwrap();
    drop(r);
    let mut r = f.open();
    assert_eq!(
        r.pending_events().unwrap().first().unwrap().sequence,
        EventSequence(3)
    );
}

#[test]
fn source_scope_and_room_ordinals_remain_stable_across_restart() {
    let f = Fixture::new();
    let mut r = f.configured();
    let result = r
        .publish(EffectId::new([1; 16]), r.revision(), message())
        .unwrap();
    assert_eq!(
        r.message_ref(&"room".into(), 0).unwrap(),
        moira_core::AgoraMessageRefV1::new([10; 16], 0, 0)
    );
    drop(r);
    assert!(matches!(
        AgoraReceiver::open(
            f.path(),
            AgoraIdentity {
                source: [11; 16],
                ..identity()
            },
            limits()
        ),
        Err(AgoraReceiverError::ScopeMismatch)
    ));
    let mut r = f.open();
    assert!(
        r.publish(EffectId::new([1; 16]), Revision(0), message())
            .unwrap()
            .replayed
    );
    assert_eq!(
        r.message_ref(&"room".into(), 0).unwrap(),
        moira_core::AgoraMessageRefV1::new([10; 16], 0, 0)
    );
    assert_eq!(result.transition.revision, r.revision());
}

#[test]
fn conflicting_publication_and_stale_command_do_not_change_stored_state() {
    let f = Fixture::new();
    let mut r = f.configured();
    r.publish(EffectId::new([1; 16]), r.revision(), message())
        .unwrap();
    let before = r.revision();
    let mut changed = message();
    changed.body = "different".into();
    assert!(matches!(
        r.publish(EffectId::new([1; 16]), before, changed),
        Err(AgoraReceiverError::PublicationConflict)
    ));
    assert!(
        r.apply(RevisionedCommandV1 {
            expected_revision: Revision(0),
            command: CommandV1::Join {
                room_id: "room".into(),
                participant: "agent".into()
            }
        })
        .is_err()
    );
    assert_eq!(r.revision(), before);
    drop(r);
    assert_eq!(f.open().revision(), before);
}

#[test]
fn receiver_exit_after_commit_retains_exact_reply_and_releases_ownership() {
    let f = Fixture::new();
    let r = f.configured();
    let expected = r.revision();
    drop(r);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_commits_reply_without_drop"])
        .env("MOIRA_AGORA_REPLY_DB", f.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "Agora receiver child failed");
    let mut r = f.open();
    assert!(
        r.publish(EffectId::new([1; 16]), expected, message())
            .unwrap()
            .replayed
    );
    assert_eq!(r.room(&"room".into()).unwrap().unwrap().messages.len(), 1);
}

#[test]
fn child_commits_reply_without_drop() {
    let Some(path) = std::env::var_os("MOIRA_AGORA_REPLY_DB") else {
        return;
    };
    let mut r = AgoraReceiver::open(PathBuf::from(path), identity(), limits()).unwrap();
    r.publish(EffectId::new([1; 16]), r.revision(), message())
        .unwrap();
    std::process::exit(0);
}
