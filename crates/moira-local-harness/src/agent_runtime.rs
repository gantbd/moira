//! Crash-conservative dispatch of one durably prepared agent operation.
//!
//! Deployment composition supplies prompt bytes, budgets, context and tools,
//! but every identity carried to Daimon is checked against the retained Moira
//! participation before dispatch. Any ambiguity after dispatch is fenced and
//! is never converted into permission to retry.

use moira_client::AgentParticipationServicePort;
use moira_host::AgentGateway;
use moira_protocol::{agent, agent_participation as participation};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedParticipationV1 {
    pub storage_revision: u64,
    pub participation: participation::ParticipationViewV1,
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

#[derive(Debug, Eq, PartialEq)]
pub enum AgentDriveErrorV1<ServiceError, MaterializerError> {
    Service(ServiceError),
    Refused(participation::RefusalV1),
    InvalidState,
    Materializer(MaterializerError),
    InvalidProjection,
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

pub type AgentDriveResultV1<P, G, M> = Result<
    AgentDriveOutcomeV1<<G as AgentGateway>::Error>,
    AgentDriveErrorV1<
        <P as AgentParticipationServicePort>::Error,
        <M as AgentCommandMaterializerV1>::Error,
    >,
>;

pub type AgentReconcileResultV1<P, G> = Result<
    AgentDriveOutcomeV1<<G as AgentGateway>::Error>,
    AgentDriveErrorV1<<P as AgentParticipationServicePort>::Error, std::convert::Infallible>,
>;

/// Dispatch exactly one already-prepared operation. The prepared receipt is
/// required so a terminal host cannot synthesize a dispatch from a read view.
pub fn dispatch_prepared<P, G, M>(
    service: &mut P,
    gateway: &mut G,
    materializer: &mut M,
    prepared: PreparedParticipationV1,
    now: u64,
    max_event_observations: u64,
) -> AgentDriveResultV1<P, G, M>
where
    P: AgentParticipationServicePort,
    G: AgentGateway,
    M: AgentCommandMaterializerV1,
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
    let Some(observation) = observation(&inflight.participation, operation, session, page) else {
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
pub fn reconcile_fenced<P, G>(
    service: &mut P,
    gateway: &mut G,
    inflight: PreparedParticipationV1,
    max_event_observations: u64,
) -> AgentReconcileResultV1<P, G>
where
    P: AgentParticipationServicePort,
    G: AgentGateway,
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
    let Some(observation) = observation(&inflight.participation, operation, session, page) else {
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

fn verify_current<P, MaterializerError>(
    service: &mut P,
    prepared: &PreparedParticipationV1,
) -> Result<(), AgentDriveErrorV1<P::Error, MaterializerError>>
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

fn applied<ServiceError, MaterializerError>(
    reply: participation::ReplyV1,
) -> Result<
    (u64, participation::ParticipationViewV1),
    AgentDriveErrorV1<ServiceError, MaterializerError>,
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
            participation::OperationV1::Open { .. } => {
                (participation::PhaseV1::Ready, None, None, None)
            }
            _ => {
                let (turn, disposition, stop_reason, output_digest) = terminal_event?;
                (
                    participation::PhaseV1::Terminal,
                    None,
                    None,
                    Some(participation::TerminalEvidenceV1 {
                        turn: participation::AgentTurnIdV1(turn.0),
                        disposition: disposition_view(disposition),
                        stop_reason: stop_view(stop_reason),
                        output_digest: output_digest.map_or_else(no_output_digest, |value| value.0),
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
        agent::PhaseV1::Opening | agent::PhaseV1::OutcomeUnknown | agent::PhaseV1::Closed => {
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
    hash.update(b"MOIRAEV1");
    hash.update(session.session.0);
    hash.update(session.revision.to_be_bytes());
    hash.update(session.profile_document_digest.0);
    hash.update((page.entries.len() as u64).to_be_bytes());
    for record in &page.entries {
        hash.update(record.cursor.to_be_bytes());
        hash_event(&mut hash, &record.event);
    }
    hash.finalize().into()
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
    }
}

fn hash_bytes(hash: &mut Sha256, tag: u8, value: &[u8]) {
    hash.update([tag]);
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

fn no_output_digest() -> [u8; 32] {
    Sha256::digest(b"MOIRA-NO-OUTPUT-V1").into()
}

fn operation_key(value: participation::OperationV1) -> participation::IdempotencyKeyV1 {
    match value {
        participation::OperationV1::Open { key, .. }
        | participation::OperationV1::Prompt { key, .. }
        | participation::OperationV1::Advance { key, .. }
        | participation::OperationV1::DecidePermission { key, .. }
        | participation::OperationV1::Cancel { key, .. }
        | participation::OperationV1::Close { key } => key,
    }
}

fn operation_turn(value: participation::OperationV1) -> Option<participation::AgentTurnIdV1> {
    match value {
        participation::OperationV1::Prompt { turn, .. }
        | participation::OperationV1::Advance { turn, .. }
        | participation::OperationV1::Cancel { turn, .. } => Some(turn),
        participation::OperationV1::DecidePermission { .. }
        | participation::OperationV1::Open { .. }
        | participation::OperationV1::Close { .. } => None,
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
