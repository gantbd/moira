//! Durable prepare/outcome coordination for Agora response publication.
//!
//! The store owns the client request namespace and the last exact outcome.
//! Preparing the request before dispatch closes the process-crash window;
//! retaining the outcome before returning closes the lost store-reply window.

use super::{
    PublicationIntentV1, PublicationLimitsV1, PublicationReplyV1, PublishErrorV1,
    ResponsePublisher, ResponsePublisherStateV1, publication_command, valid_intent,
    validate_limits,
};
use agora_client::v2::{ClientStateV2, ServicePortV2};
use agora_protocol::v2::{ClientIdV2, RequestIdV2, RequestV2};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedPublicationV1 {
    pub intent: PublicationIntentV1,
    pub reply: PublicationReplyV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublisherCheckpointV1 {
    pub client: ResponsePublisherStateV1,
    pub last: Option<RetainedPublicationV1>,
}

impl PublisherCheckpointV1 {
    pub const fn initial(client: ClientIdV2) -> Self {
        Self {
            client: ResponsePublisherStateV1 {
                client: ClientStateV2 {
                    client,
                    next_request: 0,
                    pending: None,
                },
                pending_intent: None,
            },
            last: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublisherRecordV1 {
    pub revision: u64,
    pub checkpoint: PublisherCheckpointV1,
}

/// A single-owner compare/exchange store. An error may be indeterminate: the
/// caller must reload rather than assume the proposed checkpoint was rejected.
pub trait PublisherStateStoreV1 {
    type Error;

    fn load(&mut self) -> Result<PublisherRecordV1, Self::Error>;

    fn compare_exchange(
        &mut self,
        expected_revision: u64,
        checkpoint: &PublisherCheckpointV1,
    ) -> Result<PublisherRecordV1, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurablePublicationV1 {
    pub reply: PublicationReplyV1,
    pub replayed: bool,
    pub storage_revision: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub enum DurablePublishErrorV1<StoreError, PortError> {
    Store(StoreError),
    InvalidCheckpoint,
    InvalidInput,
    RequestIdExhausted,
    Publisher(PublishErrorV1<PortError>),
}

pub struct DurableResponsePublisher<S, P> {
    store: S,
    port: Option<P>,
    limits: PublicationLimitsV1,
}

impl<S, P> DurableResponsePublisher<S, P>
where
    S: PublisherStateStoreV1,
    P: ServicePortV2,
{
    pub fn new(
        store: S,
        port: P,
        limits: PublicationLimitsV1,
    ) -> Result<Self, DurablePublishErrorV1<S::Error, P::Error>> {
        validate_limits::<P::Error>(limits).map_err(|_| DurablePublishErrorV1::InvalidInput)?;
        Ok(Self {
            store,
            port: Some(port),
            limits,
        })
    }

    pub fn publish(
        &mut self,
        intent: PublicationIntentV1,
    ) -> Result<DurablePublicationV1, DurablePublishErrorV1<S::Error, P::Error>> {
        if !valid_intent(self.limits, &intent) {
            return Err(DurablePublishErrorV1::InvalidInput);
        }
        let mut retained = self.store.load().map_err(DurablePublishErrorV1::Store)?;
        validate_checkpoint(self.limits, &retained.checkpoint)?;
        if let Some(last) = &retained.checkpoint.last
            && last.intent == intent
        {
            return Ok(DurablePublicationV1 {
                reply: last.reply,
                replayed: true,
                storage_revision: retained.revision,
            });
        }

        if retained.checkpoint.client.pending_intent.is_none() {
            let next = retained
                .checkpoint
                .client
                .client
                .next_request
                .checked_add(1)
                .ok_or(DurablePublishErrorV1::RequestIdExhausted)?;
            let request = RequestV2 {
                request_id: RequestIdV2(retained.checkpoint.client.client.next_request),
                command: publication_command(&intent),
            };
            let mut prepared = retained.checkpoint.clone();
            prepared.client.client.next_request = next;
            prepared.client.client.pending = Some(request);
            prepared.client.pending_intent = Some(intent.clone());
            retained = self
                .store
                .compare_exchange(retained.revision, &prepared)
                .map_err(DurablePublishErrorV1::Store)?;
            if retained.checkpoint != prepared {
                return Err(DurablePublishErrorV1::InvalidCheckpoint);
            }
        } else if retained.checkpoint.client.pending_intent.as_ref() != Some(&intent) {
            return Err(DurablePublishErrorV1::InvalidCheckpoint);
        }

        validate_checkpoint(self.limits, &retained.checkpoint)?;
        let port = self
            .port
            .take()
            .ok_or(DurablePublishErrorV1::InvalidCheckpoint)?;
        let mut publisher =
            ResponsePublisher::restore(port, self.limits, retained.checkpoint.client.clone())
                .map_err(|_| DurablePublishErrorV1::InvalidCheckpoint)?;
        let outcome = publisher.publish(intent.clone());
        let state = publisher.state();
        let (port, _, _) = publisher.into_parts();
        self.port = Some(port);
        let reply = outcome.map_err(DurablePublishErrorV1::Publisher)?;
        let completed = PublisherCheckpointV1 {
            client: state,
            last: Some(RetainedPublicationV1 { intent, reply }),
        };
        let committed = self
            .store
            .compare_exchange(retained.revision, &completed)
            .map_err(DurablePublishErrorV1::Store)?;
        if committed.checkpoint != completed {
            return Err(DurablePublishErrorV1::InvalidCheckpoint);
        }
        Ok(DurablePublicationV1 {
            reply,
            replayed: false,
            storage_revision: committed.revision,
        })
    }

    pub fn into_parts(self) -> (S, Option<P>) {
        (self.store, self.port)
    }
}

fn validate_checkpoint<StoreError, PortError>(
    limits: PublicationLimitsV1,
    checkpoint: &PublisherCheckpointV1,
) -> Result<(), DurablePublishErrorV1<StoreError, PortError>> {
    match (
        &checkpoint.client.pending_intent,
        &checkpoint.client.client.pending,
    ) {
        (None, None) => {}
        (Some(intent), Some(request))
            if valid_intent(limits, intent)
                && checkpoint.client.client.next_request.checked_sub(1)
                    == Some(request.request_id.0)
                && request.command == publication_command(intent) => {}
        _ => return Err(DurablePublishErrorV1::InvalidCheckpoint),
    }
    if checkpoint.last.as_ref().is_some_and(|last| {
        !valid_intent(limits, &last.intent) || !reply_matches(&last.intent, last.reply)
    }) {
        return Err(DurablePublishErrorV1::InvalidCheckpoint);
    }
    Ok(())
}

fn reply_matches(intent: &PublicationIntentV1, reply: PublicationReplyV1) -> bool {
    match reply {
        PublicationReplyV1::Published(published) => {
            published.receipt.effect == intent.effect
                && published.receipt.message.source == intent.source
                && published.receipt.message.room_sequence == intent.room_sequence
        }
        PublicationReplyV1::Refused(_) => true,
    }
}
