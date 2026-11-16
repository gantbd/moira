//! Crash-conservative execution of one already-authorized tool effect.

use moira_client::{ToolContinuationMutationClient, ToolContinuationServicePort};
use moira_core::tool_continuation as core_tools;
use moira_host::{
    ToolExecutionOutcomeV1, ToolExecutorV1, ToolInvocationV1, ToolOutputRecordV1, ToolOutputStoreV1,
};
use moira_protocol::{EffectIdV1, WorkflowIdV1, tool_continuation as tools};
use sha2::{Digest, Sha256};

#[derive(Debug, Eq, PartialEq)]
pub enum ToolRunErrorV1<ServiceError> {
    Service(ServiceError),
    Refused(tools::RefusalV1),
    InvalidState,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ToolUncertaintyV1<ExecutorError, OutputError> {
    Provider,
    Executor(ExecutorError),
    Output(OutputError),
    OutputIntegrity,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ToolRunOutcomeV1<ExecutorError, OutputError> {
    Succeeded {
        storage_revision: u64,
        saga: tools::SagaViewV1,
        record: ToolOutputRecordV1,
    },
    Failed {
        storage_revision: u64,
        saga: tools::SagaViewV1,
    },
    Uncertain {
        storage_revision: u64,
        saga: tools::SagaViewV1,
        cause: ToolUncertaintyV1<ExecutorError, OutputError>,
    },
}

pub type ToolRunResultV1<P, X, O> = Result<
    ToolRunOutcomeV1<<X as ToolExecutorV1>::Error, <O as ToolOutputStoreV1>::Error>,
    ToolRunErrorV1<<P as ToolContinuationServicePort>::Error>,
>;

#[derive(Debug, Eq, PartialEq)]
pub enum ToolReconcileErrorV1<ServiceError, OutputError> {
    Service(ServiceError),
    Output(OutputError),
    Refused(tools::RefusalV1),
    InvalidState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolReconcileOutcomeV1 {
    Succeeded {
        storage_revision: u64,
        saga: tools::SagaViewV1,
        record: ToolOutputRecordV1,
    },
    Uncertain {
        storage_revision: u64,
        saga: tools::SagaViewV1,
    },
}

/// Durably mark dispatch, invoke once, durably retain successful bytes, then
/// settle the continuation. Any ambiguity after dispatch fences the effect.
pub fn dispatch_ready<P, X, O>(
    client: &mut ToolContinuationMutationClient<P>,
    executor: &mut X,
    outputs: &mut O,
    expected_storage_revision: Option<u64>,
    saga: &tools::SagaViewV1,
    effect: EffectIdV1,
    execution: tools::ToolExecutionIdV1,
) -> ToolRunResultV1<P, X, O>
where
    P: ToolContinuationServicePort,
    X: ToolExecutorV1,
    O: ToolOutputStoreV1,
{
    if execution.0 == [0; 16] || saga.phase != tools::PhaseV1::Pending {
        return Err(ToolRunErrorV1::InvalidState);
    }
    let retained = saga
        .effects
        .iter()
        .find(|candidate| candidate.effect == effect)
        .ok_or(ToolRunErrorV1::InvalidState)?;
    let tools::ToolEffectStateV1::Ready(gate) = retained.state else {
        return Err(ToolRunErrorV1::InvalidState);
    };
    if gate.authority.capability != retained.call.capability || gate.attempt != saga.source_attempt
    {
        return Err(ToolRunErrorV1::InvalidState);
    }
    let (dispatch_revision, dispatched) = applied(
        client
            .apply(
                expected_storage_revision,
                tools::MutationV1::Dispatch {
                    workflow: saga.workflow,
                    effect,
                    execution,
                },
            )
            .map_err(ToolRunErrorV1::Service)?,
    )?;
    let dispatched_effect = dispatched
        .effects
        .iter()
        .find(|candidate| candidate.effect == effect)
        .ok_or(ToolRunErrorV1::InvalidState)?;
    if !matches!(
        dispatched_effect.state,
        tools::ToolEffectStateV1::InFlight {
            gate: retained_gate,
            execution: retained_execution
        } if retained_gate == gate && retained_execution == execution
    ) {
        return Err(ToolRunErrorV1::InvalidState);
    }
    let invocation = invocation(saga.workflow, effect, execution, gate, &retained.call);
    match executor.execute(&invocation) {
        Ok(ToolExecutionOutcomeV1::Succeeded {
            evidence_digest,
            output,
        }) if evidence_digest != [0; 32]
            && output.len() <= retained.call.budget.max_output_bytes =>
        {
            let record = match outputs.put(
                core_tools::ToolExecutionIdV1(execution.0),
                core_tools::ToolCapabilityIdV1(retained.call.capability.0),
                evidence_digest,
                &output,
            ) {
                Ok(commit) => commit.record,
                Err(error) => match outputs.find(core_tools::ToolExecutionIdV1(execution.0)) {
                    Ok(Some((record, stored_output)))
                        if stored_output == output
                            && record.execution.0 == execution.0
                            && record.capability.0 == retained.call.capability.0
                            && record.evidence_digest == evidence_digest
                            && record.output_digest != [0; 32]
                            && record.output_bytes == output.len() =>
                    {
                        record
                    }
                    _ => {
                        let (revision, saga) =
                            fence(client, dispatch_revision, saga.workflow, effect)?;
                        return Ok(ToolRunOutcomeV1::Uncertain {
                            storage_revision: revision,
                            saga,
                            cause: ToolUncertaintyV1::Output(error),
                        });
                    }
                },
            };
            let resolved = match outputs.get(record.output_digest) {
                Ok(value) => value,
                Err(error) => {
                    let (revision, saga) = fence(client, dispatch_revision, saga.workflow, effect)?;
                    return Ok(ToolRunOutcomeV1::Uncertain {
                        storage_revision: revision,
                        saga,
                        cause: ToolUncertaintyV1::Output(error),
                    });
                }
            };
            if record.execution.0 != execution.0
                || record.capability.0 != retained.call.capability.0
                || record.evidence_digest != evidence_digest
                || record.output_digest == [0; 32]
                || record.output_bytes != output.len()
                || resolved.as_deref() != Some(output.as_slice())
                || <[u8; 32]>::from(Sha256::digest(&output)) != record.output_digest
            {
                let (revision, saga) = fence(client, dispatch_revision, saga.workflow, effect)?;
                return Ok(ToolRunOutcomeV1::Uncertain {
                    storage_revision: revision,
                    saga,
                    cause: ToolUncertaintyV1::OutputIntegrity,
                });
            }
            let (storage_revision, saga) = applied(
                client
                    .apply(
                        Some(dispatch_revision),
                        tools::MutationV1::Accept {
                            workflow: saga.workflow,
                            receipt: tools::ToolReceiptV1 {
                                effect,
                                capability: retained.call.capability,
                                execution,
                                evidence_digest,
                                output_digest: record.output_digest,
                                output_bytes: record.output_bytes,
                            },
                        },
                    )
                    .map_err(ToolRunErrorV1::Service)?,
            )?;
            Ok(ToolRunOutcomeV1::Succeeded {
                storage_revision,
                saga,
                record,
            })
        }
        Ok(ToolExecutionOutcomeV1::Failed { evidence_digest }) if evidence_digest != [0; 32] => {
            let (storage_revision, saga) = applied(
                client
                    .apply(
                        Some(dispatch_revision),
                        tools::MutationV1::Fail {
                            workflow: saga.workflow,
                            failure: tools::ToolFailureV1 {
                                effect,
                                capability: retained.call.capability,
                                execution,
                                evidence_digest,
                            },
                        },
                    )
                    .map_err(ToolRunErrorV1::Service)?,
            )?;
            Ok(ToolRunOutcomeV1::Failed {
                storage_revision,
                saga,
            })
        }
        Ok(ToolExecutionOutcomeV1::OutcomeUnknown)
        | Ok(ToolExecutionOutcomeV1::Succeeded { .. })
        | Ok(ToolExecutionOutcomeV1::Failed { .. }) => {
            let (revision, saga) = fence(client, dispatch_revision, saga.workflow, effect)?;
            Ok(ToolRunOutcomeV1::Uncertain {
                storage_revision: revision,
                saga,
                cause: ToolUncertaintyV1::Provider,
            })
        }
        Err(error) => {
            let (revision, saga) = fence(client, dispatch_revision, saga.workflow, effect)?;
            Ok(ToolRunOutcomeV1::Uncertain {
                storage_revision: revision,
                saga,
                cause: ToolUncertaintyV1::Executor(error),
            })
        }
    }
}

/// Read-only recovery after a retained in-flight marker. A matching durable
/// output closes success without re-invocation; absence fences uncertainty.
pub fn reconcile_in_flight<P, O>(
    client: &mut ToolContinuationMutationClient<P>,
    outputs: &mut O,
    expected_storage_revision: Option<u64>,
    saga: &tools::SagaViewV1,
    effect: EffectIdV1,
) -> Result<ToolReconcileOutcomeV1, ToolReconcileErrorV1<P::Error, O::Error>>
where
    P: ToolContinuationServicePort,
    O: ToolOutputStoreV1,
{
    let retained = saga
        .effects
        .iter()
        .find(|candidate| candidate.effect == effect)
        .ok_or(ToolReconcileErrorV1::InvalidState)?;
    let tools::ToolEffectStateV1::InFlight { gate, execution } = retained.state else {
        return Err(ToolReconcileErrorV1::InvalidState);
    };
    if gate.authority.capability != retained.call.capability || gate.attempt != saga.source_attempt
    {
        return Err(ToolReconcileErrorV1::InvalidState);
    }
    let found = outputs
        .find(core_tools::ToolExecutionIdV1(execution.0))
        .map_err(ToolReconcileErrorV1::Output)?;
    let Some((record, output)) = found else {
        let (storage_revision, saga) = reconcile_applied(
            client
                .apply(
                    expected_storage_revision,
                    tools::MutationV1::MarkUncertain {
                        workflow: saga.workflow,
                        effect,
                    },
                )
                .map_err(ToolReconcileErrorV1::Service)?,
        )?;
        return Ok(ToolReconcileOutcomeV1::Uncertain {
            storage_revision,
            saga,
        });
    };
    if record.execution.0 != execution.0
        || record.capability.0 != retained.call.capability.0
        || record.evidence_digest == [0; 32]
        || record.output_digest == [0; 32]
        || record.output_bytes != output.len()
        || record.output_bytes > retained.call.budget.max_output_bytes
        || <[u8; 32]>::from(Sha256::digest(&output)) != record.output_digest
    {
        return Err(ToolReconcileErrorV1::InvalidState);
    }
    let (storage_revision, saga) = reconcile_applied(
        client
            .apply(
                expected_storage_revision,
                tools::MutationV1::Accept {
                    workflow: saga.workflow,
                    receipt: tools::ToolReceiptV1 {
                        effect,
                        capability: retained.call.capability,
                        execution,
                        evidence_digest: record.evidence_digest,
                        output_digest: record.output_digest,
                        output_bytes: record.output_bytes,
                    },
                },
            )
            .map_err(ToolReconcileErrorV1::Service)?,
    )?;
    Ok(ToolReconcileOutcomeV1::Succeeded {
        storage_revision,
        saga,
        record,
    })
}

/// Resolve exact retained success bytes for the next model turn.
pub fn resolve_succeeded_output<O: ToolOutputStoreV1>(
    outputs: &mut O,
    saga: &tools::SagaViewV1,
    effect: EffectIdV1,
) -> Result<Vec<u8>, ToolReconcileErrorV1<std::convert::Infallible, O::Error>> {
    let retained = saga
        .effects
        .iter()
        .find(|candidate| candidate.effect == effect)
        .ok_or(ToolReconcileErrorV1::InvalidState)?;
    let tools::ToolEffectStateV1::Succeeded { receipt, .. } = retained.state else {
        return Err(ToolReconcileErrorV1::InvalidState);
    };
    let output = outputs
        .get(receipt.output_digest)
        .map_err(ToolReconcileErrorV1::Output)?
        .ok_or(ToolReconcileErrorV1::InvalidState)?;
    if output.len() != receipt.output_bytes
        || output.len() > retained.call.budget.max_output_bytes
        || <[u8; 32]>::from(Sha256::digest(&output)) != receipt.output_digest
    {
        return Err(ToolReconcileErrorV1::InvalidState);
    }
    Ok(output)
}

/// Resolve every successful call in provider order for an exact Daimon resume.
pub fn resolve_completed_tool_results<O: ToolOutputStoreV1>(
    outputs: &mut O,
    saga: &tools::SagaViewV1,
) -> Result<
    Vec<moira_daimon::CompletedToolResultV1>,
    ToolReconcileErrorV1<std::convert::Infallible, O::Error>,
> {
    if saga.phase != tools::PhaseV1::Completed || saga.effects.is_empty() {
        return Err(ToolReconcileErrorV1::InvalidState);
    }
    let mut results = Vec::new();
    results
        .try_reserve(saga.effects.len())
        .map_err(|_| ToolReconcileErrorV1::InvalidState)?;
    for effect in &saga.effects {
        let tools::ToolEffectStateV1::Succeeded { receipt, .. } = effect.state else {
            return Err(ToolReconcileErrorV1::InvalidState);
        };
        let output = resolve_succeeded_output(outputs, saga, effect.effect)?;
        results.push(moira_daimon::CompletedToolResultV1 {
            request: effect.call.request.0,
            evidence_digest: receipt.evidence_digest,
            output_digest: receipt.output_digest,
            output,
        });
    }
    Ok(results)
}

fn fence<P: ToolContinuationServicePort>(
    client: &mut ToolContinuationMutationClient<P>,
    expected: u64,
    workflow: WorkflowIdV1,
    effect: EffectIdV1,
) -> Result<(u64, tools::SagaViewV1), ToolRunErrorV1<P::Error>> {
    applied(
        client
            .apply(
                Some(expected),
                tools::MutationV1::MarkUncertain { workflow, effect },
            )
            .map_err(ToolRunErrorV1::Service)?,
    )
}

fn applied<E>(reply: tools::ReplyV1) -> Result<(u64, tools::SagaViewV1), ToolRunErrorV1<E>> {
    match reply {
        tools::ReplyV1::Accepted(tools::OutcomeV1::Applied {
            storage_revision,
            saga,
            ..
        }) => Ok((storage_revision, saga)),
        tools::ReplyV1::Refused(refusal) => Err(ToolRunErrorV1::Refused(refusal)),
        tools::ReplyV1::Accepted(_) => Err(ToolRunErrorV1::InvalidState),
    }
}

fn reconcile_applied<ServiceError, OutputError>(
    reply: tools::ReplyV1,
) -> Result<(u64, tools::SagaViewV1), ToolReconcileErrorV1<ServiceError, OutputError>> {
    match reply {
        tools::ReplyV1::Accepted(tools::OutcomeV1::Applied {
            storage_revision,
            saga,
            ..
        }) => Ok((storage_revision, saga)),
        tools::ReplyV1::Refused(refusal) => Err(ToolReconcileErrorV1::Refused(refusal)),
        tools::ReplyV1::Accepted(_) => Err(ToolReconcileErrorV1::InvalidState),
    }
}

fn invocation(
    workflow: WorkflowIdV1,
    effect: EffectIdV1,
    execution: tools::ToolExecutionIdV1,
    gate: tools::ToolGateV1,
    call: &tools::ToolCallV1,
) -> ToolInvocationV1 {
    ToolInvocationV1 {
        workflow: moira_core::WorkflowId::new(workflow.0),
        effect: moira_core::EffectId::new(effect.0),
        execution: core_tools::ToolExecutionIdV1(execution.0),
        gate: core_tools::ToolGateV1 {
            authority: core_tools::AuthorityEvidenceV1 {
                decision: core_tools::AuthorityDecisionIdV1(gate.authority.decision.0),
                subject: core_tools::AuthoritySubjectIdV1(gate.authority.subject.0),
                capability: core_tools::ToolCapabilityIdV1(gate.authority.capability.0),
                epoch: gate.authority.epoch,
            },
            attempt: core_tools::AttemptEvidenceV1 {
                task_key: gate.attempt.task_key,
                incarnation: gate.attempt.incarnation,
                attempt: gate.attempt.attempt,
            },
        },
        call: core_tools::ToolCallV1 {
            request: core_tools::ToolRequestIdV1(call.request.0),
            capability: core_tools::ToolCapabilityIdV1(call.capability.0),
            provider_correlation: call.provider_correlation.clone(),
            arguments: call.arguments.clone(),
            budget: core_tools::ToolBudgetV1 {
                max_elapsed_millis: call.budget.max_elapsed_millis,
                max_output_bytes: call.budget.max_output_bytes,
            },
        },
    }
}
