//! Closed transport-neutral contract for the native Moira service.
//!
//! DTOs are deliberately independent of `moira-core` representation. Fixed
//! identities are correlation only, remain content-redacted in diagnostics, and
//! confer no authority.

pub mod agent;
pub mod agent_participation;
pub mod attempt_lookup;
pub mod deliberation;
pub mod model_saga;
pub mod task_agent_journey;
pub mod tool_continuation;

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub [u8; 16]);

        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(<opaque>)"))
            }
        }
    };
}

opaque_id!(WorkflowIdV1);
opaque_id!(EffectIdV1);
opaque_id!(ParticipantRefV1);
opaque_id!(SagaSourceV1);
opaque_id!(OperatorActionIdV1);

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MessageRefV1 {
    pub source: [u8; 16],
    pub room_sequence: u64,
    pub message_sequence: u64,
}

impl std::fmt::Debug for MessageRefV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MessageRefV1(<opaque>)")
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct TaskRefV1 {
    pub external_task_key: [u8; 16],
    pub incarnation: u64,
}

impl std::fmt::Debug for TaskRefV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TaskRefV1(<opaque>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionReceiptV1 {
    pub effect: EffectIdV1,
    pub task: TaskRefV1,
    pub chronos_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptReceiptV1 {
    pub task: TaskRefV1,
    pub attempt_number: u32,
    pub chronos_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispositionV1 {
    Completed,
    Failed,
    Cancelled,
    TimedOut,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResultReceiptV1 {
    pub effect: EffectIdV1,
    pub execution_id: [u8; 16],
    pub task: TaskRefV1,
    pub attempt_number: u32,
    pub disposition: DispositionV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicationReceiptV1 {
    pub effect: EffectIdV1,
    pub message: MessageRefV1,
    pub agora_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationV1 {
    AdmitMessage {
        message: MessageRefV1,
        recipient: ParticipantRefV1,
    },
    AcceptChronosAdmission {
        workflow: WorkflowIdV1,
        receipt: AdmissionReceiptV1,
    },
    AcceptChronosAttempt {
        workflow: WorkflowIdV1,
        receipt: AttemptReceiptV1,
    },
    AcceptDaimonResult {
        workflow: WorkflowIdV1,
        receipt: ResultReceiptV1,
    },
    PrepareAgoraResponse {
        workflow: WorkflowIdV1,
    },
    AcceptAgoraPublication {
        workflow: WorkflowIdV1,
        receipt: PublicationReceiptV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandV1 {
    Apply {
        expected_revision: u64,
        mutation: MutationV1,
    },
    Status,
    Workflow {
        workflow: WorkflowIdV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseV1 {
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
pub struct WorkflowViewV1 {
    pub workflow: WorkflowIdV1,
    pub revision: u64,
    pub phase: PhaseV1,
    pub cause: MessageRefV1,
    pub recipient: ParticipantRefV1,
    pub pending_effect: Option<EffectIdV1>,
    pub task: Option<TaskRefV1>,
    pub attempt: Option<AttemptReceiptV1>,
    pub result_disposition: Option<DispositionV1>,
    pub publication: Option<MessageRefV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionV1 {
    ChronosSubmissionPrepared {
        workflow: WorkflowIdV1,
        effect: EffectIdV1,
    },
    ChronosSubmissionAccepted {
        workflow: WorkflowIdV1,
        task: TaskRefV1,
    },
    DaimonExecutionPrepared {
        workflow: WorkflowIdV1,
        effect: EffectIdV1,
        task: TaskRefV1,
        attempt_number: u32,
    },
    DaimonResultAccepted {
        workflow: WorkflowIdV1,
        disposition: DispositionV1,
    },
    AgoraResponsePrepared {
        workflow: WorkflowIdV1,
        effect: EffectIdV1,
    },
    AgoraPublicationAccepted {
        workflow: WorkflowIdV1,
        message: MessageRefV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
    NotAuthorized,
    RevisionConflict { expected: u64, actual: u64 },
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusV1 {
    pub revision: u64,
    pub needs_reopen: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutcomeV1 {
    Applied {
        storage_revision: u64,
        transition: TransitionV1,
        replayed: bool,
    },
    Status(StatusV1),
    Workflow(Option<WorkflowViewV1>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyV1 {
    Accepted(OutcomeV1),
    Refused(RefusalV1),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SagaIdV1(pub u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SagaEffectIdV1(pub u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TimerIdV1(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SagaEffectKindV1 {
    Forward,
    Terminal,
    Compensation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SagaEffectOutcomeV1 {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SagaMutationV1 {
    Admit {
        source: SagaSourceV1,
        now: u64,
        deadline: u64,
        retry_budget: u8,
    },
    AcceptEffect {
        saga: SagaIdV1,
        effect: SagaEffectIdV1,
        outcome: SagaEffectOutcomeV1,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorDispositionV1 {
    AdmitEvidence {
        effect: SagaEffectIdV1,
        outcome: SagaEffectOutcomeV1,
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
pub struct OperatorOperationV1 {
    pub action: OperatorActionIdV1,
    pub saga: SagaIdV1,
    pub disposition: OperatorDispositionV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SagaCommandV1 {
    Apply {
        expected_revision: u64,
        mutation: SagaMutationV1,
    },
    Status,
    Saga {
        saga: SagaIdV1,
    },
    Inspect {
        after: Option<SagaIdV1>,
        limit: u64,
    },
    Census,
    Reconciliation {
        saga: SagaIdV1,
    },
    Evidence {
        saga: SagaIdV1,
        after: Option<SagaEffectIdV1>,
        limit: u64,
    },
    OperatorReceipt {
        action: OperatorActionIdV1,
    },
    OperatorReceipts {
        after: Option<OperatorActionIdV1>,
        limit: u64,
    },
    Reconcile {
        expected_revision: u64,
        operation: OperatorOperationV1,
    },
    ReconciliationConfirmation {
        proposal: ReconciliationProposalV1,
    },
    ConfirmReconciliation(ConfirmedReconciliationV1),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconciliationProposalV1 {
    pub saga: SagaIdV1,
    pub disposition: OperatorDispositionV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfirmedReconciliationV1 {
    pub storage_revision: u64,
    pub saga: SagaIdV1,
    pub disposition: OperatorDispositionV1,
    pub action: OperatorActionIdV1,
    pub confirmation_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconciliationConfirmationV1 {
    pub storage_revision: u64,
    pub saga: SagaIdV1,
    pub disposition: OperatorDispositionV1,
    pub action: OperatorActionIdV1,
    pub confirmation_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SagaViewV1 {
    pub saga: SagaIdV1,
    pub phase: SagaPhaseV1,
    pub deadline: u64,
    pub timer: TimerIdV1,
    pub current_effect: Option<SagaEffectIdV1>,
    pub current_kind: Option<SagaEffectKindV1>,
    pub retries_used: u8,
    pub retry_budget: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SagaPageV1 {
    pub entries: Vec<SagaViewV1>,
    pub next_after: Option<SagaIdV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
        kind: SagaEffectKindV1,
        observed_at: u64,
    },
    Deadline {
        timer: TimerIdV1,
        observed_at: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SagaReconciliationViewV1 {
    pub saga: SagaViewV1,
    pub last_confirmed_revision: u64,
    pub retained_evidence: u64,
    pub fence: Option<FenceCauseV1>,
    pub automatic_progress: AutomaticProgressV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectEvidenceV1 {
    pub effect: SagaEffectIdV1,
    pub outcome: SagaEffectOutcomeV1,
    pub observed_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectEvidencePageV1 {
    pub entries: Vec<EffectEvidenceV1>,
    pub next_after: Option<SagaEffectIdV1>,
}

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
pub enum SagaTransitionV1 {
    Admitted {
        saga: SagaIdV1,
        effect: SagaEffectIdV1,
        timer: TimerIdV1,
    },
    EffectPrepared {
        saga: SagaIdV1,
        effect: SagaEffectIdV1,
        kind: SagaEffectKindV1,
        retry: u8,
    },
    EffectAccepted {
        saga: SagaIdV1,
        effect: SagaEffectIdV1,
        outcome: SagaEffectOutcomeV1,
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
    NotAuthorized,
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
pub struct SagaStatusV1 {
    pub revision: u64,
    pub needs_reopen: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SagaOutcomeV1 {
    Applied {
        storage_revision: u64,
        transition: SagaTransitionV1,
        replayed: bool,
    },
    Status(SagaStatusV1),
    Saga(Option<SagaViewV1>),
    Page(SagaPageV1),
    Census(SagaCensusV1),
    Reconciliation(Option<SagaReconciliationViewV1>),
    Evidence(EffectEvidencePageV1),
    OperatorReceipt(Option<OperatorReceiptSummaryV1>),
    OperatorReceipts(OperatorReceiptPageV1),
    ReconciliationConfirmation(Option<ReconciliationConfirmationV1>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SagaReplyV1 {
    Accepted(SagaOutcomeV1),
    Refused(SagaRefusalV1),
}

/// Builds a presentation challenge for an exact operator proposal at the
/// current retained saga revision. The digest is a stale-choice fence, not an
/// authority credential.
pub fn reconciliation_confirmation(
    storage_revision: u64,
    view: SagaReconciliationViewV1,
    proposal: ReconciliationProposalV1,
) -> Option<ReconciliationConfirmationV1> {
    use sha2::{Digest, Sha256};

    if proposal.saga != view.saga.saga || storage_revision == u64::MAX {
        return None;
    }
    let mut hash = Sha256::new();
    hash.update(b"MOIRASRC1");
    hash.update(storage_revision.to_be_bytes());
    hash.update(proposal.saga.0.to_be_bytes());
    hash.update(view.last_confirmed_revision.to_be_bytes());
    hash.update(view.retained_evidence.to_be_bytes());
    hash.update([saga_phase_tag(view.saga.phase)]);
    hash.update(view.saga.deadline.to_be_bytes());
    hash.update(view.saga.timer.0.to_be_bytes());
    hash.update(
        view.saga
            .current_effect
            .map_or(u64::MAX, |value| value.0)
            .to_be_bytes(),
    );
    hash.update([view.saga.current_kind.map_or(u8::MAX, saga_effect_kind_tag)]);
    hash.update([view.saga.retries_used, view.saga.retry_budget]);
    hash_operator_disposition(&mut hash, proposal.disposition);
    let confirmation_digest: [u8; 32] = hash.finalize().into();
    let mut action = Sha256::new();
    action.update(b"MOIRASRA1");
    action.update(confirmation_digest);
    let action: [u8; 32] = action.finalize().into();
    let action: [u8; 16] = action[..16].try_into().ok()?;
    if action == [0; 16] {
        return None;
    }
    Some(ReconciliationConfirmationV1 {
        storage_revision,
        saga: proposal.saga,
        disposition: proposal.disposition,
        action: OperatorActionIdV1(action),
        confirmation_digest,
    })
}

/// Resolves an exact current confirmation into the replay-stable operation
/// that may be presented for command-scoped authorization.
pub fn confirmed_reconciliation_operation(
    confirmation: ReconciliationConfirmationV1,
    selected: ConfirmedReconciliationV1,
) -> Option<OperatorOperationV1> {
    (selected.saga == confirmation.saga
        && selected.storage_revision == confirmation.storage_revision
        && selected.disposition == confirmation.disposition
        && selected.action == confirmation.action
        && selected.confirmation_digest == confirmation.confirmation_digest)
        .then_some(OperatorOperationV1 {
            action: confirmation.action,
            saga: confirmation.saga,
            disposition: confirmation.disposition,
        })
}

fn hash_operator_disposition(hash: &mut sha2::Sha256, value: OperatorDispositionV1) {
    use sha2::Digest;
    match value {
        OperatorDispositionV1::AdmitEvidence {
            effect,
            outcome,
            observed_at,
        } => {
            hash.update([0, saga_effect_outcome_tag(outcome)]);
            hash.update(effect.0.to_be_bytes());
            hash.update(observed_at.to_be_bytes());
        }
        OperatorDispositionV1::KeepUncertain => hash.update([1]),
        OperatorDispositionV1::Retry { failed_effect, now } => {
            hash.update([2]);
            hash.update(failed_effect.0.to_be_bytes());
            hash.update(now.to_be_bytes());
        }
        OperatorDispositionV1::Compensate { now } => {
            hash.update([3]);
            hash.update(now.to_be_bytes());
        }
        OperatorDispositionV1::Cancel { now } => {
            hash.update([4]);
            hash.update(now.to_be_bytes());
        }
        OperatorDispositionV1::FailTerminal => hash.update([5]),
    }
}

const fn saga_phase_tag(value: SagaPhaseV1) -> u8 {
    match value {
        SagaPhaseV1::ForwardPending => 0,
        SagaPhaseV1::ForwardSucceeded => 1,
        SagaPhaseV1::ForwardFailed => 2,
        SagaPhaseV1::TerminalPending => 3,
        SagaPhaseV1::TerminalFailed => 4,
        SagaPhaseV1::CompensationPending => 5,
        SagaPhaseV1::CompensationFailed => 6,
        SagaPhaseV1::FencedUncertain => 7,
        SagaPhaseV1::Completed => 8,
        SagaPhaseV1::Failed => 9,
        SagaPhaseV1::Cancelled => 10,
    }
}

const fn saga_effect_kind_tag(value: SagaEffectKindV1) -> u8 {
    match value {
        SagaEffectKindV1::Forward => 0,
        SagaEffectKindV1::Terminal => 1,
        SagaEffectKindV1::Compensation => 2,
    }
}

const fn saga_effect_outcome_tag(value: SagaEffectOutcomeV1) -> u8 {
    match value {
        SagaEffectOutcomeV1::Succeeded => 0,
        SagaEffectOutcomeV1::Failed => 1,
        SagaEffectOutcomeV1::Uncertain => 2,
    }
}
