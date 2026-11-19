//! Native Moira service over an explicit durable coordinator port.

use moira_core as core;
use moira_host::{ApplyErrorV1, CoordinatorStore};
use moira_protocol as protocol;

pub mod agent;
pub mod agent_participation;
pub mod attempt_lookup;
pub mod deliberation;
pub mod model_saga;
pub mod saga;
pub mod task_agent_journey;
pub mod tool_continuation;

pub struct Service<S> {
    store: S,
}

/// Verified connection facts supplied by an authentication adapter. These are
/// not credentials and are never persisted in workflow state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessV1 {
    pub read: bool,
    pub mutate: bool,
}

impl<S: CoordinatorStore> Service<S> {
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

    /// Executes only after connection authentication has supplied explicit
    /// access facts. Refusal occurs before touching the durable store.
    pub fn execute_authorized(
        &mut self,
        access: AccessV1,
        command: protocol::CommandV1,
    ) -> Result<protocol::ReplyV1, S::Error> {
        let allowed = match command {
            protocol::CommandV1::Apply { .. } => access.mutate,
            protocol::CommandV1::Status | protocol::CommandV1::Workflow { .. } => access.read,
        };
        if !allowed {
            return Ok(protocol::ReplyV1::Refused(
                protocol::RefusalV1::NotAuthorized,
            ));
        }
        match command {
            protocol::CommandV1::Apply {
                expected_revision,
                mutation,
            } => match self.store.apply(
                core::WorkflowRevision(expected_revision),
                mutation_core(mutation),
            ) {
                Ok(receipt) => Ok(protocol::ReplyV1::Accepted(protocol::OutcomeV1::Applied {
                    storage_revision: receipt.storage_revision,
                    transition: transition(receipt.transition),
                    replayed: receipt.replayed,
                })),
                Err(ApplyErrorV1::Domain(value)) => Ok(protocol::ReplyV1::Refused(refusal(value))),
                Err(ApplyErrorV1::Backend(error)) => Err(error),
            },
            protocol::CommandV1::Status => Ok(protocol::ReplyV1::Accepted(
                protocol::OutcomeV1::Status(protocol::StatusV1 {
                    revision: self.store.revision().0,
                    needs_reopen: self.store.needs_reopen(),
                }),
            )),
            protocol::CommandV1::Workflow { workflow } => {
                Ok(protocol::ReplyV1::Accepted(protocol::OutcomeV1::Workflow(
                    self.store
                        .workflow(core::WorkflowId::new(workflow.0))
                        .map(workflow_view),
                )))
            }
        }
    }

    pub fn into_store(self) -> S {
        self.store
    }
}

fn message(value: protocol::MessageRefV1) -> core::AgoraMessageRefV1 {
    core::AgoraMessageRefV1::new(value.source, value.room_sequence, value.message_sequence)
}

fn message_view(value: core::AgoraMessageRefV1) -> protocol::MessageRefV1 {
    protocol::MessageRefV1 {
        source: value.source(),
        room_sequence: value.room_sequence(),
        message_sequence: value.message_sequence(),
    }
}

fn task(value: protocol::TaskRefV1) -> core::ChronosTaskRefV1 {
    core::ChronosTaskRefV1::new(value.external_task_key, value.incarnation)
}

fn task_view(value: core::ChronosTaskRefV1) -> protocol::TaskRefV1 {
    protocol::TaskRefV1 {
        external_task_key: value.external_task_key(),
        incarnation: value.incarnation(),
    }
}

fn disposition(value: protocol::DispositionV1) -> core::DaimonDispositionV1 {
    match value {
        protocol::DispositionV1::Completed => core::DaimonDispositionV1::Completed,
        protocol::DispositionV1::Failed => core::DaimonDispositionV1::Failed,
        protocol::DispositionV1::Cancelled => core::DaimonDispositionV1::Cancelled,
        protocol::DispositionV1::TimedOut => core::DaimonDispositionV1::TimedOut,
        protocol::DispositionV1::Uncertain => core::DaimonDispositionV1::Uncertain,
    }
}

fn disposition_view(value: core::DaimonDispositionV1) -> protocol::DispositionV1 {
    match value {
        core::DaimonDispositionV1::Completed => protocol::DispositionV1::Completed,
        core::DaimonDispositionV1::Failed => protocol::DispositionV1::Failed,
        core::DaimonDispositionV1::Cancelled => protocol::DispositionV1::Cancelled,
        core::DaimonDispositionV1::TimedOut => protocol::DispositionV1::TimedOut,
        core::DaimonDispositionV1::Uncertain => protocol::DispositionV1::Uncertain,
    }
}

fn mutation_core(value: protocol::MutationV1) -> core::CommandV1 {
    match value {
        protocol::MutationV1::AdmitMessage {
            message: value,
            recipient,
        } => core::CommandV1::AdmitMessage {
            message: message(value),
            recipient: core::ParticipantRefV1::new(recipient.0),
        },
        protocol::MutationV1::AcceptChronosAdmission { workflow, receipt } => {
            core::CommandV1::AcceptChronosAdmission {
                workflow: core::WorkflowId::new(workflow.0),
                receipt: core::ChronosAdmissionReceiptV1::new(
                    core::EffectId::new(receipt.effect.0),
                    task(receipt.task),
                    receipt.chronos_revision,
                ),
            }
        }
        protocol::MutationV1::AcceptChronosAttempt { workflow, receipt } => {
            core::CommandV1::AcceptChronosAttempt {
                workflow: core::WorkflowId::new(workflow.0),
                receipt: core::ChronosAttemptReceiptV1::new(
                    task(receipt.task),
                    receipt.attempt_number,
                    receipt.chronos_revision,
                ),
            }
        }
        protocol::MutationV1::AcceptDaimonResult { workflow, receipt } => {
            core::CommandV1::AcceptDaimonResult {
                workflow: core::WorkflowId::new(workflow.0),
                receipt: core::DaimonResultReceiptV1::new(
                    core::EffectId::new(receipt.effect.0),
                    receipt.execution_id,
                    task(receipt.task),
                    receipt.attempt_number,
                    disposition(receipt.disposition),
                ),
            }
        }
        protocol::MutationV1::PrepareAgoraResponse { workflow } => {
            core::CommandV1::PrepareAgoraResponse {
                workflow: core::WorkflowId::new(workflow.0),
            }
        }
        protocol::MutationV1::AcceptAgoraPublication { workflow, receipt } => {
            core::CommandV1::AcceptAgoraPublication {
                workflow: core::WorkflowId::new(workflow.0),
                receipt: core::AgoraPublicationReceiptV1::new(
                    core::EffectId::new(receipt.effect.0),
                    message(receipt.message),
                    receipt.agora_revision,
                ),
            }
        }
    }
}

fn transition(value: core::TransitionV1) -> protocol::TransitionV1 {
    match value {
        core::TransitionV1::ChronosSubmissionPrepared {
            workflow,
            effect_id,
        } => protocol::TransitionV1::ChronosSubmissionPrepared {
            workflow: protocol::WorkflowIdV1(workflow.into_bytes()),
            effect: protocol::EffectIdV1(effect_id.into_bytes()),
        },
        core::TransitionV1::ChronosSubmissionAccepted {
            workflow,
            task: value,
        } => protocol::TransitionV1::ChronosSubmissionAccepted {
            workflow: protocol::WorkflowIdV1(workflow.into_bytes()),
            task: task_view(value),
        },
        core::TransitionV1::DaimonExecutionPrepared {
            workflow,
            effect_id,
            task: value,
            attempt_number,
        } => protocol::TransitionV1::DaimonExecutionPrepared {
            workflow: protocol::WorkflowIdV1(workflow.into_bytes()),
            effect: protocol::EffectIdV1(effect_id.into_bytes()),
            task: task_view(value),
            attempt_number,
        },
        core::TransitionV1::DaimonResultAccepted {
            workflow,
            disposition: value,
        } => protocol::TransitionV1::DaimonResultAccepted {
            workflow: protocol::WorkflowIdV1(workflow.into_bytes()),
            disposition: disposition_view(value),
        },
        core::TransitionV1::AgoraResponsePrepared {
            workflow,
            effect_id,
        } => protocol::TransitionV1::AgoraResponsePrepared {
            workflow: protocol::WorkflowIdV1(workflow.into_bytes()),
            effect: protocol::EffectIdV1(effect_id.into_bytes()),
        },
        core::TransitionV1::AgoraPublicationAccepted {
            workflow,
            message: value,
        } => protocol::TransitionV1::AgoraPublicationAccepted {
            workflow: protocol::WorkflowIdV1(workflow.into_bytes()),
            message: message_view(value),
        },
    }
}

fn refusal(value: core::RefusalV1) -> protocol::RefusalV1 {
    match value {
        core::RefusalV1::RevisionConflict { expected, actual } => {
            protocol::RefusalV1::RevisionConflict {
                expected: expected.0,
                actual: actual.0,
            }
        }
        core::RefusalV1::RevisionExhausted => protocol::RefusalV1::RevisionExhausted,
        core::RefusalV1::WorkflowIdentityExhausted => {
            protocol::RefusalV1::WorkflowIdentityExhausted
        }
        core::RefusalV1::EffectIdentityExhausted => protocol::RefusalV1::EffectIdentityExhausted,
        core::RefusalV1::WorkflowLimit => protocol::RefusalV1::WorkflowLimit,
        core::RefusalV1::InputConflict => protocol::RefusalV1::InputConflict,
        core::RefusalV1::ReceiptConflict => protocol::RefusalV1::ReceiptConflict,
        core::RefusalV1::UnknownWorkflow => protocol::RefusalV1::UnknownWorkflow,
        core::RefusalV1::InvalidPhase => protocol::RefusalV1::InvalidPhase,
        core::RefusalV1::EffectMismatch => protocol::RefusalV1::EffectMismatch,
        core::RefusalV1::AttemptMismatch => protocol::RefusalV1::AttemptMismatch,
    }
}

fn phase(value: core::WorkflowPhaseV1) -> protocol::PhaseV1 {
    match value {
        core::WorkflowPhaseV1::Received => protocol::PhaseV1::Received,
        core::WorkflowPhaseV1::TaskSubmissionPending => protocol::PhaseV1::TaskSubmissionPending,
        core::WorkflowPhaseV1::TaskSubmitted => protocol::PhaseV1::TaskSubmitted,
        core::WorkflowPhaseV1::HarnessPending => protocol::PhaseV1::HarnessPending,
        core::WorkflowPhaseV1::HarnessFinished => protocol::PhaseV1::HarnessFinished,
        core::WorkflowPhaseV1::ResponsePublicationPending => {
            protocol::PhaseV1::ResponsePublicationPending
        }
        core::WorkflowPhaseV1::FencedUncertain => protocol::PhaseV1::FencedUncertain,
        core::WorkflowPhaseV1::Completed => protocol::PhaseV1::Completed,
        core::WorkflowPhaseV1::Failed => protocol::PhaseV1::Failed,
        core::WorkflowPhaseV1::Cancelled => protocol::PhaseV1::Cancelled,
    }
}

fn workflow_view(value: core::WorkflowSnapshotV1) -> protocol::WorkflowViewV1 {
    protocol::WorkflowViewV1 {
        workflow: protocol::WorkflowIdV1(value.workflow_id().into_bytes()),
        revision: value.revision().0,
        phase: phase(value.phase()),
        cause: message_view(value.cause()),
        recipient: protocol::ParticipantRefV1(value.recipient().into_bytes()),
        pending_effect: value
            .pending_effect()
            .map(|effect| protocol::EffectIdV1(effect.into_bytes())),
        task: value.chronos_task().map(task_view),
        attempt: value
            .chronos_attempt()
            .map(|attempt| protocol::AttemptReceiptV1 {
                task: task_view(attempt.task()),
                attempt_number: attempt.attempt_number(),
                chronos_revision: attempt.chronos_revision(),
            }),
        result_disposition: value
            .daimon_result()
            .map(|result| disposition_view(result.disposition())),
        publication: value
            .agora_publication()
            .map(|receipt| message_view(receipt.message())),
    }
}
