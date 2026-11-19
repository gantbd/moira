//! Bounded private Unix transport for Moira's governed participation service.

use crate::{
    AgentParticipationServicePort, DeliberationServicePort, SagaServicePort,
    TaskAgentJourneyServicePort,
};
use moira_protocol::agent_participation as participation;
use moira_protocol::deliberation;
use moira_protocol::task_agent_journey as journey;
use moira_protocol::{SagaCommandV1, SagaReplyV1};
use moira_wire::{
    LimitsV1, decode_agent_participation_reply, decode_deliberation_reply, decode_ready,
    decode_saga_reply, decode_task_agent_journey_reply, encode_agent_participation_request,
    encode_deliberation_request, encode_saga_request, encode_task_agent_journey_request,
};
use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParticipationClientLimitsV1 {
    pub max_frame_bytes: usize,
    pub max_requests: usize,
    pub io_timeout: Duration,
}

impl Default for ParticipationClientLimitsV1 {
    fn default() -> Self {
        Self {
            max_frame_bytes: 64 * 1024,
            max_requests: 1_024,
            io_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParticipationClientErrorV1 {
    InvalidLimits,
    NotSent(io::ErrorKind),
    OutcomeUnknown(io::ErrorKind),
    InvalidReady,
    InvalidReply,
    RequestLimit,
}

pub type DeliberationClientLimitsV1 = ParticipationClientLimitsV1;
pub type DeliberationClientErrorV1 = ParticipationClientErrorV1;
pub type SagaClientLimitsV1 = ParticipationClientLimitsV1;
pub type SagaClientErrorV1 = ParticipationClientErrorV1;
pub type TaskAgentJourneyClientLimitsV1 = ParticipationClientLimitsV1;
pub type TaskAgentJourneyClientErrorV1 = ParticipationClientErrorV1;

/// Reconnectable public Unix port for Moira's closed saga service.
///
/// The total request budget survives reconnects. Once transmission starts, a
/// failed exchange consumes capacity and is reported as outcome-unknown.
pub struct SagaUnixPort {
    endpoint: PathBuf,
    client_limits: SagaClientLimitsV1,
    stream: Option<UnixStream>,
    wire_limits: LimitsV1,
    requests_left: usize,
    connection_requests_left: usize,
}

impl SagaUnixPort {
    pub fn connect(
        path: impl AsRef<Path>,
        limits: SagaClientLimitsV1,
    ) -> Result<Self, SagaClientErrorV1> {
        let endpoint = path.as_ref().to_path_buf();
        let negotiated = connect(&endpoint, limits)?;
        Ok(Self {
            endpoint,
            client_limits: limits,
            stream: Some(negotiated.stream),
            wire_limits: negotiated.wire_limits,
            requests_left: limits.max_requests,
            connection_requests_left: negotiated.requests_left,
        })
    }

    fn ensure_connected(&mut self) -> Result<(), SagaClientErrorV1> {
        if self.stream.is_some() && self.connection_requests_left > 0 {
            return Ok(());
        }
        let negotiated = connect(&self.endpoint, self.client_limits)?;
        self.stream = Some(negotiated.stream);
        self.wire_limits = negotiated.wire_limits;
        self.connection_requests_left = negotiated.requests_left;
        Ok(())
    }

    fn invalidate(&mut self) {
        self.stream = None;
        self.connection_requests_left = 0;
    }
}

impl SagaServicePort for SagaUnixPort {
    type Error = SagaClientErrorV1;

    fn execute(&mut self, command: SagaCommandV1) -> Result<SagaReplyV1, Self::Error> {
        if self.requests_left == 0 {
            return Err(SagaClientErrorV1::RequestLimit);
        }
        self.ensure_connected()?;
        let bytes = encode_saga_request(&command, self.wire_limits)
            .map_err(|_| SagaClientErrorV1::NotSent(io::ErrorKind::InvalidData))?;
        // Consume both limits before transmission. A failed write can still
        // mean that the owner observed the command.
        self.requests_left -= 1;
        self.connection_requests_left -= 1;
        let stream = self.stream.as_mut().expect("connected saga port");
        if let Err(error) = write_frame(stream, &bytes) {
            self.invalidate();
            return Err(SagaClientErrorV1::OutcomeUnknown(error.kind()));
        }
        let stream = self.stream.as_mut().expect("connected saga port");
        let reply = match read_frame(stream, self.wire_limits.max_frame_bytes) {
            Ok(reply) => reply,
            Err(error) => {
                self.invalidate();
                return Err(SagaClientErrorV1::OutcomeUnknown(error.kind()));
            }
        };
        decode_saga_reply(&reply, self.wire_limits).map_err(|_| {
            self.invalidate();
            SagaClientErrorV1::InvalidReply
        })
    }
}

pub struct AgentParticipationUnixPort {
    stream: UnixStream,
    wire_limits: LimitsV1,
    requests_left: usize,
}

impl AgentParticipationUnixPort {
    pub fn connect(
        path: impl AsRef<Path>,
        limits: ParticipationClientLimitsV1,
    ) -> Result<Self, ParticipationClientErrorV1> {
        let negotiated = connect(path, limits)?;
        Ok(Self {
            stream: negotiated.stream,
            wire_limits: negotiated.wire_limits,
            requests_left: negotiated.requests_left,
        })
    }
}

impl AgentParticipationServicePort for AgentParticipationUnixPort {
    type Error = ParticipationClientErrorV1;

    fn execute_participation(
        &mut self,
        command: participation::CommandV1,
    ) -> Result<participation::ReplyV1, Self::Error> {
        if self.requests_left == 0 {
            return Err(ParticipationClientErrorV1::RequestLimit);
        }
        let bytes = encode_agent_participation_request(&command, self.wire_limits)
            .map_err(|_| ParticipationClientErrorV1::NotSent(io::ErrorKind::InvalidData))?;
        write_frame(&mut self.stream, &bytes)
            .map_err(|error| ParticipationClientErrorV1::OutcomeUnknown(error.kind()))?;
        self.requests_left -= 1;
        let reply = read_frame(&mut self.stream, self.wire_limits.max_frame_bytes)
            .map_err(|error| ParticipationClientErrorV1::OutcomeUnknown(error.kind()))?;
        decode_agent_participation_reply(&reply, self.wire_limits)
            .map_err(|_| ParticipationClientErrorV1::InvalidReply)
    }
}

pub struct DeliberationUnixPort {
    stream: UnixStream,
    wire_limits: LimitsV1,
    requests_left: usize,
}

impl DeliberationUnixPort {
    pub fn connect(
        path: impl AsRef<Path>,
        limits: DeliberationClientLimitsV1,
    ) -> Result<Self, DeliberationClientErrorV1> {
        let negotiated = connect(path, limits)?;
        Ok(Self {
            stream: negotiated.stream,
            wire_limits: negotiated.wire_limits,
            requests_left: negotiated.requests_left,
        })
    }
}

impl DeliberationServicePort for DeliberationUnixPort {
    type Error = DeliberationClientErrorV1;

    fn execute(
        &mut self,
        command: deliberation::CommandV1,
    ) -> Result<deliberation::ReplyV1, Self::Error> {
        if self.requests_left == 0 {
            return Err(ParticipationClientErrorV1::RequestLimit);
        }
        let bytes = encode_deliberation_request(&command, self.wire_limits)
            .map_err(|_| ParticipationClientErrorV1::NotSent(io::ErrorKind::InvalidData))?;
        write_frame(&mut self.stream, &bytes)
            .map_err(|error| ParticipationClientErrorV1::OutcomeUnknown(error.kind()))?;
        self.requests_left -= 1;
        let reply = read_frame(&mut self.stream, self.wire_limits.max_frame_bytes)
            .map_err(|error| ParticipationClientErrorV1::OutcomeUnknown(error.kind()))?;
        decode_deliberation_reply(&reply, self.wire_limits)
            .map_err(|_| ParticipationClientErrorV1::InvalidReply)
    }
}

pub struct TaskAgentJourneyUnixPort {
    stream: UnixStream,
    wire_limits: LimitsV1,
    requests_left: usize,
}

impl TaskAgentJourneyUnixPort {
    pub fn connect(
        path: impl AsRef<Path>,
        limits: TaskAgentJourneyClientLimitsV1,
    ) -> Result<Self, TaskAgentJourneyClientErrorV1> {
        let negotiated = connect(path, limits)?;
        Ok(Self {
            stream: negotiated.stream,
            wire_limits: negotiated.wire_limits,
            requests_left: negotiated.requests_left,
        })
    }
}

impl TaskAgentJourneyServicePort for TaskAgentJourneyUnixPort {
    type Error = TaskAgentJourneyClientErrorV1;

    fn execute_task_agent_journey(
        &mut self,
        command: journey::CommandV1,
    ) -> Result<journey::ReplyV1, Self::Error> {
        if self.requests_left == 0 {
            return Err(TaskAgentJourneyClientErrorV1::RequestLimit);
        }
        let bytes = encode_task_agent_journey_request(&command, self.wire_limits)
            .map_err(|_| TaskAgentJourneyClientErrorV1::NotSent(io::ErrorKind::InvalidData))?;
        write_frame(&mut self.stream, &bytes)
            .map_err(|error| TaskAgentJourneyClientErrorV1::OutcomeUnknown(error.kind()))?;
        self.requests_left -= 1;
        let reply = read_frame(&mut self.stream, self.wire_limits.max_frame_bytes)
            .map_err(|error| TaskAgentJourneyClientErrorV1::OutcomeUnknown(error.kind()))?;
        decode_task_agent_journey_reply(&reply, self.wire_limits)
            .map_err(|_| TaskAgentJourneyClientErrorV1::InvalidReply)
    }
}

struct NegotiatedUnixPort {
    stream: UnixStream,
    wire_limits: LimitsV1,
    requests_left: usize,
}

fn connect(
    path: impl AsRef<Path>,
    limits: ParticipationClientLimitsV1,
) -> Result<NegotiatedUnixPort, ParticipationClientErrorV1> {
    if limits.max_frame_bytes == 0 || limits.max_requests == 0 || limits.io_timeout.is_zero() {
        return Err(ParticipationClientErrorV1::InvalidLimits);
    }
    let mut stream = UnixStream::connect(path)
        .map_err(|error| ParticipationClientErrorV1::NotSent(error.kind()))?;
    stream
        .set_read_timeout(Some(limits.io_timeout))
        .and_then(|()| stream.set_write_timeout(Some(limits.io_timeout)))
        .map_err(|error| ParticipationClientErrorV1::NotSent(error.kind()))?;
    let local = LimitsV1 {
        max_frame_bytes: limits.max_frame_bytes,
    };
    let ready = read_frame(&mut stream, limits.max_frame_bytes)
        .map_err(|error| ParticipationClientErrorV1::NotSent(error.kind()))?;
    let ready =
        decode_ready(&ready, local).map_err(|_| ParticipationClientErrorV1::InvalidReady)?;
    let max_frame_bytes = limits
        .max_frame_bytes
        .min(usize::try_from(ready.max_frame_bytes).unwrap_or(usize::MAX));
    let requests_left = limits
        .max_requests
        .min(usize::try_from(ready.max_requests).unwrap_or(usize::MAX));
    if max_frame_bytes == 0 || requests_left == 0 {
        return Err(ParticipationClientErrorV1::InvalidReady);
    }
    Ok(NegotiatedUnixPort {
        stream,
        wire_limits: LimitsV1 { max_frame_bytes },
        requests_left,
    })
}

fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length"))?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(bytes)?;
    stream.flush()
}

fn read_frame(stream: &mut UnixStream, maximum: usize) -> io::Result<Vec<u8>> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header)?;
    let length = usize::try_from(u32::from_be_bytes(header))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length"))?;
    if length == 0 || length > maximum {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame limit"));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| io::Error::other("frame capacity"))?;
    bytes.resize(length, 0);
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moira_protocol::agent_participation::{OutcomeV1, ReplyV1, StatusV1};
    use moira_wire::{decode_agent_participation_request, encode_agent_participation_reply};
    use std::thread;

    #[test]
    fn bounded_port_roundtrips_only_the_participation_family() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let limits = LimitsV1::default();
        let worker = thread::spawn(move || {
            let request = read_frame(&mut server, limits.max_frame_bytes).unwrap();
            assert_eq!(
                decode_agent_participation_request(&request, limits).unwrap(),
                participation::CommandV1::Status
            );
            let reply = encode_agent_participation_reply(
                &ReplyV1::Accepted(Box::new(OutcomeV1::Status(StatusV1 {
                    storage_revision: Some(7),
                    needs_reopen: false,
                }))),
                limits,
            )
            .unwrap();
            write_frame(&mut server, &reply).unwrap();
        });
        let mut port = AgentParticipationUnixPort {
            stream: client,
            wire_limits: limits,
            requests_left: 1,
        };
        assert!(matches!(
            port.execute_participation(participation::CommandV1::Status),
            Ok(ReplyV1::Accepted(_))
        ));
        assert_eq!(
            port.execute_participation(participation::CommandV1::Status),
            Err(ParticipationClientErrorV1::RequestLimit)
        );
        worker.join().unwrap();
    }

    #[test]
    fn reply_loss_after_send_is_outcome_unknown() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let limits = LimitsV1::default();
        let worker = thread::spawn(move || {
            let request = read_frame(&mut server, limits.max_frame_bytes).unwrap();
            assert!(decode_agent_participation_request(&request, limits).is_ok());
        });
        let mut port = AgentParticipationUnixPort {
            stream: client,
            wire_limits: limits,
            requests_left: 1,
        };
        assert!(matches!(
            port.execute_participation(participation::CommandV1::Status),
            Err(ParticipationClientErrorV1::OutcomeUnknown(_))
        ));
        worker.join().unwrap();
    }
}
