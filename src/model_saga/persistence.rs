//! Canonical bounded bytes for model-proposal saga checkpoints.

use super::*;

const MAGIC: &[u8; 8] = b"MOIRAMP2";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecLimitsV1 {
    pub max_bytes: usize,
    pub max_sagas: usize,
    pub max_proposals_per_saga: usize,
    pub max_proposal_bytes: usize,
}

impl Default for CodecLimitsV1 {
    fn default() -> Self {
        Self {
            max_bytes: 32 * 1024 * 1024,
            max_sagas: 1_024,
            max_proposals_per_saga: 64,
            max_proposal_bytes: 1024 * 1024,
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

    fn raw(&mut self, bytes: &[u8]) -> Result<(), CodecErrorV1> {
        if bytes.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(CodecErrorV1::Limit);
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| CodecErrorV1::Capacity)?;
        self.bytes.extend_from_slice(bytes);
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

    fn bytes(&mut self, value: &[u8], maximum: usize) -> Result<(), CodecErrorV1> {
        if value.len() > maximum {
            return Err(CodecErrorV1::Limit);
        }
        self.number(u64::try_from(value.len()).map_err(|_| CodecErrorV1::Limit)?)?;
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
        let count = usize::try_from(self.number()?).map_err(|_| CodecErrorV1::Limit)?;
        if count > maximum || count > self.bytes.len().saturating_sub(self.position) {
            return Err(CodecErrorV1::Limit);
        }
        Ok(count)
    }

    fn bounded_number(&mut self, maximum: usize) -> Result<usize, CodecErrorV1> {
        let value = usize::try_from(self.number()?).map_err(|_| CodecErrorV1::Limit)?;
        if value == 0 || value > maximum {
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
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(count)
            .map_err(|_| CodecErrorV1::Capacity)?;
        owned.extend_from_slice(value);
        self.position = end;
        Ok(owned)
    }

    fn finish(self) -> Result<(), CodecErrorV1> {
        (self.position == self.bytes.len())
            .then_some(())
            .ok_or(CodecErrorV1::Invalid)
    }
}

pub fn encode(checkpoint: &CheckpointV1, limits: CodecLimitsV1) -> Result<Vec<u8>, CodecErrorV1> {
    validate_limits(limits)?;
    if checkpoint.limits.max_sagas > limits.max_sagas
        || checkpoint.limits.max_proposals_per_saga > limits.max_proposals_per_saga
        || checkpoint.limits.max_proposal_bytes > limits.max_proposal_bytes
        || checkpoint.sagas.len() > limits.max_sagas
    {
        return Err(CodecErrorV1::Limit);
    }
    Engine::recover(checkpoint.clone()).map_err(|_| CodecErrorV1::Invalid)?;
    let mut writer = Writer::new(limits.max_bytes);
    writer.raw(MAGIC)?;
    writer.byte(2)?;
    writer.number(to_u64(checkpoint.limits.max_sagas)?)?;
    writer.number(to_u64(checkpoint.limits.max_proposals_per_saga)?)?;
    writer.number(to_u64(checkpoint.limits.max_proposal_bytes)?)?;
    writer.number(checkpoint.next_effect)?;
    writer.number(to_u64(checkpoint.sagas.len())?)?;
    for saga in checkpoint.sagas.values() {
        writer.id(saga.workflow.into_bytes())?;
        writer.id(saga.daimon_effect.into_bytes())?;
        writer.digest(saga.selection_digest)?;
        encode_phase(&mut writer, saga.phase)?;
        writer.number(to_u64(saga.effects.len())?)?;
        for effect in &saga.effects {
            writer.id(effect.effect.into_bytes())?;
            encode_proposal(&mut writer, &effect.proposal, limits.max_proposal_bytes)?;
            encode_state(&mut writer, effect.state)?;
        }
    }
    Ok(writer.bytes)
}

pub fn decode(bytes: &[u8], limits: CodecLimitsV1) -> Result<CheckpointV1, CodecErrorV1> {
    validate_limits(limits)?;
    let mut reader = Reader::new(bytes, limits.max_bytes)?;
    if reader.raw::<8>()? != *MAGIC || reader.byte()? != 2 {
        return Err(CodecErrorV1::Invalid);
    }
    let domain_limits = LimitsV1 {
        max_sagas: reader.bounded_number(limits.max_sagas)?,
        max_proposals_per_saga: reader.bounded_number(limits.max_proposals_per_saga)?,
        max_proposal_bytes: reader.bounded_number(limits.max_proposal_bytes)?,
    };
    let next_effect = reader.number()?;
    let count = reader.count(domain_limits.max_sagas)?;
    let mut sagas = BTreeMap::new();
    let mut previous = None;
    for _ in 0..count {
        let workflow = WorkflowId::new(reader.id()?);
        if previous.is_some_and(|value| value >= workflow) {
            return Err(CodecErrorV1::Invalid);
        }
        previous = Some(workflow);
        let daimon_effect = EffectId::new(reader.id()?);
        let selection_digest = reader.digest()?;
        let phase = decode_phase(&mut reader)?;
        let effect_count = reader.count(domain_limits.max_proposals_per_saga)?;
        let mut effects = Vec::new();
        effects
            .try_reserve(effect_count)
            .map_err(|_| CodecErrorV1::Capacity)?;
        for _ in 0..effect_count {
            effects.push(ProposalEffectV1 {
                effect: EffectId::new(reader.id()?),
                proposal: decode_proposal(&mut reader, domain_limits.max_proposal_bytes)?,
                state: decode_state(&mut reader)?,
            });
        }
        sagas.insert(
            workflow,
            SagaSnapshotV1 {
                workflow,
                daimon_effect,
                selection_digest,
                phase,
                effects,
            },
        );
    }
    reader.finish()?;
    let checkpoint = CheckpointV1 {
        limits: domain_limits,
        next_effect,
        sagas,
    };
    Engine::recover(checkpoint.clone()).map_err(|_| CodecErrorV1::Invalid)?;
    Ok(checkpoint)
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

fn encode_proposal(
    writer: &mut Writer,
    proposal: &ProposalV1,
    maximum: usize,
) -> Result<(), CodecErrorV1> {
    match proposal {
        ProposalV1::AgoraMessage {
            room,
            reply_to,
            body,
        } => {
            writer.byte(1)?;
            writer.id(*room)?;
            writer.id(*reply_to)?;
            writer.bytes(body, maximum)
        }
        ProposalV1::VellumEdit {
            workspace,
            document,
            expected_revision,
            expected_extent,
            replacement,
        } => {
            writer.byte(2)?;
            writer.id(*workspace)?;
            writer.id(*document)?;
            writer.number(*expected_revision)?;
            writer.number(*expected_extent)?;
            writer.bytes(replacement, maximum)
        }
    }
}

fn decode_proposal(reader: &mut Reader<'_>, maximum: usize) -> Result<ProposalV1, CodecErrorV1> {
    Ok(match reader.byte()? {
        1 => ProposalV1::AgoraMessage {
            room: reader.id()?,
            reply_to: reader.id()?,
            body: reader.bytes(maximum)?,
        },
        2 => ProposalV1::VellumEdit {
            workspace: reader.id()?,
            document: reader.id()?,
            expected_revision: reader.number()?,
            expected_extent: reader.number()?,
            replacement: reader.bytes(maximum)?,
        },
        _ => return Err(CodecErrorV1::Invalid),
    })
}

fn encode_state(writer: &mut Writer, state: EffectStateV1) -> Result<(), CodecErrorV1> {
    match state {
        EffectStateV1::Queued => writer.byte(0),
        EffectStateV1::Pending => writer.byte(1),
        EffectStateV1::Succeeded(receipt) => {
            writer.byte(2)?;
            encode_receipt(writer, receipt)
        }
        EffectStateV1::Failed => writer.byte(3),
        EffectStateV1::Uncertain => writer.byte(4),
    }
}

fn decode_state(reader: &mut Reader<'_>) -> Result<EffectStateV1, CodecErrorV1> {
    Ok(match reader.byte()? {
        0 => EffectStateV1::Queued,
        1 => EffectStateV1::Pending,
        2 => EffectStateV1::Succeeded(decode_receipt(reader)?),
        3 => EffectStateV1::Failed,
        4 => EffectStateV1::Uncertain,
        _ => return Err(CodecErrorV1::Invalid),
    })
}

fn encode_receipt(writer: &mut Writer, receipt: PeerReceiptV1) -> Result<(), CodecErrorV1> {
    match receipt {
        PeerReceiptV1::AgoraMessage {
            effect,
            room,
            reply_to,
            message,
            agora_revision,
        } => {
            writer.byte(1)?;
            writer.id(effect.into_bytes())?;
            writer.id(room)?;
            writer.id(reply_to)?;
            writer.id(message)?;
            writer.number(agora_revision)
        }
        PeerReceiptV1::VellumEdit {
            effect,
            workspace,
            document,
            source_revision,
            durable,
        } => {
            writer.byte(2)?;
            writer.id(effect.into_bytes())?;
            writer.id(workspace)?;
            writer.id(document)?;
            writer.number(source_revision)?;
            writer.byte(u8::from(durable))
        }
    }
}

fn decode_receipt(reader: &mut Reader<'_>) -> Result<PeerReceiptV1, CodecErrorV1> {
    Ok(match reader.byte()? {
        1 => PeerReceiptV1::AgoraMessage {
            effect: EffectId::new(reader.id()?),
            room: reader.id()?,
            reply_to: reader.id()?,
            message: reader.id()?,
            agora_revision: reader.number()?,
        },
        2 => PeerReceiptV1::VellumEdit {
            effect: EffectId::new(reader.id()?),
            workspace: reader.id()?,
            document: reader.id()?,
            source_revision: reader.number()?,
            durable: match reader.byte()? {
                0 => false,
                1 => true,
                _ => return Err(CodecErrorV1::Invalid),
            },
        },
        _ => return Err(CodecErrorV1::Invalid),
    })
}

fn to_u64(value: usize) -> Result<u64, CodecErrorV1> {
    u64::try_from(value).map_err(|_| CodecErrorV1::Limit)
}

fn validate_limits(limits: CodecLimitsV1) -> Result<(), CodecErrorV1> {
    if limits.max_bytes < MAGIC.len() + 1
        || limits.max_sagas == 0
        || limits.max_proposals_per_saga == 0
        || limits.max_proposal_bytes == 0
        || limits.max_proposal_bytes > limits.max_bytes
    {
        Err(CodecErrorV1::InvalidLimits)
    } else {
        Ok(())
    }
}
