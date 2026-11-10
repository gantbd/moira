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
            r.apply(RevisionedCommandV2 {
                expected_revision: r.revision(),
                command: CommandV2::RegisterParticipant(Participant::new(id, id, kind)),
            })
            .unwrap();
        }
        r.apply(RevisionedCommandV2 {
            expected_revision: r.revision(),
            command: CommandV2::CreateRoom {
                room_id: "room".into(),
                name: "Room".into(),
                owner: "human".into(),
                admission: AdmissionPolicy::Open,
            },
        })
        .unwrap();
        r.apply(RevisionedCommandV2 {
            expected_revision: r.revision(),
            command: CommandV2::Join {
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
        for name in [
            "agora.db",
            "agora.db-journal",
            "agora.db.owner.lock",
            "origin.db",
            "origin.db-journal",
            "origin.db.owner.lock",
        ] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}

#[test]
fn addressed_event_is_passive_until_explicit_moira_admission() {
    use moira_core::{CommandV1, Engine as Moira, Limits as MoiraLimits, ParticipantRefV1};

    let f = Fixture::new();
    let mut receiver = f.configured();
    assert!(matches!(
        receiver.pending_message_fact(EventSequence(0)),
        Err(AgoraReceiverError::NotMessageEvent)
    ));
    let accepted = receiver
        .apply(RevisionedCommandV2 {
            expected_revision: receiver.revision(),
            command: CommandV2::SpeakAddressed {
                room_id: "room".into(),
                author: "human".into(),
                at: LogicalTime(9),
                body: "private question".into(),
                addressed_to: vec!["agent".into()],
            },
        })
        .unwrap();
    let fact = receiver
        .pending_message_fact(accepted.event_sequence)
        .unwrap();
    assert_eq!(fact.event_sequence(), accepted.event_sequence);
    assert_eq!(
        fact.message(),
        moira_core::AgoraMessageRefV1::new([10; 16], 0, 0)
    );
    assert_eq!(fact.author().as_str(), "human");
    assert_eq!(fact.addressed_to(), [ParticipantId::from("agent")]);
    assert!(fact.is_addressed_to(&"agent".into()));
    let debug = format!("{fact:?}");
    assert!(!debug.contains("human"));
    assert!(!debug.contains("agent"));
    assert!(!debug.contains("private question"));

    let mut moira = Moira::try_new(MoiraLimits::default()).unwrap();
    assert_eq!(moira.revision(), moira_core::WorkflowRevision(0));
    let command = CommandV1::AdmitMessage {
        message: fact.message(),
        recipient: ParticipantRefV1::new([3; 16]),
    };
    let admitted = moira.apply(moira.revision(), command).unwrap();
    let revision = moira.revision();
    assert_eq!(moira.apply(revision, command).unwrap(), admitted);
    assert_eq!(moira.revision(), revision);
    assert_eq!(
        moira.apply(
            revision,
            CommandV1::AdmitMessage {
                message: fact.message(),
                recipient: ParticipantRefV1::new([4; 16]),
            },
        ),
        Err(moira_core::RefusalV1::InputConflict)
    );
    assert_eq!(moira.revision(), revision);

    receiver
        .acknowledge_events(accepted.event_sequence)
        .unwrap();
    assert!(matches!(
        receiver.pending_message_fact(accepted.event_sequence),
        Err(AgoraReceiverError::UnknownEvent)
    ));
}

#[test]
fn addressed_fact_and_explicit_admission_survive_lost_reply_without_duplication() {
    use moira_core::persistence::CodecLimits;
    use moira_core::{CommandV1, ParticipantRefV1, TransitionV1, WorkflowRevision};
    use moira_sqlite::{SqliteStore, StoreLimits};

    let f = Fixture::new();
    let mut receiver = f.configured();
    let accepted = receiver
        .apply(RevisionedCommandV2 {
            expected_revision: receiver.revision(),
            command: CommandV2::SpeakAddressed {
                room_id: "room".into(),
                author: "human".into(),
                at: LogicalTime(9),
                body: "question".into(),
                addressed_to: vec!["agent".into()],
            },
        })
        .unwrap();
    receiver
        .apply(RevisionedCommandV2 {
            expected_revision: receiver.revision(),
            command: CommandV2::Speak {
                room_id: "room".into(),
                author: "human".into(),
                at: LogicalTime(10),
                body: "newest retained message".into(),
            },
        })
        .unwrap();
    assert!(matches!(
        receiver.message_ref(&"room".into(), 0),
        Err(AgoraReceiverError::UnretainedMessage)
    ));
    let before = receiver
        .pending_message_fact(accepted.event_sequence)
        .unwrap();
    drop(receiver);
    let mut receiver = f.open();
    assert_eq!(
        receiver
            .pending_message_fact(accepted.event_sequence)
            .unwrap(),
        before
    );

    let command = CommandV1::AdmitMessage {
        message: before.message(),
        recipient: ParticipantRefV1::new([3; 16]),
    };
    let store_limits = StoreLimits {
        codec: CodecLimits {
            max_bytes: 8_192,
            max_workflows: 2,
        },
        max_database_pages: 64,
        timing: Timing::Off,
    };
    let path = f.0.join("origin.db");
    let mut origin = SqliteStore::open(&path, [8; 16], store_limits).unwrap();
    let _lost = origin.apply(WorkflowRevision(0), command).unwrap();
    drop(origin);

    let mut origin = SqliteStore::open(&path, [8; 16], store_limits).unwrap();
    let replay = origin.apply(WorkflowRevision(1), command).unwrap();
    assert!(replay.replayed);
    assert!(matches!(
        replay.transition,
        TransitionV1::ChronosSubmissionPrepared { .. }
    ));
    assert_eq!(origin.usage().committed, 0);
    assert_eq!(origin.usage().replayed, 1);
}

#[test]
fn participant_kind_does_not_change_addressed_fact_projection() {
    for (author, recipient) in [("human", "agent"), ("agent", "human")] {
        let f = Fixture::new();
        let mut receiver = f.configured();
        let accepted = receiver
            .apply(RevisionedCommandV2 {
                expected_revision: receiver.revision(),
                command: CommandV2::SpeakAddressed {
                    room_id: "room".into(),
                    author: author.into(),
                    at: LogicalTime(9),
                    body: "message".into(),
                    addressed_to: vec![recipient.into()],
                },
            })
            .unwrap();
        let fact = receiver
            .pending_message_fact(accepted.event_sequence)
            .unwrap();
        assert_eq!(fact.author().as_str(), author);
        assert_eq!(fact.addressed_to(), [ParticipantId::from(recipient)]);
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
    r.apply(RevisionedCommandV2 {
        expected_revision: r.revision(),
        command: CommandV2::Speak {
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
        r.apply(RevisionedCommandV2 {
            expected_revision: Revision(0),
            command: CommandV2::Join {
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
