use super::{CodecErrorV1, LimitsV1, Reader, Writer};
use moira_protocol::{TaskRefV1, agent_participation::ProfileIdV1, task_agent_journey as p};

const REQUEST: u8 = 18;
const REPLY: u8 = 19;

pub fn encode_task_agent_journey_request(
    value: &p::CommandV1,
    limits: LimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    let mut writer = Writer::new(REQUEST, limits)?;
    command(&mut writer, value)?;
    Ok(writer.bytes)
}

pub fn decode_task_agent_journey_request(
    bytes: &[u8],
    limits: LimitsV1,
) -> Result<p::CommandV1, CodecErrorV1> {
    let mut reader = Reader::new(bytes, REQUEST, limits)?;
    let value = read_command(&mut reader)?;
    reader.finish()?;
    Ok(value)
}

pub fn encode_task_agent_journey_reply(
    value: &p::ReplyV1,
    limits: LimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    let mut writer = Writer::new(REPLY, limits)?;
    match value {
        p::ReplyV1::Accepted(value) => {
            writer.byte(0)?;
            outcome(&mut writer, value)?;
        }
        p::ReplyV1::Refused(value) => {
            writer.byte(1)?;
            writer.byte(refusal_tag(*value))?;
        }
    }
    Ok(writer.bytes)
}

pub fn decode_task_agent_journey_reply(
    bytes: &[u8],
    limits: LimitsV1,
) -> Result<p::ReplyV1, CodecErrorV1> {
    let mut reader = Reader::new(bytes, REPLY, limits)?;
    let value = match reader.byte()? {
        0 => p::ReplyV1::Accepted(Box::new(read_outcome(&mut reader)?)),
        1 => p::ReplyV1::Refused(read_refusal(reader.byte()?)?),
        _ => return Err(CodecErrorV1::Unsupported),
    };
    reader.finish()?;
    Ok(value)
}

fn command(writer: &mut Writer, value: &p::CommandV1) -> Result<(), CodecErrorV1> {
    match value {
        p::CommandV1::Catalogue => writer.byte(0),
        p::CommandV1::Preview {
            expected_storage_revision,
            request,
        } => {
            writer.byte(1)?;
            optional_u64(writer, *expected_storage_revision)?;
            preview_request(writer, *request)
        }
        p::CommandV1::Instantiate {
            expected_storage_revision,
            request,
        } => {
            writer.byte(2)?;
            optional_u64(writer, *expected_storage_revision)?;
            instantiate_request(writer, *request)
        }
        p::CommandV1::PreviewView { preview } => {
            writer.byte(3)?;
            preview_id(writer, *preview)
        }
        p::CommandV1::Journey { journey } => {
            writer.byte(4)?;
            journey_id(writer, *journey)
        }
        p::CommandV1::Status => writer.byte(5),
        p::CommandV1::Participation { journey } => {
            writer.byte(6)?;
            journey_id(writer, *journey)
        }
        p::CommandV1::TaskAttemptNamespace { journey } => {
            writer.byte(7)?;
            journey_id(writer, *journey)
        }
    }
}

fn read_command(reader: &mut Reader<'_>) -> Result<p::CommandV1, CodecErrorV1> {
    match reader.byte()? {
        0 => Ok(p::CommandV1::Catalogue),
        1 => Ok(p::CommandV1::Preview {
            expected_storage_revision: read_optional_u64(reader)?,
            request: read_preview_request(reader)?,
        }),
        2 => Ok(p::CommandV1::Instantiate {
            expected_storage_revision: read_optional_u64(reader)?,
            request: read_instantiate_request(reader)?,
        }),
        3 => Ok(p::CommandV1::PreviewView {
            preview: read_preview_id(reader)?,
        }),
        4 => Ok(p::CommandV1::Journey {
            journey: read_journey_id(reader)?,
        }),
        5 => Ok(p::CommandV1::Status),
        6 => Ok(p::CommandV1::Participation {
            journey: read_journey_id(reader)?,
        }),
        7 => Ok(p::CommandV1::TaskAttemptNamespace {
            journey: read_journey_id(reader)?,
        }),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn outcome(writer: &mut Writer, value: &p::OutcomeV1) -> Result<(), CodecErrorV1> {
    match value {
        p::OutcomeV1::Catalogue {
            owner,
            owner_revision,
            storage_revision,
            templates,
        } => {
            writer.byte(0)?;
            writer.raw(&owner.0)?;
            writer.u64(*owner_revision)?;
            writer.u64(*storage_revision)?;
            writer.usize(templates.len())?;
            for value in templates {
                template(writer, *value)?;
            }
            Ok(())
        }
        p::OutcomeV1::Previewed {
            storage_revision,
            receipt,
        } => {
            writer.byte(1)?;
            writer.u64(*storage_revision)?;
            preview_view(writer, receipt.preview)?;
            boolean(writer, receipt.replayed)
        }
        p::OutcomeV1::Admitted {
            storage_revision,
            receipt,
        } => {
            writer.byte(2)?;
            writer.u64(*storage_revision)?;
            journey_view(writer, receipt.journey)?;
            boolean(writer, receipt.replayed)
        }
        p::OutcomeV1::Preview(value) => {
            writer.byte(3)?;
            writer.option(*value, preview_view)
        }
        p::OutcomeV1::Journey(value) => {
            writer.byte(4)?;
            writer.option(*value, journey_view)
        }
        p::OutcomeV1::Status(value) => {
            writer.byte(5)?;
            status(writer, *value)
        }
        p::OutcomeV1::Participation(value) => {
            writer.byte(6)?;
            writer.option(*value, participation_admission)
        }
        p::OutcomeV1::TaskAttemptNamespace(value) => {
            writer.byte(7)?;
            writer.option(value.as_ref(), task_attempt_namespace)
        }
    }
}

fn read_outcome(reader: &mut Reader<'_>) -> Result<p::OutcomeV1, CodecErrorV1> {
    match reader.byte()? {
        0 => {
            let owner = p::OwnerIdV1(reader.raw()?);
            let owner_revision = reader.u64()?;
            let storage_revision = reader.u64()?;
            let count = bounded_count(reader)?;
            let mut templates = Vec::new();
            templates
                .try_reserve_exact(count)
                .map_err(|_| CodecErrorV1::Capacity)?;
            for _ in 0..count {
                templates.push(read_template(reader)?);
            }
            Ok(p::OutcomeV1::Catalogue {
                owner,
                owner_revision,
                storage_revision,
                templates,
            })
        }
        1 => Ok(p::OutcomeV1::Previewed {
            storage_revision: reader.u64()?,
            receipt: p::PreviewReceiptV1 {
                preview: read_preview_view(reader)?,
                replayed: read_boolean(reader)?,
            },
        }),
        2 => Ok(p::OutcomeV1::Admitted {
            storage_revision: reader.u64()?,
            receipt: p::JourneyReceiptV1 {
                journey: read_journey_view(reader)?,
                replayed: read_boolean(reader)?,
            },
        }),
        3 => Ok(p::OutcomeV1::Preview(reader.option(read_preview_view)?)),
        4 => Ok(p::OutcomeV1::Journey(reader.option(read_journey_view)?)),
        5 => Ok(p::OutcomeV1::Status(read_status(reader)?)),
        6 => Ok(p::OutcomeV1::Participation(
            reader.option(read_participation_admission)?,
        )),
        7 => Ok(p::OutcomeV1::TaskAttemptNamespace(
            reader.option(read_task_attempt_namespace)?,
        )),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn task_attempt_namespace(
    writer: &mut Writer,
    value: &p::TaskAttemptNamespaceViewV1,
) -> Result<(), CodecErrorV1> {
    if value.chronos_task_id.is_empty()
        || value.chronos_task_id.len() > p::MAX_CHRONOS_TASK_ID_BYTES_V1
    {
        return Err(CodecErrorV1::Limit);
    }
    journey_id(writer, value.journey)?;
    writer.u64(value.expected_journey_revision)?;
    writer.raw(&value.task.external_task_key)?;
    writer.u64(value.task.incarnation)?;
    writer.u64(u64::from(value.attempt_number))?;
    writer.u64(value.chronos_revision)?;
    writer.bytes(value.chronos_task_id.as_bytes())
}

fn read_task_attempt_namespace(
    reader: &mut Reader<'_>,
) -> Result<p::TaskAttemptNamespaceViewV1, CodecErrorV1> {
    let journey = read_journey_id(reader)?;
    let expected_journey_revision = reader.u64()?;
    let task = TaskRefV1 {
        external_task_key: reader.raw()?,
        incarnation: reader.u64()?,
    };
    let attempt_number = reader.u64()?.try_into().map_err(|_| CodecErrorV1::Limit)?;
    let chronos_revision = reader.u64()?;
    let chronos_task_id =
        String::from_utf8(reader.bytes()?).map_err(|_| CodecErrorV1::Malformed)?;
    if chronos_task_id.is_empty() || chronos_task_id.len() > p::MAX_CHRONOS_TASK_ID_BYTES_V1 {
        return Err(CodecErrorV1::Limit);
    }
    Ok(p::TaskAttemptNamespaceViewV1 {
        journey,
        expected_journey_revision,
        task,
        attempt_number,
        chronos_revision,
        chronos_task_id,
    })
}

fn template(writer: &mut Writer, value: p::TemplateV1) -> Result<(), CodecErrorV1> {
    writer.raw(&value.id.0)?;
    writer.u64(value.version)?;
    writer.raw(&value.digest)?;
    writer.byte(work_mode_tag(value.work_mode))?;
    writer.raw(&value.profile.0)?;
    writer.byte(room_policy_tag(value.room_policy))?;
    budget(writer, value.budget)?;
    writer.raw(&value.grounding_policy_digest)?;
    writer.raw(&value.authority_policy_digest)?;
    writer.raw(&value.consequence_digest)?;
    boolean(writer, value.retired)
}

fn read_template(reader: &mut Reader<'_>) -> Result<p::TemplateV1, CodecErrorV1> {
    Ok(p::TemplateV1 {
        id: p::TemplateIdV1(reader.raw()?),
        version: reader.u64()?,
        digest: reader.raw()?,
        work_mode: read_work_mode(reader.byte()?)?,
        profile: ProfileIdV1(reader.raw()?),
        room_policy: read_room_policy(reader.byte()?)?,
        budget: read_budget(reader)?,
        grounding_policy_digest: reader.raw()?,
        authority_policy_digest: reader.raw()?,
        consequence_digest: reader.raw()?,
        retired: read_boolean(reader)?,
    })
}

fn preview_request(writer: &mut Writer, value: p::PreviewRequestV1) -> Result<(), CodecErrorV1> {
    writer.raw(&value.operation.0)?;
    template_ref(writer, value.template)?;
    task_admission(writer, value.task)?;
    writer.raw(&value.profile.0)?;
    writer.byte(room_policy_tag(value.room_policy))?;
    writer.raw(&value.initiating_principal.0)?;
    writer.raw(&value.accountable_human.0)?;
    writer.raw(&value.objective_digest)?;
    writer.raw(&value.acceptance_digest)?;
    writer.raw(&value.current_evidence_digest)?;
    writer.u64(value.requested_ttl)
}

fn read_preview_request(reader: &mut Reader<'_>) -> Result<p::PreviewRequestV1, CodecErrorV1> {
    Ok(p::PreviewRequestV1 {
        operation: p::PreviewOperationIdV1(reader.raw()?),
        template: read_template_ref(reader)?,
        task: read_task_admission(reader)?,
        profile: ProfileIdV1(reader.raw()?),
        room_policy: read_room_policy(reader.byte()?)?,
        initiating_principal: p::PrincipalIdV1(reader.raw()?),
        accountable_human: p::PrincipalIdV1(reader.raw()?),
        objective_digest: reader.raw()?,
        acceptance_digest: reader.raw()?,
        current_evidence_digest: reader.raw()?,
        requested_ttl: reader.u64()?,
    })
}

fn preview_view(writer: &mut Writer, value: p::PreviewViewV1) -> Result<(), CodecErrorV1> {
    preview_id(writer, value.id)?;
    writer.u64(value.owner_revision)?;
    preview_request(writer, value.request)?;
    writer.u64(value.observed_at)?;
    writer.u64(value.expires_at)?;
    writer.byte(work_mode_tag(value.work_mode))?;
    budget(writer, value.budget)?;
    writer.raw(&value.grounding_policy_digest)?;
    writer.raw(&value.authority_policy_digest)?;
    writer.raw(&value.consequence_digest)
}

fn read_preview_view(reader: &mut Reader<'_>) -> Result<p::PreviewViewV1, CodecErrorV1> {
    Ok(p::PreviewViewV1 {
        id: read_preview_id(reader)?,
        owner_revision: reader.u64()?,
        request: read_preview_request(reader)?,
        observed_at: reader.u64()?,
        expires_at: reader.u64()?,
        work_mode: read_work_mode(reader.byte()?)?,
        budget: read_budget(reader)?,
        grounding_policy_digest: reader.raw()?,
        authority_policy_digest: reader.raw()?,
        consequence_digest: reader.raw()?,
    })
}

fn instantiate_request(
    writer: &mut Writer,
    value: p::InstantiateRequestV1,
) -> Result<(), CodecErrorV1> {
    writer.raw(&value.operation.0)?;
    preview_id(writer, value.preview)?;
    writer.u64(value.expected_owner_revision)?;
    confirmation(writer, value.confirmation)
}

fn read_instantiate_request(
    reader: &mut Reader<'_>,
) -> Result<p::InstantiateRequestV1, CodecErrorV1> {
    Ok(p::InstantiateRequestV1 {
        operation: p::InstantiateOperationIdV1(reader.raw()?),
        preview: read_preview_id(reader)?,
        expected_owner_revision: reader.u64()?,
        confirmation: read_confirmation(reader)?,
    })
}

fn journey_view(writer: &mut Writer, value: p::JourneyViewV1) -> Result<(), CodecErrorV1> {
    journey_id(writer, value.id)?;
    writer.raw(&value.workflow.owner.0)?;
    writer.u64(value.workflow.sequence)?;
    writer.u64(value.revision)?;
    writer.byte(match value.phase {
        p::PhaseV1::Admitted => 1,
        p::PhaseV1::TaskAttemptReady => 2,
        p::PhaseV1::RoomReady => 3,
        p::PhaseV1::ParticipationAdmitted => 4,
    })?;
    instantiate_request(writer, value.admission)?;
    writer.u64(value.confirmed_at)?;
    preview_view(writer, value.preview)?;
    writer.option(value.task_attempt, task_attempt)?;
    writer.option(value.room, room)
}

fn read_journey_view(reader: &mut Reader<'_>) -> Result<p::JourneyViewV1, CodecErrorV1> {
    Ok(p::JourneyViewV1 {
        id: read_journey_id(reader)?,
        workflow: p::WorkflowRefV1 {
            owner: p::OwnerIdV1(reader.raw()?),
            sequence: reader.u64()?,
        },
        revision: reader.u64()?,
        phase: match reader.byte()? {
            1 => p::PhaseV1::Admitted,
            2 => p::PhaseV1::TaskAttemptReady,
            3 => p::PhaseV1::RoomReady,
            4 => p::PhaseV1::ParticipationAdmitted,
            _ => return Err(CodecErrorV1::Unsupported),
        },
        admission: read_instantiate_request(reader)?,
        confirmed_at: reader.u64()?,
        preview: read_preview_view(reader)?,
        task_attempt: reader.option(read_task_attempt)?,
        room: reader.option(read_room)?,
    })
}

fn participation_admission(
    writer: &mut Writer,
    value: p::ParticipationAdmissionViewV1,
) -> Result<(), CodecErrorV1> {
    journey_id(writer, value.journey)?;
    writer.u64(value.expected_journey_revision)?;
    writer.raw(&value.participation)?;
    writer.u64(value.owner_revision)?;
    writer.byte(match value.owner_phase {
        moira_protocol::agent_participation::PhaseV1::Admitted => 1,
        moira_protocol::agent_participation::PhaseV1::Ready => 2,
        moira_protocol::agent_participation::PhaseV1::TurnActive => 3,
        moira_protocol::agent_participation::PhaseV1::AwaitingPermission => 4,
        moira_protocol::agent_participation::PhaseV1::Terminal => 5,
        moira_protocol::agent_participation::PhaseV1::FencedUnknown => 6,
        moira_protocol::agent_participation::PhaseV1::Closed => 7,
        moira_protocol::agent_participation::PhaseV1::ReattachRequired => 8,
    })?;
    writer.raw(&value.authority_decision)?;
    writer.raw(&value.authority_principal)?;
    writer.u64(value.authority_epoch)?;
    writer.u64(value.authority_expires_at)
}

fn read_participation_admission(
    reader: &mut Reader<'_>,
) -> Result<p::ParticipationAdmissionViewV1, CodecErrorV1> {
    Ok(p::ParticipationAdmissionViewV1 {
        journey: read_journey_id(reader)?,
        expected_journey_revision: reader.u64()?,
        participation: reader.raw()?,
        owner_revision: reader.u64()?,
        owner_phase: match reader.byte()? {
            1 => moira_protocol::agent_participation::PhaseV1::Admitted,
            2 => moira_protocol::agent_participation::PhaseV1::Ready,
            3 => moira_protocol::agent_participation::PhaseV1::TurnActive,
            4 => moira_protocol::agent_participation::PhaseV1::AwaitingPermission,
            5 => moira_protocol::agent_participation::PhaseV1::Terminal,
            6 => moira_protocol::agent_participation::PhaseV1::FencedUnknown,
            7 => moira_protocol::agent_participation::PhaseV1::Closed,
            8 => moira_protocol::agent_participation::PhaseV1::ReattachRequired,
            _ => return Err(CodecErrorV1::Unsupported),
        },
        authority_decision: reader.raw()?,
        authority_principal: reader.raw()?,
        authority_epoch: reader.u64()?,
        authority_expires_at: reader.u64()?,
    })
}

fn room(writer: &mut Writer, value: p::RoomBindingV1) -> Result<(), CodecErrorV1> {
    journey_id(writer, value.journey)?;
    writer.u64(value.expected_journey_revision)?;
    writer.raw(&value.source)?;
    writer.u64(value.room_sequence)?;
    writer.u64(value.agora_revision)
}

fn read_room(reader: &mut Reader<'_>) -> Result<p::RoomBindingV1, CodecErrorV1> {
    Ok(p::RoomBindingV1 {
        journey: read_journey_id(reader)?,
        expected_journey_revision: reader.u64()?,
        source: reader.raw()?,
        room_sequence: reader.u64()?,
        agora_revision: reader.u64()?,
    })
}

fn task_attempt(writer: &mut Writer, value: p::TaskAttemptBindingV1) -> Result<(), CodecErrorV1> {
    journey_id(writer, value.journey)?;
    writer.u64(value.expected_journey_revision)?;
    writer.raw(&value.task.external_task_key)?;
    writer.u64(value.task.incarnation)?;
    writer.u64(u64::from(value.attempt_number))?;
    writer.u64(value.chronos_revision)
}

fn read_task_attempt(reader: &mut Reader<'_>) -> Result<p::TaskAttemptBindingV1, CodecErrorV1> {
    Ok(p::TaskAttemptBindingV1 {
        journey: read_journey_id(reader)?,
        expected_journey_revision: reader.u64()?,
        task: TaskRefV1 {
            external_task_key: reader.raw()?,
            incarnation: reader.u64()?,
        },
        attempt_number: reader
            .u64()?
            .try_into()
            .map_err(|_| CodecErrorV1::Unsupported)?,
        chronos_revision: reader.u64()?,
    })
}

fn status(writer: &mut Writer, value: p::StatusV1) -> Result<(), CodecErrorV1> {
    writer.raw(&value.owner.0)?;
    writer.u64(value.owner_revision)?;
    writer.u64(value.storage_revision)?;
    writer.u64(value.template_count)?;
    writer.u64(value.preview_count)?;
    writer.u64(value.journey_count)?;
    boolean(writer, value.needs_reopen)
}

fn read_status(reader: &mut Reader<'_>) -> Result<p::StatusV1, CodecErrorV1> {
    Ok(p::StatusV1 {
        owner: p::OwnerIdV1(reader.raw()?),
        owner_revision: reader.u64()?,
        storage_revision: reader.u64()?,
        template_count: reader.u64()?,
        preview_count: reader.u64()?,
        journey_count: reader.u64()?,
        needs_reopen: read_boolean(reader)?,
    })
}

fn template_ref(writer: &mut Writer, value: p::TemplateRefV1) -> Result<(), CodecErrorV1> {
    writer.raw(&value.id.0)?;
    writer.u64(value.version)?;
    writer.raw(&value.digest)
}

fn read_template_ref(reader: &mut Reader<'_>) -> Result<p::TemplateRefV1, CodecErrorV1> {
    Ok(p::TemplateRefV1 {
        id: p::TemplateIdV1(reader.raw()?),
        version: reader.u64()?,
        digest: reader.raw()?,
    })
}

fn task_admission(writer: &mut Writer, value: p::TaskAdmissionV1) -> Result<(), CodecErrorV1> {
    writer.raw(&value.task.external_task_key)?;
    writer.u64(value.task.incarnation)?;
    writer.u64(value.chronos_revision)?;
    writer.raw(&value.evidence_digest)
}

fn read_task_admission(reader: &mut Reader<'_>) -> Result<p::TaskAdmissionV1, CodecErrorV1> {
    Ok(p::TaskAdmissionV1 {
        task: TaskRefV1 {
            external_task_key: reader.raw()?,
            incarnation: reader.u64()?,
        },
        chronos_revision: reader.u64()?,
        evidence_digest: reader.raw()?,
    })
}

fn confirmation(writer: &mut Writer, value: p::HumanConfirmationV1) -> Result<(), CodecErrorV1> {
    preview_id(writer, value.preview)?;
    writer.raw(&value.accountable_human.0)?;
    writer.raw(&value.consequence_digest)
}

fn read_confirmation(reader: &mut Reader<'_>) -> Result<p::HumanConfirmationV1, CodecErrorV1> {
    Ok(p::HumanConfirmationV1 {
        preview: read_preview_id(reader)?,
        accountable_human: p::PrincipalIdV1(reader.raw()?),
        consequence_digest: reader.raw()?,
    })
}

fn budget(writer: &mut Writer, value: p::BudgetV1) -> Result<(), CodecErrorV1> {
    writer.u64(value.max_context_bytes)?;
    writer.u64(value.max_output_tokens)?;
    writer.u64(value.max_elapsed_millis)
}

fn read_budget(reader: &mut Reader<'_>) -> Result<p::BudgetV1, CodecErrorV1> {
    Ok(p::BudgetV1 {
        max_context_bytes: reader.u64()?,
        max_output_tokens: reader.u64()?,
        max_elapsed_millis: reader.u64()?,
    })
}

fn preview_id(writer: &mut Writer, value: p::PreviewIdV1) -> Result<(), CodecErrorV1> {
    writer.raw(&value.owner.0)?;
    writer.u64(value.sequence)
}

fn read_preview_id(reader: &mut Reader<'_>) -> Result<p::PreviewIdV1, CodecErrorV1> {
    Ok(p::PreviewIdV1 {
        owner: p::OwnerIdV1(reader.raw()?),
        sequence: reader.u64()?,
    })
}

fn journey_id(writer: &mut Writer, value: p::JourneyIdV1) -> Result<(), CodecErrorV1> {
    writer.raw(&value.owner.0)?;
    writer.u64(value.sequence)
}

fn read_journey_id(reader: &mut Reader<'_>) -> Result<p::JourneyIdV1, CodecErrorV1> {
    Ok(p::JourneyIdV1 {
        owner: p::OwnerIdV1(reader.raw()?),
        sequence: reader.u64()?,
    })
}

fn optional_u64(writer: &mut Writer, value: Option<u64>) -> Result<(), CodecErrorV1> {
    writer.option(value, Writer::u64)
}

fn read_optional_u64(reader: &mut Reader<'_>) -> Result<Option<u64>, CodecErrorV1> {
    reader.option(Reader::u64)
}

fn boolean(writer: &mut Writer, value: bool) -> Result<(), CodecErrorV1> {
    writer.byte(u8::from(value))
}

fn read_boolean(reader: &mut Reader<'_>) -> Result<bool, CodecErrorV1> {
    match reader.byte()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn bounded_count(reader: &mut Reader<'_>) -> Result<usize, CodecErrorV1> {
    let value = reader.usize()?;
    if value > reader.bytes.len().saturating_sub(reader.position) {
        Err(CodecErrorV1::Limit)
    } else {
        Ok(value)
    }
}

fn work_mode_tag(value: p::WorkModeV1) -> u8 {
    match value {
        p::WorkModeV1::Collaborator => 1,
        p::WorkModeV1::ProductManager => 2,
        p::WorkModeV1::ProjectManager => 3,
        p::WorkModeV1::SiteReliability => 4,
    }
}

fn read_work_mode(value: u8) -> Result<p::WorkModeV1, CodecErrorV1> {
    match value {
        1 => Ok(p::WorkModeV1::Collaborator),
        2 => Ok(p::WorkModeV1::ProductManager),
        3 => Ok(p::WorkModeV1::ProjectManager),
        4 => Ok(p::WorkModeV1::SiteReliability),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn room_policy_tag(value: p::RoomPolicyV1) -> u8 {
    match value {
        p::RoomPolicyV1::Direct => 1,
        p::RoomPolicyV1::SharedContext => 2,
    }
}

fn read_room_policy(value: u8) -> Result<p::RoomPolicyV1, CodecErrorV1> {
    match value {
        1 => Ok(p::RoomPolicyV1::Direct),
        2 => Ok(p::RoomPolicyV1::SharedContext),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn refusal_tag(value: p::RefusalV1) -> u8 {
    match value {
        p::RefusalV1::NotAuthorized => 0,
        p::RefusalV1::StorageRevisionConflict => 1,
        p::RefusalV1::InvalidLimits => 2,
        p::RefusalV1::InvalidOwner => 3,
        p::RefusalV1::InvalidTemplate => 4,
        p::RefusalV1::DuplicateTemplate => 5,
        p::RefusalV1::TemplateLimit => 6,
        p::RefusalV1::InvalidPreview => 7,
        p::RefusalV1::PreviewLimit => 8,
        p::RefusalV1::PreviewOperationConflict => 9,
        p::RefusalV1::TemplateNotFound => 10,
        p::RefusalV1::TemplateMismatch => 11,
        p::RefusalV1::TemplateRetired => 12,
        p::RefusalV1::PreviewNotFound => 13,
        p::RefusalV1::PreviewExpired => 14,
        p::RefusalV1::OwnerRevisionConflict => 15,
        p::RefusalV1::InvalidConfirmation => 16,
        p::RefusalV1::JourneyLimit => 17,
        p::RefusalV1::InstantiateOperationConflict => 18,
        p::RefusalV1::PreviewAlreadyInstantiated => 19,
        p::RefusalV1::IdentityExhausted => 20,
        p::RefusalV1::InvalidCheckpoint => 21,
        p::RefusalV1::Storage => 22,
    }
}

fn read_refusal(value: u8) -> Result<p::RefusalV1, CodecErrorV1> {
    match value {
        0 => Ok(p::RefusalV1::NotAuthorized),
        1 => Ok(p::RefusalV1::StorageRevisionConflict),
        2 => Ok(p::RefusalV1::InvalidLimits),
        3 => Ok(p::RefusalV1::InvalidOwner),
        4 => Ok(p::RefusalV1::InvalidTemplate),
        5 => Ok(p::RefusalV1::DuplicateTemplate),
        6 => Ok(p::RefusalV1::TemplateLimit),
        7 => Ok(p::RefusalV1::InvalidPreview),
        8 => Ok(p::RefusalV1::PreviewLimit),
        9 => Ok(p::RefusalV1::PreviewOperationConflict),
        10 => Ok(p::RefusalV1::TemplateNotFound),
        11 => Ok(p::RefusalV1::TemplateMismatch),
        12 => Ok(p::RefusalV1::TemplateRetired),
        13 => Ok(p::RefusalV1::PreviewNotFound),
        14 => Ok(p::RefusalV1::PreviewExpired),
        15 => Ok(p::RefusalV1::OwnerRevisionConflict),
        16 => Ok(p::RefusalV1::InvalidConfirmation),
        17 => Ok(p::RefusalV1::JourneyLimit),
        18 => Ok(p::RefusalV1::InstantiateOperationConflict),
        19 => Ok(p::RefusalV1::PreviewAlreadyInstantiated),
        20 => Ok(p::RefusalV1::IdentityExhausted),
        21 => Ok(p::RefusalV1::InvalidCheckpoint),
        22 => Ok(p::RefusalV1::Storage),
        _ => Err(CodecErrorV1::Unsupported),
    }
}
