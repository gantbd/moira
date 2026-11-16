//! Admits one minimized Moira task through Chronos' public atomic-invocation
//! and observation clients. Prepared invocation identity is exportable so a
//! process restart can reconcile instead of blindly replaying a mutation.

#![forbid(unsafe_code)]

use chronos_client::{WireClientV1, WireClientV3, WireClientV4};
use chronos_protocol as wire;
use moira_protocol::tool_continuation::AttemptEvidenceV1;
use moira_protocol::{AdmissionReceiptV1, EffectIdV1, TaskRefV1};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptBindingV1 {
    pub external_task_key: [u8; 16],
    pub chronos_task_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptObserverConfigurationErrorV1 {
    InvalidLimits,
    EmptyRegistry,
    RegistryLimit,
    InvalidTaskId,
    DuplicateExternalKey,
    DuplicateTaskId,
}

#[derive(Debug, Eq, PartialEq)]
pub enum AttemptObservationErrorV1<E> {
    NotCurrent,
    Unavailable(E),
    InvalidResponse,
}

/// Read-only exact-attempt observation for a tool-continuation gate.
pub struct CurrentAttemptObserverV1<P> {
    port: P,
    limits: wire::Limits,
    tasks: BTreeMap<[u8; 16], String>,
}

impl<P> CurrentAttemptObserverV1<P> {
    pub fn new(
        port: P,
        limits: wire::Limits,
        max_bindings: usize,
        bindings: Vec<AttemptBindingV1>,
    ) -> Result<Self, AttemptObserverConfigurationErrorV1> {
        if max_bindings == 0 || limits.max_frame_bytes == 0 || limits.max_id_bytes == 0 {
            return Err(AttemptObserverConfigurationErrorV1::InvalidLimits);
        }
        if bindings.is_empty() {
            return Err(AttemptObserverConfigurationErrorV1::EmptyRegistry);
        }
        if bindings.len() > max_bindings {
            return Err(AttemptObserverConfigurationErrorV1::RegistryLimit);
        }
        let mut tasks = BTreeMap::new();
        let mut names = BTreeSet::new();
        for binding in bindings {
            if binding.chronos_task_id.is_empty()
                || binding.chronos_task_id.len() > limits.max_id_bytes
            {
                return Err(AttemptObserverConfigurationErrorV1::InvalidTaskId);
            }
            if !names.insert(binding.chronos_task_id.clone()) {
                return Err(AttemptObserverConfigurationErrorV1::DuplicateTaskId);
            }
            if tasks
                .insert(binding.external_task_key, binding.chronos_task_id)
                .is_some()
            {
                return Err(AttemptObserverConfigurationErrorV1::DuplicateExternalKey);
            }
        }
        Ok(Self {
            port,
            limits,
            tasks,
        })
    }

    pub fn into_port(self) -> P {
        self.port
    }
}

impl<P: WireClientV1> CurrentAttemptObserverV1<P> {
    pub fn observe(
        &mut self,
        task_key: [u8; 16],
        incarnation: u64,
        attempt: u32,
    ) -> Result<AttemptEvidenceV1, AttemptObservationErrorV1<P::Error>> {
        if attempt == 0 {
            return Err(AttemptObservationErrorV1::NotCurrent);
        }
        let Some(task_id) = self.tasks.get(&task_key) else {
            return Err(AttemptObservationErrorV1::NotCurrent);
        };
        let request = wire::Request {
            version: 1,
            command: wire::Command::Snapshot {
                task: task_id.clone(),
            },
        };
        request
            .validate(self.limits)
            .map_err(|_| AttemptObservationErrorV1::InvalidResponse)?;
        let response = self
            .port
            .request(request)
            .map_err(AttemptObservationErrorV1::Unavailable)?;
        response
            .validate(self.limits.max_id_bytes)
            .map_err(|_| AttemptObservationErrorV1::InvalidResponse)?;
        let wire::Outcome::Snapshot {
            value: Some(entry),
            revision: Some(_),
        } = response.outcome
        else {
            return Err(AttemptObservationErrorV1::NotCurrent);
        };
        let observed_incarnation = decimal(&entry.reference.incarnation)
            .ok_or(AttemptObservationErrorV1::InvalidResponse)?;
        if entry.reference.task != *task_id
            || observed_incarnation != incarnation
            || !matches!(entry.snapshot.phase, wire::Phase::Running)
            || entry.snapshot.attempt_number != Some(attempt)
        {
            return Err(AttemptObservationErrorV1::NotCurrent);
        }
        Ok(AttemptEvidenceV1 {
            task_key,
            incarnation,
            attempt,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetrySafetyV1 {
    FenceUnknown,
    ConnectorIdempotent,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AdmissionIntentV1 {
    pub effect: EffectIdV1,
    pub external_task_key: [u8; 16],
    pub task_id: String,
    pub expected_chronos_revision: u64,
    pub priority: i32,
    pub capabilities: Vec<String>,
    pub max_attempts: u32,
    pub retry_safety: RetrySafetyV1,
}

impl std::fmt::Debug for AdmissionIntentV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmissionIntentV1")
            .field("effect", &self.effect)
            .field("external_task_key", &"<opaque>")
            .field("task_id_bytes", &self.task_id.len())
            .field("expected_chronos_revision", &self.expected_chronos_revision)
            .field("priority", &self.priority)
            .field("capability_count", &self.capabilities.len())
            .field("max_attempts", &self.max_attempts)
            .field("retry_safety", &self.retry_safety)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct PreparedAdmissionV1 {
    intent: AdmissionIntentV1,
    serial: u64,
}

impl std::fmt::Debug for PreparedAdmissionV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedAdmissionV1")
            .field("intent", &self.intent)
            .field("serial", &self.serial)
            .field("invocation_key", &"<derived-opaque>")
            .finish()
    }
}

impl PreparedAdmissionV1 {
    pub fn restore(
        intent: AdmissionIntentV1,
        serial: u64,
        limits: wire::Limits,
    ) -> Result<Self, AdapterErrorV1<std::convert::Infallible>> {
        let value = Self { intent, serial };
        value
            .execute_request(limits)
            .map_err(|_| AdapterErrorV1::InvalidInput)?;
        Ok(value)
    }

    pub const fn serial(&self) -> u64 {
        self.serial
    }

    pub const fn intent(&self) -> &AdmissionIntentV1 {
        &self.intent
    }

    fn invocation(&self) -> wire::coordination_v3::Invocation {
        wire::coordination_v3::Invocation {
            serial: self.serial.to_string(),
            key: effect_key(self.intent.effect),
        }
    }

    fn execute_request(&self, limits: wire::Limits) -> Result<wire::coordination_v3::Request, ()> {
        let request = wire::coordination_v3::Request {
            version: 3,
            command: wire::coordination_v3::Command::ExecuteInvocation {
                invocation: self.invocation(),
                command: wire::coordination_v3::Mutation::SubmitGraph {
                    expected: finite(self.intent.expected_chronos_revision),
                    tasks: vec![wire::TaskSpec {
                        id: self.intent.task_id.clone(),
                        priority: self.intent.priority,
                        dependencies: Vec::new(),
                        capabilities: self.intent.capabilities.clone(),
                        max_attempts: self.intent.max_attempts,
                        retry_safety: match self.intent.retry_safety {
                            RetrySafetyV1::FenceUnknown => wire::RetrySafety::FenceUnknown,
                            RetrySafetyV1::ConnectorIdempotent => {
                                wire::RetrySafety::ConnectorIdempotent
                            }
                        },
                    }],
                },
            },
        };
        request.validate(limits).map_err(|_| ())?;
        Ok(request)
    }

    fn lookup_request(&self, limits: wire::Limits) -> Result<wire::coordination_v3::Request, ()> {
        let request = wire::coordination_v3::Request {
            version: 3,
            command: wire::coordination_v3::Command::LookupInvocation {
                invocation: self.invocation(),
            },
        };
        request.validate(limits).map_err(|_| ())?;
        Ok(request)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationRefusalV1 {
    InvalidTaskId,
    DuplicateTask,
    UnknownDependency,
    DependencyCycle,
    TaskLimit,
    DependencyEdgeLimit,
    CapabilityLimit,
    InvalidCapabilityName,
    SummaryLimit,
    InvalidAttemptLimit,
    InvalidInitialProgress,
    InvalidIdempotencyKey,
    TaskIdentityExhausted,
    TaskNotFound,
    StaleReference,
    AlreadyTerminal,
    NotTerminal,
    ReferencedBy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationRefusalV1 {
    Unauthorized,
    Limit,
    InvalidRequest,
    UnsupportedVersion,
    Capacity,
    Conflict,
    Expired,
    OutOfSequence,
    Exhausted,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionReplyV1 {
    Admitted(AdmissionReceiptV1),
    RevisionMismatch { expected: u64, actual: u64 },
    RevisionExhausted,
    MutationRefused(MutationRefusalV1),
    InvocationRefused(InvocationRefusalV1),
    HistoryLost,
}

#[derive(Debug)]
pub enum AdapterErrorV1<E> {
    InvalidLimits,
    InvalidInput,
    Transport(E),
    OutcomeUnknown(E),
    ObservationUnavailable(E),
    InvocationRefused(InvocationRefusalV1),
    InvalidResponse,
    UnexpectedResponse,
}

pub struct AdmissionAdapter<P> {
    port: P,
    limits: wire::Limits,
}

impl<P> AdmissionAdapter<P> {
    /// The host binds both wire profiles to the same Chronos service, including after recovery.
    pub fn new(port: P, limits: wire::Limits) -> Result<Self, AdapterErrorV1<P::Error>>
    where
        P: WireClientV3,
    {
        if limits.max_frame_bytes == 0
            || limits.max_id_bytes < 32
            || limits.max_graph_tasks == 0
            || limits.max_page_entries == 0
        {
            return Err(AdapterErrorV1::InvalidLimits);
        }
        Ok(Self { port, limits })
    }

    pub fn into_port(self) -> P {
        self.port
    }
}

impl<P> AdmissionAdapter<P>
where
    P: WireClientV3 + WireClientV4<Error = <P as WireClientV3>::Error>,
{
    pub fn prepare(
        &mut self,
        intent: AdmissionIntentV1,
    ) -> Result<PreparedAdmissionV1, AdapterErrorV1<<P as WireClientV3>::Error>> {
        let probe = PreparedAdmissionV1 {
            intent: intent.clone(),
            serial: 1,
        };
        probe
            .execute_request(self.limits)
            .map_err(|_| AdapterErrorV1::InvalidInput)?;
        let request = wire::coordination_v3::Request {
            version: 3,
            command: wire::coordination_v3::Command::NextInvocation {},
        };
        let response = self
            .port
            .request_v3(request_for_transport(&request))
            .map_err(AdapterErrorV1::Transport)?;
        response
            .validate_for(&request, self.limits)
            .map_err(|_| AdapterErrorV1::InvalidResponse)?;
        match response.outcome {
            wire::coordination_v3::Outcome::NextInvocation {
                serial: Some(serial),
            } => {
                let serial = decimal(&serial).ok_or(AdapterErrorV1::InvalidResponse)?;
                PreparedAdmissionV1::restore(intent, serial, self.limits)
                    .map_err(|_| AdapterErrorV1::InvalidInput)
            }
            wire::coordination_v3::Outcome::NextInvocation { serial: None } => {
                Err(AdapterErrorV1::UnexpectedResponse)
            }
            wire::coordination_v3::Outcome::Refused { code } => {
                Err(AdapterErrorV1::InvocationRefused(invocation_refusal(code)))
            }
            _ => Err(AdapterErrorV1::UnexpectedResponse),
        }
    }

    /// Executes exactly once. Any transport error after this call is outcome
    /// unknown; callers must retain `prepared` and use `reconcile`, not retry.
    pub fn execute(
        &mut self,
        prepared: &PreparedAdmissionV1,
    ) -> Result<AdmissionReplyV1, AdapterErrorV1<<P as WireClientV3>::Error>> {
        let request = prepared
            .execute_request(self.limits)
            .map_err(|_| AdapterErrorV1::InvalidInput)?;
        let response = self
            .port
            .request_v3(request_for_transport(&request))
            .map_err(AdapterErrorV1::OutcomeUnknown)?;
        self.resolve(prepared, request, response)
    }

    /// Read-only closure for an uncertain invocation, including after process
    /// restart. It never resubmits the mutation.
    pub fn reconcile(
        &mut self,
        prepared: &PreparedAdmissionV1,
    ) -> Result<AdmissionReplyV1, AdapterErrorV1<<P as WireClientV3>::Error>> {
        let request = prepared
            .lookup_request(self.limits)
            .map_err(|_| AdapterErrorV1::InvalidInput)?;
        let response = self
            .port
            .request_v3(request_for_transport(&request))
            .map_err(AdapterErrorV1::Transport)?;
        self.resolve(prepared, request, response)
    }

    fn resolve(
        &mut self,
        prepared: &PreparedAdmissionV1,
        request: wire::coordination_v3::Request,
        response: wire::coordination_v3::Response,
    ) -> Result<AdmissionReplyV1, AdapterErrorV1<<P as WireClientV3>::Error>> {
        response
            .validate_for(&request, self.limits)
            .map_err(|_| AdapterErrorV1::InvalidResponse)?;
        match response.outcome {
            wire::coordination_v3::Outcome::InvocationResult { result, .. } => match result {
                wire::coordination_v3::MutationResult::Applied {} => self.observe(prepared),
                wire::coordination_v3::MutationResult::RevisionMismatch { expected, actual } => {
                    Ok(AdmissionReplyV1::RevisionMismatch {
                        expected: revision(&expected).ok_or(AdapterErrorV1::InvalidResponse)?,
                        actual: revision(&actual).ok_or(AdapterErrorV1::InvalidResponse)?,
                    })
                }
                wire::coordination_v3::MutationResult::RevisionExhausted {} => {
                    Ok(AdmissionReplyV1::RevisionExhausted)
                }
                wire::coordination_v3::MutationResult::Refused { code } => {
                    Ok(AdmissionReplyV1::MutationRefused(mutation_refusal(code)))
                }
            },
            wire::coordination_v3::Outcome::Refused {
                code: wire::coordination_v3::RefusalCode::Expired,
            } => Ok(AdmissionReplyV1::HistoryLost),
            wire::coordination_v3::Outcome::Refused { code } => Ok(
                AdmissionReplyV1::InvocationRefused(invocation_refusal(code)),
            ),
            _ => Err(AdapterErrorV1::UnexpectedResponse),
        }
    }

    /// Observes the exact task reference after an established applied result.
    pub fn observe(
        &mut self,
        prepared: &PreparedAdmissionV1,
    ) -> Result<AdmissionReplyV1, AdapterErrorV1<<P as WireClientV3>::Error>> {
        prepared
            .execute_request(self.limits)
            .map_err(|_| AdapterErrorV1::InvalidInput)?;
        use wire::coordination_v4::{Command, Outcome, RefusalCode, Request};
        let invocation = prepared.invocation();
        let request = Request {
            version: 4,
            command: Command::LookupAllocation {
                serial: invocation.serial,
                key: invocation.key,
            },
        };
        let response = self
            .port
            .request_v4(request.clone())
            .map_err(AdapterErrorV1::ObservationUnavailable)?;
        response
            .validate_for(&request, self.limits)
            .map_err(|_| AdapterErrorV1::InvalidResponse)?;
        let (tasks, revision_value) = match response.outcome {
            Outcome::Allocation {
                tasks, revision, ..
            } => (tasks, revision),
            Outcome::Unavailable { .. }
            | Outcome::Refused {
                code: RefusalCode::Expired,
            } => {
                return Ok(AdmissionReplyV1::HistoryLost);
            }
            Outcome::Refused { code } => {
                return Ok(AdmissionReplyV1::InvocationRefused(invocation_refusal(
                    code,
                )));
            }
        };
        let [task] = tasks.as_slice() else {
            return Err(AdapterErrorV1::InvalidResponse);
        };
        if task.task != prepared.intent.task_id {
            return Err(AdapterErrorV1::InvalidResponse);
        }
        let incarnation = decimal(&task.incarnation).ok_or(AdapterErrorV1::InvalidResponse)?;
        let chronos_revision = revision(&revision_value).ok_or(AdapterErrorV1::InvalidResponse)?;
        if prepared.intent.expected_chronos_revision.checked_add(1) != Some(chronos_revision) {
            return Err(AdapterErrorV1::InvalidResponse);
        }
        Ok(AdmissionReplyV1::Admitted(AdmissionReceiptV1 {
            effect: prepared.intent.effect,
            task: TaskRefV1 {
                external_task_key: prepared.intent.external_task_key,
                incarnation,
            },
            chronos_revision,
        }))
    }
}

fn request_for_transport(
    request: &wire::coordination_v3::Request,
) -> wire::coordination_v3::Request {
    // Protocol request values do not currently implement Clone; reconstruct the
    // three adapter-owned shapes without serializing through an ambient codec.
    match &request.command {
        wire::coordination_v3::Command::NextInvocation {} => wire::coordination_v3::Request {
            version: request.version,
            command: wire::coordination_v3::Command::NextInvocation {},
        },
        wire::coordination_v3::Command::LookupInvocation { invocation } => {
            wire::coordination_v3::Request {
                version: request.version,
                command: wire::coordination_v3::Command::LookupInvocation {
                    invocation: wire::coordination_v3::Invocation {
                        serial: invocation.serial.clone(),
                        key: invocation.key.clone(),
                    },
                },
            }
        }
        wire::coordination_v3::Command::ExecuteInvocation {
            invocation,
            command: wire::coordination_v3::Mutation::SubmitGraph { expected, tasks },
        } => wire::coordination_v3::Request {
            version: request.version,
            command: wire::coordination_v3::Command::ExecuteInvocation {
                invocation: wire::coordination_v3::Invocation {
                    serial: invocation.serial.clone(),
                    key: invocation.key.clone(),
                },
                command: wire::coordination_v3::Mutation::SubmitGraph {
                    expected: clone_revision(expected),
                    tasks: tasks.iter().map(clone_task).collect(),
                },
            },
        },
        _ => unreachable!("adapter constructs only next, lookup and one-task submit"),
    }
}

fn clone_task(value: &wire::TaskSpec) -> wire::TaskSpec {
    wire::TaskSpec {
        id: value.id.clone(),
        priority: value.priority,
        dependencies: value.dependencies.clone(),
        capabilities: value.capabilities.clone(),
        max_attempts: value.max_attempts,
        retry_safety: match value.retry_safety {
            wire::RetrySafety::FenceUnknown => wire::RetrySafety::FenceUnknown,
            wire::RetrySafety::ConnectorIdempotent => wire::RetrySafety::ConnectorIdempotent,
        },
    }
}

fn finite(value: u64) -> wire::Revision {
    wire::Revision::Finite(value.to_string())
}
fn clone_revision(value: &wire::Revision) -> wire::Revision {
    match value {
        wire::Revision::Finite(value) => wire::Revision::Finite(value.clone()),
        wire::Revision::Exhausted => wire::Revision::Exhausted,
    }
}
fn revision(value: &wire::Revision) -> Option<u64> {
    match value {
        wire::Revision::Finite(value) => decimal(value),
        wire::Revision::Exhausted => None,
    }
}
fn decimal(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}
fn effect_key(effect: EffectIdV1) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(32);
    for byte in effect.0 {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    value
}
fn mutation_refusal(value: wire::coordination_v3::MutationCode) -> MutationRefusalV1 {
    use wire::coordination_v3::MutationCode as W;
    match value {
        W::InvalidTaskId => MutationRefusalV1::InvalidTaskId,
        W::DuplicateTask => MutationRefusalV1::DuplicateTask,
        W::UnknownDependency => MutationRefusalV1::UnknownDependency,
        W::DependencyCycle => MutationRefusalV1::DependencyCycle,
        W::TaskLimit => MutationRefusalV1::TaskLimit,
        W::DependencyEdgeLimit => MutationRefusalV1::DependencyEdgeLimit,
        W::CapabilityLimit => MutationRefusalV1::CapabilityLimit,
        W::InvalidCapabilityName => MutationRefusalV1::InvalidCapabilityName,
        W::SummaryLimit => MutationRefusalV1::SummaryLimit,
        W::InvalidAttemptLimit => MutationRefusalV1::InvalidAttemptLimit,
        W::InvalidInitialProgress => MutationRefusalV1::InvalidInitialProgress,
        W::InvalidIdempotencyKey => MutationRefusalV1::InvalidIdempotencyKey,
        W::TaskIdentityExhausted => MutationRefusalV1::TaskIdentityExhausted,
        W::TaskNotFound => MutationRefusalV1::TaskNotFound,
        W::StaleReference => MutationRefusalV1::StaleReference,
        W::AlreadyTerminal => MutationRefusalV1::AlreadyTerminal,
        W::NotTerminal => MutationRefusalV1::NotTerminal,
        W::ReferencedBy => MutationRefusalV1::ReferencedBy,
    }
}
fn invocation_refusal(value: wire::coordination_v3::RefusalCode) -> InvocationRefusalV1 {
    use wire::coordination_v3::RefusalCode as W;
    match value {
        W::Unauthorized => InvocationRefusalV1::Unauthorized,
        W::Limit => InvocationRefusalV1::Limit,
        W::InvalidRequest => InvocationRefusalV1::InvalidRequest,
        W::UnsupportedVersion => InvocationRefusalV1::UnsupportedVersion,
        W::Capacity => InvocationRefusalV1::Capacity,
        W::Conflict => InvocationRefusalV1::Conflict,
        W::Expired => InvocationRefusalV1::Expired,
        W::OutOfSequence => InvocationRefusalV1::OutOfSequence,
        W::Exhausted => InvocationRefusalV1::Exhausted,
        W::Unavailable => InvocationRefusalV1::Unavailable,
    }
}
