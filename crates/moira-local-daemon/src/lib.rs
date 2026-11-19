//! Bounded length-prefixed local connection boundary for Moira.

use keystone_framing::FrameError;
use moira_protocol::{CommandV1, ReplyV1, SagaCommandV1, SagaReplyV1};
use moira_wire::{
    CodecErrorV1, LimitsV1, decode_request, decode_saga_request, encode_reply, encode_saga_reply,
};

pub fn serve_agent_up_to<IO, F, E>(
    io: &mut IO,
    limits: LimitsV1,
    max_requests: usize,
    mut execute: F,
) -> Result<usize, ConnectionErrorV1<E>>
where
    IO: Read + Write,
    F: FnMut(moira_protocol::agent::CommandV1) -> Result<moira_protocol::agent::ReplyV1, E>,
{
    serve_frames(
        io,
        limits,
        max_requests,
        &mut execute,
        moira_wire::decode_agent_request,
        moira_wire::encode_agent_reply,
    )
}

/// Durable agent-participation profile. Distinct frame kinds keep governed
/// orchestration commands separate from direct provider-session commands.
pub fn serve_agent_participation_up_to<IO, F, E>(
    io: &mut IO,
    limits: LimitsV1,
    max_requests: usize,
    mut execute: F,
) -> Result<usize, ConnectionErrorV1<E>>
where
    IO: Read + Write,
    F: FnMut(
        moira_protocol::agent_participation::CommandV1,
    ) -> Result<moira_protocol::agent_participation::ReplyV1, E>,
{
    serve_frames(
        io,
        limits,
        max_requests,
        &mut execute,
        moira_wire::decode_agent_participation_request,
        moira_wire::encode_agent_participation_reply,
    )
}

/// Methodical-deliberation profile V4. Its versioned, distinct frame kinds
/// prevent both V1 reinterpretation and crossing another Moira service family.
pub fn serve_deliberation_up_to<IO, F, E>(
    io: &mut IO,
    limits: LimitsV1,
    max_requests: usize,
    mut execute: F,
) -> Result<usize, ConnectionErrorV1<E>>
where
    IO: Read + Write,
    F: FnMut(
        moira_protocol::deliberation::CommandV1,
    ) -> Result<moira_protocol::deliberation::ReplyV1, E>,
{
    serve_frames(
        io,
        limits,
        max_requests,
        &mut execute,
        moira_wire::decode_deliberation_request,
        moira_wire::encode_deliberation_reply,
    )
}

/// Task-agent journey admission profile. Its distinct frame kind prevents a
/// preview or named-human instantiation from crossing another Moira service.
pub fn serve_task_agent_journey_up_to<IO, F, E>(
    io: &mut IO,
    limits: LimitsV1,
    max_requests: usize,
    mut execute: F,
) -> Result<usize, ConnectionErrorV1<E>>
where
    IO: Read + Write,
    F: FnMut(
        moira_protocol::task_agent_journey::CommandV1,
    ) -> Result<moira_protocol::task_agent_journey::ReplyV1, E>,
{
    serve_frames(
        io,
        limits,
        max_requests,
        &mut execute,
        moira_wire::decode_task_agent_journey_request,
        moira_wire::encode_task_agent_journey_reply,
    )
}
use std::io::{Read, Write};

#[cfg(unix)]
pub mod unix;

pub type ConnectionErrorV1<E> = keystone_framing::ConnectionError<CodecErrorV1, E>;

/// Serves a finite request sequence. Clean EOF between complete frames closes
/// successfully; partial headers or bodies remain pre-dispatch failures.
pub fn serve_up_to<IO, F, E>(
    io: &mut IO,
    limits: LimitsV1,
    max_requests: usize,
    mut execute: F,
) -> Result<usize, ConnectionErrorV1<E>>
where
    IO: Read + Write,
    F: FnMut(CommandV1) -> Result<ReplyV1, E>,
{
    serve_frames(
        io,
        limits,
        max_requests,
        &mut execute,
        decode_request,
        encode_reply,
    )
}

/// Saga-profile equivalent of [`serve_up_to`]. The distinct frame kind prevents
/// a legacy workflow request from being interpreted as a saga command.
pub fn serve_saga_up_to<IO, F, E>(
    io: &mut IO,
    limits: LimitsV1,
    max_requests: usize,
    mut execute: F,
) -> Result<usize, ConnectionErrorV1<E>>
where
    IO: Read + Write,
    F: FnMut(SagaCommandV1) -> Result<SagaReplyV1, E>,
{
    serve_frames(
        io,
        limits,
        max_requests,
        &mut execute,
        decode_saga_request,
        encode_saga_reply,
    )
}

enum LocalCommand {
    Workflow(CommandV1),
    Attempt(moira_protocol::attempt_lookup::AttemptLookupRequestV1),
}

enum LocalReply {
    Workflow(ReplyV1),
    Attempt(moira_protocol::attempt_lookup::AttemptLookup),
}

fn serve_local_up_to<IO, F, E>(
    io: &mut IO,
    limits: LimitsV1,
    max_requests: usize,
    mut execute: F,
) -> Result<usize, ConnectionErrorV1<E>>
where
    IO: Read + Write,
    F: FnMut(LocalCommand) -> Result<LocalReply, E>,
{
    serve_frames(
        io,
        limits,
        max_requests,
        &mut execute,
        |bytes, limits| match decode_request(bytes, limits) {
            Ok(value) => Ok(LocalCommand::Workflow(value)),
            Err(CodecErrorV1::Unsupported) => {
                moira_wire::decode_attempt_request(bytes, limits).map(LocalCommand::Attempt)
            }
            Err(error) => Err(error),
        },
        |reply, limits| match reply {
            LocalReply::Workflow(reply) => encode_reply(reply, limits),
            LocalReply::Attempt(reply) => moira_wire::encode_attempt_reply(reply, limits),
        },
    )
}

fn serve_frames<IO, F, E, C, R>(
    io: &mut IO,
    limits: LimitsV1,
    max_requests: usize,
    execute: &mut F,
    decode: fn(&[u8], LimitsV1) -> Result<C, CodecErrorV1>,
    encode: fn(&R, LimitsV1) -> Result<Vec<u8>, CodecErrorV1>,
) -> Result<usize, ConnectionErrorV1<E>>
where
    IO: Read + Write,
    F: FnMut(C) -> Result<R, E>,
{
    keystone_framing::serve_up_to(
        io,
        limits.max_frame_bytes,
        max_requests,
        |bytes| decode(bytes, limits),
        execute,
        |reply| encode(reply, limits),
        map_frame_error,
    )
}

fn map_frame_error(error: FrameError) -> CodecErrorV1 {
    match error {
        FrameError::Capacity => CodecErrorV1::Capacity,
        FrameError::RequestLimit | FrameError::FrameLimit => CodecErrorV1::Limit,
    }
}
