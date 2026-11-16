//! Canonical bounded bytes for the closed saga profile.
//!
//! This is a semantic checkpoint contract, not a storage implementation.

use super::*;
use keystone_codec::{DecodeError, DecodeLimits, Decoder};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SagaCodecLimitsV1 {
    pub max_bytes: usize,
    pub max_sagas: usize,
    pub max_operator_actions: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SagaCodecErrorV1 {
    Limit,
    Capacity,
    Decode(DecodeError),
    Profile,
    InvalidValue,
    NonCanonical,
    Trailing,
    Recovery(SagaRefusalV1),
}

impl From<DecodeError> for SagaCodecErrorV1 {
    fn from(value: DecodeError) -> Self {
        Self::Decode(value)
    }
}

impl std::fmt::Display for SagaCodecErrorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "saga checkpoint codec: {self:?}")
    }
}

impl std::error::Error for SagaCodecErrorV1 {}

pub struct EncodedSagaCheckpointV1(Vec<u8>);

impl EncodedSagaCheckpointV1 {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for EncodedSagaCheckpointV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EncodedSagaCheckpointV1(<redacted>)")
    }
}

struct Writer {
    bytes: Vec<u8>,
    max: usize,
}

impl Writer {
    fn reserve(&mut self, count: usize) -> Result<(), SagaCodecErrorV1> {
        if self
            .bytes
            .len()
            .checked_add(count)
            .is_none_or(|length| length > self.max)
        {
            return Err(SagaCodecErrorV1::Limit);
        }
        self.bytes
            .try_reserve(count)
            .map_err(|_| SagaCodecErrorV1::Capacity)
    }

    fn byte(&mut self, value: u8) -> Result<(), SagaCodecErrorV1> {
        self.reserve(1)?;
        self.bytes.push(value);
        Ok(())
    }

    fn number(&mut self, value: u64) -> Result<(), SagaCodecErrorV1> {
        self.reserve(8)?;
        self.bytes.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }

    fn bytes(&mut self, value: &[u8]) -> Result<(), SagaCodecErrorV1> {
        self.reserve(value.len().checked_add(4).ok_or(SagaCodecErrorV1::Limit)?)?;
        keystone_codec::encode_bytes(&mut self.bytes, value).map_err(|_| SagaCodecErrorV1::Limit)
    }

    fn option<T: Copy>(
        &mut self,
        value: Option<T>,
        write: impl FnOnce(&mut Self, T) -> Result<(), SagaCodecErrorV1>,
    ) -> Result<(), SagaCodecErrorV1> {
        self.byte(u8::from(value.is_some()))?;
        if let Some(value) = value {
            write(self, value)?;
        }
        Ok(())
    }
}

struct Reader<'a>(Decoder<'a>);

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, SagaCodecErrorV1> {
        Ok(self.0.read_u8()?)
    }

    fn number(&mut self) -> Result<u64, SagaCodecErrorV1> {
        Ok(self.0.read_u64_be()?)
    }

    fn source(&mut self) -> Result<[u8; 16], SagaCodecErrorV1> {
        self.0
            .read_bytes()?
            .try_into()
            .map_err(|_| SagaCodecErrorV1::InvalidValue)
    }

    fn option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, SagaCodecErrorV1>,
    ) -> Result<Option<T>, SagaCodecErrorV1> {
        match self.byte()? {
            0 => Ok(None),
            1 => read(self).map(Some),
            _ => Err(SagaCodecErrorV1::InvalidValue),
        }
    }
}

pub fn encode_saga_checkpoint(
    checkpoint: &SagaCheckpointV1,
    limits: SagaCodecLimitsV1,
) -> Result<EncodedSagaCheckpointV1, SagaCodecErrorV1> {
    fields(limits)?;
    if checkpoint.limits.max_sagas > limits.max_sagas {
        return Err(SagaCodecErrorV1::Limit);
    }
    if checkpoint.limits.max_operator_actions > limits.max_operator_actions {
        return Err(SagaCodecErrorV1::Limit);
    }
    SagaEngineV1::recover(checkpoint.clone()).map_err(SagaCodecErrorV1::Recovery)?;
    let mut writer = Writer {
        bytes: Vec::new(),
        max: limits.max_bytes,
    };
    writer.bytes(b"MOIRASAGA")?;
    writer.byte(1)?;
    writer.number(to_u64(checkpoint.limits.max_sagas)?)?;
    writer.byte(checkpoint.limits.max_retries_per_saga)?;
    writer.number(to_u64(checkpoint.limits.max_inspection_page)?)?;
    writer.number(to_u64(checkpoint.limits.max_operator_actions)?)?;
    writer.number(checkpoint.revision)?;
    writer.number(checkpoint.next_saga)?;
    writer.number(checkpoint.next_effect)?;
    writer.number(to_u64(checkpoint.sagas.len())?)?;
    for state in checkpoint.sagas.values() {
        write_state(&mut writer, state)?;
    }
    writer.number(to_u64(checkpoint.operator_actions.len())?)?;
    for receipt in checkpoint.operator_actions.values() {
        write_operator_receipt(&mut writer, receipt)?;
    }
    Ok(EncodedSagaCheckpointV1(writer.bytes))
}

pub fn decode_saga_checkpoint(
    bytes: &[u8],
    limits: SagaCodecLimitsV1,
) -> Result<SagaCheckpointV1, SagaCodecErrorV1> {
    let mut reader = Reader(Decoder::new(
        bytes,
        DecodeLimits {
            max_input_bytes: limits.max_bytes,
            max_field_bytes: 16,
            max_fields: fields(limits)?,
        },
    )?);
    if reader.0.read_bytes()? != b"MOIRASAGA" || reader.byte()? != 1 {
        return Err(SagaCodecErrorV1::Profile);
    }
    let max_sagas = to_usize(reader.number()?)?;
    let max_retries_per_saga = reader.byte()?;
    let max_inspection_page = to_usize(reader.number()?)?;
    let max_operator_actions = to_usize(reader.number()?)?;
    if max_sagas == 0
        || max_sagas > limits.max_sagas
        || max_inspection_page == 0
        || max_operator_actions == 0
        || max_operator_actions > limits.max_operator_actions
    {
        return Err(SagaCodecErrorV1::Limit);
    }
    let revision = reader.number()?;
    let next_saga = reader.number()?;
    let next_effect = reader.number()?;
    let count = to_usize(reader.number()?)?;
    if count > max_sagas {
        return Err(SagaCodecErrorV1::Limit);
    }
    let mut sagas = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut previous = None;
    for _ in 0..count {
        let state = read_state(&mut reader, max_retries_per_saga)?;
        let saga = state.snapshot.saga;
        if previous.is_some_and(|prior| prior >= saga) {
            return Err(SagaCodecErrorV1::NonCanonical);
        }
        previous = Some(saga);
        if sources.insert(state.source, saga).is_some() {
            return Err(SagaCodecErrorV1::InvalidValue);
        }
        sagas.insert(saga, state);
    }
    let operator_count = bounded_count(&mut reader, max_operator_actions)?;
    let mut operator_actions = BTreeMap::new();
    let mut previous_action = None;
    for _ in 0..operator_count {
        let receipt = read_operator_receipt(&mut reader)?;
        if previous_action.is_some_and(|prior| prior >= receipt.cause.action) {
            return Err(SagaCodecErrorV1::NonCanonical);
        }
        previous_action = Some(receipt.cause.action);
        operator_actions.insert(receipt.cause.action, receipt);
    }
    if !reader.0.is_finished() {
        return Err(SagaCodecErrorV1::Trailing);
    }
    let checkpoint = SagaCheckpointV1 {
        limits: SagaLimitsV1 {
            max_sagas,
            max_retries_per_saga,
            max_inspection_page,
            max_operator_actions,
        },
        revision,
        next_saga,
        next_effect,
        sagas,
        sources,
        operator_actions,
    };
    SagaEngineV1::recover(checkpoint.clone()).map_err(SagaCodecErrorV1::Recovery)?;
    Ok(checkpoint)
}

fn write_state(writer: &mut Writer, state: &SagaState) -> Result<(), SagaCodecErrorV1> {
    writer.bytes(&state.source)?;
    writer.number(state.last_confirmed_revision)?;
    writer.number(state.snapshot.saga.0)?;
    writer.byte(phase_tag(state.snapshot.phase))?;
    writer.number(state.snapshot.deadline)?;
    writer.number(state.snapshot.timer.0)?;
    writer.option(state.snapshot.current_effect, |writer, effect| {
        writer.number(effect.0)
    })?;
    writer.option(state.snapshot.current_kind, |writer, kind| {
        writer.byte(kind_tag(kind))
    })?;
    writer.byte(state.snapshot.retries_used)?;
    writer.byte(state.snapshot.retry_budget)?;
    writer.number(state.initial_effect.0)?;
    writer.option(state.retry_of, |writer, effect| writer.number(effect.0))?;
    writer.number(to_u64(state.allocated.len())?)?;
    for effect in &state.allocated {
        writer.number(effect.0)?;
    }
    writer.number(to_u64(state.accepted.len())?)?;
    for (effect, evidence) in &state.accepted {
        let SagaTransitionV1::EffectAccepted { phase, .. } = evidence.transition else {
            return Err(SagaCodecErrorV1::InvalidValue);
        };
        writer.number(effect.0)?;
        writer.byte(outcome_tag(evidence.outcome))?;
        writer.number(evidence.observed_at)?;
        writer.byte(phase_tag(phase))?;
    }
    writer.option(state.timer_evidence, |writer, evidence| {
        let SagaTransitionV1::DeadlineApplied {
            phase,
            compensation,
            ..
        } = evidence.transition
        else {
            return Err(SagaCodecErrorV1::InvalidValue);
        };
        writer.number(evidence.observed_at)?;
        writer.byte(action_tag(evidence.action))?;
        writer.byte(phase_tag(phase))?;
        writer.option(compensation, |writer, effect| writer.number(effect.0))
    })
}

fn read_state(reader: &mut Reader<'_>, max_retries: u8) -> Result<SagaState, SagaCodecErrorV1> {
    let source = reader.source()?;
    let last_confirmed_revision = reader.number()?;
    let saga = SagaIdV1(reader.number()?);
    let phase = read_phase(reader.byte()?)?;
    let deadline = reader.number()?;
    let timer = TimerIdV1(reader.number()?);
    let current_effect = reader.option(|reader| Ok(SagaEffectIdV1(reader.number()?)))?;
    let current_kind = reader.option(|reader| read_kind(reader.byte()?))?;
    let retries_used = reader.byte()?;
    let retry_budget = reader.byte()?;
    let initial_effect = SagaEffectIdV1(reader.number()?);
    let retry_of = reader.option(|reader| Ok(SagaEffectIdV1(reader.number()?)))?;
    let maximum_effects = usize::from(max_retries)
        .checked_add(3)
        .ok_or(SagaCodecErrorV1::Limit)?;
    let allocated_count = bounded_count(reader, maximum_effects)?;
    let mut allocated = BTreeSet::new();
    let mut previous = None;
    for _ in 0..allocated_count {
        let effect = SagaEffectIdV1(reader.number()?);
        if previous.is_some_and(|prior| prior >= effect) {
            return Err(SagaCodecErrorV1::NonCanonical);
        }
        previous = Some(effect);
        allocated.insert(effect);
    }
    let accepted_count = bounded_count(reader, allocated_count)?;
    let mut accepted = BTreeMap::new();
    previous = None;
    for _ in 0..accepted_count {
        let effect = SagaEffectIdV1(reader.number()?);
        if previous.is_some_and(|prior| prior >= effect) {
            return Err(SagaCodecErrorV1::NonCanonical);
        }
        previous = Some(effect);
        let outcome = read_outcome(reader.byte()?)?;
        let observed_at = reader.number()?;
        let accepted_phase = read_phase(reader.byte()?)?;
        accepted.insert(
            effect,
            AcceptedEvidence {
                outcome,
                observed_at,
                transition: SagaTransitionV1::EffectAccepted {
                    saga,
                    effect,
                    outcome,
                    phase: accepted_phase,
                },
            },
        );
    }
    let timer_evidence = reader.option(|reader| {
        let observed_at = reader.number()?;
        let action = read_action(reader.byte()?)?;
        let applied_phase = read_phase(reader.byte()?)?;
        let compensation = reader.option(|reader| Ok(SagaEffectIdV1(reader.number()?)))?;
        Ok(TimerEvidence {
            observed_at,
            action,
            transition: SagaTransitionV1::DeadlineApplied {
                saga,
                timer,
                action,
                phase: applied_phase,
                compensation,
            },
        })
    })?;
    Ok(SagaState {
        source,
        initial_effect,
        snapshot: SagaSnapshotV1 {
            saga,
            phase,
            deadline,
            timer,
            current_effect,
            current_kind,
            retries_used,
            retry_budget,
        },
        retry_of,
        allocated,
        accepted,
        timer_evidence,
        last_confirmed_revision,
    })
}

fn write_operator_receipt(
    writer: &mut Writer,
    receipt: &OperatorReceipt,
) -> Result<(), SagaCodecErrorV1> {
    writer.bytes(&receipt.cause.action.0)?;
    writer.bytes(&receipt.cause.subject)?;
    writer.number(receipt.cause.authority_decision)?;
    writer.number(receipt.cause.authority_epoch)?;
    writer.number(receipt.saga.0)?;
    write_operator_disposition(writer, receipt.disposition)?;
    let SagaTransitionV1::Reconciled {
        phase, prepared, ..
    } = receipt.transition
    else {
        return Err(SagaCodecErrorV1::InvalidValue);
    };
    writer.byte(phase_tag(phase))?;
    writer.option(prepared, |writer, effect| writer.number(effect.0))
}

fn read_operator_receipt(reader: &mut Reader<'_>) -> Result<OperatorReceipt, SagaCodecErrorV1> {
    let action = OperatorActionIdV1(reader.source()?);
    let subject = reader.source()?;
    let authority_decision = reader.number()?;
    let authority_epoch = reader.number()?;
    let saga = SagaIdV1(reader.number()?);
    let disposition = read_operator_disposition(reader)?;
    let phase = read_phase(reader.byte()?)?;
    let prepared = reader.option(|reader| Ok(SagaEffectIdV1(reader.number()?)))?;
    let cause = OperatorCauseV1 {
        action,
        subject,
        authority_decision,
        authority_epoch,
    };
    Ok(OperatorReceipt {
        cause,
        saga,
        disposition,
        transition: SagaTransitionV1::Reconciled {
            saga,
            action,
            phase,
            prepared,
        },
    })
}

fn write_operator_disposition(
    writer: &mut Writer,
    value: OperatorDispositionV1,
) -> Result<(), SagaCodecErrorV1> {
    match value {
        OperatorDispositionV1::AdmitEvidence {
            effect,
            outcome,
            observed_at,
        } => {
            writer.byte(0)?;
            writer.number(effect.0)?;
            writer.byte(outcome_tag(outcome))?;
            writer.number(observed_at)
        }
        OperatorDispositionV1::KeepUncertain => writer.byte(1),
        OperatorDispositionV1::Retry { failed_effect, now } => {
            writer.byte(2)?;
            writer.number(failed_effect.0)?;
            writer.number(now)
        }
        OperatorDispositionV1::Compensate { now } => {
            writer.byte(3)?;
            writer.number(now)
        }
        OperatorDispositionV1::Cancel { now } => {
            writer.byte(4)?;
            writer.number(now)
        }
        OperatorDispositionV1::FailTerminal => writer.byte(5),
    }
}

fn read_operator_disposition(
    reader: &mut Reader<'_>,
) -> Result<OperatorDispositionV1, SagaCodecErrorV1> {
    match reader.byte()? {
        0 => Ok(OperatorDispositionV1::AdmitEvidence {
            effect: SagaEffectIdV1(reader.number()?),
            outcome: read_outcome(reader.byte()?)?,
            observed_at: reader.number()?,
        }),
        1 => Ok(OperatorDispositionV1::KeepUncertain),
        2 => Ok(OperatorDispositionV1::Retry {
            failed_effect: SagaEffectIdV1(reader.number()?),
            now: reader.number()?,
        }),
        3 => Ok(OperatorDispositionV1::Compensate {
            now: reader.number()?,
        }),
        4 => Ok(OperatorDispositionV1::Cancel {
            now: reader.number()?,
        }),
        5 => Ok(OperatorDispositionV1::FailTerminal),
        _ => Err(SagaCodecErrorV1::InvalidValue),
    }
}

fn fields(limits: SagaCodecLimitsV1) -> Result<usize, SagaCodecErrorV1> {
    if limits.max_bytes == 0
        || limits.max_bytes > isize::MAX as usize
        || limits.max_sagas == 0
        || limits.max_operator_actions == 0
    {
        return Err(SagaCodecErrorV1::Limit);
    }
    limits
        .max_sagas
        .checked_mul(2_100)
        .and_then(|value| {
            limits
                .max_operator_actions
                .checked_mul(24)
                .and_then(|operators| value.checked_add(operators))
        })
        .and_then(|value| value.checked_add(16))
        .ok_or(SagaCodecErrorV1::Limit)
}

fn bounded_count(reader: &mut Reader<'_>, max: usize) -> Result<usize, SagaCodecErrorV1> {
    let count = to_usize(reader.number()?)?;
    if count > max {
        return Err(SagaCodecErrorV1::Limit);
    }
    Ok(count)
}

fn to_u64(value: usize) -> Result<u64, SagaCodecErrorV1> {
    value.try_into().map_err(|_| SagaCodecErrorV1::Limit)
}

fn to_usize(value: u64) -> Result<usize, SagaCodecErrorV1> {
    value.try_into().map_err(|_| SagaCodecErrorV1::Limit)
}

fn kind_tag(value: EffectKindV1) -> u8 {
    match value {
        EffectKindV1::Forward => 0,
        EffectKindV1::Terminal => 1,
        EffectKindV1::Compensation => 2,
    }
}

fn read_kind(value: u8) -> Result<EffectKindV1, SagaCodecErrorV1> {
    match value {
        0 => Ok(EffectKindV1::Forward),
        1 => Ok(EffectKindV1::Terminal),
        2 => Ok(EffectKindV1::Compensation),
        _ => Err(SagaCodecErrorV1::InvalidValue),
    }
}

fn outcome_tag(value: EffectOutcomeV1) -> u8 {
    match value {
        EffectOutcomeV1::Succeeded => 0,
        EffectOutcomeV1::Failed => 1,
        EffectOutcomeV1::Uncertain => 2,
    }
}

fn read_outcome(value: u8) -> Result<EffectOutcomeV1, SagaCodecErrorV1> {
    match value {
        0 => Ok(EffectOutcomeV1::Succeeded),
        1 => Ok(EffectOutcomeV1::Failed),
        2 => Ok(EffectOutcomeV1::Uncertain),
        _ => Err(SagaCodecErrorV1::InvalidValue),
    }
}

fn action_tag(value: DeadlineActionV1) -> u8 {
    match value {
        DeadlineActionV1::KeepUncertain => 0,
        DeadlineActionV1::Fail => 1,
        DeadlineActionV1::Compensate => 2,
    }
}

fn read_action(value: u8) -> Result<DeadlineActionV1, SagaCodecErrorV1> {
    match value {
        0 => Ok(DeadlineActionV1::KeepUncertain),
        1 => Ok(DeadlineActionV1::Fail),
        2 => Ok(DeadlineActionV1::Compensate),
        _ => Err(SagaCodecErrorV1::InvalidValue),
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

fn read_phase(value: u8) -> Result<SagaPhaseV1, SagaCodecErrorV1> {
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
        _ => Err(SagaCodecErrorV1::InvalidValue),
    }
}
