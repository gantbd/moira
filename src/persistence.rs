//! Bounded, canonical semantic checkpoint data. No storage, clocks or authority.
use super::*;
use keystone_codec::{DecodeError, DecodeLimits, Decoder};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecLimits {
    pub max_bytes: usize,
    pub max_workflows: usize,
}
impl CodecLimits {
    fn fields(self) -> Result<usize, CodecError> {
        if self.max_bytes == 0 || self.max_bytes > isize::MAX as usize || self.max_workflows == 0 {
            return Err(CodecError::Limit);
        }
        self.max_workflows
            .checked_mul(80)
            .and_then(|n| n.checked_add(16))
            .ok_or(CodecError::Limit)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    Limit,
    Capacity,
    Decode(DecodeError),
    Profile,
    InvalidValue,
    NonCanonical,
    Trailing,
    Recovery(RecoveryErrorV1),
}
impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "checkpoint codec: {self:?}")
    }
}
impl std::error::Error for CodecError {}
impl From<DecodeError> for CodecError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

pub struct EncodedCheckpoint(Vec<u8>);
impl EncodedCheckpoint {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl std::fmt::Debug for EncodedCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EncodedCheckpoint(<redacted>)")
    }
}

struct Writer {
    bytes: Vec<u8>,
    max: usize,
}
impl Writer {
    fn reserve(&mut self, count: usize) -> Result<(), CodecError> {
        if self
            .bytes
            .len()
            .checked_add(count)
            .is_none_or(|n| n > self.max)
        {
            return Err(CodecError::Limit);
        }
        self.bytes
            .try_reserve(count)
            .map_err(|_| CodecError::Capacity)
    }
    fn byte(&mut self, value: u8) -> Result<(), CodecError> {
        self.reserve(1)?;
        self.bytes.push(value);
        Ok(())
    }
    fn number(&mut self, value: u64) -> Result<(), CodecError> {
        self.reserve(8)?;
        self.bytes.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }
    fn bytes(&mut self, value: &[u8]) -> Result<(), CodecError> {
        self.reserve(value.len().checked_add(4).ok_or(CodecError::Limit)?)?;
        keystone_codec::encode_bytes(&mut self.bytes, value).map_err(|_| CodecError::Limit)
    }
    fn option<T: Copy>(
        &mut self,
        value: Option<T>,
        write: impl FnOnce(&mut Self, T) -> Result<(), CodecError>,
    ) -> Result<(), CodecError> {
        self.byte(u8::from(value.is_some()))?;
        if let Some(value) = value {
            write(self, value)?;
        }
        Ok(())
    }
    fn message(&mut self, value: AgoraMessageRefV1) -> Result<(), CodecError> {
        self.bytes(&value.source)?;
        self.number(value.room_sequence)?;
        self.number(value.message_sequence)
    }
    fn task(&mut self, value: ChronosTaskRefV1) -> Result<(), CodecError> {
        self.bytes(&value.external_task_key)?;
        self.number(value.incarnation)
    }
    fn attempt(&mut self, value: ChronosAttemptReceiptV1) -> Result<(), CodecError> {
        self.task(value.task)?;
        self.number(value.attempt_number.into())?;
        self.number(value.chronos_revision)
    }
    fn snapshot(&mut self, s: &WorkflowSnapshotV1) -> Result<(), CodecError> {
        self.bytes(&s.workflow_id.0)?;
        self.number(s.revision.0)?;
        self.message(s.cause)?;
        self.bytes(&s.recipient.0)?;
        self.byte(phase_tag(s.phase))?;
        self.bytes(&s.submission_effect.0)?;
        self.option(s.pending_effect, |w, v| w.bytes(&v.0))?;
        self.option(s.chronos_task, Self::task)?;
        self.option(s.chronos_revision, Self::number)?;
        self.option(s.chronos_attempt, Self::attempt)?;
        self.option(s.harness_effect, |w, v| w.bytes(&v.0))?;
        self.option(s.daimon_result, |w, v| {
            w.bytes(&v.effect_id.0)?;
            w.bytes(&v.execution_id)?;
            w.task(v.task)?;
            w.number(v.attempt_number.into())?;
            w.byte(disposition_tag(v.disposition))
        })?;
        self.option(s.publication_effect, |w, v| w.bytes(&v.0))?;
        self.option(s.agora_publication, |w, v| {
            w.bytes(&v.effect_id.0)?;
            w.message(v.message)?;
            w.number(v.agora_revision)
        })
    }
}

struct Reader<'a>(Decoder<'a>);
impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, CodecError> {
        Ok(self.0.read_u8()?)
    }
    fn number(&mut self) -> Result<u64, CodecError> {
        Ok(self.0.read_u64_be()?)
    }
    fn id(&mut self) -> Result<[u8; 16], CodecError> {
        self.0
            .read_bytes()?
            .try_into()
            .map_err(|_| CodecError::InvalidValue)
    }
    fn option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, CodecError>,
    ) -> Result<Option<T>, CodecError> {
        match self.byte()? {
            0 => Ok(None),
            1 => read(self).map(Some),
            _ => Err(CodecError::InvalidValue),
        }
    }
    fn message(&mut self) -> Result<AgoraMessageRefV1, CodecError> {
        Ok(AgoraMessageRefV1::new(
            self.id()?,
            self.number()?,
            self.number()?,
        ))
    }
    fn task(&mut self) -> Result<ChronosTaskRefV1, CodecError> {
        Ok(ChronosTaskRefV1::new(self.id()?, self.number()?))
    }
    fn attempt(&mut self) -> Result<ChronosAttemptReceiptV1, CodecError> {
        Ok(ChronosAttemptReceiptV1::new(
            self.task()?,
            self.number()?
                .try_into()
                .map_err(|_| CodecError::InvalidValue)?,
            self.number()?,
        ))
    }
    fn snapshot(&mut self) -> Result<WorkflowSnapshotV1, CodecError> {
        Ok(WorkflowSnapshotV1 {
            workflow_id: WorkflowId::new(self.id()?),
            revision: WorkflowRevision(self.number()?),
            cause: self.message()?,
            recipient: ParticipantRefV1::new(self.id()?),
            phase: read_phase(self.byte()?)?,
            submission_effect: EffectId::new(self.id()?),
            pending_effect: self.option(|r| Ok(EffectId::new(r.id()?)))?,
            chronos_task: self.option(Self::task)?,
            chronos_revision: self.option(Self::number)?,
            chronos_attempt: self.option(Self::attempt)?,
            harness_effect: self.option(|r| Ok(EffectId::new(r.id()?)))?,
            daimon_result: self.option(|r| {
                Ok(DaimonResultReceiptV1::new(
                    EffectId::new(r.id()?),
                    r.id()?,
                    r.task()?,
                    r.number()?
                        .try_into()
                        .map_err(|_| CodecError::InvalidValue)?,
                    read_disposition(r.byte()?)?,
                ))
            })?,
            publication_effect: self.option(|r| Ok(EffectId::new(r.id()?)))?,
            agora_publication: self.option(|r| {
                Ok(AgoraPublicationReceiptV1::new(
                    EffectId::new(r.id()?),
                    r.message()?,
                    r.number()?,
                ))
            })?,
        })
    }
}

pub fn encode_checkpoint(
    checkpoint: &CheckpointV1,
    limits: CodecLimits,
) -> Result<EncodedCheckpoint, CodecError> {
    limits.fields()?;
    if checkpoint.limits.max_workflows > limits.max_workflows {
        return Err(CodecError::Limit);
    }
    Engine::recover(checkpoint.clone()).map_err(CodecError::Recovery)?;
    let mut w = Writer {
        bytes: Vec::new(),
        max: limits.max_bytes,
    };
    w.bytes(b"MOIRACP")?;
    w.byte(1)?;
    w.number(
        checkpoint
            .limits
            .max_workflows
            .try_into()
            .map_err(|_| CodecError::Limit)?,
    )?;
    w.number(checkpoint.revision.0)?;
    w.number(checkpoint.next_workflow)?;
    w.number(checkpoint.next_effect)?;
    w.number(
        checkpoint
            .workflows
            .len()
            .try_into()
            .map_err(|_| CodecError::Limit)?,
    )?;
    for snapshot in checkpoint.workflows.values() {
        w.snapshot(snapshot)?;
    }
    Ok(EncodedCheckpoint(w.bytes))
}

pub fn decode_checkpoint(bytes: &[u8], limits: CodecLimits) -> Result<CheckpointV1, CodecError> {
    let mut r = Reader(Decoder::new(
        bytes,
        DecodeLimits {
            max_input_bytes: limits.max_bytes,
            max_field_bytes: 16,
            max_fields: limits.fields()?,
        },
    )?);
    if r.0.read_bytes()? != b"MOIRACP" || r.byte()? != 1 {
        return Err(CodecError::Profile);
    }
    let max_workflows: usize = r.number()?.try_into().map_err(|_| CodecError::Limit)?;
    if max_workflows == 0 || max_workflows > limits.max_workflows {
        return Err(CodecError::Limit);
    }
    let revision = WorkflowRevision(r.number()?);
    let next_workflow = r.number()?;
    let next_effect = r.number()?;
    let count: usize = r.number()?.try_into().map_err(|_| CodecError::Limit)?;
    if count > max_workflows {
        return Err(CodecError::Limit);
    }
    let mut workflows = BTreeMap::new();
    let mut admitted_messages = BTreeMap::new();
    let mut previous = None;
    for _ in 0..count {
        let snapshot = r.snapshot()?;
        if previous.is_some_and(|id| id >= snapshot.workflow_id) {
            return Err(CodecError::NonCanonical);
        }
        previous = Some(snapshot.workflow_id);
        if admitted_messages
            .insert(snapshot.cause, snapshot.workflow_id)
            .is_some()
        {
            return Err(CodecError::InvalidValue);
        }
        workflows.insert(snapshot.workflow_id, snapshot);
    }
    if !r.0.is_finished() {
        return Err(CodecError::Trailing);
    }
    let checkpoint = CheckpointV1 {
        limits: Limits { max_workflows },
        revision,
        next_workflow,
        next_effect,
        workflows,
        admitted_messages,
    };
    Engine::recover(checkpoint.clone()).map_err(CodecError::Recovery)?;
    Ok(checkpoint)
}

fn phase_tag(value: WorkflowPhaseV1) -> u8 {
    match value {
        WorkflowPhaseV1::Received => 0,
        WorkflowPhaseV1::TaskSubmissionPending => 1,
        WorkflowPhaseV1::TaskSubmitted => 2,
        WorkflowPhaseV1::HarnessPending => 3,
        WorkflowPhaseV1::HarnessFinished => 4,
        WorkflowPhaseV1::ResponsePublicationPending => 5,
        WorkflowPhaseV1::FencedUncertain => 6,
        WorkflowPhaseV1::Completed => 7,
        WorkflowPhaseV1::Failed => 8,
        WorkflowPhaseV1::Cancelled => 9,
    }
}
fn read_phase(value: u8) -> Result<WorkflowPhaseV1, CodecError> {
    Ok(match value {
        0 => WorkflowPhaseV1::Received,
        1 => WorkflowPhaseV1::TaskSubmissionPending,
        2 => WorkflowPhaseV1::TaskSubmitted,
        3 => WorkflowPhaseV1::HarnessPending,
        4 => WorkflowPhaseV1::HarnessFinished,
        5 => WorkflowPhaseV1::ResponsePublicationPending,
        6 => WorkflowPhaseV1::FencedUncertain,
        7 => WorkflowPhaseV1::Completed,
        8 => WorkflowPhaseV1::Failed,
        9 => WorkflowPhaseV1::Cancelled,
        _ => return Err(CodecError::InvalidValue),
    })
}
fn disposition_tag(value: DaimonDispositionV1) -> u8 {
    match value {
        DaimonDispositionV1::Completed => 0,
        DaimonDispositionV1::Failed => 1,
        DaimonDispositionV1::Cancelled => 2,
        DaimonDispositionV1::TimedOut => 3,
        DaimonDispositionV1::Uncertain => 4,
    }
}
fn read_disposition(value: u8) -> Result<DaimonDispositionV1, CodecError> {
    Ok(match value {
        0 => DaimonDispositionV1::Completed,
        1 => DaimonDispositionV1::Failed,
        2 => DaimonDispositionV1::Cancelled,
        3 => DaimonDispositionV1::TimedOut,
        4 => DaimonDispositionV1::Uncertain,
        _ => return Err(CodecError::InvalidValue),
    })
}
