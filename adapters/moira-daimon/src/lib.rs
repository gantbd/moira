//! Maps one Moira execution intent through Daimon's public client contract.
//!
//! Authority, current Chronos-attempt validation and provider selection remain
//! Daimon service responsibilities. No authority value crosses this adapter.

#![forbid(unsafe_code)]

use daimon_client::{ExecutionClient, ModelExecutionClient, ModelServicePort, ServicePort};
use daimon_protocol as daimon;
use moira_client::{ToolContinuationMutationClient, ToolContinuationServicePort};
use moira_protocol::{DispositionV1, EffectIdV1, ResultReceiptV1, TaskRefV1};

pub mod agent;
pub mod ensemble;

#[derive(Clone, Eq, PartialEq)]
pub struct ExecutionIntentV1 {
    pub effect: EffectIdV1,
    pub execution_id: [u8; 16],
    pub task: TaskRefV1,
    pub attempt_number: u32,
    pub principal: [u8; 16],
    pub profile: [u8; 16],
    pub idempotency_key: [u8; 16],
    pub context: Vec<[u8; 16]>,
    pub tools: Vec<[u8; 16]>,
    pub budget: daimon::ExecutionBudgetV1,
}

impl std::fmt::Debug for ExecutionIntentV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExecutionIntentV1")
            .field("effect", &self.effect)
            .field("execution_id", &"<opaque>")
            .field("task", &self.task)
            .field("attempt_number", &self.attempt_number)
            .field("principal", &"<redacted>")
            .field("profile", &"<opaque>")
            .field("context_count", &self.context.len())
            .field("tool_count", &self.tools.len())
            .field("budget", &self.budget)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionReplyV1 {
    Result(ResultReceiptV1),
    Refused(daimon::RefusalV1),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelProposalV1 {
    AgoraMessage {
        room: [u8; 16],
        reply_to: [u8; 16],
        body: Vec<u8>,
    },
    VellumEdit {
        workspace: [u8; 16],
        document: [u8; 16],
        expected_revision: u64,
        replacement: Vec<u8>,
    },
    ToolCall {
        request: [u8; 16],
        capability: [u8; 16],
        provider_correlation: Vec<u8>,
        arguments: Vec<u8>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelResultV1 {
    pub receipt: ResultReceiptV1,
    pub evidence_digest: [u8; 32],
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub output: Vec<u8>,
    pub proposals: Vec<ModelProposalV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedToolResultV1 {
    pub request: [u8; 16],
    pub evidence_digest: [u8; 32],
    pub output_digest: [u8; 32],
    pub output: Vec<u8>,
}

/// Projects provider-native tool proposals into the continuation contract.
/// The budget is supplied by Moira policy; models cannot choose it.
pub fn tool_calls(
    result: &ModelResultV1,
    budget: moira_protocol::tool_continuation::ToolBudgetV1,
) -> Vec<moira_protocol::tool_continuation::ToolCallV1> {
    result
        .proposals
        .iter()
        .filter_map(|proposal| match proposal {
            ModelProposalV1::ToolCall {
                request,
                capability,
                provider_correlation,
                arguments,
            } => Some(moira_protocol::tool_continuation::ToolCallV1 {
                request: moira_protocol::tool_continuation::ToolRequestIdV1(*request),
                capability: moira_protocol::tool_continuation::ToolCapabilityIdV1(*capability),
                provider_correlation: provider_correlation.clone(),
                arguments: arguments.clone(),
                budget,
            }),
            ModelProposalV1::AgoraMessage { .. } | ModelProposalV1::VellumEdit { .. } => None,
        })
        .collect()
}

/// Admits native model tool calls through Moira's capability-shaped client.
/// An empty projection is a no-op, and provider output never supplies policy
/// budgets or storage revisions.
pub fn admit_tool_continuation<P: ToolContinuationServicePort>(
    client: &mut ToolContinuationMutationClient<P>,
    expected_storage_revision: Option<u64>,
    workflow: moira_protocol::WorkflowIdV1,
    source_execution: EffectIdV1,
    result: &ModelResultV1,
    budget: moira_protocol::tool_continuation::ToolBudgetV1,
) -> Result<Option<moira_protocol::tool_continuation::ReplyV1>, P::Error> {
    let calls = tool_calls(result, budget);
    if calls.is_empty() {
        return Ok(None);
    }
    client
        .apply(
            expected_storage_revision,
            moira_protocol::tool_continuation::MutationV1::Admit {
                workflow,
                source_execution,
                source_attempt: moira_protocol::tool_continuation::AttemptEvidenceV1 {
                    task_key: result.receipt.task.external_task_key,
                    incarnation: result.receipt.task.incarnation,
                    attempt: result.receipt.attempt_number,
                },
                calls,
            },
        )
        .map(Some)
}

/// Commits the independently obtained Aegis and Chronos evidence as one exact
/// continuation gate. The domain service verifies both against retained call
/// capability and source-attempt admission evidence.
pub fn authorize_tool_effect<P: ToolContinuationServicePort>(
    client: &mut ToolContinuationMutationClient<P>,
    expected_storage_revision: Option<u64>,
    workflow: moira_protocol::WorkflowIdV1,
    effect: EffectIdV1,
    authority: moira_protocol::tool_continuation::AuthorityEvidenceV1,
    attempt: moira_protocol::tool_continuation::AttemptEvidenceV1,
) -> Result<moira_protocol::tool_continuation::ReplyV1, P::Error> {
    client.apply(
        expected_storage_revision,
        moira_protocol::tool_continuation::MutationV1::Authorize {
            workflow,
            effect,
            gate: moira_protocol::tool_continuation::ToolGateV1 { authority, attempt },
        },
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelExecutionReplyV1 {
    Result(ModelResultV1),
    Uncertain(ResultReceiptV1),
    Refused(daimon::RefusalV1),
}

#[derive(Debug, Eq, PartialEq)]
pub enum AdapterErrorV1<E> {
    InvalidInput,
    OutcomeUnknown(E),
    InvalidResponse,
}

pub struct ExecutionAdapter<P> {
    client: ExecutionClient<P>,
}

pub struct ModelExecutionAdapter<P> {
    client: ModelExecutionClient<P>,
}

impl<P: ModelServicePort> ModelExecutionAdapter<P> {
    pub const fn new(session: daimon::SessionHandleV1, port: P) -> Self {
        Self {
            client: ModelExecutionClient::new(session, port),
        }
    }

    pub fn into_port(self) -> P {
        self.client.into_port()
    }

    pub fn execute(
        &mut self,
        intent: ExecutionIntentV1,
        now: u64,
    ) -> Result<ModelExecutionReplyV1, AdapterErrorV1<P::Error>> {
        if intent.attempt_number == 0
            || intent.budget.max_elapsed_millis == 0
            || intent.budget.max_input_tokens == 0
            || intent.budget.max_output_tokens == 0
            || intent.budget.max_context_bytes == 0
            || intent.budget.max_output_bytes == 0
        {
            return Err(AdapterErrorV1::InvalidInput);
        }
        let reply = self
            .client
            .execute(request(&intent), now)
            .map_err(AdapterErrorV1::OutcomeUnknown)?;
        model_reply(&intent, reply)
    }

    pub fn resume(
        &mut self,
        intent: ExecutionIntentV1,
        source_execution: [u8; 16],
        results: Vec<CompletedToolResultV1>,
        now: u64,
    ) -> Result<ModelExecutionReplyV1, AdapterErrorV1<P::Error>> {
        if source_execution == [0; 16]
            || results.is_empty()
            || results.iter().any(|result| {
                result.request == [0; 16]
                    || result.evidence_digest == [0; 32]
                    || result.output_digest == [0; 32]
            })
        {
            return Err(AdapterErrorV1::InvalidInput);
        }
        let reply = self
            .client
            .resume(
                request(&intent),
                daimon::ExecutionIdV1::new(source_execution),
                results
                    .into_iter()
                    .map(|result| daimon::ResumeToolResultV1 {
                        request: result.request,
                        evidence_digest: daimon::ContentDigestV1::new(result.evidence_digest),
                        output_digest: daimon::ContentDigestV1::new(result.output_digest),
                        output: result.output,
                    })
                    .collect(),
                now,
            )
            .map_err(AdapterErrorV1::OutcomeUnknown)?;
        model_reply(&intent, reply)
    }
}

fn model_reply<E>(
    intent: &ExecutionIntentV1,
    reply: daimon::ModelReplyV1,
) -> Result<ModelExecutionReplyV1, AdapterErrorV1<E>> {
    match reply {
        daimon::ModelReplyV1::Accepted(daimon::ModelOutcomeV1::Executed(result)) => {
            if result.execution_id.into_bytes() != intent.execution_id {
                return Err(AdapterErrorV1::InvalidResponse);
            }
            Ok(ModelExecutionReplyV1::Result(ModelResultV1 {
                receipt: receipt(intent, disposition(result.disposition)),
                evidence_digest: result.evidence_digest.into_bytes(),
                input_tokens: result.input_tokens,
                output_tokens: result.output_tokens,
                output: result.output,
                proposals: result.proposals.into_iter().map(proposal).collect(),
            }))
        }
        daimon::ModelReplyV1::Accepted(daimon::ModelOutcomeV1::OutcomeUnknown { execution_id }) => {
            if execution_id.into_bytes() != intent.execution_id {
                return Err(AdapterErrorV1::InvalidResponse);
            }
            Ok(ModelExecutionReplyV1::Uncertain(receipt(
                intent,
                DispositionV1::Uncertain,
            )))
        }
        daimon::ModelReplyV1::Refused(refusal) => Ok(ModelExecutionReplyV1::Refused(refusal)),
        daimon::ModelReplyV1::Accepted(daimon::ModelOutcomeV1::Status { .. }) => {
            Err(AdapterErrorV1::InvalidResponse)
        }
    }
}

impl<P: ServicePort> ExecutionAdapter<P> {
    pub const fn new(session: daimon::SessionHandleV1, port: P) -> Self {
        Self {
            client: ExecutionClient::new(session, port),
        }
    }

    pub fn into_port(self) -> P {
        self.client.into_port()
    }

    pub fn execute(
        &mut self,
        intent: ExecutionIntentV1,
        now: u64,
    ) -> Result<ExecutionReplyV1, AdapterErrorV1<P::Error>> {
        if intent.attempt_number == 0
            || intent.budget.max_elapsed_millis == 0
            || intent.budget.max_input_tokens == 0
            || intent.budget.max_output_tokens == 0
            || intent.budget.max_tool_calls == 0
            || intent.budget.max_context_bytes == 0
            || intent.budget.max_output_bytes == 0
        {
            return Err(AdapterErrorV1::InvalidInput);
        }
        let request = request(&intent);
        let reply = self
            .client
            .execute(request, now)
            .map_err(AdapterErrorV1::OutcomeUnknown)?;
        match reply {
            daimon::ReplyV1::Accepted(daimon::OutcomeV1::Executed(result)) => {
                if result.execution_id.into_bytes() != intent.execution_id {
                    return Err(AdapterErrorV1::InvalidResponse);
                }
                Ok(ExecutionReplyV1::Result(receipt(
                    &intent,
                    disposition(result.disposition),
                )))
            }
            daimon::ReplyV1::Accepted(daimon::OutcomeV1::OutcomeUnknown { execution_id }) => {
                if execution_id.into_bytes() != intent.execution_id {
                    return Err(AdapterErrorV1::InvalidResponse);
                }
                Ok(ExecutionReplyV1::Result(receipt(
                    &intent,
                    DispositionV1::Uncertain,
                )))
            }
            daimon::ReplyV1::Refused(refusal) => Ok(ExecutionReplyV1::Refused(refusal)),
            daimon::ReplyV1::Accepted(
                daimon::OutcomeV1::Status { .. } | daimon::OutcomeV1::Presence { .. },
            ) => Err(AdapterErrorV1::InvalidResponse),
        }
    }
}

fn request(intent: &ExecutionIntentV1) -> daimon::ExecuteRequestV1 {
    daimon::ExecuteRequestV1 {
        execution_id: daimon::ExecutionIdV1::new(intent.execution_id),
        task_attempt: daimon::TaskAttemptRefV1 {
            task_key: intent.task.external_task_key,
            task_incarnation: intent.task.incarnation,
            attempt_number: intent.attempt_number,
        },
        principal: daimon::PrincipalIdV1::new(intent.principal),
        profile: daimon::HarnessProfileIdV1::new(intent.profile),
        idempotency_key: daimon::IdempotencyKeyV1::new(intent.idempotency_key),
        context: intent
            .context
            .iter()
            .map(|value| daimon::ContextReferenceIdV1::new(*value))
            .collect(),
        tools: intent
            .tools
            .iter()
            .map(|value| daimon::ToolCapabilityIdV1::new(*value))
            .collect(),
        budget: intent.budget,
    }
}

fn receipt(intent: &ExecutionIntentV1, disposition: DispositionV1) -> ResultReceiptV1 {
    ResultReceiptV1 {
        effect: intent.effect,
        execution_id: intent.execution_id,
        task: intent.task,
        attempt_number: intent.attempt_number,
        disposition,
    }
}

fn disposition(value: daimon::ExecutionDispositionV1) -> DispositionV1 {
    match value {
        daimon::ExecutionDispositionV1::Completed => DispositionV1::Completed,
        daimon::ExecutionDispositionV1::Failed => DispositionV1::Failed,
        daimon::ExecutionDispositionV1::Cancelled => DispositionV1::Cancelled,
        daimon::ExecutionDispositionV1::TimedOut => DispositionV1::TimedOut,
        daimon::ExecutionDispositionV1::Uncertain => DispositionV1::Uncertain,
    }
}

fn proposal(value: daimon::OutputProposalV1) -> ModelProposalV1 {
    match value {
        daimon::OutputProposalV1::AgoraMessage(value) => ModelProposalV1::AgoraMessage {
            room: value.room,
            reply_to: value.reply_to,
            body: value.body,
        },
        daimon::OutputProposalV1::VellumEdit(value) => ModelProposalV1::VellumEdit {
            workspace: value.workspace,
            document: value.document,
            expected_revision: value.expected_revision,
            replacement: value.replacement,
        },
        daimon::OutputProposalV1::ToolCall(value) => ModelProposalV1::ToolCall {
            request: value.request,
            capability: value.capability.into_bytes(),
            provider_correlation: value.provider_correlation,
            arguments: value.arguments,
        },
    }
}

pub use daimon_client::ExecutionStatusV1;

/// Reads current Daimon status through the caller's own authenticated client.
pub fn inspect_execution<P: ServicePort>(
    client: &mut daimon_client::ReadOnlyClient<P>,
    execution_id: [u8; 16],
) -> ExecutionStatusV1 {
    client.inspect_status(daimon::ExecutionIdV1::new(execution_id))
}
