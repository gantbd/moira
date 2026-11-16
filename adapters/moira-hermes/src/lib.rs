//! Publishes Moira-owned workflow transition records through Hermes' public
//! append client. This adapter owns the event schema; Hermes remains opaque.

#![forbid(unsafe_code)]

use hermes_client::{AppendClient, ServicePort};
use hermes_protocol::{
    CausationIdV1, CorrelationIdV1, EventEnvelopeV1, EventSequenceV1, IdempotencyKeyV1,
    ProducerIdV1, ReplyV1, SchemaIdV1, SessionHandleV1, StreamIdV1,
};
use moira_protocol::{DispositionV1, TransitionV1, WorkflowIdV1};

const MAGIC: &[u8; 8] = b"MOIRAWF1";
pub const MAX_TRANSITION_BYTES_V1: usize = 69;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct WorkflowEventIdV1(pub [u8; 16]);

impl std::fmt::Debug for WorkflowEventIdV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WorkflowEventIdV1(<opaque>)")
    }
}

pub struct PublicationV1 {
    pub stream: StreamIdV1,
    pub expected_predecessor: Option<EventSequenceV1>,
    pub producer: ProducerIdV1,
    pub schema: SchemaIdV1,
    pub event: WorkflowEventIdV1,
    pub workflow: WorkflowIdV1,
    pub causation: Option<CausationIdV1>,
    pub transition: TransitionV1,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PublishErrorV1<E> {
    WorkflowMismatch,
    Transport(E),
}

pub struct WorkflowEventPublisher<P>(AppendClient<P>);

impl<P: ServicePort> WorkflowEventPublisher<P> {
    pub const fn new(session: SessionHandleV1, port: P) -> Self {
        Self(AppendClient::new(session, port))
    }

    pub fn publish(
        &mut self,
        publication: PublicationV1,
    ) -> Result<ReplyV1, PublishErrorV1<P::Error>> {
        if transition_workflow(publication.transition) != publication.workflow {
            return Err(PublishErrorV1::WorkflowMismatch);
        }
        let payload = encode_transition_v1(publication.transition);
        self.0
            .append(
                publication.stream,
                publication.expected_predecessor,
                EventEnvelopeV1 {
                    producer: publication.producer,
                    schema: publication.schema,
                    idempotency_key: IdempotencyKeyV1(publication.event.0),
                    correlation: CorrelationIdV1(publication.workflow.0),
                    causation: publication.causation,
                    payload,
                },
            )
            .map_err(PublishErrorV1::Transport)
    }

    pub fn into_port(self) -> P {
        self.0.into_port()
    }
}

/// Canonical V1 bytes for the closed legacy workflow transition family.
pub fn encode_transition_v1(value: TransitionV1) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(MAX_TRANSITION_BYTES_V1);
    bytes.extend_from_slice(MAGIC);
    match value {
        TransitionV1::ChronosSubmissionPrepared { workflow, effect } => {
            bytes.push(0);
            bytes.extend_from_slice(&workflow.0);
            bytes.extend_from_slice(&effect.0);
        }
        TransitionV1::ChronosSubmissionAccepted { workflow, task } => {
            bytes.push(1);
            bytes.extend_from_slice(&workflow.0);
            bytes.extend_from_slice(&task.external_task_key);
            bytes.extend_from_slice(&task.incarnation.to_be_bytes());
        }
        TransitionV1::DaimonExecutionPrepared {
            workflow,
            effect,
            task,
            attempt_number,
        } => {
            bytes.push(2);
            bytes.extend_from_slice(&workflow.0);
            bytes.extend_from_slice(&effect.0);
            bytes.extend_from_slice(&task.external_task_key);
            bytes.extend_from_slice(&task.incarnation.to_be_bytes());
            bytes.extend_from_slice(&attempt_number.to_be_bytes());
        }
        TransitionV1::DaimonResultAccepted {
            workflow,
            disposition,
        } => {
            bytes.push(3);
            bytes.extend_from_slice(&workflow.0);
            bytes.push(disposition_tag(disposition));
        }
        TransitionV1::AgoraResponsePrepared { workflow, effect } => {
            bytes.push(4);
            bytes.extend_from_slice(&workflow.0);
            bytes.extend_from_slice(&effect.0);
        }
        TransitionV1::AgoraPublicationAccepted { workflow, message } => {
            bytes.push(5);
            bytes.extend_from_slice(&workflow.0);
            bytes.extend_from_slice(&message.source);
            bytes.extend_from_slice(&message.room_sequence.to_be_bytes());
            bytes.extend_from_slice(&message.message_sequence.to_be_bytes());
        }
    }
    bytes
}

const fn transition_workflow(value: TransitionV1) -> WorkflowIdV1 {
    match value {
        TransitionV1::ChronosSubmissionPrepared { workflow, .. }
        | TransitionV1::ChronosSubmissionAccepted { workflow, .. }
        | TransitionV1::DaimonExecutionPrepared { workflow, .. }
        | TransitionV1::DaimonResultAccepted { workflow, .. }
        | TransitionV1::AgoraResponsePrepared { workflow, .. }
        | TransitionV1::AgoraPublicationAccepted { workflow, .. } => workflow,
    }
}

const fn disposition_tag(value: DispositionV1) -> u8 {
    match value {
        DispositionV1::Completed => 0,
        DispositionV1::Failed => 1,
        DispositionV1::Cancelled => 2,
        DispositionV1::TimedOut => 3,
        DispositionV1::Uncertain => 4,
    }
}
