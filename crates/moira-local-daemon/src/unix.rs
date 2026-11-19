//! Private Unix listener with explicit authentication and owned-path cleanup.

use crate::{
    ConnectionErrorV1, LocalCommand, LocalReply, serve_agent_participation_up_to,
    serve_agent_up_to, serve_deliberation_up_to, serve_local_up_to, serve_saga_up_to,
    serve_task_agent_journey_up_to,
};
use keystone_unix_listener::OwnedUnixListener;
use moira_host::{
    AgentGateway, AgentParticipationStore, AgentPromptCommitV1, AgentPromptStoreV1,
    AttemptReceiptStore, CoordinatorStore, DeliberationStore, SagaStore, TaskAgentJourneyStore,
};
use moira_protocol::attempt_lookup::{AttemptLookup, AttemptLookupRequestV1};
use moira_protocol::{OperatorOperationV1, SagaCommandV1, agent_participation as participation};
use moira_service::{
    AccessV1, Service,
    agent::AgentService,
    agent_participation::{AgentParticipationService, PermissionAccessV1, PromptSubmissionErrorV1},
    attempt_lookup::{AttemptLookupPolicy, ExactAttempt},
    deliberation::{DeliberationService, HumanGateAccessV1},
    saga::{OperatorAccessV1, SagaService},
    task_agent_journey::{
        ErrorV1 as TaskAgentJourneyServiceErrorV1, TaskAgentJourneyService, TrustedTimeV1,
    },
};
use moira_wire::{LimitsV1, ReadyV1, encode_ready};
use std::{
    io::{self, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

pub trait Authenticator {
    type Error;
    fn authenticate(&mut self, stream: &UnixStream) -> Result<AccessV1, Self::Error>;

    /// Command-scoped named-human attribution for one exact deliberation gate
    /// response. Ordinary/read-only authenticators deny this by default.
    fn authorize_human_gate(
        &mut self,
        _operation: moira_protocol::deliberation::HumanGateOperationV1,
    ) -> Result<Option<HumanGateAccessV1>, Self::Error> {
        Ok(None)
    }
}

/// Connection authentication plus command-scoped operator authorization for
/// the saga profile. Implementations retain any session credential internally;
/// neither credentials nor grant handles enter protocol DTOs or durable state.
pub trait SagaAuthenticator {
    type Error;
    fn authenticate(&mut self, stream: &UnixStream) -> Result<AccessV1, Self::Error>;
    fn authorize_operator(
        &mut self,
        operation: OperatorOperationV1,
    ) -> Result<Option<OperatorAccessV1>, Self::Error>;
}

/// Connection authentication plus command-scoped authorization for an exact
/// provider permission selection. Grant handles remain adapter-private.
pub trait AgentParticipationAuthenticator {
    type Error;
    fn authenticate(&mut self, stream: &UnixStream) -> Result<AccessV1, Self::Error>;
    fn authorize_permission(
        &mut self,
        selection: participation::PermissionSelectionV1,
    ) -> Result<Option<PermissionAccessV1>, Self::Error>;

    /// Separate worker capability for reading only the prompt bound to an
    /// exact current pending operation. Ordinary terminal authenticators deny
    /// this by default.
    fn authorize_prompt_materialization(
        &mut self,
        _request: participation::PromptMaterializationRequestV1,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HealthV1 {
    pub ready: bool,
    pub active_connections: u64,
    pub accepted_connections: u64,
    pub refused_connections: u64,
    pub outcome_unknown: u64,
    pub saturated: bool,
}

#[derive(Debug)]
pub enum DaemonErrorV1<A, S> {
    Io(io::Error),
    Authentication(A),
    Connection(ConnectionErrorV1<S>),
}

#[derive(Debug)]
pub enum SagaDispatchErrorV1<A, S> {
    Authorization(A),
    Service(S),
}

#[derive(Debug)]
pub enum SagaDaemonErrorV1<A, S> {
    Io(io::Error),
    Authentication(A),
    Connection(ConnectionErrorV1<SagaDispatchErrorV1<A, S>>),
}

#[derive(Debug)]
pub enum AgentDaemonErrorV1<A, S> {
    Io(io::Error),
    Authentication(A),
    Connection(ConnectionErrorV1<S>),
}

#[derive(Debug)]
pub enum DeliberationDaemonErrorV1<A, S> {
    Io(io::Error),
    Authentication(A),
    Connection(ConnectionErrorV1<DeliberationDispatchErrorV1<A, S>>),
}

#[derive(Debug)]
pub enum DeliberationDispatchErrorV1<A, S> {
    Authorization(A),
    Service(S),
}

#[derive(Debug)]
pub enum AgentParticipationDispatchErrorV1<A, S, P = PromptSubmissionDisabledErrorV1> {
    Authorization(A),
    Service(S),
    PromptStore(P),
    PromptStoreIntegrity,
}

#[derive(Debug)]
pub enum AgentParticipationDaemonErrorV1<A, S, P = PromptSubmissionDisabledErrorV1> {
    Io(io::Error),
    Authentication(A),
    Connection(ConnectionErrorV1<AgentParticipationDispatchErrorV1<A, S, P>>),
}

type AgentParticipationServeResultV1<S, A, P> = Result<
    (),
    AgentParticipationDaemonErrorV1<
        <A as AgentParticipationAuthenticator>::Error,
        <S as AgentParticipationStore>::Error,
        <P as AgentPromptStoreV1>::Error,
    >,
>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptSubmissionDisabledErrorV1;

#[derive(Debug, Default)]
pub struct PromptSubmissionDisabledV1;

impl AgentPromptStoreV1 for PromptSubmissionDisabledV1 {
    type Error = PromptSubmissionDisabledErrorV1;

    fn put(&mut self, _prompt: &[u8]) -> Result<AgentPromptCommitV1, Self::Error> {
        Err(PromptSubmissionDisabledErrorV1)
    }

    fn get(&mut self, _prompt_digest: [u8; 32]) -> Result<Option<Vec<u8>>, Self::Error> {
        Err(PromptSubmissionDisabledErrorV1)
    }
}

pub struct AgentParticipationDaemon<
    S: AgentParticipationStore,
    A: AgentParticipationAuthenticator,
    P: AgentPromptStoreV1 = PromptSubmissionDisabledV1,
> {
    listener: OwnedUnixListener,
    service: AgentParticipationService<S>,
    authenticator: A,
    prompts: P,
    prompt_submission: bool,
    wire_limits: LimitsV1,
    io_timeout: Duration,
    max_requests: usize,
    health: HealthV1,
}

impl<S: AgentParticipationStore, A: AgentParticipationAuthenticator>
    AgentParticipationDaemon<S, A, PromptSubmissionDisabledV1>
{
    pub fn bind_new(
        socket_path: impl AsRef<Path>,
        service: AgentParticipationService<S>,
        authenticator: A,
        wire_limits: LimitsV1,
        io_timeout: Duration,
        max_requests: usize,
    ) -> io::Result<Self> {
        if io_timeout.is_zero() || max_requests == 0 || wire_limits.max_frame_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "disabled limit",
            ));
        }
        Ok(Self {
            listener: OwnedUnixListener::bind_new(socket_path)?,
            service,
            authenticator,
            prompts: PromptSubmissionDisabledV1,
            prompt_submission: false,
            wire_limits,
            io_timeout,
            max_requests,
            health: HealthV1 {
                ready: true,
                ..HealthV1::default()
            },
        })
    }
}

impl<S: AgentParticipationStore, A: AgentParticipationAuthenticator, P: AgentPromptStoreV1>
    AgentParticipationDaemon<S, A, P>
{
    pub fn bind_new_with_prompt_store(
        socket_path: impl AsRef<Path>,
        service: AgentParticipationService<S>,
        authenticator: A,
        prompts: P,
        wire_limits: LimitsV1,
        io_timeout: Duration,
        max_requests: usize,
    ) -> io::Result<Self> {
        if io_timeout.is_zero() || max_requests == 0 || wire_limits.max_frame_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "disabled limit",
            ));
        }
        Ok(Self {
            listener: OwnedUnixListener::bind_new(socket_path)?,
            service,
            authenticator,
            prompts,
            prompt_submission: true,
            wire_limits,
            io_timeout,
            max_requests,
            health: HealthV1 {
                ready: true,
                ..HealthV1::default()
            },
        })
    }

    pub fn health(&self) -> HealthV1 {
        self.health
    }

    pub fn serve_next(&mut self) -> AgentParticipationServeResultV1<S, A, P> {
        let (mut stream, _) = self
            .listener
            .accept()
            .map_err(AgentParticipationDaemonErrorV1::Io)?;
        increment(
            &mut self.health.active_connections,
            &mut self.health.saturated,
        );
        let result = self.serve_stream(&mut stream);
        self.health.active_connections = self.health.active_connections.saturating_sub(1);
        if result.is_err() {
            increment(
                &mut self.health.refused_connections,
                &mut self.health.saturated,
            );
        } else {
            increment(
                &mut self.health.accepted_connections,
                &mut self.health.saturated,
            );
        }
        if matches!(
            &result,
            Err(AgentParticipationDaemonErrorV1::Connection(
                ConnectionErrorV1::OutcomeUnknown(_)
            ))
        ) {
            increment(&mut self.health.outcome_unknown, &mut self.health.saturated);
        }
        result
    }

    fn serve_stream(
        &mut self,
        stream: &mut UnixStream,
    ) -> AgentParticipationServeResultV1<S, A, P> {
        stream
            .set_read_timeout(Some(self.io_timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.io_timeout)))
            .map_err(AgentParticipationDaemonErrorV1::Io)?;
        let access = self
            .authenticator
            .authenticate(stream)
            .map_err(AgentParticipationDaemonErrorV1::Authentication)?;
        write_ready(stream, self.wire_limits, self.max_requests)
            .map_err(AgentParticipationDaemonErrorV1::Io)?;
        serve_agent_participation_up_to(stream, self.wire_limits, self.max_requests, |command| {
            match command {
                participation::CommandV1::PrepareOpen { participation } => self
                    .service
                    .execute_open_authorized(access, participation)
                    .map_err(AgentParticipationDispatchErrorV1::Service),
                participation::CommandV1::ConfirmPermission(confirmed) => {
                    let selection = match self.service.prepare_confirmed_permission(confirmed) {
                        Ok(selection) => selection,
                        Err(refusal) => {
                            return Ok(participation::ReplyV1::Refused(refusal));
                        }
                    };
                    let permission = self
                        .authenticator
                        .authorize_permission(selection)
                        .map_err(AgentParticipationDispatchErrorV1::Authorization)?;
                    match permission {
                        Some(permission) => self
                            .service
                            .execute_permission_authorized(
                                permission,
                                participation::CommandV1::SelectPermission(selection),
                            )
                            .map_err(AgentParticipationDispatchErrorV1::Service),
                        None => Ok(participation::ReplyV1::Refused(
                            participation::RefusalV1::NotAuthorized,
                        )),
                    }
                }
                participation::CommandV1::SelectPermission(selection) => {
                    let permission = self
                        .authenticator
                        .authorize_permission(selection)
                        .map_err(AgentParticipationDispatchErrorV1::Authorization)?;
                    match permission {
                        Some(permission) => self
                            .service
                            .execute_permission_authorized(
                                permission,
                                participation::CommandV1::SelectPermission(selection),
                            )
                            .map_err(AgentParticipationDispatchErrorV1::Service),
                        None => Ok(participation::ReplyV1::Refused(
                            participation::RefusalV1::NotAuthorized,
                        )),
                    }
                }
                participation::CommandV1::SubmitPrompt(submission) => {
                    if !self.prompt_submission {
                        return Ok(participation::ReplyV1::Refused(
                            participation::RefusalV1::NotAuthorized,
                        ));
                    }
                    self.service
                        .execute_prompt_authorized(access, submission, &mut self.prompts)
                        .map_err(|error| match error {
                            PromptSubmissionErrorV1::Service(error) => {
                                AgentParticipationDispatchErrorV1::Service(error)
                            }
                            PromptSubmissionErrorV1::PromptStore(error) => {
                                AgentParticipationDispatchErrorV1::PromptStore(error)
                            }
                            PromptSubmissionErrorV1::PromptStoreIntegrity => {
                                AgentParticipationDispatchErrorV1::PromptStoreIntegrity
                            }
                        })
                }
                participation::CommandV1::MaterializePrompt(request) => {
                    if !self.prompt_submission {
                        return Ok(participation::ReplyV1::Refused(
                            participation::RefusalV1::NotAuthorized,
                        ));
                    }
                    let authorized = self
                        .authenticator
                        .authorize_prompt_materialization(request)
                        .map_err(AgentParticipationDispatchErrorV1::Authorization)?;
                    if !authorized {
                        return Ok(participation::ReplyV1::Refused(
                            participation::RefusalV1::NotAuthorized,
                        ));
                    }
                    self.service
                        .execute_prompt_materialization(request, &mut self.prompts)
                        .map_err(|error| match error {
                            moira_service::agent_participation::PromptMaterializationErrorV1::Service(error) => {
                                AgentParticipationDispatchErrorV1::Service(error)
                            }
                            moira_service::agent_participation::PromptMaterializationErrorV1::PromptStore(error) => {
                                AgentParticipationDispatchErrorV1::PromptStore(error)
                            }
                            moira_service::agent_participation::PromptMaterializationErrorV1::PromptStoreIntegrity => {
                                AgentParticipationDispatchErrorV1::PromptStoreIntegrity
                            }
                        })
                }
                _ => self
                    .service
                    .execute_authorized(access, command)
                    .map_err(AgentParticipationDispatchErrorV1::Service),
            }
        })
        .map(|_| ())
        .map_err(AgentParticipationDaemonErrorV1::Connection)
    }

    pub fn into_prompt_parts(self) -> (AgentParticipationService<S>, A, P) {
        (self.service, self.authenticator, self.prompts)
    }
}

impl<S: AgentParticipationStore, A: AgentParticipationAuthenticator>
    AgentParticipationDaemon<S, A, PromptSubmissionDisabledV1>
{
    pub fn into_parts(self) -> (AgentParticipationService<S>, A) {
        (self.service, self.authenticator)
    }
}

pub struct AgentDaemon<G: AgentGateway, A: Authenticator> {
    listener: OwnedUnixListener,
    service: AgentService<G>,
    authenticator: A,
    wire_limits: LimitsV1,
    io_timeout: Duration,
    max_requests: usize,
    health: HealthV1,
}

impl<G: AgentGateway, A: Authenticator> AgentDaemon<G, A> {
    pub fn bind_new(
        socket_path: impl AsRef<Path>,
        service: AgentService<G>,
        authenticator: A,
        wire_limits: LimitsV1,
        io_timeout: Duration,
        max_requests: usize,
    ) -> io::Result<Self> {
        if io_timeout.is_zero() || max_requests == 0 || wire_limits.max_frame_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "disabled limit",
            ));
        }
        Ok(Self {
            listener: OwnedUnixListener::bind_new(socket_path)?,
            service,
            authenticator,
            wire_limits,
            io_timeout,
            max_requests,
            health: HealthV1 {
                ready: true,
                ..HealthV1::default()
            },
        })
    }

    pub fn health(&self) -> HealthV1 {
        self.health
    }

    pub fn serve_next(&mut self) -> Result<(), AgentDaemonErrorV1<A::Error, G::Error>> {
        let (mut stream, _) = self.listener.accept().map_err(AgentDaemonErrorV1::Io)?;
        increment(
            &mut self.health.active_connections,
            &mut self.health.saturated,
        );
        let result = self.serve_stream(&mut stream);
        self.health.active_connections = self.health.active_connections.saturating_sub(1);
        if result.is_err() {
            increment(
                &mut self.health.refused_connections,
                &mut self.health.saturated,
            );
        } else {
            increment(
                &mut self.health.accepted_connections,
                &mut self.health.saturated,
            );
        }
        if matches!(
            &result,
            Err(AgentDaemonErrorV1::Connection(
                ConnectionErrorV1::OutcomeUnknown(_)
            ))
        ) {
            increment(&mut self.health.outcome_unknown, &mut self.health.saturated);
        }
        result
    }

    fn serve_stream(
        &mut self,
        stream: &mut UnixStream,
    ) -> Result<(), AgentDaemonErrorV1<A::Error, G::Error>> {
        stream
            .set_read_timeout(Some(self.io_timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.io_timeout)))
            .map_err(AgentDaemonErrorV1::Io)?;
        let access = self
            .authenticator
            .authenticate(stream)
            .map_err(AgentDaemonErrorV1::Authentication)?;
        write_ready(stream, self.wire_limits, self.max_requests).map_err(AgentDaemonErrorV1::Io)?;
        serve_agent_up_to(stream, self.wire_limits, self.max_requests, |command| {
            self.service.execute_authorized(access, command)
        })
        .map(|_| ())
        .map_err(AgentDaemonErrorV1::Connection)
    }

    pub fn into_parts(self) -> (AgentService<G>, A) {
        (self.service, self.authenticator)
    }
}

/// Private deliberation owner projection. Authentication yields only the
/// service access facts; it cannot manufacture proof, Pareto selection, or
/// action authority.
pub struct DeliberationDaemon<S: DeliberationStore, A: Authenticator> {
    listener: OwnedUnixListener,
    service: DeliberationService<S>,
    authenticator: A,
    wire_limits: LimitsV1,
    io_timeout: Duration,
    max_requests: usize,
    health: HealthV1,
}

impl<S: DeliberationStore, A: Authenticator> DeliberationDaemon<S, A> {
    pub fn bind_new(
        socket_path: impl AsRef<Path>,
        service: DeliberationService<S>,
        authenticator: A,
        wire_limits: LimitsV1,
        io_timeout: Duration,
        max_requests: usize,
    ) -> io::Result<Self> {
        if io_timeout.is_zero() || max_requests == 0 || wire_limits.max_frame_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "disabled limit",
            ));
        }
        Ok(Self {
            listener: OwnedUnixListener::bind_new(socket_path)?,
            service,
            authenticator,
            wire_limits,
            io_timeout,
            max_requests,
            health: HealthV1 {
                ready: true,
                ..HealthV1::default()
            },
        })
    }

    pub fn health(&self) -> HealthV1 {
        self.health
    }

    pub fn serve_next(&mut self) -> Result<(), DeliberationDaemonErrorV1<A::Error, S::Error>> {
        let (mut stream, _) = self
            .listener
            .accept()
            .map_err(DeliberationDaemonErrorV1::Io)?;
        increment(
            &mut self.health.active_connections,
            &mut self.health.saturated,
        );
        let result = self.serve_stream(&mut stream);
        self.health.active_connections = self.health.active_connections.saturating_sub(1);
        if result.is_err() {
            increment(
                &mut self.health.refused_connections,
                &mut self.health.saturated,
            );
        } else {
            increment(
                &mut self.health.accepted_connections,
                &mut self.health.saturated,
            );
        }
        if matches!(
            &result,
            Err(DeliberationDaemonErrorV1::Connection(
                ConnectionErrorV1::OutcomeUnknown(_)
            ))
        ) {
            increment(&mut self.health.outcome_unknown, &mut self.health.saturated);
        }
        result
    }

    fn serve_stream(
        &mut self,
        stream: &mut UnixStream,
    ) -> Result<(), DeliberationDaemonErrorV1<A::Error, S::Error>> {
        stream
            .set_read_timeout(Some(self.io_timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.io_timeout)))
            .map_err(DeliberationDaemonErrorV1::Io)?;
        let access = self
            .authenticator
            .authenticate(stream)
            .map_err(DeliberationDaemonErrorV1::Authentication)?;
        write_ready(stream, self.wire_limits, self.max_requests)
            .map_err(DeliberationDaemonErrorV1::Io)?;
        serve_deliberation_up_to(stream, self.wire_limits, self.max_requests, |command| {
            match command {
                moira_protocol::deliberation::CommandV1::RespondHumanGate { operation } => {
                    let Some(verified) = self
                        .authenticator
                        .authorize_human_gate(operation)
                        .map_err(DeliberationDispatchErrorV1::Authorization)?
                    else {
                        return Ok(moira_protocol::deliberation::ReplyV1::Refused(
                            moira_protocol::deliberation::RefusalV1::NotAuthorized,
                        ));
                    };
                    self.service
                        .execute_human_gate_authorized(access, verified, operation)
                        .map_err(DeliberationDispatchErrorV1::Service)
                }
                command => self
                    .service
                    .execute_authorized(access, command)
                    .map_err(DeliberationDispatchErrorV1::Service),
            }
        })
        .map(|_| ())
        .map_err(DeliberationDaemonErrorV1::Connection)
    }

    pub fn into_parts(self) -> (DeliberationService<S>, A) {
        (self.service, self.authenticator)
    }
}

/// Private task-agent journey admission owner. Authentication supplies only
/// read/mutate access; named-human confirmation remains explicit protocol data
/// validated against the owner-issued preview and grants no peer authority.
pub struct TaskAgentJourneyDaemon<S: TaskAgentJourneyStore, T: TrustedTimeV1, A: Authenticator> {
    listener: OwnedUnixListener,
    service: TaskAgentJourneyService<S, T>,
    authenticator: A,
    wire_limits: LimitsV1,
    io_timeout: Duration,
    max_requests: usize,
    health: HealthV1,
}

pub type TaskAgentJourneyDaemonErrorV1<A, S, T> = DaemonErrorV1<
    <A as Authenticator>::Error,
    TaskAgentJourneyServiceErrorV1<
        <S as TaskAgentJourneyStore>::Error,
        <T as TrustedTimeV1>::Error,
    >,
>;
type TaskAgentJourneyDaemonResultV1<S, T, A> = Result<(), TaskAgentJourneyDaemonErrorV1<A, S, T>>;

impl<S: TaskAgentJourneyStore, T: TrustedTimeV1, A: Authenticator> TaskAgentJourneyDaemon<S, T, A> {
    pub fn bind_new(
        socket_path: impl AsRef<Path>,
        service: TaskAgentJourneyService<S, T>,
        authenticator: A,
        wire_limits: LimitsV1,
        io_timeout: Duration,
        max_requests: usize,
    ) -> io::Result<Self> {
        if io_timeout.is_zero() || max_requests == 0 || wire_limits.max_frame_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "disabled limit",
            ));
        }
        Ok(Self {
            listener: OwnedUnixListener::bind_new(socket_path)?,
            service,
            authenticator,
            wire_limits,
            io_timeout,
            max_requests,
            health: HealthV1 {
                ready: true,
                ..HealthV1::default()
            },
        })
    }

    pub fn health(&self) -> HealthV1 {
        self.health
    }

    pub fn serve_next(&mut self) -> TaskAgentJourneyDaemonResultV1<S, T, A> {
        let (mut stream, _) = self.listener.accept().map_err(DaemonErrorV1::Io)?;
        increment(
            &mut self.health.active_connections,
            &mut self.health.saturated,
        );
        let result = self.serve_stream(&mut stream);
        self.health.active_connections = self.health.active_connections.saturating_sub(1);
        if result.is_err() {
            increment(
                &mut self.health.refused_connections,
                &mut self.health.saturated,
            );
        } else {
            increment(
                &mut self.health.accepted_connections,
                &mut self.health.saturated,
            );
        }
        if matches!(
            &result,
            Err(DaemonErrorV1::Connection(
                ConnectionErrorV1::OutcomeUnknown(_)
            ))
        ) {
            increment(&mut self.health.outcome_unknown, &mut self.health.saturated);
        }
        result
    }

    fn serve_stream(&mut self, stream: &mut UnixStream) -> TaskAgentJourneyDaemonResultV1<S, T, A> {
        stream
            .set_read_timeout(Some(self.io_timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.io_timeout)))
            .map_err(DaemonErrorV1::Io)?;
        let access = self
            .authenticator
            .authenticate(stream)
            .map_err(DaemonErrorV1::Authentication)?;
        write_ready(stream, self.wire_limits, self.max_requests).map_err(DaemonErrorV1::Io)?;
        serve_task_agent_journey_up_to(stream, self.wire_limits, self.max_requests, |command| {
            self.service.execute_authorized(access, command)
        })
        .map(|_| ())
        .map_err(DaemonErrorV1::Connection)
    }

    pub fn into_parts(self) -> (TaskAgentJourneyService<S, T>, A) {
        (self.service, self.authenticator)
    }
}

type LookupHandler<S, A> =
    Box<dyn FnMut(&Service<S>, &mut A, AccessV1, &AttemptLookupRequestV1) -> AttemptLookup + Send>;

pub struct Daemon<S: CoordinatorStore, A: Authenticator> {
    listener: OwnedUnixListener,
    service: Service<S>,
    authenticator: A,
    wire_limits: LimitsV1,
    io_timeout: Duration,
    max_requests: usize,
    lookup: Option<LookupHandler<S, A>>,
    health: HealthV1,
}

impl<S: CoordinatorStore, A: Authenticator> Daemon<S, A> {
    pub fn bind_new(
        socket_path: impl AsRef<Path>,
        service: Service<S>,
        authenticator: A,
        wire_limits: LimitsV1,
        io_timeout: Duration,
        max_requests: usize,
    ) -> io::Result<Self> {
        if io_timeout.is_zero() || max_requests == 0 || wire_limits.max_frame_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "disabled limit",
            ));
        }
        let listener = OwnedUnixListener::bind_new(socket_path)?;
        Ok(Self {
            listener,
            service,
            authenticator,
            wire_limits,
            io_timeout,
            max_requests,
            lookup: None,
            health: HealthV1 {
                ready: true,
                ..HealthV1::default()
            },
        })
    }

    pub fn health(&self) -> HealthV1 {
        self.health
    }

    pub fn serve_next(&mut self) -> Result<(), DaemonErrorV1<A::Error, S::Error>> {
        let (mut stream, _) = self.listener.accept().map_err(DaemonErrorV1::Io)?;
        increment(
            &mut self.health.active_connections,
            &mut self.health.saturated,
        );
        let result = self.serve_stream(&mut stream);
        self.health.active_connections = self.health.active_connections.saturating_sub(1);
        if result.is_err() {
            increment(
                &mut self.health.refused_connections,
                &mut self.health.saturated,
            );
        }
        if matches!(
            &result,
            Err(DaemonErrorV1::Connection(
                ConnectionErrorV1::OutcomeUnknown(_)
            ))
        ) {
            increment(&mut self.health.outcome_unknown, &mut self.health.saturated);
        }
        result
    }

    fn serve_stream(
        &mut self,
        stream: &mut UnixStream,
    ) -> Result<(), DaemonErrorV1<A::Error, S::Error>> {
        stream
            .set_read_timeout(Some(self.io_timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.io_timeout)))
            .map_err(DaemonErrorV1::Io)?;
        let access = self
            .authenticator
            .authenticate(stream)
            .map_err(DaemonErrorV1::Authentication)?;
        write_ready(stream, self.wire_limits, self.max_requests).map_err(DaemonErrorV1::Io)?;
        let result =
            serve_local_up_to(
                stream,
                self.wire_limits,
                self.max_requests,
                |command| match command {
                    LocalCommand::Workflow(command) => self
                        .service
                        .execute_authorized(access, command)
                        .map(LocalReply::Workflow),
                    LocalCommand::Attempt(request) => Ok(LocalReply::Attempt(if !access.read {
                        AttemptLookup::Denied
                    } else if let Some(handler) = &mut self.lookup {
                        handler(&self.service, &mut self.authenticator, access, &request)
                    } else {
                        AttemptLookup::Unavailable
                    })),
                },
            )
            .map(|_| ())
            .map_err(DaemonErrorV1::Connection);
        if result.is_ok() {
            increment(
                &mut self.health.accepted_connections,
                &mut self.health.saturated,
            );
        }
        result
    }
}

impl<S: AttemptReceiptStore, A: Authenticator> Daemon<S, A> {
    /// Opt in with a trusted policy built from the authenticated session.
    /// Called for every lookup so the host can apply current disclosure rules.
    pub fn set_attempt_lookup_policy<P: AttemptLookupPolicy>(
        &mut self,
        mut policy_for: impl FnMut(&mut A) -> P + Send + 'static,
    ) {
        self.lookup = Some(Box::new(move |service, authenticator, access, request| {
            let mut policy = policy_for(authenticator);
            service.lookup_attempt(
                access,
                &ExactAttempt {
                    service_scope: &request.service_scope,
                    task: &request.task,
                    incarnation: request.incarnation,
                    attempt: request.attempt,
                },
                &mut policy,
            )
        }));
    }
}

pub struct SagaDaemon<S: SagaStore, A: SagaAuthenticator> {
    listener: OwnedUnixListener,
    service: SagaService<S>,
    authenticator: A,
    wire_limits: LimitsV1,
    io_timeout: Duration,
    max_requests: usize,
    health: HealthV1,
}

impl<S: SagaStore, A: SagaAuthenticator> SagaDaemon<S, A> {
    pub fn bind_new(
        socket_path: impl AsRef<Path>,
        service: SagaService<S>,
        authenticator: A,
        wire_limits: LimitsV1,
        io_timeout: Duration,
        max_requests: usize,
    ) -> io::Result<Self> {
        if io_timeout.is_zero() || max_requests == 0 || wire_limits.max_frame_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "disabled limit",
            ));
        }
        let listener = OwnedUnixListener::bind_new(socket_path)?;
        Ok(Self {
            listener,
            service,
            authenticator,
            wire_limits,
            io_timeout,
            max_requests,
            health: HealthV1 {
                ready: true,
                ..HealthV1::default()
            },
        })
    }

    pub fn health(&self) -> HealthV1 {
        self.health
    }

    pub fn serve_next(&mut self) -> Result<(), SagaDaemonErrorV1<A::Error, S::Error>> {
        let (mut stream, _) = self.listener.accept().map_err(SagaDaemonErrorV1::Io)?;
        increment(
            &mut self.health.active_connections,
            &mut self.health.saturated,
        );
        let result = self.serve_stream(&mut stream);
        self.health.active_connections = self.health.active_connections.saturating_sub(1);
        if result.is_err() {
            increment(
                &mut self.health.refused_connections,
                &mut self.health.saturated,
            );
        }
        if matches!(
            &result,
            Err(SagaDaemonErrorV1::Connection(
                ConnectionErrorV1::OutcomeUnknown(_)
            ))
        ) {
            increment(&mut self.health.outcome_unknown, &mut self.health.saturated);
        }
        result
    }

    fn serve_stream(
        &mut self,
        stream: &mut UnixStream,
    ) -> Result<(), SagaDaemonErrorV1<A::Error, S::Error>> {
        stream
            .set_read_timeout(Some(self.io_timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.io_timeout)))
            .map_err(SagaDaemonErrorV1::Io)?;
        let access = self
            .authenticator
            .authenticate(stream)
            .map_err(SagaDaemonErrorV1::Authentication)?;
        write_ready(stream, self.wire_limits, self.max_requests).map_err(SagaDaemonErrorV1::Io)?;
        let result =
            serve_saga_up_to(
                stream,
                self.wire_limits,
                self.max_requests,
                |command| match command {
                    SagaCommandV1::ConfirmReconciliation(selected) => {
                        let (expected_revision, operation) =
                            match self.service.prepare_confirmed_reconciliation(selected) {
                                Ok(value) => value,
                                Err(refusal) => {
                                    return Ok(moira_protocol::SagaReplyV1::Refused(refusal));
                                }
                            };
                        let operator = self
                            .authenticator
                            .authorize_operator(operation)
                            .map_err(SagaDispatchErrorV1::Authorization)?;
                        match operator {
                            Some(operator) => self
                                .service
                                .execute_operator_authorized(
                                    operator,
                                    SagaCommandV1::Reconcile {
                                        expected_revision,
                                        operation,
                                    },
                                )
                                .map_err(SagaDispatchErrorV1::Service),
                            None => Ok(moira_protocol::SagaReplyV1::Refused(
                                moira_protocol::SagaRefusalV1::NotAuthorized,
                            )),
                        }
                    }
                    // Raw operations remain an internal service command. The
                    // public daemon accepts only a current server-prepared
                    // confirmation, so callers cannot invent action identity.
                    SagaCommandV1::Reconcile { .. } => Ok(moira_protocol::SagaReplyV1::Refused(
                        moira_protocol::SagaRefusalV1::NotAuthorized,
                    )),
                    _ => self
                        .service
                        .execute_authorized(access, command)
                        .map_err(SagaDispatchErrorV1::Service),
                },
            )
            .map(|_| ())
            .map_err(SagaDaemonErrorV1::Connection);
        if result.is_ok() {
            increment(
                &mut self.health.accepted_connections,
                &mut self.health.saturated,
            );
        }
        result
    }
}

fn write_ready(stream: &mut UnixStream, limits: LimitsV1, max_requests: usize) -> io::Result<()> {
    let value = ReadyV1 {
        protocol_version: 1,
        max_frame_bytes: u32::try_from(limits.max_frame_bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame limit"))?,
        max_requests: u32::try_from(max_requests)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "request limit"))?,
    };
    let bytes = encode_ready(value, limits)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "ready frame"))?;
    let length = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "ready frame length"))?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()
}

fn increment(value: &mut u64, saturated: &mut bool) {
    if let Some(next) = value.checked_add(1) {
        *value = next;
    } else {
        *saturated = true;
    }
}
