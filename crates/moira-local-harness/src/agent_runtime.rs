//! Crash-conservative dispatch of one durably prepared agent operation.
//!
//! Deployment composition supplies budgets, context and tools. The standard
//! store-backed materializer resolves prompt bytes from their retained digest,
//! and every identity carried to Daimon is checked against the retained Moira
//! participation before dispatch. Any ambiguity after dispatch is fenced and
//! is never converted into permission to retry.

use moira_client::AgentParticipationServicePort;
use moira_host::{AgentGateway, AgentOutputStoreV1, AgentPromptStoreV1};
use moira_protocol::{agent, agent_participation as participation};
use sha2::{Digest, Sha256};

pub const MAX_FIXED_AGENT_CONTEXT_V1: usize = 64;
pub const MAX_FIXED_AGENT_TOOLS_V1: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedParticipationV1 {
    pub storage_revision: u64,
    pub participation: participation::ParticipationViewV1,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PreparedParticipationLoadErrorV1<E> {
    Service(E),
    Refused(participation::RefusalV1),
    InvalidStatus,
    NotFound,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PromptMaterializationLoadErrorV1<E> {
    Service(E),
    Refused(participation::RefusalV1),
    InvalidReply,
    Integrity,
}

#[derive(Debug, Eq, PartialEq)]
pub enum AgentWorkerOutcomeV1<E> {
    Idle(PreparedParticipationV1),
    Progress(AgentDriveOutcomeV1<E>),
}

#[derive(Debug, Eq, PartialEq)]
pub enum AgentWorkerErrorV1<ServiceError, MaterializerError, OutputError> {
    Load(PreparedParticipationLoadErrorV1<ServiceError>),
    Dispatch(AgentDriveErrorV1<ServiceError, MaterializerError, OutputError>),
    Reconcile(AgentDriveErrorV1<ServiceError, std::convert::Infallible, OutputError>),
}

pub type AgentWorkerResultV1<P, G, M, O> = Result<
    AgentWorkerOutcomeV1<<G as AgentGateway>::Error>,
    AgentWorkerErrorV1<
        <P as AgentParticipationServicePort>::Error,
        <M as AgentCommandMaterializerV1>::Error,
        <O as AgentOutputStoreV1>::Error,
    >,
>;

/// Load one exact participation together with the store revision that must
/// still be current when a worker fences a dispatch. This is read-only and
/// cannot manufacture a prepared operation from terminal input.
pub fn load_prepared<P: AgentParticipationServicePort>(
    service: &mut P,
    id: participation::ParticipationIdV1,
) -> Result<PreparedParticipationV1, PreparedParticipationLoadErrorV1<P::Error>> {
    let storage_revision = match service
        .execute_participation(participation::CommandV1::Status)
        .map_err(PreparedParticipationLoadErrorV1::Service)?
    {
        participation::ReplyV1::Accepted(value) => match *value {
            participation::OutcomeV1::Status(value) if !value.needs_reopen => value
                .storage_revision
                .ok_or(PreparedParticipationLoadErrorV1::InvalidStatus)?,
            _ => return Err(PreparedParticipationLoadErrorV1::InvalidStatus),
        },
        participation::ReplyV1::Refused(refusal) => {
            return Err(PreparedParticipationLoadErrorV1::Refused(refusal));
        }
    };
    let participation = match service
        .execute_participation(participation::CommandV1::Participation { participation: id })
        .map_err(PreparedParticipationLoadErrorV1::Service)?
    {
        participation::ReplyV1::Accepted(value) => match *value {
            participation::OutcomeV1::Participation(Some(value)) => value,
            participation::OutcomeV1::Participation(None) => {
                return Err(PreparedParticipationLoadErrorV1::NotFound);
            }
            _ => return Err(PreparedParticipationLoadErrorV1::InvalidStatus),
        },
        participation::ReplyV1::Refused(refusal) => {
            return Err(PreparedParticipationLoadErrorV1::Refused(refusal));
        }
    };
    Ok(PreparedParticipationV1 {
        storage_revision,
        participation,
    })
}

/// Resolve prompt bytes only through the owning participation daemon and only
/// for the exact current pending prompt. Callers should close this connection
/// and reconnect for dispatch so the daemon's bounded single-connection loop
/// is never starved by a second retained client.
pub fn materialize_pending_prompt<P: AgentParticipationServicePort>(
    service: &mut P,
    prepared: &PreparedParticipationV1,
) -> Result<Option<Vec<u8>>, PromptMaterializationLoadErrorV1<P::Error>> {
    let Some(participation::OperationV1::Prompt { prompt_digest, .. }) =
        prepared.participation.pending
    else {
        return Ok(None);
    };
    let reply = service
        .execute_participation(participation::CommandV1::MaterializePrompt(
            participation::PromptMaterializationRequestV1 {
                participation: prepared.participation.binding.participation,
                expected_revision: prepared.participation.revision,
                prompt_digest,
            },
        ))
        .map_err(PromptMaterializationLoadErrorV1::Service)?;
    let value = match reply {
        participation::ReplyV1::Accepted(value) => match *value {
            participation::OutcomeV1::PromptMaterialization(value) => value,
            _ => return Err(PromptMaterializationLoadErrorV1::InvalidReply),
        },
        participation::ReplyV1::Refused(refusal) => {
            return Err(PromptMaterializationLoadErrorV1::Refused(refusal));
        }
    };
    if value.participation != prepared.participation.binding.participation
        || value.participation_revision != prepared.participation.revision
        || value.prompt_digest != prompt_digest
        || participation::agent_prompt_digest(&value.prompt) != Some(prompt_digest)
    {
        return Err(PromptMaterializationLoadErrorV1::Integrity);
    }
    Ok(Some(value.prompt))
}

/// Drive at most one durable participation step. Prepared operations dispatch
/// once; fenced operations can only reconcile by inspection. A ready idle
/// record performs one read-only session inspection so a detached provider
/// binding can be durably marked before any reattach is prepared. A caller may
/// loop this function under its own bounded process lifecycle without gaining
/// any additional authority.
pub fn drive_current<P, G, M, O>(
    service: &mut P,
    gateway: &mut G,
    materializer: &mut M,
    outputs: &mut O,
    id: participation::ParticipationIdV1,
    now: u64,
    max_event_observations: u64,
) -> AgentWorkerResultV1<P, G, M, O>
where
    P: AgentParticipationServicePort,
    G: AgentGateway,
    M: AgentCommandMaterializerV1,
    O: AgentOutputStoreV1,
{
    let prepared = load_prepared(service, id).map_err(AgentWorkerErrorV1::Load)?;
    if prepared.participation.phase == participation::PhaseV1::FencedUnknown {
        return reconcile_fenced(service, gateway, outputs, prepared, max_event_observations)
            .map(AgentWorkerOutcomeV1::Progress)
            .map_err(AgentWorkerErrorV1::Reconcile);
    }
    if prepared.participation.pending.is_none() {
        if prepared.participation.phase == participation::PhaseV1::Ready
            && prepared.participation.capabilities.resume_session
        {
            let session = agent::SessionIdV1(prepared.participation.binding.session.0);
            if let Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(observed))) =
                gateway.execute_agent(agent::CommandV1::Inspect { session })
                && observed.phase == agent::PhaseV1::ReattachRequired
            {
                if !valid_reattach_required(&prepared.participation, &observed) {
                    return Err(AgentWorkerErrorV1::Dispatch(
                        AgentDriveErrorV1::InvalidState,
                    ));
                }
                let reply = service
                    .execute_participation(participation::CommandV1::Apply {
                        expected_storage_revision: Some(prepared.storage_revision),
                        mutation: Box::new(participation::MutationV1::MarkReattachRequired {
                            participation: prepared.participation.binding.participation,
                            expected_revision: prepared.participation.revision,
                            session_revision: observed.revision,
                        }),
                    })
                    .map_err(|error| {
                        AgentWorkerErrorV1::Dispatch(AgentDriveErrorV1::Service(error))
                    })?;
                let (storage_revision, participation) =
                    applied(reply).map_err(AgentWorkerErrorV1::Dispatch)?;
                return Ok(AgentWorkerOutcomeV1::Progress(
                    AgentDriveOutcomeV1::Observed {
                        storage_revision,
                        participation,
                    },
                ));
            }
        }
        return Ok(AgentWorkerOutcomeV1::Idle(prepared));
    }
    dispatch_prepared(
        service,
        gateway,
        materializer,
        outputs,
        prepared,
        now,
        max_event_observations,
    )
    .map(AgentWorkerOutcomeV1::Progress)
    .map_err(AgentWorkerErrorV1::Dispatch)
}

fn valid_reattach_required(
    retained: &participation::ParticipationViewV1,
    observed: &agent::SessionViewV1,
) -> bool {
    observed.session.0 == retained.binding.session.0
        && Some(observed.revision) == retained.session_revision
        && observed.active_turn.is_none()
        && observed.last_event_cursor == retained.last_event_cursor
        && retained.profile_document_digest == Some(observed.profile_document_digest.0)
        && observed.capabilities.resume_session
        && capabilities_view(observed.capabilities) == retained.capabilities
}

pub trait AgentCommandMaterializerV1 {
    type Error;

    fn materialize(
        &mut self,
        participation: &participation::ParticipationViewV1,
        operation: participation::OperationV1,
        now: u64,
    ) -> Result<agent::CommandV1, Self::Error>;
}

/// Deployment-owned command policy after prompt content has been resolved from
/// durable storage. `prompt` is present exactly for a prompt operation.
pub trait AgentCommandTemplateV1 {
    type Error;

    fn materialize(
        &mut self,
        participation: &participation::ParticipationViewV1,
        operation: participation::OperationV1,
        prompt: Option<&[u8]>,
        now: u64,
    ) -> Result<agent::CommandV1, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixedAgentCommandPolicyV1 {
    pub principal: agent::PrincipalIdV1,
    pub workspace: [u8; 16],
    pub context: Vec<agent::ContextIdV1>,
    pub tools: Vec<agent::ToolIdV1>,
    pub execution_budget: agent::ExecutionBudgetV1,
    pub session_budget: agent::SessionBudgetV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedAgentCommandTemplateErrorV1 {
    InvalidPolicy,
    MissingPrompt,
    UnexpectedPrompt,
    MissingSessionRevision,
}

pub struct FixedAgentCommandTemplateV1 {
    policy: FixedAgentCommandPolicyV1,
}

impl FixedAgentCommandTemplateV1 {
    pub fn new(
        policy: FixedAgentCommandPolicyV1,
    ) -> Result<Self, FixedAgentCommandTemplateErrorV1> {
        if policy.execution_budget.max_elapsed_millis == 0
            || policy.execution_budget.max_input_tokens == 0
            || policy.execution_budget.max_output_tokens == 0
            || policy.execution_budget.max_context_bytes == 0
            || policy.execution_budget.max_output_bytes == 0
            || policy.session_budget.max_elapsed_millis == 0
            || policy.session_budget.max_turns == 0
            || policy.session_budget.max_events == 0
            || policy.session_budget.max_event_bytes == 0
            || policy.session_budget.max_prompt_bytes == 0
            || policy.session_budget.max_permission_requests == 0
            || policy.context.len() > MAX_FIXED_AGENT_CONTEXT_V1
            || policy.tools.len() > MAX_FIXED_AGENT_TOOLS_V1
            || has_duplicate_context(&policy.context)
            || has_duplicate_tools(&policy.tools)
        {
            return Err(FixedAgentCommandTemplateErrorV1::InvalidPolicy);
        }
        Ok(Self { policy })
    }
}

impl AgentCommandTemplateV1 for FixedAgentCommandTemplateV1 {
    type Error = FixedAgentCommandTemplateErrorV1;

    fn materialize(
        &mut self,
        view: &participation::ParticipationViewV1,
        operation: participation::OperationV1,
        prompt: Option<&[u8]>,
        now: u64,
    ) -> Result<agent::CommandV1, Self::Error> {
        let session = agent::SessionIdV1(view.binding.session.0);
        let revision = || {
            view.session_revision
                .ok_or(FixedAgentCommandTemplateErrorV1::MissingSessionRevision)
        };
        let command = match operation {
            participation::OperationV1::Open { key, execution } => {
                no_prompt(prompt)?;
                agent::CommandV1::Open {
                    request: agent::OpenRequestV1 {
                        session,
                        execution: self.execution(view, execution, key, view.binding.attempt),
                        workspace: self.policy.workspace,
                        budget: self.policy.session_budget,
                    },
                    now,
                }
            }
            participation::OperationV1::Prompt {
                key,
                turn,
                execution,
                prompt_digest,
                attempt,
            } => agent::CommandV1::Prompt {
                request: agent::PromptRequestV1 {
                    session,
                    turn: agent::TurnIdV1(turn.0),
                    expected_revision: revision()?,
                    execution: self.execution(view, execution, key, attempt),
                    prompt_digest: agent::DigestV1(prompt_digest),
                    prompt: prompt
                        .ok_or(FixedAgentCommandTemplateErrorV1::MissingPrompt)?
                        .to_vec(),
                },
                now,
            },
            participation::OperationV1::DecidePermission {
                key,
                execution,
                request,
                option,
                attempt,
                ..
            } => {
                no_prompt(prompt)?;
                agent::CommandV1::DecidePermission {
                    session,
                    expected_revision: revision()?,
                    execution: self.execution(view, execution, key, attempt),
                    decision: agent::PermissionDecisionV1 {
                        request: agent::PermissionRequestIdV1(request.0),
                        option: agent::PermissionOptionIdV1(option.0),
                    },
                    now,
                }
            }
            participation::OperationV1::Advance { key, turn } => {
                no_prompt(prompt)?;
                agent::CommandV1::Advance {
                    session,
                    turn: agent::TurnIdV1(turn.0),
                    expected_revision: revision()?,
                    idempotency_key: agent::IdempotencyKeyV1(key.0),
                    now,
                }
            }
            participation::OperationV1::Cancel { key, turn } => {
                no_prompt(prompt)?;
                agent::CommandV1::Cancel {
                    session,
                    turn: agent::TurnIdV1(turn.0),
                    expected_revision: revision()?,
                    idempotency_key: agent::IdempotencyKeyV1(key.0),
                    now,
                }
            }
            participation::OperationV1::Close { key } => {
                no_prompt(prompt)?;
                agent::CommandV1::Close {
                    session,
                    expected_revision: revision()?,
                    idempotency_key: agent::IdempotencyKeyV1(key.0),
                    now,
                }
            }
            participation::OperationV1::Reattach {
                key,
                execution,
                attempt,
            } => {
                no_prompt(prompt)?;
                agent::CommandV1::Reattach {
                    session,
                    expected_revision: revision()?,
                    execution: self.execution(view, execution, key, attempt),
                    now,
                }
            }
        };
        Ok(command)
    }
}

impl FixedAgentCommandTemplateV1 {
    fn execution(
        &self,
        view: &participation::ParticipationViewV1,
        execution: participation::ExecutionIdV1,
        key: participation::IdempotencyKeyV1,
        attempt: participation::AttemptV1,
    ) -> agent::ExecutionV1 {
        agent::ExecutionV1 {
            id: agent::ExecutionIdV1(execution.0),
            attempt: agent::AttemptV1 {
                task_key: attempt.task_key,
                incarnation: attempt.incarnation,
                attempt: attempt.attempt,
            },
            principal: self.policy.principal,
            profile: agent::ProfileIdV1(view.binding.profile.0),
            idempotency_key: agent::IdempotencyKeyV1(key.0),
            context: self.policy.context.clone(),
            tools: self.policy.tools.clone(),
            budget: self.policy.execution_budget,
        }
    }
}

fn no_prompt(prompt: Option<&[u8]>) -> Result<(), FixedAgentCommandTemplateErrorV1> {
    if prompt.is_some() {
        return Err(FixedAgentCommandTemplateErrorV1::UnexpectedPrompt);
    }
    Ok(())
}

fn has_duplicate_context(values: &[agent::ContextIdV1]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[..index].contains(value))
}

fn has_duplicate_tools(values: &[agent::ToolIdV1]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[..index].contains(value))
}

#[derive(Debug, Eq, PartialEq)]
pub enum StoredPromptMaterializerErrorV1<S, T> {
    Store(S),
    Missing,
    Integrity,
    Template(T),
}

pub struct StoredPromptMaterializerV1<P, T> {
    prompts: P,
    template: T,
}

/// Command materializer for prompt bytes already fetched through the owning
/// daemon's worker-only materialization command. It performs the digest check
/// again immediately before constructing the provider command.
pub struct RetainedPromptMaterializerV1<T> {
    prompt: Option<Vec<u8>>,
    template: T,
}

impl<T> RetainedPromptMaterializerV1<T> {
    pub const fn new(prompt: Option<Vec<u8>>, template: T) -> Self {
        Self { prompt, template }
    }

    pub fn into_parts(self) -> (Option<Vec<u8>>, T) {
        (self.prompt, self.template)
    }
}

impl<T: AgentCommandTemplateV1> AgentCommandMaterializerV1 for RetainedPromptMaterializerV1<T> {
    type Error = StoredPromptMaterializerErrorV1<std::convert::Infallible, T::Error>;

    fn materialize(
        &mut self,
        participation: &participation::ParticipationViewV1,
        operation: participation::OperationV1,
        now: u64,
    ) -> Result<agent::CommandV1, Self::Error> {
        let prompt = match operation {
            participation::OperationV1::Prompt { prompt_digest, .. } => {
                let bytes = self
                    .prompt
                    .as_deref()
                    .ok_or(StoredPromptMaterializerErrorV1::Missing)?;
                if participation::agent_prompt_digest(bytes) != Some(prompt_digest) {
                    return Err(StoredPromptMaterializerErrorV1::Integrity);
                }
                Some(bytes)
            }
            _ => None,
        };
        self.template
            .materialize(participation, operation, prompt, now)
            .map_err(StoredPromptMaterializerErrorV1::Template)
    }
}

impl<P, T> StoredPromptMaterializerV1<P, T> {
    pub const fn new(prompts: P, template: T) -> Self {
        Self { prompts, template }
    }

    pub fn into_parts(self) -> (P, T) {
        (self.prompts, self.template)
    }
}

impl<P: AgentPromptStoreV1, T: AgentCommandTemplateV1> AgentCommandMaterializerV1
    for StoredPromptMaterializerV1<P, T>
{
    type Error = StoredPromptMaterializerErrorV1<P::Error, T::Error>;

    fn materialize(
        &mut self,
        participation: &participation::ParticipationViewV1,
        operation: participation::OperationV1,
        now: u64,
    ) -> Result<agent::CommandV1, Self::Error> {
        let prompt = match operation {
            participation::OperationV1::Prompt { prompt_digest, .. } => {
                let bytes = self
                    .prompts
                    .get(prompt_digest)
                    .map_err(StoredPromptMaterializerErrorV1::Store)?
                    .ok_or(StoredPromptMaterializerErrorV1::Missing)?;
                if <[u8; 32]>::from(Sha256::digest(&bytes)) != prompt_digest {
                    return Err(StoredPromptMaterializerErrorV1::Integrity);
                }
                Some(bytes)
            }
            _ => None,
        };
        self.template
            .materialize(participation, operation, prompt.as_deref(), now)
            .map_err(StoredPromptMaterializerErrorV1::Template)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum AgentDriveErrorV1<ServiceError, MaterializerError, OutputError = std::convert::Infallible>
{
    Service(ServiceError),
    Refused(participation::RefusalV1),
    InvalidState,
    Materializer(MaterializerError),
    Output(OutputError),
    InvalidProjection,
    InvalidOutput,
}

#[derive(Debug, Eq, PartialEq)]
pub enum AgentFenceCauseV1<GatewayError> {
    Gateway(GatewayError),
    ProviderOutcomeUnknown,
    ProviderRefused(agent::RefusalV1),
    InvalidProviderEvidence,
}

#[derive(Debug, Eq, PartialEq)]
pub enum AgentDriveOutcomeV1<GatewayError> {
    Observed {
        storage_revision: u64,
        participation: participation::ParticipationViewV1,
    },
    Fenced {
        storage_revision: u64,
        participation: participation::ParticipationViewV1,
        cause: AgentFenceCauseV1<GatewayError>,
    },
}

pub type AgentDriveResultV1<P, G, M, O> = Result<
    AgentDriveOutcomeV1<<G as AgentGateway>::Error>,
    AgentDriveErrorV1<
        <P as AgentParticipationServicePort>::Error,
        <M as AgentCommandMaterializerV1>::Error,
        <O as AgentOutputStoreV1>::Error,
    >,
>;

pub type AgentReconcileResultV1<P, G, O> = Result<
    AgentDriveOutcomeV1<<G as AgentGateway>::Error>,
    AgentDriveErrorV1<
        <P as AgentParticipationServicePort>::Error,
        std::convert::Infallible,
        <O as AgentOutputStoreV1>::Error,
    >,
>;

/// Dispatch exactly one already-prepared operation. The prepared receipt is
/// required so a terminal host cannot synthesize a dispatch from a read view.
pub fn dispatch_prepared<P, G, M, O>(
    service: &mut P,
    gateway: &mut G,
    materializer: &mut M,
    outputs: &mut O,
    prepared: PreparedParticipationV1,
    now: u64,
    max_event_observations: u64,
) -> AgentDriveResultV1<P, G, M, O>
where
    P: AgentParticipationServicePort,
    G: AgentGateway,
    M: AgentCommandMaterializerV1,
    O: AgentOutputStoreV1,
{
    if max_event_observations == 0
        || prepared.participation.phase == participation::PhaseV1::FencedUnknown
    {
        return Err(AgentDriveErrorV1::InvalidState);
    }
    verify_current(service, &prepared)?;
    let operation = prepared
        .participation
        .pending
        .ok_or(AgentDriveErrorV1::InvalidState)?;
    let command = materializer
        .materialize(&prepared.participation, operation, now)
        .map_err(AgentDriveErrorV1::Materializer)?;
    if !valid_projection(&prepared.participation, operation, &command, now) {
        return Err(AgentDriveErrorV1::InvalidProjection);
    }
    // This durable one-way fence is the dispatch marker. From this point on,
    // recovery may reconcile provider state but may never issue the operation
    // again. A successful exact observation closes the fence below.
    let (storage_revision, participation) = applied(
        service
            .execute_participation(participation::CommandV1::Apply {
                expected_storage_revision: Some(prepared.storage_revision),
                mutation: Box::new(participation::MutationV1::FenceUnknown {
                    participation: prepared.participation.binding.participation,
                    expected_revision: prepared.participation.revision,
                    key: operation_key(operation),
                }),
            })
            .map_err(AgentDriveErrorV1::Service)?,
    )?;
    let inflight = PreparedParticipationV1 {
        storage_revision,
        participation,
    };
    let reply = match gateway.execute_agent(command) {
        Ok(reply) => reply,
        Err(error) => {
            return Ok(fenced(&inflight, AgentFenceCauseV1::Gateway(error)));
        }
    };
    let session = match reply {
        agent::ReplyV1::Accepted(agent::OutcomeV1::Session(value)) => value,
        agent::ReplyV1::Accepted(agent::OutcomeV1::OutcomeUnknown { .. }) => {
            return Ok(fenced(&inflight, AgentFenceCauseV1::ProviderOutcomeUnknown));
        }
        agent::ReplyV1::Refused(refusal) => {
            return Ok(fenced(
                &inflight,
                AgentFenceCauseV1::ProviderRefused(refusal),
            ));
        }
        agent::ReplyV1::Accepted(agent::OutcomeV1::Events(_)) => {
            return Ok(fenced(
                &inflight,
                AgentFenceCauseV1::InvalidProviderEvidence,
            ));
        }
    };
    if session.session.0 != inflight.participation.binding.session.0
        || inflight
            .participation
            .session_revision
            .is_some_and(|prior| session.revision <= prior)
    {
        return Ok(fenced(
            &inflight,
            AgentFenceCauseV1::InvalidProviderEvidence,
        ));
    }
    let page = match gateway.execute_agent(agent::CommandV1::Events {
        session: session.session,
        after: inflight.participation.last_event_cursor,
        limit: max_event_observations,
    }) {
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Events(page))) => page,
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::OutcomeUnknown { .. })) => {
            return Ok(fenced(&inflight, AgentFenceCauseV1::ProviderOutcomeUnknown));
        }
        Ok(agent::ReplyV1::Refused(refusal)) => {
            return Ok(fenced(
                &inflight,
                AgentFenceCauseV1::ProviderRefused(refusal),
            ));
        }
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(_))) => {
            return Ok(fenced(
                &inflight,
                AgentFenceCauseV1::InvalidProviderEvidence,
            ));
        }
        Err(error) => {
            return Ok(fenced(&inflight, AgentFenceCauseV1::Gateway(error)));
        }
    };
    if page.session != session.session
        || !valid_event_page(&inflight.participation, &session, &page)
    {
        return Ok(fenced(
            &inflight,
            AgentFenceCauseV1::InvalidProviderEvidence,
        ));
    }
    let composed_output =
        compose_output(outputs, &inflight.participation, operation, &session, &page)?;
    let Some(observation) = observation(
        &inflight.participation,
        operation,
        session,
        page,
        composed_output,
    ) else {
        return Ok(fenced(
            &inflight,
            AgentFenceCauseV1::InvalidProviderEvidence,
        ));
    };
    applied(
        service
            .execute_participation(participation::CommandV1::Apply {
                expected_storage_revision: Some(inflight.storage_revision),
                mutation: Box::new(participation::MutationV1::Observe {
                    participation: inflight.participation.binding.participation,
                    expected_revision: inflight.participation.revision,
                    key: operation_key(operation),
                    observation,
                }),
            })
            .map_err(AgentDriveErrorV1::Service)?,
    )
    .map(
        |(storage_revision, participation)| AgentDriveOutcomeV1::Observed {
            storage_revision,
            participation,
        },
    )
}

/// Read-only recovery for a retained dispatch fence. This function can inspect
/// provider state and commit an exact observation; it cannot construct or
/// submit any mutating provider command.
pub fn reconcile_fenced<P, G, O>(
    service: &mut P,
    gateway: &mut G,
    outputs: &mut O,
    inflight: PreparedParticipationV1,
    max_event_observations: u64,
) -> AgentReconcileResultV1<P, G, O>
where
    P: AgentParticipationServicePort,
    G: AgentGateway,
    O: AgentOutputStoreV1,
{
    if max_event_observations == 0
        || inflight.participation.phase != participation::PhaseV1::FencedUnknown
    {
        return Err(AgentDriveErrorV1::InvalidState);
    }
    verify_current(service, &inflight)?;
    let operation = inflight
        .participation
        .pending
        .ok_or(AgentDriveErrorV1::InvalidState)?;
    let session = agent::SessionIdV1(inflight.participation.binding.session.0);
    let session = match gateway.execute_agent(agent::CommandV1::Inspect { session }) {
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(value))) => value,
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::OutcomeUnknown { .. })) => {
            return Ok(fenced(&inflight, AgentFenceCauseV1::ProviderOutcomeUnknown));
        }
        Ok(agent::ReplyV1::Refused(refusal)) => {
            return Ok(fenced(
                &inflight,
                AgentFenceCauseV1::ProviderRefused(refusal),
            ));
        }
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Events(_))) => {
            return Ok(fenced(
                &inflight,
                AgentFenceCauseV1::InvalidProviderEvidence,
            ));
        }
        Err(error) => {
            return Ok(fenced(&inflight, AgentFenceCauseV1::Gateway(error)));
        }
    };
    if session.session.0 != inflight.participation.binding.session.0
        || inflight
            .participation
            .session_revision
            .is_some_and(|prior| session.revision <= prior)
    {
        return Ok(fenced(
            &inflight,
            AgentFenceCauseV1::InvalidProviderEvidence,
        ));
    }
    let page = match gateway.execute_agent(agent::CommandV1::Events {
        session: session.session,
        after: inflight.participation.last_event_cursor,
        limit: max_event_observations,
    }) {
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Events(page))) => page,
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::OutcomeUnknown { .. })) => {
            return Ok(fenced(&inflight, AgentFenceCauseV1::ProviderOutcomeUnknown));
        }
        Ok(agent::ReplyV1::Refused(refusal)) => {
            return Ok(fenced(
                &inflight,
                AgentFenceCauseV1::ProviderRefused(refusal),
            ));
        }
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(_))) => {
            return Ok(fenced(
                &inflight,
                AgentFenceCauseV1::InvalidProviderEvidence,
            ));
        }
        Err(error) => {
            return Ok(fenced(&inflight, AgentFenceCauseV1::Gateway(error)));
        }
    };
    if page.session != session.session
        || !valid_event_page(&inflight.participation, &session, &page)
    {
        return Ok(fenced(
            &inflight,
            AgentFenceCauseV1::InvalidProviderEvidence,
        ));
    }
    let composed_output =
        compose_output(outputs, &inflight.participation, operation, &session, &page)?;
    let Some(observation) = observation(
        &inflight.participation,
        operation,
        session,
        page,
        composed_output,
    ) else {
        return Ok(fenced(
            &inflight,
            AgentFenceCauseV1::InvalidProviderEvidence,
        ));
    };
    applied(
        service
            .execute_participation(participation::CommandV1::Apply {
                expected_storage_revision: Some(inflight.storage_revision),
                mutation: Box::new(participation::MutationV1::Observe {
                    participation: inflight.participation.binding.participation,
                    expected_revision: inflight.participation.revision,
                    key: operation_key(operation),
                    observation,
                }),
            })
            .map_err(AgentDriveErrorV1::Service)?,
    )
    .map(
        |(storage_revision, participation)| AgentDriveOutcomeV1::Observed {
            storage_revision,
            participation,
        },
    )
}

fn verify_current<P, MaterializerError, OutputError>(
    service: &mut P,
    prepared: &PreparedParticipationV1,
) -> Result<(), AgentDriveErrorV1<P::Error, MaterializerError, OutputError>>
where
    P: AgentParticipationServicePort,
{
    let status = service
        .execute_participation(participation::CommandV1::Status)
        .map_err(AgentDriveErrorV1::Service)?;
    match status {
        participation::ReplyV1::Accepted(value) => match *value {
            participation::OutcomeV1::Status(value)
                if value.storage_revision == Some(prepared.storage_revision)
                    && !value.needs_reopen => {}
            _ => return Err(AgentDriveErrorV1::InvalidState),
        },
        participation::ReplyV1::Refused(refusal) => {
            return Err(AgentDriveErrorV1::Refused(refusal));
        }
    }
    let retained = service
        .execute_participation(participation::CommandV1::Participation {
            participation: prepared.participation.binding.participation,
        })
        .map_err(AgentDriveErrorV1::Service)?;
    match retained {
        participation::ReplyV1::Accepted(value) => match *value {
            participation::OutcomeV1::Participation(Some(value))
                if value == prepared.participation =>
            {
                Ok(())
            }
            _ => Err(AgentDriveErrorV1::InvalidState),
        },
        participation::ReplyV1::Refused(refusal) => Err(AgentDriveErrorV1::Refused(refusal)),
    }
}

fn fenced<GatewayError>(
    inflight: &PreparedParticipationV1,
    cause: AgentFenceCauseV1<GatewayError>,
) -> AgentDriveOutcomeV1<GatewayError> {
    AgentDriveOutcomeV1::Fenced {
        storage_revision: inflight.storage_revision,
        participation: inflight.participation.clone(),
        cause,
    }
}

fn applied<ServiceError, MaterializerError, OutputError>(
    reply: participation::ReplyV1,
) -> Result<
    (u64, participation::ParticipationViewV1),
    AgentDriveErrorV1<ServiceError, MaterializerError, OutputError>,
> {
    match reply {
        participation::ReplyV1::Accepted(value) => match *value {
            participation::OutcomeV1::Applied {
                storage_revision,
                participation,
                ..
            } => Ok((storage_revision, participation)),
            _ => Err(AgentDriveErrorV1::InvalidState),
        },
        participation::ReplyV1::Refused(refusal) => Err(AgentDriveErrorV1::Refused(refusal)),
    }
}

fn valid_projection(
    view: &participation::ParticipationViewV1,
    operation: participation::OperationV1,
    command: &agent::CommandV1,
    now: u64,
) -> bool {
    let session = agent::SessionIdV1(view.binding.session.0);
    let revision = view.session_revision;
    match (operation, command) {
        (
            participation::OperationV1::Open { key, execution },
            agent::CommandV1::Open {
                request,
                now: command_now,
            },
        ) => {
            *command_now == now
                && request.session == session
                && valid_execution(
                    view,
                    &request.execution,
                    execution,
                    key,
                    view.binding.attempt,
                )
        }
        (
            participation::OperationV1::Prompt {
                key,
                turn,
                execution,
                prompt_digest,
                attempt,
            },
            agent::CommandV1::Prompt {
                request,
                now: command_now,
            },
        ) => {
            *command_now == now
                && request.session == session
                && request.turn.0 == turn.0
                && Some(request.expected_revision) == revision
                && request.prompt_digest.0 == prompt_digest
                && <[u8; 32]>::from(Sha256::digest(&request.prompt)) == prompt_digest
                && valid_execution(view, &request.execution, execution, key, attempt)
        }
        (
            participation::OperationV1::DecidePermission {
                key,
                execution,
                request,
                option,
                attempt,
                ..
            },
            agent::CommandV1::DecidePermission {
                session: command_session,
                expected_revision,
                execution: command_execution,
                decision,
                now: command_now,
            },
        ) => {
            *command_now == now
                && *command_session == session
                && Some(*expected_revision) == revision
                && decision.request.0 == request.0
                && decision.option.0 == option.0
                && valid_execution(view, command_execution, execution, key, attempt)
        }
        (
            participation::OperationV1::Advance { key, turn },
            agent::CommandV1::Advance {
                session: command_session,
                turn: command_turn,
                expected_revision,
                idempotency_key,
                now: command_now,
            },
        )
        | (
            participation::OperationV1::Cancel { key, turn },
            agent::CommandV1::Cancel {
                session: command_session,
                turn: command_turn,
                expected_revision,
                idempotency_key,
                now: command_now,
            },
        ) => {
            *command_now == now
                && *command_session == session
                && command_turn.0 == turn.0
                && Some(*expected_revision) == revision
                && idempotency_key.0 == key.0
        }
        (
            participation::OperationV1::Close { key },
            agent::CommandV1::Close {
                session: command_session,
                expected_revision,
                idempotency_key,
                now: command_now,
            },
        ) => {
            *command_now == now
                && *command_session == session
                && Some(*expected_revision) == revision
                && idempotency_key.0 == key.0
        }
        (
            participation::OperationV1::Reattach {
                key,
                execution,
                attempt,
            },
            agent::CommandV1::Reattach {
                session: command_session,
                expected_revision,
                execution: command_execution,
                now: command_now,
            },
        ) => {
            *command_now == now
                && *command_session == session
                && Some(*expected_revision) == revision
                && valid_execution(view, command_execution, execution, key, attempt)
        }
        _ => false,
    }
}

fn valid_execution(
    view: &participation::ParticipationViewV1,
    value: &agent::ExecutionV1,
    execution: participation::ExecutionIdV1,
    key: participation::IdempotencyKeyV1,
    attempt: participation::AttemptV1,
) -> bool {
    value.id.0 == execution.0
        && value.idempotency_key.0 == key.0
        && value.profile.0 == view.binding.profile.0
        && value.attempt.task_key == attempt.task_key
        && value.attempt.incarnation == attempt.incarnation
        && value.attempt.attempt == attempt.attempt
}

fn observation(
    prior: &participation::ParticipationViewV1,
    operation: participation::OperationV1,
    session: agent::SessionViewV1,
    page: agent::EventPageV1,
    composed_output: Option<[u8; 32]>,
) -> Option<participation::ObservationV1> {
    if page.session != session.session || !valid_event_page(prior, &session, &page) {
        return None;
    }
    let turn = match operation {
        participation::OperationV1::DecidePermission { .. } => prior.active_turn,
        _ => operation_turn(operation),
    };
    let permission = page
        .entries
        .iter()
        .rev()
        .find_map(|record| match &record.event {
            agent::EventV1::Permission(value)
                if Some(value.turn.0) == turn.map(|value| value.0) =>
            {
                let (option_count, option_ids) = retained_option_ids(&value.options)?;
                Some(participation::PermissionEvidenceV1 {
                    request: participation::PermissionRequestIdV1(value.request.0),
                    turn: participation::AgentTurnIdV1(value.turn.0),
                    capability_digest: Sha256::digest(&value.capability).into(),
                    options_digest: options_digest(&value.options),
                    option_count,
                    option_ids,
                })
            }
            _ => None,
        });
    let terminal_event = page
        .entries
        .iter()
        .rev()
        .find_map(|record| match record.event {
            agent::EventV1::TurnFinished {
                turn: event_turn,
                disposition,
                stop_reason,
                output_digest,
            } if Some(event_turn.0) == turn.map(|value| value.0) => {
                Some((event_turn, disposition, stop_reason, output_digest))
            }
            _ => None,
        });
    let (phase, active_turn, permission, terminal) = match session.phase {
        agent::PhaseV1::Ready => match operation {
            participation::OperationV1::Open { .. }
            | participation::OperationV1::Reattach { .. } => {
                (participation::PhaseV1::Ready, None, None, None)
            }
            _ => {
                let (turn, disposition, stop_reason, provider_output_digest) = terminal_event?;
                let output_digest = composed_output?;
                if provider_output_digest.is_some_and(|value| value.0 != output_digest) {
                    return None;
                }
                (
                    participation::PhaseV1::Terminal,
                    None,
                    None,
                    Some(participation::TerminalEvidenceV1 {
                        turn: participation::AgentTurnIdV1(turn.0),
                        disposition: disposition_view(disposition),
                        stop_reason: stop_view(stop_reason),
                        output_digest,
                        provider_evidence_digest: evidence_digest(&session, &page),
                    }),
                )
            }
        },
        agent::PhaseV1::TurnActive => (
            participation::PhaseV1::TurnActive,
            session
                .active_turn
                .map(|value| participation::AgentTurnIdV1(value.0)),
            None,
            None,
        ),
        agent::PhaseV1::AwaitingPermission => (
            participation::PhaseV1::AwaitingPermission,
            session
                .active_turn
                .map(|value| participation::AgentTurnIdV1(value.0)),
            Some(permission?),
            None,
        ),
        agent::PhaseV1::Closed if matches!(operation, participation::OperationV1::Close { .. }) => {
            (participation::PhaseV1::Closed, None, None, None)
        }
        agent::PhaseV1::Opening
        | agent::PhaseV1::OutcomeUnknown
        | agent::PhaseV1::Closed
        | agent::PhaseV1::ReattachRequired => {
            return None;
        }
    };
    Some(participation::ObservationV1 {
        phase,
        session_revision: session.revision,
        active_turn,
        last_event_cursor: session.last_event_cursor,
        profile_document_digest: session.profile_document_digest.0,
        capabilities: capabilities_view(session.capabilities),
        permission,
        terminal,
    })
}

fn compose_output<O: AgentOutputStoreV1, ServiceError, MaterializerError>(
    outputs: &mut O,
    prior: &participation::ParticipationViewV1,
    operation: participation::OperationV1,
    session: &agent::SessionViewV1,
    page: &agent::EventPageV1,
) -> Result<Option<[u8; 32]>, AgentDriveErrorV1<ServiceError, MaterializerError, O::Error>> {
    let turn = match operation {
        participation::OperationV1::Prompt { turn, .. }
        | participation::OperationV1::Advance { turn, .. }
        | participation::OperationV1::Cancel { turn, .. } => Some(turn),
        participation::OperationV1::DecidePermission { .. } => prior.active_turn,
        participation::OperationV1::Open { .. }
        | participation::OperationV1::Close { .. }
        | participation::OperationV1::Reattach { .. } => None,
    };
    let Some(turn) = turn else {
        return Ok(None);
    };
    if page.entries.is_empty() {
        return outputs
            .get(prior.binding.participation.0, turn.0)
            .map_err(AgentDriveErrorV1::Output)
            .map(|value| value.map(|(record, _)| record.output_digest));
    }
    let through = session
        .last_event_cursor
        .ok_or(AgentDriveErrorV1::InvalidOutput)?;
    let assistant_len = page
        .entries
        .iter()
        .filter_map(|record| match &record.event {
            agent::EventV1::AssistantMessage { bytes } => Some(bytes.len()),
            _ => None,
        })
        .try_fold(0_usize, |total, length| total.checked_add(length))
        .ok_or(AgentDriveErrorV1::InvalidOutput)?;
    let mut assistant = Vec::new();
    assistant
        .try_reserve_exact(assistant_len)
        .map_err(|_| AgentDriveErrorV1::InvalidOutput)?;
    for record in &page.entries {
        if let agent::EventV1::AssistantMessage { bytes } = &record.event {
            assistant.extend_from_slice(bytes);
        }
    }
    let commit = outputs
        .append(
            prior.binding.participation.0,
            turn.0,
            prior.last_event_cursor,
            through,
            evidence_digest(session, page),
            &assistant,
        )
        .map_err(AgentDriveErrorV1::Output)?;
    if commit.record.participation != prior.binding.participation.0
        || commit.record.turn != turn.0
        || commit.record.last_cursor != through
        || commit.record.output_digest == [0; 32]
    {
        return Err(AgentDriveErrorV1::InvalidOutput);
    }
    Ok(Some(commit.record.output_digest))
}

fn valid_event_page(
    prior: &participation::ParticipationViewV1,
    session: &agent::SessionViewV1,
    page: &agent::EventPageV1,
) -> bool {
    let mut previous = prior.last_event_cursor;
    for record in &page.entries {
        let expected = match previous {
            Some(value) => value.checked_add(1),
            None => Some(0),
        };
        if Some(record.cursor) != expected {
            return false;
        }
        previous = Some(record.cursor);
    }
    previous == session.last_event_cursor && page.next_after.is_none()
}

fn options_digest(values: &[agent::PermissionOptionV1]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRAOPT1");
    hash.update((values.len() as u64).to_be_bytes());
    for value in values {
        hash.update(value.id.0);
        hash.update([match value.kind {
            agent::PermissionOptionKindV1::AllowOnce => 0,
            agent::PermissionOptionKindV1::AllowAlways => 1,
            agent::PermissionOptionKindV1::RejectOnce => 2,
            agent::PermissionOptionKindV1::RejectAlways => 3,
        }]);
        hash.update((value.label.len() as u64).to_be_bytes());
        hash.update(&value.label);
    }
    hash.finalize().into()
}

fn retained_option_ids(
    values: &[agent::PermissionOptionV1],
) -> Option<(
    u8,
    [participation::PermissionOptionIdV1; participation::MAX_PERMISSION_OPTIONS_V1],
)> {
    if values.is_empty() || values.len() > participation::MAX_PERMISSION_OPTIONS_V1 {
        return None;
    }
    let mut retained =
        [participation::PermissionOptionIdV1([0; 16]); participation::MAX_PERMISSION_OPTIONS_V1];
    for (index, value) in values.iter().enumerate() {
        let id = participation::PermissionOptionIdV1(value.id.0);
        if id.0 == [0; 16] || retained[..index].contains(&id) {
            return None;
        }
        retained[index] = id;
    }
    Some((values.len().try_into().ok()?, retained))
}

fn evidence_digest(session: &agent::SessionViewV1, page: &agent::EventPageV1) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRAEV2");
    hash.update(session.session.0);
    hash.update(session.revision.to_be_bytes());
    hash.update(session.profile_document_digest.0);
    hash_context_receipt(&mut hash, session.last_context.as_ref());
    hash.update((page.entries.len() as u64).to_be_bytes());
    for record in &page.entries {
        hash.update(record.cursor.to_be_bytes());
        hash_event(&mut hash, &record.event);
    }
    hash.finalize().into()
}

fn hash_context_receipt(hash: &mut Sha256, value: Option<&agent::ContextReceiptV1>) {
    let Some(value) = value else {
        hash.update([0]);
        return;
    };
    hash.update([1]);
    hash.update(value.execution.0);
    hash.update(value.manifest_digest.0);
    hash.update((value.entries.len() as u64).to_be_bytes());
    for entry in &value.entries {
        hash.update(entry.reference.0);
        hash.update([match entry.kind {
            agent::ContextKindV1::AgoraMessage => 0,
            agent::ContextKindV1::ChronosWork => 1,
            agent::ContextKindV1::VellumDocument => 2,
            agent::ContextKindV1::DendriteGrounding => 3,
        }]);
        hash.update(entry.source_digest.0);
        hash.update((entry.content_bytes as u64).to_be_bytes());
        match entry.grounding {
            None => hash.update([0]),
            Some(grounding) => {
                hash.update([1]);
                hash.update([match grounding.depth {
                    agent::GroundingDepthV1::Ambient => 0,
                    agent::GroundingDepthV1::Activated => 1,
                    agent::GroundingDepthV1::Reference => 2,
                }]);
                hash.update(grounding.grounding_digest.0);
                hash.update(grounding.surface_digest.0);
            }
        }
    }
}

#[cfg(test)]
mod context_evidence_tests {
    use super::*;

    fn session(source_digest: [u8; 32]) -> agent::SessionViewV1 {
        agent::SessionViewV1 {
            session: agent::SessionIdV1([1; 16]),
            revision: 2,
            phase: agent::PhaseV1::Ready,
            profile_document_digest: agent::DigestV1([3; 32]),
            active_turn: None,
            last_event_cursor: None,
            last_context: Some(agent::ContextReceiptV1 {
                execution: agent::ExecutionIdV1([4; 16]),
                manifest_digest: agent::DigestV1([5; 32]),
                entries: vec![agent::ContextManifestEntryV1 {
                    reference: agent::ContextIdV1([6; 16]),
                    kind: agent::ContextKindV1::VellumDocument,
                    source_digest: agent::DigestV1(source_digest),
                    content_bytes: 21,
                    grounding: None,
                }],
            }),
            capabilities: agent::CapabilitiesV1::default(),
        }
    }

    #[test]
    fn evidence_digest_binds_exact_context_source() {
        let page = agent::EventPageV1 {
            session: agent::SessionIdV1([1; 16]),
            entries: Vec::new(),
            next_after: None,
        };
        assert_ne!(
            evidence_digest(&session([7; 32]), &page),
            evidence_digest(&session([8; 32]), &page)
        );
    }
}

fn hash_event(hash: &mut Sha256, value: &agent::EventV1) {
    match value {
        agent::EventV1::AssistantMessage { bytes } => hash_bytes(hash, 0, bytes),
        agent::EventV1::Thought { bytes } => hash_bytes(hash, 1, bytes),
        agent::EventV1::Plan { bytes } => hash_bytes(hash, 2, bytes),
        agent::EventV1::ToolCall { title, detail } => {
            hash_bytes(hash, 3, title);
            hash_bytes(hash, 3, detail);
        }
        agent::EventV1::Terminal { stream, bytes } => {
            hash_bytes(hash, 4, stream);
            hash_bytes(hash, 4, bytes);
        }
        agent::EventV1::Permission(value) => {
            hash.update([5]);
            hash.update(value.request.0);
            hash.update(value.turn.0);
            hash.update(Sha256::digest(&value.capability));
            hash.update(options_digest(&value.options));
        }
        agent::EventV1::FileChange {
            path,
            content_digest,
        } => {
            hash_bytes(hash, 6, path);
            hash.update(content_digest.0);
        }
        agent::EventV1::Usage {
            input_tokens,
            output_tokens,
        } => {
            hash.update([7]);
            hash.update(input_tokens.to_be_bytes());
            hash.update(output_tokens.to_be_bytes());
        }
        agent::EventV1::TurnFinished {
            turn,
            disposition,
            stop_reason,
            output_digest,
        } => {
            hash.update([8]);
            hash.update(turn.0);
            hash.update([disposition_tag(*disposition), stop_tag(*stop_reason)]);
            hash.update(output_digest.map_or([0; 32], |value| value.0));
        }
        agent::EventV1::FileRead {
            path,
            content_digest,
            content_bytes,
        } => {
            hash_bytes(hash, 9, path);
            hash.update(content_digest.0);
            hash.update((*content_bytes as u64).to_be_bytes());
        }
    }
}

fn hash_bytes(hash: &mut Sha256, tag: u8, value: &[u8]) {
    hash.update([tag]);
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

fn operation_key(value: participation::OperationV1) -> participation::IdempotencyKeyV1 {
    match value {
        participation::OperationV1::Open { key, .. }
        | participation::OperationV1::Prompt { key, .. }
        | participation::OperationV1::Advance { key, .. }
        | participation::OperationV1::DecidePermission { key, .. }
        | participation::OperationV1::Cancel { key, .. }
        | participation::OperationV1::Close { key }
        | participation::OperationV1::Reattach { key, .. } => key,
    }
}

fn operation_turn(value: participation::OperationV1) -> Option<participation::AgentTurnIdV1> {
    match value {
        participation::OperationV1::Prompt { turn, .. }
        | participation::OperationV1::Advance { turn, .. }
        | participation::OperationV1::Cancel { turn, .. } => Some(turn),
        participation::OperationV1::DecidePermission { .. }
        | participation::OperationV1::Open { .. }
        | participation::OperationV1::Close { .. }
        | participation::OperationV1::Reattach { .. } => None,
    }
}

fn capabilities_view(value: agent::CapabilitiesV1) -> participation::CapabilitiesV1 {
    participation::CapabilitiesV1 {
        load_session: value.load_session,
        resume_session: value.resume_session,
        permissions: value.permissions,
        filesystem_read: value.filesystem_read,
        filesystem_write: value.filesystem_write,
        terminal: value.terminal,
        session_modes: value.session_modes,
        session_models: value.session_models,
    }
}

fn disposition_view(value: agent::DispositionV1) -> participation::DispositionV1 {
    match value {
        agent::DispositionV1::Completed => participation::DispositionV1::Completed,
        agent::DispositionV1::Failed => participation::DispositionV1::Failed,
        agent::DispositionV1::Cancelled => participation::DispositionV1::Cancelled,
        agent::DispositionV1::TimedOut => participation::DispositionV1::TimedOut,
        agent::DispositionV1::Uncertain => participation::DispositionV1::Uncertain,
    }
}

fn stop_view(value: agent::StopReasonV1) -> participation::StopReasonV1 {
    match value {
        agent::StopReasonV1::EndTurn => participation::StopReasonV1::EndTurn,
        agent::StopReasonV1::Cancelled => participation::StopReasonV1::Cancelled,
        agent::StopReasonV1::TimedOut => participation::StopReasonV1::TimedOut,
        agent::StopReasonV1::Failed => participation::StopReasonV1::Failed,
        agent::StopReasonV1::Disconnected => participation::StopReasonV1::Disconnected,
        agent::StopReasonV1::OutcomeUnknown => participation::StopReasonV1::OutcomeUnknown,
    }
}

fn disposition_tag(value: agent::DispositionV1) -> u8 {
    match value {
        agent::DispositionV1::Completed => 0,
        agent::DispositionV1::Failed => 1,
        agent::DispositionV1::Cancelled => 2,
        agent::DispositionV1::TimedOut => 3,
        agent::DispositionV1::Uncertain => 4,
    }
}

fn stop_tag(value: agent::StopReasonV1) -> u8 {
    match value {
        agent::StopReasonV1::EndTurn => 0,
        agent::StopReasonV1::Cancelled => 1,
        agent::StopReasonV1::TimedOut => 2,
        agent::StopReasonV1::Failed => 3,
        agent::StopReasonV1::Disconnected => 4,
        agent::StopReasonV1::OutcomeUnknown => 5,
    }
}
