//! Drives provider proposals through Moira's durable peer-effect saga.
//!
//! Projection is deliberately separate from dispatch: trusted composition
//! supplies deployment facts omitted from model output, while peer adapters
//! return receipts rather than allowing a proposal to stand in for success.

use crate::action_selection::SelectedPeerActionV1;
use moira_client::{ModelSagaMutationClient, ModelSagaServicePort};
use moira_daimon::{ModelProposalV1, ModelResultV1};
use moira_protocol::{DispositionV1, WorkflowIdV1, model_saga as saga};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerEffectOutcomeV1 {
    Succeeded(saga::PeerReceiptV1),
    Failed,
    Uncertain,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerReconcileOutcomeV1 {
    Succeeded(saga::PeerReceiptV1),
    Uncertain,
}

/// Deployment-owned peer access. `reconcile` must use a peer's retained
/// request or historical receipt lookup; it must never issue a fresh effect.
pub trait PeerEffectPortV1 {
    type Error;

    fn execute(
        &mut self,
        effect: &saga::ProposalEffectV1,
    ) -> Result<PeerEffectOutcomeV1, Self::Error>;

    fn reconcile(
        &mut self,
        effect: &saga::ProposalEffectV1,
    ) -> Result<PeerReconcileOutcomeV1, Self::Error>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum PeerTurnOutcomeV1 {
    Completed {
        storage_revision: u64,
        saga: saga::SagaViewV1,
    },
    Failed {
        storage_revision: u64,
        saga: saga::SagaViewV1,
    },
    Uncertain {
        storage_revision: u64,
        saga: saga::SagaViewV1,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub enum PeerTurnErrorV1<ServiceError, PeerError> {
    Service(ServiceError),
    Peer(PeerError),
    Refused(saga::RefusalV1),
    InvalidState,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PeerProjectionErrorV1<E> {
    NonCompleted,
    Extent(E),
}

#[derive(Debug, Eq, PartialEq)]
pub enum SelectedPeerTurnErrorV1<ProjectionError, ServiceError, PeerError> {
    Projection(PeerProjectionErrorV1<ProjectionError>),
    Turn(PeerTurnErrorV1<ServiceError, PeerError>),
}

/// Project and execute only a result carrying a previously validated action
/// selection. The same digest is retained by the durable peer saga.
pub fn execute_selected_peer_turn<P, X, F, ProjectionError>(
    client: &mut ModelSagaMutationClient<P>,
    peers: &mut X,
    expected_storage_revision: Option<u64>,
    workflow: WorkflowIdV1,
    selected: SelectedPeerActionV1<'_>,
    vellum_extent: F,
) -> Result<PeerTurnOutcomeV1, SelectedPeerTurnErrorV1<ProjectionError, P::Error, X::Error>>
where
    P: ModelSagaServicePort,
    X: PeerEffectPortV1,
    F: FnMut([u8; 16], [u8; 16], u64) -> Result<u64, ProjectionError>,
{
    let proposals = project_peer_proposals(selected.result, vellum_extent)
        .map_err(SelectedPeerTurnErrorV1::Projection)?;
    execute_peer_turn(
        client,
        peers,
        expected_storage_revision,
        workflow,
        selected.selection_digest,
        selected.result,
        proposals,
    )
    .map_err(SelectedPeerTurnErrorV1::Turn)
}

/// Convert provider proposals to durable saga proposals without granting
/// provider output any deployment authority. Tool calls are handled by the
/// independent tool-continuation saga. Vellum source extent is resolved from
/// trusted retained context, not inferred from replacement bytes.
pub fn project_peer_proposals<E, F>(
    result: &ModelResultV1,
    mut vellum_extent: F,
) -> Result<Vec<saga::ProposalV1>, PeerProjectionErrorV1<E>>
where
    F: FnMut([u8; 16], [u8; 16], u64) -> Result<u64, E>,
{
    if result.receipt.disposition != DispositionV1::Completed {
        return Err(PeerProjectionErrorV1::NonCompleted);
    }
    result
        .proposals
        .iter()
        .filter_map(|proposal| match proposal {
            ModelProposalV1::AgoraMessage {
                room,
                reply_to,
                body,
            } => Some(Ok(saga::ProposalV1::AgoraMessage {
                room: *room,
                reply_to: *reply_to,
                body: body.clone(),
            })),
            ModelProposalV1::VellumEdit {
                workspace,
                document,
                expected_revision,
                replacement,
            } => Some(
                vellum_extent(*workspace, *document, *expected_revision)
                    .map(|expected_extent| saga::ProposalV1::VellumEdit {
                        workspace: *workspace,
                        document: *document,
                        expected_revision: *expected_revision,
                        expected_extent,
                        replacement: replacement.clone(),
                    })
                    .map_err(PeerProjectionErrorV1::Extent),
            ),
            ModelProposalV1::ToolCall { .. } => None,
        })
        .collect()
}

/// Admit and settle one model result's peer proposals in provider order.
/// Matching replay is side-effect free after completion. A retained uncertain
/// effect enters only `reconcile`; it is never passed to `execute` again.
pub fn execute_peer_turn<P, X>(
    client: &mut ModelSagaMutationClient<P>,
    peers: &mut X,
    expected_storage_revision: Option<u64>,
    workflow: WorkflowIdV1,
    selection_digest: [u8; 32],
    result: &ModelResultV1,
    proposals: Vec<saga::ProposalV1>,
) -> Result<PeerTurnOutcomeV1, PeerTurnErrorV1<P::Error, X::Error>>
where
    P: ModelSagaServicePort,
    X: PeerEffectPortV1,
{
    let reply = client
        .apply(
            expected_storage_revision,
            saga::MutationV1::Admit {
                workflow,
                daimon_effect: result.receipt.effect,
                selection_digest,
                proposals,
            },
        )
        .map_err(PeerTurnErrorV1::Service)?;
    let (revision, retained) = applied(reply)?;
    settle_peer_turn(peers, revision, retained, |expected, mutation| {
        client.apply(expected, mutation)
    })
}

/// Continue an already admitted peer saga from a trusted service read.
///
/// This is the crash-recovery boundary after action selection and admission:
/// the retained saga already binds the original selection digest and projected
/// proposals, so recovery neither asks the selector for a new decision nor
/// reconstructs deployment facts from model output.
pub fn resume_peer_turn<P, X>(
    port: &mut P,
    peers: &mut X,
    workflow: WorkflowIdV1,
) -> Result<PeerTurnOutcomeV1, PeerTurnErrorV1<P::Error, X::Error>>
where
    P: ModelSagaServicePort,
    X: PeerEffectPortV1,
{
    let status = port
        .execute(saga::CommandV1::Status)
        .map_err(PeerTurnErrorV1::Service)?;
    let saga::ReplyV1::Accepted(saga::OutcomeV1::Status(status)) = status else {
        return refused_or_invalid(status);
    };
    let Some(storage_revision) = status.storage_revision else {
        return Err(PeerTurnErrorV1::InvalidState);
    };
    if status.needs_reopen {
        return Err(PeerTurnErrorV1::InvalidState);
    }
    let retained = port
        .execute(saga::CommandV1::Saga { workflow })
        .map_err(PeerTurnErrorV1::Service)?;
    let saga::ReplyV1::Accepted(saga::OutcomeV1::Saga(Some(retained))) = retained else {
        return refused_or_invalid(retained);
    };
    settle_peer_turn(peers, storage_revision, retained, |expected, mutation| {
        port.execute(saga::CommandV1::Apply {
            expected_storage_revision: expected,
            mutation,
        })
    })
}

fn settle_peer_turn<X, ServiceError, Apply>(
    peers: &mut X,
    mut storage_revision: u64,
    mut retained: saga::SagaViewV1,
    mut apply: Apply,
) -> Result<PeerTurnOutcomeV1, PeerTurnErrorV1<ServiceError, X::Error>>
where
    X: PeerEffectPortV1,
    Apply: FnMut(Option<u64>, saga::MutationV1) -> Result<saga::ReplyV1, ServiceError>,
{
    loop {
        match retained.phase {
            saga::PhaseV1::Completed => {
                return Ok(PeerTurnOutcomeV1::Completed {
                    storage_revision,
                    saga: retained,
                });
            }
            saga::PhaseV1::Failed { .. } => {
                return Ok(PeerTurnOutcomeV1::Failed {
                    storage_revision,
                    saga: retained,
                });
            }
            saga::PhaseV1::Pending => {
                let active = retained
                    .effects
                    .iter()
                    .find(|effect| effect.state == saga::EffectStateV1::Pending)
                    .cloned()
                    .ok_or(PeerTurnErrorV1::InvalidState)?;
                let outcome = peers.execute(&active).map_err(PeerTurnErrorV1::Peer)?;
                let mutation = mutation_for(retained.workflow, active.effect, outcome);
                (storage_revision, retained) = applied(
                    apply(Some(storage_revision), mutation).map_err(PeerTurnErrorV1::Service)?,
                )?;
            }
            saga::PhaseV1::FencedUncertain { effect } => {
                let active = retained
                    .effects
                    .iter()
                    .find(|candidate| candidate.effect == effect)
                    .cloned()
                    .ok_or(PeerTurnErrorV1::InvalidState)?;
                let outcome = peers.reconcile(&active).map_err(PeerTurnErrorV1::Peer)?;
                let PeerReconcileOutcomeV1::Succeeded(receipt) = outcome else {
                    return Ok(PeerTurnOutcomeV1::Uncertain {
                        storage_revision,
                        saga: retained,
                    });
                };
                (storage_revision, retained) = applied(
                    apply(
                        Some(storage_revision),
                        saga::MutationV1::Reconcile {
                            workflow: retained.workflow,
                            receipt,
                        },
                    )
                    .map_err(PeerTurnErrorV1::Service)?,
                )?;
            }
        }
    }
}

fn refused_or_invalid<ServiceError, PeerError>(
    reply: saga::ReplyV1,
) -> Result<PeerTurnOutcomeV1, PeerTurnErrorV1<ServiceError, PeerError>> {
    match reply {
        saga::ReplyV1::Refused(refusal) => Err(PeerTurnErrorV1::Refused(refusal)),
        saga::ReplyV1::Accepted(_) => Err(PeerTurnErrorV1::InvalidState),
    }
}

fn mutation_for(
    workflow: WorkflowIdV1,
    effect: moira_protocol::EffectIdV1,
    outcome: PeerEffectOutcomeV1,
) -> saga::MutationV1 {
    match outcome {
        PeerEffectOutcomeV1::Succeeded(receipt) => saga::MutationV1::Accept { workflow, receipt },
        PeerEffectOutcomeV1::Failed => saga::MutationV1::Fail { workflow, effect },
        PeerEffectOutcomeV1::Uncertain => saga::MutationV1::MarkUncertain { workflow, effect },
    }
}

fn applied<ServiceError, PeerError>(
    reply: saga::ReplyV1,
) -> Result<(u64, saga::SagaViewV1), PeerTurnErrorV1<ServiceError, PeerError>> {
    match reply {
        saga::ReplyV1::Accepted(saga::OutcomeV1::Applied {
            storage_revision,
            saga,
            ..
        }) => Ok((storage_revision, saga)),
        saga::ReplyV1::Refused(refusal) => Err(PeerTurnErrorV1::Refused(refusal)),
        saga::ReplyV1::Accepted(_) => Err(PeerTurnErrorV1::InvalidState),
    }
}
