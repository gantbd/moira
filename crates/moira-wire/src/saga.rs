use super::{CodecErrorV1, LimitsV1, Reader, Writer, boolean};
use moira_protocol::*;

const SAGA_REQUEST: u8 = 4;
const SAGA_REPLY: u8 = 5;

pub fn encode_saga_request(
    value: &SagaCommandV1,
    limits: LimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    let mut writer = Writer::new(SAGA_REQUEST, limits)?;
    command(&mut writer, value)?;
    Ok(writer.bytes)
}

pub fn decode_saga_request(bytes: &[u8], limits: LimitsV1) -> Result<SagaCommandV1, CodecErrorV1> {
    let mut reader = Reader::new(bytes, SAGA_REQUEST, limits)?;
    let value = read_command(&mut reader)?;
    reader.finish()?;
    Ok(value)
}

pub fn encode_saga_reply(value: &SagaReplyV1, limits: LimitsV1) -> Result<Vec<u8>, CodecErrorV1> {
    let mut writer = Writer::new(SAGA_REPLY, limits)?;
    match value {
        SagaReplyV1::Accepted(value) => {
            writer.byte(0)?;
            outcome(&mut writer, value)?;
        }
        SagaReplyV1::Refused(value) => {
            writer.byte(1)?;
            refusal(&mut writer, *value)?;
        }
    }
    Ok(writer.bytes)
}

pub fn decode_saga_reply(bytes: &[u8], limits: LimitsV1) -> Result<SagaReplyV1, CodecErrorV1> {
    let mut reader = Reader::new(bytes, SAGA_REPLY, limits)?;
    let value = match reader.byte()? {
        0 => SagaReplyV1::Accepted(read_outcome(&mut reader)?),
        1 => SagaReplyV1::Refused(read_refusal(&mut reader)?),
        _ => return Err(CodecErrorV1::Unsupported),
    };
    reader.finish()?;
    Ok(value)
}

fn command(w: &mut Writer, value: &SagaCommandV1) -> Result<(), CodecErrorV1> {
    match value {
        SagaCommandV1::Apply {
            expected_revision,
            mutation: value,
        } => {
            w.byte(0)?;
            w.u64(*expected_revision)?;
            mutation(w, *value)
        }
        SagaCommandV1::Status => w.byte(1),
        SagaCommandV1::Saga { saga } => {
            w.byte(2)?;
            w.u64(saga.0)
        }
        SagaCommandV1::Inspect { after, limit } => {
            w.byte(3)?;
            w.option(*after, |w, value| w.u64(value.0))?;
            w.u64(*limit)
        }
        SagaCommandV1::Census => w.byte(4),
        SagaCommandV1::Reconciliation { saga } => {
            w.byte(5)?;
            w.u64(saga.0)
        }
        SagaCommandV1::Evidence { saga, after, limit } => {
            w.byte(6)?;
            w.u64(saga.0)?;
            w.option(*after, |w, value| w.u64(value.0))?;
            w.u64(*limit)
        }
        SagaCommandV1::OperatorReceipt { action } => {
            w.byte(7)?;
            w.raw(&action.0)
        }
        SagaCommandV1::OperatorReceipts { after, limit } => {
            w.byte(8)?;
            w.option(*after, |w, value| w.raw(&value.0))?;
            w.u64(*limit)
        }
        SagaCommandV1::Reconcile {
            expected_revision,
            operation,
        } => {
            w.byte(9)?;
            w.u64(*expected_revision)?;
            operator_operation(w, *operation)
        }
        SagaCommandV1::ReconciliationConfirmation { proposal } => {
            w.byte(10)?;
            w.u64(proposal.saga.0)?;
            operator_disposition(w, proposal.disposition)
        }
        SagaCommandV1::ConfirmReconciliation(value) => {
            w.byte(11)?;
            w.u64(value.storage_revision)?;
            w.u64(value.saga.0)?;
            operator_disposition(w, value.disposition)?;
            w.raw(&value.action.0)?;
            w.raw(&value.confirmation_digest)
        }
    }
}

fn read_command(r: &mut Reader<'_>) -> Result<SagaCommandV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => SagaCommandV1::Apply {
            expected_revision: r.u64()?,
            mutation: read_mutation(r)?,
        },
        1 => SagaCommandV1::Status,
        2 => SagaCommandV1::Saga {
            saga: SagaIdV1(r.u64()?),
        },
        3 => SagaCommandV1::Inspect {
            after: r.option(|r| r.u64().map(SagaIdV1))?,
            limit: r.u64()?,
        },
        4 => SagaCommandV1::Census,
        5 => SagaCommandV1::Reconciliation {
            saga: SagaIdV1(r.u64()?),
        },
        6 => SagaCommandV1::Evidence {
            saga: SagaIdV1(r.u64()?),
            after: r.option(|r| r.u64().map(SagaEffectIdV1))?,
            limit: r.u64()?,
        },
        7 => SagaCommandV1::OperatorReceipt {
            action: OperatorActionIdV1(r.raw()?),
        },
        8 => SagaCommandV1::OperatorReceipts {
            after: r.option(|r| r.raw().map(OperatorActionIdV1))?,
            limit: r.u64()?,
        },
        9 => SagaCommandV1::Reconcile {
            expected_revision: r.u64()?,
            operation: read_operator_operation(r)?,
        },
        10 => SagaCommandV1::ReconciliationConfirmation {
            proposal: ReconciliationProposalV1 {
                saga: SagaIdV1(r.u64()?),
                disposition: read_operator_disposition(r)?,
            },
        },
        11 => SagaCommandV1::ConfirmReconciliation(ConfirmedReconciliationV1 {
            storage_revision: r.u64()?,
            saga: SagaIdV1(r.u64()?),
            disposition: read_operator_disposition(r)?,
            action: OperatorActionIdV1(r.raw()?),
            confirmation_digest: r.raw()?,
        }),
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn mutation(w: &mut Writer, value: SagaMutationV1) -> Result<(), CodecErrorV1> {
    match value {
        SagaMutationV1::Admit {
            source,
            now,
            deadline,
            retry_budget,
        } => {
            w.byte(0)?;
            w.raw(&source.0)?;
            w.u64(now)?;
            w.u64(deadline)?;
            w.byte(retry_budget)
        }
        SagaMutationV1::AcceptEffect {
            saga,
            effect,
            outcome,
            observed_at,
        } => {
            w.byte(1)?;
            w.u64(saga.0)?;
            w.u64(effect.0)?;
            w.byte(effect_outcome_tag(outcome))?;
            w.u64(observed_at)
        }
        SagaMutationV1::PrepareTerminal { saga, now } => {
            w.byte(2)?;
            w.u64(saga.0)?;
            w.u64(now)
        }
        SagaMutationV1::Retry {
            saga,
            failed_effect,
            now,
        } => {
            w.byte(3)?;
            w.u64(saga.0)?;
            w.u64(failed_effect.0)?;
            w.u64(now)
        }
        SagaMutationV1::Cancel { saga, now } => {
            w.byte(4)?;
            w.u64(saga.0)?;
            w.u64(now)
        }
        SagaMutationV1::FireDeadline {
            saga,
            timer,
            observed_at,
            action,
        } => {
            w.byte(5)?;
            w.u64(saga.0)?;
            w.u64(timer.0)?;
            w.u64(observed_at)?;
            w.byte(deadline_action_tag(action))
        }
    }
}

fn read_mutation(r: &mut Reader<'_>) -> Result<SagaMutationV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => SagaMutationV1::Admit {
            source: SagaSourceV1(r.raw()?),
            now: r.u64()?,
            deadline: r.u64()?,
            retry_budget: r.byte()?,
        },
        1 => SagaMutationV1::AcceptEffect {
            saga: SagaIdV1(r.u64()?),
            effect: SagaEffectIdV1(r.u64()?),
            outcome: read_effect_outcome(r.byte()?)?,
            observed_at: r.u64()?,
        },
        2 => SagaMutationV1::PrepareTerminal {
            saga: SagaIdV1(r.u64()?),
            now: r.u64()?,
        },
        3 => SagaMutationV1::Retry {
            saga: SagaIdV1(r.u64()?),
            failed_effect: SagaEffectIdV1(r.u64()?),
            now: r.u64()?,
        },
        4 => SagaMutationV1::Cancel {
            saga: SagaIdV1(r.u64()?),
            now: r.u64()?,
        },
        5 => SagaMutationV1::FireDeadline {
            saga: SagaIdV1(r.u64()?),
            timer: TimerIdV1(r.u64()?),
            observed_at: r.u64()?,
            action: read_deadline_action(r.byte()?)?,
        },
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn operator_operation(w: &mut Writer, value: OperatorOperationV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.action.0)?;
    w.u64(value.saga.0)?;
    operator_disposition(w, value.disposition)
}

fn read_operator_operation(r: &mut Reader<'_>) -> Result<OperatorOperationV1, CodecErrorV1> {
    Ok(OperatorOperationV1 {
        action: OperatorActionIdV1(r.raw()?),
        saga: SagaIdV1(r.u64()?),
        disposition: read_operator_disposition(r)?,
    })
}

fn operator_disposition(w: &mut Writer, value: OperatorDispositionV1) -> Result<(), CodecErrorV1> {
    match value {
        OperatorDispositionV1::AdmitEvidence {
            effect,
            outcome,
            observed_at,
        } => {
            w.byte(0)?;
            w.u64(effect.0)?;
            w.byte(effect_outcome_tag(outcome))?;
            w.u64(observed_at)
        }
        OperatorDispositionV1::KeepUncertain => w.byte(1),
        OperatorDispositionV1::Retry { failed_effect, now } => {
            w.byte(2)?;
            w.u64(failed_effect.0)?;
            w.u64(now)
        }
        OperatorDispositionV1::Compensate { now } => {
            w.byte(3)?;
            w.u64(now)
        }
        OperatorDispositionV1::Cancel { now } => {
            w.byte(4)?;
            w.u64(now)
        }
        OperatorDispositionV1::FailTerminal => w.byte(5),
    }
}

fn read_operator_disposition(r: &mut Reader<'_>) -> Result<OperatorDispositionV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => OperatorDispositionV1::AdmitEvidence {
            effect: SagaEffectIdV1(r.u64()?),
            outcome: read_effect_outcome(r.byte()?)?,
            observed_at: r.u64()?,
        },
        1 => OperatorDispositionV1::KeepUncertain,
        2 => OperatorDispositionV1::Retry {
            failed_effect: SagaEffectIdV1(r.u64()?),
            now: r.u64()?,
        },
        3 => OperatorDispositionV1::Compensate { now: r.u64()? },
        4 => OperatorDispositionV1::Cancel { now: r.u64()? },
        5 => OperatorDispositionV1::FailTerminal,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn outcome(w: &mut Writer, value: &SagaOutcomeV1) -> Result<(), CodecErrorV1> {
    match value {
        SagaOutcomeV1::Applied {
            storage_revision,
            transition: value,
            replayed,
        } => {
            w.byte(0)?;
            w.u64(*storage_revision)?;
            transition(w, *value)?;
            w.byte(u8::from(*replayed))
        }
        SagaOutcomeV1::Status(value) => {
            w.byte(1)?;
            w.u64(value.revision)?;
            w.byte(u8::from(value.needs_reopen))
        }
        SagaOutcomeV1::Saga(value) => {
            w.byte(2)?;
            w.option(*value, saga_view)
        }
        SagaOutcomeV1::Page(value) => {
            w.byte(3)?;
            write_len(w, value.entries.len())?;
            for entry in &value.entries {
                saga_view(w, *entry)?;
            }
            w.option(value.next_after, |w, value| w.u64(value.0))
        }
        SagaOutcomeV1::Census(value) => {
            w.byte(4)?;
            census(w, *value)
        }
        SagaOutcomeV1::Reconciliation(value) => {
            w.byte(5)?;
            w.option(*value, reconciliation)
        }
        SagaOutcomeV1::Evidence(value) => {
            w.byte(6)?;
            write_len(w, value.entries.len())?;
            for entry in &value.entries {
                evidence(w, *entry)?;
            }
            w.option(value.next_after, |w, value| w.u64(value.0))
        }
        SagaOutcomeV1::OperatorReceipt(value) => {
            w.byte(7)?;
            w.option(*value, operator_receipt)
        }
        SagaOutcomeV1::OperatorReceipts(value) => {
            w.byte(8)?;
            write_len(w, value.entries.len())?;
            for entry in &value.entries {
                operator_receipt(w, *entry)?;
            }
            w.option(value.next_after, |w, value| w.raw(&value.0))
        }
        SagaOutcomeV1::ReconciliationConfirmation(value) => {
            w.byte(9)?;
            w.option(*value, reconciliation_confirmation)
        }
    }
}

fn read_outcome(r: &mut Reader<'_>) -> Result<SagaOutcomeV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => SagaOutcomeV1::Applied {
            storage_revision: r.u64()?,
            transition: read_transition(r)?,
            replayed: boolean(r.byte()?)?,
        },
        1 => SagaOutcomeV1::Status(SagaStatusV1 {
            revision: r.u64()?,
            needs_reopen: boolean(r.byte()?)?,
        }),
        2 => SagaOutcomeV1::Saga(r.option(read_saga_view)?),
        3 => SagaOutcomeV1::Page(SagaPageV1 {
            entries: read_vec(r, read_saga_view)?,
            next_after: r.option(|r| r.u64().map(SagaIdV1))?,
        }),
        4 => SagaOutcomeV1::Census(read_census(r)?),
        5 => SagaOutcomeV1::Reconciliation(r.option(read_reconciliation)?),
        6 => SagaOutcomeV1::Evidence(EffectEvidencePageV1 {
            entries: read_vec(r, read_evidence)?,
            next_after: r.option(|r| r.u64().map(SagaEffectIdV1))?,
        }),
        7 => SagaOutcomeV1::OperatorReceipt(r.option(read_operator_receipt)?),
        8 => SagaOutcomeV1::OperatorReceipts(OperatorReceiptPageV1 {
            entries: read_vec(r, read_operator_receipt)?,
            next_after: r.option(|r| r.raw().map(OperatorActionIdV1))?,
        }),
        9 => SagaOutcomeV1::ReconciliationConfirmation(r.option(read_reconciliation_confirmation)?),
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn reconciliation_confirmation(
    w: &mut Writer,
    value: ReconciliationConfirmationV1,
) -> Result<(), CodecErrorV1> {
    w.u64(value.storage_revision)?;
    w.u64(value.saga.0)?;
    operator_disposition(w, value.disposition)?;
    w.raw(&value.action.0)?;
    w.raw(&value.confirmation_digest)
}

fn read_reconciliation_confirmation(
    r: &mut Reader<'_>,
) -> Result<ReconciliationConfirmationV1, CodecErrorV1> {
    Ok(ReconciliationConfirmationV1 {
        storage_revision: r.u64()?,
        saga: SagaIdV1(r.u64()?),
        disposition: read_operator_disposition(r)?,
        action: OperatorActionIdV1(r.raw()?),
        confirmation_digest: r.raw()?,
    })
}

fn saga_view(w: &mut Writer, value: SagaViewV1) -> Result<(), CodecErrorV1> {
    w.u64(value.saga.0)?;
    w.byte(phase_tag(value.phase))?;
    w.u64(value.deadline)?;
    w.u64(value.timer.0)?;
    w.option(value.current_effect, |w, value| w.u64(value.0))?;
    w.option(value.current_kind, |w, value| {
        w.byte(effect_kind_tag(value))
    })?;
    w.byte(value.retries_used)?;
    w.byte(value.retry_budget)
}

fn read_saga_view(r: &mut Reader<'_>) -> Result<SagaViewV1, CodecErrorV1> {
    Ok(SagaViewV1 {
        saga: SagaIdV1(r.u64()?),
        phase: read_phase(r.byte()?)?,
        deadline: r.u64()?,
        timer: TimerIdV1(r.u64()?),
        current_effect: r.option(|r| r.u64().map(SagaEffectIdV1))?,
        current_kind: r.option(|r| read_effect_kind(r.byte()?))?,
        retries_used: r.byte()?,
        retry_budget: r.byte()?,
    })
}

fn census(w: &mut Writer, value: SagaCensusV1) -> Result<(), CodecErrorV1> {
    w.u64(value.total)?;
    w.u64(value.runnable_effects)?;
    w.u64(value.awaiting_decision)?;
    w.u64(value.fenced)?;
    w.u64(value.terminal)?;
    w.u64(value.retries_consumed)?;
    w.byte(u8::from(value.saturated))
}

fn read_census(r: &mut Reader<'_>) -> Result<SagaCensusV1, CodecErrorV1> {
    Ok(SagaCensusV1 {
        total: r.u64()?,
        runnable_effects: r.u64()?,
        awaiting_decision: r.u64()?,
        fenced: r.u64()?,
        terminal: r.u64()?,
        retries_consumed: r.u64()?,
        saturated: boolean(r.byte()?)?,
    })
}

fn reconciliation(w: &mut Writer, value: SagaReconciliationViewV1) -> Result<(), CodecErrorV1> {
    saga_view(w, value.saga)?;
    w.u64(value.last_confirmed_revision)?;
    w.u64(value.retained_evidence)?;
    w.option(value.fence, fence)?;
    w.byte(automatic_progress_tag(value.automatic_progress))
}

fn read_reconciliation(r: &mut Reader<'_>) -> Result<SagaReconciliationViewV1, CodecErrorV1> {
    Ok(SagaReconciliationViewV1 {
        saga: read_saga_view(r)?,
        last_confirmed_revision: r.u64()?,
        retained_evidence: r.u64()?,
        fence: r.option(read_fence)?,
        automatic_progress: read_automatic_progress(r.byte()?)?,
    })
}

fn fence(w: &mut Writer, value: FenceCauseV1) -> Result<(), CodecErrorV1> {
    match value {
        FenceCauseV1::EffectUncertain {
            effect,
            kind,
            observed_at,
        } => {
            w.byte(0)?;
            w.u64(effect.0)?;
            w.byte(effect_kind_tag(kind))?;
            w.u64(observed_at)
        }
        FenceCauseV1::Deadline { timer, observed_at } => {
            w.byte(1)?;
            w.u64(timer.0)?;
            w.u64(observed_at)
        }
    }
}

fn read_fence(r: &mut Reader<'_>) -> Result<FenceCauseV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => FenceCauseV1::EffectUncertain {
            effect: SagaEffectIdV1(r.u64()?),
            kind: read_effect_kind(r.byte()?)?,
            observed_at: r.u64()?,
        },
        1 => FenceCauseV1::Deadline {
            timer: TimerIdV1(r.u64()?),
            observed_at: r.u64()?,
        },
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn evidence(w: &mut Writer, value: EffectEvidenceV1) -> Result<(), CodecErrorV1> {
    w.u64(value.effect.0)?;
    w.byte(effect_outcome_tag(value.outcome))?;
    w.u64(value.observed_at)
}

fn read_evidence(r: &mut Reader<'_>) -> Result<EffectEvidenceV1, CodecErrorV1> {
    Ok(EffectEvidenceV1 {
        effect: SagaEffectIdV1(r.u64()?),
        outcome: read_effect_outcome(r.byte()?)?,
        observed_at: r.u64()?,
    })
}

fn operator_receipt(w: &mut Writer, value: OperatorReceiptSummaryV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.action.0)?;
    w.u64(value.saga.0)?;
    w.u64(value.authority_decision)?;
    w.u64(value.authority_epoch)?;
    operator_disposition(w, value.disposition)?;
    w.byte(phase_tag(value.phase))?;
    w.option(value.prepared, |w, value| w.u64(value.0))
}

fn read_operator_receipt(r: &mut Reader<'_>) -> Result<OperatorReceiptSummaryV1, CodecErrorV1> {
    Ok(OperatorReceiptSummaryV1 {
        action: OperatorActionIdV1(r.raw()?),
        saga: SagaIdV1(r.u64()?),
        authority_decision: r.u64()?,
        authority_epoch: r.u64()?,
        disposition: read_operator_disposition(r)?,
        phase: read_phase(r.byte()?)?,
        prepared: r.option(|r| r.u64().map(SagaEffectIdV1))?,
    })
}

fn transition(w: &mut Writer, value: SagaTransitionV1) -> Result<(), CodecErrorV1> {
    match value {
        SagaTransitionV1::Admitted {
            saga,
            effect,
            timer,
        } => {
            w.byte(0)?;
            w.u64(saga.0)?;
            w.u64(effect.0)?;
            w.u64(timer.0)
        }
        SagaTransitionV1::EffectPrepared {
            saga,
            effect,
            kind,
            retry,
        } => {
            w.byte(1)?;
            w.u64(saga.0)?;
            w.u64(effect.0)?;
            w.byte(effect_kind_tag(kind))?;
            w.byte(retry)
        }
        SagaTransitionV1::EffectAccepted {
            saga,
            effect,
            outcome,
            phase,
        } => {
            w.byte(2)?;
            w.u64(saga.0)?;
            w.u64(effect.0)?;
            w.byte(effect_outcome_tag(outcome))?;
            w.byte(phase_tag(phase))
        }
        SagaTransitionV1::DeadlineApplied {
            saga,
            timer,
            action,
            phase,
            compensation,
        } => {
            w.byte(3)?;
            w.u64(saga.0)?;
            w.u64(timer.0)?;
            w.byte(deadline_action_tag(action))?;
            w.byte(phase_tag(phase))?;
            w.option(compensation, |w, value| w.u64(value.0))
        }
        SagaTransitionV1::Cancelled { saga, compensation } => {
            w.byte(4)?;
            w.u64(saga.0)?;
            w.option(compensation, |w, value| w.u64(value.0))
        }
        SagaTransitionV1::Reconciled {
            saga,
            action,
            phase,
            prepared,
        } => {
            w.byte(5)?;
            w.u64(saga.0)?;
            w.raw(&action.0)?;
            w.byte(phase_tag(phase))?;
            w.option(prepared, |w, value| w.u64(value.0))
        }
    }
}

fn read_transition(r: &mut Reader<'_>) -> Result<SagaTransitionV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => SagaTransitionV1::Admitted {
            saga: SagaIdV1(r.u64()?),
            effect: SagaEffectIdV1(r.u64()?),
            timer: TimerIdV1(r.u64()?),
        },
        1 => SagaTransitionV1::EffectPrepared {
            saga: SagaIdV1(r.u64()?),
            effect: SagaEffectIdV1(r.u64()?),
            kind: read_effect_kind(r.byte()?)?,
            retry: r.byte()?,
        },
        2 => SagaTransitionV1::EffectAccepted {
            saga: SagaIdV1(r.u64()?),
            effect: SagaEffectIdV1(r.u64()?),
            outcome: read_effect_outcome(r.byte()?)?,
            phase: read_phase(r.byte()?)?,
        },
        3 => SagaTransitionV1::DeadlineApplied {
            saga: SagaIdV1(r.u64()?),
            timer: TimerIdV1(r.u64()?),
            action: read_deadline_action(r.byte()?)?,
            phase: read_phase(r.byte()?)?,
            compensation: r.option(|r| r.u64().map(SagaEffectIdV1))?,
        },
        4 => SagaTransitionV1::Cancelled {
            saga: SagaIdV1(r.u64()?),
            compensation: r.option(|r| r.u64().map(SagaEffectIdV1))?,
        },
        5 => SagaTransitionV1::Reconciled {
            saga: SagaIdV1(r.u64()?),
            action: OperatorActionIdV1(r.raw()?),
            phase: read_phase(r.byte()?)?,
            prepared: r.option(|r| r.u64().map(SagaEffectIdV1))?,
        },
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn refusal(w: &mut Writer, value: SagaRefusalV1) -> Result<(), CodecErrorV1> {
    match value {
        SagaRefusalV1::NotAuthorized => w.byte(0),
        SagaRefusalV1::InvalidLimits => w.byte(1),
        SagaRefusalV1::InvalidPageLimit => w.byte(2),
        SagaRefusalV1::RevisionConflict { expected, actual } => {
            w.byte(3)?;
            w.u64(expected)?;
            w.u64(actual)
        }
        SagaRefusalV1::RevisionExhausted => w.byte(4),
        SagaRefusalV1::IdentityExhausted => w.byte(5),
        SagaRefusalV1::SagaLimit => w.byte(6),
        SagaRefusalV1::InvalidDeadline => w.byte(7),
        SagaRefusalV1::RetryBudget => w.byte(8),
        SagaRefusalV1::InputConflict => w.byte(9),
        SagaRefusalV1::UnknownSaga => w.byte(10),
        SagaRefusalV1::InvalidPhase => w.byte(11),
        SagaRefusalV1::EffectMismatch => w.byte(12),
        SagaRefusalV1::EvidenceConflict => w.byte(13),
        SagaRefusalV1::DeadlineNotReached => w.byte(14),
        SagaRefusalV1::DeadlineReached => w.byte(15),
        SagaRefusalV1::TimerMismatch => w.byte(16),
        SagaRefusalV1::TimerConflict => w.byte(17),
        SagaRefusalV1::LateEvidence => w.byte(18),
        SagaRefusalV1::InvalidDeadlineAction => w.byte(19),
        SagaRefusalV1::InvalidCheckpoint => w.byte(20),
        SagaRefusalV1::OperatorActionLimit => w.byte(21),
        SagaRefusalV1::OperatorActionConflict => w.byte(22),
    }
}

fn read_refusal(r: &mut Reader<'_>) -> Result<SagaRefusalV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => SagaRefusalV1::NotAuthorized,
        1 => SagaRefusalV1::InvalidLimits,
        2 => SagaRefusalV1::InvalidPageLimit,
        3 => SagaRefusalV1::RevisionConflict {
            expected: r.u64()?,
            actual: r.u64()?,
        },
        4 => SagaRefusalV1::RevisionExhausted,
        5 => SagaRefusalV1::IdentityExhausted,
        6 => SagaRefusalV1::SagaLimit,
        7 => SagaRefusalV1::InvalidDeadline,
        8 => SagaRefusalV1::RetryBudget,
        9 => SagaRefusalV1::InputConflict,
        10 => SagaRefusalV1::UnknownSaga,
        11 => SagaRefusalV1::InvalidPhase,
        12 => SagaRefusalV1::EffectMismatch,
        13 => SagaRefusalV1::EvidenceConflict,
        14 => SagaRefusalV1::DeadlineNotReached,
        15 => SagaRefusalV1::DeadlineReached,
        16 => SagaRefusalV1::TimerMismatch,
        17 => SagaRefusalV1::TimerConflict,
        18 => SagaRefusalV1::LateEvidence,
        19 => SagaRefusalV1::InvalidDeadlineAction,
        20 => SagaRefusalV1::InvalidCheckpoint,
        21 => SagaRefusalV1::OperatorActionLimit,
        22 => SagaRefusalV1::OperatorActionConflict,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn write_len(w: &mut Writer, value: usize) -> Result<(), CodecErrorV1> {
    w.u32(u32::try_from(value).map_err(|_| CodecErrorV1::Limit)?)
}

fn read_vec<T>(
    r: &mut Reader<'_>,
    mut item: impl FnMut(&mut Reader<'_>) -> Result<T, CodecErrorV1>,
) -> Result<Vec<T>, CodecErrorV1> {
    let count = usize::try_from(r.u32()?).map_err(|_| CodecErrorV1::Limit)?;
    if count > r.bytes.len().saturating_sub(r.position) {
        return Err(CodecErrorV1::Limit);
    }
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..count {
        values.push(item(r)?);
    }
    Ok(values)
}

fn effect_kind_tag(value: SagaEffectKindV1) -> u8 {
    match value {
        SagaEffectKindV1::Forward => 0,
        SagaEffectKindV1::Terminal => 1,
        SagaEffectKindV1::Compensation => 2,
    }
}
fn read_effect_kind(value: u8) -> Result<SagaEffectKindV1, CodecErrorV1> {
    match value {
        0 => Ok(SagaEffectKindV1::Forward),
        1 => Ok(SagaEffectKindV1::Terminal),
        2 => Ok(SagaEffectKindV1::Compensation),
        _ => Err(CodecErrorV1::Unsupported),
    }
}
fn effect_outcome_tag(value: SagaEffectOutcomeV1) -> u8 {
    match value {
        SagaEffectOutcomeV1::Succeeded => 0,
        SagaEffectOutcomeV1::Failed => 1,
        SagaEffectOutcomeV1::Uncertain => 2,
    }
}
fn read_effect_outcome(value: u8) -> Result<SagaEffectOutcomeV1, CodecErrorV1> {
    match value {
        0 => Ok(SagaEffectOutcomeV1::Succeeded),
        1 => Ok(SagaEffectOutcomeV1::Failed),
        2 => Ok(SagaEffectOutcomeV1::Uncertain),
        _ => Err(CodecErrorV1::Unsupported),
    }
}
fn deadline_action_tag(value: DeadlineActionV1) -> u8 {
    match value {
        DeadlineActionV1::KeepUncertain => 0,
        DeadlineActionV1::Fail => 1,
        DeadlineActionV1::Compensate => 2,
    }
}
fn read_deadline_action(value: u8) -> Result<DeadlineActionV1, CodecErrorV1> {
    match value {
        0 => Ok(DeadlineActionV1::KeepUncertain),
        1 => Ok(DeadlineActionV1::Fail),
        2 => Ok(DeadlineActionV1::Compensate),
        _ => Err(CodecErrorV1::Unsupported),
    }
}
fn phase_tag(value: SagaPhaseV1) -> u8 {
    match value {
        SagaPhaseV1::ForwardPending => 0,
        SagaPhaseV1::ForwardSucceeded => 1,
        SagaPhaseV1::ForwardFailed => 2,
        SagaPhaseV1::TerminalPending => 3,
        SagaPhaseV1::TerminalFailed => 4,
        SagaPhaseV1::CompensationPending => 5,
        SagaPhaseV1::CompensationFailed => 6,
        SagaPhaseV1::FencedUncertain => 7,
        SagaPhaseV1::Completed => 8,
        SagaPhaseV1::Failed => 9,
        SagaPhaseV1::Cancelled => 10,
    }
}
fn read_phase(value: u8) -> Result<SagaPhaseV1, CodecErrorV1> {
    match value {
        0 => Ok(SagaPhaseV1::ForwardPending),
        1 => Ok(SagaPhaseV1::ForwardSucceeded),
        2 => Ok(SagaPhaseV1::ForwardFailed),
        3 => Ok(SagaPhaseV1::TerminalPending),
        4 => Ok(SagaPhaseV1::TerminalFailed),
        5 => Ok(SagaPhaseV1::CompensationPending),
        6 => Ok(SagaPhaseV1::CompensationFailed),
        7 => Ok(SagaPhaseV1::FencedUncertain),
        8 => Ok(SagaPhaseV1::Completed),
        9 => Ok(SagaPhaseV1::Failed),
        10 => Ok(SagaPhaseV1::Cancelled),
        _ => Err(CodecErrorV1::Unsupported),
    }
}
fn automatic_progress_tag(value: AutomaticProgressV1) -> u8 {
    match value {
        AutomaticProgressV1::Runnable => 0,
        AutomaticProgressV1::AwaitingDecision => 1,
        AutomaticProgressV1::BlockedUncertain => 2,
        AutomaticProgressV1::Terminal => 3,
    }
}
fn read_automatic_progress(value: u8) -> Result<AutomaticProgressV1, CodecErrorV1> {
    match value {
        0 => Ok(AutomaticProgressV1::Runnable),
        1 => Ok(AutomaticProgressV1::AwaitingDecision),
        2 => Ok(AutomaticProgressV1::BlockedUncertain),
        3 => Ok(AutomaticProgressV1::Terminal),
        _ => Err(CodecErrorV1::Unsupported),
    }
}
