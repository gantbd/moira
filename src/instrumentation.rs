//! Optional command observations, not authority, audit, persistence or peer
//! effect evidence. No clock, I/O, callback, exporter or sensitive labels.
use crate::{
    CommandV1, DaimonDispositionV1, Engine, RefusalV1, TransitionV1, WorkflowPhaseV1,
    WorkflowRevision,
};
use keystone_observe::CaptureBuffer;
use std::{collections::TryReserveError, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    AdmitMessage,
    AcceptChronosAdmission,
    AcceptChronosAttempt,
    AcceptDaimonResult,
    PrepareAgoraResponse,
    AcceptAgoraPublication,
}
impl Operation {
    fn index(self) -> usize {
        match self {
            Self::AdmitMessage => 0,
            Self::AcceptChronosAdmission => 1,
            Self::AcceptChronosAttempt => 2,
            Self::AcceptDaimonResult => 3,
            Self::PrepareAgoraResponse => 4,
            Self::AcceptAgoraPublication => 5,
        }
    }
    fn of(command: CommandV1) -> Self {
        match command {
            CommandV1::AdmitMessage { .. } => Self::AdmitMessage,
            CommandV1::AcceptChronosAdmission { .. } => Self::AcceptChronosAdmission,
            CommandV1::AcceptChronosAttempt { .. } => Self::AcceptChronosAttempt,
            CommandV1::AcceptDaimonResult { .. } => Self::AcceptDaimonResult,
            CommandV1::PrepareAgoraResponse { .. } => Self::PrepareAgoraResponse,
            CommandV1::AcceptAgoraPublication { .. } => Self::AcceptAgoraPublication,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalKind {
    Stale,
    Exhausted,
    Limit,
    Conflict,
    Unavailable,
    Attribution,
}
impl RefusalKind {
    fn index(self) -> usize {
        match self {
            Self::Stale => 0,
            Self::Exhausted => 1,
            Self::Limit => 2,
            Self::Conflict => 3,
            Self::Unavailable => 4,
            Self::Attribution => 5,
        }
    }
    fn of(error: RefusalV1) -> Self {
        match error {
            RefusalV1::RevisionConflict { .. } => Self::Stale,
            RefusalV1::RevisionExhausted
            | RefusalV1::WorkflowIdentityExhausted
            | RefusalV1::EffectIdentityExhausted => Self::Exhausted,
            RefusalV1::WorkflowLimit => Self::Limit,
            RefusalV1::InputConflict | RefusalV1::ReceiptConflict => Self::Conflict,
            RefusalV1::UnknownWorkflow | RefusalV1::InvalidPhase => Self::Unavailable,
            RefusalV1::EffectMismatch | RefusalV1::AttemptMismatch => Self::Attribution,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Disposition {
    /// Volatile domain acceptance only; never durable commitment/peer success.
    Accepted,
    Replayed,
    Refused(RefusalKind),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Observation {
    pub operation: Operation,
    pub disposition: Disposition,
    /// Accepted/replayed receipt classification, not proof of connector truth.
    pub harness_disposition: Option<DaimonDispositionV1>,
    /// Core emits None: it reads no clock. None is unknown, never zero latency.
    pub service_time: Option<Duration>,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OperationCounts {
    pub offered: u64,
    pub accepted: u64,
    pub replayed: u64,
    pub refused: u64,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HarnessReceiptCounts {
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub timed_out: u64,
    pub uncertain: u64,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    pub offered: u64,
    pub accepted: u64,
    pub replayed: u64,
    pub refused: u64,
    /// Newly accepted receipts only; matching replay does not double count.
    pub harness_receipts: HarnessReceiptCounts,
    /// Sticky saturation indication; counters never wrap or reset on drain.
    pub overflowed: bool,
    operations: [OperationCounts; 6],
    refusals: [u64; 6],
}
impl Counts {
    pub fn for_operation(&self, operation: Operation) -> OperationCounts {
        self.operations[operation.index()]
    }
    pub fn for_refusal(&self, kind: RefusalKind) -> u64 {
        self.refusals[kind.index()]
    }
}
fn increment(value: &mut u64, overflowed: &mut bool) {
    if let Some(next) = value.checked_add(1) {
        *value = next;
    } else {
        *overflowed = true;
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureUsage {
    pub enabled: bool,
    pub queued: usize,
    pub capacity: usize,
    pub saturated: bool,
    pub dropped: u64,
    pub dropped_at_ceiling: bool,
}
#[derive(Debug)]
pub struct Recorder {
    buffer: CaptureBuffer<Observation>,
    counts: Counts,
}
impl Recorder {
    pub fn disabled() -> Self {
        Self {
            buffer: CaptureBuffer::disabled(),
            counts: Counts::default(),
        }
    }
    /// Preallocate outside transitions. Enabled zero capacity drops every event;
    /// disabled capture does not count opt-out as overload. Counts remain live.
    pub fn try_with_capacity(capacity: usize) -> Result<Self, TryReserveError> {
        Ok(Self {
            buffer: CaptureBuffer::try_with_capacity(capacity)?,
            counts: Counts::default(),
        })
    }
    pub fn counts(&self) -> Counts {
        self.counts
    }
    pub fn usage(&self) -> CaptureUsage {
        CaptureUsage {
            enabled: self.buffer.is_enabled(),
            queued: self.buffer.queued(),
            capacity: self.buffer.capacity(),
            saturated: self.buffer.is_enabled() && self.buffer.is_saturated(),
            dropped: self.buffer.dropped(),
            dropped_at_ceiling: self.buffer.dropped() == u64::MAX,
        }
    }
    pub fn pop_front(&mut self) -> Option<Observation> {
        self.buffer.pop_front()
    }
    fn record(&mut self, event: Observation) {
        let c = &mut self.counts;
        let op = &mut c.operations[event.operation.index()];
        increment(&mut c.offered, &mut c.overflowed);
        increment(&mut op.offered, &mut c.overflowed);
        match event.disposition {
            Disposition::Accepted => {
                increment(&mut c.accepted, &mut c.overflowed);
                increment(&mut op.accepted, &mut c.overflowed);
            }
            Disposition::Replayed => {
                increment(&mut c.replayed, &mut c.overflowed);
                increment(&mut op.replayed, &mut c.overflowed);
            }
            Disposition::Refused(kind) => {
                increment(&mut c.refused, &mut c.overflowed);
                increment(&mut op.refused, &mut c.overflowed);
                increment(&mut c.refusals[kind.index()], &mut c.overflowed);
            }
        }
        if event.disposition == Disposition::Accepted
            && let Some(disposition) = event.harness_disposition
        {
            let value = match disposition {
                DaimonDispositionV1::Completed => &mut c.harness_receipts.completed,
                DaimonDispositionV1::Failed => &mut c.harness_receipts.failed,
                DaimonDispositionV1::Cancelled => &mut c.harness_receipts.cancelled,
                DaimonDispositionV1::TimedOut => &mut c.harness_receipts.timed_out,
                DaimonDispositionV1::Uncertain => &mut c.harness_receipts.uncertain,
            };
            increment(value, &mut c.overflowed);
        }
        // Capture loss never changes or substitutes for the domain result.
        self.buffer.record(event);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EngineUsage {
    pub workflows: usize,
    pub workflow_capacity: usize,
    pub pending_effects: usize,
    pub received: usize,
    pub task_submission_pending: usize,
    pub task_submitted: usize,
    pub harness_pending: usize,
    pub harness_finished: usize,
    pub response_publication_pending: usize,
    pub fenced_uncertain: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
}
impl Engine {
    /// Same deterministic transition with optional concrete, non-reentrant
    /// post-result capture. No subscribers, external callbacks or clock reads.
    pub fn apply_observed(
        &mut self,
        expected: WorkflowRevision,
        command: CommandV1,
        recorder: &mut Recorder,
    ) -> Result<TransitionV1, RefusalV1> {
        let before = self.revision();
        let result = self.apply(expected, command);
        let disposition = match result {
            Ok(_) if self.revision() == before => Disposition::Replayed,
            Ok(_) => Disposition::Accepted,
            Err(error) => Disposition::Refused(RefusalKind::of(error)),
        };
        let harness_disposition = if result.is_ok() {
            match command {
                CommandV1::AcceptDaimonResult { receipt, .. } => Some(receipt.disposition),
                _ => None,
            }
        } else {
            None
        };
        recorder.record(Observation {
            operation: Operation::of(command),
            disposition,
            harness_disposition,
            service_time: None,
        });
        result
    }
    /// Current bounded state, not reconstruction from potentially lost events.
    pub fn usage(&self) -> EngineUsage {
        let mut usage = EngineUsage {
            workflows: self.workflows.len(),
            workflow_capacity: self.limits.max_workflows,
            ..EngineUsage::default()
        };
        for snapshot in self.workflows.values() {
            usage.pending_effects += usize::from(snapshot.pending_effect.is_some());
            let count = match snapshot.phase {
                WorkflowPhaseV1::Received => &mut usage.received,
                WorkflowPhaseV1::TaskSubmissionPending => &mut usage.task_submission_pending,
                WorkflowPhaseV1::TaskSubmitted => &mut usage.task_submitted,
                WorkflowPhaseV1::HarnessPending => &mut usage.harness_pending,
                WorkflowPhaseV1::HarnessFinished => &mut usage.harness_finished,
                WorkflowPhaseV1::ResponsePublicationPending => {
                    &mut usage.response_publication_pending
                }
                WorkflowPhaseV1::FencedUncertain => &mut usage.fenced_uncertain,
                WorkflowPhaseV1::Completed => &mut usage.completed,
                WorkflowPhaseV1::Failed => &mut usage.failed,
                WorkflowPhaseV1::Cancelled => &mut usage.cancelled,
            };
            *count += 1;
        }
        usage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observation_counters_saturate_without_overflowing_or_resetting_on_drain() {
        let mut recorder = Recorder::try_with_capacity(1).unwrap();
        recorder.counts.offered = u64::MAX;
        recorder.record(Observation {
            operation: Operation::AdmitMessage,
            disposition: Disposition::Accepted,
            harness_disposition: None,
            service_time: None,
        });
        assert_eq!(recorder.counts().offered, u64::MAX);
        assert!(recorder.counts().overflowed);
        recorder.pop_front();
        assert!(recorder.counts().overflowed);
    }
}
