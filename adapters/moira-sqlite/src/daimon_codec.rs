//! Adapter-owned fake-provider intent/result data, never serialized authority.
use daimon_harness::*;
use keystone_codec::{DecodeLimits, Decoder};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeReply {
    pub disposition: ExecutionDispositionV1,
    pub output_bytes: usize,
    pub artifacts: Vec<ArtifactReferenceId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    Limit,
    Capacity,
    Invalid,
    Truncated,
}

pub(crate) struct Intent {
    pub request: HarnessRequestV1,
    pub reply: FakeReply,
}

struct Writer {
    bytes: Vec<u8>,
    max: usize,
}
impl Writer {
    fn reserve(&mut self, n: usize) -> Result<(), CodecError> {
        if self.bytes.len().checked_add(n).is_none_or(|n| n > self.max) {
            return Err(CodecError::Limit);
        }
        self.bytes.try_reserve(n).map_err(|_| CodecError::Capacity)
    }
    fn byte(&mut self, v: u8) -> Result<(), CodecError> {
        self.reserve(1)?;
        self.bytes.push(v);
        Ok(())
    }
    fn number(&mut self, v: u64) -> Result<(), CodecError> {
        self.reserve(8)?;
        self.bytes.extend_from_slice(&v.to_be_bytes());
        Ok(())
    }
    fn field(&mut self, v: &[u8]) -> Result<(), CodecError> {
        self.reserve(v.len().checked_add(4).ok_or(CodecError::Limit)?)?;
        keystone_codec::encode_bytes(&mut self.bytes, v).map_err(|_| CodecError::Limit)
    }
}
struct Reader<'a>(Decoder<'a>, usize);
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], max: usize, limits: Limits) -> Result<Self, CodecError> {
        let fields = limits
            .max_context_references
            .checked_add(limits.max_tools)
            .and_then(|n| n.checked_add(limits.max_artifacts))
            .and_then(|n| n.checked_add(32))
            .ok_or(CodecError::Limit)?;
        Ok(Self(
            Decoder::new(
                bytes,
                DecodeLimits {
                    max_input_bytes: max,
                    max_field_bytes: 16,
                    max_fields: fields,
                },
            )
            .map_err(|_| CodecError::Limit)?,
            bytes.len() / 20,
        ))
    }
    fn byte(&mut self) -> Result<u8, CodecError> {
        self.0.read_u8().map_err(|_| CodecError::Truncated)
    }
    fn number(&mut self) -> Result<u64, CodecError> {
        self.0.read_u64_be().map_err(|_| CodecError::Truncated)
    }
    fn id(&mut self) -> Result<[u8; 16], CodecError> {
        self.0
            .read_bytes()
            .map_err(|_| CodecError::Truncated)?
            .try_into()
            .map_err(|_| CodecError::Invalid)
    }
    fn count(&mut self, max: usize) -> Result<usize, CodecError> {
        let n = self.number()?.try_into().map_err(|_| CodecError::Limit)?;
        if n > max {
            return Err(CodecError::Limit);
        }
        Ok(n)
    }
    fn ids<T>(&mut self, max: usize, make: impl Fn([u8; 16]) -> T) -> Result<Vec<T>, CodecError> {
        // Each fixed-width identity occupies twenty encoded bytes. Do not
        // reserve from a claimed count that cannot fit even in the whole frame.
        let count = self.count(max.min(self.1))?;
        let mut values = Vec::new();
        values
            .try_reserve(count)
            .map_err(|_| CodecError::Capacity)?;
        for _ in 0..count {
            values.push(make(self.id()?))
        }
        Ok(values)
    }
    fn header(&mut self, magic: &[u8]) -> Result<(), CodecError> {
        if self.0.read_bytes().map_err(|_| CodecError::Truncated)? != magic || self.byte()? != 1 {
            return Err(CodecError::Invalid);
        }
        Ok(())
    }
    fn finish(&self) -> Result<(), CodecError> {
        if self.0.is_finished() {
            Ok(())
        } else {
            Err(CodecError::Invalid)
        }
    }
}
fn tag(d: ExecutionDispositionV1) -> u8 {
    match d {
        ExecutionDispositionV1::Completed => 0,
        ExecutionDispositionV1::Failed => 1,
        ExecutionDispositionV1::Cancelled => 2,
        ExecutionDispositionV1::TimedOut => 3,
        ExecutionDispositionV1::Uncertain => 4,
    }
}
fn disposition(v: u8) -> Result<ExecutionDispositionV1, CodecError> {
    Ok(match v {
        0 => ExecutionDispositionV1::Completed,
        1 => ExecutionDispositionV1::Failed,
        2 => ExecutionDispositionV1::Cancelled,
        3 => ExecutionDispositionV1::TimedOut,
        4 => ExecutionDispositionV1::Uncertain,
        _ => return Err(CodecError::Invalid),
    })
}

pub(crate) fn validate(
    request: &HarnessRequestV1,
    reply: &FakeReply,
    limits: Limits,
) -> Result<(), CodecError> {
    if request.context.len() > limits.max_context_references
        || request.tools.len() > limits.max_tools
        || reply.artifacts.len() > limits.max_artifacts
        || request.task_attempt.attempt_number() == 0
        || reply.output_bytes > request.budget.max_output_bytes
    {
        return Err(CodecError::Limit);
    }
    let b = request.budget;
    if b.max_elapsed_millis == 0
        || b.max_elapsed_millis > limits.max_elapsed_millis
        || b.max_input_tokens == 0
        || b.max_input_tokens > limits.max_input_tokens
        || b.max_output_tokens == 0
        || b.max_output_tokens > limits.max_output_tokens
        || b.max_tool_calls == 0
        || b.max_tool_calls > limits.max_tool_calls
        || b.max_context_bytes == 0
        || b.max_context_bytes > limits.max_context_bytes
        || b.max_output_bytes == 0
        || b.max_output_bytes > limits.max_output_bytes
    {
        return Err(CodecError::Limit);
    }
    Ok(())
}
pub(crate) fn encode_intent(
    request: &HarnessRequestV1,
    reply: &FakeReply,
    max: usize,
    limits: Limits,
) -> Result<Vec<u8>, CodecError> {
    validate(request, reply, limits)?;
    let mut w = Writer {
        bytes: Vec::new(),
        max,
    };
    w.field(b"MDAIMON")?;
    w.byte(1)?;
    w.field(&request.execution_id.into_bytes())?;
    w.field(&request.task_attempt.task_key())?;
    w.number(request.task_attempt.task_incarnation())?;
    w.number(request.task_attempt.attempt_number().into())?;
    w.field(&request.principal.into_bytes())?;
    w.field(&request.profile.into_bytes())?;
    w.field(&request.idempotency_key.into_bytes())?;
    w.number(request.context.len() as u64)?;
    for id in &request.context {
        w.field(&id.into_bytes())?
    }
    w.number(request.tools.len() as u64)?;
    for id in &request.tools {
        w.field(&id.into_bytes())?
    }
    let b = request.budget;
    for n in [
        b.max_elapsed_millis,
        b.max_input_tokens,
        b.max_output_tokens,
        b.max_tool_calls.into(),
        b.max_context_bytes as u64,
        b.max_output_bytes as u64,
    ] {
        w.number(n)?
    }
    w.byte(tag(reply.disposition))?;
    w.number(reply.output_bytes as u64)?;
    w.number(reply.artifacts.len() as u64)?;
    for id in &reply.artifacts {
        w.field(&id.into_bytes())?
    }
    Ok(w.bytes)
}
pub(crate) fn decode_intent(
    bytes: &[u8],
    max: usize,
    limits: Limits,
) -> Result<Intent, CodecError> {
    let mut r = Reader::new(bytes, max, limits)?;
    r.header(b"MDAIMON")?;
    let execution = ExecutionId::new(r.id()?);
    let task = TaskAttemptRefV1::new(
        r.id()?,
        r.number()?,
        r.number()?.try_into().map_err(|_| CodecError::Invalid)?,
    );
    let principal = PrincipalId::new(r.id()?);
    let profile = HarnessProfileId::new(r.id()?);
    let key = IdempotencyKey::new(r.id()?);
    let context = r.ids(limits.max_context_references, ContextReferenceId::new)?;
    let tools = r.ids(limits.max_tools, ToolCapabilityId::new)?;
    let budget = ExecutionBudgetV1 {
        max_elapsed_millis: r.number()?,
        max_input_tokens: r.number()?,
        max_output_tokens: r.number()?,
        max_tool_calls: r.number()?.try_into().map_err(|_| CodecError::Invalid)?,
        max_context_bytes: r.number()?.try_into().map_err(|_| CodecError::Limit)?,
        max_output_bytes: r.number()?.try_into().map_err(|_| CodecError::Limit)?,
    };
    let reply = FakeReply {
        disposition: disposition(r.byte()?)?,
        output_bytes: r.number()?.try_into().map_err(|_| CodecError::Limit)?,
        artifacts: r.ids(limits.max_artifacts, ArtifactReferenceId::new)?,
    };
    r.finish()?;
    let request = HarnessRequestV1::new(
        execution, task, principal, profile, key, context, tools, budget,
    );
    validate(&request, &reply, limits)?;
    Ok(Intent { request, reply })
}
pub(crate) fn encode_result(
    result: &HarnessResultV1,
    max: usize,
    limits: Limits,
) -> Result<Vec<u8>, CodecError> {
    if result.output_bytes > limits.max_output_bytes
        || result.produced_artifacts.len() > limits.max_artifacts
    {
        return Err(CodecError::Limit);
    }
    let mut w = Writer {
        bytes: Vec::new(),
        max,
    };
    w.field(b"MDAIRES")?;
    w.byte(1)?;
    w.field(&result.execution_id.into_bytes())?;
    w.byte(tag(result.disposition))?;
    w.number(result.output_bytes as u64)?;
    w.number(result.produced_artifacts.len() as u64)?;
    for id in &result.produced_artifacts {
        w.field(&id.into_bytes())?
    }
    Ok(w.bytes)
}
pub(crate) fn decode_result(
    bytes: &[u8],
    max: usize,
    limits: Limits,
) -> Result<HarnessResultV1, CodecError> {
    let mut r = Reader::new(bytes, max, limits)?;
    r.header(b"MDAIRES")?;
    let result = HarnessResultV1 {
        execution_id: ExecutionId::new(r.id()?),
        disposition: disposition(r.byte()?)?,
        output_bytes: r.number()?.try_into().map_err(|_| CodecError::Limit)?,
        produced_artifacts: r.ids(limits.max_artifacts, ArtifactReferenceId::new)?,
    };
    r.finish()?;
    if result.output_bytes > limits.max_output_bytes {
        return Err(CodecError::Limit);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> HarnessRequestV1 {
        HarnessRequestV1::new(
            ExecutionId::new([1; 16]),
            TaskAttemptRefV1::new([2; 16], 3, 4),
            PrincipalId::new([5; 16]),
            HarnessProfileId::new([6; 16]),
            IdempotencyKey::new([7; 16]),
            vec![ContextReferenceId::new([8; 16])],
            vec![ToolCapabilityId::new([9; 16])],
            ExecutionBudgetV1 {
                max_elapsed_millis: 100,
                max_input_tokens: 10,
                max_output_tokens: 10,
                max_tool_calls: 1,
                max_context_bytes: 100,
                max_output_bytes: 100,
            },
        )
    }
    #[test]
    fn all_fake_results_and_complete_intents_roundtrip_without_authority() {
        for d in [
            ExecutionDispositionV1::Completed,
            ExecutionDispositionV1::Failed,
            ExecutionDispositionV1::Cancelled,
            ExecutionDispositionV1::TimedOut,
            ExecutionDispositionV1::Uncertain,
        ] {
            let request = request();
            let reply = FakeReply {
                disposition: d,
                output_bytes: 50,
                artifacts: vec![ArtifactReferenceId::new([10; 16])],
            };
            let encoded = encode_intent(&request, &reply, 4096, Limits::default()).unwrap();
            let decoded = decode_intent(&encoded, 4096, Limits::default()).unwrap();
            assert_eq!(decoded.request, request);
            assert_eq!(decoded.reply, reply);
            assert_eq!(
                encode_intent(&decoded.request, &decoded.reply, 4096, Limits::default()).unwrap(),
                encoded
            );
            let result = HarnessResultV1 {
                execution_id: request.execution_id,
                disposition: d,
                output_bytes: 50,
                produced_artifacts: reply.artifacts,
            };
            let encoded = encode_result(&result, 4096, Limits::default()).unwrap();
            assert_eq!(
                decode_result(&encoded, 4096, Limits::default()).unwrap(),
                result
            );
        }
    }
    #[test]
    fn truncation_trailing_bytes_and_host_bounds_refuse() {
        let request = request();
        let reply = FakeReply {
            disposition: ExecutionDispositionV1::Completed,
            output_bytes: 0,
            artifacts: vec![],
        };
        let bytes = encode_intent(&request, &reply, 4096, Limits::default()).unwrap();
        for end in 0..bytes.len() {
            assert!(decode_intent(&bytes[..end], 4096, Limits::default()).is_err())
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_intent(&trailing, 4096, Limits::default()).is_err());
        assert!(decode_intent(&bytes, 8, Limits::default()).is_err());
        assert!(
            decode_intent(
                &bytes,
                4096,
                Limits {
                    max_tools: 0,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(encode_intent(&request, &reply, 8, Limits::default()).is_err());
    }
}
