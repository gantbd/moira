//! Closed V1 saga algebra with explicit time, retry, timer and compensation.
//!
//! This module performs no I/O and reads no clock. Compensation is a new
//! attributable effect; it never rewrites or claims to undo a forward effect.

use std::collections::{BTreeMap, BTreeSet};

pub mod persistence;

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub u64);
    };
}
id!(SagaIdV1);
id!(SagaEffectIdV1);
id!(TimerIdV1);

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct OperatorActionIdV1(pub [u8; 16]);

impl std::fmt::Debug for OperatorActionIdV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OperatorActionIdV1(<opaque>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SagaLimitsV1 {
    pub max_sagas: usize,
    pub max_retries_per_saga: u8,
    pub max_inspection_page: usize,
    pub max_operator_actions: usize,
}
impl Default for SagaLimitsV1 {
    fn default() -> Self {
        Self {
            max_sagas: 1_024,
            max_retries_per_saga: 8,
            max_inspection_page: 256,
            max_operator_actions: 4_096,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectKindV1 {
    Forward,
    Terminal,
    Compensation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectOutcomeV1 {
    Succeeded,
    Failed,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SagaPhaseV1 {
    ForwardPending,
    ForwardSucceeded,
    ForwardFailed,
    TerminalPending,
    TerminalFailed,
    CompensationPending,
    CompensationFailed,
    FencedUncertain,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeadlineActionV1 {
    KeepUncertain,
    Fail,
    Compensate,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct OperatorCauseV1 {
    pub action: OperatorActionIdV1,
    pub subject: [u8; 16],
    pub authority_decision: u64,
    pub authority_epoch: u64,
}

impl std::fmt::Debug for OperatorCauseV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OperatorCauseV1")
            .field("action", &self.action)
            .field("subject", &"<redacted>")
            .field("authority_decision", &self.authority_decision)
            .field("authority_epoch", &self.authority_epoch)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorDispositionV1 {
    AdmitEvidence {
        effect: SagaEffectIdV1,
        outcome: EffectOutcomeV1,
        observed_at: u64,
    },
    KeepUncertain,
    Retry {
        failed_effect: SagaEffectIdV1,
        now: u64,
    },
    Compensate {
        now: u64,
    },
    Cancel {
        now: u64,
    },
    FailTerminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SagaCommandV1 {
    Admit {
        source: [u8; 16],
        now: u64,
        deadline: u64,
        retry_budget: u8,
    },
    AcceptEffect {
        saga: SagaIdV1,
        effect: SagaEffectIdV1,
        outcome: EffectOutcomeV1,
        observed_at: u64,
    },
    PrepareTerminal {
        saga: SagaIdV1,
        now: u64,
    },
    Retry {
        saga: SagaIdV1,
        failed_effect: SagaEffectIdV1,
        now: u64,
    },
    Cancel {
        saga: SagaIdV1,
        now: u64,
    },
    FireDeadline {
        saga: SagaIdV1,
        timer: TimerIdV1,
        observed_at: u64,
        action: DeadlineActionV1,
    },
    Reconcile {
        cause: OperatorCauseV1,
        saga: SagaIdV1,
        disposition: OperatorDispositionV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SagaTransitionV1 {
    Admitted {
        saga: SagaIdV1,
        effect: SagaEffectIdV1,
        timer: TimerIdV1,
    },
    EffectPrepared {
        saga: SagaIdV1,
        effect: SagaEffectIdV1,
        kind: EffectKindV1,
        retry: u8,
    },
    EffectAccepted {
        saga: SagaIdV1,
        effect: SagaEffectIdV1,
        outcome: EffectOutcomeV1,
        phase: SagaPhaseV1,
    },
    DeadlineApplied {
        saga: SagaIdV1,
        timer: TimerIdV1,
        action: DeadlineActionV1,
        phase: SagaPhaseV1,
        compensation: Option<SagaEffectIdV1>,
    },
    Cancelled {
        saga: SagaIdV1,
        compensation: Option<SagaEffectIdV1>,
    },
    Reconciled {
        saga: SagaIdV1,
        action: OperatorActionIdV1,
        phase: SagaPhaseV1,
        prepared: Option<SagaEffectIdV1>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SagaRefusalV1 {
    InvalidLimits,
    InvalidPageLimit,
    RevisionConflict { expected: u64, actual: u64 },
    RevisionExhausted,
    IdentityExhausted,
    SagaLimit,
    InvalidDeadline,
    RetryBudget,
    InputConflict,
    UnknownSaga,
    InvalidPhase,
    EffectMismatch,
    EvidenceConflict,
    DeadlineNotReached,
    DeadlineReached,
    TimerMismatch,
    TimerConflict,
    LateEvidence,
    InvalidDeadlineAction,
    InvalidCheckpoint,
    OperatorActionLimit,
    OperatorActionConflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SagaSnapshotV1 {
    pub saga: SagaIdV1,
    pub phase: SagaPhaseV1,
    pub deadline: u64,
    pub timer: TimerIdV1,
    pub current_effect: Option<SagaEffectIdV1>,
    pub current_kind: Option<EffectKindV1>,
    pub retries_used: u8,
    pub retry_budget: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SagaPageV1 {
    pub entries: Vec<SagaSnapshotV1>,
    pub next_after: Option<SagaIdV1>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SagaCensusV1 {
    pub total: u64,
    pub runnable_effects: u64,
    pub awaiting_decision: u64,
    pub fenced: u64,
    pub terminal: u64,
    pub retries_consumed: u64,
    pub saturated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutomaticProgressV1 {
    Runnable,
    AwaitingDecision,
    BlockedUncertain,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FenceCauseV1 {
    EffectUncertain {
        effect: SagaEffectIdV1,
        kind: EffectKindV1,
        observed_at: u64,
    },
    Deadline {
        timer: TimerIdV1,
        observed_at: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SagaReconciliationViewV1 {
    pub snapshot: SagaSnapshotV1,
    pub last_confirmed_revision: u64,
    pub retained_evidence: u64,
    pub fence: Option<FenceCauseV1>,
    pub automatic_progress: AutomaticProgressV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectEvidenceSummaryV1 {
    pub effect: SagaEffectIdV1,
    pub outcome: EffectOutcomeV1,
    pub observed_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectEvidencePageV1 {
    pub entries: Vec<EffectEvidenceSummaryV1>,
    pub next_after: Option<SagaEffectIdV1>,
}

/// Content-safe durable account of an authorized reconciliation. The verified
/// subject is deliberately absent; the authority decision remains attribution,
/// not a reusable credential.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperatorReceiptSummaryV1 {
    pub action: OperatorActionIdV1,
    pub saga: SagaIdV1,
    pub authority_decision: u64,
    pub authority_epoch: u64,
    pub disposition: OperatorDispositionV1,
    pub phase: SagaPhaseV1,
    pub prepared: Option<SagaEffectIdV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorReceiptPageV1 {
    pub entries: Vec<OperatorReceiptSummaryV1>,
    pub next_after: Option<OperatorActionIdV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AcceptedEvidence {
    outcome: EffectOutcomeV1,
    observed_at: u64,
    transition: SagaTransitionV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TimerEvidence {
    observed_at: u64,
    action: DeadlineActionV1,
    transition: SagaTransitionV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SagaState {
    source: [u8; 16],
    initial_effect: SagaEffectIdV1,
    snapshot: SagaSnapshotV1,
    retry_of: Option<SagaEffectIdV1>,
    allocated: BTreeSet<SagaEffectIdV1>,
    accepted: BTreeMap<SagaEffectIdV1, AcceptedEvidence>,
    timer_evidence: Option<TimerEvidence>,
    last_confirmed_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OperatorReceipt {
    cause: OperatorCauseV1,
    saga: SagaIdV1,
    disposition: OperatorDispositionV1,
    transition: SagaTransitionV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SagaCheckpointV1 {
    limits: SagaLimitsV1,
    revision: u64,
    next_saga: u64,
    next_effect: u64,
    sagas: BTreeMap<SagaIdV1, SagaState>,
    sources: BTreeMap<[u8; 16], SagaIdV1>,
    operator_actions: BTreeMap<OperatorActionIdV1, OperatorReceipt>,
}

impl SagaCheckpointV1 {
    pub const fn limits(&self) -> SagaLimitsV1 {
        self.limits
    }
}

pub struct SagaEngineV1 {
    state: SagaCheckpointV1,
}

impl SagaEngineV1 {
    pub fn try_new(limits: SagaLimitsV1) -> Result<Self, SagaRefusalV1> {
        if limits.max_sagas == 0
            || limits.max_inspection_page == 0
            || limits.max_operator_actions == 0
        {
            return Err(SagaRefusalV1::InvalidLimits);
        }
        Ok(Self {
            state: SagaCheckpointV1 {
                limits,
                revision: 0,
                next_saga: 0,
                next_effect: 0,
                sagas: BTreeMap::new(),
                sources: BTreeMap::new(),
                operator_actions: BTreeMap::new(),
            },
        })
    }

    pub const fn revision(&self) -> u64 {
        self.state.revision
    }

    pub fn saga(&self, id: SagaIdV1) -> Option<SagaSnapshotV1> {
        self.state.sagas.get(&id).map(|state| state.snapshot)
    }

    /// Stable identifier-ordered inspection. Ordering is observation only and
    /// does not select, lease or schedule work.
    pub fn inspect_page(
        &self,
        after: Option<SagaIdV1>,
        limit: usize,
    ) -> Result<SagaPageV1, SagaRefusalV1> {
        if limit == 0 || limit > self.state.limits.max_inspection_page {
            return Err(SagaRefusalV1::InvalidPageLimit);
        }
        let entries = self
            .state
            .sagas
            .range(after.map_or(SagaIdV1(0).., |id| SagaIdV1(id.0.saturating_add(1))..))
            .take(limit)
            .map(|(_, state)| state.snapshot)
            .collect::<Vec<_>>();
        let next_after = (entries.len() == limit).then(|| entries.last().unwrap().saga);
        Ok(SagaPageV1 {
            entries,
            next_after,
        })
    }

    /// Content-free phase census for fairness monitoring. Saturation is sticky
    /// within this computed value and carries no authority.
    pub fn census(&self) -> SagaCensusV1 {
        let mut value = SagaCensusV1::default();
        for state in self.state.sagas.values() {
            add(&mut value.total, 1, &mut value.saturated);
            add(
                &mut value.retries_consumed,
                u64::from(state.snapshot.retries_used),
                &mut value.saturated,
            );
            match state.snapshot.phase {
                SagaPhaseV1::ForwardPending
                | SagaPhaseV1::TerminalPending
                | SagaPhaseV1::CompensationPending => {
                    add(&mut value.runnable_effects, 1, &mut value.saturated)
                }
                SagaPhaseV1::ForwardSucceeded
                | SagaPhaseV1::ForwardFailed
                | SagaPhaseV1::TerminalFailed
                | SagaPhaseV1::CompensationFailed => {
                    add(&mut value.awaiting_decision, 1, &mut value.saturated)
                }
                SagaPhaseV1::FencedUncertain => add(&mut value.fenced, 1, &mut value.saturated),
                SagaPhaseV1::Completed | SagaPhaseV1::Failed | SagaPhaseV1::Cancelled => {
                    add(&mut value.terminal, 1, &mut value.saturated)
                }
            }
        }
        value
    }

    pub fn reconciliation_view(&self, saga: SagaIdV1) -> Option<SagaReconciliationViewV1> {
        let state = self.state.sagas.get(&saga)?;
        let automatic_progress = match state.snapshot.phase {
            SagaPhaseV1::ForwardPending
            | SagaPhaseV1::TerminalPending
            | SagaPhaseV1::CompensationPending => AutomaticProgressV1::Runnable,
            SagaPhaseV1::FencedUncertain => AutomaticProgressV1::BlockedUncertain,
            SagaPhaseV1::Completed | SagaPhaseV1::Failed | SagaPhaseV1::Cancelled => {
                AutomaticProgressV1::Terminal
            }
            SagaPhaseV1::ForwardSucceeded
            | SagaPhaseV1::ForwardFailed
            | SagaPhaseV1::TerminalFailed
            | SagaPhaseV1::CompensationFailed => AutomaticProgressV1::AwaitingDecision,
        };
        let fence = (state.snapshot.phase == SagaPhaseV1::FencedUncertain)
            .then(|| {
                state
                    .snapshot
                    .current_effect
                    .and_then(|effect| {
                        state.accepted.get(&effect).and_then(|evidence| {
                            (evidence.outcome == EffectOutcomeV1::Uncertain).then_some(
                                FenceCauseV1::EffectUncertain {
                                    effect,
                                    kind: state.snapshot.current_kind?,
                                    observed_at: evidence.observed_at,
                                },
                            )
                        })
                    })
                    .or_else(|| {
                        state.timer_evidence.map(|evidence| FenceCauseV1::Deadline {
                            timer: state.snapshot.timer,
                            observed_at: evidence.observed_at,
                        })
                    })
            })
            .flatten();
        Some(SagaReconciliationViewV1 {
            snapshot: state.snapshot,
            last_confirmed_revision: state.last_confirmed_revision,
            retained_evidence: u64::try_from(state.accepted.len()).unwrap_or(u64::MAX),
            fence,
            automatic_progress,
        })
    }

    pub fn evidence_page(
        &self,
        saga: SagaIdV1,
        after: Option<SagaEffectIdV1>,
        limit: usize,
    ) -> Result<EffectEvidencePageV1, SagaRefusalV1> {
        if limit == 0 || limit > self.state.limits.max_inspection_page {
            return Err(SagaRefusalV1::InvalidPageLimit);
        }
        let state = self
            .state
            .sagas
            .get(&saga)
            .ok_or(SagaRefusalV1::UnknownSaga)?;
        let entries = state
            .accepted
            .range(after.map_or(SagaEffectIdV1(0).., |effect| {
                SagaEffectIdV1(effect.0.saturating_add(1))..
            }))
            .take(limit)
            .map(|(effect, evidence)| EffectEvidenceSummaryV1 {
                effect: *effect,
                outcome: evidence.outcome,
                observed_at: evidence.observed_at,
            })
            .collect::<Vec<_>>();
        let next_after = (entries.len() == limit).then(|| entries.last().unwrap().effect);
        Ok(EffectEvidencePageV1 {
            entries,
            next_after,
        })
    }

    pub fn operator_receipt(&self, action: OperatorActionIdV1) -> Option<OperatorReceiptSummaryV1> {
        self.state
            .operator_actions
            .get(&action)
            .map(operator_summary)
    }

    /// Stable action-identifier ordering is observation only. It neither
    /// reauthorizes the retained decision nor exposes the verified subject.
    pub fn operator_receipt_page(
        &self,
        after: Option<OperatorActionIdV1>,
        limit: usize,
    ) -> Result<OperatorReceiptPageV1, SagaRefusalV1> {
        if limit == 0 || limit > self.state.limits.max_inspection_page {
            return Err(SagaRefusalV1::InvalidPageLimit);
        }
        let entries = self
            .state
            .operator_actions
            .range((
                after.map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded),
                std::ops::Bound::Unbounded,
            ))
            .take(limit)
            .map(|(_, receipt)| operator_summary(receipt))
            .collect::<Vec<_>>();
        let next_after = (entries.len() == limit).then(|| entries.last().unwrap().action);
        Ok(OperatorReceiptPageV1 {
            entries,
            next_after,
        })
    }

    pub fn checkpoint(&self) -> SagaCheckpointV1 {
        self.state.clone()
    }

    pub fn recover(state: SagaCheckpointV1) -> Result<Self, SagaRefusalV1> {
        if state.limits.max_sagas == 0
            || state.limits.max_inspection_page == 0
            || state.limits.max_operator_actions == 0
            || state.operator_actions.len() > state.limits.max_operator_actions
            || state.sagas.len() > state.limits.max_sagas
            || state.sources.len() != state.sagas.len()
            || state.next_saga != state.sagas.len() as u64
            || state.sources.iter().any(|(source, saga)| {
                state
                    .sagas
                    .get(saga)
                    .is_none_or(|value| value.source != *source || value.snapshot.saga != *saga)
            })
            || state.sagas.values().any(|value| {
                !valid_state(
                    value,
                    state.next_effect,
                    state.revision,
                    state.limits.max_retries_per_saga,
                )
            })
            || state.operator_actions.iter().any(|(action, receipt)| {
                receipt.cause.action != *action
                    || !state.sagas.contains_key(&receipt.saga)
                    || !matches!(
                        receipt.transition,
                        SagaTransitionV1::Reconciled {
                            saga,
                            action: retained,
                            ..
                        } if saga == receipt.saga && retained == *action
                    )
            })
        {
            return Err(SagaRefusalV1::InvalidCheckpoint);
        }
        let mut effects = BTreeSet::new();
        let mut transitions =
            u64::try_from(state.sagas.len()).map_err(|_| SagaRefusalV1::InvalidCheckpoint)?;
        for value in state.sagas.values() {
            for effect in &value.allocated {
                effects.insert(effect.0);
            }
            transitions = transitions
                .checked_add(
                    u64::try_from(value.accepted.len())
                        .map_err(|_| SagaRefusalV1::InvalidCheckpoint)?,
                )
                .ok_or(SagaRefusalV1::InvalidCheckpoint)?;
            if value.timer_evidence.is_some_and(|evidence| {
                matches!(
                    evidence.transition,
                    SagaTransitionV1::DeadlineApplied {
                        compensation: None,
                        ..
                    }
                )
            }) {
                transitions = transitions
                    .checked_add(1)
                    .ok_or(SagaRefusalV1::InvalidCheckpoint)?;
            }
            if value.snapshot.phase == SagaPhaseV1::Cancelled
                && value.snapshot.current_kind == Some(EffectKindV1::Forward)
            {
                transitions = transitions
                    .checked_add(1)
                    .ok_or(SagaRefusalV1::InvalidCheckpoint)?;
            }
        }
        let effect_count =
            u64::try_from(effects.len()).map_err(|_| SagaRefusalV1::InvalidCheckpoint)?;
        let prepared = state
            .next_effect
            .checked_sub(state.next_saga)
            .ok_or(SagaRefusalV1::InvalidCheckpoint)?;
        transitions = transitions
            .checked_add(prepared)
            .ok_or(SagaRefusalV1::InvalidCheckpoint)?;
        transitions = transitions
            .checked_add(
                u64::try_from(state.operator_actions.len())
                    .map_err(|_| SagaRefusalV1::InvalidCheckpoint)?,
            )
            .ok_or(SagaRefusalV1::InvalidCheckpoint)?;
        if effect_count != state.next_effect
            || effects
                .iter()
                .enumerate()
                .any(|(index, effect)| u64::try_from(index).ok() != Some(*effect))
            || transitions != state.revision
        {
            return Err(SagaRefusalV1::InvalidCheckpoint);
        }
        Ok(Self { state })
    }

    pub fn apply(
        &mut self,
        expected_revision: u64,
        command: SagaCommandV1,
    ) -> Result<SagaTransitionV1, SagaRefusalV1> {
        if let SagaCommandV1::Reconcile {
            cause,
            saga,
            disposition,
        } = command
            && let Some(receipt) = self.state.operator_actions.get(&cause.action)
        {
            return if same_operator_request(receipt, cause, saga, disposition) {
                Ok(receipt.transition)
            } else {
                Err(SagaRefusalV1::OperatorActionConflict)
            };
        }
        if self.state.revision == u64::MAX {
            return Err(SagaRefusalV1::RevisionExhausted);
        }
        if expected_revision != self.state.revision {
            return Err(SagaRefusalV1::RevisionConflict {
                expected: expected_revision,
                actual: self.state.revision,
            });
        }
        match command {
            SagaCommandV1::Admit {
                source,
                now,
                deadline,
                retry_budget,
            } => self.admit(source, now, deadline, retry_budget),
            SagaCommandV1::AcceptEffect {
                saga,
                effect,
                outcome,
                observed_at,
            } => self.accept(saga, effect, outcome, observed_at),
            SagaCommandV1::PrepareTerminal { saga, now } => self.prepare_terminal(saga, now),
            SagaCommandV1::Retry {
                saga,
                failed_effect,
                now,
            } => self.retry(saga, failed_effect, now),
            SagaCommandV1::Cancel { saga, now } => self.cancel(saga, now),
            SagaCommandV1::FireDeadline {
                saga,
                timer,
                observed_at,
                action,
            } => self.fire_deadline(saga, timer, observed_at, action),
            SagaCommandV1::Reconcile {
                cause,
                saga,
                disposition,
            } => {
                let mut candidate = Self::recover(self.checkpoint())?;
                let transition = candidate.reconcile(cause, saga, disposition)?;
                self.state = candidate.state;
                Ok(transition)
            }
        }
    }

    fn reconcile(
        &mut self,
        cause: OperatorCauseV1,
        saga: SagaIdV1,
        disposition: OperatorDispositionV1,
    ) -> Result<SagaTransitionV1, SagaRefusalV1> {
        if let Some(receipt) = self.state.operator_actions.get(&cause.action) {
            return if same_operator_request(receipt, cause, saga, disposition) {
                Ok(receipt.transition)
            } else {
                Err(SagaRefusalV1::OperatorActionConflict)
            };
        }
        if self.state.operator_actions.len() == self.state.limits.max_operator_actions {
            return Err(SagaRefusalV1::OperatorActionLimit);
        }
        let snapshot = self
            .state
            .sagas
            .get(&saga)
            .ok_or(SagaRefusalV1::UnknownSaga)?
            .snapshot;
        let mut prepared = None;
        match disposition {
            OperatorDispositionV1::AdmitEvidence {
                effect, outcome, ..
            } => {
                if snapshot.phase != SagaPhaseV1::FencedUncertain
                    || snapshot.current_effect != Some(effect)
                    || outcome == EffectOutcomeV1::Uncertain
                {
                    return Err(SagaRefusalV1::InvalidPhase);
                }
                let kind = snapshot.current_kind.ok_or(SagaRefusalV1::EffectMismatch)?;
                let phase = adjudicated_phase(kind, outcome);
                self.state
                    .sagas
                    .get_mut(&saga)
                    .expect("validated saga")
                    .snapshot
                    .phase = phase;
            }
            OperatorDispositionV1::KeepUncertain => {
                if snapshot.phase != SagaPhaseV1::FencedUncertain {
                    return Err(SagaRefusalV1::InvalidPhase);
                }
            }
            OperatorDispositionV1::Retry { failed_effect, now } => {
                let value = self.retry(saga, failed_effect, now)?;
                let SagaTransitionV1::EffectPrepared { effect, .. } = value else {
                    unreachable!()
                };
                prepared = Some(effect);
            }
            OperatorDispositionV1::Compensate { now } => {
                if snapshot.phase != SagaPhaseV1::FencedUncertain
                    || snapshot.current_kind == Some(EffectKindV1::Compensation)
                {
                    return Err(SagaRefusalV1::InvalidPhase);
                }
                let value = self.prepare(saga, EffectKindV1::Compensation, None, now)?;
                let SagaTransitionV1::EffectPrepared { effect, .. } = value else {
                    unreachable!()
                };
                prepared = Some(effect);
            }
            OperatorDispositionV1::Cancel { now } => {
                if snapshot.phase == SagaPhaseV1::FencedUncertain {
                    if snapshot.current_kind == Some(EffectKindV1::Compensation) {
                        return Err(SagaRefusalV1::InvalidPhase);
                    }
                    let value = self.prepare(saga, EffectKindV1::Compensation, None, now)?;
                    let SagaTransitionV1::EffectPrepared { effect, .. } = value else {
                        unreachable!()
                    };
                    prepared = Some(effect);
                } else {
                    let value = self.cancel(saga, now)?;
                    if let SagaTransitionV1::Cancelled { compensation, .. } = value {
                        prepared = compensation;
                    }
                }
            }
            OperatorDispositionV1::FailTerminal => {
                if !matches!(
                    snapshot.phase,
                    SagaPhaseV1::FencedUncertain
                        | SagaPhaseV1::ForwardFailed
                        | SagaPhaseV1::TerminalFailed
                        | SagaPhaseV1::CompensationFailed
                ) {
                    return Err(SagaRefusalV1::InvalidPhase);
                }
                self.state
                    .sagas
                    .get_mut(&saga)
                    .expect("validated saga")
                    .snapshot
                    .phase = SagaPhaseV1::Failed;
            }
        }
        let phase = self.state.sagas[&saga].snapshot.phase;
        let transition = SagaTransitionV1::Reconciled {
            saga,
            action: cause.action,
            phase,
            prepared,
        };
        self.state.operator_actions.insert(
            cause.action,
            OperatorReceipt {
                cause,
                saga,
                disposition,
                transition,
            },
        );
        self.advance_revision()?;
        self.state
            .sagas
            .get_mut(&saga)
            .expect("validated saga")
            .last_confirmed_revision = self.state.revision;
        Ok(transition)
    }

    fn admit(
        &mut self,
        source: [u8; 16],
        now: u64,
        deadline: u64,
        retry_budget: u8,
    ) -> Result<SagaTransitionV1, SagaRefusalV1> {
        if let Some(id) = self.state.sources.get(&source) {
            let value = self.state.sagas.get(id).expect("source index is valid");
            if value.snapshot.deadline != deadline || value.snapshot.retry_budget != retry_budget {
                return Err(SagaRefusalV1::InputConflict);
            }
            return Ok(SagaTransitionV1::Admitted {
                saga: *id,
                effect: value.initial_effect,
                timer: value.snapshot.timer,
            });
        }
        if deadline <= now {
            return Err(SagaRefusalV1::InvalidDeadline);
        }
        if retry_budget > self.state.limits.max_retries_per_saga {
            return Err(SagaRefusalV1::RetryBudget);
        }
        if self.state.sagas.len() == self.state.limits.max_sagas {
            return Err(SagaRefusalV1::SagaLimit);
        }
        let saga = SagaIdV1(self.state.next_saga);
        let effect = SagaEffectIdV1(self.state.next_effect);
        let timer = TimerIdV1(self.state.next_saga);
        let next_saga = self
            .state
            .next_saga
            .checked_add(1)
            .ok_or(SagaRefusalV1::IdentityExhausted)?;
        let next_effect = self
            .state
            .next_effect
            .checked_add(1)
            .ok_or(SagaRefusalV1::IdentityExhausted)?;
        let transition = SagaTransitionV1::Admitted {
            saga,
            effect,
            timer,
        };
        self.state.sagas.insert(
            saga,
            SagaState {
                source,
                initial_effect: effect,
                snapshot: SagaSnapshotV1 {
                    saga,
                    phase: SagaPhaseV1::ForwardPending,
                    deadline,
                    timer,
                    current_effect: Some(effect),
                    current_kind: Some(EffectKindV1::Forward),
                    retries_used: 0,
                    retry_budget,
                },
                retry_of: None,
                allocated: BTreeSet::from([effect]),
                accepted: BTreeMap::new(),
                timer_evidence: None,
                last_confirmed_revision: self.state.revision + 1,
            },
        );
        self.state.sources.insert(source, saga);
        self.state.next_saga = next_saga;
        self.state.next_effect = next_effect;
        self.advance_revision()?;
        Ok(transition)
    }

    fn accept(
        &mut self,
        saga: SagaIdV1,
        effect: SagaEffectIdV1,
        outcome: EffectOutcomeV1,
        observed_at: u64,
    ) -> Result<SagaTransitionV1, SagaRefusalV1> {
        let state = self
            .state
            .sagas
            .get(&saga)
            .ok_or(SagaRefusalV1::UnknownSaga)?;
        if let Some(evidence) = state.accepted.get(&effect) {
            return if evidence.outcome == outcome && evidence.observed_at == observed_at {
                Ok(evidence.transition)
            } else {
                Err(SagaRefusalV1::EvidenceConflict)
            };
        }
        if matches!(
            state.snapshot.phase,
            SagaPhaseV1::Cancelled
                | SagaPhaseV1::Failed
                | SagaPhaseV1::Completed
                | SagaPhaseV1::FencedUncertain
        ) || state.snapshot.current_effect != Some(effect)
        {
            return Err(SagaRefusalV1::LateEvidence);
        }
        let kind = state
            .snapshot
            .current_kind
            .ok_or(SagaRefusalV1::EffectMismatch)?;
        let phase = match (kind, outcome) {
            (_, EffectOutcomeV1::Uncertain) => SagaPhaseV1::FencedUncertain,
            (EffectKindV1::Forward, EffectOutcomeV1::Succeeded) => SagaPhaseV1::ForwardSucceeded,
            (EffectKindV1::Forward, EffectOutcomeV1::Failed) => SagaPhaseV1::ForwardFailed,
            (EffectKindV1::Terminal, EffectOutcomeV1::Succeeded) => SagaPhaseV1::Completed,
            (EffectKindV1::Terminal, EffectOutcomeV1::Failed) => SagaPhaseV1::TerminalFailed,
            (EffectKindV1::Compensation, EffectOutcomeV1::Succeeded) => SagaPhaseV1::Cancelled,
            (EffectKindV1::Compensation, EffectOutcomeV1::Failed) => {
                SagaPhaseV1::CompensationFailed
            }
        };
        let transition = SagaTransitionV1::EffectAccepted {
            saga,
            effect,
            outcome,
            phase,
        };
        let state = self.state.sagas.get_mut(&saga).expect("validated saga");
        state.snapshot.phase = phase;
        state.accepted.insert(
            effect,
            AcceptedEvidence {
                outcome,
                observed_at,
                transition,
            },
        );
        self.advance_revision()?;
        self.state
            .sagas
            .get_mut(&saga)
            .expect("validated saga")
            .last_confirmed_revision = self.state.revision;
        Ok(transition)
    }

    fn prepare_terminal(
        &mut self,
        saga: SagaIdV1,
        now: u64,
    ) -> Result<SagaTransitionV1, SagaRefusalV1> {
        let state = self
            .state
            .sagas
            .get(&saga)
            .ok_or(SagaRefusalV1::UnknownSaga)?;
        if state.snapshot.current_kind == Some(EffectKindV1::Terminal) {
            return Ok(prepared(state.snapshot, EffectKindV1::Terminal));
        }
        if state.snapshot.phase != SagaPhaseV1::ForwardSucceeded {
            return Err(SagaRefusalV1::InvalidPhase);
        }
        self.prepare(saga, EffectKindV1::Terminal, None, now)
    }

    fn retry(
        &mut self,
        saga: SagaIdV1,
        failed_effect: SagaEffectIdV1,
        now: u64,
    ) -> Result<SagaTransitionV1, SagaRefusalV1> {
        let state = self
            .state
            .sagas
            .get(&saga)
            .ok_or(SagaRefusalV1::UnknownSaga)?;
        if state.retry_of == Some(failed_effect) {
            return Ok(prepared(
                state.snapshot,
                state
                    .snapshot
                    .current_kind
                    .ok_or(SagaRefusalV1::InvalidPhase)?,
            ));
        }
        if state.snapshot.current_effect != Some(failed_effect)
            || !matches!(
                state.snapshot.phase,
                SagaPhaseV1::ForwardFailed
                    | SagaPhaseV1::TerminalFailed
                    | SagaPhaseV1::CompensationFailed
            )
        {
            return Err(SagaRefusalV1::InvalidPhase);
        }
        if state.snapshot.retries_used == state.snapshot.retry_budget {
            return Err(SagaRefusalV1::RetryBudget);
        }
        let kind = state
            .snapshot
            .current_kind
            .ok_or(SagaRefusalV1::InvalidPhase)?;
        self.prepare(saga, kind, Some(failed_effect), now)
    }

    fn cancel(&mut self, saga: SagaIdV1, now: u64) -> Result<SagaTransitionV1, SagaRefusalV1> {
        let state = self
            .state
            .sagas
            .get(&saga)
            .ok_or(SagaRefusalV1::UnknownSaga)?;
        if state.snapshot.phase == SagaPhaseV1::Cancelled {
            return Ok(SagaTransitionV1::Cancelled {
                saga,
                compensation: state
                    .snapshot
                    .current_kind
                    .eq(&Some(EffectKindV1::Compensation))
                    .then_some(state.snapshot.current_effect)
                    .flatten(),
            });
        }
        if state.snapshot.phase == SagaPhaseV1::CompensationPending
            && state.snapshot.current_kind == Some(EffectKindV1::Compensation)
        {
            return Ok(SagaTransitionV1::Cancelled {
                saga,
                compensation: state.snapshot.current_effect,
            });
        }
        match state.snapshot.phase {
            SagaPhaseV1::ForwardPending | SagaPhaseV1::ForwardFailed => {
                let state = self.state.sagas.get_mut(&saga).expect("validated saga");
                state.snapshot.phase = SagaPhaseV1::Cancelled;
                let value = SagaTransitionV1::Cancelled {
                    saga,
                    compensation: None,
                };
                self.advance_revision()?;
                self.state
                    .sagas
                    .get_mut(&saga)
                    .expect("validated saga")
                    .last_confirmed_revision = self.state.revision;
                Ok(value)
            }
            SagaPhaseV1::ForwardSucceeded
            | SagaPhaseV1::TerminalPending
            | SagaPhaseV1::TerminalFailed => {
                let prepared = self.prepare(saga, EffectKindV1::Compensation, None, now)?;
                let SagaTransitionV1::EffectPrepared { effect, .. } = prepared else {
                    unreachable!()
                };
                Ok(SagaTransitionV1::Cancelled {
                    saga,
                    compensation: Some(effect),
                })
            }
            _ => Err(SagaRefusalV1::InvalidPhase),
        }
    }

    fn fire_deadline(
        &mut self,
        saga: SagaIdV1,
        timer: TimerIdV1,
        observed_at: u64,
        action: DeadlineActionV1,
    ) -> Result<SagaTransitionV1, SagaRefusalV1> {
        let state = self
            .state
            .sagas
            .get(&saga)
            .ok_or(SagaRefusalV1::UnknownSaga)?;
        if state.snapshot.timer != timer {
            return Err(SagaRefusalV1::TimerMismatch);
        }
        if let Some(evidence) = state.timer_evidence {
            return if evidence.observed_at == observed_at && evidence.action == action {
                Ok(evidence.transition)
            } else {
                Err(SagaRefusalV1::TimerConflict)
            };
        }
        if observed_at < state.snapshot.deadline {
            return Err(SagaRefusalV1::DeadlineNotReached);
        }
        let current = state.snapshot.phase;
        let (phase, compensation) = match (current, action) {
            (
                SagaPhaseV1::ForwardPending | SagaPhaseV1::ForwardFailed,
                DeadlineActionV1::KeepUncertain,
            ) => (SagaPhaseV1::FencedUncertain, None),
            (SagaPhaseV1::ForwardPending | SagaPhaseV1::ForwardFailed, DeadlineActionV1::Fail) => {
                (SagaPhaseV1::Failed, None)
            }
            (
                SagaPhaseV1::TerminalPending | SagaPhaseV1::TerminalFailed,
                DeadlineActionV1::KeepUncertain,
            )
            | (
                SagaPhaseV1::CompensationPending | SagaPhaseV1::CompensationFailed,
                DeadlineActionV1::KeepUncertain,
            ) => (SagaPhaseV1::FencedUncertain, None),
            (
                SagaPhaseV1::CompensationPending | SagaPhaseV1::CompensationFailed,
                DeadlineActionV1::Fail,
            ) => (SagaPhaseV1::Failed, None),
            (
                SagaPhaseV1::TerminalPending | SagaPhaseV1::TerminalFailed,
                DeadlineActionV1::Compensate,
            ) => {
                let prepared = self.prepare(saga, EffectKindV1::Compensation, None, observed_at)?;
                let SagaTransitionV1::EffectPrepared { effect, .. } = prepared else {
                    unreachable!()
                };
                (SagaPhaseV1::CompensationPending, Some(effect))
            }
            _ => return Err(SagaRefusalV1::InvalidDeadlineAction),
        };
        let transition = SagaTransitionV1::DeadlineApplied {
            saga,
            timer,
            action,
            phase,
            compensation,
        };
        let state = self.state.sagas.get_mut(&saga).expect("validated saga");
        state.snapshot.phase = phase;
        state.timer_evidence = Some(TimerEvidence {
            observed_at,
            action,
            transition,
        });
        if compensation.is_none() {
            self.advance_revision()?;
            self.state
                .sagas
                .get_mut(&saga)
                .expect("validated saga")
                .last_confirmed_revision = self.state.revision;
        }
        Ok(transition)
    }

    fn prepare(
        &mut self,
        saga: SagaIdV1,
        kind: EffectKindV1,
        retry_of: Option<SagaEffectIdV1>,
        now: u64,
    ) -> Result<SagaTransitionV1, SagaRefusalV1> {
        let state = self
            .state
            .sagas
            .get(&saga)
            .ok_or(SagaRefusalV1::UnknownSaga)?;
        if now >= state.snapshot.deadline && kind != EffectKindV1::Compensation {
            return Err(SagaRefusalV1::DeadlineReached);
        }
        let effect = SagaEffectIdV1(self.state.next_effect);
        self.state.next_effect = self
            .state
            .next_effect
            .checked_add(1)
            .ok_or(SagaRefusalV1::IdentityExhausted)?;
        let state = self.state.sagas.get_mut(&saga).expect("validated saga");
        state.snapshot.current_effect = Some(effect);
        state.snapshot.current_kind = Some(kind);
        state.snapshot.phase = match kind {
            EffectKindV1::Forward => SagaPhaseV1::ForwardPending,
            EffectKindV1::Terminal => SagaPhaseV1::TerminalPending,
            EffectKindV1::Compensation => SagaPhaseV1::CompensationPending,
        };
        if retry_of.is_some() {
            state.snapshot.retries_used = state
                .snapshot
                .retries_used
                .checked_add(1)
                .ok_or(SagaRefusalV1::RetryBudget)?;
        }
        state.retry_of = retry_of;
        state.allocated.insert(effect);
        let transition = prepared(state.snapshot, kind);
        self.advance_revision()?;
        self.state
            .sagas
            .get_mut(&saga)
            .expect("validated saga")
            .last_confirmed_revision = self.state.revision;
        Ok(transition)
    }

    fn advance_revision(&mut self) -> Result<(), SagaRefusalV1> {
        self.state.revision = self
            .state
            .revision
            .checked_add(1)
            .ok_or(SagaRefusalV1::RevisionExhausted)?;
        Ok(())
    }
}

fn operator_summary(receipt: &OperatorReceipt) -> OperatorReceiptSummaryV1 {
    let SagaTransitionV1::Reconciled {
        phase, prepared, ..
    } = receipt.transition
    else {
        unreachable!("operator receipts retain reconciled transitions")
    };
    OperatorReceiptSummaryV1 {
        action: receipt.cause.action,
        saga: receipt.saga,
        authority_decision: receipt.cause.authority_decision,
        authority_epoch: receipt.cause.authority_epoch,
        disposition: receipt.disposition,
        phase,
        prepared,
    }
}

fn prepared(snapshot: SagaSnapshotV1, kind: EffectKindV1) -> SagaTransitionV1 {
    SagaTransitionV1::EffectPrepared {
        saga: snapshot.saga,
        effect: snapshot.current_effect.expect("prepared effect"),
        kind,
        retry: snapshot.retries_used,
    }
}

fn adjudicated_phase(kind: EffectKindV1, outcome: EffectOutcomeV1) -> SagaPhaseV1 {
    match (kind, outcome) {
        (EffectKindV1::Forward, EffectOutcomeV1::Succeeded) => SagaPhaseV1::ForwardSucceeded,
        (EffectKindV1::Forward, EffectOutcomeV1::Failed) => SagaPhaseV1::ForwardFailed,
        (EffectKindV1::Terminal, EffectOutcomeV1::Succeeded) => SagaPhaseV1::Completed,
        (EffectKindV1::Terminal, EffectOutcomeV1::Failed) => SagaPhaseV1::TerminalFailed,
        (EffectKindV1::Compensation, EffectOutcomeV1::Succeeded) => SagaPhaseV1::Cancelled,
        (EffectKindV1::Compensation, EffectOutcomeV1::Failed) => SagaPhaseV1::CompensationFailed,
        (_, EffectOutcomeV1::Uncertain) => unreachable!("validated operator evidence"),
    }
}

fn same_operator_request(
    receipt: &OperatorReceipt,
    cause: OperatorCauseV1,
    saga: SagaIdV1,
    disposition: OperatorDispositionV1,
) -> bool {
    receipt.cause.action == cause.action
        && receipt.cause.subject == cause.subject
        && receipt.saga == saga
        && receipt.disposition == disposition
}

fn valid_state(value: &SagaState, next_effect: u64, revision: u64, max_retries: u8) -> bool {
    let snapshot = value.snapshot;
    value.last_confirmed_revision > 0
        && value.last_confirmed_revision <= revision
        && snapshot.retry_budget <= max_retries
        && snapshot.retries_used <= snapshot.retry_budget
        && value.initial_effect.0 < next_effect
        && value.allocated.contains(&value.initial_effect)
        && snapshot
            .current_effect
            .is_none_or(|effect| value.allocated.contains(&effect))
        && value
            .accepted
            .keys()
            .all(|effect| value.allocated.contains(effect))
        && value.allocated.iter().all(|effect| effect.0 < next_effect)
        && snapshot
            .current_effect
            .is_none_or(|effect| effect.0 < next_effect)
        && value.accepted.keys().all(|effect| effect.0 < next_effect)
        && value.accepted.iter().all(|(effect, evidence)| {
            matches!(
                evidence.transition,
                SagaTransitionV1::EffectAccepted {
                    saga,
                    effect: retained,
                    outcome,
                    ..
                } if saga == snapshot.saga && retained == *effect && outcome == evidence.outcome
            )
        })
        && value
            .timer_evidence
            .is_none_or(|evidence| evidence.observed_at >= snapshot.deadline)
}

fn add(value: &mut u64, amount: u64, saturated: &mut bool) {
    if let Some(next) = value.checked_add(amount) {
        *value = next;
    } else {
        *saturated = true;
    }
}
