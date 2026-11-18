//! Canonical bounded V1 codec for Moira requests and replies.

use moira_protocol::*;

mod attempt_lookup;
pub use attempt_lookup::{
    decode_attempt_reply, decode_attempt_request, encode_attempt_reply, encode_attempt_request,
};
mod agent;
pub use agent::{
    decode_agent_reply, decode_agent_request, encode_agent_reply, encode_agent_request,
};
mod agent_participation;
pub use agent_participation::{
    decode_agent_participation_reply, decode_agent_participation_request,
    encode_agent_participation_reply, encode_agent_participation_request,
};
mod deliberation;
pub use deliberation::{
    DELIBERATION_PROFILE_VERSION, decode_deliberation_reply, decode_deliberation_request,
    encode_deliberation_reply, encode_deliberation_request,
};
mod saga;
pub use saga::{decode_saga_reply, decode_saga_request, encode_saga_reply, encode_saga_request};

const MAGIC: &[u8; 8] = b"MOIRAW01";
const REQUEST: u8 = 1;
const REPLY: u8 = 2;
const READY: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitsV1 {
    pub max_frame_bytes: usize,
}

impl Default for LimitsV1 {
    fn default() -> Self {
        Self {
            max_frame_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecErrorV1 {
    Limit,
    Malformed,
    Unsupported,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadyV1 {
    pub protocol_version: u8,
    pub max_frame_bytes: u32,
    pub max_requests: u32,
}

struct Writer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Writer {
    fn new(kind: u8, limits: LimitsV1) -> Result<Self, CodecErrorV1> {
        let mut value = Self {
            bytes: Vec::new(),
            limit: limits.max_frame_bytes,
        };
        value.raw(MAGIC)?;
        value.byte(kind)?;
        Ok(value)
    }
    fn raw(&mut self, bytes: &[u8]) -> Result<(), CodecErrorV1> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
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
    fn u32(&mut self, value: u32) -> Result<(), CodecErrorV1> {
        self.raw(&value.to_be_bytes())
    }
    fn u64(&mut self, value: u64) -> Result<(), CodecErrorV1> {
        self.raw(&value.to_be_bytes())
    }
    fn usize(&mut self, value: usize) -> Result<(), CodecErrorV1> {
        self.u64(u64::try_from(value).map_err(|_| CodecErrorV1::Limit)?)
    }
    fn bytes(&mut self, value: &[u8]) -> Result<(), CodecErrorV1> {
        self.u32(u32::try_from(value.len()).map_err(|_| CodecErrorV1::Limit)?)?;
        self.raw(value)
    }
    fn option<T>(
        &mut self,
        value: Option<T>,
        encode: impl FnOnce(&mut Self, T) -> Result<(), CodecErrorV1>,
    ) -> Result<(), CodecErrorV1> {
        match value {
            None => self.byte(0),
            Some(value) => {
                self.byte(1)?;
                encode(self, value)
            }
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], kind: u8, limits: LimitsV1) -> Result<Self, CodecErrorV1> {
        if bytes.len() > limits.max_frame_bytes {
            return Err(CodecErrorV1::Limit);
        }
        if bytes.get(..8) != Some(MAGIC.as_slice()) || bytes.get(8) != Some(&kind) {
            return Err(CodecErrorV1::Unsupported);
        }
        Ok(Self { bytes, position: 9 })
    }
    fn raw<const N: usize>(&mut self) -> Result<[u8; N], CodecErrorV1> {
        let end = self
            .position
            .checked_add(N)
            .ok_or(CodecErrorV1::Malformed)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(CodecErrorV1::Malformed)?;
        self.position = end;
        value.try_into().map_err(|_| CodecErrorV1::Malformed)
    }
    fn byte(&mut self) -> Result<u8, CodecErrorV1> {
        Ok(self.raw::<1>()?[0])
    }
    fn u32(&mut self) -> Result<u32, CodecErrorV1> {
        Ok(u32::from_be_bytes(self.raw()?))
    }
    fn u64(&mut self) -> Result<u64, CodecErrorV1> {
        Ok(u64::from_be_bytes(self.raw()?))
    }
    fn usize(&mut self) -> Result<usize, CodecErrorV1> {
        usize::try_from(self.u64()?).map_err(|_| CodecErrorV1::Limit)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, CodecErrorV1> {
        let length = usize::try_from(self.u32()?).map_err(|_| CodecErrorV1::Limit)?;
        let end = self
            .position
            .checked_add(length)
            .ok_or(CodecErrorV1::Malformed)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(CodecErrorV1::Malformed)?;
        self.position = end;
        let mut copy = Vec::new();
        copy.try_reserve_exact(length)
            .map_err(|_| CodecErrorV1::Capacity)?;
        copy.extend_from_slice(value);
        Ok(copy)
    }
    fn option<T>(
        &mut self,
        decode: impl FnOnce(&mut Self) -> Result<T, CodecErrorV1>,
    ) -> Result<Option<T>, CodecErrorV1> {
        match self.byte()? {
            0 => Ok(None),
            1 => decode(self).map(Some),
            _ => Err(CodecErrorV1::Malformed),
        }
    }
    fn finish(self) -> Result<(), CodecErrorV1> {
        (self.position == self.bytes.len())
            .then_some(())
            .ok_or(CodecErrorV1::Malformed)
    }
}

pub fn encode_request(value: &CommandV1, limits: LimitsV1) -> Result<Vec<u8>, CodecErrorV1> {
    let mut writer = Writer::new(REQUEST, limits)?;
    command(&mut writer, value)?;
    Ok(writer.bytes)
}

pub fn decode_request(bytes: &[u8], limits: LimitsV1) -> Result<CommandV1, CodecErrorV1> {
    let mut reader = Reader::new(bytes, REQUEST, limits)?;
    let value = read_command(&mut reader)?;
    reader.finish()?;
    Ok(value)
}

pub fn encode_reply(value: &ReplyV1, limits: LimitsV1) -> Result<Vec<u8>, CodecErrorV1> {
    let mut writer = Writer::new(REPLY, limits)?;
    match value {
        ReplyV1::Accepted(value) => {
            writer.byte(0)?;
            outcome(&mut writer, value)?;
        }
        ReplyV1::Refused(value) => {
            writer.byte(1)?;
            refusal(&mut writer, *value)?;
        }
    }
    Ok(writer.bytes)
}

pub fn decode_reply(bytes: &[u8], limits: LimitsV1) -> Result<ReplyV1, CodecErrorV1> {
    let mut reader = Reader::new(bytes, REPLY, limits)?;
    let value = match reader.byte()? {
        0 => ReplyV1::Accepted(read_outcome(&mut reader)?),
        1 => ReplyV1::Refused(read_refusal(&mut reader)?),
        _ => return Err(CodecErrorV1::Unsupported),
    };
    reader.finish()?;
    Ok(value)
}

pub fn encode_ready(value: ReadyV1, limits: LimitsV1) -> Result<Vec<u8>, CodecErrorV1> {
    if value.protocol_version != 1 || value.max_frame_bytes == 0 || value.max_requests == 0 {
        return Err(CodecErrorV1::Malformed);
    }
    let mut writer = Writer::new(READY, limits)?;
    writer.byte(value.protocol_version)?;
    writer.u32(value.max_frame_bytes)?;
    writer.u32(value.max_requests)?;
    Ok(writer.bytes)
}

pub fn decode_ready(bytes: &[u8], limits: LimitsV1) -> Result<ReadyV1, CodecErrorV1> {
    let mut reader = Reader::new(bytes, READY, limits)?;
    let value = ReadyV1 {
        protocol_version: reader.byte()?,
        max_frame_bytes: reader.u32()?,
        max_requests: reader.u32()?,
    };
    if value.protocol_version != 1 || value.max_frame_bytes == 0 || value.max_requests == 0 {
        return Err(CodecErrorV1::Malformed);
    }
    reader.finish()?;
    Ok(value)
}

fn command(w: &mut Writer, value: &CommandV1) -> Result<(), CodecErrorV1> {
    match value {
        CommandV1::Apply {
            expected_revision,
            mutation: value,
        } => {
            w.byte(0)?;
            w.u64(*expected_revision)?;
            mutation(w, value)
        }
        CommandV1::Status => w.byte(1),
        CommandV1::Workflow { workflow } => {
            w.byte(2)?;
            w.raw(&workflow.0)
        }
    }
}

fn read_command(r: &mut Reader<'_>) -> Result<CommandV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(CommandV1::Apply {
            expected_revision: r.u64()?,
            mutation: read_mutation(r)?,
        }),
        1 => Ok(CommandV1::Status),
        2 => Ok(CommandV1::Workflow {
            workflow: WorkflowIdV1(r.raw()?),
        }),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn mutation(w: &mut Writer, value: &MutationV1) -> Result<(), CodecErrorV1> {
    match value {
        MutationV1::AdmitMessage {
            message: value,
            recipient,
        } => {
            w.byte(0)?;
            message(w, *value)?;
            w.raw(&recipient.0)
        }
        MutationV1::AcceptChronosAdmission { workflow, receipt } => {
            w.byte(1)?;
            w.raw(&workflow.0)?;
            w.raw(&receipt.effect.0)?;
            task(w, receipt.task)?;
            w.u64(receipt.chronos_revision)
        }
        MutationV1::AcceptChronosAttempt { workflow, receipt } => {
            w.byte(2)?;
            w.raw(&workflow.0)?;
            attempt(w, *receipt)
        }
        MutationV1::AcceptDaimonResult { workflow, receipt } => {
            w.byte(3)?;
            w.raw(&workflow.0)?;
            w.raw(&receipt.effect.0)?;
            w.raw(&receipt.execution_id)?;
            task(w, receipt.task)?;
            w.u32(receipt.attempt_number)?;
            w.byte(disposition_tag(receipt.disposition))
        }
        MutationV1::PrepareAgoraResponse { workflow } => {
            w.byte(4)?;
            w.raw(&workflow.0)
        }
        MutationV1::AcceptAgoraPublication { workflow, receipt } => {
            w.byte(5)?;
            w.raw(&workflow.0)?;
            w.raw(&receipt.effect.0)?;
            message(w, receipt.message)?;
            w.u64(receipt.agora_revision)
        }
    }
}

fn read_mutation(r: &mut Reader<'_>) -> Result<MutationV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(MutationV1::AdmitMessage {
            message: read_message(r)?,
            recipient: ParticipantRefV1(r.raw()?),
        }),
        1 => Ok(MutationV1::AcceptChronosAdmission {
            workflow: WorkflowIdV1(r.raw()?),
            receipt: AdmissionReceiptV1 {
                effect: EffectIdV1(r.raw()?),
                task: read_task(r)?,
                chronos_revision: r.u64()?,
            },
        }),
        2 => Ok(MutationV1::AcceptChronosAttempt {
            workflow: WorkflowIdV1(r.raw()?),
            receipt: read_attempt(r)?,
        }),
        3 => Ok(MutationV1::AcceptDaimonResult {
            workflow: WorkflowIdV1(r.raw()?),
            receipt: ResultReceiptV1 {
                effect: EffectIdV1(r.raw()?),
                execution_id: r.raw()?,
                task: read_task(r)?,
                attempt_number: r.u32()?,
                disposition: read_disposition(r.byte()?)?,
            },
        }),
        4 => Ok(MutationV1::PrepareAgoraResponse {
            workflow: WorkflowIdV1(r.raw()?),
        }),
        5 => Ok(MutationV1::AcceptAgoraPublication {
            workflow: WorkflowIdV1(r.raw()?),
            receipt: PublicationReceiptV1 {
                effect: EffectIdV1(r.raw()?),
                message: read_message(r)?,
                agora_revision: r.u64()?,
            },
        }),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn outcome(w: &mut Writer, value: &OutcomeV1) -> Result<(), CodecErrorV1> {
    match value {
        OutcomeV1::Applied {
            storage_revision,
            transition: value,
            replayed,
        } => {
            w.byte(0)?;
            w.u64(*storage_revision)?;
            transition(w, *value)?;
            w.byte(u8::from(*replayed))
        }
        OutcomeV1::Status(value) => {
            w.byte(1)?;
            w.u64(value.revision)?;
            w.byte(u8::from(value.needs_reopen))
        }
        OutcomeV1::Workflow(value) => {
            w.byte(2)?;
            w.option(*value, workflow)
        }
    }
}

fn read_outcome(r: &mut Reader<'_>) -> Result<OutcomeV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(OutcomeV1::Applied {
            storage_revision: r.u64()?,
            transition: read_transition(r)?,
            replayed: boolean(r.byte()?)?,
        }),
        1 => Ok(OutcomeV1::Status(StatusV1 {
            revision: r.u64()?,
            needs_reopen: boolean(r.byte()?)?,
        })),
        2 => Ok(OutcomeV1::Workflow(r.option(read_workflow)?)),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn workflow(w: &mut Writer, value: WorkflowViewV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.workflow.0)?;
    w.u64(value.revision)?;
    w.byte(phase_tag(value.phase))?;
    message(w, value.cause)?;
    w.raw(&value.recipient.0)?;
    w.option(value.pending_effect, |w, v| w.raw(&v.0))?;
    w.option(value.task, task)?;
    w.option(value.attempt, attempt)?;
    w.option(value.result_disposition, |w, v| w.byte(disposition_tag(v)))?;
    w.option(value.publication, message)
}

fn read_workflow(r: &mut Reader<'_>) -> Result<WorkflowViewV1, CodecErrorV1> {
    Ok(WorkflowViewV1 {
        workflow: WorkflowIdV1(r.raw()?),
        revision: r.u64()?,
        phase: read_phase(r.byte()?)?,
        cause: read_message(r)?,
        recipient: ParticipantRefV1(r.raw()?),
        pending_effect: r.option(|r| Ok(EffectIdV1(r.raw()?)))?,
        task: r.option(read_task)?,
        attempt: r.option(read_attempt)?,
        result_disposition: r.option(|r| read_disposition(r.byte()?))?,
        publication: r.option(read_message)?,
    })
}

fn transition(w: &mut Writer, value: TransitionV1) -> Result<(), CodecErrorV1> {
    match value {
        TransitionV1::ChronosSubmissionPrepared { workflow, effect } => {
            w.byte(0)?;
            w.raw(&workflow.0)?;
            w.raw(&effect.0)
        }
        TransitionV1::ChronosSubmissionAccepted {
            workflow,
            task: value,
        } => {
            w.byte(1)?;
            w.raw(&workflow.0)?;
            task(w, value)
        }
        TransitionV1::DaimonExecutionPrepared {
            workflow,
            effect,
            task: value,
            attempt_number,
        } => {
            w.byte(2)?;
            w.raw(&workflow.0)?;
            w.raw(&effect.0)?;
            task(w, value)?;
            w.u32(attempt_number)
        }
        TransitionV1::DaimonResultAccepted {
            workflow,
            disposition,
        } => {
            w.byte(3)?;
            w.raw(&workflow.0)?;
            w.byte(disposition_tag(disposition))
        }
        TransitionV1::AgoraResponsePrepared { workflow, effect } => {
            w.byte(4)?;
            w.raw(&workflow.0)?;
            w.raw(&effect.0)
        }
        TransitionV1::AgoraPublicationAccepted {
            workflow,
            message: value,
        } => {
            w.byte(5)?;
            w.raw(&workflow.0)?;
            message(w, value)
        }
    }
}

fn read_transition(r: &mut Reader<'_>) -> Result<TransitionV1, CodecErrorV1> {
    let workflow = |r: &mut Reader<'_>| r.raw().map(WorkflowIdV1);
    match r.byte()? {
        0 => Ok(TransitionV1::ChronosSubmissionPrepared {
            workflow: workflow(r)?,
            effect: EffectIdV1(r.raw()?),
        }),
        1 => Ok(TransitionV1::ChronosSubmissionAccepted {
            workflow: workflow(r)?,
            task: read_task(r)?,
        }),
        2 => Ok(TransitionV1::DaimonExecutionPrepared {
            workflow: workflow(r)?,
            effect: EffectIdV1(r.raw()?),
            task: read_task(r)?,
            attempt_number: r.u32()?,
        }),
        3 => Ok(TransitionV1::DaimonResultAccepted {
            workflow: workflow(r)?,
            disposition: read_disposition(r.byte()?)?,
        }),
        4 => Ok(TransitionV1::AgoraResponsePrepared {
            workflow: workflow(r)?,
            effect: EffectIdV1(r.raw()?),
        }),
        5 => Ok(TransitionV1::AgoraPublicationAccepted {
            workflow: workflow(r)?,
            message: read_message(r)?,
        }),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn refusal(w: &mut Writer, value: RefusalV1) -> Result<(), CodecErrorV1> {
    match value {
        RefusalV1::NotAuthorized => w.byte(0),
        RefusalV1::RevisionConflict { expected, actual } => {
            w.byte(1)?;
            w.u64(expected)?;
            w.u64(actual)
        }
        RefusalV1::RevisionExhausted => w.byte(2),
        RefusalV1::WorkflowIdentityExhausted => w.byte(3),
        RefusalV1::EffectIdentityExhausted => w.byte(4),
        RefusalV1::WorkflowLimit => w.byte(5),
        RefusalV1::InputConflict => w.byte(6),
        RefusalV1::ReceiptConflict => w.byte(7),
        RefusalV1::UnknownWorkflow => w.byte(8),
        RefusalV1::InvalidPhase => w.byte(9),
        RefusalV1::EffectMismatch => w.byte(10),
        RefusalV1::AttemptMismatch => w.byte(11),
    }
}

fn read_refusal(r: &mut Reader<'_>) -> Result<RefusalV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => RefusalV1::NotAuthorized,
        1 => RefusalV1::RevisionConflict {
            expected: r.u64()?,
            actual: r.u64()?,
        },
        2 => RefusalV1::RevisionExhausted,
        3 => RefusalV1::WorkflowIdentityExhausted,
        4 => RefusalV1::EffectIdentityExhausted,
        5 => RefusalV1::WorkflowLimit,
        6 => RefusalV1::InputConflict,
        7 => RefusalV1::ReceiptConflict,
        8 => RefusalV1::UnknownWorkflow,
        9 => RefusalV1::InvalidPhase,
        10 => RefusalV1::EffectMismatch,
        11 => RefusalV1::AttemptMismatch,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn message(w: &mut Writer, value: MessageRefV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.source)?;
    w.u64(value.room_sequence)?;
    w.u64(value.message_sequence)
}
fn read_message(r: &mut Reader<'_>) -> Result<MessageRefV1, CodecErrorV1> {
    Ok(MessageRefV1 {
        source: r.raw()?,
        room_sequence: r.u64()?,
        message_sequence: r.u64()?,
    })
}
fn task(w: &mut Writer, value: TaskRefV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.external_task_key)?;
    w.u64(value.incarnation)
}
fn read_task(r: &mut Reader<'_>) -> Result<TaskRefV1, CodecErrorV1> {
    Ok(TaskRefV1 {
        external_task_key: r.raw()?,
        incarnation: r.u64()?,
    })
}
fn attempt(w: &mut Writer, value: AttemptReceiptV1) -> Result<(), CodecErrorV1> {
    task(w, value.task)?;
    w.u32(value.attempt_number)?;
    w.u64(value.chronos_revision)
}
fn read_attempt(r: &mut Reader<'_>) -> Result<AttemptReceiptV1, CodecErrorV1> {
    Ok(AttemptReceiptV1 {
        task: read_task(r)?,
        attempt_number: r.u32()?,
        chronos_revision: r.u64()?,
    })
}
fn boolean(value: u8) -> Result<bool, CodecErrorV1> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CodecErrorV1::Malformed),
    }
}
fn disposition_tag(value: DispositionV1) -> u8 {
    match value {
        DispositionV1::Completed => 0,
        DispositionV1::Failed => 1,
        DispositionV1::Cancelled => 2,
        DispositionV1::TimedOut => 3,
        DispositionV1::Uncertain => 4,
    }
}
fn read_disposition(value: u8) -> Result<DispositionV1, CodecErrorV1> {
    match value {
        0 => Ok(DispositionV1::Completed),
        1 => Ok(DispositionV1::Failed),
        2 => Ok(DispositionV1::Cancelled),
        3 => Ok(DispositionV1::TimedOut),
        4 => Ok(DispositionV1::Uncertain),
        _ => Err(CodecErrorV1::Unsupported),
    }
}
fn phase_tag(value: PhaseV1) -> u8 {
    match value {
        PhaseV1::Received => 0,
        PhaseV1::TaskSubmissionPending => 1,
        PhaseV1::TaskSubmitted => 2,
        PhaseV1::HarnessPending => 3,
        PhaseV1::HarnessFinished => 4,
        PhaseV1::ResponsePublicationPending => 5,
        PhaseV1::FencedUncertain => 6,
        PhaseV1::Completed => 7,
        PhaseV1::Failed => 8,
        PhaseV1::Cancelled => 9,
    }
}
fn read_phase(value: u8) -> Result<PhaseV1, CodecErrorV1> {
    match value {
        0 => Ok(PhaseV1::Received),
        1 => Ok(PhaseV1::TaskSubmissionPending),
        2 => Ok(PhaseV1::TaskSubmitted),
        3 => Ok(PhaseV1::HarnessPending),
        4 => Ok(PhaseV1::HarnessFinished),
        5 => Ok(PhaseV1::ResponsePublicationPending),
        6 => Ok(PhaseV1::FencedUncertain),
        7 => Ok(PhaseV1::Completed),
        8 => Ok(PhaseV1::Failed),
        9 => Ok(PhaseV1::Cancelled),
        _ => Err(CodecErrorV1::Unsupported),
    }
}
