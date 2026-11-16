//! Trusted deployment routing for selected Agora and Vellum peer effects.

use crate::peer_turn::{PeerEffectOutcomeV1, PeerEffectPortV1, PeerReconcileOutcomeV1};
use agora_client::v2::{ClientErrorV2, ServicePortV2};
use moira_agora::{
    PublicationIntentV1, PublicationReplyV1, PublishErrorV1,
    durable::{DurablePublishErrorV1, DurableResponsePublisher, PublisherStateStoreV1},
};
use moira_protocol::model_saga as saga;
use moira_vellum::{
    AdapterErrorV1 as VellumErrorV1, EditAdapter, EditIntentV1, EditReplyV1, ExternalDocumentRefV1,
};
use std::collections::BTreeMap;
use vellum_protocol::ServicePort as VellumServicePort;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExternalConversationRefV1 {
    pub room: [u8; 16],
    pub reply_to: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgoraRouteV1 {
    pub source: [u8; 16],
    pub room_sequence: u64,
    pub room_id: String,
    pub expected_agora_revision: u64,
    pub logical_time: u64,
    pub addressed_to: Vec<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PeerRouterErrorV1<StoreError, AgoraError> {
    UnknownAgoraRoute,
    InvalidAgoraBody,
    Agora(DurablePublishErrorV1<StoreError, AgoraError>),
    Vellum(VellumErrorV1),
}

/// Routes opaque model references through deployment-owned Agora/Vellum
/// identities. Agora uses durable exact-request replay; Vellum uses durable
/// operation execution and historical lookup.
pub struct FederatedPeerRouterV1<S, A, V> {
    agora: DurableResponsePublisher<S, A>,
    agora_routes: BTreeMap<ExternalConversationRefV1, AgoraRouteV1>,
    vellum: EditAdapter<V>,
}

impl<S, A, V> FederatedPeerRouterV1<S, A, V> {
    pub fn new(
        agora: DurableResponsePublisher<S, A>,
        agora_routes: BTreeMap<ExternalConversationRefV1, AgoraRouteV1>,
        vellum: EditAdapter<V>,
    ) -> Self {
        Self {
            agora,
            agora_routes,
            vellum,
        }
    }

    pub fn into_parts(
        self,
    ) -> (
        DurableResponsePublisher<S, A>,
        BTreeMap<ExternalConversationRefV1, AgoraRouteV1>,
        EditAdapter<V>,
    ) {
        (self.agora, self.agora_routes, self.vellum)
    }
}

impl<S, A, V> PeerEffectPortV1 for FederatedPeerRouterV1<S, A, V>
where
    S: PublisherStateStoreV1,
    A: ServicePortV2,
    V: VellumServicePort,
{
    type Error = PeerRouterErrorV1<S::Error, A::Error>;

    fn execute(
        &mut self,
        effect: &saga::ProposalEffectV1,
    ) -> Result<PeerEffectOutcomeV1, Self::Error> {
        match &effect.proposal {
            saga::ProposalV1::AgoraMessage {
                room,
                reply_to,
                body,
            } => self.publish_agora(effect.effect, *room, *reply_to, body, false),
            saga::ProposalV1::VellumEdit {
                workspace,
                document,
                expected_revision,
                expected_extent,
                replacement,
            } => {
                let intent = EditIntentV1 {
                    effect: effect.effect,
                    target: ExternalDocumentRefV1 {
                        workspace: *workspace,
                        document: *document,
                    },
                    expected_revision: *expected_revision,
                    expected_extent: *expected_extent,
                    replacement: replacement.clone(),
                };
                match self.vellum.execute(&intent).map_err(Self::vellum_error)? {
                    EditReplyV1::Completed { receipt, .. } => Ok(PeerEffectOutcomeV1::Succeeded(
                        saga::PeerReceiptV1::VellumEdit {
                            effect: effect.effect,
                            workspace: *workspace,
                            document: *document,
                            source_revision: receipt.source_revision,
                            durable: matches!(
                                receipt.persistence,
                                vellum_protocol::EditPersistenceV1::Durable(_)
                            ),
                        },
                    )),
                    EditReplyV1::OutcomeUnknown => Ok(PeerEffectOutcomeV1::Uncertain),
                    EditReplyV1::Refused(_)
                    | EditReplyV1::Retired
                    | EditReplyV1::AbsentOrForeign => Ok(PeerEffectOutcomeV1::Failed),
                }
            }
        }
    }

    fn reconcile(
        &mut self,
        effect: &saga::ProposalEffectV1,
    ) -> Result<PeerReconcileOutcomeV1, Self::Error> {
        match &effect.proposal {
            saga::ProposalV1::AgoraMessage {
                room,
                reply_to,
                body,
            } => match self.publish_agora(effect.effect, *room, *reply_to, body, true)? {
                PeerEffectOutcomeV1::Succeeded(receipt) => {
                    Ok(PeerReconcileOutcomeV1::Succeeded(receipt))
                }
                PeerEffectOutcomeV1::Failed | PeerEffectOutcomeV1::Uncertain => {
                    Ok(PeerReconcileOutcomeV1::Uncertain)
                }
            },
            saga::ProposalV1::VellumEdit {
                workspace,
                document,
                expected_revision,
                expected_extent,
                replacement,
            } => {
                let intent = EditIntentV1 {
                    effect: effect.effect,
                    target: ExternalDocumentRefV1 {
                        workspace: *workspace,
                        document: *document,
                    },
                    expected_revision: *expected_revision,
                    expected_extent: *expected_extent,
                    replacement: replacement.clone(),
                };
                match self.vellum.lookup(&intent).map_err(Self::vellum_error)? {
                    EditReplyV1::Completed { receipt, .. } => Ok(
                        PeerReconcileOutcomeV1::Succeeded(saga::PeerReceiptV1::VellumEdit {
                            effect: effect.effect,
                            workspace: *workspace,
                            document: *document,
                            source_revision: receipt.source_revision,
                            durable: matches!(
                                receipt.persistence,
                                vellum_protocol::EditPersistenceV1::Durable(_)
                            ),
                        }),
                    ),
                    EditReplyV1::Refused(_)
                    | EditReplyV1::Retired
                    | EditReplyV1::OutcomeUnknown
                    | EditReplyV1::AbsentOrForeign => Ok(PeerReconcileOutcomeV1::Uncertain),
                }
            }
        }
    }
}

impl<S, A, V> FederatedPeerRouterV1<S, A, V>
where
    S: PublisherStateStoreV1,
    A: ServicePortV2,
    V: VellumServicePort,
{
    fn publish_agora(
        &mut self,
        effect: moira_protocol::EffectIdV1,
        room: [u8; 16],
        reply_to: [u8; 16],
        body: &[u8],
        reconciling: bool,
    ) -> Result<PeerEffectOutcomeV1, PeerRouterErrorV1<S::Error, A::Error>> {
        let route = self
            .agora_routes
            .get(&ExternalConversationRefV1 { room, reply_to })
            .ok_or(PeerRouterErrorV1::UnknownAgoraRoute)?;
        let body =
            String::from_utf8(body.to_vec()).map_err(|_| PeerRouterErrorV1::InvalidAgoraBody)?;
        let intent = PublicationIntentV1 {
            effect,
            source: route.source,
            room_sequence: route.room_sequence,
            room_id: route.room_id.clone(),
            expected_agora_revision: route.expected_agora_revision,
            logical_time: route.logical_time,
            body,
            addressed_to: route.addressed_to.clone(),
        };
        match self.agora.publish(intent) {
            Ok(published) => match published.reply {
                PublicationReplyV1::Published(receipt) => Ok(PeerEffectOutcomeV1::Succeeded(
                    saga::PeerReceiptV1::AgoraMessage {
                        effect,
                        room,
                        reply_to,
                        message: receipt.message_id.into_bytes(),
                        agora_revision: receipt.receipt.agora_revision,
                    },
                )),
                PublicationReplyV1::Refused(_) if reconciling => Ok(PeerEffectOutcomeV1::Uncertain),
                PublicationReplyV1::Refused(_) => Ok(PeerEffectOutcomeV1::Failed),
            },
            Err(DurablePublishErrorV1::Publisher(PublishErrorV1::Client(
                ClientErrorV2::OutcomeUnknown(_),
            ))) => Ok(PeerEffectOutcomeV1::Uncertain),
            Err(error) => Err(PeerRouterErrorV1::Agora(error)),
        }
    }

    fn vellum_error(error: VellumErrorV1) -> PeerRouterErrorV1<S::Error, A::Error> {
        PeerRouterErrorV1::Vellum(error)
    }
}
