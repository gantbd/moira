//! Deterministic state machine for native structured tool continuations.
//!
//! A model may propose a call, but it cannot authorize or execute it. Every
//! call receives a distinct Moira effect and must retain current Aegis and
//! exact Chronos-attempt evidence before a dispatch identity can be admitted.

use crate::{EffectId, WorkflowId, allocated_sequence, identity_bytes};
use std::collections::{BTreeMap, BTreeSet};

pub mod persistence;

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub [u8; 16]);
    };
}

id!(ToolRequestIdV1);
id!(ToolCapabilityIdV1);
id!(ToolExecutionIdV1);
id!(AuthorityDecisionIdV1);
id!(AuthoritySubjectIdV1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolBudgetV1 {
    pub max_elapsed_millis: u64,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCallV1 {
    pub request: ToolRequestIdV1,
    pub capability: ToolCapabilityIdV1,
    pub provider_correlation: Vec<u8>,
    pub arguments: Vec<u8>,
    pub budget: ToolBudgetV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorityEvidenceV1 {
    pub decision: AuthorityDecisionIdV1,
    pub subject: AuthoritySubjectIdV1,
    pub capability: ToolCapabilityIdV1,
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptEvidenceV1 {
    pub task_key: [u8; 16],
    pub incarnation: u64,
    pub attempt: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolGateV1 {
    pub authority: AuthorityEvidenceV1,
    pub attempt: AttemptEvidenceV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolReceiptV1 {
    pub effect: EffectId,
    pub capability: ToolCapabilityIdV1,
    pub execution: ToolExecutionIdV1,
    pub evidence_digest: [u8; 32],
    pub output_digest: [u8; 32],
    pub output_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolFailureV1 {
    pub effect: EffectId,
    pub capability: ToolCapabilityIdV1,
    pub execution: ToolExecutionIdV1,
    pub evidence_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolEffectStateV1 {
    Queued,
    PendingAuthorization,
    Ready(ToolGateV1),
    InFlight {
        gate: ToolGateV1,
        execution: ToolExecutionIdV1,
    },
    Succeeded {
        gate: ToolGateV1,
        receipt: ToolReceiptV1,
    },
    Failed {
        gate: ToolGateV1,
        failure: ToolFailureV1,
    },
    Uncertain {
        gate: ToolGateV1,
        execution: ToolExecutionIdV1,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolEffectV1 {
    pub effect: EffectId,
    pub call: ToolCallV1,
    pub state: ToolEffectStateV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseV1 {
    Pending,
    FencedUncertain { effect: EffectId },
    Completed,
    Failed { effect: EffectId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotV1 {
    pub workflow: WorkflowId,
    pub source_execution: EffectId,
    pub source_attempt: AttemptEvidenceV1,
    pub phase: PhaseV1,
    pub effects: Vec<ToolEffectV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointV1 {
    pub version: u8,
    pub limits: LimitsV1,
    pub next_effect: u64,
    pub sagas: Vec<SnapshotV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandV1 {
    Admit {
        workflow: WorkflowId,
        source_execution: EffectId,
        source_attempt: AttemptEvidenceV1,
        calls: Vec<ToolCallV1>,
    },
    Authorize {
        workflow: WorkflowId,
        effect: EffectId,
        gate: ToolGateV1,
    },
    Dispatch {
        workflow: WorkflowId,
        effect: EffectId,
        execution: ToolExecutionIdV1,
    },
    Accept {
        workflow: WorkflowId,
        receipt: ToolReceiptV1,
    },
    Fail {
        workflow: WorkflowId,
        failure: ToolFailureV1,
    },
    MarkUncertain {
        workflow: WorkflowId,
        effect: EffectId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionV1 {
    pub saga: SnapshotV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitsV1 {
    pub max_sagas: usize,
    pub max_calls_per_saga: usize,
    pub max_argument_bytes: usize,
    pub max_elapsed_millis: u64,
    pub max_output_bytes: usize,
}

impl Default for LimitsV1 {
    fn default() -> Self {
        Self {
            max_sagas: 1_024,
            max_calls_per_saga: 32,
            max_argument_bytes: 1_048_576,
            max_elapsed_millis: 3_600_000,
            max_output_bytes: 1_048_576,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
    InvalidLimits,
    Limit,
    Conflict,
    DuplicateRequest,
    UnknownSaga,
    UnknownEffect,
    NotPendingAuthorization,
    NotReady,
    NotInFlight,
    InvalidEvidence,
    ReceiptMismatch,
    IdentityExhausted,
    InvalidCheckpoint,
}

pub struct Engine {
    limits: LimitsV1,
    next_effect: u64,
    sagas: BTreeMap<WorkflowId, SnapshotV1>,
}

impl Engine {
    pub fn try_new(limits: LimitsV1) -> Result<Self, RefusalV1> {
        if limits.max_sagas == 0
            || limits.max_calls_per_saga == 0
            || limits.max_argument_bytes == 0
            || limits.max_elapsed_millis == 0
            || limits.max_output_bytes == 0
        {
            return Err(RefusalV1::InvalidLimits);
        }
        Ok(Self {
            limits,
            next_effect: 0,
            sagas: BTreeMap::new(),
        })
    }

    pub fn saga(&self, workflow: WorkflowId) -> Option<&SnapshotV1> {
        self.sagas.get(&workflow)
    }

    pub fn checkpoint(&self) -> CheckpointV1 {
        CheckpointV1 {
            version: 3,
            limits: self.limits,
            next_effect: self.next_effect,
            sagas: self.sagas.values().cloned().collect(),
        }
    }

    pub fn apply(&mut self, command: CommandV1) -> Result<TransitionV1, RefusalV1> {
        let saga = match command {
            CommandV1::Admit {
                workflow,
                source_execution,
                source_attempt,
                calls,
            } => self.admit(workflow, source_execution, source_attempt, calls)?,
            CommandV1::Authorize {
                workflow,
                effect,
                gate,
            } => self.authorize(workflow, effect, gate)?,
            CommandV1::Dispatch {
                workflow,
                effect,
                execution,
            } => self.dispatch(workflow, effect, execution)?,
            CommandV1::Accept { workflow, receipt } => self.accept(workflow, receipt)?,
            CommandV1::Fail { workflow, failure } => self.fail(workflow, failure)?,
            CommandV1::MarkUncertain { workflow, effect } => {
                self.mark_uncertain(workflow, effect)?
            }
        };
        Ok(TransitionV1 { saga: saga.clone() })
    }

    pub fn recover(checkpoint: CheckpointV1) -> Result<Self, RefusalV1> {
        if checkpoint.version != 3 {
            return Err(RefusalV1::InvalidCheckpoint);
        }
        let mut recovered = Self::try_new(checkpoint.limits)?;
        if checkpoint.sagas.len() > checkpoint.limits.max_sagas {
            return Err(RefusalV1::InvalidCheckpoint);
        }
        let mut prior_workflow = None;
        let mut sequences = BTreeSet::new();
        for saga in checkpoint.sagas {
            if prior_workflow.is_some_and(|prior| prior >= saga.workflow)
                || saga.source_execution.into_bytes() == [0; 16]
                || !valid_attempt(saga.source_attempt)
                || !valid_snapshot(&saga, checkpoint.limits)
            {
                return Err(RefusalV1::InvalidCheckpoint);
            }
            prior_workflow = Some(saga.workflow);
            for effect in &saga.effects {
                let sequence = allocated_sequence(effect.effect.into_bytes(), b't')
                    .ok_or(RefusalV1::InvalidCheckpoint)?;
                if !sequences.insert(sequence) {
                    return Err(RefusalV1::InvalidCheckpoint);
                }
            }
            recovered.sagas.insert(saga.workflow, saga);
        }
        let expected_count = u64::try_from(sequences.len()).map_err(|_| RefusalV1::Limit)?;
        if checkpoint.next_effect != expected_count
            || sequences.iter().copied().ne(0..checkpoint.next_effect)
        {
            return Err(RefusalV1::InvalidCheckpoint);
        }
        recovered.next_effect = checkpoint.next_effect;
        Ok(recovered)
    }

    pub fn admit(
        &mut self,
        workflow: WorkflowId,
        source_execution: EffectId,
        source_attempt: AttemptEvidenceV1,
        calls: Vec<ToolCallV1>,
    ) -> Result<&SnapshotV1, RefusalV1> {
        if self.sagas.contains_key(&workflow) {
            let matches = self.sagas.get(&workflow).is_some_and(|existing| {
                existing.source_execution == source_execution
                    && existing.source_attempt == source_attempt
                    && existing
                        .effects
                        .iter()
                        .map(|effect| &effect.call)
                        .eq(calls.iter())
            });
            return if matches {
                self.sagas.get(&workflow).ok_or(RefusalV1::UnknownSaga)
            } else {
                Err(RefusalV1::Conflict)
            };
        }
        if self.sagas.len() >= self.limits.max_sagas
            || source_execution.into_bytes() == [0; 16]
            || !valid_attempt(source_attempt)
            || calls.is_empty()
            || calls.len() > self.limits.max_calls_per_saga
        {
            return Err(RefusalV1::Limit);
        }
        let mut requests = BTreeSet::new();
        let mut argument_bytes = 0_usize;
        for call in &calls {
            if !requests.insert(call.request) {
                return Err(RefusalV1::DuplicateRequest);
            }
            argument_bytes = argument_bytes
                .checked_add(call.provider_correlation.len())
                .ok_or(RefusalV1::Limit)?
                .checked_add(call.arguments.len())
                .ok_or(RefusalV1::Limit)?;
            if call.request.0 == [0; 16]
                || call.capability.0 == [0; 16]
                || call.provider_correlation.is_empty()
                || call.provider_correlation.len() > 1_024
                || call.arguments.is_empty()
                || call.budget.max_elapsed_millis == 0
                || call.budget.max_elapsed_millis > self.limits.max_elapsed_millis
                || call.budget.max_output_bytes == 0
                || call.budget.max_output_bytes > self.limits.max_output_bytes
            {
                return Err(RefusalV1::Limit);
            }
        }
        if argument_bytes > self.limits.max_argument_bytes {
            return Err(RefusalV1::Limit);
        }
        let count = u64::try_from(calls.len()).map_err(|_| RefusalV1::Limit)?;
        let next_effect = self
            .next_effect
            .checked_add(count)
            .ok_or(RefusalV1::IdentityExhausted)?;
        let effects = calls
            .into_iter()
            .enumerate()
            .map(|(index, call)| {
                let sequence = self
                    .next_effect
                    .checked_add(u64::try_from(index).map_err(|_| RefusalV1::Limit)?)
                    .ok_or(RefusalV1::IdentityExhausted)?;
                Ok(ToolEffectV1 {
                    effect: EffectId::new(identity_bytes(b't', sequence)),
                    call,
                    state: if index == 0 {
                        ToolEffectStateV1::PendingAuthorization
                    } else {
                        ToolEffectStateV1::Queued
                    },
                })
            })
            .collect::<Result<Vec<_>, RefusalV1>>()?;
        self.next_effect = next_effect;
        self.sagas.insert(
            workflow,
            SnapshotV1 {
                workflow,
                source_execution,
                source_attempt,
                phase: PhaseV1::Pending,
                effects,
            },
        );
        self.sagas.get(&workflow).ok_or(RefusalV1::UnknownSaga)
    }

    pub fn authorize(
        &mut self,
        workflow: WorkflowId,
        effect: EffectId,
        gate: ToolGateV1,
    ) -> Result<&SnapshotV1, RefusalV1> {
        if !valid_gate(gate) {
            return Err(RefusalV1::InvalidEvidence);
        }
        let saga = self
            .sagas
            .get_mut(&workflow)
            .ok_or(RefusalV1::UnknownSaga)?;
        if gate.attempt != saga.source_attempt {
            return Err(RefusalV1::InvalidEvidence);
        }
        let target = saga
            .effects
            .iter_mut()
            .find(|candidate| candidate.effect == effect)
            .ok_or(RefusalV1::UnknownEffect)?;
        if gate.authority.capability != target.call.capability {
            return Err(RefusalV1::InvalidEvidence);
        }
        match target.state {
            ToolEffectStateV1::PendingAuthorization => {
                target.state = ToolEffectStateV1::Ready(gate);
            }
            ToolEffectStateV1::Ready(existing) if existing == gate => {}
            ToolEffectStateV1::Ready(_) => return Err(RefusalV1::Conflict),
            _ => return Err(RefusalV1::NotPendingAuthorization),
        }
        Ok(saga)
    }

    pub fn dispatch(
        &mut self,
        workflow: WorkflowId,
        effect: EffectId,
        execution: ToolExecutionIdV1,
    ) -> Result<&SnapshotV1, RefusalV1> {
        if execution.0 == [0; 16] {
            return Err(RefusalV1::InvalidEvidence);
        }
        let saga = self
            .sagas
            .get_mut(&workflow)
            .ok_or(RefusalV1::UnknownSaga)?;
        let target = saga
            .effects
            .iter_mut()
            .find(|candidate| candidate.effect == effect)
            .ok_or(RefusalV1::UnknownEffect)?;
        match target.state {
            ToolEffectStateV1::Ready(gate) => {
                target.state = ToolEffectStateV1::InFlight { gate, execution };
            }
            ToolEffectStateV1::InFlight {
                execution: existing,
                ..
            } if existing == execution => {}
            ToolEffectStateV1::InFlight { .. } => return Err(RefusalV1::Conflict),
            _ => return Err(RefusalV1::NotReady),
        }
        Ok(saga)
    }

    pub fn accept(
        &mut self,
        workflow: WorkflowId,
        receipt: ToolReceiptV1,
    ) -> Result<&SnapshotV1, RefusalV1> {
        let saga = self
            .sagas
            .get_mut(&workflow)
            .ok_or(RefusalV1::UnknownSaga)?;
        let index = saga
            .effects
            .iter()
            .position(|candidate| candidate.effect == receipt.effect)
            .ok_or(RefusalV1::UnknownEffect)?;
        let target = &mut saga.effects[index];
        let (gate, execution) = match target.state {
            ToolEffectStateV1::InFlight { gate, execution } => (gate, execution),
            ToolEffectStateV1::Succeeded {
                receipt: existing, ..
            } if existing == receipt => return Ok(saga),
            ToolEffectStateV1::Succeeded { .. } => return Err(RefusalV1::Conflict),
            _ => return Err(RefusalV1::NotInFlight),
        };
        if receipt.capability != target.call.capability
            || receipt.execution != execution
            || receipt.evidence_digest == [0; 32]
            || receipt.output_digest == [0; 32]
            || receipt.output_bytes > target.call.budget.max_output_bytes
        {
            return Err(RefusalV1::ReceiptMismatch);
        }
        target.state = ToolEffectStateV1::Succeeded { gate, receipt };
        if let Some(next) = saga.effects.get_mut(index + 1) {
            if next.state != ToolEffectStateV1::Queued {
                return Err(RefusalV1::Conflict);
            }
            next.state = ToolEffectStateV1::PendingAuthorization;
        } else {
            saga.phase = PhaseV1::Completed;
        }
        Ok(saga)
    }

    pub fn mark_uncertain(
        &mut self,
        workflow: WorkflowId,
        effect: EffectId,
    ) -> Result<&SnapshotV1, RefusalV1> {
        let saga = self
            .sagas
            .get_mut(&workflow)
            .ok_or(RefusalV1::UnknownSaga)?;
        let target = saga
            .effects
            .iter_mut()
            .find(|candidate| candidate.effect == effect)
            .ok_or(RefusalV1::UnknownEffect)?;
        match target.state {
            ToolEffectStateV1::InFlight { gate, execution } => {
                target.state = ToolEffectStateV1::Uncertain { gate, execution };
                saga.phase = PhaseV1::FencedUncertain { effect };
            }
            ToolEffectStateV1::Uncertain { .. } => {}
            _ => return Err(RefusalV1::NotInFlight),
        }
        Ok(saga)
    }

    pub fn fail(
        &mut self,
        workflow: WorkflowId,
        failure: ToolFailureV1,
    ) -> Result<&SnapshotV1, RefusalV1> {
        let saga = self
            .sagas
            .get_mut(&workflow)
            .ok_or(RefusalV1::UnknownSaga)?;
        let target = saga
            .effects
            .iter_mut()
            .find(|candidate| candidate.effect == failure.effect)
            .ok_or(RefusalV1::UnknownEffect)?;
        let (gate, execution) = match target.state {
            ToolEffectStateV1::InFlight { gate, execution } => (gate, execution),
            ToolEffectStateV1::Failed {
                failure: existing, ..
            } if existing == failure => return Ok(saga),
            ToolEffectStateV1::Failed { .. } => return Err(RefusalV1::Conflict),
            _ => return Err(RefusalV1::NotInFlight),
        };
        if failure.capability != target.call.capability
            || failure.execution != execution
            || failure.evidence_digest == [0; 32]
        {
            return Err(RefusalV1::ReceiptMismatch);
        }
        target.state = ToolEffectStateV1::Failed { gate, failure };
        saga.phase = PhaseV1::Failed {
            effect: failure.effect,
        };
        Ok(saga)
    }
}

fn valid_gate(gate: ToolGateV1) -> bool {
    gate.authority.decision.0 != [0; 16]
        && gate.authority.subject.0 != [0; 16]
        && gate.attempt.task_key != [0; 16]
        && gate.attempt.incarnation != 0
        && gate.attempt.attempt != 0
}

fn valid_attempt(attempt: AttemptEvidenceV1) -> bool {
    attempt.task_key != [0; 16] && attempt.incarnation != 0 && attempt.attempt != 0
}

fn state_gate(state: ToolEffectStateV1) -> Option<ToolGateV1> {
    match state {
        ToolEffectStateV1::Ready(gate)
        | ToolEffectStateV1::InFlight { gate, .. }
        | ToolEffectStateV1::Succeeded { gate, .. }
        | ToolEffectStateV1::Failed { gate, .. }
        | ToolEffectStateV1::Uncertain { gate, .. } => Some(gate),
        ToolEffectStateV1::Queued | ToolEffectStateV1::PendingAuthorization => None,
    }
}

fn valid_snapshot(snapshot: &SnapshotV1, limits: LimitsV1) -> bool {
    if snapshot.effects.is_empty() || snapshot.effects.len() > limits.max_calls_per_saga {
        return false;
    }
    let mut requests = BTreeSet::new();
    let mut argument_bytes = 0_usize;
    for effect in &snapshot.effects {
        let call = &effect.call;
        let Some(total) = argument_bytes
            .checked_add(call.provider_correlation.len())
            .and_then(|total| total.checked_add(call.arguments.len()))
        else {
            return false;
        };
        argument_bytes = total;
        if !requests.insert(call.request)
            || call.request.0 == [0; 16]
            || call.capability.0 == [0; 16]
            || call.provider_correlation.is_empty()
            || call.provider_correlation.len() > 1_024
            || call.arguments.is_empty()
            || call.budget.max_elapsed_millis == 0
            || call.budget.max_elapsed_millis > limits.max_elapsed_millis
            || call.budget.max_output_bytes == 0
            || call.budget.max_output_bytes > limits.max_output_bytes
            || !valid_effect_state(effect)
            || state_gate(effect.state).is_some_and(|gate| gate.attempt != snapshot.source_attempt)
        {
            return false;
        }
    }
    if argument_bytes > limits.max_argument_bytes {
        return false;
    }
    match snapshot.phase {
        PhaseV1::Pending => {
            let Some(active) = snapshot.effects.iter().position(|effect| {
                matches!(
                    effect.state,
                    ToolEffectStateV1::PendingAuthorization
                        | ToolEffectStateV1::Ready(_)
                        | ToolEffectStateV1::InFlight { .. }
                )
            }) else {
                return false;
            };
            snapshot.effects[..active]
                .iter()
                .all(|effect| matches!(effect.state, ToolEffectStateV1::Succeeded { .. }))
                && snapshot.effects[active + 1..]
                    .iter()
                    .all(|effect| effect.state == ToolEffectStateV1::Queued)
                && snapshot.effects[active + 1..].iter().all(|effect| {
                    !matches!(
                        effect.state,
                        ToolEffectStateV1::PendingAuthorization
                            | ToolEffectStateV1::Ready(_)
                            | ToolEffectStateV1::InFlight { .. }
                    )
                })
        }
        PhaseV1::FencedUncertain { effect } => terminal_shape(snapshot, effect, true),
        PhaseV1::Failed { effect } => terminal_shape(snapshot, effect, false),
        PhaseV1::Completed => snapshot
            .effects
            .iter()
            .all(|effect| matches!(effect.state, ToolEffectStateV1::Succeeded { .. })),
    }
}

fn terminal_shape(snapshot: &SnapshotV1, expected: EffectId, uncertain: bool) -> bool {
    let Some(index) = snapshot
        .effects
        .iter()
        .position(|effect| effect.effect == expected)
    else {
        return false;
    };
    let target_matches = if uncertain {
        matches!(
            snapshot.effects[index].state,
            ToolEffectStateV1::Uncertain { .. }
        )
    } else {
        matches!(
            snapshot.effects[index].state,
            ToolEffectStateV1::Failed { .. }
        )
    };
    target_matches
        && snapshot.effects[..index]
            .iter()
            .all(|effect| matches!(effect.state, ToolEffectStateV1::Succeeded { .. }))
        && snapshot.effects[index + 1..]
            .iter()
            .all(|effect| effect.state == ToolEffectStateV1::Queued)
}

fn valid_effect_state(effect: &ToolEffectV1) -> bool {
    match effect.state {
        ToolEffectStateV1::Queued | ToolEffectStateV1::PendingAuthorization => true,
        ToolEffectStateV1::Ready(gate) => {
            valid_gate(gate) && gate.authority.capability == effect.call.capability
        }
        ToolEffectStateV1::InFlight { gate, execution }
        | ToolEffectStateV1::Uncertain { gate, execution } => {
            valid_gate(gate)
                && gate.authority.capability == effect.call.capability
                && execution.0 != [0; 16]
        }
        ToolEffectStateV1::Succeeded { gate, receipt } => {
            valid_gate(gate)
                && gate.authority.capability == effect.call.capability
                && receipt.effect == effect.effect
                && receipt.capability == effect.call.capability
                && receipt.execution.0 != [0; 16]
                && receipt.evidence_digest != [0; 32]
                && receipt.output_digest != [0; 32]
                && receipt.output_bytes <= effect.call.budget.max_output_bytes
        }
        ToolEffectStateV1::Failed { gate, failure } => {
            valid_gate(gate)
                && gate.authority.capability == effect.call.capability
                && failure.effect == effect.effect
                && failure.capability == effect.call.capability
                && failure.execution.0 != [0; 16]
                && failure.evidence_digest != [0; 32]
        }
    }
}
