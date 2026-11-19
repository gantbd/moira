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
    encode_version(checkpoint, limits, 2)
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
    for value in canonical.journeys {
        journey(&mut writer, value, version)?;
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
    if !matches!(version, 1 | 2) {
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
    })?;
    instantiate_request(writer, value.admission)?;
    writer.number(value.confirmed_at)?;
    preview(writer, value.preview)?;
    if version == 1 {
        if value.task_attempt.is_some() || value.phase != PhaseV1::Admitted {
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
    Ok(JourneySnapshotV1 {
        id,
        workflow,
        revision,
        phase,
        admission,
        confirmed_at,
        preview,
        task_attempt,
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
