//! Canonical bounded bytes for structured tool-continuation checkpoints.

use super::*;

const MAGIC: &[u8; 8] = b"MOIRATC3";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecLimitsV1 {
    pub max_bytes: usize,
    pub domain: LimitsV1,
}

impl Default for CodecLimitsV1 {
    fn default() -> Self {
        Self {
            max_bytes: 32 * 1_024 * 1_024,
            domain: LimitsV1::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecErrorV1 {
    InvalidLimits,
    Limit,
    Capacity,
    Truncated,
    Invalid,
    NonCanonical,
}

struct Writer {
    bytes: Vec<u8>,
    maximum: usize,
}

impl Writer {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
        }
    }

    fn raw(&mut self, value: &[u8]) -> Result<(), CodecErrorV1> {
        if value.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(CodecErrorV1::Limit);
        }
        self.bytes
            .try_reserve(value.len())
            .map_err(|_| CodecErrorV1::Capacity)?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn byte(&mut self, value: u8) -> Result<(), CodecErrorV1> {
        self.raw(&[value])
    }

    fn number(&mut self, value: u64) -> Result<(), CodecErrorV1> {
        self.raw(&value.to_be_bytes())
    }

    fn id(&mut self, value: [u8; 16]) -> Result<(), CodecErrorV1> {
        self.raw(&value)
    }

    fn digest(&mut self, value: [u8; 32]) -> Result<(), CodecErrorV1> {
        self.raw(&value)
    }

    fn count(&mut self, value: usize) -> Result<(), CodecErrorV1> {
        self.number(u64::try_from(value).map_err(|_| CodecErrorV1::Limit)?)
    }

    fn bytes(&mut self, value: &[u8], maximum: usize) -> Result<(), CodecErrorV1> {
        if value.len() > maximum {
            return Err(CodecErrorV1::Limit);
        }
        self.count(value.len())?;
        self.raw(value)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], maximum: usize) -> Result<Self, CodecErrorV1> {
        if bytes.len() > maximum {
            return Err(CodecErrorV1::Limit);
        }
        Ok(Self { bytes, position: 0 })
    }

    fn raw<const N: usize>(&mut self) -> Result<[u8; N], CodecErrorV1> {
        let end = self.position.checked_add(N).ok_or(CodecErrorV1::Invalid)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(CodecErrorV1::Truncated)?;
        self.position = end;
        value.try_into().map_err(|_| CodecErrorV1::Invalid)
    }

    fn byte(&mut self) -> Result<u8, CodecErrorV1> {
        Ok(self.raw::<1>()?[0])
    }

    fn number(&mut self) -> Result<u64, CodecErrorV1> {
        Ok(u64::from_be_bytes(self.raw()?))
    }

    fn id(&mut self) -> Result<[u8; 16], CodecErrorV1> {
        self.raw()
    }

    fn digest(&mut self) -> Result<[u8; 32], CodecErrorV1> {
        self.raw()
    }

    fn count(&mut self, maximum: usize) -> Result<usize, CodecErrorV1> {
        let value = usize::try_from(self.number()?).map_err(|_| CodecErrorV1::Limit)?;
        if value > maximum {
            return Err(CodecErrorV1::Limit);
        }
        Ok(value)
    }

    fn bytes(&mut self, maximum: usize) -> Result<Vec<u8>, CodecErrorV1> {
        let count = self.count(maximum)?;
        let end = self
            .position
            .checked_add(count)
            .ok_or(CodecErrorV1::Invalid)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(CodecErrorV1::Truncated)?;
        self.position = end;
        let mut owned = Vec::new();
        owned
            .try_reserve(count)
            .map_err(|_| CodecErrorV1::Capacity)?;
        owned.extend_from_slice(value);
        Ok(owned)
    }

    fn finish(self) -> Result<(), CodecErrorV1> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(CodecErrorV1::Invalid)
        }
    }
}

pub fn encode(checkpoint: &CheckpointV1, limits: CodecLimitsV1) -> Result<Vec<u8>, CodecErrorV1> {
    validate_limits(limits)?;
    host_bounds(checkpoint.limits, limits.domain)?;
    Engine::recover(checkpoint.clone()).map_err(|_| CodecErrorV1::Invalid)?;
    let mut writer = Writer::new(limits.max_bytes);
    writer.raw(MAGIC)?;
    writer.byte(3)?;
    encode_limits(&mut writer, checkpoint.limits)?;
    writer.number(checkpoint.next_effect)?;
    writer.count(checkpoint.sagas.len())?;
    for saga in &checkpoint.sagas {
        writer.id(saga.workflow.into_bytes())?;
        writer.id(saga.source_execution.into_bytes())?;
        encode_attempt(&mut writer, saga.source_attempt)?;
        encode_phase(&mut writer, saga.phase)?;
        writer.count(saga.effects.len())?;
        for effect in &saga.effects {
            encode_effect(&mut writer, effect, checkpoint.limits.max_argument_bytes)?;
        }
    }
    Ok(writer.bytes)
}

pub fn decode(bytes: &[u8], limits: CodecLimitsV1) -> Result<CheckpointV1, CodecErrorV1> {
    validate_limits(limits)?;
    let mut reader = Reader::new(bytes, limits.max_bytes)?;
    if reader.raw::<8>()? != *MAGIC || reader.byte()? != 3 {
        return Err(CodecErrorV1::Invalid);
    }
    let domain = decode_limits(&mut reader, limits.domain)?;
    let next_effect = reader.number()?;
    let saga_count = reader.count(domain.max_sagas)?;
    let mut sagas = Vec::new();
    sagas
        .try_reserve(saga_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..saga_count {
        let workflow = WorkflowId::new(reader.id()?);
        let source_execution = EffectId::new(reader.id()?);
        let source_attempt = decode_attempt(&mut reader)?;
        let phase = decode_phase(&mut reader)?;
        let effect_count = reader.count(domain.max_calls_per_saga)?;
        let mut effects = Vec::new();
        effects
            .try_reserve(effect_count)
            .map_err(|_| CodecErrorV1::Capacity)?;
        for _ in 0..effect_count {
            effects.push(decode_effect(&mut reader, domain.max_argument_bytes)?);
        }
        sagas.push(SnapshotV1 {
            workflow,
            source_execution,
            source_attempt,
            phase,
            effects,
        });
    }
    reader.finish()?;
    let checkpoint = CheckpointV1 {
        version: 3,
        limits: domain,
        next_effect,
        sagas,
    };
    Engine::recover(checkpoint.clone()).map_err(|_| CodecErrorV1::Invalid)?;
    if encode(&checkpoint, limits)? != bytes {
        return Err(CodecErrorV1::NonCanonical);
    }
    Ok(checkpoint)
}

fn encode_limits(writer: &mut Writer, limits: LimitsV1) -> Result<(), CodecErrorV1> {
    writer.count(limits.max_sagas)?;
    writer.count(limits.max_calls_per_saga)?;
    writer.count(limits.max_argument_bytes)?;
    writer.number(limits.max_elapsed_millis)?;
    writer.count(limits.max_output_bytes)
}

fn decode_limits(reader: &mut Reader<'_>, host: LimitsV1) -> Result<LimitsV1, CodecErrorV1> {
    let limits = LimitsV1 {
        max_sagas: nonzero_count(reader, host.max_sagas)?,
        max_calls_per_saga: nonzero_count(reader, host.max_calls_per_saga)?,
        max_argument_bytes: nonzero_count(reader, host.max_argument_bytes)?,
        max_elapsed_millis: reader.number()?,
        max_output_bytes: nonzero_count(reader, host.max_output_bytes)?,
    };
    if limits.max_elapsed_millis == 0 || limits.max_elapsed_millis > host.max_elapsed_millis {
        return Err(CodecErrorV1::Limit);
    }
    Ok(limits)
}

fn nonzero_count(reader: &mut Reader<'_>, maximum: usize) -> Result<usize, CodecErrorV1> {
    let value = reader.count(maximum)?;
    if value == 0 {
        Err(CodecErrorV1::Limit)
    } else {
        Ok(value)
    }
}

fn encode_phase(writer: &mut Writer, phase: PhaseV1) -> Result<(), CodecErrorV1> {
    match phase {
        PhaseV1::Pending => writer.byte(0),
        PhaseV1::FencedUncertain { effect } => {
            writer.byte(1)?;
            writer.id(effect.into_bytes())
        }
        PhaseV1::Completed => writer.byte(2),
        PhaseV1::Failed { effect } => {
            writer.byte(3)?;
            writer.id(effect.into_bytes())
        }
    }
}

fn decode_phase(reader: &mut Reader<'_>) -> Result<PhaseV1, CodecErrorV1> {
    Ok(match reader.byte()? {
        0 => PhaseV1::Pending,
        1 => PhaseV1::FencedUncertain {
            effect: EffectId::new(reader.id()?),
        },
        2 => PhaseV1::Completed,
        3 => PhaseV1::Failed {
            effect: EffectId::new(reader.id()?),
        },
        _ => return Err(CodecErrorV1::Invalid),
    })
}

fn encode_attempt(writer: &mut Writer, attempt: AttemptEvidenceV1) -> Result<(), CodecErrorV1> {
    writer.id(attempt.task_key)?;
    writer.number(attempt.incarnation)?;
    writer.number(u64::from(attempt.attempt))
}

fn decode_attempt(reader: &mut Reader<'_>) -> Result<AttemptEvidenceV1, CodecErrorV1> {
    Ok(AttemptEvidenceV1 {
        task_key: reader.id()?,
        incarnation: reader.number()?,
        attempt: u32::try_from(reader.number()?).map_err(|_| CodecErrorV1::Limit)?,
    })
}

fn encode_effect(
    writer: &mut Writer,
    effect: &ToolEffectV1,
    maximum: usize,
) -> Result<(), CodecErrorV1> {
    writer.id(effect.effect.into_bytes())?;
    writer.id(effect.call.request.0)?;
    writer.id(effect.call.capability.0)?;
    writer.bytes(&effect.call.provider_correlation, maximum)?;
    writer.number(effect.call.budget.max_elapsed_millis)?;
    writer.count(effect.call.budget.max_output_bytes)?;
    writer.bytes(&effect.call.arguments, maximum)?;
    match effect.state {
        ToolEffectStateV1::Queued => writer.byte(0),
        ToolEffectStateV1::PendingAuthorization => writer.byte(1),
        ToolEffectStateV1::Ready(gate) => {
            writer.byte(2)?;
            encode_gate(writer, gate)
        }
        ToolEffectStateV1::InFlight { gate, execution } => {
            writer.byte(3)?;
            encode_gate(writer, gate)?;
            writer.id(execution.0)
        }
        ToolEffectStateV1::Succeeded { gate, receipt } => {
            writer.byte(4)?;
            encode_gate(writer, gate)?;
            encode_receipt(writer, receipt)
        }
        ToolEffectStateV1::Failed { gate, failure } => {
            writer.byte(5)?;
            encode_gate(writer, gate)?;
            encode_failure(writer, failure)
        }
        ToolEffectStateV1::Uncertain { gate, execution } => {
            writer.byte(6)?;
            encode_gate(writer, gate)?;
            writer.id(execution.0)
        }
    }
}

fn decode_effect(reader: &mut Reader<'_>, maximum: usize) -> Result<ToolEffectV1, CodecErrorV1> {
    let effect = EffectId::new(reader.id()?);
    let call = ToolCallV1 {
        request: ToolRequestIdV1(reader.id()?),
        capability: ToolCapabilityIdV1(reader.id()?),
        provider_correlation: reader.bytes(maximum)?,
        budget: ToolBudgetV1 {
            max_elapsed_millis: reader.number()?,
            max_output_bytes: usize::try_from(reader.number()?).map_err(|_| CodecErrorV1::Limit)?,
        },
        arguments: reader.bytes(maximum)?,
    };
    let state = match reader.byte()? {
        0 => ToolEffectStateV1::Queued,
        1 => ToolEffectStateV1::PendingAuthorization,
        2 => ToolEffectStateV1::Ready(decode_gate(reader)?),
        3 => ToolEffectStateV1::InFlight {
            gate: decode_gate(reader)?,
            execution: ToolExecutionIdV1(reader.id()?),
        },
        4 => ToolEffectStateV1::Succeeded {
            gate: decode_gate(reader)?,
            receipt: decode_receipt(reader)?,
        },
        5 => ToolEffectStateV1::Failed {
            gate: decode_gate(reader)?,
            failure: decode_failure(reader)?,
        },
        6 => ToolEffectStateV1::Uncertain {
            gate: decode_gate(reader)?,
            execution: ToolExecutionIdV1(reader.id()?),
        },
        _ => return Err(CodecErrorV1::Invalid),
    };
    Ok(ToolEffectV1 {
        effect,
        call,
        state,
    })
}

fn encode_gate(writer: &mut Writer, gate: ToolGateV1) -> Result<(), CodecErrorV1> {
    writer.id(gate.authority.decision.0)?;
    writer.id(gate.authority.subject.0)?;
    writer.id(gate.authority.capability.0)?;
    writer.number(gate.authority.epoch)?;
    writer.id(gate.attempt.task_key)?;
    writer.number(gate.attempt.incarnation)?;
    writer.number(u64::from(gate.attempt.attempt))
}

fn decode_gate(reader: &mut Reader<'_>) -> Result<ToolGateV1, CodecErrorV1> {
    Ok(ToolGateV1 {
        authority: AuthorityEvidenceV1 {
            decision: AuthorityDecisionIdV1(reader.id()?),
            subject: AuthoritySubjectIdV1(reader.id()?),
            capability: ToolCapabilityIdV1(reader.id()?),
            epoch: reader.number()?,
        },
        attempt: AttemptEvidenceV1 {
            task_key: reader.id()?,
            incarnation: reader.number()?,
            attempt: u32::try_from(reader.number()?).map_err(|_| CodecErrorV1::Invalid)?,
        },
    })
}

fn encode_receipt(writer: &mut Writer, receipt: ToolReceiptV1) -> Result<(), CodecErrorV1> {
    writer.id(receipt.effect.into_bytes())?;
    writer.id(receipt.capability.0)?;
    writer.id(receipt.execution.0)?;
    writer.digest(receipt.evidence_digest)?;
    writer.digest(receipt.output_digest)?;
    writer.number(u64::try_from(receipt.output_bytes).map_err(|_| CodecErrorV1::Limit)?)
}

fn decode_receipt(reader: &mut Reader<'_>) -> Result<ToolReceiptV1, CodecErrorV1> {
    Ok(ToolReceiptV1 {
        effect: EffectId::new(reader.id()?),
        capability: ToolCapabilityIdV1(reader.id()?),
        execution: ToolExecutionIdV1(reader.id()?),
        evidence_digest: reader.digest()?,
        output_digest: reader.digest()?,
        output_bytes: usize::try_from(reader.number()?).map_err(|_| CodecErrorV1::Limit)?,
    })
}

fn encode_failure(writer: &mut Writer, failure: ToolFailureV1) -> Result<(), CodecErrorV1> {
    writer.id(failure.effect.into_bytes())?;
    writer.id(failure.capability.0)?;
    writer.id(failure.execution.0)?;
    writer.digest(failure.evidence_digest)
}

fn decode_failure(reader: &mut Reader<'_>) -> Result<ToolFailureV1, CodecErrorV1> {
    Ok(ToolFailureV1 {
        effect: EffectId::new(reader.id()?),
        capability: ToolCapabilityIdV1(reader.id()?),
        execution: ToolExecutionIdV1(reader.id()?),
        evidence_digest: reader.digest()?,
    })
}

fn validate_limits(limits: CodecLimitsV1) -> Result<(), CodecErrorV1> {
    if limits.max_bytes == 0 || Engine::try_new(limits.domain).is_err() {
        Err(CodecErrorV1::InvalidLimits)
    } else {
        Ok(())
    }
}

fn host_bounds(value: LimitsV1, host: LimitsV1) -> Result<(), CodecErrorV1> {
    if value.max_sagas > host.max_sagas
        || value.max_calls_per_saga > host.max_calls_per_saga
        || value.max_argument_bytes > host.max_argument_bytes
        || value.max_elapsed_millis > host.max_elapsed_millis
        || value.max_output_bytes > host.max_output_bytes
    {
        Err(CodecErrorV1::Limit)
    } else {
        Ok(())
    }
}
