//! Native service mapping for the model-proposal saga contract.

use crate::AccessV1;
use moira_core::{self as core, model_saga as domain};
use moira_host::{ModelSagaApplyErrorV1, ModelSagaStore};
use moira_protocol::model_saga as protocol;

pub struct ModelSagaService<S> {
    store: S,
}

impl<S: ModelSagaStore> ModelSagaService<S> {
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    pub fn execute(&mut self, command: protocol::CommandV1) -> Result<protocol::ReplyV1, S::Error> {
        self.execute_authorized(
            AccessV1 {
                read: true,
                mutate: true,
            },
            command,
        )
    }

    pub fn execute_authorized(
        &mut self,
        access: AccessV1,
        command: protocol::CommandV1,
    ) -> Result<protocol::ReplyV1, S::Error> {
        let allowed = match command {
            protocol::CommandV1::Apply { .. } => access.mutate,
            protocol::CommandV1::Status | protocol::CommandV1::Saga { .. } => access.read,
        };
        if !allowed {
            return Ok(protocol::ReplyV1::Refused(
                protocol::RefusalV1::NotAuthorized,
            ));
        }
        match command {
            protocol::CommandV1::Apply {
                expected_storage_revision,
                mutation,
            } => match self
                .store
                .apply(expected_storage_revision, mutation_domain(mutation))
            {
                Ok(receipt) => Ok(protocol::ReplyV1::Accepted(protocol::OutcomeV1::Applied {
                    storage_revision: receipt.storage_revision,
                    saga: saga_view(receipt.transition.saga),
                    replayed: receipt.replayed,
                })),
                Err(ModelSagaApplyErrorV1::Domain(value)) => {
                    Ok(protocol::ReplyV1::Refused(refusal(value)))
                }
                Err(ModelSagaApplyErrorV1::Conflict { expected, actual }) => {
                    Ok(protocol::ReplyV1::Refused(
                        protocol::RefusalV1::StorageRevisionConflict { expected, actual },
                    ))
                }
                Err(ModelSagaApplyErrorV1::Backend(error)) => Err(error),
            },
            protocol::CommandV1::Status => Ok(protocol::ReplyV1::Accepted(
                protocol::OutcomeV1::Status(protocol::StatusV1 {
                    storage_revision: self.store.storage_revision(),
                    needs_reopen: self.store.needs_reopen(),
                }),
            )),
            protocol::CommandV1::Saga { workflow } => {
                Ok(protocol::ReplyV1::Accepted(protocol::OutcomeV1::Saga(
                    self.store
                        .saga(core::WorkflowId::new(workflow.0))
                        .map(saga_view),
                )))
            }
        }
    }

    pub fn into_store(self) -> S {
        self.store
    }
}

fn mutation_domain(value: protocol::MutationV1) -> domain::CommandV1 {
    match value {
        protocol::MutationV1::Admit {
            workflow,
            daimon_effect,
            selection_digest,
            proposals,
        } => domain::CommandV1::Admit {
            workflow: core::WorkflowId::new(workflow.0),
            daimon_effect: core::EffectId::new(daimon_effect.0),
            selection_digest,
            proposals: proposals.into_iter().map(proposal_domain).collect(),
        },
        protocol::MutationV1::Accept { workflow, receipt } => domain::CommandV1::Accept {
            workflow: core::WorkflowId::new(workflow.0),
            receipt: receipt_domain(receipt),
        },
        protocol::MutationV1::Fail { workflow, effect } => domain::CommandV1::Fail {
            workflow: core::WorkflowId::new(workflow.0),
            effect: core::EffectId::new(effect.0),
        },
        protocol::MutationV1::MarkUncertain { workflow, effect } => {
            domain::CommandV1::MarkUncertain {
                workflow: core::WorkflowId::new(workflow.0),
                effect: core::EffectId::new(effect.0),
            }
        }
        protocol::MutationV1::Reconcile { workflow, receipt } => domain::CommandV1::Reconcile {
            workflow: core::WorkflowId::new(workflow.0),
            receipt: receipt_domain(receipt),
        },
    }
}

fn proposal_domain(value: protocol::ProposalV1) -> domain::ProposalV1 {
    match value {
        protocol::ProposalV1::AgoraMessage {
            room,
            reply_to,
            body,
        } => domain::ProposalV1::AgoraMessage {
            room,
            reply_to,
            body,
        },
        protocol::ProposalV1::VellumEdit {
            workspace,
            document,
            expected_revision,
            expected_extent,
            replacement,
        } => domain::ProposalV1::VellumEdit {
            workspace,
            document,
            expected_revision,
            expected_extent,
            replacement,
        },
    }
}

fn receipt_domain(value: protocol::PeerReceiptV1) -> domain::PeerReceiptV1 {
    match value {
        protocol::PeerReceiptV1::AgoraMessage {
            effect,
            room,
            reply_to,
            message,
            agora_revision,
        } => domain::PeerReceiptV1::AgoraMessage {
            effect: core::EffectId::new(effect.0),
            room,
            reply_to,
            message,
            agora_revision,
        },
        protocol::PeerReceiptV1::VellumEdit {
            effect,
            workspace,
            document,
            source_revision,
            durable,
        } => domain::PeerReceiptV1::VellumEdit {
            effect: core::EffectId::new(effect.0),
            workspace,
            document,
            source_revision,
            durable,
        },
    }
}

fn proposal_view(value: domain::ProposalV1) -> protocol::ProposalV1 {
    match value {
        domain::ProposalV1::AgoraMessage {
            room,
            reply_to,
            body,
        } => protocol::ProposalV1::AgoraMessage {
            room,
            reply_to,
            body,
        },
        domain::ProposalV1::VellumEdit {
            workspace,
            document,
            expected_revision,
            expected_extent,
            replacement,
        } => protocol::ProposalV1::VellumEdit {
            workspace,
            document,
            expected_revision,
            expected_extent,
            replacement,
        },
    }
}

fn receipt_view(value: domain::PeerReceiptV1) -> protocol::PeerReceiptV1 {
    match value {
        domain::PeerReceiptV1::AgoraMessage {
            effect,
            room,
            reply_to,
            message,
            agora_revision,
        } => protocol::PeerReceiptV1::AgoraMessage {
            effect: moira_protocol::EffectIdV1(effect.into_bytes()),
            room,
            reply_to,
            message,
            agora_revision,
        },
        domain::PeerReceiptV1::VellumEdit {
            effect,
            workspace,
            document,
            source_revision,
            durable,
        } => protocol::PeerReceiptV1::VellumEdit {
            effect: moira_protocol::EffectIdV1(effect.into_bytes()),
            workspace,
            document,
            source_revision,
            durable,
        },
    }
}

fn saga_view(value: domain::SagaSnapshotV1) -> protocol::SagaViewV1 {
    protocol::SagaViewV1 {
        workflow: moira_protocol::WorkflowIdV1(value.workflow.into_bytes()),
        daimon_effect: moira_protocol::EffectIdV1(value.daimon_effect.into_bytes()),
        selection_digest: value.selection_digest,
        phase: match value.phase {
            domain::PhaseV1::Pending => protocol::PhaseV1::Pending,
            domain::PhaseV1::FencedUncertain { effect } => protocol::PhaseV1::FencedUncertain {
                effect: moira_protocol::EffectIdV1(effect.into_bytes()),
            },
            domain::PhaseV1::Completed => protocol::PhaseV1::Completed,
            domain::PhaseV1::Failed { effect } => protocol::PhaseV1::Failed {
                effect: moira_protocol::EffectIdV1(effect.into_bytes()),
            },
        },
        effects: value
            .effects
            .into_iter()
            .map(|effect| protocol::ProposalEffectV1 {
                effect: moira_protocol::EffectIdV1(effect.effect.into_bytes()),
                proposal: proposal_view(effect.proposal),
                state: match effect.state {
                    domain::EffectStateV1::Queued => protocol::EffectStateV1::Queued,
                    domain::EffectStateV1::Pending => protocol::EffectStateV1::Pending,
                    domain::EffectStateV1::Succeeded(receipt) => {
                        protocol::EffectStateV1::Succeeded(receipt_view(receipt))
                    }
                    domain::EffectStateV1::Failed => protocol::EffectStateV1::Failed,
                    domain::EffectStateV1::Uncertain => protocol::EffectStateV1::Uncertain,
                },
            })
            .collect(),
    }
}

fn refusal(value: domain::RefusalV1) -> protocol::RefusalV1 {
    match value {
        domain::RefusalV1::InvalidLimits => protocol::RefusalV1::InvalidLimits,
        domain::RefusalV1::InvalidSelection => protocol::RefusalV1::InvalidSelection,
        domain::RefusalV1::Limit => protocol::RefusalV1::Limit,
        domain::RefusalV1::Conflict => protocol::RefusalV1::Conflict,
        domain::RefusalV1::UnknownSaga => protocol::RefusalV1::UnknownSaga,
        domain::RefusalV1::UnknownEffect => protocol::RefusalV1::UnknownEffect,
        domain::RefusalV1::NotPending => protocol::RefusalV1::NotPending,
        domain::RefusalV1::ReceiptMismatch => protocol::RefusalV1::ReceiptMismatch,
        domain::RefusalV1::IdentityExhausted => protocol::RefusalV1::IdentityExhausted,
        domain::RefusalV1::InvalidCheckpoint => protocol::RefusalV1::InvalidCheckpoint,
    }
}
