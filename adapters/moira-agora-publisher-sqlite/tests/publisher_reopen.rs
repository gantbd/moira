use agora_client::v2::{PortFailureV2, ServicePortV2};
use agora_protocol::v2::{
    ClientIdV2, CommandV2, MessageIdV1, OutcomeV2, ReplyV2, RequestIdV2, RequestV2,
};
use moira_agora::{
    PublicationIntentV1, PublicationLimitsV1, PublicationReplyV1,
    durable::{DurablePublishErrorV1, DurableResponsePublisher, PublisherStateStoreV1},
};
use moira_agora_publisher_sqlite::{
    PublisherSqliteStore, PublisherStoreErrorV1, PublisherStoreLimitsV1,
};
use moira_protocol::EffectIdV1;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortError {
    LostReply,
}

#[derive(Default)]
struct AgoraState {
    calls: usize,
    messages: usize,
    retained: BTreeMap<(ClientIdV2, RequestIdV2), (RequestV2, ReplyV2)>,
}

struct Port {
    state: Rc<RefCell<AgoraState>>,
    lose_reply: bool,
}

impl ServicePortV2 for Port {
    type Error = PortError;

    fn execute(
        &mut self,
        client: ClientIdV2,
        request: RequestV2,
    ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
        let mut state = self.state.borrow_mut();
        state.calls += 1;
        let key = (client, request.request_id);
        let reply = if let Some((retained, reply)) = state.retained.get(&key) {
            assert_eq!(retained, &request);
            reply.clone()
        } else {
            assert!(matches!(
                request.command,
                CommandV2::SpeakIdentified { ref room_id, .. } if room_id == "room"
            ));
            state.messages += 1;
            let reply = ReplyV2::Accepted(OutcomeV2::IdentifiedMessageAccepted {
                revision: 8,
                message_id: MessageIdV1::new([9; 16]),
                sequence: 10,
            });
            state.retained.insert(key, (request, reply.clone()));
            reply
        };
        if self.lose_reply {
            self.lose_reply = false;
            Err(PortFailureV2::OutcomeUnknown(PortError::LostReply))
        } else {
            Ok(reply)
        }
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moira-agora-publisher-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> PathBuf {
        self.0.join("publisher.db")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn intent() -> PublicationIntentV1 {
    PublicationIntentV1 {
        effect: EffectIdV1([1; 16]),
        source: [2; 16],
        room_sequence: 3,
        room_id: "room".into(),
        expected_agora_revision: 7,
        logical_time: 4,
        body: "answer".into(),
        addressed_to: vec!["owner".into()],
    }
}

#[test]
fn prepared_request_and_completed_outcome_survive_separate_sqlite_reopens() {
    let fixture = Fixture::new();
    let state = Rc::new(RefCell::new(AgoraState::default()));
    let limits = PublisherStoreLimitsV1::default();
    let store = PublisherSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(6), limits).unwrap();
    let mut publisher = DurableResponsePublisher::new(
        store,
        Port {
            state: Rc::clone(&state),
            lose_reply: true,
        },
        PublicationLimitsV1::default(),
    )
    .unwrap();
    assert!(matches!(
        publisher.publish(intent()),
        Err(DurablePublishErrorV1::Publisher(_))
    ));
    drop(publisher);
    assert_eq!(state.borrow().messages, 1);

    let mut store =
        PublisherSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(6), limits).unwrap();
    assert!(
        store
            .load()
            .unwrap()
            .checkpoint
            .client
            .pending_intent
            .is_some()
    );
    let mut publisher = DurableResponsePublisher::new(
        store,
        Port {
            state: Rc::clone(&state),
            lose_reply: false,
        },
        PublicationLimitsV1::default(),
    )
    .unwrap();
    let completed = publisher.publish(intent()).unwrap();
    assert!(!completed.replayed);
    assert!(matches!(completed.reply, PublicationReplyV1::Published(_)));
    drop(publisher);
    assert_eq!(state.borrow().calls, 2);
    assert_eq!(state.borrow().messages, 1);

    let store = PublisherSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(6), limits).unwrap();
    let mut publisher = DurableResponsePublisher::new(
        store,
        Port {
            state: Rc::clone(&state),
            lose_reply: false,
        },
        PublicationLimitsV1::default(),
    )
    .unwrap();
    let replay = publisher.publish(intent()).unwrap();
    assert!(replay.replayed);
    assert_eq!(state.borrow().calls, 2);
    assert_eq!(state.borrow().messages, 1);
}

#[test]
fn owner_profile_and_client_drift_refuse_reopen() {
    let fixture = Fixture::new();
    let limits = PublisherStoreLimitsV1::default();
    drop(PublisherSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(6), limits).unwrap());
    assert!(matches!(
        PublisherSqliteStore::open(fixture.path(), [7; 16], ClientIdV2(6), limits),
        Err(PublisherStoreErrorV1::OwnerMismatch)
    ));
    let mut changed = limits;
    changed.max_checkpoint_bytes += 1;
    assert!(matches!(
        PublisherSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(6), changed),
        Err(PublisherStoreErrorV1::ProfileMismatch)
    ));
    assert!(matches!(
        PublisherSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(8), limits),
        Err(PublisherStoreErrorV1::ClientMismatch)
    ));
}
