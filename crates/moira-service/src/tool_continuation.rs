//! Native service mapping for structured tool continuations.

use crate::AccessV1;
use moira_core::{self as core, tool_continuation as domain};
use moira_host::{ToolContinuationApplyErrorV1, ToolContinuationStore};
use moira_protocol::tool_continuation as protocol;

pub struct ToolContinuationService<S> {
    store: S,
}

impl<S: ToolContinuationStore> ToolContinuationService<S> {
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
                Err(ToolContinuationApplyErrorV1::Domain(value)) => {
                    Ok(protocol::ReplyV1::Refused(refusal(value)))
                }
                Err(ToolContinuationApplyErrorV1::Conflict { expected, actual }) => {
                    Ok(protocol::ReplyV1::Refused(
                        protocol::RefusalV1::StorageRevisionConflict { expected, actual },
                    ))
                }
                Err(ToolContinuationApplyErrorV1::Backend(error)) => Err(error),
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
            source_execution,
            source_attempt,
            calls,
        } => domain::CommandV1::Admit {
            workflow: core::WorkflowId::new(workflow.0),
            source_execution: core::EffectId::new(source_execution.0),
            source_attempt: domain::AttemptEvidenceV1 {
                task_key: source_attempt.task_key,
                incarnation: source_attempt.incarnation,
                attempt: source_attempt.attempt,
            },
            calls: calls.into_iter().map(call_domain).collect(),
        },
        protocol::MutationV1::Authorize {
            workflow,
            effect,
            gate,
        } => domain::CommandV1::Authorize {
            workflow: core::WorkflowId::new(workflow.0),
            effect: core::EffectId::new(effect.0),
            gate: gate_domain(gate),
        },
        protocol::MutationV1::Dispatch {
            workflow,
            effect,
            execution,
        } => domain::CommandV1::Dispatch {
            workflow: core::WorkflowId::new(workflow.0),
            effect: core::EffectId::new(effect.0),
            execution: domain::ToolExecutionIdV1(execution.0),
        },
        protocol::MutationV1::Accept { workflow, receipt } => domain::CommandV1::Accept {
            workflow: core::WorkflowId::new(workflow.0),
            receipt: receipt_domain(receipt),
        },
        protocol::MutationV1::Fail { workflow, failure } => domain::CommandV1::Fail {
            workflow: core::WorkflowId::new(workflow.0),
            failure: failure_domain(failure),
        },
        protocol::MutationV1::MarkUncertain { workflow, effect } => {
            domain::CommandV1::MarkUncertain {
                workflow: core::WorkflowId::new(workflow.0),
                effect: core::EffectId::new(effect.0),
            }
        }
    }
}

fn call_domain(value: protocol::ToolCallV1) -> domain::ToolCallV1 {
    domain::ToolCallV1 {
        request: domain::ToolRequestIdV1(value.request.0),
        capability: domain::ToolCapabilityIdV1(value.capability.0),
        provider_correlation: value.provider_correlation,
        arguments: value.arguments,
        budget: domain::ToolBudgetV1 {
            max_elapsed_millis: value.budget.max_elapsed_millis,
            max_output_bytes: value.budget.max_output_bytes,
        },
    }
}

fn gate_domain(value: protocol::ToolGateV1) -> domain::ToolGateV1 {
    domain::ToolGateV1 {
        authority: domain::AuthorityEvidenceV1 {
            decision: domain::AuthorityDecisionIdV1(value.authority.decision.0),
            subject: domain::AuthoritySubjectIdV1(value.authority.subject.0),
            capability: domain::ToolCapabilityIdV1(value.authority.capability.0),
            epoch: value.authority.epoch,
        },
        attempt: domain::AttemptEvidenceV1 {
            task_key: value.attempt.task_key,
            incarnation: value.attempt.incarnation,
            attempt: value.attempt.attempt,
        },
    }
}

fn receipt_domain(value: protocol::ToolReceiptV1) -> domain::ToolReceiptV1 {
    domain::ToolReceiptV1 {
        effect: core::EffectId::new(value.effect.0),
        capability: domain::ToolCapabilityIdV1(value.capability.0),
        execution: domain::ToolExecutionIdV1(value.execution.0),
        evidence_digest: value.evidence_digest,
        output_digest: value.output_digest,
        output_bytes: value.output_bytes,
    }
}

fn failure_domain(value: protocol::ToolFailureV1) -> domain::ToolFailureV1 {
    domain::ToolFailureV1 {
        effect: core::EffectId::new(value.effect.0),
        capability: domain::ToolCapabilityIdV1(value.capability.0),
        execution: domain::ToolExecutionIdV1(value.execution.0),
        evidence_digest: value.evidence_digest,
    }
}

fn call_view(value: domain::ToolCallV1) -> protocol::ToolCallV1 {
    protocol::ToolCallV1 {
        request: protocol::ToolRequestIdV1(value.request.0),
        capability: protocol::ToolCapabilityIdV1(value.capability.0),
        provider_correlation: value.provider_correlation,
        arguments: value.arguments,
        budget: protocol::ToolBudgetV1 {
            max_elapsed_millis: value.budget.max_elapsed_millis,
            max_output_bytes: value.budget.max_output_bytes,
        },
    }
}

fn gate_view(value: domain::ToolGateV1) -> protocol::ToolGateV1 {
    protocol::ToolGateV1 {
        authority: protocol::AuthorityEvidenceV1 {
            decision: protocol::AuthorityDecisionIdV1(value.authority.decision.0),
            subject: protocol::AuthoritySubjectIdV1(value.authority.subject.0),
            capability: protocol::ToolCapabilityIdV1(value.authority.capability.0),
            epoch: value.authority.epoch,
        },
        attempt: protocol::AttemptEvidenceV1 {
            task_key: value.attempt.task_key,
            incarnation: value.attempt.incarnation,
            attempt: value.attempt.attempt,
        },
    }
}

fn receipt_view(value: domain::ToolReceiptV1) -> protocol::ToolReceiptV1 {
    protocol::ToolReceiptV1 {
        effect: moira_protocol::EffectIdV1(value.effect.into_bytes()),
        capability: protocol::ToolCapabilityIdV1(value.capability.0),
        execution: protocol::ToolExecutionIdV1(value.execution.0),
        evidence_digest: value.evidence_digest,
        output_digest: value.output_digest,
        output_bytes: value.output_bytes,
    }
}

fn failure_view(value: domain::ToolFailureV1) -> protocol::ToolFailureV1 {
    protocol::ToolFailureV1 {
        effect: moira_protocol::EffectIdV1(value.effect.into_bytes()),
        capability: protocol::ToolCapabilityIdV1(value.capability.0),
        execution: protocol::ToolExecutionIdV1(value.execution.0),
        evidence_digest: value.evidence_digest,
    }
}

fn state_view(value: domain::ToolEffectStateV1) -> protocol::ToolEffectStateV1 {
    match value {
        domain::ToolEffectStateV1::Queued => protocol::ToolEffectStateV1::Queued,
        domain::ToolEffectStateV1::PendingAuthorization => {
            protocol::ToolEffectStateV1::PendingAuthorization
        }
        domain::ToolEffectStateV1::Ready(gate) => {
            protocol::ToolEffectStateV1::Ready(gate_view(gate))
        }
        domain::ToolEffectStateV1::InFlight { gate, execution } => {
            protocol::ToolEffectStateV1::InFlight {
                gate: gate_view(gate),
                execution: protocol::ToolExecutionIdV1(execution.0),
            }
        }
        domain::ToolEffectStateV1::Succeeded { gate, receipt } => {
            protocol::ToolEffectStateV1::Succeeded {
                gate: gate_view(gate),
                receipt: receipt_view(receipt),
            }
        }
        domain::ToolEffectStateV1::Failed { gate, failure } => {
            protocol::ToolEffectStateV1::Failed {
                gate: gate_view(gate),
                failure: failure_view(failure),
            }
        }
        domain::ToolEffectStateV1::Uncertain { gate, execution } => {
            protocol::ToolEffectStateV1::Uncertain {
                gate: gate_view(gate),
                execution: protocol::ToolExecutionIdV1(execution.0),
            }
        }
    }
}

fn saga_view(value: domain::SnapshotV1) -> protocol::SagaViewV1 {
    protocol::SagaViewV1 {
        workflow: moira_protocol::WorkflowIdV1(value.workflow.into_bytes()),
        source_execution: moira_protocol::EffectIdV1(value.source_execution.into_bytes()),
        source_attempt: protocol::AttemptEvidenceV1 {
            task_key: value.source_attempt.task_key,
            incarnation: value.source_attempt.incarnation,
            attempt: value.source_attempt.attempt,
        },
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
            .map(|effect| protocol::ToolEffectV1 {
                effect: moira_protocol::EffectIdV1(effect.effect.into_bytes()),
                call: call_view(effect.call),
                state: state_view(effect.state),
            })
            .collect(),
    }
}

fn refusal(value: domain::RefusalV1) -> protocol::RefusalV1 {
    match value {
        domain::RefusalV1::InvalidLimits => protocol::RefusalV1::InvalidLimits,
        domain::RefusalV1::Limit => protocol::RefusalV1::Limit,
        domain::RefusalV1::Conflict => protocol::RefusalV1::Conflict,
        domain::RefusalV1::DuplicateRequest => protocol::RefusalV1::DuplicateRequest,
        domain::RefusalV1::UnknownSaga => protocol::RefusalV1::UnknownSaga,
        domain::RefusalV1::UnknownEffect => protocol::RefusalV1::UnknownEffect,
        domain::RefusalV1::NotPendingAuthorization => protocol::RefusalV1::NotPendingAuthorization,
        domain::RefusalV1::NotReady => protocol::RefusalV1::NotReady,
        domain::RefusalV1::NotInFlight => protocol::RefusalV1::NotInFlight,
        domain::RefusalV1::InvalidEvidence => protocol::RefusalV1::InvalidEvidence,
        domain::RefusalV1::ReceiptMismatch => protocol::RefusalV1::ReceiptMismatch,
        domain::RefusalV1::IdentityExhausted => protocol::RefusalV1::IdentityExhausted,
        domain::RefusalV1::InvalidCheckpoint => protocol::RefusalV1::InvalidCheckpoint,
    }
}
