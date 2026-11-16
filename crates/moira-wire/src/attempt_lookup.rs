//! Separate frame kinds preserve the closed workflow request profile.
use super::{CodecErrorV1, LimitsV1, Reader, Writer, disposition_tag, read_disposition};
use moira_protocol::{
    WorkflowIdV1,
    attempt_lookup::{
        AttemptLookup, AttemptLookupRequestV1, ExecutionEvidence, MAX_SCOPE_BYTES, MAX_TASK_BYTES,
    },
};

const REQUEST: u8 = 6;
const REPLY: u8 = 7;

pub fn encode_attempt_request(
    value: &AttemptLookupRequestV1,
    limits: LimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    if value.attempt == 0 {
        return Err(CodecErrorV1::Malformed);
    }
    let mut writer = Writer::new(REQUEST, limits)?;
    for (field, max) in [
        (&value.service_scope, MAX_SCOPE_BYTES),
        (&value.task, MAX_TASK_BYTES),
    ] {
        if !(1..=max).contains(&field.len()) {
            return Err(CodecErrorV1::Limit);
        }
        writer.u32(field.len() as u32)?;
        writer.raw(field.as_bytes())?;
    }
    writer.u64(value.incarnation)?;
    writer.u32(value.attempt)?;
    Ok(writer.bytes)
}

pub fn decode_attempt_request(
    bytes: &[u8],
    limits: LimitsV1,
) -> Result<AttemptLookupRequestV1, CodecErrorV1> {
    fn field(reader: &mut Reader<'_>, max: usize) -> Result<String, CodecErrorV1> {
        let length = reader.u32()? as usize;
        if !(1..=max).contains(&length) {
            return Err(CodecErrorV1::Limit);
        }
        let end = reader
            .position
            .checked_add(length)
            .ok_or(CodecErrorV1::Malformed)?;
        let bytes = reader
            .bytes
            .get(reader.position..end)
            .ok_or(CodecErrorV1::Malformed)?;
        reader.position = end;
        Ok(std::str::from_utf8(bytes)
            .map_err(|_| CodecErrorV1::Malformed)?
            .to_owned())
    }
    let mut reader = Reader::new(bytes, REQUEST, limits)?;
    let value = AttemptLookupRequestV1 {
        service_scope: field(&mut reader, MAX_SCOPE_BYTES)?,
        task: field(&mut reader, MAX_TASK_BYTES)?,
        incarnation: reader.u64()?,
        attempt: reader.u32()?,
    };
    reader.finish()?;
    if value.attempt == 0 {
        return Err(CodecErrorV1::Malformed);
    }
    Ok(value)
}

pub fn encode_attempt_reply(
    value: &AttemptLookup,
    limits: LimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    let mut writer = Writer::new(REPLY, limits)?;
    match value {
        AttemptLookup::Denied => writer.byte(0)?,
        AttemptLookup::Unavailable => writer.byte(1)?,
        AttemptLookup::Unresolved => writer.byte(2)?,
        AttemptLookup::Found {
            workflow,
            execution,
        } => {
            writer.byte(3)?;
            writer.raw(&workflow.0)?;
            match execution {
                ExecutionEvidence::ReceiptPending => writer.byte(0)?,
                ExecutionEvidence::Recorded {
                    execution_id,
                    disposition: observed,
                } => {
                    writer.byte(1)?;
                    writer.raw(execution_id)?;
                    writer.byte(disposition_tag(*observed))?;
                }
            }
        }
    }
    Ok(writer.bytes)
}

pub fn decode_attempt_reply(bytes: &[u8], limits: LimitsV1) -> Result<AttemptLookup, CodecErrorV1> {
    let mut reader = Reader::new(bytes, REPLY, limits)?;
    let value = match reader.byte()? {
        0 => AttemptLookup::Denied,
        1 => AttemptLookup::Unavailable,
        2 => AttemptLookup::Unresolved,
        3 => AttemptLookup::Found {
            workflow: WorkflowIdV1(reader.raw()?),
            execution: match reader.byte()? {
                0 => ExecutionEvidence::ReceiptPending,
                1 => ExecutionEvidence::Recorded {
                    execution_id: reader.raw()?,
                    disposition: read_disposition(reader.byte()?)?,
                },
                _ => return Err(CodecErrorV1::Malformed),
            },
        },
        _ => return Err(CodecErrorV1::Unsupported),
    };
    reader.finish()?;
    Ok(value)
}
