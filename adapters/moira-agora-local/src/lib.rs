//! Private Unix transport for Agora's public V2 client contract.
//!
//! This package owns only handshake, framing and delivery-phase truth. Agora
//! continues to own authentication, participant sessions, conversation
//! semantics and replay. A server-issued client identity is never inferred or
//! configured by Moira.

#![forbid(unsafe_code)]

use agora_client::v2::{PortFailureV2, ServicePortV2};
use agora_protocol::v2::{ClientIdV2, ReplyV2, RequestV2};
use agora_wire::{
    WireLimits,
    local_handshake::{
        SERVER_HELLO_BYTES_V1, ServerIncarnationV1, decode_server_hello_v1, encode_client_hello_v1,
    },
    v2::{WireReplyV2, WireRequestV2, decode_reply_v2, encode_request_v2},
};
use std::{
    io::{self, ErrorKind, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgoraUnixLimitsV1 {
    pub wire: WireLimits,
    pub timeout: Duration,
}

impl Default for AgoraUnixLimitsV1 {
    fn default() -> Self {
        Self {
            wire: WireLimits::reference(),
            timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgoraUnixErrorV1 {
    InvalidLimits,
    NotConnected(ErrorKind),
    Handshake,
    Wire,
    FrameLimit,
    FrameBudget,
    ClientMismatch,
    UnexpectedReply,
    Io(ErrorKind),
}

pub struct AgoraUnixPortV1 {
    stream: UnixStream,
    client: ClientIdV2,
    incarnation: ServerIncarnationV1,
    wire: WireLimits,
    max_frame_bytes: usize,
    remaining_frames: usize,
}

impl AgoraUnixPortV1 {
    pub const fn client(&self) -> ClientIdV2 {
        self.client
    }

    pub const fn server_incarnation(&self) -> ServerIncarnationV1 {
        self.incarnation
    }

    pub const fn remaining_frames(&self) -> usize {
        self.remaining_frames
    }

    pub fn disconnect(mut self) -> Result<(), AgoraUnixErrorV1> {
        let bytes = encode_request_v2(&WireRequestV2::Disconnect(self.client), self.wire)
            .map_err(|_| AgoraUnixErrorV1::Wire)?;
        self.send_frame(&bytes).map_err(|failure| failure.error)?;
        match self.read_reply()? {
            WireReplyV2::Disconnected => Ok(()),
            _ => Err(AgoraUnixErrorV1::UnexpectedReply),
        }
    }

    fn send_frame(&mut self, bytes: &[u8]) -> Result<(), WriteFailureV1> {
        if self.remaining_frames == 0 {
            return Err(WriteFailureV1 {
                sent: false,
                error: AgoraUnixErrorV1::FrameBudget,
            });
        }
        if bytes.is_empty() || bytes.len() > self.max_frame_bytes {
            return Err(WriteFailureV1 {
                sent: false,
                error: AgoraUnixErrorV1::FrameLimit,
            });
        }
        let length = u32::try_from(bytes.len()).map_err(|_| WriteFailureV1 {
            sent: false,
            error: AgoraUnixErrorV1::FrameLimit,
        })?;
        let mut sent = 0_usize;
        write_phase(&mut self.stream, &length.to_be_bytes(), &mut sent)?;
        write_phase(&mut self.stream, bytes, &mut sent)?;
        self.stream.flush().map_err(|error| WriteFailureV1 {
            sent: true,
            error: AgoraUnixErrorV1::Io(error.kind()),
        })?;
        self.remaining_frames -= 1;
        Ok(())
    }

    fn read_reply(&mut self) -> Result<WireReplyV2, AgoraUnixErrorV1> {
        let bytes = read_frame(&mut self.stream, self.max_frame_bytes)
            .map_err(|error| AgoraUnixErrorV1::Io(error.kind()))?;
        decode_reply_v2(&bytes, self.wire).map_err(|_| AgoraUnixErrorV1::Wire)
    }
}

impl ServicePortV2 for AgoraUnixPortV1 {
    type Error = AgoraUnixErrorV1;

    fn execute(
        &mut self,
        client: ClientIdV2,
        request: RequestV2,
    ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
        if client != self.client {
            return Err(PortFailureV2::NotSent(AgoraUnixErrorV1::ClientMismatch));
        }
        let bytes = encode_request_v2(&WireRequestV2::Execute { client, request }, self.wire)
            .map_err(|_| PortFailureV2::NotSent(AgoraUnixErrorV1::Wire))?;
        self.send_frame(&bytes).map_err(|failure| {
            if failure.sent {
                PortFailureV2::OutcomeUnknown(failure.error)
            } else {
                PortFailureV2::NotSent(failure.error)
            }
        })?;
        match self.read_reply().map_err(PortFailureV2::OutcomeUnknown)? {
            WireReplyV2::Executed(reply) => Ok(reply),
            _ => Err(PortFailureV2::OutcomeUnknown(
                AgoraUnixErrorV1::UnexpectedReply,
            )),
        }
    }
}

pub fn connect_agora_v2(
    path: impl AsRef<Path>,
    participant: &str,
    limits: AgoraUnixLimitsV1,
) -> Result<(ClientIdV2, AgoraUnixPortV1), AgoraUnixErrorV1> {
    validate_limits(limits)?;
    let stream =
        UnixStream::connect(path).map_err(|error| AgoraUnixErrorV1::NotConnected(error.kind()))?;
    establish(stream, participant, limits)
}

fn establish(
    mut stream: UnixStream,
    participant: &str,
    limits: AgoraUnixLimitsV1,
) -> Result<(ClientIdV2, AgoraUnixPortV1), AgoraUnixErrorV1> {
    validate_limits(limits)?;
    stream
        .set_read_timeout(Some(limits.timeout))
        .and_then(|()| stream.set_write_timeout(Some(limits.timeout)))
        .map_err(|error| AgoraUnixErrorV1::NotConnected(error.kind()))?;
    stream
        .write_all(&encode_client_hello_v1())
        .and_then(|()| stream.flush())
        .map_err(|error| AgoraUnixErrorV1::NotConnected(error.kind()))?;
    let mut hello = [0_u8; SERVER_HELLO_BYTES_V1];
    stream
        .read_exact(&mut hello)
        .map_err(|_| AgoraUnixErrorV1::Handshake)?;
    let hello = decode_server_hello_v1(&hello).map_err(|_| AgoraUnixErrorV1::Handshake)?;
    let max_frame_bytes = usize::try_from(hello.max_frame_bytes())
        .ok()
        .map(|server| server.min(limits.wire.max_frame_bytes))
        .filter(|maximum| *maximum > 0)
        .ok_or(AgoraUnixErrorV1::Handshake)?;
    let remaining_frames = usize::try_from(hello.max_frames_after_connect())
        .ok()
        .filter(|maximum| *maximum > 0)
        .ok_or(AgoraUnixErrorV1::Handshake)?;
    let connect = encode_request_v2(
        &WireRequestV2::Connect {
            participant_id: participant.to_owned(),
        },
        limits.wire,
    )
    .map_err(|_| AgoraUnixErrorV1::Wire)?;
    write_frame(&mut stream, &connect, max_frame_bytes)
        .map_err(|error| AgoraUnixErrorV1::Io(error.kind()))?;
    let reply = read_frame(&mut stream, max_frame_bytes)
        .map_err(|error| AgoraUnixErrorV1::Io(error.kind()))?;
    let WireReplyV2::Connected(client) =
        decode_reply_v2(&reply, limits.wire).map_err(|_| AgoraUnixErrorV1::Wire)?
    else {
        return Err(AgoraUnixErrorV1::UnexpectedReply);
    };
    Ok((
        client,
        AgoraUnixPortV1 {
            stream,
            client,
            incarnation: hello.server_incarnation(),
            wire: limits.wire,
            max_frame_bytes,
            remaining_frames,
        },
    ))
}

fn validate_limits(limits: AgoraUnixLimitsV1) -> Result<(), AgoraUnixErrorV1> {
    if limits.timeout.is_zero()
        || limits.wire.max_frame_bytes == 0
        || limits.wire.max_frame_bytes > u32::MAX as usize
        || limits.wire.max_text_bytes == 0
        || limits.wire.max_fields == 0
        || limits.wire.max_list_items == 0
    {
        return Err(AgoraUnixErrorV1::InvalidLimits);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct WriteFailureV1 {
    sent: bool,
    error: AgoraUnixErrorV1,
}

fn write_phase(
    stream: &mut UnixStream,
    mut bytes: &[u8],
    sent: &mut usize,
) -> Result<(), WriteFailureV1> {
    while !bytes.is_empty() {
        match stream.write(bytes) {
            Ok(0) => {
                return Err(WriteFailureV1 {
                    sent: *sent > 0,
                    error: AgoraUnixErrorV1::Io(ErrorKind::WriteZero),
                });
            }
            Ok(count) => {
                *sent = sent.saturating_add(count);
                bytes = &bytes[count..];
            }
            Err(error) => {
                return Err(WriteFailureV1 {
                    sent: *sent > 0,
                    error: AgoraUnixErrorV1::Io(error.kind()),
                });
            }
        }
    }
    Ok(())
}

fn write_frame(stream: &mut UnixStream, bytes: &[u8], maximum: usize) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > maximum {
        return Err(io::Error::new(ErrorKind::InvalidData, "frame limit"));
    }
    let length = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(ErrorKind::InvalidData, "frame length"))?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(bytes)?;
    stream.flush()
}

fn read_frame(stream: &mut UnixStream, maximum: usize) -> io::Result<Vec<u8>> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header)?;
    let length = usize::try_from(u32::from_be_bytes(header))
        .map_err(|_| io::Error::new(ErrorKind::InvalidData, "frame length"))?;
    if length == 0 || length > maximum {
        return Err(io::Error::new(ErrorKind::InvalidData, "frame limit"));
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
    use agora_protocol::v2::{CommandV2, RefusalV2, ReplyV2, RequestIdV2};
    use agora_wire::{
        local_handshake::{CLIENT_HELLO_BYTES_V1, decode_client_hello_v1, encode_server_hello_v1},
        v2::{WireRequestV2, decode_request_v2, encode_reply_v2},
    };
    use std::thread;

    #[test]
    fn invalid_limits_refuse_before_transport() {
        let limits = AgoraUnixLimitsV1 {
            timeout: Duration::ZERO,
            ..AgoraUnixLimitsV1::default()
        };
        assert!(matches!(
            connect_agora_v2("/not/contacted", "bot", limits),
            Err(AgoraUnixErrorV1::InvalidLimits)
        ));
    }

    #[test]
    fn public_client_port_negotiates_server_identity_and_exact_frames() {
        let wire = WireLimits {
            max_frame_bytes: 4_096,
            max_text_bytes: 128,
            max_fields: 512,
            max_list_items: 32,
        };
        let (client_stream, mut server_stream) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || {
            let mut hello = [0_u8; CLIENT_HELLO_BYTES_V1];
            server_stream.read_exact(&mut hello).unwrap();
            decode_client_hello_v1(&hello).unwrap();
            server_stream
                .write_all(
                    &encode_server_hello_v1(ServerIncarnationV1::new([7; 16]), 4_096, 3).unwrap(),
                )
                .unwrap();
            let connect = read_frame(&mut server_stream, 4_096).unwrap();
            assert_eq!(
                decode_request_v2(&connect, wire).unwrap(),
                WireRequestV2::Connect {
                    participant_id: "bot".into()
                }
            );
            write_frame(
                &mut server_stream,
                &encode_reply_v2(&WireReplyV2::Connected(ClientIdV2(9)), wire).unwrap(),
                4_096,
            )
            .unwrap();
            let execute = read_frame(&mut server_stream, 4_096).unwrap();
            assert_eq!(
                decode_request_v2(&execute, wire).unwrap(),
                WireRequestV2::Execute {
                    client: ClientIdV2(9),
                    request: RequestV2 {
                        request_id: RequestIdV2(1),
                        command: CommandV2::Capabilities,
                    },
                }
            );
            write_frame(
                &mut server_stream,
                &encode_reply_v2(
                    &WireReplyV2::Executed(ReplyV2::Refused(RefusalV2::NotAuthorized)),
                    wire,
                )
                .unwrap(),
                4_096,
            )
            .unwrap();
            let disconnect = read_frame(&mut server_stream, 4_096).unwrap();
            assert_eq!(
                decode_request_v2(&disconnect, wire).unwrap(),
                WireRequestV2::Disconnect(ClientIdV2(9))
            );
            write_frame(
                &mut server_stream,
                &encode_reply_v2(&WireReplyV2::Disconnected, wire).unwrap(),
                4_096,
            )
            .unwrap();
        });

        let (client, mut port) = establish(
            client_stream,
            "bot",
            AgoraUnixLimitsV1 {
                wire,
                timeout: Duration::from_secs(1),
            },
        )
        .unwrap();
        assert_eq!(client, ClientIdV2(9));
        assert_eq!(port.client(), client);
        assert_eq!(port.server_incarnation().as_bytes(), [7; 16]);
        assert_eq!(port.remaining_frames(), 3);
        assert_eq!(
            port.execute(
                ClientIdV2(8),
                RequestV2 {
                    request_id: RequestIdV2(0),
                    command: CommandV2::Capabilities,
                }
            ),
            Err(PortFailureV2::NotSent(AgoraUnixErrorV1::ClientMismatch))
        );
        assert_eq!(port.remaining_frames(), 3);
        assert_eq!(
            port.execute(
                client,
                RequestV2 {
                    request_id: RequestIdV2(1),
                    command: CommandV2::Capabilities,
                }
            ),
            Ok(ReplyV2::Refused(RefusalV2::NotAuthorized))
        );
        assert_eq!(port.remaining_frames(), 2);
        port.disconnect().unwrap();
        server.join().unwrap();
    }
}
