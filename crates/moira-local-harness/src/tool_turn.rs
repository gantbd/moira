//! Drives one provider-native tool turn through independent authority,
//! current-attempt observation, durable execution and exact result recovery.

use crate::tool_runtime::{
    ToolReconcileErrorV1, ToolReconcileOutcomeV1, ToolRunErrorV1, ToolRunOutcomeV1,
    ToolUncertaintyV1, dispatch_ready, reconcile_in_flight, resolve_completed_tool_results,
};
use moira_client::{ToolContinuationMutationClient, ToolContinuationServicePort};
use moira_daimon::{ModelResultV1, admit_tool_continuation, authorize_tool_effect};
use moira_host::{ToolExecutorV1, ToolOutputStoreV1};
use moira_protocol::{EffectIdV1, WorkflowIdV1, tool_continuation as tools};
use sha2::{Digest, Sha256};

#[derive(Debug, Eq, PartialEq)]
pub enum ToolTurnUncertaintyV1<ExecutorError, OutputError> {
    Dispatch(ToolUncertaintyV1<ExecutorError, OutputError>),
    RecoveryWithoutOutput,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ToolTurnOutcomeV1<ExecutorError, OutputError> {
    Completed {
        storage_revision: u64,
        saga: tools::SagaViewV1,
        results: Vec<moira_daimon::CompletedToolResultV1>,
    },
    Failed {
        storage_revision: u64,
        saga: tools::SagaViewV1,
    },
    Uncertain {
        storage_revision: u64,
        saga: tools::SagaViewV1,
        cause: ToolTurnUncertaintyV1<ExecutorError, OutputError>,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub enum ToolTurnErrorV1<ServiceError, AuthorityError, AttemptError, OutputError> {
    Service(ServiceError),
    Refused(tools::RefusalV1),
    InvalidState,
    Authority(AuthorityError),
    Attempt(AttemptError),
    Dispatch(ToolRunErrorV1<ServiceError>),
    Recovery(ToolReconcileErrorV1<ServiceError, OutputError>),
    Output(ToolReconcileErrorV1<std::convert::Infallible, OutputError>),
}

pub type ToolTurnResultV1<P, X, O, AuthorityError, AttemptError> = Result<
    ToolTurnOutcomeV1<<X as ToolExecutorV1>::Error, <O as ToolOutputStoreV1>::Error>,
    ToolTurnErrorV1<
        <P as ToolContinuationServicePort>::Error,
        AuthorityError,
        AttemptError,
        <O as ToolOutputStoreV1>::Error,
    >,
>;

/// Settle every call in one model turn in provider order. The caller supplies
/// live, independent Aegis and Chronos observations and a restart-stable
/// execution identity allocator. Matching retained admission is replay-safe;
/// retained in-flight state is reconciled from durable output and is never
/// blindly reinvoked.
#[allow(clippy::too_many_arguments)]
pub fn execute_tool_turn<P, X, O, A, T, AuthorityError, AttemptError>(
    client: &mut ToolContinuationMutationClient<P>,
    executor: &mut X,
    outputs: &mut O,
    expected_storage_revision: Option<u64>,
    workflow: WorkflowIdV1,
    result: &ModelResultV1,
    budget: tools::ToolBudgetV1,
    authorize: &mut A,
    observe_attempt: &mut T,
) -> ToolTurnResultV1<P, X, O, AuthorityError, AttemptError>
where
    P: ToolContinuationServicePort,
    X: ToolExecutorV1,
    O: ToolOutputStoreV1,
    A: FnMut(tools::ToolCapabilityIdV1) -> Result<tools::AuthorityEvidenceV1, AuthorityError>,
    T: FnMut([u8; 16], u64, u32) -> Result<tools::AttemptEvidenceV1, AttemptError>,
{
    let admitted = admit_tool_continuation(
        client,
        expected_storage_revision,
        workflow,
        result.receipt.effect,
        result,
        budget,
    )
    .map_err(ToolTurnErrorV1::Service)?
    .ok_or(ToolTurnErrorV1::InvalidState)?;
    let (mut revision, mut saga) = applied(admitted)?;

    loop {
        match saga.phase {
            tools::PhaseV1::Completed => {
                let results = resolve_completed_tool_results(outputs, &saga)
                    .map_err(ToolTurnErrorV1::Output)?;
                return Ok(ToolTurnOutcomeV1::Completed {
                    storage_revision: revision,
                    saga,
                    results,
                });
            }
            tools::PhaseV1::Failed { .. } => {
                return Ok(ToolTurnOutcomeV1::Failed {
                    storage_revision: revision,
                    saga,
                });
            }
            tools::PhaseV1::FencedUncertain { .. } => {
                return Ok(ToolTurnOutcomeV1::Uncertain {
                    storage_revision: revision,
                    saga,
                    cause: ToolTurnUncertaintyV1::RecoveryWithoutOutput,
                });
            }
            tools::PhaseV1::Pending => {}
        }

        let active = saga
            .effects
            .iter()
            .find(|effect| {
                matches!(
                    effect.state,
                    tools::ToolEffectStateV1::PendingAuthorization
                        | tools::ToolEffectStateV1::Ready(_)
                        | tools::ToolEffectStateV1::InFlight { .. }
                )
            })
            .cloned()
            .ok_or(ToolTurnErrorV1::InvalidState)?;

        match active.state {
            tools::ToolEffectStateV1::PendingAuthorization => {
                let attempt = observe_attempt(
                    saga.source_attempt.task_key,
                    saga.source_attempt.incarnation,
                    saga.source_attempt.attempt,
                )
                .map_err(ToolTurnErrorV1::Attempt)?;
                let authority =
                    authorize(active.call.capability).map_err(ToolTurnErrorV1::Authority)?;
                let reply = authorize_tool_effect(
                    client,
                    Some(revision),
                    workflow,
                    active.effect,
                    authority,
                    attempt,
                )
                .map_err(ToolTurnErrorV1::Service)?;
                (revision, saga) = applied(reply)?;
            }
            tools::ToolEffectStateV1::Ready(_) => {
                match dispatch_ready(
                    client,
                    executor,
                    outputs,
                    Some(revision),
                    &saga,
                    active.effect,
                    tool_execution_id(workflow, active.effect),
                )
                .map_err(ToolTurnErrorV1::Dispatch)?
                {
                    ToolRunOutcomeV1::Succeeded {
                        storage_revision,
                        saga: retained,
                        ..
                    } => {
                        revision = storage_revision;
                        saga = retained;
                    }
                    ToolRunOutcomeV1::Failed {
                        storage_revision,
                        saga,
                    } => {
                        return Ok(ToolTurnOutcomeV1::Failed {
                            storage_revision,
                            saga,
                        });
                    }
                    ToolRunOutcomeV1::Uncertain {
                        storage_revision,
                        saga,
                        cause,
                    } => {
                        return Ok(ToolTurnOutcomeV1::Uncertain {
                            storage_revision,
                            saga,
                            cause: ToolTurnUncertaintyV1::Dispatch(cause),
                        });
                    }
                }
            }
            tools::ToolEffectStateV1::InFlight { .. } => {
                match reconcile_in_flight(client, outputs, Some(revision), &saga, active.effect)
                    .map_err(ToolTurnErrorV1::Recovery)?
                {
                    ToolReconcileOutcomeV1::Succeeded {
                        storage_revision,
                        saga: retained,
                        ..
                    } => {
                        revision = storage_revision;
                        saga = retained;
                    }
                    ToolReconcileOutcomeV1::Uncertain {
                        storage_revision,
                        saga,
                    } => {
                        return Ok(ToolTurnOutcomeV1::Uncertain {
                            storage_revision,
                            saga,
                            cause: ToolTurnUncertaintyV1::RecoveryWithoutOutput,
                        });
                    }
                }
            }
            tools::ToolEffectStateV1::Queued
            | tools::ToolEffectStateV1::Succeeded { .. }
            | tools::ToolEffectStateV1::Failed { .. }
            | tools::ToolEffectStateV1::Uncertain { .. } => {
                return Err(ToolTurnErrorV1::InvalidState);
            }
        }
    }
}

/// Stable nonzero execution identity for one exact workflow effect. Derivation
/// is owned by the harness so restart behavior cannot depend on a caller-local
/// counter or random generator.
pub fn tool_execution_id(workflow: WorkflowIdV1, effect: EffectIdV1) -> tools::ToolExecutionIdV1 {
    let mut hash = Sha256::new();
    hash.update(b"moira-local-tool-execution-v1\0");
    hash.update(workflow.0);
    hash.update(effect.0);
    let digest: [u8; 32] = hash.finalize().into();
    let mut identity = [0_u8; 16];
    identity.copy_from_slice(&digest[..16]);
    identity[0] |= 0x80;
    tools::ToolExecutionIdV1(identity)
}

fn applied<ServiceError, AuthorityError, AttemptError, OutputError>(
    reply: tools::ReplyV1,
) -> Result<
    (u64, tools::SagaViewV1),
    ToolTurnErrorV1<ServiceError, AuthorityError, AttemptError, OutputError>,
> {
    match reply {
        tools::ReplyV1::Accepted(tools::OutcomeV1::Applied {
            storage_revision,
            saga,
            ..
        }) => Ok((storage_revision, saga)),
        tools::ReplyV1::Refused(refusal) => Err(ToolTurnErrorV1::Refused(refusal)),
        tools::ReplyV1::Accepted(_) => Err(ToolTurnErrorV1::InvalidState),
    }
}
