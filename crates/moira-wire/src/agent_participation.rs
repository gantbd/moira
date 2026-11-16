use super::{CodecErrorV1, LimitsV1, Reader, Writer};
use moira_protocol::{agent_participation as p, deliberation as d};

const REQUEST: u8 = 8;
const REPLY: u8 = 9;

pub fn encode_agent_participation_request(
    value: &p::CommandV1,
    limits: LimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    let mut w = Writer::new(REQUEST, limits)?;
    command(&mut w, value)?;
    Ok(w.bytes)
}

pub fn decode_agent_participation_request(
    bytes: &[u8],
    limits: LimitsV1,
) -> Result<p::CommandV1, CodecErrorV1> {
    let mut r = Reader::new(bytes, REQUEST, limits)?;
    let value = read_command(&mut r)?;
    r.finish()?;
    Ok(value)
}

pub fn encode_agent_participation_reply(
    value: &p::ReplyV1,
    limits: LimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    let mut w = Writer::new(REPLY, limits)?;
    match value {
        p::ReplyV1::Accepted(value) => {
            w.byte(0)?;
            outcome(&mut w, value)?;
        }
        p::ReplyV1::Refused(value) => {
            w.byte(1)?;
            refusal(&mut w, *value)?;
        }
    }
    Ok(w.bytes)
}

pub fn decode_agent_participation_reply(
    bytes: &[u8],
    limits: LimitsV1,
) -> Result<p::ReplyV1, CodecErrorV1> {
    let mut r = Reader::new(bytes, REPLY, limits)?;
    let value = match r.byte()? {
        0 => p::ReplyV1::Accepted(Box::new(read_outcome(&mut r)?)),
        1 => p::ReplyV1::Refused(read_refusal(&mut r)?),
        _ => return Err(CodecErrorV1::Unsupported),
    };
    r.finish()?;
    Ok(value)
}

fn command(w: &mut Writer, value: &p::CommandV1) -> Result<(), CodecErrorV1> {
    match value {
        p::CommandV1::Apply {
            expected_storage_revision,
            mutation: value,
        } => {
            w.byte(0)?;
            optional_u64(w, *expected_storage_revision)?;
            mutation(w, value)
        }
        p::CommandV1::Status => w.byte(1),
        p::CommandV1::Participation { participation } => {
            w.byte(2)?;
            w.raw(&participation.0)
        }
        p::CommandV1::SelectPermission(value) => {
            w.byte(3)?;
            selection(w, *value)
        }
        p::CommandV1::PermissionConfirmation { participation } => {
            w.byte(4)?;
            w.raw(&participation.0)
        }
        p::CommandV1::ConfirmPermission(value) => {
            w.byte(5)?;
            w.raw(&value.participation.0)?;
            w.raw(&value.confirmation_digest)?;
            w.raw(&value.option.0)
        }
        p::CommandV1::CancelConfirmation { participation } => {
            w.byte(6)?;
            w.raw(&participation.0)
        }
        p::CommandV1::ConfirmCancel(value) => {
            w.byte(7)?;
            w.raw(&value.participation.0)?;
            w.raw(&value.confirmation_digest)
        }
        p::CommandV1::SubmitPrompt(value) => {
            w.byte(8)?;
            w.raw(&value.participation.0)?;
            w.bytes(&value.prompt)
        }
        p::CommandV1::MaterializePrompt(value) => {
            w.byte(9)?;
            w.raw(&value.participation.0)?;
            w.u64(value.expected_revision)?;
            w.raw(&value.prompt_digest)
        }
        p::CommandV1::ReattachConfirmation { participation } => {
            w.byte(10)?;
            w.raw(&participation.0)
        }
        p::CommandV1::ConfirmReattach(value) => {
            w.byte(11)?;
            w.raw(&value.participation.0)?;
            w.raw(&value.confirmation_digest)
        }
    }
}

fn read_command(r: &mut Reader<'_>) -> Result<p::CommandV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(p::CommandV1::Apply {
            expected_storage_revision: read_optional_u64(r)?,
            mutation: Box::new(read_mutation(r)?),
        }),
        1 => Ok(p::CommandV1::Status),
        2 => Ok(p::CommandV1::Participation {
            participation: p::ParticipationIdV1(r.raw()?),
        }),
        3 => Ok(p::CommandV1::SelectPermission(read_selection(r)?)),
        4 => Ok(p::CommandV1::PermissionConfirmation {
            participation: p::ParticipationIdV1(r.raw()?),
        }),
        5 => Ok(p::CommandV1::ConfirmPermission(
            p::ConfirmedPermissionSelectionV1 {
                participation: p::ParticipationIdV1(r.raw()?),
                confirmation_digest: r.raw()?,
                option: p::PermissionOptionIdV1(r.raw()?),
            },
        )),
        6 => Ok(p::CommandV1::CancelConfirmation {
            participation: p::ParticipationIdV1(r.raw()?),
        }),
        7 => Ok(p::CommandV1::ConfirmCancel(p::ConfirmedCancelV1 {
            participation: p::ParticipationIdV1(r.raw()?),
            confirmation_digest: r.raw()?,
        })),
        8 => Ok(p::CommandV1::SubmitPrompt(p::PromptSubmissionV1 {
            participation: p::ParticipationIdV1(r.raw()?),
            prompt: r.bytes()?,
        })),
        9 => Ok(p::CommandV1::MaterializePrompt(
            p::PromptMaterializationRequestV1 {
                participation: p::ParticipationIdV1(r.raw()?),
                expected_revision: r.u64()?,
                prompt_digest: r.raw()?,
            },
        )),
        10 => Ok(p::CommandV1::ReattachConfirmation {
            participation: p::ParticipationIdV1(r.raw()?),
        }),
        11 => Ok(p::CommandV1::ConfirmReattach(p::ConfirmedReattachV1 {
            participation: p::ParticipationIdV1(r.raw()?),
            confirmation_digest: r.raw()?,
        })),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn mutation(w: &mut Writer, value: &p::MutationV1) -> Result<(), CodecErrorV1> {
    match value {
        p::MutationV1::Admit(value) => {
            w.byte(0)?;
            binding(w, *value)
        }
        p::MutationV1::Prepare {
            participation,
            expected_revision,
            operation: value,
        } => {
            w.byte(1)?;
            w.raw(&participation.0)?;
            w.u64(*expected_revision)?;
            operation(w, *value)
        }
        p::MutationV1::Observe {
            participation,
            expected_revision,
            key,
            observation: value,
        } => {
            w.byte(2)?;
            w.raw(&participation.0)?;
            w.u64(*expected_revision)?;
            w.raw(&key.0)?;
            observation(w, *value)
        }
        p::MutationV1::FenceUnknown {
            participation,
            expected_revision,
            key,
        } => {
            w.byte(3)?;
            w.raw(&participation.0)?;
            w.u64(*expected_revision)?;
            w.raw(&key.0)
        }
        p::MutationV1::MarkReattachRequired {
            participation,
            expected_revision,
            session_revision,
        } => {
            w.byte(4)?;
            w.raw(&participation.0)?;
            w.u64(*expected_revision)?;
            w.u64(*session_revision)
        }
    }
}

fn read_mutation(r: &mut Reader<'_>) -> Result<p::MutationV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(p::MutationV1::Admit(read_binding(r)?)),
        1 => Ok(p::MutationV1::Prepare {
            participation: p::ParticipationIdV1(r.raw()?),
            expected_revision: r.u64()?,
            operation: read_operation(r)?,
        }),
        2 => Ok(p::MutationV1::Observe {
            participation: p::ParticipationIdV1(r.raw()?),
            expected_revision: r.u64()?,
            key: p::IdempotencyKeyV1(r.raw()?),
            observation: read_observation(r)?,
        }),
        3 => Ok(p::MutationV1::FenceUnknown {
            participation: p::ParticipationIdV1(r.raw()?),
            expected_revision: r.u64()?,
            key: p::IdempotencyKeyV1(r.raw()?),
        }),
        4 => Ok(p::MutationV1::MarkReattachRequired {
            participation: p::ParticipationIdV1(r.raw()?),
            expected_revision: r.u64()?,
            session_revision: r.u64()?,
        }),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn selection(w: &mut Writer, value: p::PermissionSelectionV1) -> Result<(), CodecErrorV1> {
    optional_u64(w, value.expected_storage_revision)?;
    w.raw(&value.participation.0)?;
    w.u64(value.expected_revision)?;
    w.raw(&value.key.0)?;
    w.raw(&value.execution.0)?;
    w.raw(&value.request.0)?;
    w.raw(&value.option.0)?;
    w.raw(&value.capability_digest)?;
    attempt(w, value.attempt)
}

fn read_selection(r: &mut Reader<'_>) -> Result<p::PermissionSelectionV1, CodecErrorV1> {
    Ok(p::PermissionSelectionV1 {
        expected_storage_revision: read_optional_u64(r)?,
        participation: p::ParticipationIdV1(r.raw()?),
        expected_revision: r.u64()?,
        key: p::IdempotencyKeyV1(r.raw()?),
        execution: p::ExecutionIdV1(r.raw()?),
        request: p::PermissionRequestIdV1(r.raw()?),
        option: p::PermissionOptionIdV1(r.raw()?),
        capability_digest: r.raw()?,
        attempt: read_attempt(r)?,
    })
}

fn outcome(w: &mut Writer, value: &p::OutcomeV1) -> Result<(), CodecErrorV1> {
    match value {
        p::OutcomeV1::Applied {
            storage_revision,
            participation,
            replayed,
        } => {
            w.byte(0)?;
            w.u64(*storage_revision)?;
            view(w, participation)?;
            boolean(w, *replayed)
        }
        p::OutcomeV1::Status(value) => {
            w.byte(1)?;
            optional_u64(w, value.storage_revision)?;
            boolean(w, value.needs_reopen)
        }
        p::OutcomeV1::Participation(value) => {
            w.byte(2)?;
            w.option(value.as_ref(), view)
        }
        p::OutcomeV1::PermissionConfirmation(value) => {
            w.byte(3)?;
            w.option(*value, permission_confirmation)
        }
        p::OutcomeV1::CancelConfirmation(value) => {
            w.byte(4)?;
            w.option(*value, cancel_confirmation)
        }
        p::OutcomeV1::PromptMaterialization(value) => {
            w.byte(5)?;
            w.raw(&value.participation.0)?;
            w.u64(value.participation_revision)?;
            w.raw(&value.prompt_digest)?;
            w.bytes(&value.prompt)
        }
        p::OutcomeV1::ReattachConfirmation(value) => {
            w.byte(6)?;
            w.option(*value, reattach_confirmation)
        }
    }
}

fn read_outcome(r: &mut Reader<'_>) -> Result<p::OutcomeV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(p::OutcomeV1::Applied {
            storage_revision: r.u64()?,
            participation: read_view(r)?,
            replayed: read_boolean(r)?,
        }),
        1 => Ok(p::OutcomeV1::Status(p::StatusV1 {
            storage_revision: read_optional_u64(r)?,
            needs_reopen: read_boolean(r)?,
        })),
        2 => Ok(p::OutcomeV1::Participation(r.option(read_view)?)),
        3 => Ok(p::OutcomeV1::PermissionConfirmation(
            r.option(read_permission_confirmation)?,
        )),
        4 => Ok(p::OutcomeV1::CancelConfirmation(
            r.option(read_cancel_confirmation)?,
        )),
        5 => Ok(p::OutcomeV1::PromptMaterialization(
            p::PromptMaterializationV1 {
                participation: p::ParticipationIdV1(r.raw()?),
                participation_revision: r.u64()?,
                prompt_digest: r.raw()?,
                prompt: r.bytes()?,
            },
        )),
        6 => Ok(p::OutcomeV1::ReattachConfirmation(
            r.option(read_reattach_confirmation)?,
        )),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn cancel_confirmation(w: &mut Writer, value: p::CancelConfirmationV1) -> Result<(), CodecErrorV1> {
    w.u64(value.storage_revision)?;
    w.raw(&value.participation.0)?;
    w.u64(value.participation_revision)?;
    w.u64(value.session_revision)?;
    w.raw(&value.turn.0)?;
    w.raw(&value.confirmation_digest)
}

fn read_cancel_confirmation(r: &mut Reader<'_>) -> Result<p::CancelConfirmationV1, CodecErrorV1> {
    Ok(p::CancelConfirmationV1 {
        storage_revision: r.u64()?,
        participation: p::ParticipationIdV1(r.raw()?),
        participation_revision: r.u64()?,
        session_revision: r.u64()?,
        turn: p::AgentTurnIdV1(r.raw()?),
        confirmation_digest: r.raw()?,
    })
}

fn reattach_confirmation(
    w: &mut Writer,
    value: p::ReattachConfirmationV1,
) -> Result<(), CodecErrorV1> {
    w.u64(value.storage_revision)?;
    w.raw(&value.participation.0)?;
    w.u64(value.participation_revision)?;
    w.u64(value.session_revision)?;
    w.raw(&value.confirmation_digest)
}

fn read_reattach_confirmation(
    r: &mut Reader<'_>,
) -> Result<p::ReattachConfirmationV1, CodecErrorV1> {
    Ok(p::ReattachConfirmationV1 {
        storage_revision: r.u64()?,
        participation: p::ParticipationIdV1(r.raw()?),
        participation_revision: r.u64()?,
        session_revision: r.u64()?,
        confirmation_digest: r.raw()?,
    })
}

fn permission_confirmation(
    w: &mut Writer,
    value: p::PermissionConfirmationV1,
) -> Result<(), CodecErrorV1> {
    w.u64(value.storage_revision)?;
    w.raw(&value.participation.0)?;
    w.u64(value.participation_revision)?;
    w.u64(value.session_revision)?;
    w.raw(&value.request.0)?;
    w.raw(&value.turn.0)?;
    w.raw(&value.capability_digest)?;
    w.raw(&value.options_digest)?;
    w.byte(value.option_count)?;
    for option in value.option_ids {
        w.raw(&option.0)?;
    }
    w.raw(&value.confirmation_digest)
}

fn read_permission_confirmation(
    r: &mut Reader<'_>,
) -> Result<p::PermissionConfirmationV1, CodecErrorV1> {
    let storage_revision = r.u64()?;
    let participation = p::ParticipationIdV1(r.raw()?);
    let participation_revision = r.u64()?;
    let session_revision = r.u64()?;
    let request = p::PermissionRequestIdV1(r.raw()?);
    let turn = p::AgentTurnIdV1(r.raw()?);
    let capability_digest = r.raw()?;
    let options_digest = r.raw()?;
    let option_count = r.byte()?;
    let mut option_ids = [p::PermissionOptionIdV1([0; 16]); p::MAX_PERMISSION_OPTIONS_V1];
    for option in &mut option_ids {
        *option = p::PermissionOptionIdV1(r.raw()?);
    }
    Ok(p::PermissionConfirmationV1 {
        storage_revision,
        participation,
        participation_revision,
        session_revision,
        request,
        turn,
        capability_digest,
        options_digest,
        option_count,
        option_ids,
        confirmation_digest: r.raw()?,
    })
}

fn binding(w: &mut Writer, value: p::BindingV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.participation.0)?;
    w.raw(&value.workflow.0)?;
    w.raw(&value.deliberation.0)?;
    w.raw(&value.member.0)?;
    w.raw(&value.session.0)?;
    w.raw(&value.profile.0)?;
    attempt(w, value.attempt)?;
    w.raw(&value.method.machine_digest)?;
    w.raw(&value.method.state_digest)?;
    grounding(w, value.grounding)?;
    w.byte(persona_tag(value.persona))
}

fn read_binding(r: &mut Reader<'_>) -> Result<p::BindingV1, CodecErrorV1> {
    Ok(p::BindingV1 {
        participation: p::ParticipationIdV1(r.raw()?),
        workflow: moira_protocol::WorkflowIdV1(r.raw()?),
        deliberation: d::DeliberationIdV1(r.raw()?),
        member: d::EnsembleMemberIdV1(r.raw()?),
        session: p::AgentSessionIdV1(r.raw()?),
        profile: p::ProfileIdV1(r.raw()?),
        attempt: read_attempt(r)?,
        method: d::MethodRefV1 {
            machine_digest: r.raw()?,
            state_digest: r.raw()?,
        },
        grounding: read_grounding(r)?,
        persona: read_persona(r.byte()?)?,
    })
}

fn operation(w: &mut Writer, value: p::OperationV1) -> Result<(), CodecErrorV1> {
    match value {
        p::OperationV1::Open { key, execution } => {
            w.byte(0)?;
            w.raw(&key.0)?;
            w.raw(&execution.0)
        }
        p::OperationV1::Prompt {
            key,
            turn,
            execution,
            prompt_digest,
            attempt: value,
        } => {
            w.byte(1)?;
            w.raw(&key.0)?;
            w.raw(&turn.0)?;
            w.raw(&execution.0)?;
            w.raw(&prompt_digest)?;
            attempt(w, value)
        }
        p::OperationV1::Advance { key, turn } => {
            w.byte(2)?;
            w.raw(&key.0)?;
            w.raw(&turn.0)
        }
        p::OperationV1::DecidePermission {
            key,
            execution,
            request,
            option,
            authority,
            attempt: value,
        } => {
            w.byte(3)?;
            w.raw(&key.0)?;
            w.raw(&execution.0)?;
            w.raw(&request.0)?;
            w.raw(&option.0)?;
            w.raw(&authority.decision.0)?;
            w.raw(&authority.subject.0)?;
            w.raw(&authority.capability_digest)?;
            w.u64(authority.epoch)?;
            attempt(w, value)
        }
        p::OperationV1::Cancel { key, turn } => {
            w.byte(4)?;
            w.raw(&key.0)?;
            w.raw(&turn.0)
        }
        p::OperationV1::Close { key } => {
            w.byte(5)?;
            w.raw(&key.0)
        }
        p::OperationV1::Reattach {
            key,
            execution,
            attempt: value,
        } => {
            w.byte(6)?;
            w.raw(&key.0)?;
            w.raw(&execution.0)?;
            attempt(w, value)
        }
    }
}

fn read_operation(r: &mut Reader<'_>) -> Result<p::OperationV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(p::OperationV1::Open {
            key: p::IdempotencyKeyV1(r.raw()?),
            execution: p::ExecutionIdV1(r.raw()?),
        }),
        1 => Ok(p::OperationV1::Prompt {
            key: p::IdempotencyKeyV1(r.raw()?),
            turn: p::AgentTurnIdV1(r.raw()?),
            execution: p::ExecutionIdV1(r.raw()?),
            prompt_digest: r.raw()?,
            attempt: read_attempt(r)?,
        }),
        2 => Ok(p::OperationV1::Advance {
            key: p::IdempotencyKeyV1(r.raw()?),
            turn: p::AgentTurnIdV1(r.raw()?),
        }),
        3 => Ok(p::OperationV1::DecidePermission {
            key: p::IdempotencyKeyV1(r.raw()?),
            execution: p::ExecutionIdV1(r.raw()?),
            request: p::PermissionRequestIdV1(r.raw()?),
            option: p::PermissionOptionIdV1(r.raw()?),
            authority: p::PermissionAuthorityV1 {
                decision: p::AuthorityDecisionIdV1(r.raw()?),
                subject: p::AuthoritySubjectIdV1(r.raw()?),
                capability_digest: r.raw()?,
                epoch: r.u64()?,
            },
            attempt: read_attempt(r)?,
        }),
        4 => Ok(p::OperationV1::Cancel {
            key: p::IdempotencyKeyV1(r.raw()?),
            turn: p::AgentTurnIdV1(r.raw()?),
        }),
        5 => Ok(p::OperationV1::Close {
            key: p::IdempotencyKeyV1(r.raw()?),
        }),
        6 => Ok(p::OperationV1::Reattach {
            key: p::IdempotencyKeyV1(r.raw()?),
            execution: p::ExecutionIdV1(r.raw()?),
            attempt: read_attempt(r)?,
        }),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn observation(w: &mut Writer, value: p::ObservationV1) -> Result<(), CodecErrorV1> {
    w.byte(phase_tag(value.phase))?;
    w.u64(value.session_revision)?;
    w.option(value.active_turn, |w, value| w.raw(&value.0))?;
    optional_u64(w, value.last_event_cursor)?;
    w.raw(&value.profile_document_digest)?;
    capabilities(w, value.capabilities)?;
    w.option(value.permission, permission_evidence)?;
    w.option(value.terminal, terminal)
}

fn read_observation(r: &mut Reader<'_>) -> Result<p::ObservationV1, CodecErrorV1> {
    Ok(p::ObservationV1 {
        phase: read_phase(r.byte()?)?,
        session_revision: r.u64()?,
        active_turn: r.option(|r| r.raw().map(p::AgentTurnIdV1))?,
        last_event_cursor: read_optional_u64(r)?,
        profile_document_digest: r.raw()?,
        capabilities: read_capabilities(r)?,
        permission: r.option(read_permission_evidence)?,
        terminal: r.option(read_terminal)?,
    })
}

fn view(w: &mut Writer, value: &p::ParticipationViewV1) -> Result<(), CodecErrorV1> {
    binding(w, value.binding)?;
    w.u64(value.revision)?;
    w.byte(phase_tag(value.phase))?;
    optional_u64(w, value.session_revision)?;
    optional_u64(w, value.last_event_cursor)?;
    w.option(value.profile_document_digest, |w, value| w.raw(&value))?;
    capabilities(w, value.capabilities)?;
    w.option(value.active_turn, |w, value| w.raw(&value.0))?;
    w.option(value.permission, permission_evidence)?;
    w.option(value.terminal, terminal)?;
    w.option(value.pending, operation)
}

fn read_view(r: &mut Reader<'_>) -> Result<p::ParticipationViewV1, CodecErrorV1> {
    Ok(p::ParticipationViewV1 {
        binding: read_binding(r)?,
        revision: r.u64()?,
        phase: read_phase(r.byte()?)?,
        session_revision: read_optional_u64(r)?,
        last_event_cursor: read_optional_u64(r)?,
        profile_document_digest: r.option(Reader::raw)?,
        capabilities: read_capabilities(r)?,
        active_turn: r.option(|r| r.raw().map(p::AgentTurnIdV1))?,
        permission: r.option(read_permission_evidence)?,
        terminal: r.option(read_terminal)?,
        pending: r.option(read_operation)?,
    })
}

fn attempt(w: &mut Writer, value: p::AttemptV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.task_key)?;
    w.u64(value.incarnation)?;
    w.u32(value.attempt)
}

fn read_attempt(r: &mut Reader<'_>) -> Result<p::AttemptV1, CodecErrorV1> {
    Ok(p::AttemptV1 {
        task_key: r.raw()?,
        incarnation: r.u64()?,
        attempt: r.u32()?,
    })
}

fn grounding(w: &mut Writer, value: d::GroundingRefV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.authority_digest)?;
    w.raw(&value.surface_digest)?;
    w.byte(match value.depth {
        d::SurfaceDepthV1::Ambient => 0,
        d::SurfaceDepthV1::Activated => 1,
        d::SurfaceDepthV1::Reference => 2,
    })
}

fn read_grounding(r: &mut Reader<'_>) -> Result<d::GroundingRefV1, CodecErrorV1> {
    Ok(d::GroundingRefV1 {
        authority_digest: r.raw()?,
        surface_digest: r.raw()?,
        depth: match r.byte()? {
            0 => d::SurfaceDepthV1::Ambient,
            1 => d::SurfaceDepthV1::Activated,
            2 => d::SurfaceDepthV1::Reference,
            _ => return Err(CodecErrorV1::Unsupported),
        },
    })
}

fn capabilities(w: &mut Writer, value: p::CapabilitiesV1) -> Result<(), CodecErrorV1> {
    let values = [
        value.load_session,
        value.resume_session,
        value.permissions,
        value.filesystem_read,
        value.filesystem_write,
        value.terminal,
        value.session_modes,
        value.session_models,
    ];
    let mut bits = 0_u8;
    for (index, enabled) in values.into_iter().enumerate() {
        if enabled {
            bits |= 1 << index;
        }
    }
    w.byte(bits)
}

fn read_capabilities(r: &mut Reader<'_>) -> Result<p::CapabilitiesV1, CodecErrorV1> {
    let bits = r.byte()?;
    Ok(p::CapabilitiesV1 {
        load_session: bits & 1 != 0,
        resume_session: bits & 2 != 0,
        permissions: bits & 4 != 0,
        filesystem_read: bits & 8 != 0,
        filesystem_write: bits & 16 != 0,
        terminal: bits & 32 != 0,
        session_modes: bits & 64 != 0,
        session_models: bits & 128 != 0,
    })
}

fn permission_evidence(w: &mut Writer, value: p::PermissionEvidenceV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.request.0)?;
    w.raw(&value.turn.0)?;
    w.raw(&value.capability_digest)?;
    w.raw(&value.options_digest)?;
    w.byte(value.option_count)?;
    for value in &value.option_ids[..usize::from(value.option_count)] {
        w.raw(&value.0)?;
    }
    Ok(())
}

fn read_permission_evidence(r: &mut Reader<'_>) -> Result<p::PermissionEvidenceV1, CodecErrorV1> {
    let request = p::PermissionRequestIdV1(r.raw()?);
    let turn = p::AgentTurnIdV1(r.raw()?);
    let capability_digest = r.raw()?;
    let options_digest = r.raw()?;
    let option_count = r.byte()?;
    if usize::from(option_count) > p::MAX_PERMISSION_OPTIONS_V1 {
        return Err(CodecErrorV1::Malformed);
    }
    let mut option_ids = [p::PermissionOptionIdV1([0; 16]); p::MAX_PERMISSION_OPTIONS_V1];
    for value in &mut option_ids[..usize::from(option_count)] {
        *value = p::PermissionOptionIdV1(r.raw()?);
    }
    Ok(p::PermissionEvidenceV1 {
        request,
        turn,
        capability_digest,
        options_digest,
        option_count,
        option_ids,
    })
}

fn terminal(w: &mut Writer, value: p::TerminalEvidenceV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.turn.0)?;
    w.byte(disposition_tag(value.disposition))?;
    w.byte(stop_tag(value.stop_reason))?;
    w.raw(&value.output_digest)?;
    w.raw(&value.provider_evidence_digest)
}

fn read_terminal(r: &mut Reader<'_>) -> Result<p::TerminalEvidenceV1, CodecErrorV1> {
    Ok(p::TerminalEvidenceV1 {
        turn: p::AgentTurnIdV1(r.raw()?),
        disposition: read_disposition(r.byte()?)?,
        stop_reason: read_stop(r.byte()?)?,
        output_digest: r.raw()?,
        provider_evidence_digest: r.raw()?,
    })
}

fn refusal(w: &mut Writer, value: p::RefusalV1) -> Result<(), CodecErrorV1> {
    match value {
        p::RefusalV1::StorageRevisionConflict { expected, actual } => {
            w.byte(1)?;
            optional_u64(w, expected)?;
            optional_u64(w, actual)
        }
        value => w.byte(match value {
            p::RefusalV1::NotAuthorized => 0,
            p::RefusalV1::InvalidLimits => 2,
            p::RefusalV1::Limit => 3,
            p::RefusalV1::InvalidBinding => 4,
            p::RefusalV1::DuplicateParticipation => 5,
            p::RefusalV1::UnknownParticipation => 6,
            p::RefusalV1::RevisionConflict => 7,
            p::RefusalV1::RevisionExhausted => 8,
            p::RefusalV1::InvalidOperation => 9,
            p::RefusalV1::OperationConflict => 10,
            p::RefusalV1::NoPendingOperation => 11,
            p::RefusalV1::InvalidObservation => 12,
            p::RefusalV1::ObservationConflict => 13,
            p::RefusalV1::InvalidCheckpoint => 14,
            p::RefusalV1::StorageRevisionConflict { .. } => unreachable!(),
        }),
    }
}

fn read_refusal(r: &mut Reader<'_>) -> Result<p::RefusalV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => p::RefusalV1::NotAuthorized,
        1 => p::RefusalV1::StorageRevisionConflict {
            expected: read_optional_u64(r)?,
            actual: read_optional_u64(r)?,
        },
        2 => p::RefusalV1::InvalidLimits,
        3 => p::RefusalV1::Limit,
        4 => p::RefusalV1::InvalidBinding,
        5 => p::RefusalV1::DuplicateParticipation,
        6 => p::RefusalV1::UnknownParticipation,
        7 => p::RefusalV1::RevisionConflict,
        8 => p::RefusalV1::RevisionExhausted,
        9 => p::RefusalV1::InvalidOperation,
        10 => p::RefusalV1::OperationConflict,
        11 => p::RefusalV1::NoPendingOperation,
        12 => p::RefusalV1::InvalidObservation,
        13 => p::RefusalV1::ObservationConflict,
        14 => p::RefusalV1::InvalidCheckpoint,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn optional_u64(w: &mut Writer, value: Option<u64>) -> Result<(), CodecErrorV1> {
    w.option(value, Writer::u64)
}

fn read_optional_u64(r: &mut Reader<'_>) -> Result<Option<u64>, CodecErrorV1> {
    r.option(Reader::u64)
}

fn boolean(w: &mut Writer, value: bool) -> Result<(), CodecErrorV1> {
    w.byte(u8::from(value))
}

fn read_boolean(r: &mut Reader<'_>) -> Result<bool, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CodecErrorV1::Malformed),
    }
}

fn phase_tag(value: p::PhaseV1) -> u8 {
    match value {
        p::PhaseV1::Admitted => 0,
        p::PhaseV1::Ready => 1,
        p::PhaseV1::TurnActive => 2,
        p::PhaseV1::AwaitingPermission => 3,
        p::PhaseV1::Terminal => 4,
        p::PhaseV1::FencedUnknown => 5,
        p::PhaseV1::Closed => 6,
        p::PhaseV1::ReattachRequired => 7,
    }
}

fn read_phase(value: u8) -> Result<p::PhaseV1, CodecErrorV1> {
    match value {
        0 => Ok(p::PhaseV1::Admitted),
        1 => Ok(p::PhaseV1::Ready),
        2 => Ok(p::PhaseV1::TurnActive),
        3 => Ok(p::PhaseV1::AwaitingPermission),
        4 => Ok(p::PhaseV1::Terminal),
        5 => Ok(p::PhaseV1::FencedUnknown),
        6 => Ok(p::PhaseV1::Closed),
        7 => Ok(p::PhaseV1::ReattachRequired),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn persona_tag(value: d::PersonaLensV1) -> u8 {
    match value {
        d::PersonaLensV1::Historian => 0,
        d::PersonaLensV1::IdeaStormer => 1,
        d::PersonaLensV1::Theorist => 2,
        d::PersonaLensV1::Researcher => 3,
        d::PersonaLensV1::RigorReviewer => 4,
        d::PersonaLensV1::ConvergenceFacilitator => 5,
    }
}

fn read_persona(value: u8) -> Result<d::PersonaLensV1, CodecErrorV1> {
    match value {
        0 => Ok(d::PersonaLensV1::Historian),
        1 => Ok(d::PersonaLensV1::IdeaStormer),
        2 => Ok(d::PersonaLensV1::Theorist),
        3 => Ok(d::PersonaLensV1::Researcher),
        4 => Ok(d::PersonaLensV1::RigorReviewer),
        5 => Ok(d::PersonaLensV1::ConvergenceFacilitator),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn disposition_tag(value: p::DispositionV1) -> u8 {
    match value {
        p::DispositionV1::Completed => 0,
        p::DispositionV1::Failed => 1,
        p::DispositionV1::Cancelled => 2,
        p::DispositionV1::TimedOut => 3,
        p::DispositionV1::Uncertain => 4,
    }
}

fn read_disposition(value: u8) -> Result<p::DispositionV1, CodecErrorV1> {
    match value {
        0 => Ok(p::DispositionV1::Completed),
        1 => Ok(p::DispositionV1::Failed),
        2 => Ok(p::DispositionV1::Cancelled),
        3 => Ok(p::DispositionV1::TimedOut),
        4 => Ok(p::DispositionV1::Uncertain),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn stop_tag(value: p::StopReasonV1) -> u8 {
    match value {
        p::StopReasonV1::EndTurn => 0,
        p::StopReasonV1::Cancelled => 1,
        p::StopReasonV1::TimedOut => 2,
        p::StopReasonV1::Failed => 3,
        p::StopReasonV1::Disconnected => 4,
        p::StopReasonV1::OutcomeUnknown => 5,
    }
}

fn read_stop(value: u8) -> Result<p::StopReasonV1, CodecErrorV1> {
    match value {
        0 => Ok(p::StopReasonV1::EndTurn),
        1 => Ok(p::StopReasonV1::Cancelled),
        2 => Ok(p::StopReasonV1::TimedOut),
        3 => Ok(p::StopReasonV1::Failed),
        4 => Ok(p::StopReasonV1::Disconnected),
        5 => Ok(p::StopReasonV1::OutcomeUnknown),
        _ => Err(CodecErrorV1::Unsupported),
    }
}
