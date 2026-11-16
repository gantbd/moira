//! Canonical bounded checkpoint codec for interactive-agent participation.

use super::*;

const MAGIC: &[u8; 8] = b"MOIRAAP1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecLimitsV1 {
    pub max_bytes: usize,
    pub domain: LimitsV1,
}

impl Default for CodecLimitsV1 {
    fn default() -> Self {
        Self {
            max_bytes: 8 * 1_024 * 1_024,
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
    fn u32(&mut self, value: u32) -> Result<(), CodecErrorV1> {
        self.raw(&value.to_be_bytes())
    }
    fn u64(&mut self, value: u64) -> Result<(), CodecErrorV1> {
        self.raw(&value.to_be_bytes())
    }
    fn id(&mut self, value: [u8; 16]) -> Result<(), CodecErrorV1> {
        self.raw(&value)
    }
    fn digest(&mut self, value: [u8; 32]) -> Result<(), CodecErrorV1> {
        self.raw(&value)
    }
    fn count(&mut self, value: usize) -> Result<(), CodecErrorV1> {
        self.u64(u64::try_from(value).map_err(|_| CodecErrorV1::Limit)?)
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
    fn u32(&mut self) -> Result<u32, CodecErrorV1> {
        Ok(u32::from_be_bytes(self.raw()?))
    }
    fn u64(&mut self) -> Result<u64, CodecErrorV1> {
        Ok(u64::from_be_bytes(self.raw()?))
    }
    fn id(&mut self) -> Result<[u8; 16], CodecErrorV1> {
        self.raw()
    }
    fn digest(&mut self) -> Result<[u8; 32], CodecErrorV1> {
        self.raw()
    }
    fn count(&mut self, maximum: usize) -> Result<usize, CodecErrorV1> {
        let value = usize::try_from(self.u64()?).map_err(|_| CodecErrorV1::Limit)?;
        if value > maximum {
            Err(CodecErrorV1::Limit)
        } else {
            Ok(value)
        }
    }
    fn option<T>(
        &mut self,
        decode: impl FnOnce(&mut Self) -> Result<T, CodecErrorV1>,
    ) -> Result<Option<T>, CodecErrorV1> {
        match self.byte()? {
            0 => Ok(None),
            1 => decode(self).map(Some),
            _ => Err(CodecErrorV1::Invalid),
        }
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
    let mut w = Writer::new(limits.max_bytes);
    w.raw(MAGIC)?;
    w.byte(1)?;
    w.count(checkpoint.limits.max_participations)?;
    w.count(checkpoint.participations.len())?;
    for value in &checkpoint.participations {
        snapshot(&mut w, value)?;
    }
    Ok(w.bytes)
}

pub fn decode(bytes: &[u8], limits: CodecLimitsV1) -> Result<CheckpointV1, CodecErrorV1> {
    validate_limits(limits)?;
    let mut r = Reader::new(bytes, limits.max_bytes)?;
    if r.raw::<8>()? != *MAGIC || r.byte()? != 1 {
        return Err(CodecErrorV1::Invalid);
    }
    let domain = LimitsV1 {
        max_participations: nonzero_count(&mut r, limits.domain.max_participations)?,
    };
    let length = r.count(domain.max_participations)?;
    let mut participations = Vec::new();
    participations
        .try_reserve_exact(length)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..length {
        participations.push(read_snapshot(&mut r)?);
    }
    r.finish()?;
    let checkpoint = CheckpointV1 {
        version: 1,
        limits: domain,
        participations,
    };
    Engine::recover(checkpoint.clone()).map_err(|_| CodecErrorV1::Invalid)?;
    if encode(&checkpoint, limits)? != bytes {
        return Err(CodecErrorV1::NonCanonical);
    }
    Ok(checkpoint)
}

fn snapshot(w: &mut Writer, value: &SnapshotV1) -> Result<(), CodecErrorV1> {
    binding(w, value.binding)?;
    w.u64(value.revision)?;
    w.byte(phase_tag(value.phase))?;
    w.option(value.session_revision, Writer::u64)?;
    w.option(value.last_event_cursor, Writer::u64)?;
    w.option(value.profile_document_digest, Writer::digest)?;
    capabilities(w, value.capabilities)?;
    w.option(value.active_turn, |w, v| w.id(v.0))?;
    w.option(value.permission, permission)?;
    w.option(value.terminal, terminal)?;
    w.option(value.pending, operation)?;
    w.option(value.last_operation, operation)?;
    w.option(value.last_observation, observation)
}
fn read_snapshot(r: &mut Reader<'_>) -> Result<SnapshotV1, CodecErrorV1> {
    Ok(SnapshotV1 {
        binding: read_binding(r)?,
        revision: r.u64()?,
        phase: read_phase(r.byte()?)?,
        session_revision: r.option(Reader::u64)?,
        last_event_cursor: r.option(Reader::u64)?,
        profile_document_digest: r.option(Reader::digest)?,
        capabilities: read_capabilities(r)?,
        active_turn: r.option(|r| r.id().map(AgentTurnIdV1))?,
        permission: r.option(read_permission)?,
        terminal: r.option(read_terminal)?,
        pending: r.option(read_operation)?,
        last_operation: r.option(read_operation)?,
        last_observation: r.option(read_observation)?,
    })
}

fn binding(w: &mut Writer, value: BindingV1) -> Result<(), CodecErrorV1> {
    w.id(value.participation.0)?;
    w.id(value.workflow.into_bytes())?;
    w.id(value.deliberation.0)?;
    w.id(value.member.0)?;
    w.id(value.session.0)?;
    w.id(value.profile.0)?;
    attempt(w, value.attempt)?;
    w.digest(value.method.machine_digest)?;
    w.digest(value.method.state_digest)?;
    w.digest(value.grounding.authority_digest)?;
    w.digest(value.grounding.surface_digest)?;
    w.byte(depth_tag(value.grounding.depth))?;
    w.byte(persona_tag(value.persona))
}
fn read_binding(r: &mut Reader<'_>) -> Result<BindingV1, CodecErrorV1> {
    Ok(BindingV1 {
        participation: ParticipationIdV1(r.id()?),
        workflow: WorkflowId::new(r.id()?),
        deliberation: deliberation::DeliberationIdV1(r.id()?),
        member: deliberation::EnsembleMemberIdV1(r.id()?),
        session: AgentSessionIdV1(r.id()?),
        profile: ProfileIdV1(r.id()?),
        attempt: read_attempt(r)?,
        method: deliberation::MethodRefV1 {
            machine_digest: r.digest()?,
            state_digest: r.digest()?,
        },
        grounding: deliberation::GroundingRefV1 {
            authority_digest: r.digest()?,
            surface_digest: r.digest()?,
            depth: read_depth(r.byte()?)?,
        },
        persona: read_persona(r.byte()?)?,
    })
}
fn attempt(w: &mut Writer, value: AttemptV1) -> Result<(), CodecErrorV1> {
    w.id(value.task_key)?;
    w.u64(value.incarnation)?;
    w.u32(value.attempt)
}
fn read_attempt(r: &mut Reader<'_>) -> Result<AttemptV1, CodecErrorV1> {
    Ok(AttemptV1 {
        task_key: r.id()?,
        incarnation: r.u64()?,
        attempt: r.u32()?,
    })
}

fn operation(w: &mut Writer, value: OperationV1) -> Result<(), CodecErrorV1> {
    match value {
        OperationV1::Open { key, execution } => {
            w.byte(0)?;
            w.id(key.0)?;
            w.id(execution.0)
        }
        OperationV1::Prompt {
            key,
            turn,
            execution,
            prompt_digest,
            attempt: value,
        } => {
            w.byte(1)?;
            w.id(key.0)?;
            w.id(turn.0)?;
            w.id(execution.0)?;
            w.digest(prompt_digest)?;
            attempt(w, value)
        }
        OperationV1::Advance { key, turn } => {
            w.byte(2)?;
            w.id(key.0)?;
            w.id(turn.0)
        }
        OperationV1::DecidePermission {
            key,
            execution,
            request,
            option,
            authority,
            attempt: value,
        } => {
            w.byte(3)?;
            w.id(key.0)?;
            w.id(execution.0)?;
            w.id(request.0)?;
            w.id(option.0)?;
            permission_authority(w, authority)?;
            attempt(w, value)
        }
        OperationV1::Cancel { key, turn } => {
            w.byte(4)?;
            w.id(key.0)?;
            w.id(turn.0)
        }
        OperationV1::Close { key } => {
            w.byte(5)?;
            w.id(key.0)
        }
    }
}
fn read_operation(r: &mut Reader<'_>) -> Result<OperationV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => OperationV1::Open {
            key: IdempotencyKeyV1(r.id()?),
            execution: ExecutionIdV1(r.id()?),
        },
        1 => OperationV1::Prompt {
            key: IdempotencyKeyV1(r.id()?),
            turn: AgentTurnIdV1(r.id()?),
            execution: ExecutionIdV1(r.id()?),
            prompt_digest: r.digest()?,
            attempt: read_attempt(r)?,
        },
        2 => OperationV1::Advance {
            key: IdempotencyKeyV1(r.id()?),
            turn: AgentTurnIdV1(r.id()?),
        },
        3 => OperationV1::DecidePermission {
            key: IdempotencyKeyV1(r.id()?),
            execution: ExecutionIdV1(r.id()?),
            request: PermissionRequestIdV1(r.id()?),
            option: PermissionOptionIdV1(r.id()?),
            authority: read_permission_authority(r)?,
            attempt: read_attempt(r)?,
        },
        4 => OperationV1::Cancel {
            key: IdempotencyKeyV1(r.id()?),
            turn: AgentTurnIdV1(r.id()?),
        },
        5 => OperationV1::Close {
            key: IdempotencyKeyV1(r.id()?),
        },
        _ => return Err(CodecErrorV1::Invalid),
    })
}

fn observation(w: &mut Writer, value: ObservationV1) -> Result<(), CodecErrorV1> {
    w.byte(phase_tag(value.phase))?;
    w.u64(value.session_revision)?;
    w.option(value.active_turn, |w, v| w.id(v.0))?;
    w.option(value.last_event_cursor, Writer::u64)?;
    w.digest(value.profile_document_digest)?;
    capabilities(w, value.capabilities)?;
    w.option(value.permission, permission)?;
    w.option(value.terminal, terminal)
}
fn read_observation(r: &mut Reader<'_>) -> Result<ObservationV1, CodecErrorV1> {
    Ok(ObservationV1 {
        phase: read_phase(r.byte()?)?,
        session_revision: r.u64()?,
        active_turn: r.option(|r| r.id().map(AgentTurnIdV1))?,
        last_event_cursor: r.option(Reader::u64)?,
        profile_document_digest: r.digest()?,
        capabilities: read_capabilities(r)?,
        permission: r.option(read_permission)?,
        terminal: r.option(read_terminal)?,
    })
}
fn permission(w: &mut Writer, value: PermissionEvidenceV1) -> Result<(), CodecErrorV1> {
    w.id(value.request.0)?;
    w.id(value.turn.0)?;
    w.digest(value.capability_digest)?;
    w.digest(value.options_digest)?;
    w.byte(value.option_count)?;
    for value in &value.option_ids[..usize::from(value.option_count)] {
        w.id(value.0)?;
    }
    Ok(())
}
fn read_permission(r: &mut Reader<'_>) -> Result<PermissionEvidenceV1, CodecErrorV1> {
    let request = PermissionRequestIdV1(r.id()?);
    let turn = AgentTurnIdV1(r.id()?);
    let capability_digest = r.digest()?;
    let options_digest = r.digest()?;
    let option_count = r.byte()?;
    if usize::from(option_count) > MAX_PERMISSION_OPTIONS_V1 {
        return Err(CodecErrorV1::Invalid);
    }
    let mut option_ids = [PermissionOptionIdV1([0; 16]); MAX_PERMISSION_OPTIONS_V1];
    for value in &mut option_ids[..usize::from(option_count)] {
        *value = PermissionOptionIdV1(r.id()?);
    }
    Ok(PermissionEvidenceV1 {
        request,
        turn,
        capability_digest,
        options_digest,
        option_count,
        option_ids,
    })
}
fn permission_authority(w: &mut Writer, value: PermissionAuthorityV1) -> Result<(), CodecErrorV1> {
    w.id(value.decision.0)?;
    w.id(value.subject.0)?;
    w.digest(value.capability_digest)?;
    w.u64(value.epoch)
}
fn read_permission_authority(r: &mut Reader<'_>) -> Result<PermissionAuthorityV1, CodecErrorV1> {
    Ok(PermissionAuthorityV1 {
        decision: AuthorityDecisionIdV1(r.id()?),
        subject: AuthoritySubjectIdV1(r.id()?),
        capability_digest: r.digest()?,
        epoch: r.u64()?,
    })
}
fn terminal(w: &mut Writer, value: TerminalEvidenceV1) -> Result<(), CodecErrorV1> {
    w.id(value.turn.0)?;
    w.byte(disposition_tag(value.disposition))?;
    w.byte(stop_tag(value.stop_reason))?;
    w.digest(value.output_digest)?;
    w.digest(value.provider_evidence_digest)
}
fn read_terminal(r: &mut Reader<'_>) -> Result<TerminalEvidenceV1, CodecErrorV1> {
    Ok(TerminalEvidenceV1 {
        turn: AgentTurnIdV1(r.id()?),
        disposition: read_disposition(r.byte()?)?,
        stop_reason: read_stop(r.byte()?)?,
        output_digest: r.digest()?,
        provider_evidence_digest: r.digest()?,
    })
}
fn capabilities(w: &mut Writer, value: CapabilitiesV1) -> Result<(), CodecErrorV1> {
    let f = (value.load_session as u8)
        | ((value.resume_session as u8) << 1)
        | ((value.permissions as u8) << 2)
        | ((value.filesystem_read as u8) << 3)
        | ((value.filesystem_write as u8) << 4)
        | ((value.terminal as u8) << 5)
        | ((value.session_modes as u8) << 6)
        | ((value.session_models as u8) << 7);
    w.byte(f)
}
fn read_capabilities(r: &mut Reader<'_>) -> Result<CapabilitiesV1, CodecErrorV1> {
    let f = r.byte()?;
    Ok(CapabilitiesV1 {
        load_session: f & 1 != 0,
        resume_session: f & 2 != 0,
        permissions: f & 4 != 0,
        filesystem_read: f & 8 != 0,
        filesystem_write: f & 16 != 0,
        terminal: f & 32 != 0,
        session_modes: f & 64 != 0,
        session_models: f & 128 != 0,
    })
}

macro_rules! tags{($tag:ident,$read:ident,$ty:ty,{$($variant:path=>$n:expr),+$(,)?})=>{fn $tag(value:$ty)->u8{match value{$($variant=>$n),+}}fn $read(value:u8)->Result<$ty,CodecErrorV1>{match value{$($n=>Ok($variant),)+_=>Err(CodecErrorV1::Invalid)}}};}
tags!(phase_tag,read_phase,PhaseV1,{PhaseV1::Admitted=>0,PhaseV1::Ready=>1,PhaseV1::TurnActive=>2,PhaseV1::AwaitingPermission=>3,PhaseV1::Terminal=>4,PhaseV1::FencedUnknown=>5,PhaseV1::Closed=>6});
tags!(disposition_tag,read_disposition,DispositionV1,{DispositionV1::Completed=>0,DispositionV1::Failed=>1,DispositionV1::Cancelled=>2,DispositionV1::TimedOut=>3,DispositionV1::Uncertain=>4});
tags!(stop_tag,read_stop,StopReasonV1,{StopReasonV1::EndTurn=>0,StopReasonV1::Cancelled=>1,StopReasonV1::TimedOut=>2,StopReasonV1::Failed=>3,StopReasonV1::Disconnected=>4,StopReasonV1::OutcomeUnknown=>5});
tags!(depth_tag,read_depth,deliberation::SurfaceDepthV1,{deliberation::SurfaceDepthV1::Ambient=>0,deliberation::SurfaceDepthV1::Activated=>1,deliberation::SurfaceDepthV1::Reference=>2});
tags!(persona_tag,read_persona,deliberation::PersonaLensV1,{deliberation::PersonaLensV1::Historian=>0,deliberation::PersonaLensV1::IdeaStormer=>1,deliberation::PersonaLensV1::Theorist=>2,deliberation::PersonaLensV1::Researcher=>3,deliberation::PersonaLensV1::RigorReviewer=>4,deliberation::PersonaLensV1::ConvergenceFacilitator=>5});

fn validate_limits(value: CodecLimitsV1) -> Result<(), CodecErrorV1> {
    if value.max_bytes == 0 || value.domain.max_participations == 0 {
        Err(CodecErrorV1::InvalidLimits)
    } else {
        Ok(())
    }
}
fn host_bounds(value: LimitsV1, host: LimitsV1) -> Result<(), CodecErrorV1> {
    if value.max_participations == 0 || value.max_participations > host.max_participations {
        Err(CodecErrorV1::Limit)
    } else {
        Ok(())
    }
}
fn nonzero_count(r: &mut Reader<'_>, maximum: usize) -> Result<usize, CodecErrorV1> {
    let value = r.count(maximum)?;
    if value == 0 {
        Err(CodecErrorV1::Limit)
    } else {
        Ok(value)
    }
}
