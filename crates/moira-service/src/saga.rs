//! Native application service for the closed saga profile.

use crate::AccessV1;
use moira_core::saga as core;
use moira_host::{SagaApplyErrorV1, SagaStore};
use moira_protocol as protocol;

pub struct SagaService<S> {
    store: S,
}

/// Verified facts returned by an Aegis authorization adapter. The decision is
/// retained for attribution only; it is not a reusable credential.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct OperatorAccessV1 {
    pub subject: [u8; 16],
    pub authority_decision: u64,
    pub authority_epoch: u64,
}

impl std::fmt::Debug for OperatorAccessV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OperatorAccessV1")
            .field("subject", &"<redacted>")
            .field("authority_decision", &self.authority_decision)
            .field("authority_epoch", &self.authority_epoch)
            .finish()
    }
}

/// Command-scoped saga authority supplied independently of connection
/// authentication. Implementations return verified attribution facts, never a
/// reusable credential or grant.
pub trait OperatorAuthorizer {
    type Error;

    fn authorize_operator(
        &mut self,
        operation: protocol::OperatorOperationV1,
    ) -> Result<Option<OperatorAccessV1>, Self::Error>;
}

impl<S: SagaStore> SagaService<S> {
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    pub fn execute(
        &mut self,
        command: protocol::SagaCommandV1,
    ) -> Result<protocol::SagaReplyV1, S::Error> {
        self.execute_authorized(
            AccessV1 {
                read: true,
                mutate: true,
            },
            command,
        )
    }

    /// Authentication supplies these connection facts. Mutation refusal occurs
    /// before the durable store is touched.
    pub fn execute_authorized(
        &mut self,
        access: AccessV1,
        command: protocol::SagaCommandV1,
    ) -> Result<protocol::SagaReplyV1, S::Error> {
        let allowed = match command {
            protocol::SagaCommandV1::Apply { .. } => access.mutate,
            protocol::SagaCommandV1::Status
            | protocol::SagaCommandV1::Saga { .. }
            | protocol::SagaCommandV1::Inspect { .. }
            | protocol::SagaCommandV1::Census
            | protocol::SagaCommandV1::Reconciliation { .. }
            | protocol::SagaCommandV1::Evidence { .. }
            | protocol::SagaCommandV1::OperatorReceipt { .. }
            | protocol::SagaCommandV1::OperatorReceipts { .. } => access.read,
            protocol::SagaCommandV1::Reconcile { .. } => false,
        };
        if !allowed {
            return Ok(protocol::SagaReplyV1::Refused(
                protocol::SagaRefusalV1::NotAuthorized,
            ));
        }
        match command {
            protocol::SagaCommandV1::Apply {
                expected_revision,
                mutation,
            } => match self.store.apply(expected_revision, mutation_core(mutation)) {
                Ok(receipt) => Ok(protocol::SagaReplyV1::Accepted(
                    protocol::SagaOutcomeV1::Applied {
                        storage_revision: receipt.storage_revision,
                        transition: transition(receipt.transition),
                        replayed: receipt.replayed,
                    },
                )),
                Err(SagaApplyErrorV1::Domain(value)) => {
                    Ok(protocol::SagaReplyV1::Refused(refusal(value)))
                }
                Err(SagaApplyErrorV1::Backend(error)) => Err(error),
            },
            protocol::SagaCommandV1::Status => Ok(protocol::SagaReplyV1::Accepted(
                protocol::SagaOutcomeV1::Status(protocol::SagaStatusV1 {
                    revision: self.store.revision(),
                    needs_reopen: self.store.needs_reopen(),
                }),
            )),
            protocol::SagaCommandV1::Saga { saga } => Ok(protocol::SagaReplyV1::Accepted(
                protocol::SagaOutcomeV1::Saga(
                    self.store.saga(core::SagaIdV1(saga.0)).map(snapshot),
                ),
            )),
            protocol::SagaCommandV1::Inspect { after, limit } => {
                let Ok(limit) = usize::try_from(limit) else {
                    return Ok(protocol::SagaReplyV1::Refused(
                        protocol::SagaRefusalV1::InvalidPageLimit,
                    ));
                };
                match self
                    .store
                    .inspect_page(after.map(|value| core::SagaIdV1(value.0)), limit)
                {
                    Ok(page) => Ok(protocol::SagaReplyV1::Accepted(
                        protocol::SagaOutcomeV1::Page(protocol::SagaPageV1 {
                            entries: page.entries.into_iter().map(snapshot).collect(),
                            next_after: page.next_after.map(|value| protocol::SagaIdV1(value.0)),
                        }),
                    )),
                    Err(value) => Ok(protocol::SagaReplyV1::Refused(refusal(value))),
                }
            }
            protocol::SagaCommandV1::Census => {
                let value = self.store.census();
                Ok(protocol::SagaReplyV1::Accepted(
                    protocol::SagaOutcomeV1::Census(protocol::SagaCensusV1 {
                        total: value.total,
                        runnable_effects: value.runnable_effects,
                        awaiting_decision: value.awaiting_decision,
                        fenced: value.fenced,
                        terminal: value.terminal,
                        retries_consumed: value.retries_consumed,
                        saturated: value.saturated,
                    }),
                ))
            }
            protocol::SagaCommandV1::Reconciliation { saga } => Ok(
                protocol::SagaReplyV1::Accepted(protocol::SagaOutcomeV1::Reconciliation(
                    self.store
                        .reconciliation_view(core::SagaIdV1(saga.0))
                        .map(reconciliation_view),
                )),
            ),
            protocol::SagaCommandV1::Evidence { saga, after, limit } => {
                let Ok(limit) = usize::try_from(limit) else {
                    return Ok(protocol::SagaReplyV1::Refused(
                        protocol::SagaRefusalV1::InvalidPageLimit,
                    ));
                };
                match self.store.evidence_page(
                    core::SagaIdV1(saga.0),
                    after.map(|value| core::SagaEffectIdV1(value.0)),
                    limit,
                ) {
                    Ok(page) => Ok(protocol::SagaReplyV1::Accepted(
                        protocol::SagaOutcomeV1::Evidence(protocol::EffectEvidencePageV1 {
                            entries: page
                                .entries
                                .into_iter()
                                .map(|entry| protocol::EffectEvidenceV1 {
                                    effect: protocol::SagaEffectIdV1(entry.effect.0),
                                    outcome: outcome(entry.outcome),
                                    observed_at: entry.observed_at,
                                })
                                .collect(),
                            next_after: page
                                .next_after
                                .map(|effect| protocol::SagaEffectIdV1(effect.0)),
                        }),
                    )),
                    Err(value) => Ok(protocol::SagaReplyV1::Refused(refusal(value))),
                }
            }
            protocol::SagaCommandV1::OperatorReceipt { action } => Ok(
                protocol::SagaReplyV1::Accepted(protocol::SagaOutcomeV1::OperatorReceipt(
                    self.store
                        .operator_receipt(core::OperatorActionIdV1(action.0))
                        .map(operator_receipt),
                )),
            ),
            protocol::SagaCommandV1::OperatorReceipts { after, limit } => {
                let Ok(limit) = usize::try_from(limit) else {
                    return Ok(protocol::SagaReplyV1::Refused(
                        protocol::SagaRefusalV1::InvalidPageLimit,
                    ));
                };
                match self.store.operator_receipt_page(
                    after.map(|value| core::OperatorActionIdV1(value.0)),
                    limit,
                ) {
                    Ok(page) => Ok(protocol::SagaReplyV1::Accepted(
                        protocol::SagaOutcomeV1::OperatorReceipts(
                            protocol::OperatorReceiptPageV1 {
                                entries: page.entries.into_iter().map(operator_receipt).collect(),
                                next_after: page
                                    .next_after
                                    .map(|value| protocol::OperatorActionIdV1(value.0)),
                            },
                        ),
                    )),
                    Err(value) => Ok(protocol::SagaReplyV1::Refused(refusal(value))),
                }
            }
            protocol::SagaCommandV1::Reconcile { .. } => Ok(protocol::SagaReplyV1::Refused(
                protocol::SagaRefusalV1::NotAuthorized,
            )),
        }
    }

    /// Executes only a reconciliation operation after an Aegis adapter has
    /// verified administer authority for the saga resource.
    pub fn execute_operator_authorized(
        &mut self,
        access: OperatorAccessV1,
        command: protocol::SagaCommandV1,
    ) -> Result<protocol::SagaReplyV1, S::Error> {
        let protocol::SagaCommandV1::Reconcile {
            expected_revision,
            operation,
        } = command
        else {
            return Ok(protocol::SagaReplyV1::Refused(
                protocol::SagaRefusalV1::NotAuthorized,
            ));
        };
        let command = core::SagaCommandV1::Reconcile {
            cause: core::OperatorCauseV1 {
                action: core::OperatorActionIdV1(operation.action.0),
                subject: access.subject,
                authority_decision: access.authority_decision,
                authority_epoch: access.authority_epoch,
            },
            saga: core::SagaIdV1(operation.saga.0),
            disposition: operator_disposition_core(operation.disposition),
        };
        match self.store.apply(expected_revision, command) {
            Ok(receipt) => Ok(protocol::SagaReplyV1::Accepted(
                protocol::SagaOutcomeV1::Applied {
                    storage_revision: receipt.storage_revision,
                    transition: transition(receipt.transition),
                    replayed: receipt.replayed,
                },
            )),
            Err(SagaApplyErrorV1::Domain(value)) => {
                Ok(protocol::SagaReplyV1::Refused(refusal(value)))
            }
            Err(SagaApplyErrorV1::Backend(error)) => Err(error),
        }
    }

    pub fn into_store(self) -> S {
        self.store
    }
}

fn mutation_core(value: protocol::SagaMutationV1) -> core::SagaCommandV1 {
    match value {
        protocol::SagaMutationV1::Admit {
            source,
            now,
            deadline,
            retry_budget,
        } => core::SagaCommandV1::Admit {
            source: source.0,
            now,
            deadline,
            retry_budget,
        },
        protocol::SagaMutationV1::AcceptEffect {
            saga,
            effect,
            outcome,
            observed_at,
        } => core::SagaCommandV1::AcceptEffect {
            saga: core::SagaIdV1(saga.0),
            effect: core::SagaEffectIdV1(effect.0),
            outcome: outcome_core(outcome),
            observed_at,
        },
        protocol::SagaMutationV1::PrepareTerminal { saga, now } => {
            core::SagaCommandV1::PrepareTerminal {
                saga: core::SagaIdV1(saga.0),
                now,
            }
        }
        protocol::SagaMutationV1::Retry {
            saga,
            failed_effect,
            now,
        } => core::SagaCommandV1::Retry {
            saga: core::SagaIdV1(saga.0),
            failed_effect: core::SagaEffectIdV1(failed_effect.0),
            now,
        },
        protocol::SagaMutationV1::Cancel { saga, now } => core::SagaCommandV1::Cancel {
            saga: core::SagaIdV1(saga.0),
            now,
        },
        protocol::SagaMutationV1::FireDeadline {
            saga,
            timer,
            observed_at,
            action,
        } => core::SagaCommandV1::FireDeadline {
            saga: core::SagaIdV1(saga.0),
            timer: core::TimerIdV1(timer.0),
            observed_at,
            action: action_core(action),
        },
    }
}

fn operator_disposition_core(
    value: protocol::OperatorDispositionV1,
) -> core::OperatorDispositionV1 {
    match value {
        protocol::OperatorDispositionV1::AdmitEvidence {
            effect,
            outcome,
            observed_at,
        } => core::OperatorDispositionV1::AdmitEvidence {
            effect: core::SagaEffectIdV1(effect.0),
            outcome: outcome_core(outcome),
            observed_at,
        },
        protocol::OperatorDispositionV1::KeepUncertain => {
            core::OperatorDispositionV1::KeepUncertain
        }
        protocol::OperatorDispositionV1::Retry { failed_effect, now } => {
            core::OperatorDispositionV1::Retry {
                failed_effect: core::SagaEffectIdV1(failed_effect.0),
                now,
            }
        }
        protocol::OperatorDispositionV1::Compensate { now } => {
            core::OperatorDispositionV1::Compensate { now }
        }
        protocol::OperatorDispositionV1::Cancel { now } => {
            core::OperatorDispositionV1::Cancel { now }
        }
        protocol::OperatorDispositionV1::FailTerminal => core::OperatorDispositionV1::FailTerminal,
    }
}

fn operator_disposition(value: core::OperatorDispositionV1) -> protocol::OperatorDispositionV1 {
    match value {
        core::OperatorDispositionV1::AdmitEvidence {
            effect,
            outcome: value,
            observed_at,
        } => protocol::OperatorDispositionV1::AdmitEvidence {
            effect: protocol::SagaEffectIdV1(effect.0),
            outcome: outcome(value),
            observed_at,
        },
        core::OperatorDispositionV1::KeepUncertain => {
            protocol::OperatorDispositionV1::KeepUncertain
        }
        core::OperatorDispositionV1::Retry { failed_effect, now } => {
            protocol::OperatorDispositionV1::Retry {
                failed_effect: protocol::SagaEffectIdV1(failed_effect.0),
                now,
            }
        }
        core::OperatorDispositionV1::Compensate { now } => {
            protocol::OperatorDispositionV1::Compensate { now }
        }
        core::OperatorDispositionV1::Cancel { now } => {
            protocol::OperatorDispositionV1::Cancel { now }
        }
        core::OperatorDispositionV1::FailTerminal => protocol::OperatorDispositionV1::FailTerminal,
    }
}

fn operator_receipt(value: core::OperatorReceiptSummaryV1) -> protocol::OperatorReceiptSummaryV1 {
    protocol::OperatorReceiptSummaryV1 {
        action: protocol::OperatorActionIdV1(value.action.0),
        saga: protocol::SagaIdV1(value.saga.0),
        authority_decision: value.authority_decision,
        authority_epoch: value.authority_epoch,
        disposition: operator_disposition(value.disposition),
        phase: phase(value.phase),
        prepared: value
            .prepared
            .map(|effect| protocol::SagaEffectIdV1(effect.0)),
    }
}

fn outcome_core(value: protocol::SagaEffectOutcomeV1) -> core::EffectOutcomeV1 {
    match value {
        protocol::SagaEffectOutcomeV1::Succeeded => core::EffectOutcomeV1::Succeeded,
        protocol::SagaEffectOutcomeV1::Failed => core::EffectOutcomeV1::Failed,
        protocol::SagaEffectOutcomeV1::Uncertain => core::EffectOutcomeV1::Uncertain,
    }
}

fn outcome(value: core::EffectOutcomeV1) -> protocol::SagaEffectOutcomeV1 {
    match value {
        core::EffectOutcomeV1::Succeeded => protocol::SagaEffectOutcomeV1::Succeeded,
        core::EffectOutcomeV1::Failed => protocol::SagaEffectOutcomeV1::Failed,
        core::EffectOutcomeV1::Uncertain => protocol::SagaEffectOutcomeV1::Uncertain,
    }
}

fn kind(value: core::EffectKindV1) -> protocol::SagaEffectKindV1 {
    match value {
        core::EffectKindV1::Forward => protocol::SagaEffectKindV1::Forward,
        core::EffectKindV1::Terminal => protocol::SagaEffectKindV1::Terminal,
        core::EffectKindV1::Compensation => protocol::SagaEffectKindV1::Compensation,
    }
}

fn action_core(value: protocol::DeadlineActionV1) -> core::DeadlineActionV1 {
    match value {
        protocol::DeadlineActionV1::KeepUncertain => core::DeadlineActionV1::KeepUncertain,
        protocol::DeadlineActionV1::Fail => core::DeadlineActionV1::Fail,
        protocol::DeadlineActionV1::Compensate => core::DeadlineActionV1::Compensate,
    }
}

fn action(value: core::DeadlineActionV1) -> protocol::DeadlineActionV1 {
    match value {
        core::DeadlineActionV1::KeepUncertain => protocol::DeadlineActionV1::KeepUncertain,
        core::DeadlineActionV1::Fail => protocol::DeadlineActionV1::Fail,
        core::DeadlineActionV1::Compensate => protocol::DeadlineActionV1::Compensate,
    }
}

fn phase(value: core::SagaPhaseV1) -> protocol::SagaPhaseV1 {
    match value {
        core::SagaPhaseV1::ForwardPending => protocol::SagaPhaseV1::ForwardPending,
        core::SagaPhaseV1::ForwardSucceeded => protocol::SagaPhaseV1::ForwardSucceeded,
        core::SagaPhaseV1::ForwardFailed => protocol::SagaPhaseV1::ForwardFailed,
        core::SagaPhaseV1::TerminalPending => protocol::SagaPhaseV1::TerminalPending,
        core::SagaPhaseV1::TerminalFailed => protocol::SagaPhaseV1::TerminalFailed,
        core::SagaPhaseV1::CompensationPending => protocol::SagaPhaseV1::CompensationPending,
        core::SagaPhaseV1::CompensationFailed => protocol::SagaPhaseV1::CompensationFailed,
        core::SagaPhaseV1::FencedUncertain => protocol::SagaPhaseV1::FencedUncertain,
        core::SagaPhaseV1::Completed => protocol::SagaPhaseV1::Completed,
        core::SagaPhaseV1::Failed => protocol::SagaPhaseV1::Failed,
        core::SagaPhaseV1::Cancelled => protocol::SagaPhaseV1::Cancelled,
    }
}

fn snapshot(value: core::SagaSnapshotV1) -> protocol::SagaViewV1 {
    protocol::SagaViewV1 {
        saga: protocol::SagaIdV1(value.saga.0),
        phase: phase(value.phase),
        deadline: value.deadline,
        timer: protocol::TimerIdV1(value.timer.0),
        current_effect: value
            .current_effect
            .map(|effect| protocol::SagaEffectIdV1(effect.0)),
        current_kind: value.current_kind.map(kind),
        retries_used: value.retries_used,
        retry_budget: value.retry_budget,
    }
}

fn reconciliation_view(
    value: core::SagaReconciliationViewV1,
) -> protocol::SagaReconciliationViewV1 {
    protocol::SagaReconciliationViewV1 {
        saga: snapshot(value.snapshot),
        last_confirmed_revision: value.last_confirmed_revision,
        retained_evidence: value.retained_evidence,
        fence: value.fence.map(|fence| match fence {
            core::FenceCauseV1::EffectUncertain {
                effect,
                kind: value,
                observed_at,
            } => protocol::FenceCauseV1::EffectUncertain {
                effect: protocol::SagaEffectIdV1(effect.0),
                kind: kind(value),
                observed_at,
            },
            core::FenceCauseV1::Deadline { timer, observed_at } => {
                protocol::FenceCauseV1::Deadline {
                    timer: protocol::TimerIdV1(timer.0),
                    observed_at,
                }
            }
        }),
        automatic_progress: match value.automatic_progress {
            core::AutomaticProgressV1::Runnable => protocol::AutomaticProgressV1::Runnable,
            core::AutomaticProgressV1::AwaitingDecision => {
                protocol::AutomaticProgressV1::AwaitingDecision
            }
            core::AutomaticProgressV1::BlockedUncertain => {
                protocol::AutomaticProgressV1::BlockedUncertain
            }
            core::AutomaticProgressV1::Terminal => protocol::AutomaticProgressV1::Terminal,
        },
    }
}

fn transition(value: core::SagaTransitionV1) -> protocol::SagaTransitionV1 {
    match value {
        core::SagaTransitionV1::Admitted {
            saga,
            effect,
            timer,
        } => protocol::SagaTransitionV1::Admitted {
            saga: protocol::SagaIdV1(saga.0),
            effect: protocol::SagaEffectIdV1(effect.0),
            timer: protocol::TimerIdV1(timer.0),
        },
        core::SagaTransitionV1::EffectPrepared {
            saga,
            effect,
            kind: value,
            retry,
        } => protocol::SagaTransitionV1::EffectPrepared {
            saga: protocol::SagaIdV1(saga.0),
            effect: protocol::SagaEffectIdV1(effect.0),
            kind: kind(value),
            retry,
        },
        core::SagaTransitionV1::EffectAccepted {
            saga,
            effect,
            outcome: value,
            phase: value_phase,
        } => protocol::SagaTransitionV1::EffectAccepted {
            saga: protocol::SagaIdV1(saga.0),
            effect: protocol::SagaEffectIdV1(effect.0),
            outcome: outcome(value),
            phase: phase(value_phase),
        },
        core::SagaTransitionV1::DeadlineApplied {
            saga,
            timer,
            action: value,
            phase: value_phase,
            compensation,
        } => protocol::SagaTransitionV1::DeadlineApplied {
            saga: protocol::SagaIdV1(saga.0),
            timer: protocol::TimerIdV1(timer.0),
            action: action(value),
            phase: phase(value_phase),
            compensation: compensation.map(|effect| protocol::SagaEffectIdV1(effect.0)),
        },
        core::SagaTransitionV1::Cancelled { saga, compensation } => {
            protocol::SagaTransitionV1::Cancelled {
                saga: protocol::SagaIdV1(saga.0),
                compensation: compensation.map(|effect| protocol::SagaEffectIdV1(effect.0)),
            }
        }
        core::SagaTransitionV1::Reconciled {
            saga,
            action,
            phase: value_phase,
            prepared,
        } => protocol::SagaTransitionV1::Reconciled {
            saga: protocol::SagaIdV1(saga.0),
            action: protocol::OperatorActionIdV1(action.0),
            phase: phase(value_phase),
            prepared: prepared.map(|effect| protocol::SagaEffectIdV1(effect.0)),
        },
    }
}

fn refusal(value: core::SagaRefusalV1) -> protocol::SagaRefusalV1 {
    match value {
        core::SagaRefusalV1::InvalidLimits => protocol::SagaRefusalV1::InvalidLimits,
        core::SagaRefusalV1::InvalidPageLimit => protocol::SagaRefusalV1::InvalidPageLimit,
        core::SagaRefusalV1::RevisionConflict { expected, actual } => {
            protocol::SagaRefusalV1::RevisionConflict { expected, actual }
        }
        core::SagaRefusalV1::RevisionExhausted => protocol::SagaRefusalV1::RevisionExhausted,
        core::SagaRefusalV1::IdentityExhausted => protocol::SagaRefusalV1::IdentityExhausted,
        core::SagaRefusalV1::SagaLimit => protocol::SagaRefusalV1::SagaLimit,
        core::SagaRefusalV1::InvalidDeadline => protocol::SagaRefusalV1::InvalidDeadline,
        core::SagaRefusalV1::RetryBudget => protocol::SagaRefusalV1::RetryBudget,
        core::SagaRefusalV1::InputConflict => protocol::SagaRefusalV1::InputConflict,
        core::SagaRefusalV1::UnknownSaga => protocol::SagaRefusalV1::UnknownSaga,
        core::SagaRefusalV1::InvalidPhase => protocol::SagaRefusalV1::InvalidPhase,
        core::SagaRefusalV1::EffectMismatch => protocol::SagaRefusalV1::EffectMismatch,
        core::SagaRefusalV1::EvidenceConflict => protocol::SagaRefusalV1::EvidenceConflict,
        core::SagaRefusalV1::DeadlineNotReached => protocol::SagaRefusalV1::DeadlineNotReached,
        core::SagaRefusalV1::DeadlineReached => protocol::SagaRefusalV1::DeadlineReached,
        core::SagaRefusalV1::TimerMismatch => protocol::SagaRefusalV1::TimerMismatch,
        core::SagaRefusalV1::TimerConflict => protocol::SagaRefusalV1::TimerConflict,
        core::SagaRefusalV1::LateEvidence => protocol::SagaRefusalV1::LateEvidence,
        core::SagaRefusalV1::InvalidDeadlineAction => {
            protocol::SagaRefusalV1::InvalidDeadlineAction
        }
        core::SagaRefusalV1::InvalidCheckpoint => protocol::SagaRefusalV1::InvalidCheckpoint,
        core::SagaRefusalV1::OperatorActionLimit => protocol::SagaRefusalV1::OperatorActionLimit,
        core::SagaRefusalV1::OperatorActionConflict => {
            protocol::SagaRefusalV1::OperatorActionConflict
        }
    }
}
