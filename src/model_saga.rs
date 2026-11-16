//! Deterministic coordinator state for Daimon-owned model proposals.
//!
//! Each proposal receives a distinct Moira effect identity. Effects advance in
//! order and uncertainty fences the tail; a proposal is never peer completion.

use crate::{EffectId, WorkflowId, allocated_sequence, identity_bytes};
use std::collections::{BTreeMap, BTreeSet};

pub mod persistence;

#[derive(Clone, Eq, PartialEq)]
pub enum ProposalV1 {
    AgoraMessage {
        room: [u8; 16],
        reply_to: [u8; 16],
        body: Vec<u8>,
    },
    VellumEdit {
        workspace: [u8; 16],
        document: [u8; 16],
        expected_revision: u64,
        expected_extent: u64,
        replacement: Vec<u8>,
    },
}

impl std::fmt::Debug for ProposalV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AgoraMessage { body, .. } => formatter
                .debug_struct("AgoraMessage")
                .field("room", &"<opaque>")
                .field("reply_to", &"<opaque>")
                .field("body_bytes", &body.len())
                .finish(),
            Self::VellumEdit {
                expected_revision,
                expected_extent,
                replacement,
                ..
            } => formatter
                .debug_struct("VellumEdit")
                .field("workspace", &"<opaque>")
                .field("document", &"<opaque>")
                .field("expected_revision", expected_revision)
                .field("expected_extent", expected_extent)
                .field("replacement_bytes", &replacement.len())
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerReceiptV1 {
    AgoraMessage {
        effect: EffectId,
        room: [u8; 16],
        reply_to: [u8; 16],
        message: [u8; 16],
        agora_revision: u64,
    },
    VellumEdit {
        effect: EffectId,
        workspace: [u8; 16],
        document: [u8; 16],
        source_revision: u64,
        durable: bool,
    },
}

impl PeerReceiptV1 {
    pub const fn effect(self) -> EffectId {
        match self {
            Self::AgoraMessage { effect, .. } | Self::VellumEdit { effect, .. } => effect,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectStateV1 {
    Queued,
    Pending,
    Succeeded(PeerReceiptV1),
    Failed,
    Uncertain,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalEffectV1 {
    pub effect: EffectId,
    pub proposal: ProposalV1,
    pub state: EffectStateV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseV1 {
    Pending,
    FencedUncertain { effect: EffectId },
    Completed,
    Failed { effect: EffectId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SagaSnapshotV1 {
    pub workflow: WorkflowId,
    pub daimon_effect: EffectId,
    pub selection_digest: [u8; 32],
    pub phase: PhaseV1,
    pub effects: Vec<ProposalEffectV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitsV1 {
    pub max_sagas: usize,
    pub max_proposals_per_saga: usize,
    pub max_proposal_bytes: usize,
}

impl Default for LimitsV1 {
    fn default() -> Self {
        Self {
            max_sagas: 1_024,
            max_proposals_per_saga: 64,
            max_proposal_bytes: 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
    InvalidLimits,
    InvalidSelection,
    Limit,
    Conflict,
    UnknownSaga,
    UnknownEffect,
    NotPending,
    ReceiptMismatch,
    IdentityExhausted,
    InvalidCheckpoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandV1 {
    Admit {
        workflow: WorkflowId,
        daimon_effect: EffectId,
        selection_digest: [u8; 32],
        proposals: Vec<ProposalV1>,
    },
    Accept {
        workflow: WorkflowId,
        receipt: PeerReceiptV1,
    },
    Fail {
        workflow: WorkflowId,
        effect: EffectId,
    },
    MarkUncertain {
        workflow: WorkflowId,
        effect: EffectId,
    },
    Reconcile {
        workflow: WorkflowId,
        receipt: PeerReceiptV1,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionV1 {
    pub saga: SagaSnapshotV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointV1 {
    limits: LimitsV1,
    next_effect: u64,
    sagas: BTreeMap<WorkflowId, SagaSnapshotV1>,
}

pub struct Engine {
    limits: LimitsV1,
    next_effect: u64,
    sagas: BTreeMap<WorkflowId, SagaSnapshotV1>,
}

impl Engine {
    pub fn try_new(limits: LimitsV1) -> Result<Self, RefusalV1> {
        validate_limits(limits)?;
        Ok(Self {
            limits,
            next_effect: 0,
            sagas: BTreeMap::new(),
        })
    }

    pub fn apply(&mut self, command: CommandV1) -> Result<TransitionV1, RefusalV1> {
        let saga = match command {
            CommandV1::Admit {
                workflow,
                daimon_effect,
                selection_digest,
                proposals,
            } => self.admit(workflow, daimon_effect, selection_digest, proposals)?,
            CommandV1::Accept { workflow, receipt } => self.accept(workflow, receipt)?,
            CommandV1::Fail { workflow, effect } => self.fail(workflow, effect)?,
            CommandV1::MarkUncertain { workflow, effect } => {
                self.mark_uncertain(workflow, effect)?
            }
            CommandV1::Reconcile { workflow, receipt } => self.reconcile(workflow, receipt)?,
        };
        Ok(TransitionV1 { saga: saga.clone() })
    }

    pub fn admit(
        &mut self,
        workflow: WorkflowId,
        daimon_effect: EffectId,
        selection_digest: [u8; 32],
        proposals: Vec<ProposalV1>,
    ) -> Result<&SagaSnapshotV1, RefusalV1> {
        if selection_digest == [0; 32] {
            return Err(RefusalV1::InvalidSelection);
        }
        if self.sagas.contains_key(&workflow) {
            let matches = self.sagas.get(&workflow).is_some_and(|existing| {
                existing.daimon_effect == daimon_effect
                    && existing.selection_digest == selection_digest
                    && existing
                        .effects
                        .iter()
                        .map(|effect| &effect.proposal)
                        .eq(proposals.iter())
            });
            if !matches {
                return Err(RefusalV1::Conflict);
            }
            return self.sagas.get(&workflow).ok_or(RefusalV1::UnknownSaga);
        }
        if self.sagas.len() >= self.limits.max_sagas
            || proposals.len() > self.limits.max_proposals_per_saga
            || proposals.iter().any(|proposal| !valid_proposal(proposal))
            || proposal_bytes(&proposals)? > self.limits.max_proposal_bytes
        {
            return Err(RefusalV1::Limit);
        }
        let mut effects = Vec::new();
        effects
            .try_reserve(proposals.len())
            .map_err(|_| RefusalV1::Limit)?;
        let proposal_count = u64::try_from(proposals.len()).map_err(|_| RefusalV1::Limit)?;
        let next_effect = self
            .next_effect
            .checked_add(proposal_count)
            .ok_or(RefusalV1::IdentityExhausted)?;
        for (index, proposal) in proposals.into_iter().enumerate() {
            let sequence = self
                .next_effect
                .checked_add(u64::try_from(index).map_err(|_| RefusalV1::Limit)?)
                .ok_or(RefusalV1::IdentityExhausted)?;
            let effect = EffectId::new(identity_bytes(b'p', sequence));
            effects.push(ProposalEffectV1 {
                effect,
                proposal,
                state: if index == 0 {
                    EffectStateV1::Pending
                } else {
                    EffectStateV1::Queued
                },
            });
        }
        let phase = if effects.is_empty() {
            PhaseV1::Completed
        } else {
            PhaseV1::Pending
        };
        self.next_effect = next_effect;
        self.sagas.insert(
            workflow,
            SagaSnapshotV1 {
                workflow,
                daimon_effect,
                selection_digest,
                phase,
                effects,
            },
        );
        self.sagas.get(&workflow).ok_or(RefusalV1::UnknownSaga)
    }

    pub fn saga(&self, workflow: WorkflowId) -> Option<&SagaSnapshotV1> {
        self.sagas.get(&workflow)
    }

    pub fn pending(&self, workflow: WorkflowId) -> Option<&ProposalEffectV1> {
        let saga = self.sagas.get(&workflow)?;
        (saga.phase == PhaseV1::Pending)
            .then(|| {
                saga.effects
                    .iter()
                    .find(|effect| effect.state == EffectStateV1::Pending)
            })
            .flatten()
    }

    pub fn accept(
        &mut self,
        workflow: WorkflowId,
        receipt: PeerReceiptV1,
    ) -> Result<&SagaSnapshotV1, RefusalV1> {
        let saga = self
            .sagas
            .get_mut(&workflow)
            .ok_or(RefusalV1::UnknownSaga)?;
        let index = saga
            .effects
            .iter()
            .position(|effect| effect.effect == receipt.effect())
            .ok_or(RefusalV1::UnknownEffect)?;
        if let EffectStateV1::Succeeded(existing) = saga.effects[index].state {
            return if existing == receipt {
                Ok(saga)
            } else {
                Err(RefusalV1::ReceiptMismatch)
            };
        }
        if saga.phase != PhaseV1::Pending || saga.effects[index].state != EffectStateV1::Pending {
            return Err(RefusalV1::NotPending);
        }
        if !receipt_matches(&saga.effects[index].proposal, receipt) {
            return Err(RefusalV1::ReceiptMismatch);
        }
        saga.effects[index].state = EffectStateV1::Succeeded(receipt);
        if let Some(next) = saga.effects.get_mut(index + 1) {
            if next.state != EffectStateV1::Queued {
                return Err(RefusalV1::InvalidCheckpoint);
            }
            next.state = EffectStateV1::Pending;
        } else {
            saga.phase = PhaseV1::Completed;
        }
        Ok(saga)
    }

    pub fn fail(
        &mut self,
        workflow: WorkflowId,
        effect: EffectId,
    ) -> Result<&SagaSnapshotV1, RefusalV1> {
        self.finish_pending(workflow, effect, false)
    }

    pub fn mark_uncertain(
        &mut self,
        workflow: WorkflowId,
        effect: EffectId,
    ) -> Result<&SagaSnapshotV1, RefusalV1> {
        self.finish_pending(workflow, effect, true)
    }

    /// Accepts historical peer evidence for the one uncertainty-fenced effect.
    /// This transition never implies that the peer action was dispatched again.
    pub fn reconcile(
        &mut self,
        workflow: WorkflowId,
        receipt: PeerReceiptV1,
    ) -> Result<&SagaSnapshotV1, RefusalV1> {
        let saga = self
            .sagas
            .get_mut(&workflow)
            .ok_or(RefusalV1::UnknownSaga)?;
        let effect = receipt.effect();
        let index = saga
            .effects
            .iter()
            .position(|candidate| candidate.effect == effect)
            .ok_or(RefusalV1::UnknownEffect)?;
        if let EffectStateV1::Succeeded(existing) = saga.effects[index].state {
            return if existing == receipt {
                Ok(saga)
            } else {
                Err(RefusalV1::ReceiptMismatch)
            };
        }
        if saga.phase != (PhaseV1::FencedUncertain { effect })
            || saga.effects[index].state != EffectStateV1::Uncertain
        {
            return Err(RefusalV1::NotPending);
        }
        if !receipt_matches(&saga.effects[index].proposal, receipt) {
            return Err(RefusalV1::ReceiptMismatch);
        }
        saga.effects[index].state = EffectStateV1::Succeeded(receipt);
        if let Some(next) = saga.effects.get_mut(index + 1) {
            if next.state != EffectStateV1::Queued {
                return Err(RefusalV1::InvalidCheckpoint);
            }
            next.state = EffectStateV1::Pending;
            saga.phase = PhaseV1::Pending;
        } else {
            saga.phase = PhaseV1::Completed;
        }
        Ok(saga)
    }

    pub fn checkpoint(&self) -> CheckpointV1 {
        CheckpointV1 {
            limits: self.limits,
            next_effect: self.next_effect,
            sagas: self.sagas.clone(),
        }
    }

    pub fn recover(checkpoint: CheckpointV1) -> Result<Self, RefusalV1> {
        validate_limits(checkpoint.limits)?;
        if checkpoint.sagas.len() > checkpoint.limits.max_sagas {
            return Err(RefusalV1::InvalidCheckpoint);
        }
        let mut allocated = BTreeSet::new();
        for (workflow, saga) in &checkpoint.sagas {
            if saga.workflow != *workflow
                || saga.selection_digest == [0; 32]
                || saga.effects.len() > checkpoint.limits.max_proposals_per_saga
                || proposal_bytes(
                    &saga
                        .effects
                        .iter()
                        .map(|effect| effect.proposal.clone())
                        .collect::<Vec<_>>(),
                )? > checkpoint.limits.max_proposal_bytes
                || saga
                    .effects
                    .iter()
                    .any(|effect| !valid_proposal(&effect.proposal))
                || !valid_state(saga)
            {
                return Err(RefusalV1::InvalidCheckpoint);
            }
            for effect in &saga.effects {
                let Some(sequence) = allocated_sequence(effect.effect.into_bytes(), b'p') else {
                    return Err(RefusalV1::InvalidCheckpoint);
                };
                if sequence >= checkpoint.next_effect || !allocated.insert(effect.effect) {
                    return Err(RefusalV1::InvalidCheckpoint);
                }
            }
        }
        if u64::try_from(allocated.len()).map_err(|_| RefusalV1::InvalidCheckpoint)?
            != checkpoint.next_effect
        {
            return Err(RefusalV1::InvalidCheckpoint);
        }
        Ok(Self {
            limits: checkpoint.limits,
            next_effect: checkpoint.next_effect,
            sagas: checkpoint.sagas,
        })
    }

    fn finish_pending(
        &mut self,
        workflow: WorkflowId,
        effect: EffectId,
        uncertain: bool,
    ) -> Result<&SagaSnapshotV1, RefusalV1> {
        let saga = self
            .sagas
            .get_mut(&workflow)
            .ok_or(RefusalV1::UnknownSaga)?;
        if saga.phase
            == if uncertain {
                PhaseV1::FencedUncertain { effect }
            } else {
                PhaseV1::Failed { effect }
            }
        {
            return Ok(saga);
        }
        let pending = saga
            .effects
            .iter_mut()
            .find(|candidate| candidate.effect == effect)
            .ok_or(RefusalV1::UnknownEffect)?;
        if saga.phase != PhaseV1::Pending || pending.state != EffectStateV1::Pending {
            return Err(RefusalV1::NotPending);
        }
        pending.state = if uncertain {
            EffectStateV1::Uncertain
        } else {
            EffectStateV1::Failed
        };
        saga.phase = if uncertain {
            PhaseV1::FencedUncertain { effect }
        } else {
            PhaseV1::Failed { effect }
        };
        Ok(saga)
    }
}

fn validate_limits(limits: LimitsV1) -> Result<(), RefusalV1> {
    if limits.max_sagas == 0 || limits.max_proposals_per_saga == 0 || limits.max_proposal_bytes == 0
    {
        Err(RefusalV1::InvalidLimits)
    } else {
        Ok(())
    }
}

fn valid_proposal(proposal: &ProposalV1) -> bool {
    match proposal {
        ProposalV1::AgoraMessage { body, .. } => !body.is_empty(),
        ProposalV1::VellumEdit {
            expected_revision,
            replacement,
            ..
        } => *expected_revision != 0 && !replacement.is_empty(),
    }
}

fn proposal_bytes(proposals: &[ProposalV1]) -> Result<usize, RefusalV1> {
    proposals.iter().try_fold(0usize, |total, proposal| {
        total
            .checked_add(match proposal {
                ProposalV1::AgoraMessage { body, .. } => body.len(),
                ProposalV1::VellumEdit { replacement, .. } => replacement.len(),
            })
            .ok_or(RefusalV1::Limit)
    })
}

fn receipt_matches(proposal: &ProposalV1, receipt: PeerReceiptV1) -> bool {
    match (proposal, receipt) {
        (
            ProposalV1::AgoraMessage { room, reply_to, .. },
            PeerReceiptV1::AgoraMessage {
                room: observed_room,
                reply_to: observed_reply,
                agora_revision,
                ..
            },
        ) => *room == observed_room && *reply_to == observed_reply && agora_revision != 0,
        (
            ProposalV1::VellumEdit {
                workspace,
                document,
                expected_revision,
                ..
            },
            PeerReceiptV1::VellumEdit {
                workspace: observed_workspace,
                document: observed_document,
                source_revision,
                durable,
                ..
            },
        ) => {
            *workspace == observed_workspace
                && *document == observed_document
                && source_revision > *expected_revision
                && durable
        }
        _ => false,
    }
}

fn valid_state(saga: &SagaSnapshotV1) -> bool {
    if saga.effects.iter().any(|effect| {
        matches!(effect.state, EffectStateV1::Succeeded(receipt)
            if receipt.effect() != effect.effect || !receipt_matches(&effect.proposal, receipt))
    }) {
        return false;
    }
    match saga.phase {
        PhaseV1::Pending => {
            let Some(index) = saga
                .effects
                .iter()
                .position(|effect| effect.state == EffectStateV1::Pending)
            else {
                return false;
            };
            saga.effects[..index]
                .iter()
                .all(|effect| matches!(effect.state, EffectStateV1::Succeeded(_)))
                && saga.effects[index + 1..]
                    .iter()
                    .all(|effect| effect.state == EffectStateV1::Queued)
        }
        PhaseV1::Completed => {
            saga.effects.is_empty()
                || saga
                    .effects
                    .iter()
                    .all(|effect| matches!(effect.state, EffectStateV1::Succeeded(_)))
        }
        PhaseV1::FencedUncertain { effect } => {
            valid_terminal_tail(&saga.effects, effect, EffectStateV1::Uncertain)
        }
        PhaseV1::Failed { effect } => {
            valid_terminal_tail(&saga.effects, effect, EffectStateV1::Failed)
        }
    }
}

fn valid_terminal_tail(
    effects: &[ProposalEffectV1],
    target: EffectId,
    terminal: EffectStateV1,
) -> bool {
    let Some(index) = effects
        .iter()
        .position(|effect| effect.effect == target && effect.state == terminal)
    else {
        return false;
    };
    effects[..index]
        .iter()
        .all(|effect| matches!(effect.state, EffectStateV1::Succeeded(_)))
        && effects[index + 1..]
            .iter()
            .all(|effect| effect.state == EffectStateV1::Queued)
}
