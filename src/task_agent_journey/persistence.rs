//! Canonical bounded checkpoint bytes for task-agent journey admission.

use super::*;

const MAGIC: &[u8; 8] = b"MOIRATJ1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecLimitsV1 {
    pub max_bytes: usize,
    pub domain: LimitsV1,
}

impl Default for CodecLimitsV1 {
    fn default() -> Self {
        Self {
            max_bytes: 16 * 1_024 * 1_024,
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

    fn text(&mut self, value: &str) -> Result<(), CodecErrorV1> {
        self.number(to_u64(value.len())?)?;
        self.raw(value.as_bytes())
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

    fn text(&mut self, maximum: usize) -> Result<String, CodecErrorV1> {
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
        std::str::from_utf8(value)
            .map(str::to_owned)
            .map_err(|_| CodecErrorV1::Invalid)
    }

    fn count(&mut self, maximum: usize) -> Result<usize, CodecErrorV1> {
        let value = usize::try_from(self.number()?).map_err(|_| CodecErrorV1::Limit)?;
        if value > maximum || value > self.bytes.len().saturating_sub(self.position) {
            Err(CodecErrorV1::Limit)
        } else {
            Ok(value)
        }
    }

    fn bounded_count(&mut self, maximum: usize) -> Result<usize, CodecErrorV1> {
        let value = self.count(maximum)?;
        if value == 0 {
            Err(CodecErrorV1::InvalidLimits)
        } else {
            Ok(value)
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
    encode_version(checkpoint, limits, 7)
}

fn encode_version(
    checkpoint: &CheckpointV1,
    limits: CodecLimitsV1,
    version: u8,
) -> Result<Vec<u8>, CodecErrorV1> {
    validate_limits(limits)?;
    host_bounds(checkpoint.limits, limits.domain)?;
    let recovered = Engine::recover(checkpoint.clone()).map_err(|_| CodecErrorV1::Invalid)?;
    let canonical = recovered.checkpoint();
    if &canonical != checkpoint {
        return Err(CodecErrorV1::NonCanonical);
    }
    let mut writer = Writer::new(limits.max_bytes);
    writer.raw(MAGIC)?;
    writer.byte(version)?;
    writer.id(canonical.owner.0)?;
    domain_limits(&mut writer, canonical.limits)?;
    writer.number(canonical.owner_revision)?;
    writer.number(canonical.next_preview)?;
    writer.number(canonical.next_journey)?;
    writer.number(to_u64(canonical.templates.len())?)?;
    for value in canonical.templates {
        template(&mut writer, value)?;
    }
    writer.number(to_u64(canonical.previews.len())?)?;
    for value in canonical.previews {
        preview(&mut writer, value)?;
    }
    writer.number(to_u64(canonical.journeys.len())?)?;
    for value in canonical.journeys.iter().copied() {
        journey(&mut writer, value, version)?;
    }
    if version >= 4 {
        writer.number(to_u64(canonical.participation_blueprints.len())?)?;
        for value in canonical.participation_blueprints {
            participation_blueprint(&mut writer, value)?;
        }
    } else if !canonical.participation_blueprints.is_empty() {
        return Err(CodecErrorV1::Invalid);
    }
    if version >= 5 {
        writer.number(to_u64(canonical.participation_attributions.len())?)?;
        for value in canonical.participation_attributions {
            participation_attribution(&mut writer, value)?;
        }
    } else if !canonical.participation_attributions.is_empty()
        || canonical
            .journeys
            .iter()
            .any(|journey| journey.participation.is_some())
    {
        return Err(CodecErrorV1::Invalid);
    }
    if version >= 6 {
        writer.number(to_u64(canonical.task_attempt_namespaces.len())?)?;
        for value in canonical.task_attempt_namespaces {
            task_attempt_namespace(&mut writer, &value)?;
        }
    } else if !canonical.task_attempt_namespaces.is_empty() {
        return Err(CodecErrorV1::Invalid);
    }
    if version >= 7 {
        writer.number(to_u64(canonical.actor_participation_relationships.len())?)?;
        for value in canonical.actor_participation_relationships {
            actor_participation_relationship(&mut writer, &value)?;
        }
    } else if !canonical.actor_participation_relationships.is_empty() {
        return Err(CodecErrorV1::Invalid);
    }
    Ok(writer.bytes)
}

pub fn decode(bytes: &[u8], limits: CodecLimitsV1) -> Result<CheckpointV1, CodecErrorV1> {
    validate_limits(limits)?;
    let mut reader = Reader::new(bytes, limits.max_bytes)?;
    if reader.raw::<8>()? != *MAGIC {
        return Err(CodecErrorV1::Invalid);
    }
    let version = reader.byte()?;
    if !matches!(version, 1..=7) {
        return Err(CodecErrorV1::Invalid);
    }
    let owner = OwnerIdV1(reader.id()?);
    let domain = read_domain_limits(&mut reader, limits.domain)?;
    let owner_revision = reader.number()?;
    let next_preview = reader.number()?;
    let next_journey = reader.number()?;
    let template_count = reader.count(domain.max_templates)?;
    let mut templates = Vec::new();
    templates
        .try_reserve_exact(template_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..template_count {
        templates.push(read_template(&mut reader)?);
    }
    let preview_count = reader.count(domain.max_previews)?;
    let mut previews = Vec::new();
    previews
        .try_reserve_exact(preview_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..preview_count {
        previews.push(read_preview(&mut reader)?);
    }
    let journey_count = reader.count(domain.max_journeys)?;
    let mut journeys = Vec::new();
    journeys
        .try_reserve_exact(journey_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..journey_count {
        journeys.push(read_journey(&mut reader, version)?);
    }
    let participation_blueprints = if version < 4 {
        Vec::new()
    } else {
        let count = reader.count(domain.max_journeys)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| CodecErrorV1::Capacity)?;
        for _ in 0..count {
            values.push(read_participation_blueprint(&mut reader)?);
        }
        values
    };
    let participation_attributions = if version < 5 {
        Vec::new()
    } else {
        let count = reader.count(domain.max_journeys)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| CodecErrorV1::Capacity)?;
        for _ in 0..count {
            values.push(read_participation_attribution(&mut reader)?);
        }
        values
    };
    let task_attempt_namespaces = if version < 6 {
        Vec::new()
    } else {
        let count = reader.count(domain.max_journeys)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| CodecErrorV1::Capacity)?;
        for _ in 0..count {
            values.push(read_task_attempt_namespace(&mut reader)?);
        }
        values
    };
    let actor_participation_relationships = if version < 7 {
        Vec::new()
    } else {
        let count = reader.count(domain.max_journeys)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| CodecErrorV1::Capacity)?;
        for _ in 0..count {
            values.push(read_actor_participation_relationship(&mut reader)?);
        }
        values
    };
    reader.finish()?;
    let checkpoint = CheckpointV1 {
        owner,
        limits: domain,
        owner_revision,
        next_preview,
        next_journey,
        templates,
        previews,
        journeys,
        participation_blueprints,
        participation_attributions,
        task_attempt_namespaces,
        actor_participation_relationships,
    };
    Engine::recover(checkpoint.clone()).map_err(|_| CodecErrorV1::Invalid)?;
    if encode_version(&checkpoint, limits, version)? != bytes {
        return Err(CodecErrorV1::NonCanonical);
    }
    Ok(checkpoint)
}

fn domain_limits(writer: &mut Writer, value: LimitsV1) -> Result<(), CodecErrorV1> {
    writer.number(to_u64(value.max_templates)?)?;
    writer.number(to_u64(value.max_previews)?)?;
    writer.number(to_u64(value.max_journeys)?)?;
    writer.number(value.max_preview_ttl)
}

fn read_domain_limits(reader: &mut Reader<'_>, host: LimitsV1) -> Result<LimitsV1, CodecErrorV1> {
    Ok(LimitsV1 {
        max_templates: reader.bounded_count(host.max_templates)?,
        max_previews: reader.bounded_count(host.max_previews)?,
        max_journeys: reader.bounded_count(host.max_journeys)?,
        max_preview_ttl: bounded_u64(reader.number()?, host.max_preview_ttl)?,
    })
}

fn template(writer: &mut Writer, value: TemplateV1) -> Result<(), CodecErrorV1> {
    writer.id(value.id.0)?;
    writer.number(value.version)?;
    writer.digest(value.digest)?;
    writer.byte(work_mode_tag(value.work_mode))?;
    writer.id(value.profile.0)?;
    writer.byte(room_policy_tag(value.room_policy))?;
    budget(writer, value.budget)?;
    writer.digest(value.grounding_policy_digest)?;
    writer.digest(value.authority_policy_digest)?;
    writer.digest(value.consequence_digest)?;
    writer.byte(u8::from(value.retired))
}

fn read_template(reader: &mut Reader<'_>) -> Result<TemplateV1, CodecErrorV1> {
    Ok(TemplateV1 {
        id: TemplateIdV1(reader.id()?),
        version: reader.number()?,
        digest: reader.digest()?,
        work_mode: read_work_mode(reader.byte()?)?,
        profile: ProfileIdV1(reader.id()?),
        room_policy: read_room_policy(reader.byte()?)?,
        budget: read_budget(reader)?,
        grounding_policy_digest: reader.digest()?,
        authority_policy_digest: reader.digest()?,
        consequence_digest: reader.digest()?,
        retired: read_bool(reader.byte()?)?,
    })
}

fn preview(writer: &mut Writer, value: PreviewSnapshotV1) -> Result<(), CodecErrorV1> {
    scoped_preview_id(writer, value.id)?;
    writer.number(value.owner_revision)?;
    preview_request(writer, value.request)?;
    writer.number(value.observed_at)?;
    writer.number(value.expires_at)?;
    writer.byte(work_mode_tag(value.work_mode))?;
    budget(writer, value.budget)?;
    writer.digest(value.grounding_policy_digest)?;
    writer.digest(value.authority_policy_digest)?;
    writer.digest(value.consequence_digest)
}

fn read_preview(reader: &mut Reader<'_>) -> Result<PreviewSnapshotV1, CodecErrorV1> {
    Ok(PreviewSnapshotV1 {
        id: read_preview_id(reader)?,
        owner_revision: reader.number()?,
        request: read_preview_request(reader)?,
        observed_at: reader.number()?,
        expires_at: reader.number()?,
        work_mode: read_work_mode(reader.byte()?)?,
        budget: read_budget(reader)?,
        grounding_policy_digest: reader.digest()?,
        authority_policy_digest: reader.digest()?,
        consequence_digest: reader.digest()?,
    })
}

fn preview_request(writer: &mut Writer, value: PreviewRequestV1) -> Result<(), CodecErrorV1> {
    writer.id(value.operation.0)?;
    template_ref(writer, value.template)?;
    task_admission(writer, value.task)?;
    writer.id(value.profile.0)?;
    writer.byte(room_policy_tag(value.room_policy))?;
    writer.id(value.initiating_principal.0)?;
    writer.id(value.accountable_human.0)?;
    writer.digest(value.objective_digest)?;
    writer.digest(value.acceptance_digest)?;
    writer.digest(value.current_evidence_digest)?;
    writer.number(value.requested_ttl)
}

fn read_preview_request(reader: &mut Reader<'_>) -> Result<PreviewRequestV1, CodecErrorV1> {
    Ok(PreviewRequestV1 {
        operation: PreviewOperationIdV1(reader.id()?),
        template: read_template_ref(reader)?,
        task: read_task_admission(reader)?,
        profile: ProfileIdV1(reader.id()?),
        room_policy: read_room_policy(reader.byte()?)?,
        initiating_principal: PrincipalIdV1(reader.id()?),
        accountable_human: PrincipalIdV1(reader.id()?),
        objective_digest: reader.digest()?,
        acceptance_digest: reader.digest()?,
        current_evidence_digest: reader.digest()?,
        requested_ttl: reader.number()?,
    })
}

fn journey(writer: &mut Writer, value: JourneySnapshotV1, version: u8) -> Result<(), CodecErrorV1> {
    scoped_journey_id(writer, value.id)?;
    writer.id(value.workflow.owner.0)?;
    writer.number(value.workflow.sequence)?;
    writer.number(value.revision)?;
    writer.byte(match value.phase {
        PhaseV1::Admitted => 1,
        PhaseV1::TaskAttemptReady => 2,
        PhaseV1::RoomReady => 3,
        PhaseV1::ParticipationAdmitted => 4,
    })?;
    instantiate_request(writer, value.admission)?;
    writer.number(value.confirmed_at)?;
    preview(writer, value.preview)?;
    if version == 1 {
        if value.task_attempt.is_some()
            || value.room.is_some()
            || value.participation.is_some()
            || value.phase != PhaseV1::Admitted
        {
            return Err(CodecErrorV1::Invalid);
        }
        return Ok(());
    }
    match value.task_attempt {
        None => writer.byte(0),
        Some(binding) => {
            writer.byte(1)?;
            task_attempt(writer, binding)
        }
    }?;
    if version == 2 {
        if value.room.is_some()
            || value.participation.is_some()
            || matches!(
                value.phase,
                PhaseV1::RoomReady | PhaseV1::ParticipationAdmitted
            )
        {
            return Err(CodecErrorV1::Invalid);
        }
        return Ok(());
    }
    match value.room {
        None => writer.byte(0),
        Some(binding) => {
            writer.byte(1)?;
            room(writer, binding)
        }
    }?;
    if version < 5 {
        if value.participation.is_some() || value.phase == PhaseV1::ParticipationAdmitted {
            return Err(CodecErrorV1::Invalid);
        }
        return Ok(());
    }
    match value.participation {
        None => writer.byte(0),
        Some(binding) => {
            writer.byte(1)?;
            participation_admission(writer, binding)
        }
    }
}

fn read_journey(reader: &mut Reader<'_>, version: u8) -> Result<JourneySnapshotV1, CodecErrorV1> {
    let id = read_journey_id(reader)?;
    let workflow = WorkflowRefV1 {
        owner: OwnerIdV1(reader.id()?),
        sequence: reader.number()?,
    };
    let revision = reader.number()?;
    let phase = match reader.byte()? {
        1 => PhaseV1::Admitted,
        2 if version >= 2 => PhaseV1::TaskAttemptReady,
        3 if version >= 3 => PhaseV1::RoomReady,
        4 if version >= 5 => PhaseV1::ParticipationAdmitted,
        _ => return Err(CodecErrorV1::Invalid),
    };
    let admission = read_instantiate_request(reader)?;
    let confirmed_at = reader.number()?;
    let preview = read_preview(reader)?;
    let task_attempt = if version == 1 {
        None
    } else {
        match reader.byte()? {
            0 => None,
            1 => Some(read_task_attempt(reader)?),
            _ => return Err(CodecErrorV1::Invalid),
        }
    };
    let room = if version < 3 {
        None
    } else {
        match reader.byte()? {
            0 => None,
            1 => Some(read_room(reader)?),
            _ => return Err(CodecErrorV1::Invalid),
        }
    };
    let participation = if version < 5 {
        None
    } else {
        match reader.byte()? {
            0 => None,
            1 => Some(read_participation_admission(reader)?),
            _ => return Err(CodecErrorV1::Invalid),
        }
    };
    Ok(JourneySnapshotV1 {
        id,
        workflow,
        revision,
        phase,
        admission,
        confirmed_at,
        preview,
        task_attempt,
        room,
        participation,
    })
}

fn participation_attribution(
    writer: &mut Writer,
    value: ParticipationDispatchAttributionV1,
) -> Result<(), CodecErrorV1> {
    participation_blueprint(writer, value.blueprint)?;
    participation_authority(writer, value.authority)
}

fn read_participation_attribution(
    reader: &mut Reader<'_>,
) -> Result<ParticipationDispatchAttributionV1, CodecErrorV1> {
    Ok(ParticipationDispatchAttributionV1 {
        blueprint: read_participation_blueprint(reader)?,
        authority: read_participation_authority(reader)?,
    })
}

fn participation_authority(
    writer: &mut Writer,
    value: ParticipationAdmissionAuthorityV1,
) -> Result<(), CodecErrorV1> {
    scoped_journey_id(writer, value.journey)?;
    writer.id(value.participation.0)?;
    writer.id(value.decision)?;
    writer.id(value.principal)?;
    writer.number(value.epoch)?;
    writer.number(value.expires_at)
}

fn read_participation_authority(
    reader: &mut Reader<'_>,
) -> Result<ParticipationAdmissionAuthorityV1, CodecErrorV1> {
    Ok(ParticipationAdmissionAuthorityV1 {
        journey: read_journey_id(reader)?,
        participation: crate::agent_participation::ParticipationIdV1(reader.id()?),
        decision: reader.id()?,
        principal: reader.id()?,
        epoch: reader.number()?,
        expires_at: reader.number()?,
    })
}

fn participation_admission(
    writer: &mut Writer,
    value: ParticipationAdmissionBindingV1,
) -> Result<(), CodecErrorV1> {
    scoped_journey_id(writer, value.journey)?;
    writer.number(value.expected_journey_revision)?;
    writer.id(value.participation.0)?;
    writer.number(value.owner_revision)?;
    writer.byte(agent_participation_phase_tag(value.owner_phase))?;
    participation_authority(writer, value.authority)
}

fn read_participation_admission(
    reader: &mut Reader<'_>,
) -> Result<ParticipationAdmissionBindingV1, CodecErrorV1> {
    Ok(ParticipationAdmissionBindingV1 {
        journey: read_journey_id(reader)?,
        expected_journey_revision: reader.number()?,
        participation: crate::agent_participation::ParticipationIdV1(reader.id()?),
        owner_revision: reader.number()?,
        owner_phase: read_agent_participation_phase(reader.byte()?)?,
        authority: read_participation_authority(reader)?,
    })
}

fn room(writer: &mut Writer, value: RoomBindingV1) -> Result<(), CodecErrorV1> {
    scoped_journey_id(writer, value.journey)?;
    writer.number(value.expected_journey_revision)?;
    writer.id(value.source)?;
    writer.number(value.room_sequence)?;
    writer.number(value.agora_revision)
}

fn read_room(reader: &mut Reader<'_>) -> Result<RoomBindingV1, CodecErrorV1> {
    Ok(RoomBindingV1 {
        journey: read_journey_id(reader)?,
        expected_journey_revision: reader.number()?,
        source: reader.id()?,
        room_sequence: reader.number()?,
        agora_revision: reader.number()?,
    })
}

fn participation_blueprint(
    writer: &mut Writer,
    value: ParticipationBlueprintV1,
) -> Result<(), CodecErrorV1> {
    scoped_journey_id(writer, value.journey)?;
    writer.number(value.expected_journey_revision)?;
    participation_workflow(writer, value.workflow)?;
    deliberation_member(writer, value.deliberation)?;
    participation_binding(writer, value.binding)
}

fn read_participation_blueprint(
    reader: &mut Reader<'_>,
) -> Result<ParticipationBlueprintV1, CodecErrorV1> {
    Ok(ParticipationBlueprintV1 {
        journey: read_journey_id(reader)?,
        expected_journey_revision: reader.number()?,
        workflow: read_participation_workflow(reader)?,
        deliberation: read_deliberation_member(reader)?,
        binding: read_participation_binding(reader)?,
    })
}

fn participation_workflow(
    writer: &mut Writer,
    value: ParticipationWorkflowReceiptV1,
) -> Result<(), CodecErrorV1> {
    writer.id(value.source)?;
    writer.number(value.workflow_sequence)?;
    writer.id(value.participation_workflow.into_bytes())?;
    writer.number(value.relationship_revision)
}

fn read_participation_workflow(
    reader: &mut Reader<'_>,
) -> Result<ParticipationWorkflowReceiptV1, CodecErrorV1> {
    Ok(ParticipationWorkflowReceiptV1 {
        source: reader.id()?,
        workflow_sequence: reader.number()?,
        participation_workflow: WorkflowId::new(reader.id()?),
        relationship_revision: reader.number()?,
    })
}

fn deliberation_member(
    writer: &mut Writer,
    value: DeliberationMemberReceiptV1,
) -> Result<(), CodecErrorV1> {
    writer.id(value.deliberation.0)?;
    writer.id(value.member.0)?;
    method(writer, value.method)?;
    grounding(writer, value.grounding)?;
    writer.byte(persona_tag(value.persona))?;
    ensemble_admission(writer, value.ensemble_admission)?;
    writer.number(value.owner_revision)
}

fn read_deliberation_member(
    reader: &mut Reader<'_>,
) -> Result<DeliberationMemberReceiptV1, CodecErrorV1> {
    Ok(DeliberationMemberReceiptV1 {
        deliberation: DeliberationIdV1(reader.id()?),
        member: EnsembleMemberIdV1(reader.id()?),
        method: read_method(reader)?,
        grounding: read_grounding(reader)?,
        persona: read_persona(reader.byte()?)?,
        ensemble_admission: read_ensemble_admission(reader)?,
        owner_revision: reader.number()?,
    })
}

fn participation_binding(
    writer: &mut Writer,
    value: AgentParticipationBindingV1,
) -> Result<(), CodecErrorV1> {
    writer.id(value.participation.0)?;
    writer.id(value.workflow.into_bytes())?;
    writer.id(value.deliberation.0)?;
    writer.id(value.member.0)?;
    writer.id(value.session.0)?;
    writer.id(value.profile.0)?;
    writer.id(value.attempt.task_key)?;
    writer.number(value.attempt.incarnation)?;
    writer.number(u64::from(value.attempt.attempt))?;
    method(writer, value.method)?;
    grounding(writer, value.grounding)?;
    writer.byte(persona_tag(value.persona))
}

fn read_participation_binding(
    reader: &mut Reader<'_>,
) -> Result<AgentParticipationBindingV1, CodecErrorV1> {
    Ok(AgentParticipationBindingV1 {
        participation: crate::agent_participation::ParticipationIdV1(reader.id()?),
        workflow: WorkflowId::new(reader.id()?),
        deliberation: DeliberationIdV1(reader.id()?),
        member: EnsembleMemberIdV1(reader.id()?),
        session: crate::agent_participation::AgentSessionIdV1(reader.id()?),
        profile: ProfileIdV1(reader.id()?),
        attempt: crate::agent_participation::AttemptV1 {
            task_key: reader.id()?,
            incarnation: reader.number()?,
            attempt: reader
                .number()?
                .try_into()
                .map_err(|_| CodecErrorV1::Invalid)?,
        },
        method: read_method(reader)?,
        grounding: read_grounding(reader)?,
        persona: read_persona(reader.byte()?)?,
    })
}

fn method(writer: &mut Writer, value: MethodRefV1) -> Result<(), CodecErrorV1> {
    writer.digest(value.machine_digest)?;
    writer.digest(value.state_digest)
}

fn read_method(reader: &mut Reader<'_>) -> Result<MethodRefV1, CodecErrorV1> {
    Ok(MethodRefV1 {
        machine_digest: reader.digest()?,
        state_digest: reader.digest()?,
    })
}

fn grounding(writer: &mut Writer, value: GroundingRefV1) -> Result<(), CodecErrorV1> {
    writer.digest(value.authority_digest)?;
    writer.digest(value.surface_digest)?;
    writer.byte(surface_depth_tag(value.depth))
}

fn read_grounding(reader: &mut Reader<'_>) -> Result<GroundingRefV1, CodecErrorV1> {
    Ok(GroundingRefV1 {
        authority_digest: reader.digest()?,
        surface_digest: reader.digest()?,
        depth: read_surface_depth(reader.byte()?)?,
    })
}

fn ensemble_admission(writer: &mut Writer, value: EnsembleAdmissionV1) -> Result<(), CodecErrorV1> {
    writer.digest(value.affected_population_digest)?;
    writer.digest(value.stakeholder_map_digest)?;
    writer.digest(value.coverage_manifest_digest)?;
    writer.digest(value.equity_assessment_digest)?;
    writer.id(value.assessor_principal)?;
    writer.byte(ensemble_fitness_tag(value.fitness))
}

fn read_ensemble_admission(reader: &mut Reader<'_>) -> Result<EnsembleAdmissionV1, CodecErrorV1> {
    Ok(EnsembleAdmissionV1 {
        affected_population_digest: reader.digest()?,
        stakeholder_map_digest: reader.digest()?,
        coverage_manifest_digest: reader.digest()?,
        equity_assessment_digest: reader.digest()?,
        assessor_principal: reader.id()?,
        fitness: read_ensemble_fitness(reader.byte()?)?,
    })
}

fn task_attempt(writer: &mut Writer, value: TaskAttemptBindingV1) -> Result<(), CodecErrorV1> {
    scoped_journey_id(writer, value.journey)?;
    writer.number(value.expected_journey_revision)?;
    writer.id(value.task.external_task_key())?;
    writer.number(value.task.incarnation())?;
    writer.number(u64::from(value.attempt_number))?;
    writer.number(value.chronos_revision)
}

fn read_task_attempt(reader: &mut Reader<'_>) -> Result<TaskAttemptBindingV1, CodecErrorV1> {
    Ok(TaskAttemptBindingV1 {
        journey: read_journey_id(reader)?,
        expected_journey_revision: reader.number()?,
        task: ChronosTaskRefV1::new(reader.id()?, reader.number()?),
        attempt_number: reader
            .number()?
            .try_into()
            .map_err(|_| CodecErrorV1::Invalid)?,
        chronos_revision: reader.number()?,
    })
}

fn task_attempt_namespace(
    writer: &mut Writer,
    value: &TaskAttemptNamespaceBindingV1,
) -> Result<(), CodecErrorV1> {
    scoped_journey_id(writer, value.journey)?;
    writer.number(value.expected_journey_revision)?;
    writer.id(value.task.external_task_key())?;
    writer.number(value.task.incarnation())?;
    writer.number(u64::from(value.attempt_number))?;
    writer.number(value.chronos_revision)?;
    writer.text(&value.chronos_task_id)
}

fn read_task_attempt_namespace(
    reader: &mut Reader<'_>,
) -> Result<TaskAttemptNamespaceBindingV1, CodecErrorV1> {
    Ok(TaskAttemptNamespaceBindingV1 {
        journey: read_journey_id(reader)?,
        expected_journey_revision: reader.number()?,
        task: ChronosTaskRefV1::new(reader.id()?, reader.number()?),
        attempt_number: reader
            .number()?
            .try_into()
            .map_err(|_| CodecErrorV1::Invalid)?,
        chronos_revision: reader.number()?,
        chronos_task_id: reader.text(MAX_CHRONOS_TASK_ID_BYTES_V1)?,
    })
}

fn actor_participation_relationship(
    writer: &mut Writer,
    value: &ActorParticipationRelationshipV1,
) -> Result<(), CodecErrorV1> {
    scoped_journey_id(writer, value.journey)?;
    writer.number(value.journey_revision)?;
    participation_binding(writer, value.participation)?;
    writer.number(value.participation_owner_revision)?;
    participation_authority(writer, value.authority)?;
    actor_publication_evidence(writer, &value.publication)
}

fn read_actor_participation_relationship(
    reader: &mut Reader<'_>,
) -> Result<ActorParticipationRelationshipV1, CodecErrorV1> {
    Ok(ActorParticipationRelationshipV1 {
        journey: read_journey_id(reader)?,
        journey_revision: reader.number()?,
        participation: read_participation_binding(reader)?,
        participation_owner_revision: reader.number()?,
        authority: read_participation_authority(reader)?,
        publication: read_actor_publication_evidence(reader)?,
    })
}

fn actor_publication_evidence(
    writer: &mut Writer,
    value: &ActorPublicationEvidenceV1,
) -> Result<(), CodecErrorV1> {
    writer.id(value.effect)?;
    writer.id(value.source)?;
    writer.number(value.room_sequence)?;
    writer.text(&value.room_id)?;
    writer.number(value.message_sequence)?;
    writer.id(value.message_id)?;
    writer.number(value.agora_revision)?;
    writer.text(&value.actor)?;
    writer.id(value.subject)
}

fn read_actor_publication_evidence(
    reader: &mut Reader<'_>,
) -> Result<ActorPublicationEvidenceV1, CodecErrorV1> {
    Ok(ActorPublicationEvidenceV1 {
        effect: reader.id()?,
        source: reader.id()?,
        room_sequence: reader.number()?,
        room_id: reader.text(MAX_AGORA_ROOM_ID_BYTES_V1)?,
        message_sequence: reader.number()?,
        message_id: reader.id()?,
        agora_revision: reader.number()?,
        actor: reader.text(MAX_AGORA_ACTOR_BYTES_V1)?,
        subject: reader.id()?,
    })
}

fn instantiate_request(
    writer: &mut Writer,
    value: InstantiateRequestV1,
) -> Result<(), CodecErrorV1> {
    writer.id(value.operation.0)?;
    scoped_preview_id(writer, value.preview)?;
    writer.number(value.expected_owner_revision)?;
    confirmation(writer, value.confirmation)
}

fn read_instantiate_request(reader: &mut Reader<'_>) -> Result<InstantiateRequestV1, CodecErrorV1> {
    Ok(InstantiateRequestV1 {
        operation: InstantiateOperationIdV1(reader.id()?),
        preview: read_preview_id(reader)?,
        expected_owner_revision: reader.number()?,
        confirmation: read_confirmation(reader)?,
    })
}

fn confirmation(writer: &mut Writer, value: HumanConfirmationV1) -> Result<(), CodecErrorV1> {
    scoped_preview_id(writer, value.preview)?;
    writer.id(value.accountable_human.0)?;
    writer.digest(value.consequence_digest)
}

fn read_confirmation(reader: &mut Reader<'_>) -> Result<HumanConfirmationV1, CodecErrorV1> {
    Ok(HumanConfirmationV1 {
        preview: read_preview_id(reader)?,
        accountable_human: PrincipalIdV1(reader.id()?),
        consequence_digest: reader.digest()?,
    })
}

fn template_ref(writer: &mut Writer, value: TemplateRefV1) -> Result<(), CodecErrorV1> {
    writer.id(value.id.0)?;
    writer.number(value.version)?;
    writer.digest(value.digest)
}

fn read_template_ref(reader: &mut Reader<'_>) -> Result<TemplateRefV1, CodecErrorV1> {
    Ok(TemplateRefV1 {
        id: TemplateIdV1(reader.id()?),
        version: reader.number()?,
        digest: reader.digest()?,
    })
}

fn task_admission(writer: &mut Writer, value: TaskAdmissionV1) -> Result<(), CodecErrorV1> {
    writer.id(value.task.external_task_key())?;
    writer.number(value.task.incarnation())?;
    writer.number(value.chronos_revision)?;
    writer.digest(value.evidence_digest)
}

fn read_task_admission(reader: &mut Reader<'_>) -> Result<TaskAdmissionV1, CodecErrorV1> {
    Ok(TaskAdmissionV1 {
        task: ChronosTaskRefV1::new(reader.id()?, reader.number()?),
        chronos_revision: reader.number()?,
        evidence_digest: reader.digest()?,
    })
}

fn budget(writer: &mut Writer, value: BudgetV1) -> Result<(), CodecErrorV1> {
    writer.number(value.max_context_bytes)?;
    writer.number(value.max_output_tokens)?;
    writer.number(value.max_elapsed_millis)
}

fn read_budget(reader: &mut Reader<'_>) -> Result<BudgetV1, CodecErrorV1> {
    Ok(BudgetV1 {
        max_context_bytes: reader.number()?,
        max_output_tokens: reader.number()?,
        max_elapsed_millis: reader.number()?,
    })
}

fn scoped_preview_id(writer: &mut Writer, value: PreviewIdV1) -> Result<(), CodecErrorV1> {
    writer.id(value.owner.0)?;
    writer.number(value.sequence)
}

fn read_preview_id(reader: &mut Reader<'_>) -> Result<PreviewIdV1, CodecErrorV1> {
    Ok(PreviewIdV1 {
        owner: OwnerIdV1(reader.id()?),
        sequence: reader.number()?,
    })
}

fn scoped_journey_id(writer: &mut Writer, value: JourneyIdV1) -> Result<(), CodecErrorV1> {
    writer.id(value.owner.0)?;
    writer.number(value.sequence)
}

fn read_journey_id(reader: &mut Reader<'_>) -> Result<JourneyIdV1, CodecErrorV1> {
    Ok(JourneyIdV1 {
        owner: OwnerIdV1(reader.id()?),
        sequence: reader.number()?,
    })
}

fn work_mode_tag(value: WorkModeV1) -> u8 {
    match value {
        WorkModeV1::Collaborator => 1,
        WorkModeV1::ProductManager => 2,
        WorkModeV1::ProjectManager => 3,
        WorkModeV1::SiteReliability => 4,
    }
}

fn read_work_mode(value: u8) -> Result<WorkModeV1, CodecErrorV1> {
    match value {
        1 => Ok(WorkModeV1::Collaborator),
        2 => Ok(WorkModeV1::ProductManager),
        3 => Ok(WorkModeV1::ProjectManager),
        4 => Ok(WorkModeV1::SiteReliability),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn room_policy_tag(value: RoomPolicyV1) -> u8 {
    match value {
        RoomPolicyV1::Direct => 1,
        RoomPolicyV1::SharedContext => 2,
    }
}

fn read_room_policy(value: u8) -> Result<RoomPolicyV1, CodecErrorV1> {
    match value {
        1 => Ok(RoomPolicyV1::Direct),
        2 => Ok(RoomPolicyV1::SharedContext),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn persona_tag(value: PersonaLensV1) -> u8 {
    match value {
        PersonaLensV1::Historian => 1,
        PersonaLensV1::IdeaStormer => 2,
        PersonaLensV1::Theorist => 3,
        PersonaLensV1::Researcher => 4,
        PersonaLensV1::RigorReviewer => 5,
        PersonaLensV1::ConvergenceFacilitator => 6,
    }
}

fn read_persona(value: u8) -> Result<PersonaLensV1, CodecErrorV1> {
    match value {
        1 => Ok(PersonaLensV1::Historian),
        2 => Ok(PersonaLensV1::IdeaStormer),
        3 => Ok(PersonaLensV1::Theorist),
        4 => Ok(PersonaLensV1::Researcher),
        5 => Ok(PersonaLensV1::RigorReviewer),
        6 => Ok(PersonaLensV1::ConvergenceFacilitator),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn surface_depth_tag(value: crate::deliberation::SurfaceDepthV1) -> u8 {
    match value {
        crate::deliberation::SurfaceDepthV1::Ambient => 1,
        crate::deliberation::SurfaceDepthV1::Activated => 2,
        crate::deliberation::SurfaceDepthV1::Reference => 3,
    }
}

fn read_surface_depth(value: u8) -> Result<crate::deliberation::SurfaceDepthV1, CodecErrorV1> {
    match value {
        1 => Ok(crate::deliberation::SurfaceDepthV1::Ambient),
        2 => Ok(crate::deliberation::SurfaceDepthV1::Activated),
        3 => Ok(crate::deliberation::SurfaceDepthV1::Reference),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn ensemble_fitness_tag(value: EnsembleFitnessV1) -> u8 {
    match value {
        EnsembleFitnessV1::Yes => 1,
        EnsembleFitnessV1::No => 2,
        EnsembleFitnessV1::Contested => 3,
    }
}

fn read_ensemble_fitness(value: u8) -> Result<EnsembleFitnessV1, CodecErrorV1> {
    match value {
        1 => Ok(EnsembleFitnessV1::Yes),
        2 => Ok(EnsembleFitnessV1::No),
        3 => Ok(EnsembleFitnessV1::Contested),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn agent_participation_phase_tag(value: AgentParticipationPhaseV1) -> u8 {
    match value {
        AgentParticipationPhaseV1::Admitted => 1,
        AgentParticipationPhaseV1::Ready => 2,
        AgentParticipationPhaseV1::TurnActive => 3,
        AgentParticipationPhaseV1::AwaitingPermission => 4,
        AgentParticipationPhaseV1::Terminal => 5,
        AgentParticipationPhaseV1::FencedUnknown => 6,
        AgentParticipationPhaseV1::Closed => 7,
        AgentParticipationPhaseV1::ReattachRequired => 8,
    }
}

fn read_agent_participation_phase(value: u8) -> Result<AgentParticipationPhaseV1, CodecErrorV1> {
    match value {
        1 => Ok(AgentParticipationPhaseV1::Admitted),
        2 => Ok(AgentParticipationPhaseV1::Ready),
        3 => Ok(AgentParticipationPhaseV1::TurnActive),
        4 => Ok(AgentParticipationPhaseV1::AwaitingPermission),
        5 => Ok(AgentParticipationPhaseV1::Terminal),
        6 => Ok(AgentParticipationPhaseV1::FencedUnknown),
        7 => Ok(AgentParticipationPhaseV1::Closed),
        8 => Ok(AgentParticipationPhaseV1::ReattachRequired),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn read_bool(value: u8) -> Result<bool, CodecErrorV1> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn validate_limits(value: CodecLimitsV1) -> Result<(), CodecErrorV1> {
    if value.max_bytes == 0
        || value.domain.max_templates == 0
        || value.domain.max_previews == 0
        || value.domain.max_journeys == 0
        || value.domain.max_preview_ttl == 0
    {
        Err(CodecErrorV1::InvalidLimits)
    } else {
        Ok(())
    }
}

fn host_bounds(value: LimitsV1, host: LimitsV1) -> Result<(), CodecErrorV1> {
    if value.max_templates > host.max_templates
        || value.max_previews > host.max_previews
        || value.max_journeys > host.max_journeys
        || value.max_preview_ttl > host.max_preview_ttl
    {
        Err(CodecErrorV1::Limit)
    } else {
        Ok(())
    }
}

fn bounded_u64(value: u64, maximum: u64) -> Result<u64, CodecErrorV1> {
    if value == 0 || value > maximum {
        Err(CodecErrorV1::Limit)
    } else {
        Ok(value)
    }
}

fn to_u64(value: usize) -> Result<u64, CodecErrorV1> {
    u64::try_from(value).map_err(|_| CodecErrorV1::Limit)
}
