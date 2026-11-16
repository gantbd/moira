//! Private Unix listener with explicit authentication and owned-path cleanup.

use crate::{
    ConnectionErrorV1, LocalCommand, LocalReply, serve_agent_participation_up_to,
    serve_agent_up_to, serve_local_up_to, serve_saga_up_to,
};
use keystone_unix_listener::OwnedUnixListener;
use moira_host::{
    AgentGateway, AgentParticipationStore, AttemptReceiptStore, CoordinatorStore, SagaStore,
};
use moira_protocol::attempt_lookup::{AttemptLookup, AttemptLookupRequestV1};
use moira_protocol::{OperatorOperationV1, SagaCommandV1, agent_participation as participation};
use moira_service::{
    AccessV1, Service,
    agent::AgentService,
    agent_participation::{AgentParticipationService, PermissionAccessV1},
    attempt_lookup::{AttemptLookupPolicy, ExactAttempt},
    saga::{OperatorAccessV1, SagaService},
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
pub enum AgentParticipationDispatchErrorV1<A, S> {
    Authorization(A),
    Service(S),
}

#[derive(Debug)]
pub enum AgentParticipationDaemonErrorV1<A, S> {
    Io(io::Error),
    Authentication(A),
    Connection(ConnectionErrorV1<AgentParticipationDispatchErrorV1<A, S>>),
}

pub struct AgentParticipationDaemon<S: AgentParticipationStore, A: AgentParticipationAuthenticator>
{
    listener: OwnedUnixListener,
    service: AgentParticipationService<S>,
    authenticator: A,
    wire_limits: LimitsV1,
    io_timeout: Duration,
    max_requests: usize,
    health: HealthV1,
}

impl<S: AgentParticipationStore, A: AgentParticipationAuthenticator>
    AgentParticipationDaemon<S, A>
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

    pub fn serve_next(
        &mut self,
    ) -> Result<(), AgentParticipationDaemonErrorV1<A::Error, S::Error>> {
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
    ) -> Result<(), AgentParticipationDaemonErrorV1<A::Error, S::Error>> {
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
                _ => self
                    .service
                    .execute_authorized(access, command)
                    .map_err(AgentParticipationDispatchErrorV1::Service),
            }
        })
        .map(|_| ())
        .map_err(AgentParticipationDaemonErrorV1::Connection)
    }

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
                    SagaCommandV1::Reconcile { operation, .. } => {
                        let operator = self
                            .authenticator
                            .authorize_operator(operation)
                            .map_err(SagaDispatchErrorV1::Authorization)?;
                        match operator {
                            Some(operator) => self
                                .service
                                .execute_operator_authorized(operator, command)
                                .map_err(SagaDispatchErrorV1::Service),
                            None => Ok(moira_protocol::SagaReplyV1::Refused(
                                moira_protocol::SagaRefusalV1::NotAuthorized,
                            )),
                        }
                    }
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
