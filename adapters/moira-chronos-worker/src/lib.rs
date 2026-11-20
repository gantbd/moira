//! Moira-owned use of Chronos's typed in-process worker profile.
//!
//! Native grants and leases remain inside this adapter. Callers receive only an
//! exact attempt receipt and may settle it only with matching result evidence.

#![forbid(unsafe_code)]

use chronos_engine::{
    Agent, CapabilityGrant, CommandOutcomeV2, EngineTime, LeaseToken, StateRevision, TaskId,
    TaskState,
};
use chronos_host::worker::{WorkerClientV1, WorkerRefusalV1, WorkerRequestV1};
use moira_core::{
    ChronosAttemptReceiptV1, ChronosTaskRefV1,
    task_agent_journey::{JourneyIdV1, JourneySnapshotV1},
};
use moira_host::{
    TaskAgentJourneyApplyErrorV1, TaskAgentJourneyApplyV1, TaskAgentJourneyMutationV1,
    TaskAgentJourneyStore,
};
use moira_protocol::{AttemptReceiptV1, DispositionV1, EffectIdV1, ResultReceiptV1, TaskRefV1};
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationV1 {
    pub agent: Agent,
    pub grant_validity: Duration,
    pub lease_duration: Duration,
}

impl ConfigurationV1 {
    pub fn new(
        agent_id: impl Into<String>,
        grant_validity: Duration,
        lease_duration: Duration,
    ) -> Result<Self, ConfigurationErrorV1> {
        let agent_id = agent_id.into();
        if agent_id.is_empty() || grant_validity.is_zero() || lease_duration.is_zero() {
            return Err(ConfigurationErrorV1::Invalid);
        }
        Ok(Self {
            agent: Agent::new(agent_id),
            grant_validity,
            lease_duration,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationErrorV1 {
    Invalid,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ClaimIntentV1 {
    pub task_id: String,
    pub external_task_key: [u8; 16],
    pub expected_revision: u64,
    pub now: SystemTime,
}

impl std::fmt::Debug for ClaimIntentV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClaimIntentV1")
            .field("task_id", &"<redacted>")
            .field("external_task_key", &"<opaque>")
            .field("expected_revision", &self.expected_revision)
            .field("now", &self.now)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClaimedAttemptV1 {
    pub receipt: AttemptReceiptV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadyTaskAttemptV1 {
    pub claim: ClaimedAttemptV1,
    pub storage_revision: u64,
    pub journey: JourneySnapshotV1,
    pub replayed: bool,
}

#[derive(Debug)]
pub enum JourneyBindingErrorV1<E> {
    NoActiveAttempt,
    Store {
        claim: ClaimedAttemptV1,
        error: TaskAgentJourneyApplyErrorV1<E>,
    },
    UnexpectedOutcome {
        claim: ClaimedAttemptV1,
    },
}

#[derive(Debug)]
pub enum ClaimAndBindErrorV1<W, S> {
    Claim(ErrorV1<W>),
    Bind(JourneyBindingErrorV1<S>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResultBindingV1 {
    pub effect: EffectIdV1,
    pub execution_id: [u8; 16],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettlementV1 {
    Completed,
    FencedUnknown,
}

#[derive(Debug)]
pub enum ErrorV1<E> {
    InvalidConfiguration,
    InvalidIntent,
    AlreadyActive,
    NoActiveAttempt,
    ExecutionAlreadyBound,
    ExecutionNotBound,
    EvidenceMismatch,
    UnsupportedDisposition,
    OwnerRefused(WorkerRefusalV1),
    DomainRefused,
    UnexpectedOutcome,
    OutcomeUnknown(E),
    ResultWithheld,
    Fenced,
}

struct ActiveAttempt {
    task_id: TaskId,
    task: TaskRefV1,
    attempt_number: u32,
    chronos_revision: u64,
    lease: LeaseToken,
    binding: Option<ResultBindingV1>,
}

enum State {
    Idle,
    Active(ActiveAttempt),
    Fenced,
}

pub struct TrustedWorkerV1<C> {
    client: C,
    configuration: ConfigurationV1,
    state: State,
}

impl<C> std::fmt::Debug for TrustedWorkerV1<C> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TrustedWorkerV1")
            .field(
                "state",
                &match self.state {
                    State::Idle => "idle",
                    State::Active(_) => "active",
                    State::Fenced => "fenced",
                },
            )
            .finish_non_exhaustive()
    }
}

impl<C> TrustedWorkerV1<C> {
    pub fn new(
        client: C,
        configuration: ConfigurationV1,
    ) -> Result<Self, ErrorV1<C::TransportError>>
    where
        C: WorkerClientV1,
    {
        if configuration.agent.id.is_empty()
            || configuration.grant_validity.is_zero()
            || configuration.lease_duration.is_zero()
        {
            return Err(ErrorV1::InvalidConfiguration);
        }
        Ok(Self {
            client,
            configuration,
            state: State::Idle,
        })
    }

    pub fn is_fenced(&self) -> bool {
        matches!(self.state, State::Fenced)
    }

    pub fn active(&self) -> Option<ClaimedAttemptV1> {
        let State::Active(active) = &self.state else {
            return None;
        };
        Some(ClaimedAttemptV1 {
            receipt: AttemptReceiptV1 {
                task: active.task,
                attempt_number: active.attempt_number,
                chronos_revision: active.chronos_revision,
            },
        })
    }

    pub fn into_client(self) -> C {
        self.client
    }
}

impl<C: WorkerClientV1> TrustedWorkerV1<C> {
    fn call(
        &mut self,
        request: WorkerRequestV1,
    ) -> Result<CommandOutcomeV2, ErrorV1<C::TransportError>> {
        match self.client.call_worker(request) {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(refusal @ (WorkerRefusalV1::Unauthorized | WorkerRefusalV1::Limit))) => {
                Err(ErrorV1::OwnerRefused(refusal))
            }
            Ok(Err(WorkerRefusalV1::ResultWithheld)) => {
                self.state = State::Fenced;
                Err(ErrorV1::ResultWithheld)
            }
            Err(error) => {
                self.state = State::Fenced;
                Err(ErrorV1::OutcomeUnknown(error))
            }
        }
    }

    fn advance_time(&mut self, now: SystemTime) -> Result<EngineTime, ErrorV1<C::TransportError>> {
        match self.call(WorkerRequestV1::AdvanceTime { now })? {
            CommandOutcomeV2::AdvanceTime(Ok(time)) => Ok(time),
            CommandOutcomeV2::AdvanceTime(Err(_)) => Err(ErrorV1::DomainRefused),
            _ => {
                self.state = State::Fenced;
                Err(ErrorV1::UnexpectedOutcome)
            }
        }
    }

    fn revoke(&mut self, grant: CapabilityGrant) -> Result<(), ErrorV1<C::TransportError>> {
        match self.call(WorkerRequestV1::RevokeGrant(grant))? {
            CommandOutcomeV2::RevokeGrant(Ok(())) => Ok(()),
            CommandOutcomeV2::RevokeGrant(Err(_)) => {
                self.state = State::Fenced;
                Err(ErrorV1::DomainRefused)
            }
            _ => {
                self.state = State::Fenced;
                Err(ErrorV1::UnexpectedOutcome)
            }
        }
    }

    pub fn claim(
        &mut self,
        intent: ClaimIntentV1,
    ) -> Result<ClaimedAttemptV1, ErrorV1<C::TransportError>> {
        match self.state {
            State::Idle => {}
            State::Active(_) => return Err(ErrorV1::AlreadyActive),
            State::Fenced => return Err(ErrorV1::Fenced),
        }
        if intent.task_id.is_empty()
            || intent.external_task_key == [0; 16]
            || intent.expected_revision >= u64::MAX - 1
        {
            return Err(ErrorV1::InvalidIntent);
        }
        let time = self.advance_time(intent.now)?;
        let grant = match self.call(WorkerRequestV1::IssueGrant {
            agent: self.configuration.agent.clone(),
            time: time.clone(),
            validity_duration: self.configuration.grant_validity,
            claim_budget: Some(1),
        })? {
            CommandOutcomeV2::IssueGrant(Ok(grant)) => grant,
            CommandOutcomeV2::IssueGrant(Err(_)) => return Err(ErrorV1::DomainRefused),
            _ => {
                self.state = State::Fenced;
                return Err(ErrorV1::UnexpectedOutcome);
            }
        };
        let task_id = TaskId::new(intent.task_id);
        let outcome = self.call(WorkerRequestV1::ClaimTask {
            expected: StateRevision::finite(intent.expected_revision),
            grant: grant.clone(),
            task_id: task_id.clone(),
            time,
            lease_duration: self.configuration.lease_duration,
        });
        let task = match outcome {
            Ok(CommandOutcomeV2::ClaimTask(Ok(task))) => task,
            Ok(CommandOutcomeV2::ClaimTask(Err(_))) => {
                self.revoke(grant)?;
                return Err(ErrorV1::DomainRefused);
            }
            Ok(_) => {
                self.state = State::Fenced;
                return Err(ErrorV1::UnexpectedOutcome);
            }
            Err(error) => return Err(error),
        };
        if task.id != task_id {
            self.state = State::Fenced;
            return Err(ErrorV1::UnexpectedOutcome);
        }
        let attempt = task.attempt_reference().ok_or_else(|| {
            self.state = State::Fenced;
            ErrorV1::UnexpectedOutcome
        })?;
        let TaskState::Running(ref claim) = task.state else {
            self.state = State::Fenced;
            return Err(ErrorV1::UnexpectedOutcome);
        };
        let receipt = AttemptReceiptV1 {
            task: TaskRefV1 {
                external_task_key: intent.external_task_key,
                incarnation: attempt.task().incarnation().get(),
            },
            attempt_number: attempt.attempt_number(),
            chronos_revision: intent.expected_revision + 1,
        };
        self.state = State::Active(ActiveAttempt {
            task_id,
            task: receipt.task,
            attempt_number: receipt.attempt_number,
            chronos_revision: receipt.chronos_revision,
            lease: claim.lease.clone(),
            binding: None,
        });
        Ok(ClaimedAttemptV1 { receipt })
    }

    /// Claim one exact Chronos attempt and immediately retain its journey
    /// binding. If storage refuses or becomes indeterminate the returned error
    /// retains the successful claim and the worker remains active; callers may
    /// reconcile and invoke `bind_active_journey` without claiming again.
    pub fn claim_and_bind<S: TaskAgentJourneyStore>(
        &mut self,
        store: &mut S,
        expected_storage_revision: Option<u64>,
        journey: JourneyIdV1,
        intent: ClaimIntentV1,
    ) -> Result<ReadyTaskAttemptV1, ClaimAndBindErrorV1<C::TransportError, S::Error>> {
        self.claim(intent).map_err(ClaimAndBindErrorV1::Claim)?;
        self.bind_active_journey(store, expected_storage_revision, journey)
            .map_err(ClaimAndBindErrorV1::Bind)
    }

    /// Retain the already-active exact claim without issuing another Chronos
    /// mutation. This is the only safe continuation after a storage conflict
    /// or indeterminate persistence outcome from `claim_and_bind`.
    pub fn bind_active_journey<S: TaskAgentJourneyStore>(
        &mut self,
        store: &mut S,
        expected_storage_revision: Option<u64>,
        journey: JourneyIdV1,
    ) -> Result<ReadyTaskAttemptV1, JourneyBindingErrorV1<S::Error>> {
        let (claim, chronos_task_id) = match &self.state {
            State::Active(active) => (
                ClaimedAttemptV1 {
                    receipt: AttemptReceiptV1 {
                        task: active.task,
                        attempt_number: active.attempt_number,
                        chronos_revision: active.chronos_revision,
                    },
                },
                active.task_id.as_str().to_owned(),
            ),
            State::Idle | State::Fenced => {
                return Err(JourneyBindingErrorV1::NoActiveAttempt);
            }
        };
        let receipt = ChronosAttemptReceiptV1::new(
            ChronosTaskRefV1::new(
                claim.receipt.task.external_task_key,
                claim.receipt.task.incarnation,
            ),
            claim.receipt.attempt_number,
            claim.receipt.chronos_revision,
        );
        let applied = store
            .apply(
                expected_storage_revision,
                TaskAgentJourneyMutationV1::BindTaskAttemptNamespace {
                    journey,
                    receipt,
                    chronos_task_id: chronos_task_id.clone(),
                },
            )
            .map_err(|error| JourneyBindingErrorV1::Store { claim, error })?;
        let TaskAgentJourneyApplyV1::TaskAttempt(bound) = applied.outcome else {
            self.state = State::Fenced;
            return Err(JourneyBindingErrorV1::UnexpectedOutcome { claim });
        };
        let exact = bound.journey.id == journey
            && bound.journey.task_attempt.is_some_and(|binding| {
                binding.task == receipt.task()
                    && binding.attempt_number == receipt.attempt_number()
                    && binding.chronos_revision == receipt.chronos_revision()
            })
            && store
                .task_attempt_namespace(journey)
                .is_some_and(|binding| {
                    binding.task == receipt.task()
                        && binding.attempt_number == receipt.attempt_number()
                        && binding.chronos_revision == receipt.chronos_revision()
                        && binding.chronos_task_id == chronos_task_id
                });
        if !exact {
            self.state = State::Fenced;
            return Err(JourneyBindingErrorV1::UnexpectedOutcome { claim });
        }
        Ok(ReadyTaskAttemptV1 {
            claim,
            storage_revision: applied.storage_revision,
            journey: bound.journey,
            replayed: bound.replayed,
        })
    }

    pub fn bind_result(
        &mut self,
        binding: ResultBindingV1,
    ) -> Result<(), ErrorV1<C::TransportError>> {
        if binding.effect.0 == [0; 16] || binding.execution_id == [0; 16] {
            return Err(ErrorV1::InvalidIntent);
        }
        let state_fenced = matches!(self.state, State::Fenced);
        let State::Active(active) = &mut self.state else {
            return Err(if state_fenced {
                ErrorV1::Fenced
            } else {
                ErrorV1::NoActiveAttempt
            });
        };
        if active.binding.is_some() {
            return Err(ErrorV1::ExecutionAlreadyBound);
        }
        active.binding = Some(binding);
        Ok(())
    }

    pub fn settle(
        &mut self,
        result: ResultReceiptV1,
        now: SystemTime,
    ) -> Result<SettlementV1, ErrorV1<C::TransportError>> {
        let settled = match result.disposition {
            DispositionV1::Completed => SettlementV1::Completed,
            DispositionV1::Uncertain => SettlementV1::FencedUnknown,
            DispositionV1::Failed | DispositionV1::Cancelled | DispositionV1::TimedOut => {
                return Err(ErrorV1::UnsupportedDisposition);
            }
        };
        let state_fenced = matches!(self.state, State::Fenced);
        let (task_id, lease) = {
            let State::Active(active) = &self.state else {
                return Err(if state_fenced {
                    ErrorV1::Fenced
                } else {
                    ErrorV1::NoActiveAttempt
                });
            };
            let binding = active.binding.ok_or(ErrorV1::ExecutionNotBound)?;
            if result.effect != binding.effect
                || result.execution_id != binding.execution_id
                || result.task != active.task
                || result.attempt_number != active.attempt_number
            {
                return Err(ErrorV1::EvidenceMismatch);
            }
            (active.task_id.clone(), active.lease.clone())
        };
        let time = self.advance_time(now)?;
        let request = match settled {
            SettlementV1::Completed => WorkerRequestV1::Complete {
                task_id,
                lease,
                time,
            },
            SettlementV1::FencedUnknown => WorkerRequestV1::FenceUncertain {
                task_id,
                lease,
                time,
            },
        };
        let outcome = self.call(request)?;
        let applied = matches!(
            (&outcome, settled),
            (CommandOutcomeV2::Complete(Ok(())), SettlementV1::Completed)
                | (
                    CommandOutcomeV2::FenceUncertain(Ok(())),
                    SettlementV1::FencedUnknown
                )
        );
        if !applied {
            self.state = State::Fenced;
            return Err(match outcome {
                CommandOutcomeV2::Complete(Err(_)) | CommandOutcomeV2::FenceUncertain(Err(_)) => {
                    ErrorV1::DomainRefused
                }
                _ => ErrorV1::UnexpectedOutcome,
            });
        }
        self.state = State::Idle;
        Ok(settled)
    }
}
