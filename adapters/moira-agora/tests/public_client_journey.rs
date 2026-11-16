use agora_client::v2::{PortFailureV2, ServicePortV2};
use agora_protocol::v2::{
    ClientIdV2, CommandV2, MessageIdV1, OutcomeV2, RefusalV2, ReplyV2, RequestIdV2, RequestV2,
};
use moira_agora::{
    PublicationIntentV1, PublicationLimitsV1, PublicationReplyV1, PublishErrorV1,
    ResponsePublisher, ResponsePublisherStateV1,
    durable::{
        DurablePublishErrorV1, DurableResponsePublisher, PublisherCheckpointV1, PublisherRecordV1,
        PublisherStateStoreV1,
    },
};
use moira_protocol::EffectIdV1;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortError {
    LostReply,
}

/// Independent public-contract conformance port. It models only Agora's exact
/// `(client, request)` retention and identified-message outcome; it does not
/// import Agora engine, service, wire or storage internals.
struct Port {
    lose_next_reply: bool,
    calls: usize,
    retained: BTreeMap<(ClientIdV2, RequestIdV2), (RequestV2, ReplyV2)>,
    messages: Vec<(MessageIdV1, String)>,
}

impl ServicePortV2 for Port {
    type Error = PortError;
    fn execute(
        &mut self,
        client: ClientIdV2,
        request: RequestV2,
    ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
        self.calls += 1;
        let key = (client, request.request_id);
        let reply = if let Some((retained, reply)) = self.retained.get(&key) {
            if retained == &request {
                reply.clone()
            } else {
                ReplyV2::Refused(RefusalV2::RequestConflict)
            }
        } else {
            let reply = match &request.command {
                CommandV2::SpeakIdentified {
                    room_id,
                    expected_revision,
                    body,
                    ..
                } if room_id == "room" && *expected_revision == 4 => {
                    let message_id = MessageIdV1::new([7; 16]);
                    self.messages.push((message_id, body.clone()));
                    ReplyV2::Accepted(OutcomeV2::IdentifiedMessageAccepted {
                        revision: 5,
                        message_id,
                        sequence: 0,
                    })
                }
                _ => ReplyV2::Refused(RefusalV2::InvalidInput),
            };
            self.retained.insert(key, (request.clone(), reply.clone()));
            reply
        };
        if self.lose_next_reply {
            self.lose_next_reply = false;
            return Err(PortFailureV2::OutcomeUnknown(PortError::LostReply));
        }
        Ok(reply)
    }
}

fn port(lose_next_reply: bool) -> Port {
    Port {
        lose_next_reply,
        calls: 0,
        retained: BTreeMap::new(),
        messages: Vec::new(),
    }
}

fn intent(effect: u8) -> PublicationIntentV1 {
    PublicationIntentV1 {
        effect: EffectIdV1([effect; 16]),
        source: [2; 16],
        room_sequence: 3,
        room_id: "room".to_owned(),
        expected_agora_revision: 4,
        logical_time: 5,
        body: "workflow response".to_owned(),
        addressed_to: vec!["owner".to_owned()],
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoreError {
    Conflict,
    LostCommitReply,
}

struct StateStore {
    record: PublisherRecordV1,
    compare_calls: usize,
    lose_reply_after_call: Option<usize>,
}

impl StateStore {
    fn new() -> Self {
        Self {
            record: PublisherRecordV1 {
                revision: 0,
                checkpoint: PublisherCheckpointV1::initial(ClientIdV2(1)),
            },
            compare_calls: 0,
            lose_reply_after_call: None,
        }
    }
}

impl PublisherStateStoreV1 for StateStore {
    type Error = StoreError;

    fn load(&mut self) -> Result<PublisherRecordV1, Self::Error> {
        Ok(self.record.clone())
    }

    fn compare_exchange(
        &mut self,
        expected_revision: u64,
        checkpoint: &PublisherCheckpointV1,
    ) -> Result<PublisherRecordV1, Self::Error> {
        if self.record.revision != expected_revision {
            return Err(StoreError::Conflict);
        }
        self.compare_calls += 1;
        self.record = PublisherRecordV1 {
            revision: expected_revision + 1,
            checkpoint: checkpoint.clone(),
        };
        if self.lose_reply_after_call == Some(self.compare_calls) {
            return Err(StoreError::LostCommitReply);
        }
        Ok(self.record.clone())
    }
}

#[test]
fn lost_public_client_reply_replays_one_message_under_the_same_moira_effect() {
    let mut publisher =
        ResponsePublisher::new(ClientIdV2(1), port(true), PublicationLimitsV1::default()).unwrap();

    assert_eq!(
        publisher.publish(intent(1)),
        Err(PublishErrorV1::Client(
            agora_client::v2::ClientErrorV2::OutcomeUnknown(PortError::LostReply)
        ))
    );
    assert_eq!(publisher.pending(), Some(&intent(1)));
    let state = publisher.state();
    let (port, pending_intent, pending_request) = publisher.into_parts();
    assert_eq!(pending_intent, Some(intent(1)));
    assert_eq!(pending_request.as_ref().unwrap().request_id, RequestIdV2(0));
    let mut publisher =
        ResponsePublisher::restore(port, PublicationLimitsV1::default(), state).unwrap();
    assert_eq!(
        publisher.publish(intent(9)),
        Err(PublishErrorV1::UnresolvedPublication)
    );
    let PublicationReplyV1::Published(published) = publisher.publish(intent(1)).unwrap() else {
        panic!("publication expected");
    };
    assert_eq!(published.receipt.effect, EffectIdV1([1; 16]));
    assert_eq!(published.receipt.message.source, [2; 16]);
    assert_eq!(published.receipt.message.room_sequence, 3);
    assert_eq!(published.receipt.message.message_sequence, 0);
    assert_eq!(published.receipt.agora_revision, 5);

    let (port, pending_intent, pending_request) = publisher.into_parts();
    assert!(pending_intent.is_none());
    assert!(pending_request.is_none());
    assert_eq!(port.calls, 2);
    assert_eq!(port.retained.len(), 1);
    assert_eq!(port.messages.len(), 1);
    assert_eq!(port.messages[0].0, published.message_id);
    assert_eq!(port.messages[0].1, "workflow response");
}

#[test]
fn durable_prepare_survives_peer_reply_loss_without_allocating_a_new_request() {
    let store = StateStore::new();
    let mut publisher =
        DurableResponsePublisher::new(store, port(true), PublicationLimitsV1::default()).unwrap();
    assert!(matches!(
        publisher.publish(intent(1)),
        Err(DurablePublishErrorV1::Publisher(PublishErrorV1::Client(
            agora_client::v2::ClientErrorV2::OutcomeUnknown(PortError::LostReply)
        )))
    ));
    let (store, port) = publisher.into_parts();
    assert_eq!(store.record.revision, 1);
    assert!(store.record.checkpoint.client.pending_intent.is_some());
    let mut publisher =
        DurableResponsePublisher::new(store, port.unwrap(), PublicationLimitsV1::default())
            .unwrap();
    let completed = publisher.publish(intent(1)).unwrap();
    assert!(!completed.replayed);
    assert_eq!(completed.storage_revision, 2);
    let (store, port) = publisher.into_parts();
    assert!(store.record.checkpoint.client.pending_intent.is_none());
    assert_eq!(port.unwrap().calls, 2);
}

#[test]
fn lost_outcome_commit_reply_replays_locally_without_another_agora_call() {
    let mut store = StateStore::new();
    store.lose_reply_after_call = Some(2);
    let mut publisher =
        DurableResponsePublisher::new(store, port(false), PublicationLimitsV1::default()).unwrap();
    assert_eq!(
        publisher.publish(intent(1)),
        Err(DurablePublishErrorV1::Store(StoreError::LostCommitReply))
    );
    let (store, port) = publisher.into_parts();
    assert_eq!(store.record.revision, 2);
    assert!(store.record.checkpoint.last.is_some());
    assert_eq!(port.as_ref().unwrap().calls, 1);

    let mut publisher =
        DurableResponsePublisher::new(store, port.unwrap(), PublicationLimitsV1::default())
            .unwrap();
    let replay = publisher.publish(intent(1)).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.storage_revision, 2);
    let (_, port) = publisher.into_parts();
    assert_eq!(port.unwrap().calls, 1);
}

#[test]
fn bounds_refuse_before_dispatch_and_abandonment_returns_both_identities() {
    let mut publisher = ResponsePublisher::new(
        ClientIdV2(1),
        port(true),
        PublicationLimitsV1 {
            max_body_bytes: 4,
            ..PublicationLimitsV1::default()
        },
    )
    .unwrap();
    assert_eq!(
        publisher.publish(intent(1)),
        Err(PublishErrorV1::InvalidInput)
    );
    let mut bounded = intent(1);
    bounded.body = "okay".to_owned();
    assert!(matches!(
        publisher.publish(bounded.clone()),
        Err(PublishErrorV1::Client(
            agora_client::v2::ClientErrorV2::OutcomeUnknown(PortError::LostReply)
        ))
    ));
    let (abandoned_intent, abandoned_request) = publisher.abandon_uncertainty().unwrap();
    assert_eq!(abandoned_intent, bounded);
    assert_eq!(abandoned_request.request_id, RequestIdV2(0));
    let (port, _, _) = publisher.into_parts();
    assert_eq!(port.calls, 1);
    assert_eq!(port.messages.len(), 1);
}

#[test]
fn restore_refuses_mismatched_moira_intent_and_agora_request() {
    let mut publisher =
        ResponsePublisher::new(ClientIdV2(1), port(true), PublicationLimitsV1::default()).unwrap();
    assert!(matches!(
        publisher.publish(intent(1)),
        Err(PublishErrorV1::Client(
            agora_client::v2::ClientErrorV2::OutcomeUnknown(PortError::LostReply)
        ))
    ));
    let (lost_reply_port, _, _) = publisher.into_parts();
    let mut state = ResponsePublisherStateV1 {
        client: agora_client::v2::ClientStateV2 {
            client: ClientIdV2(1),
            next_request: 1,
            pending: Some(RequestV2 {
                request_id: RequestIdV2(0),
                command: CommandV2::SpeakIdentified {
                    room_id: "other".to_owned(),
                    expected_revision: 4,
                    logical_time: 5,
                    body: "workflow response".to_owned(),
                    addressed_to: vec!["owner".to_owned()],
                },
            }),
        },
        pending_intent: Some(intent(1)),
    };
    assert!(matches!(
        ResponsePublisher::restore(
            lost_reply_port,
            PublicationLimitsV1::default(),
            state.clone(),
        ),
        Err(PublishErrorV1::InvalidState)
    ));
    state.client.pending.as_mut().unwrap().request_id = RequestIdV2(9);
    assert!(matches!(
        ResponsePublisher::restore(port(false), PublicationLimitsV1::default(), state),
        Err(PublishErrorV1::InvalidState)
    ));
}
