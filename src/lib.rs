//! Deterministic core for Moira's first cross-engine workflow.
//!
//! This crate performs no I/O. Adapters execute typed effect intents and return
//! attributable receipts. Persistence and production connectors remain open.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
mod recovery_tests;

/// Honest implementation maturity marker exposed to early integrators.
pub const PROJECT_STAGE: &str = "deterministic-reference-slice";

pub use keystone_effects::EffectKnowledge as EffectKnowledgeV1;
pub mod agent_participation;
pub mod deliberation;
pub mod instrumentation;
pub mod model_saga;
pub mod persistence;
pub mod saga;
pub mod tool_continuation;

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            pub const fn new(value: [u8; 16]) -> Self {
                Self(value)
            }

            pub const fn into_bytes(self) -> [u8; 16] {
                self.0
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(<opaque>)"))
            }
        }
    };
}

id!(WorkflowId);
id!(CorrelationId);
id!(CausationId);
id!(EffectId);
id!(ParticipantRefV1);

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AgoraMessageRefV1 {
    source: [u8; 16],
    room_sequence: u64,
    message_sequence: u64,
}

impl AgoraMessageRefV1 {
    pub const fn new(source: [u8; 16], room_sequence: u64, message_sequence: u64) -> Self {
        Self {
            source,
            room_sequence,
            message_sequence,
        }
    }

    pub const fn source(self) -> [u8; 16] {
        self.source
    }

    pub const fn room_sequence(self) -> u64 {
        self.room_sequence
    }

    pub const fn message_sequence(self) -> u64 {
        self.message_sequence
    }
}

impl std::fmt::Debug for AgoraMessageRefV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgoraMessageRefV1(<redacted>)")
    }
}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ChronosTaskRefV1 {
    external_task_key: [u8; 16],
    incarnation: u64,
}

impl ChronosTaskRefV1 {
    pub const fn new(external_task_key: [u8; 16], incarnation: u64) -> Self {
        Self {
            external_task_key,
            incarnation,
        }
    }

    pub const fn external_task_key(self) -> [u8; 16] {
        self.external_task_key
    }

    pub const fn incarnation(self) -> u64 {
        self.incarnation
    }
}

impl std::fmt::Debug for ChronosTaskRefV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ChronosTaskRefV1(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorkflowRevision(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkflowPhaseV1 {
    Received,
    TaskSubmissionPending,
    TaskSubmitted,
    HarnessPending,
    HarnessFinished,
    ResponsePublicationPending,
    FencedUncertain,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectIntentV1 {
    SubmitChronosTask { effect_id: EffectId },
    RequestDaimonExecution { effect_id: EffectId },
    PublishAgoraResponse { effect_id: EffectId },
    PublishVellumEdit { effect_id: EffectId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkflowSnapshotV1 {
    workflow_id: WorkflowId,
    revision: WorkflowRevision,
    cause: AgoraMessageRefV1,
    recipient: ParticipantRefV1,
    phase: WorkflowPhaseV1,
    submission_effect: EffectId,
    pending_effect: Option<EffectId>,
    chronos_task: Option<ChronosTaskRefV1>,
    chronos_revision: Option<u64>,
    chronos_attempt: Option<ChronosAttemptReceiptV1>,
    harness_effect: Option<EffectId>,
    daimon_result: Option<DaimonResultReceiptV1>,
    publication_effect: Option<EffectId>,
    agora_publication: Option<AgoraPublicationReceiptV1>,
}

impl WorkflowSnapshotV1 {
    pub const fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }

    pub const fn revision(&self) -> WorkflowRevision {
        self.revision
    }

    pub const fn cause(&self) -> AgoraMessageRefV1 {
        self.cause
    }

    pub const fn recipient(&self) -> ParticipantRefV1 {
        self.recipient
    }

    pub const fn phase(&self) -> WorkflowPhaseV1 {
        self.phase
    }

    pub const fn chronos_task(&self) -> Option<ChronosTaskRefV1> {
        self.chronos_task
    }

    pub const fn chronos_revision(&self) -> Option<u64> {
        self.chronos_revision
    }

    pub const fn pending_effect(&self) -> Option<EffectId> {
        self.pending_effect
    }

    pub const fn chronos_attempt(&self) -> Option<ChronosAttemptReceiptV1> {
        self.chronos_attempt
    }

    pub const fn daimon_result(&self) -> Option<DaimonResultReceiptV1> {
        self.daimon_result
    }

    pub const fn agora_publication(&self) -> Option<AgoraPublicationReceiptV1> {
        self.agora_publication
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptReceiptConflict;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_workflows: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_workflows: 1_024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitsError {
    WorkflowLimitDisabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChronosAdmissionReceiptV1 {
    effect_id: EffectId,
    task: ChronosTaskRefV1,
    chronos_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChronosAttemptReceiptV1 {
    task: ChronosTaskRefV1,
    attempt_number: u32,
    chronos_revision: u64,
}

impl ChronosAttemptReceiptV1 {
    pub const fn new(task: ChronosTaskRefV1, attempt_number: u32, chronos_revision: u64) -> Self {
        Self {
            task,
            attempt_number,
            chronos_revision,
        }
    }

    pub const fn task(self) -> ChronosTaskRefV1 {
        self.task
    }

    pub const fn attempt_number(self) -> u32 {
        self.attempt_number
    }

    pub const fn chronos_revision(self) -> u64 {
        self.chronos_revision
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DaimonDispositionV1 {
    Completed,
    Failed,
    Cancelled,
    TimedOut,
    Uncertain,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct DaimonResultReceiptV1 {
    effect_id: EffectId,
    execution_id: [u8; 16],
    task: ChronosTaskRefV1,
    attempt_number: u32,
    disposition: DaimonDispositionV1,
}

impl std::fmt::Debug for DaimonResultReceiptV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaimonResultReceiptV1")
            .field("effect_id", &self.effect_id)
            .field("execution_id", &"<opaque>")
            .field("task", &self.task)
            .field("attempt_number", &self.attempt_number)
            .field("disposition", &self.disposition)
            .finish()
    }
}

impl DaimonResultReceiptV1 {
    pub const fn new(
        effect_id: EffectId,
        execution_id: [u8; 16],
        task: ChronosTaskRefV1,
        attempt_number: u32,
        disposition: DaimonDispositionV1,
    ) -> Self {
        Self {
            effect_id,
            execution_id,
            task,
            attempt_number,
            disposition,
        }
    }

    pub const fn effect_id(self) -> EffectId {
        self.effect_id
    }

    pub const fn execution_id(self) -> [u8; 16] {
        self.execution_id
    }

    pub const fn task(self) -> ChronosTaskRefV1 {
        self.task
    }

    pub const fn attempt_number(self) -> u32 {
        self.attempt_number
    }

    pub const fn disposition(self) -> DaimonDispositionV1 {
        self.disposition
    }
}

impl ChronosAdmissionReceiptV1 {
    pub const fn new(effect_id: EffectId, task: ChronosTaskRefV1, chronos_revision: u64) -> Self {
        Self {
            effect_id,
            task,
            chronos_revision,
        }
    }

    pub const fn effect_id(self) -> EffectId {
        self.effect_id
    }

    pub const fn task(self) -> ChronosTaskRefV1 {
        self.task
    }

    pub const fn chronos_revision(self) -> u64 {
        self.chronos_revision
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgoraPublicationReceiptV1 {
    effect_id: EffectId,
    message: AgoraMessageRefV1,
    agora_revision: u64,
}

impl AgoraPublicationReceiptV1 {
    pub const fn new(effect_id: EffectId, message: AgoraMessageRefV1, agora_revision: u64) -> Self {
        Self {
            effect_id,
            message,
            agora_revision,
        }
    }

    pub const fn effect_id(self) -> EffectId {
        self.effect_id
    }

    pub const fn message(self) -> AgoraMessageRefV1 {
        self.message
    }

    pub const fn agora_revision(self) -> u64 {
        self.agora_revision
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandV1 {
    AdmitMessage {
        message: AgoraMessageRefV1,
        recipient: ParticipantRefV1,
    },
    AcceptChronosAdmission {
        workflow: WorkflowId,
        receipt: ChronosAdmissionReceiptV1,
    },
    AcceptChronosAttempt {
        workflow: WorkflowId,
        receipt: ChronosAttemptReceiptV1,
    },
    AcceptDaimonResult {
        workflow: WorkflowId,
        receipt: DaimonResultReceiptV1,
    },
    PrepareAgoraResponse {
        workflow: WorkflowId,
    },
    AcceptAgoraPublication {
        workflow: WorkflowId,
        receipt: AgoraPublicationReceiptV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionV1 {
    ChronosSubmissionPrepared {
        workflow: WorkflowId,
        effect_id: EffectId,
    },
    ChronosSubmissionAccepted {
        workflow: WorkflowId,
        task: ChronosTaskRefV1,
    },
    DaimonExecutionPrepared {
        workflow: WorkflowId,
        effect_id: EffectId,
        task: ChronosTaskRefV1,
        attempt_number: u32,
    },
    DaimonResultAccepted {
        workflow: WorkflowId,
        disposition: DaimonDispositionV1,
    },
    AgoraResponsePrepared {
        workflow: WorkflowId,
        effect_id: EffectId,
    },
    AgoraPublicationAccepted {
        workflow: WorkflowId,
        message: AgoraMessageRefV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
    RevisionConflict {
        expected: WorkflowRevision,
        actual: WorkflowRevision,
    },
    RevisionExhausted,
    WorkflowIdentityExhausted,
    EffectIdentityExhausted,
    WorkflowLimit,
    InputConflict,
    ReceiptConflict,
    UnknownWorkflow,
    InvalidPhase,
    EffectMismatch,
    AttemptMismatch,
}

#[derive(Clone)]
pub struct CheckpointV1 {
    limits: Limits,
    revision: WorkflowRevision,
    next_workflow: u64,
    next_effect: u64,
    workflows: BTreeMap<WorkflowId, WorkflowSnapshotV1>,
    admitted_messages: BTreeMap<AgoraMessageRefV1, WorkflowId>,
}

impl std::fmt::Debug for CheckpointV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CheckpointV1")
            .field("revision", &self.revision)
            .field("workflow_count", &self.workflows.len())
            .field("admitted_message_count", &self.admitted_messages.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryErrorV1 {
    InvalidLimits(LimitsError),
    WorkflowLimit,
    DanglingMessageIndex,
    InvalidSnapshot,
}

/// One serialized, deterministic owner for the first workflow profile.
pub struct Engine {
    limits: Limits,
    revision: WorkflowRevision,
    next_workflow: u64,
    next_effect: u64,
    workflows: BTreeMap<WorkflowId, WorkflowSnapshotV1>,
    admitted_messages: BTreeMap<AgoraMessageRefV1, WorkflowId>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Engine")
            .field("revision", &self.revision)
            .field("workflow_count", &self.workflows.len())
            .finish()
    }
}

impl Engine {
    pub fn try_new(limits: Limits) -> Result<Self, LimitsError> {
        if limits.max_workflows == 0 {
            return Err(LimitsError::WorkflowLimitDisabled);
        }
        Ok(Self {
            limits,
            revision: WorkflowRevision(0),
            next_workflow: 0,
            next_effect: 0,
            workflows: BTreeMap::new(),
            admitted_messages: BTreeMap::new(),
        })
    }

    pub const fn revision(&self) -> WorkflowRevision {
        self.revision
    }

    pub fn workflow(&self, workflow: &WorkflowId) -> Option<&WorkflowSnapshotV1> {
        self.workflows.get(workflow)
    }

    /// Find a unique retained workflow for the host-bound task and attempt.
    /// Duplicate evidence is unresolved even if one workflow appears newer.
    pub fn workflow_for_attempt(
        &self,
        task: ChronosTaskRefV1,
        attempt: u32,
    ) -> Result<Option<WorkflowSnapshotV1>, AttemptReceiptConflict> {
        let mut matches = self.workflows.values().filter(|workflow| {
            workflow.chronos_attempt().is_some_and(|receipt| {
                receipt.task() == task && receipt.attempt_number() == attempt
            })
        });
        let first = matches.next().copied();
        if matches.next().is_some() {
            return Err(AttemptReceiptConflict);
        }
        Ok(first)
    }

    pub fn pending_effect(&self, workflow: &WorkflowId) -> Option<EffectId> {
        self.workflows.get(workflow)?.pending_effect
    }

    /// Captures all semantic state needed to resume exact effect identities.
    /// Encoding and atomic persistence remain adapter responsibilities.
    pub fn checkpoint(&self) -> CheckpointV1 {
        CheckpointV1 {
            limits: self.limits,
            revision: self.revision,
            next_workflow: self.next_workflow,
            next_effect: self.next_effect,
            workflows: self.workflows.clone(),
            admitted_messages: self.admitted_messages.clone(),
        }
    }

    /// Reconstitutes an engine only after validating cross-record invariants.
    pub fn recover(checkpoint: CheckpointV1) -> Result<Self, RecoveryErrorV1> {
        Self::try_new(checkpoint.limits).map_err(RecoveryErrorV1::InvalidLimits)?;
        if checkpoint.workflows.len() > checkpoint.limits.max_workflows {
            return Err(RecoveryErrorV1::WorkflowLimit);
        }
        if checkpoint.admitted_messages.len() != checkpoint.workflows.len()
            || checkpoint.workflows.iter().any(|(workflow, snapshot)| {
                checkpoint.admitted_messages.get(&snapshot.cause) != Some(workflow)
            })
        {
            return Err(RecoveryErrorV1::DanglingMessageIndex);
        }
        for (message, workflow) in &checkpoint.admitted_messages {
            if checkpoint
                .workflows
                .get(workflow)
                .is_none_or(|snapshot| snapshot.cause != *message)
            {
                return Err(RecoveryErrorV1::DanglingMessageIndex);
            }
        }
        if checkpoint.workflows.iter().any(|(workflow, snapshot)| {
            snapshot.workflow_id != *workflow
                || snapshot.revision > checkpoint.revision
                || !valid_snapshot(snapshot)
        }) {
            return Err(RecoveryErrorV1::InvalidSnapshot);
        }
        validate_census(&checkpoint)?;
        Ok(Self {
            limits: checkpoint.limits,
            revision: checkpoint.revision,
            next_workflow: checkpoint.next_workflow,
            next_effect: checkpoint.next_effect,
            workflows: checkpoint.workflows,
            admitted_messages: checkpoint.admitted_messages,
        })
    }

    pub fn apply(
        &mut self,
        expected: WorkflowRevision,
        command: CommandV1,
    ) -> Result<TransitionV1, RefusalV1> {
        self.guard_revision(expected)?;
        match command {
            CommandV1::AdmitMessage { message, recipient } => {
                self.admit_message(message, recipient)
            }
            CommandV1::AcceptChronosAdmission { workflow, receipt } => {
                self.accept_chronos_admission(workflow, receipt)
            }
            CommandV1::AcceptChronosAttempt { workflow, receipt } => {
                self.accept_chronos_attempt(workflow, receipt)
            }
            CommandV1::AcceptDaimonResult { workflow, receipt } => {
                self.accept_daimon_result(workflow, receipt)
            }
            CommandV1::PrepareAgoraResponse { workflow } => self.prepare_agora_response(workflow),
            CommandV1::AcceptAgoraPublication { workflow, receipt } => {
                self.accept_agora_publication(workflow, receipt)
            }
        }
    }

    fn guard_revision(&self, expected: WorkflowRevision) -> Result<(), RefusalV1> {
        if self.revision.0 == u64::MAX {
            return Err(RefusalV1::RevisionExhausted);
        }
        if expected != self.revision {
            return Err(RefusalV1::RevisionConflict {
                expected,
                actual: self.revision,
            });
        }
        Ok(())
    }

    fn admit_message(
        &mut self,
        message: AgoraMessageRefV1,
        recipient: ParticipantRefV1,
    ) -> Result<TransitionV1, RefusalV1> {
        if let Some(workflow) = self.admitted_messages.get(&message) {
            let snapshot = self
                .workflows
                .get(workflow)
                .expect("admitted input must reference an existing workflow");
            if snapshot.recipient != recipient {
                return Err(RefusalV1::InputConflict);
            }
            return Ok(TransitionV1::ChronosSubmissionPrepared {
                workflow: *workflow,
                effect_id: snapshot.submission_effect,
            });
        }
        if self.workflows.len() == self.limits.max_workflows {
            return Err(RefusalV1::WorkflowLimit);
        }
        let workflow = WorkflowId::new(identity_bytes(b'w', self.next_workflow));
        let next_workflow = self
            .next_workflow
            .checked_add(1)
            .ok_or(RefusalV1::WorkflowIdentityExhausted)?;
        let effect_id = EffectId::new(identity_bytes(b'e', self.next_effect));
        let next_effect = self
            .next_effect
            .checked_add(1)
            .ok_or(RefusalV1::EffectIdentityExhausted)?;
        let next_revision = self
            .revision
            .0
            .checked_add(1)
            .ok_or(RefusalV1::RevisionExhausted)?;

        let snapshot = WorkflowSnapshotV1 {
            workflow_id: workflow,
            revision: WorkflowRevision(next_revision),
            cause: message,
            recipient,
            phase: WorkflowPhaseV1::TaskSubmissionPending,
            submission_effect: effect_id,
            pending_effect: Some(effect_id),
            chronos_task: None,
            chronos_revision: None,
            chronos_attempt: None,
            harness_effect: None,
            daimon_result: None,
            publication_effect: None,
            agora_publication: None,
        };
        self.workflows.insert(workflow, snapshot);
        self.admitted_messages.insert(message, workflow);
        self.next_workflow = next_workflow;
        self.next_effect = next_effect;
        self.revision = WorkflowRevision(next_revision);
        Ok(TransitionV1::ChronosSubmissionPrepared {
            workflow,
            effect_id,
        })
    }

    fn accept_chronos_admission(
        &mut self,
        workflow: WorkflowId,
        receipt: ChronosAdmissionReceiptV1,
    ) -> Result<TransitionV1, RefusalV1> {
        let snapshot = self
            .workflows
            .get(&workflow)
            .ok_or(RefusalV1::UnknownWorkflow)?;
        if let Some(task) = snapshot.chronos_task {
            if snapshot.submission_effect == receipt.effect_id
                && task == receipt.task
                && snapshot.chronos_revision == Some(receipt.chronos_revision)
            {
                return Ok(TransitionV1::ChronosSubmissionAccepted {
                    workflow,
                    task: receipt.task,
                });
            }
            return Err(RefusalV1::ReceiptConflict);
        }
        if snapshot.phase != WorkflowPhaseV1::TaskSubmissionPending {
            return Err(RefusalV1::InvalidPhase);
        }
        if snapshot.pending_effect != Some(receipt.effect_id) {
            return Err(RefusalV1::EffectMismatch);
        }
        let next_revision = self
            .revision
            .0
            .checked_add(1)
            .ok_or(RefusalV1::RevisionExhausted)?;

        let snapshot = self
            .workflows
            .get_mut(&workflow)
            .expect("validated workflow must exist");
        snapshot.revision = WorkflowRevision(next_revision);
        snapshot.phase = WorkflowPhaseV1::TaskSubmitted;
        snapshot.pending_effect = None;
        snapshot.chronos_task = Some(receipt.task);
        snapshot.chronos_revision = Some(receipt.chronos_revision);
        self.revision = WorkflowRevision(next_revision);
        Ok(TransitionV1::ChronosSubmissionAccepted {
            workflow,
            task: receipt.task,
        })
    }

    fn accept_chronos_attempt(
        &mut self,
        workflow: WorkflowId,
        receipt: ChronosAttemptReceiptV1,
    ) -> Result<TransitionV1, RefusalV1> {
        let snapshot = self
            .workflows
            .get(&workflow)
            .ok_or(RefusalV1::UnknownWorkflow)?;
        if let Some(stored) = snapshot.chronos_attempt {
            if stored == receipt {
                return Ok(TransitionV1::DaimonExecutionPrepared {
                    workflow,
                    effect_id: snapshot
                        .harness_effect
                        .expect("accepted attempt must retain its effect identity"),
                    task: receipt.task,
                    attempt_number: receipt.attempt_number,
                });
            }
            return Err(RefusalV1::ReceiptConflict);
        }
        if snapshot.phase != WorkflowPhaseV1::TaskSubmitted {
            return Err(RefusalV1::InvalidPhase);
        }
        if snapshot.chronos_task != Some(receipt.task)
            || receipt.attempt_number == 0
            || snapshot
                .chronos_revision
                .is_none_or(|admission_revision| receipt.chronos_revision < admission_revision)
        {
            return Err(RefusalV1::AttemptMismatch);
        }

        let effect_id = EffectId::new(identity_bytes(b'e', self.next_effect));
        let next_effect = self
            .next_effect
            .checked_add(1)
            .ok_or(RefusalV1::EffectIdentityExhausted)?;
        let next_revision = self
            .revision
            .0
            .checked_add(1)
            .ok_or(RefusalV1::RevisionExhausted)?;

        let snapshot = self
            .workflows
            .get_mut(&workflow)
            .expect("validated workflow must exist");
        snapshot.revision = WorkflowRevision(next_revision);
        snapshot.phase = WorkflowPhaseV1::HarnessPending;
        snapshot.pending_effect = Some(effect_id);
        snapshot.chronos_attempt = Some(receipt);
        snapshot.harness_effect = Some(effect_id);
        self.next_effect = next_effect;
        self.revision = WorkflowRevision(next_revision);
        Ok(TransitionV1::DaimonExecutionPrepared {
            workflow,
            effect_id,
            task: receipt.task,
            attempt_number: receipt.attempt_number,
        })
    }

    fn accept_daimon_result(
        &mut self,
        workflow: WorkflowId,
        receipt: DaimonResultReceiptV1,
    ) -> Result<TransitionV1, RefusalV1> {
        let snapshot = self
            .workflows
            .get(&workflow)
            .ok_or(RefusalV1::UnknownWorkflow)?;
        if let Some(stored) = snapshot.daimon_result {
            if stored == receipt {
                return Ok(TransitionV1::DaimonResultAccepted {
                    workflow,
                    disposition: receipt.disposition,
                });
            }
            return Err(RefusalV1::ReceiptConflict);
        }
        if snapshot.phase != WorkflowPhaseV1::HarnessPending {
            return Err(RefusalV1::InvalidPhase);
        }
        if snapshot.pending_effect != Some(receipt.effect_id)
            || snapshot.harness_effect != Some(receipt.effect_id)
        {
            return Err(RefusalV1::EffectMismatch);
        }
        if snapshot.chronos_attempt.is_none_or(|attempt| {
            attempt.task != receipt.task || attempt.attempt_number != receipt.attempt_number
        }) {
            return Err(RefusalV1::AttemptMismatch);
        }
        let next_revision = self
            .revision
            .0
            .checked_add(1)
            .ok_or(RefusalV1::RevisionExhausted)?;

        let snapshot = self
            .workflows
            .get_mut(&workflow)
            .expect("validated workflow must exist");
        snapshot.revision = WorkflowRevision(next_revision);
        snapshot.phase = if receipt.disposition == DaimonDispositionV1::Uncertain {
            WorkflowPhaseV1::FencedUncertain
        } else {
            WorkflowPhaseV1::HarnessFinished
        };
        snapshot.pending_effect = None;
        snapshot.daimon_result = Some(receipt);
        self.revision = WorkflowRevision(next_revision);
        Ok(TransitionV1::DaimonResultAccepted {
            workflow,
            disposition: receipt.disposition,
        })
    }

    fn prepare_agora_response(&mut self, workflow: WorkflowId) -> Result<TransitionV1, RefusalV1> {
        let snapshot = self
            .workflows
            .get(&workflow)
            .ok_or(RefusalV1::UnknownWorkflow)?;
        if let Some(effect_id) = snapshot.publication_effect {
            return Ok(TransitionV1::AgoraResponsePrepared {
                workflow,
                effect_id,
            });
        }
        if snapshot.phase != WorkflowPhaseV1::HarnessFinished {
            return Err(RefusalV1::InvalidPhase);
        }

        let effect_id = EffectId::new(identity_bytes(b'e', self.next_effect));
        let next_effect = self
            .next_effect
            .checked_add(1)
            .ok_or(RefusalV1::EffectIdentityExhausted)?;
        let next_revision = self
            .revision
            .0
            .checked_add(1)
            .ok_or(RefusalV1::RevisionExhausted)?;

        let snapshot = self
            .workflows
            .get_mut(&workflow)
            .expect("validated workflow must exist");
        snapshot.revision = WorkflowRevision(next_revision);
        snapshot.phase = WorkflowPhaseV1::ResponsePublicationPending;
        snapshot.pending_effect = Some(effect_id);
        snapshot.publication_effect = Some(effect_id);
        self.next_effect = next_effect;
        self.revision = WorkflowRevision(next_revision);
        Ok(TransitionV1::AgoraResponsePrepared {
            workflow,
            effect_id,
        })
    }

    fn accept_agora_publication(
        &mut self,
        workflow: WorkflowId,
        receipt: AgoraPublicationReceiptV1,
    ) -> Result<TransitionV1, RefusalV1> {
        let snapshot = self
            .workflows
            .get(&workflow)
            .ok_or(RefusalV1::UnknownWorkflow)?;
        if let Some(stored) = snapshot.agora_publication {
            if stored == receipt {
                return Ok(TransitionV1::AgoraPublicationAccepted {
                    workflow,
                    message: receipt.message,
                });
            }
            return Err(RefusalV1::ReceiptConflict);
        }
        if snapshot.phase != WorkflowPhaseV1::ResponsePublicationPending {
            return Err(RefusalV1::InvalidPhase);
        }
        if snapshot.pending_effect != Some(receipt.effect_id)
            || snapshot.publication_effect != Some(receipt.effect_id)
        {
            return Err(RefusalV1::EffectMismatch);
        }
        let next_revision = self
            .revision
            .0
            .checked_add(1)
            .ok_or(RefusalV1::RevisionExhausted)?;

        let snapshot = self
            .workflows
            .get_mut(&workflow)
            .expect("validated workflow must exist");
        snapshot.revision = WorkflowRevision(next_revision);
        snapshot.phase = WorkflowPhaseV1::Completed;
        snapshot.pending_effect = None;
        snapshot.agora_publication = Some(receipt);
        self.revision = WorkflowRevision(next_revision);
        Ok(TransitionV1::AgoraPublicationAccepted {
            workflow,
            message: receipt.message,
        })
    }
}

const fn identity_bytes(kind: u8, sequence: u64) -> [u8; 16] {
    let bytes = sequence.to_le_bytes();
    [
        b'm', b'o', b'i', b'r', b'a', kind, 0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4],
        bytes[5], bytes[6], bytes[7],
    ]
}

fn allocated_sequence(bytes: [u8; 16], kind: u8) -> Option<u64> {
    let sequence = u64::from_le_bytes(bytes[8..].try_into().expect("fixed-width identity"));
    (bytes == identity_bytes(kind, sequence)).then_some(sequence)
}

/// Current V1 never removes workflows, burns an allocation on refusal, or
/// advances revision on replay. Its census therefore has no holes. These checks
/// establish internal consistency, not authenticated provenance or durability.
fn validate_census(checkpoint: &CheckpointV1) -> Result<(), RecoveryErrorV1> {
    let invalid = RecoveryErrorV1::InvalidSnapshot;
    let count = u64::try_from(checkpoint.workflows.len()).map_err(|_| invalid)?;
    if checkpoint.next_workflow != count {
        return Err(invalid);
    }
    let mut effects = BTreeSet::new();
    let mut revisions = BTreeSet::new();
    let mut transitions = 0u64;
    let mut latest = WorkflowRevision(0);
    for (workflow, snapshot) in &checkpoint.workflows {
        if allocated_sequence(workflow.0, b'w').is_none_or(|n| n >= count) {
            return Err(invalid);
        }
        let rank = match snapshot.phase {
            WorkflowPhaseV1::TaskSubmissionPending => 1,
            WorkflowPhaseV1::TaskSubmitted => 2,
            WorkflowPhaseV1::HarnessPending => 3,
            WorkflowPhaseV1::HarnessFinished | WorkflowPhaseV1::FencedUncertain => 4,
            WorkflowPhaseV1::ResponsePublicationPending => 5,
            WorkflowPhaseV1::Completed => 6,
            _ => return Err(invalid),
        };
        transitions = transitions.checked_add(rank).ok_or(invalid)?;
        if snapshot.revision.0 < rank || !revisions.insert(snapshot.revision) {
            return Err(invalid);
        }
        latest = latest.max(snapshot.revision);
        let mut prior = None;
        for effect in [
            Some(snapshot.submission_effect),
            snapshot.harness_effect,
            snapshot.publication_effect,
        ]
        .into_iter()
        .flatten()
        {
            let sequence = allocated_sequence(effect.0, b'e').ok_or(invalid)?;
            if sequence >= checkpoint.next_effect
                || prior.is_some_and(|n| n >= sequence)
                || !effects.insert(sequence)
            {
                return Err(invalid);
            }
            prior = Some(sequence);
        }
    }
    // Unique in-range allocations plus exact cardinality prove full prefix
    // coverage without iterating an attacker-supplied counter's numeric range.
    if u64::try_from(effects.len()).map_err(|_| invalid)? != checkpoint.next_effect
        || transitions != checkpoint.revision.0
        || latest != checkpoint.revision
    {
        return Err(invalid);
    }
    Ok(())
}

fn valid_snapshot(snapshot: &WorkflowSnapshotV1) -> bool {
    if snapshot.chronos_task.is_some() != snapshot.chronos_revision.is_some() {
        return false;
    }
    if let Some(attempt) = snapshot.chronos_attempt
        && (snapshot.chronos_task != Some(attempt.task)
            || attempt.attempt_number == 0
            || snapshot
                .chronos_revision
                .is_none_or(|r| attempt.chronos_revision < r))
    {
        return false;
    }
    if let Some(result) = snapshot.daimon_result
        && (snapshot.harness_effect != Some(result.effect_id)
            || snapshot
                .chronos_attempt
                .is_none_or(|a| a.task != result.task || a.attempt_number != result.attempt_number))
    {
        return false;
    }
    if let Some(receipt) = snapshot.agora_publication
        && snapshot.publication_effect != Some(receipt.effect_id)
    {
        return false;
    }
    if snapshot.publication_effect.is_some()
        && snapshot
            .daimon_result
            .is_none_or(|r| r.disposition == DaimonDispositionV1::Uncertain)
    {
        return false;
    }
    let admitted = snapshot.chronos_task.is_some() && snapshot.chronos_revision.is_some();
    match snapshot.phase {
        WorkflowPhaseV1::TaskSubmissionPending => {
            snapshot.pending_effect == Some(snapshot.submission_effect)
                && !admitted
                && snapshot.chronos_attempt.is_none()
                && snapshot.harness_effect.is_none()
                && snapshot.daimon_result.is_none()
                && snapshot.publication_effect.is_none()
                && snapshot.agora_publication.is_none()
        }
        WorkflowPhaseV1::TaskSubmitted => {
            snapshot.pending_effect.is_none()
                && admitted
                && snapshot.chronos_attempt.is_none()
                && snapshot.harness_effect.is_none()
                && snapshot.daimon_result.is_none()
                && snapshot.publication_effect.is_none()
                && snapshot.agora_publication.is_none()
        }
        WorkflowPhaseV1::HarnessPending => {
            admitted
                && snapshot.chronos_attempt.is_some()
                && snapshot.harness_effect.is_some()
                && snapshot.pending_effect == snapshot.harness_effect
                && snapshot.daimon_result.is_none()
                && snapshot.publication_effect.is_none()
                && snapshot.agora_publication.is_none()
        }
        WorkflowPhaseV1::HarnessFinished => {
            admitted
                && snapshot.chronos_attempt.is_some()
                && snapshot.harness_effect.is_some()
                && snapshot.pending_effect.is_none()
                && snapshot
                    .daimon_result
                    .is_some_and(|result| result.disposition != DaimonDispositionV1::Uncertain)
                && snapshot.publication_effect.is_none()
                && snapshot.agora_publication.is_none()
        }
        WorkflowPhaseV1::FencedUncertain => {
            admitted
                && snapshot.chronos_attempt.is_some()
                && snapshot.harness_effect.is_some()
                && snapshot.pending_effect.is_none()
                && snapshot
                    .daimon_result
                    .is_some_and(|result| result.disposition == DaimonDispositionV1::Uncertain)
                && snapshot.publication_effect.is_none()
                && snapshot.agora_publication.is_none()
        }
        WorkflowPhaseV1::ResponsePublicationPending => {
            admitted
                && snapshot.chronos_attempt.is_some()
                && snapshot.daimon_result.is_some()
                && snapshot.publication_effect.is_some()
                && snapshot.pending_effect == snapshot.publication_effect
                && snapshot.agora_publication.is_none()
        }
        WorkflowPhaseV1::Completed => {
            admitted
                && snapshot.chronos_attempt.is_some()
                && snapshot.daimon_result.is_some()
                && snapshot.publication_effect.is_some()
                && snapshot.pending_effect.is_none()
                && snapshot.agora_publication.is_some()
        }
        WorkflowPhaseV1::Received | WorkflowPhaseV1::Failed | WorkflowPhaseV1::Cancelled => false,
    }
}
