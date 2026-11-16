use moira_local_daemon::{ConnectionErrorV1, serve_agent_up_to, serve_saga_up_to, serve_up_to};
use moira_protocol::{
    CommandV1, OutcomeV1, ReplyV1, SagaCommandV1, SagaOutcomeV1, SagaReplyV1, SagaStatusV1,
    StatusV1,
};
use moira_wire::{
    LimitsV1, decode_agent_reply, decode_reply, decode_saga_reply, encode_agent_request,
    encode_request, encode_saga_request,
};
use std::io::{Cursor, Read, Result as IoResult, Write};

struct Duplex {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
    fail_write: bool,
}
impl Read for Duplex {
    fn read(&mut self, bytes: &mut [u8]) -> IoResult<usize> {
        self.input.read(bytes)
    }
}
impl Write for Duplex {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        if self.fail_write {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        self.output.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}
fn framed(command: CommandV1) -> Vec<u8> {
    let bytes = encode_request(&command, LimitsV1::default()).unwrap();
    [(bytes.len() as u32).to_be_bytes().as_slice(), &bytes].concat()
}
fn framed_saga(command: SagaCommandV1) -> Vec<u8> {
    let bytes = encode_saga_request(&command, LimitsV1::default()).unwrap();
    [(bytes.len() as u32).to_be_bytes().as_slice(), &bytes].concat()
}
fn framed_agent(command: moira_protocol::agent::CommandV1) -> Vec<u8> {
    let bytes = encode_agent_request(&command, LimitsV1::default()).unwrap();
    [(bytes.len() as u32).to_be_bytes().as_slice(), &bytes].concat()
}

#[test]
fn bounded_sequence_accepts_clean_eof_and_writes_exact_replies() {
    let input = [framed(CommandV1::Status), framed(CommandV1::Status)].concat();
    let mut io = Duplex {
        input: Cursor::new(input),
        output: Vec::new(),
        fail_write: false,
    };
    let reply = ReplyV1::Accepted(OutcomeV1::Status(StatusV1 {
        revision: 3,
        needs_reopen: false,
    }));
    let mut calls = 0;
    assert_eq!(
        serve_up_to(&mut io, LimitsV1::default(), 8, |_| {
            calls += 1;
            Ok::<_, ()>(reply)
        })
        .unwrap(),
        2
    );
    assert_eq!(calls, 2);
    let first = u32::from_be_bytes(io.output[..4].try_into().unwrap()) as usize;
    assert_eq!(
        decode_reply(&io.output[4..4 + first], LimitsV1::default()),
        Ok(reply)
    );
}

#[test]
fn reply_loss_is_outcome_unknown_after_one_dispatch() {
    let mut io = Duplex {
        input: Cursor::new(framed(CommandV1::Status)),
        output: Vec::new(),
        fail_write: true,
    };
    let mut calls = 0;
    assert!(matches!(
        serve_up_to(&mut io, LimitsV1::default(), 1, |_| {
            calls += 1;
            Ok::<_, ()>(ReplyV1::Refused(moira_protocol::RefusalV1::NotAuthorized))
        }),
        Err(ConnectionErrorV1::OutcomeUnknown(_))
    ));
    assert_eq!(calls, 1);
}

#[test]
fn saga_sequence_preserves_bounded_connection_semantics() {
    let input = [
        framed_saga(SagaCommandV1::Status),
        framed_saga(SagaCommandV1::Census),
    ]
    .concat();
    let mut io = Duplex {
        input: Cursor::new(input),
        output: Vec::new(),
        fail_write: false,
    };
    let mut calls = 0;
    assert_eq!(
        serve_saga_up_to(&mut io, LimitsV1::default(), 2, |command| {
            calls += 1;
            Ok::<_, ()>(match command {
                SagaCommandV1::Status => {
                    SagaReplyV1::Accepted(SagaOutcomeV1::Status(SagaStatusV1 {
                        revision: 4,
                        needs_reopen: false,
                    }))
                }
                SagaCommandV1::Census => {
                    SagaReplyV1::Refused(moira_protocol::SagaRefusalV1::NotAuthorized)
                }
                _ => unreachable!(),
            })
        })
        .unwrap(),
        2
    );
    assert_eq!(calls, 2);
    let first = u32::from_be_bytes(io.output[..4].try_into().unwrap()) as usize;
    assert!(matches!(
        decode_saga_reply(&io.output[4..4 + first], LimitsV1::default()),
        Ok(SagaReplyV1::Accepted(SagaOutcomeV1::Status(_)))
    ));
}

#[test]
fn saga_reply_loss_is_outcome_unknown_after_one_dispatch() {
    let mut io = Duplex {
        input: Cursor::new(framed_saga(SagaCommandV1::Status)),
        output: Vec::new(),
        fail_write: true,
    };
    let mut calls = 0;
    assert!(matches!(
        serve_saga_up_to(&mut io, LimitsV1::default(), 1, |_| {
            calls += 1;
            Ok::<_, ()>(SagaReplyV1::Refused(
                moira_protocol::SagaRefusalV1::NotAuthorized,
            ))
        }),
        Err(ConnectionErrorV1::OutcomeUnknown(_))
    ));
    assert_eq!(calls, 1);
}

#[test]
fn agent_sequence_preserves_profile_and_reply_loss_boundaries() {
    use moira_protocol::agent::{CommandV1, RefusalV1, ReplyV1, SessionIdV1};
    let command = CommandV1::Inspect {
        session: SessionIdV1([1; 16]),
    };
    let mut io = Duplex {
        input: Cursor::new(framed_agent(command)),
        output: Vec::new(),
        fail_write: false,
    };
    assert_eq!(
        serve_agent_up_to(&mut io, LimitsV1::default(), 1, |_| Ok::<_, ()>(
            ReplyV1::Refused(RefusalV1::NotAuthorized)
        ))
        .unwrap(),
        1
    );
    let length = u32::from_be_bytes(io.output[..4].try_into().unwrap()) as usize;
    assert_eq!(
        decode_agent_reply(&io.output[4..4 + length], LimitsV1::default()),
        Ok(ReplyV1::Refused(RefusalV1::NotAuthorized))
    );

    let mut lost = Duplex {
        input: Cursor::new(framed_agent(CommandV1::Inspect {
            session: SessionIdV1([1; 16]),
        })),
        output: Vec::new(),
        fail_write: true,
    };
    let mut calls = 0;
    assert!(matches!(
        serve_agent_up_to(&mut lost, LimitsV1::default(), 1, |_| {
            calls += 1;
            Ok::<_, ()>(ReplyV1::Refused(RefusalV1::NotAuthorized))
        }),
        Err(ConnectionErrorV1::OutcomeUnknown(_))
    ));
    assert_eq!(calls, 1);
}
