use agora_client::v2::{PortFailureV2, ServicePortV2};
use agora_durable::{
    DurableConfigV2, DurablePublicationV2, DurableServiceV2, DurableSuppressionV2,
};
use agora_engine::{
    AdmissionPolicy, CommandV2 as EngineCommand, Engine, Limits, Participant, ParticipantKind,
    RoomId,
};
use agora_protocol::v2::{ClientIdV2, ReplyV2, RequestV2};
use agora_service::v2::{
    ServiceLimitsV2, ServiceV2,
    persistence::{ServiceCheckpointScopeV1, ServiceRecoveryLimitsV2},
};
use keystone_storage::{
    AcknowledgementStrength, Checkpoint, CheckpointStore, CommitOutcome, CommitReceipt, Revision,
    StoreRefusal,
};
use moira_agora::{
    PublicationIntentV1, PublicationLimitsV1, PublicationReplyV1, PublishErrorV1, ResponsePublisher,
};
use moira_protocol::EffectIdV1;
use std::{cell::RefCell, rc::Rc};

const MAX_SNAPSHOT_BYTES: usize = 2 * 1_024 * 1_024;
const SCOPE: ServiceCheckpointScopeV1 = ServiceCheckpointScopeV1::new(*b"moira-agora-v1!!");

#[derive(Default)]
struct StoreState {
    checkpoint: Option<Checkpoint>,
    commits: usize,
}

#[derive(Clone, Default)]
struct Store(Rc<RefCell<StoreState>>);

impl Store {
    fn commits(&self) -> usize {
        self.0.borrow().commits
    }
}

impl CheckpointStore for Store {
    type Error = ();

    fn load(&self, _: &str) -> Result<Option<Checkpoint>, Self::Error> {
        Ok(self.0.borrow().checkpoint.clone())
    }

    fn compare_exchange(
        &mut self,
        _: &str,
        expected: Option<Revision>,
        bytes: &[u8],
    ) -> Result<CommitOutcome, Self::Error> {
        let mut state = self.0.borrow_mut();
        let actual = state
            .checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.revision);
        if actual != expected {
            return Ok(CommitOutcome::Refused(StoreRefusal::RevisionConflict {
                expected,
                actual,
            }));
        }
        let revision = Revision(actual.map_or(0, |value| value.0 + 1));
        state.commits += 1;
        state.checkpoint = Some(Checkpoint {
            revision,
            bytes: bytes.to_vec(),
            strength: AcknowledgementStrength::Durable,
        });
        Ok(CommitOutcome::Committed(CommitReceipt {
            revision,
            mechanism: "moira-agora-qualification-v1",
            strength: AcknowledgementStrength::Durable,
        }))
    }
}

fn config() -> DurableConfigV2 {
    DurableConfigV2::new(
        "agora",
        SCOPE,
        MAX_SNAPSHOT_BYTES,
        ServiceRecoveryLimitsV2 {
            max_snapshot_bytes: MAX_SNAPSHOT_BYTES,
            max_engine_snapshot_bytes: MAX_SNAPSHOT_BYTES,
            maximum_service_limits: ServiceLimitsV2::default(),
            maximum_engine_limits: Limits::default(),
        },
    )
}

fn service() -> ServiceV2 {
    let mut engine = Engine::new();
    engine
        .apply(EngineCommand::RegisterParticipant(Participant::new(
            "owner",
            "Owner",
            ParticipantKind::Human,
        )))
        .unwrap();
    engine
        .apply(EngineCommand::CreateRoom {
            room_id: "room".into(),
            name: "Room".to_owned(),
            owner: "owner".into(),
            admission: AdmissionPolicy::Open,
        })
        .unwrap();
    ServiceV2::new(engine, ServiceLimitsV2::default())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortError {
    Suppressed,
    LostReply,
}

struct Port {
    service: DurableServiceV2<Store>,
    lose_next_reply: bool,
}

impl Port {
    fn restart(self) -> Self {
        let store = self.service.into_store();
        Self {
            service: DurableServiceV2::recover(store, config()).unwrap(),
            lose_next_reply: false,
        }
    }
}

impl ServicePortV2 for Port {
    type Error = PortError;

    fn execute(
        &mut self,
        client: ClientIdV2,
        request: RequestV2,
    ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
        let reply = match self.service.execute(client, request) {
            DurablePublicationV2::Published { value, .. } => value,
            DurablePublicationV2::Suppressed(
                DurableSuppressionV2::Fenced
                | DurableSuppressionV2::Checkpoint(_)
                | DurableSuppressionV2::StoreRefused(_)
                | DurableSuppressionV2::OutcomeUnknown
                | DurableSuppressionV2::NotDurable
                | DurableSuppressionV2::InvalidReceipt,
            ) => return Err(PortFailureV2::OutcomeUnknown(PortError::Suppressed)),
        };
        if self.lose_next_reply {
            self.lose_next_reply = false;
            return Err(PortFailureV2::OutcomeUnknown(PortError::LostReply));
        }
        Ok(reply)
    }
}

fn intent(effect: u8) -> PublicationIntentV1 {
    PublicationIntentV1 {
        effect: EffectIdV1([effect; 16]),
        source: [2; 16],
        room_sequence: 3,
        room_id: "room".to_owned(),
        expected_agora_revision: 2,
        logical_time: 5,
        body: "workflow response".to_owned(),
        addressed_to: vec!["owner".to_owned()],
    }
}

#[test]
fn publisher_and_durable_agora_restart_close_one_lost_reply_without_duplicate_message() {
    let store = Store::default();
    let mut service = DurableServiceV2::initialize(service(), store.clone(), config()).unwrap();
    let DurablePublicationV2::Published {
        value: Ok(client), ..
    } = service.connect("owner")
    else {
        panic!("durable client expected");
    };
    let port = Port {
        service,
        lose_next_reply: true,
    };
    let limits = PublicationLimitsV1::default();
    let mut publisher = ResponsePublisher::new(client, port, limits).unwrap();
    assert_eq!(
        publisher.publish(intent(1)),
        Err(PublishErrorV1::Client(
            agora_client::v2::ClientErrorV2::OutcomeUnknown(PortError::LostReply)
        ))
    );
    let publisher_state = publisher.state();
    let (port, pending_intent, pending_request) = publisher.into_parts();
    assert_eq!(pending_intent, Some(intent(1)));
    assert!(pending_request.is_some());
    let commits_after_publish = store.commits();

    let mut publisher =
        ResponsePublisher::restore(port.restart(), limits, publisher_state).unwrap();
    assert_eq!(
        publisher.publish(intent(9)),
        Err(PublishErrorV1::UnresolvedPublication)
    );
    let PublicationReplyV1::Published(published) = publisher.publish(intent(1)).unwrap() else {
        panic!("exact retained publication expected");
    };
    assert_eq!(published.receipt.effect, EffectIdV1([1; 16]));
    assert_eq!(published.receipt.agora_revision, 3);
    assert_eq!(published.receipt.message.message_sequence, 0);
    assert_eq!(
        store.commits(),
        commits_after_publish,
        "replay must not commit"
    );

    let (port, pending_intent, pending_request) = publisher.into_parts();
    assert!(pending_intent.is_none());
    assert!(pending_request.is_none());
    let message = port
        .service
        .service()
        .unwrap()
        .engine()
        .message(&RoomId::new("room"), 0)
        .unwrap();
    assert_eq!(message.body(), "workflow response");
}
