use agora_client::{
    room_command::CreateRoomRequestV1,
    v2::{PortFailureV2, ServicePortV2},
};
use agora_protocol::v2::{
    AdmissionPolicyV2, ClientIdV2, CommandV2, OutcomeV2, ReplyV2, RequestIdV2, RequestV2,
};
use moira_agora::room::{
    DurableRoomErrorV1, DurableRoomProvisioner, RoomIntentV1, RoomLimitsV1, RoomReplyV1,
    RoomStateStoreV1,
};
use moira_agora_publisher_sqlite::{
    PublisherStoreErrorV1,
    room::{RoomSqliteStore, RoomStoreLimitsV1},
};
use moira_core::task_agent_journey::{JourneyIdV1, OwnerIdV1};
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
    rooms: usize,
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
                CommandV2::CreateRoom { ref room_id, .. } if room_id == "task-agent-3"
            ));
            state.rooms += 1;
            let reply = ReplyV2::Accepted(OutcomeV2::RoomCreated { revision: 8 });
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
            "moira-agora-room-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> PathBuf {
        self.0.join("room.db")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn intent() -> RoomIntentV1 {
    RoomIntentV1 {
        journey: JourneyIdV1 {
            owner: OwnerIdV1([5; 16]),
            sequence: 3,
        },
        expected_journey_revision: 1,
        source: [5; 16],
        room_sequence: 3,
        request: CreateRoomRequestV1 {
            room_id: "task-agent-3".into(),
            name: "Task agent 3".into(),
            admission: AdmissionPolicyV2::InviteOnly,
            expected_revision: 7,
        },
    }
}

#[test]
fn prepared_room_and_completed_outcome_survive_separate_sqlite_reopens() {
    let fixture = Fixture::new();
    let state = Rc::new(RefCell::new(AgoraState::default()));
    let limits = RoomStoreLimitsV1::default();
    let store = RoomSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(6), limits).unwrap();
    let mut provisioner = DurableRoomProvisioner::new(
        store,
        Port {
            state: Rc::clone(&state),
            lose_reply: true,
        },
        RoomLimitsV1::default(),
    )
    .unwrap();
    assert!(matches!(
        provisioner.provision(intent()),
        Err(DurableRoomErrorV1::Client(_))
    ));
    drop(provisioner);
    assert_eq!(state.borrow().rooms, 1);

    let mut store = RoomSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(6), limits).unwrap();
    assert!(store.load().unwrap().checkpoint.pending_intent.is_some());
    let mut provisioner = DurableRoomProvisioner::new(
        store,
        Port {
            state: Rc::clone(&state),
            lose_reply: false,
        },
        RoomLimitsV1::default(),
    )
    .unwrap();
    let completed = provisioner.provision(intent()).unwrap();
    assert!(!completed.replayed);
    assert!(matches!(completed.reply, RoomReplyV1::Created(_)));
    drop(provisioner);
    assert_eq!(state.borrow().calls, 2);
    assert_eq!(state.borrow().rooms, 1);

    let store = RoomSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(6), limits).unwrap();
    let mut provisioner = DurableRoomProvisioner::new(
        store,
        Port {
            state: Rc::clone(&state),
            lose_reply: false,
        },
        RoomLimitsV1::default(),
    )
    .unwrap();
    let replay = provisioner.provision(intent()).unwrap();
    assert!(replay.replayed);
    assert_eq!(state.borrow().calls, 2);
    assert_eq!(state.borrow().rooms, 1);
}

#[test]
fn room_owner_profile_and_client_drift_refuse_reopen() {
    let fixture = Fixture::new();
    let limits = RoomStoreLimitsV1::default();
    drop(RoomSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(6), limits).unwrap());
    assert!(matches!(
        RoomSqliteStore::open(fixture.path(), [7; 16], ClientIdV2(6), limits),
        Err(PublisherStoreErrorV1::OwnerMismatch)
    ));
    let mut changed = limits;
    changed.max_checkpoint_bytes += 1;
    assert!(matches!(
        RoomSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(6), changed),
        Err(PublisherStoreErrorV1::ProfileMismatch)
    ));
    assert!(matches!(
        RoomSqliteStore::open(fixture.path(), [5; 16], ClientIdV2(8), limits),
        Err(PublisherStoreErrorV1::ClientMismatch)
    ));
}
