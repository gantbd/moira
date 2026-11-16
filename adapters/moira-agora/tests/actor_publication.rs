use agora_actor_publication_port::{
    ActorPublicationAcceptedV1, ActorPublicationObservationRefusalV1,
    ActorPublicationObservationV1, ActorPublicationRefusalV1, ActorPublicationRequestV1,
    ActorPublicationWireClientV1,
};
use moira_agora::{
    PublicationIntentV1, PublicationLimitsV1,
    actor::{
        ActorObservationReplyV1, ActorPublicationReplyV1, ActorPublisherErrorV1,
        ActorResponsePublisher,
    },
};
use moira_protocol::EffectIdV1;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Error {
    LostReply,
}

#[derive(Clone)]
struct Client {
    retained: Arc<Mutex<Option<(ActorPublicationRequestV1, ActorPublicationAcceptedV1)>>>,
    lose_publish_reply: bool,
}

impl ActorPublicationWireClientV1 for Client {
    type Error = Error;

    fn publish_actor_wire(
        &mut self,
        request: ActorPublicationRequestV1,
    ) -> Result<Result<ActorPublicationAcceptedV1, ActorPublicationRefusalV1>, Self::Error> {
        let mut retained = self.retained.lock().unwrap();
        let accepted = ActorPublicationAcceptedV1 {
            revision: 19,
            message_id: [0x44; 16],
            message_sequence: 7,
        };
        match retained.as_ref() {
            Some((prior, prior_accepted)) if prior == &request => return Ok(Ok(*prior_accepted)),
            Some(_) => return Ok(Err(ActorPublicationRefusalV1::Conflict)),
            None => *retained = Some((request, accepted)),
        }
        if self.lose_publish_reply {
            Err(Error::LostReply)
        } else {
            Ok(Ok(accepted))
        }
    }

    fn observe_actor_wire(
        &mut self,
        request: ActorPublicationRequestV1,
    ) -> Result<
        Result<ActorPublicationObservationV1, ActorPublicationObservationRefusalV1>,
        Self::Error,
    > {
        Ok(match self.retained.lock().unwrap().as_ref() {
            Some((prior, accepted)) if prior == &request => {
                Ok(ActorPublicationObservationV1::Accepted(*accepted))
            }
            Some(_) => Err(ActorPublicationObservationRefusalV1::Conflict),
            None => Ok(ActorPublicationObservationV1::Absent),
        })
    }
}

fn intent() -> PublicationIntentV1 {
    PublicationIntentV1 {
        effect: EffectIdV1([0x31; 16]),
        source: [0x32; 16],
        room_sequence: 5,
        room_id: "room".into(),
        expected_agora_revision: 18,
        logical_time: 23,
        body: "answer".into(),
        addressed_to: vec!["human".into()],
    }
}

#[test]
fn lost_publish_reply_is_observed_after_independent_reconnect() {
    let retained = Arc::new(Mutex::new(None));
    let first = Client {
        retained: retained.clone(),
        lose_publish_reply: true,
    };
    let mut publisher = ActorResponsePublisher::new(first, PublicationLimitsV1::default()).unwrap();
    assert_eq!(
        publisher.publish(&intent()),
        Err(ActorPublisherErrorV1::Transport(Error::LostReply))
    );

    let reconnected = Client {
        retained,
        lose_publish_reply: false,
    };
    let mut publisher =
        ActorResponsePublisher::new(reconnected, PublicationLimitsV1::default()).unwrap();
    let ActorObservationReplyV1::Published(observed) = publisher.observe(&intent()).unwrap() else {
        panic!("retained publication expected")
    };
    assert_eq!(observed.receipt.effect, intent().effect);
    assert_eq!(observed.receipt.message.message_sequence, 7);
    assert_eq!(observed.receipt.agora_revision, 19);
    assert_eq!(observed.message_id.into_bytes(), [0x44; 16]);
    assert!(matches!(
        publisher.publish(&intent()),
        Ok(ActorPublicationReplyV1::Published(replayed)) if replayed == observed
    ));
}

#[test]
fn absence_and_conflict_never_fall_through_to_publication() {
    let retained = Arc::new(Mutex::new(None));
    let client = Client {
        retained,
        lose_publish_reply: false,
    };
    let mut publisher =
        ActorResponsePublisher::new(client, PublicationLimitsV1::default()).unwrap();
    assert_eq!(
        publisher.observe(&intent()).unwrap(),
        ActorObservationReplyV1::Absent
    );
    assert!(matches!(
        publisher.publish(&intent()).unwrap(),
        ActorPublicationReplyV1::Published(_)
    ));
    let mut changed = intent();
    changed.body = "changed".into();
    assert_eq!(
        publisher.observe(&changed).unwrap(),
        ActorObservationReplyV1::Refused(ActorPublicationObservationRefusalV1::Conflict)
    );
}
