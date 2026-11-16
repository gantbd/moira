use hermes_client::ServicePort;
use hermes_host::DurableLedger;
use hermes_ledger::{Limits, persistence::CheckpointBudgetV1};
use hermes_protocol::{
    AppendOutcomeV1, CommandV1, EventSequenceV1, OutcomeV1, ProducerIdV1, ReplyV1, SchemaIdV1,
    SessionHandleV1, StreamIdV1,
};
use hermes_service::{
    AuthenticatedSessionV1, AuthorizationActionV1, AuthorizationResourceV1, Authorizer, Service,
    ServiceLimitsV1,
};
use keystone_storage::{
    AcknowledgementStrength, Checkpoint, CheckpointStore, CommitOutcome, CommitReceipt, Revision,
    StoreRefusal,
};
use moira_hermes::{
    MAX_TRANSITION_BYTES_V1, PublicationV1, PublishErrorV1, WorkflowEventIdV1,
    WorkflowEventPublisher, encode_transition_v1,
};
use moira_protocol::{
    DispositionV1, EffectIdV1, MessageRefV1, TaskRefV1, TransitionV1, WorkflowIdV1,
};
use std::convert::Infallible;

#[derive(Default)]
struct Store(Option<Checkpoint>);
impl CheckpointStore for Store {
    type Error = Infallible;
    fn load(&self, _: &str) -> Result<Option<Checkpoint>, Self::Error> {
        Ok(self.0.clone())
    }
    fn compare_exchange(
        &mut self,
        _: &str,
        expected: Option<Revision>,
        bytes: &[u8],
    ) -> Result<CommitOutcome, Self::Error> {
        let actual = self.0.as_ref().map(|value| value.revision);
        if actual != expected {
            return Ok(CommitOutcome::Refused(StoreRefusal::RevisionConflict {
                expected,
                actual,
            }));
        }
        let revision = Revision(actual.map_or(0, |value| value.0 + 1));
        self.0 = Some(Checkpoint {
            revision,
            bytes: bytes.to_vec(),
            strength: AcknowledgementStrength::Durable,
        });
        Ok(CommitOutcome::Committed(CommitReceipt {
            revision,
            mechanism: "moira-hermes-test",
            strength: AcknowledgementStrength::Durable,
        }))
    }
}

struct Policy;
impl Authorizer for Policy {
    type Error = Infallible;
    fn authorize(
        &mut self,
        subject: [u8; 16],
        _: AuthorizationActionV1,
        _: AuthorizationResourceV1,
    ) -> Result<bool, Self::Error> {
        Ok(subject == [1; 16])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortError {
    LostReply,
    Service,
}

struct Port {
    service: Service<Store, Policy>,
    session: SessionHandleV1,
    lose_next_reply: bool,
    calls: usize,
}
impl ServicePort for Port {
    type Error = PortError;
    fn execute(
        &mut self,
        session: SessionHandleV1,
        command: CommandV1,
    ) -> Result<ReplyV1, Self::Error> {
        self.calls += 1;
        let reply = self
            .service
            .execute(session, command)
            .map_err(|_| PortError::Service)?;
        if self.lose_next_reply {
            self.lose_next_reply = false;
            return Err(PortError::LostReply);
        }
        Ok(reply)
    }
}

fn publication(transition: TransitionV1) -> PublicationV1 {
    PublicationV1 {
        stream: StreamIdV1([8; 16]),
        expected_predecessor: None,
        producer: ProducerIdV1([9; 16]),
        schema: SchemaIdV1([10; 16]),
        event: WorkflowEventIdV1([11; 16]),
        workflow: WorkflowIdV1([12; 16]),
        causation: None,
        transition,
    }
}

#[test]
fn lost_public_client_reply_replays_one_moira_owned_event() {
    let ledger = DurableLedger::create_new(
        Store::default(),
        Limits::default(),
        CheckpointBudgetV1::default(),
    )
    .unwrap();
    let mut service = Service::new(ledger, Policy, ServiceLimitsV1::default()).unwrap();
    let session = service
        .open_session(AuthenticatedSessionV1 { subject: [1; 16] })
        .unwrap();
    let port = Port {
        service,
        session,
        lose_next_reply: true,
        calls: 0,
    };
    let transition = TransitionV1::ChronosSubmissionPrepared {
        workflow: WorkflowIdV1([12; 16]),
        effect: EffectIdV1([11; 16]),
    };
    let expected_payload = encode_transition_v1(transition);
    let mut publisher = WorkflowEventPublisher::new(session, port);

    assert_eq!(
        publisher.publish(publication(transition)),
        Err(PublishErrorV1::Transport(PortError::LostReply))
    );
    assert_eq!(
        publisher.publish(publication(transition)).unwrap(),
        ReplyV1::Accepted(OutcomeV1::Append(AppendOutcomeV1::Replayed(
            EventSequenceV1(0)
        )))
    );
    let mut port = publisher.into_port();
    assert_eq!(port.calls, 2);
    let read = port
        .service
        .execute(
            port.session,
            CommandV1::Read {
                stream: StreamIdV1([8; 16]),
                start: EventSequenceV1(0),
                limit: 2,
            },
        )
        .unwrap();
    assert!(matches!(
        read,
        ReplyV1::Accepted(OutcomeV1::Read(batch))
            if batch.events.len() == 1
                && batch.events[0].envelope.payload == expected_payload
                && batch.events[0].envelope.correlation.0 == [12; 16]
                && batch.events[0].envelope.idempotency_key.0 == [11; 16]
    ));
}

#[test]
fn closed_transition_codec_is_distinct_bounded_and_rejects_wrong_correlation() {
    let workflow = WorkflowIdV1([12; 16]);
    let task = TaskRefV1 {
        external_task_key: [13; 16],
        incarnation: 14,
    };
    let transitions = [
        TransitionV1::ChronosSubmissionPrepared {
            workflow,
            effect: EffectIdV1([1; 16]),
        },
        TransitionV1::ChronosSubmissionAccepted { workflow, task },
        TransitionV1::DaimonExecutionPrepared {
            workflow,
            effect: EffectIdV1([2; 16]),
            task,
            attempt_number: 3,
        },
        TransitionV1::DaimonResultAccepted {
            workflow,
            disposition: DispositionV1::Completed,
        },
        TransitionV1::AgoraResponsePrepared {
            workflow,
            effect: EffectIdV1([4; 16]),
        },
        TransitionV1::AgoraPublicationAccepted {
            workflow,
            message: MessageRefV1 {
                source: [5; 16],
                room_sequence: 6,
                message_sequence: 7,
            },
        },
    ];
    for (tag, transition) in transitions.into_iter().enumerate() {
        let bytes = encode_transition_v1(transition);
        assert_eq!(&bytes[..8], b"MOIRAWF1");
        assert_eq!(bytes[8], tag as u8);
        assert!(bytes.len() <= MAX_TRANSITION_BYTES_V1);
    }

    let ledger = DurableLedger::create_new(
        Store::default(),
        Limits::default(),
        CheckpointBudgetV1::default(),
    )
    .unwrap();
    let mut service = Service::new(ledger, Policy, ServiceLimitsV1::default()).unwrap();
    let session = service
        .open_session(AuthenticatedSessionV1 { subject: [1; 16] })
        .unwrap();
    let port = Port {
        service,
        session,
        lose_next_reply: false,
        calls: 0,
    };
    let mut publisher = WorkflowEventPublisher::new(session, port);
    let mut wrong = publication(transitions[0]);
    wrong.workflow = WorkflowIdV1([99; 16]);
    assert_eq!(
        publisher.publish(wrong),
        Err(PublishErrorV1::WorkflowMismatch)
    );
    assert_eq!(publisher.into_port().calls, 0);
}
