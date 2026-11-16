use super::{CodecErrorV1, LimitsV1, Reader, Writer};
use moira_protocol::agent::*;

const AGENT_REQUEST: u8 = 6;
const AGENT_REPLY: u8 = 7;

pub fn encode_agent_request(value: &CommandV1, limits: LimitsV1) -> Result<Vec<u8>, CodecErrorV1> {
    let mut writer = Writer::new(AGENT_REQUEST, limits)?;
    command(&mut writer, value)?;
    Ok(writer.bytes)
}

pub fn decode_agent_request(bytes: &[u8], limits: LimitsV1) -> Result<CommandV1, CodecErrorV1> {
    let mut reader = Reader::new(bytes, AGENT_REQUEST, limits)?;
    let value = read_command(&mut reader)?;
    reader.finish()?;
    Ok(value)
}

pub fn encode_agent_reply(value: &ReplyV1, limits: LimitsV1) -> Result<Vec<u8>, CodecErrorV1> {
    let mut writer = Writer::new(AGENT_REPLY, limits)?;
    match value {
        ReplyV1::Accepted(value) => {
            writer.byte(0)?;
            outcome(&mut writer, value)?;
        }
        ReplyV1::Refused(value) => {
            writer.byte(1)?;
            writer.byte(refusal_tag(*value))?;
        }
    }
    Ok(writer.bytes)
}

pub fn decode_agent_reply(bytes: &[u8], limits: LimitsV1) -> Result<ReplyV1, CodecErrorV1> {
    let mut reader = Reader::new(bytes, AGENT_REPLY, limits)?;
    let value = match reader.byte()? {
        0 => ReplyV1::Accepted(read_outcome(&mut reader)?),
        1 => ReplyV1::Refused(read_refusal(reader.byte()?)?),
        _ => return Err(CodecErrorV1::Unsupported),
    };
    reader.finish()?;
    Ok(value)
}

fn command(w: &mut Writer, value: &CommandV1) -> Result<(), CodecErrorV1> {
    match value {
        CommandV1::Open { request, now } => {
            w.byte(0)?;
            open(w, request)?;
            w.u64(*now)
        }
        CommandV1::Prompt { request, now } => {
            w.byte(1)?;
            prompt(w, request)?;
            w.u64(*now)
        }
        CommandV1::Events {
            session,
            after,
            limit,
        } => {
            w.byte(2)?;
            w.raw(&session.0)?;
            w.option(*after, |w, v| w.u64(v))?;
            w.u64(*limit)
        }
        CommandV1::Inspect { session } => {
            w.byte(3)?;
            w.raw(&session.0)
        }
        CommandV1::DecidePermission {
            session,
            expected_revision,
            execution: value,
            decision,
            now,
        } => {
            w.byte(4)?;
            w.raw(&session.0)?;
            w.u64(*expected_revision)?;
            execution(w, value)?;
            w.raw(&decision.request.0)?;
            w.raw(&decision.option.0)?;
            w.u64(*now)
        }
        CommandV1::Advance {
            session,
            turn,
            expected_revision,
            idempotency_key,
            now,
        } => control(
            w,
            5,
            *session,
            Some(*turn),
            *expected_revision,
            *idempotency_key,
            *now,
        ),
        CommandV1::Cancel {
            session,
            turn,
            expected_revision,
            idempotency_key,
            now,
        } => control(
            w,
            6,
            *session,
            Some(*turn),
            *expected_revision,
            *idempotency_key,
            *now,
        ),
        CommandV1::Close {
            session,
            expected_revision,
            idempotency_key,
            now,
        } => control(
            w,
            7,
            *session,
            None,
            *expected_revision,
            *idempotency_key,
            *now,
        ),
    }
}

fn read_command(r: &mut Reader<'_>) -> Result<CommandV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => CommandV1::Open {
            request: read_open(r)?,
            now: r.u64()?,
        },
        1 => CommandV1::Prompt {
            request: read_prompt(r)?,
            now: r.u64()?,
        },
        2 => CommandV1::Events {
            session: SessionIdV1(r.raw()?),
            after: r.option(Reader::u64)?,
            limit: r.u64()?,
        },
        3 => CommandV1::Inspect {
            session: SessionIdV1(r.raw()?),
        },
        4 => CommandV1::DecidePermission {
            session: SessionIdV1(r.raw()?),
            expected_revision: r.u64()?,
            execution: read_execution(r)?,
            decision: PermissionDecisionV1 {
                request: PermissionRequestIdV1(r.raw()?),
                option: PermissionOptionIdV1(r.raw()?),
            },
            now: r.u64()?,
        },
        5 => {
            let (session, turn, expected_revision, idempotency_key, now) = read_control(r, true)?;
            CommandV1::Advance {
                session,
                turn: turn.ok_or(CodecErrorV1::Malformed)?,
                expected_revision,
                idempotency_key,
                now,
            }
        }
        6 => {
            let (session, turn, expected_revision, idempotency_key, now) = read_control(r, true)?;
            CommandV1::Cancel {
                session,
                turn: turn.ok_or(CodecErrorV1::Malformed)?,
                expected_revision,
                idempotency_key,
                now,
            }
        }
        7 => {
            let (session, _, expected_revision, idempotency_key, now) = read_control(r, false)?;
            CommandV1::Close {
                session,
                expected_revision,
                idempotency_key,
                now,
            }
        }
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn control(
    w: &mut Writer,
    tag: u8,
    session: SessionIdV1,
    turn: Option<TurnIdV1>,
    revision: u64,
    key: IdempotencyKeyV1,
    now: u64,
) -> Result<(), CodecErrorV1> {
    w.byte(tag)?;
    w.raw(&session.0)?;
    if let Some(turn) = turn {
        w.raw(&turn.0)?;
    }
    w.u64(revision)?;
    w.raw(&key.0)?;
    w.u64(now)
}

fn read_control(
    r: &mut Reader<'_>,
    has_turn: bool,
) -> Result<(SessionIdV1, Option<TurnIdV1>, u64, IdempotencyKeyV1, u64), CodecErrorV1> {
    let session = SessionIdV1(r.raw()?);
    let turn = if has_turn {
        Some(TurnIdV1(r.raw()?))
    } else {
        None
    };
    Ok((
        session,
        turn,
        r.u64()?,
        IdempotencyKeyV1(r.raw()?),
        r.u64()?,
    ))
}

fn open(w: &mut Writer, value: &OpenRequestV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.session.0)?;
    execution(w, &value.execution)?;
    w.raw(&value.workspace)?;
    w.u64(value.budget.max_elapsed_millis)?;
    w.u32(value.budget.max_turns)?;
    w.u64(value.budget.max_events)?;
    w.usize(value.budget.max_event_bytes)?;
    w.usize(value.budget.max_prompt_bytes)?;
    w.u32(value.budget.max_permission_requests)
}

fn read_open(r: &mut Reader<'_>) -> Result<OpenRequestV1, CodecErrorV1> {
    Ok(OpenRequestV1 {
        session: SessionIdV1(r.raw()?),
        execution: read_execution(r)?,
        workspace: r.raw()?,
        budget: SessionBudgetV1 {
            max_elapsed_millis: r.u64()?,
            max_turns: r.u32()?,
            max_events: r.u64()?,
            max_event_bytes: r.usize()?,
            max_prompt_bytes: r.usize()?,
            max_permission_requests: r.u32()?,
        },
    })
}

fn prompt(w: &mut Writer, value: &PromptRequestV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.session.0)?;
    w.raw(&value.turn.0)?;
    w.u64(value.expected_revision)?;
    execution(w, &value.execution)?;
    w.raw(&value.prompt_digest.0)?;
    w.bytes(&value.prompt)
}

fn read_prompt(r: &mut Reader<'_>) -> Result<PromptRequestV1, CodecErrorV1> {
    Ok(PromptRequestV1 {
        session: SessionIdV1(r.raw()?),
        turn: TurnIdV1(r.raw()?),
        expected_revision: r.u64()?,
        execution: read_execution(r)?,
        prompt_digest: DigestV1(r.raw()?),
        prompt: r.bytes()?,
    })
}

fn execution(w: &mut Writer, value: &ExecutionV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    w.raw(&value.attempt.task_key)?;
    w.u64(value.attempt.incarnation)?;
    w.u32(value.attempt.attempt)?;
    w.raw(&value.principal.0)?;
    w.raw(&value.profile.0)?;
    w.raw(&value.idempotency_key.0)?;
    list_len(w, value.context.len())?;
    for id in &value.context {
        w.raw(&id.0)?;
    }
    list_len(w, value.tools.len())?;
    for id in &value.tools {
        w.raw(&id.0)?;
    }
    w.u64(value.budget.max_elapsed_millis)?;
    w.u64(value.budget.max_input_tokens)?;
    w.u64(value.budget.max_output_tokens)?;
    w.u32(value.budget.max_tool_calls)?;
    w.usize(value.budget.max_context_bytes)?;
    w.usize(value.budget.max_output_bytes)
}

fn read_execution(r: &mut Reader<'_>) -> Result<ExecutionV1, CodecErrorV1> {
    let id = ExecutionIdV1(r.raw()?);
    let task_key = r.raw()?;
    let incarnation = r.u64()?;
    let attempt = r.u32()?;
    let principal = PrincipalIdV1(r.raw()?);
    let profile = ProfileIdV1(r.raw()?);
    let idempotency_key = IdempotencyKeyV1(r.raw()?);
    let context = read_ids(r, ContextIdV1)?;
    let tools = read_ids(r, ToolIdV1)?;
    Ok(ExecutionV1 {
        id,
        attempt: AttemptV1 {
            task_key,
            incarnation,
            attempt,
        },
        principal,
        profile,
        idempotency_key,
        context,
        tools,
        budget: ExecutionBudgetV1 {
            max_elapsed_millis: r.u64()?,
            max_input_tokens: r.u64()?,
            max_output_tokens: r.u64()?,
            max_tool_calls: r.u32()?,
            max_context_bytes: r.usize()?,
            max_output_bytes: r.usize()?,
        },
    })
}

fn list_len(w: &mut Writer, length: usize) -> Result<(), CodecErrorV1> {
    w.u32(u32::try_from(length).map_err(|_| CodecErrorV1::Limit)?)
}
fn read_ids<T>(r: &mut Reader<'_>, wrap: fn([u8; 16]) -> T) -> Result<Vec<T>, CodecErrorV1> {
    let length = usize::try_from(r.u32()?).map_err(|_| CodecErrorV1::Limit)?;
    if length > r.bytes.len().saturating_sub(r.position) / 16 {
        return Err(CodecErrorV1::Malformed);
    }
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..length {
        values.push(wrap(r.raw()?));
    }
    Ok(values)
}

fn outcome(w: &mut Writer, value: &OutcomeV1) -> Result<(), CodecErrorV1> {
    match value {
        OutcomeV1::Session(value) => {
            w.byte(0)?;
            session_view(w, value)
        }
        OutcomeV1::Events(value) => {
            w.byte(1)?;
            event_page(w, value)
        }
        OutcomeV1::OutcomeUnknown {
            session,
            idempotency_key,
        } => {
            w.byte(2)?;
            w.raw(&session.0)?;
            w.raw(&idempotency_key.0)
        }
    }
}

fn read_outcome(r: &mut Reader<'_>) -> Result<OutcomeV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(OutcomeV1::Session(read_session_view(r)?)),
        1 => Ok(OutcomeV1::Events(read_event_page(r)?)),
        2 => Ok(OutcomeV1::OutcomeUnknown {
            session: SessionIdV1(r.raw()?),
            idempotency_key: IdempotencyKeyV1(r.raw()?),
        }),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn session_view(w: &mut Writer, value: &SessionViewV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.session.0)?;
    w.u64(value.revision)?;
    w.byte(phase_tag(value.phase))?;
    w.raw(&value.profile_document_digest.0)?;
    w.option(value.active_turn, |w, v| w.raw(&v.0))?;
    w.option(value.last_event_cursor, Writer::u64)?;
    capabilities(w, value.capabilities)
}
fn read_session_view(r: &mut Reader<'_>) -> Result<SessionViewV1, CodecErrorV1> {
    Ok(SessionViewV1 {
        session: SessionIdV1(r.raw()?),
        revision: r.u64()?,
        phase: read_phase(r.byte()?)?,
        profile_document_digest: DigestV1(r.raw()?),
        active_turn: r.option(|r| r.raw().map(TurnIdV1))?,
        last_event_cursor: r.option(Reader::u64)?,
        capabilities: read_capabilities(r)?,
    })
}

fn capabilities(w: &mut Writer, value: CapabilitiesV1) -> Result<(), CodecErrorV1> {
    let flags = (value.load_session as u8)
        | ((value.resume_session as u8) << 1)
        | ((value.permissions as u8) << 2)
        | ((value.filesystem_read as u8) << 3)
        | ((value.filesystem_write as u8) << 4)
        | ((value.terminal as u8) << 5)
        | ((value.session_modes as u8) << 6)
        | ((value.session_models as u8) << 7);
    w.byte(flags)
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

fn event_page(w: &mut Writer, value: &EventPageV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.session.0)?;
    list_len(w, value.entries.len())?;
    for record in &value.entries {
        w.u64(record.cursor)?;
        event(w, &record.event)?;
    }
    w.option(value.next_after, Writer::u64)
}
fn read_event_page(r: &mut Reader<'_>) -> Result<EventPageV1, CodecErrorV1> {
    let session = SessionIdV1(r.raw()?);
    let length = usize::try_from(r.u32()?).map_err(|_| CodecErrorV1::Limit)?;
    if length > r.bytes.len().saturating_sub(r.position) / 9 {
        return Err(CodecErrorV1::Malformed);
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(length)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..length {
        entries.push(EventRecordV1 {
            cursor: r.u64()?,
            event: read_event(r)?,
        });
    }
    Ok(EventPageV1 {
        session,
        entries,
        next_after: r.option(Reader::u64)?,
    })
}

fn event(w: &mut Writer, value: &EventV1) -> Result<(), CodecErrorV1> {
    match value {
        EventV1::AssistantMessage { bytes } => {
            w.byte(0)?;
            w.bytes(bytes)
        }
        EventV1::Thought { bytes } => {
            w.byte(1)?;
            w.bytes(bytes)
        }
        EventV1::Plan { bytes } => {
            w.byte(2)?;
            w.bytes(bytes)
        }
        EventV1::ToolCall { title, detail } => {
            w.byte(3)?;
            w.bytes(title)?;
            w.bytes(detail)
        }
        EventV1::Terminal { stream, bytes } => {
            w.byte(4)?;
            w.bytes(stream)?;
            w.bytes(bytes)
        }
        EventV1::Permission(value) => {
            w.byte(5)?;
            permission(w, value)
        }
        EventV1::FileChange {
            path,
            content_digest,
        } => {
            w.byte(6)?;
            w.bytes(path)?;
            w.raw(&content_digest.0)
        }
        EventV1::Usage {
            input_tokens,
            output_tokens,
        } => {
            w.byte(7)?;
            w.u64(*input_tokens)?;
            w.u64(*output_tokens)
        }
        EventV1::TurnFinished {
            turn,
            disposition,
            stop_reason,
            output_digest,
        } => {
            w.byte(8)?;
            w.raw(&turn.0)?;
            w.byte(disposition_tag(*disposition))?;
            w.byte(stop_tag(*stop_reason))?;
            w.option(*output_digest, |w, v| w.raw(&v.0))
        }
    }
}
fn read_event(r: &mut Reader<'_>) -> Result<EventV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => EventV1::AssistantMessage { bytes: r.bytes()? },
        1 => EventV1::Thought { bytes: r.bytes()? },
        2 => EventV1::Plan { bytes: r.bytes()? },
        3 => EventV1::ToolCall {
            title: r.bytes()?,
            detail: r.bytes()?,
        },
        4 => EventV1::Terminal {
            stream: r.bytes()?,
            bytes: r.bytes()?,
        },
        5 => EventV1::Permission(read_permission(r)?),
        6 => EventV1::FileChange {
            path: r.bytes()?,
            content_digest: DigestV1(r.raw()?),
        },
        7 => EventV1::Usage {
            input_tokens: r.u64()?,
            output_tokens: r.u64()?,
        },
        8 => EventV1::TurnFinished {
            turn: TurnIdV1(r.raw()?),
            disposition: read_disposition(r.byte()?)?,
            stop_reason: read_stop(r.byte()?)?,
            output_digest: r.option(|r| r.raw().map(DigestV1))?,
        },
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn permission(w: &mut Writer, value: &PermissionRequestV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.request.0)?;
    w.raw(&value.turn.0)?;
    w.bytes(&value.capability)?;
    w.bytes(&value.description)?;
    list_len(w, value.options.len())?;
    for option in &value.options {
        w.raw(&option.id.0)?;
        w.byte(option_kind_tag(option.kind))?;
        w.bytes(&option.label)?;
    }
    Ok(())
}
fn read_permission(r: &mut Reader<'_>) -> Result<PermissionRequestV1, CodecErrorV1> {
    let request = PermissionRequestIdV1(r.raw()?);
    let turn = TurnIdV1(r.raw()?);
    let capability = r.bytes()?;
    let description = r.bytes()?;
    let length = usize::try_from(r.u32()?).map_err(|_| CodecErrorV1::Limit)?;
    if length > r.bytes.len().saturating_sub(r.position) / 21 {
        return Err(CodecErrorV1::Malformed);
    }
    let mut options = Vec::new();
    options
        .try_reserve_exact(length)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..length {
        options.push(PermissionOptionV1 {
            id: PermissionOptionIdV1(r.raw()?),
            kind: read_option_kind(r.byte()?)?,
            label: r.bytes()?,
        });
    }
    Ok(PermissionRequestV1 {
        request,
        turn,
        capability,
        description,
        options,
    })
}

macro_rules! tags { ($tag:ident,$read:ident,$ty:ty,{$($variant:path=>$n:expr),+$(,)?}) => { fn $tag(value:$ty)->u8{match value{$($variant=>$n),+}} fn $read(value:u8)->Result<$ty,CodecErrorV1>{match value{$($n=>Ok($variant),)+_=>Err(CodecErrorV1::Unsupported)}} }; }
tags!(phase_tag,read_phase,PhaseV1,{PhaseV1::Opening=>0,PhaseV1::Ready=>1,PhaseV1::TurnActive=>2,PhaseV1::AwaitingPermission=>3,PhaseV1::OutcomeUnknown=>4,PhaseV1::Closed=>5});
tags!(disposition_tag,read_disposition,DispositionV1,{DispositionV1::Completed=>0,DispositionV1::Failed=>1,DispositionV1::Cancelled=>2,DispositionV1::TimedOut=>3,DispositionV1::Uncertain=>4});
tags!(stop_tag,read_stop,StopReasonV1,{StopReasonV1::EndTurn=>0,StopReasonV1::Cancelled=>1,StopReasonV1::TimedOut=>2,StopReasonV1::Failed=>3,StopReasonV1::Disconnected=>4,StopReasonV1::OutcomeUnknown=>5});
tags!(option_kind_tag,read_option_kind,PermissionOptionKindV1,{PermissionOptionKindV1::AllowOnce=>0,PermissionOptionKindV1::AllowAlways=>1,PermissionOptionKindV1::RejectOnce=>2,PermissionOptionKindV1::RejectAlways=>3});
tags!(refusal_tag,read_refusal,RefusalV1,{RefusalV1::NotAuthorized=>0,RefusalV1::UnknownServiceSession=>1,RefusalV1::PrincipalMismatch=>2,RefusalV1::UnknownAgentSession=>3,RefusalV1::UnknownTurn=>4,RefusalV1::UnknownPermissionRequest=>5,RefusalV1::UnknownPermissionOption=>6,RefusalV1::RevisionConflict=>7,RefusalV1::AuthorityDenied=>8,RefusalV1::AuthorityExpired=>9,RefusalV1::AttemptDenied=>10,RefusalV1::IdempotencyConflict=>11,RefusalV1::SessionConflict=>12,RefusalV1::TurnConflict=>13,RefusalV1::ProviderReceiptMismatch=>14,RefusalV1::ProviderReceiptConflict=>15,RefusalV1::Busy=>16,RefusalV1::Capacity=>17,RefusalV1::Limit=>18,RefusalV1::Unsupported=>19,RefusalV1::InvalidInput=>20,RefusalV1::Closed=>21});
