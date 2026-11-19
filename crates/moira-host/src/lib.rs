//! Trusted persistence port. It owns no workflow or connector semantics.

use moira_core::EffectId;
use moira_core::agent_participation::{
    BindingV1 as AgentParticipationBindingV1, ObservationV1 as AgentParticipationObservationV1,
    OperationV1 as AgentParticipationOperationV1, ParticipationIdV1 as AgentParticipationIdV1,
    ReceiptV1 as AgentParticipationReceiptV1, RefusalV1 as AgentParticipationRefusalV1,
    SnapshotV1 as AgentParticipationSnapshotV1,
};
use moira_core::deliberation::{
    AuthorityRefV1, CandidateIdV1, DeliberationCheckpointV1, DeliberationIdV1, EnsembleMemberIdV1,
    ReportIdV1,
    registry::{RegistryCommandV1, RegistryReceiptV1, RegistryRefusalV1, RegistryRevisionV1},
};
use moira_core::tool_continuation::{
    CommandV1 as ToolContinuationCommandV1, RefusalV1 as ToolContinuationRefusalV1,
    SnapshotV1 as ToolContinuationSnapshotV1, TransitionV1 as ToolContinuationTransitionV1,
};
use moira_protocol::agent;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolutionSubjectV1 {
    pub deliberation: DeliberationIdV1,
    pub question_digest: [u8; 32],
    pub checkpoint_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProofRequestV1 {
    pub subject: ResolutionSubjectV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProofWitnessV1 {
    pub subject: ResolutionSubjectV1,
    pub validator: [u8; 16],
    pub value: bool,
    pub evidence_digest: [u8; 32],
}

pub trait BooleanValidatorV1 {
    type Error;

    fn validate(&mut self, request: ProofRequestV1) -> Result<ProofWitnessV1, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChoiceRequestV1 {
    pub subject: ResolutionSubjectV1,
    pub frontier: Vec<CandidateIdV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChoiceDecisionV1 {
    pub subject: ResolutionSubjectV1,
    pub authority: AuthorityRefV1,
    pub selected: CandidateIdV1,
}

pub trait ChoiceAuthorityV1 {
    type Error;

    fn choose(&mut self, request: ChoiceRequestV1) -> Result<ChoiceDecisionV1, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionSelectionSubjectV1 {
    pub deliberation: DeliberationIdV1,
    pub checkpoint_digest: [u8; 32],
    pub member: EnsembleMemberIdV1,
    pub report: ReportIdV1,
    pub output_digest: [u8; 32],
    pub daimon_effect: EffectId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionSelectionDecisionV1 {
    pub subject: ActionSelectionSubjectV1,
    pub authority: AuthorityRefV1,
}

/// Accountable approval for one exact retained report/result to create peer
/// effects. This is distinct from choosing a Pareto candidate.
pub trait ActionSelectorV1 {
    type Error;

    fn select(
        &mut self,
        subject: ActionSelectionSubjectV1,
    ) -> Result<ActionSelectionDecisionV1, Self::Error>;
}

/// Provider-neutral interactive-agent gateway. Implementations may project to
/// Daimon, but may not mint authority, attempts, launch profiles, or credentials.
pub trait AgentGateway {
    type Error;

    fn execute_agent(&mut self, command: agent::CommandV1) -> Result<agent::ReplyV1, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentParticipationMutationV1 {
    Admit(AgentParticipationBindingV1),
    Prepare {
        participation: AgentParticipationIdV1,
        expected_revision: u64,
        operation: AgentParticipationOperationV1,
    },
    Observe {
        participation: AgentParticipationIdV1,
        expected_revision: u64,
        key: moira_core::agent_participation::IdempotencyKeyV1,
        observation: AgentParticipationObservationV1,
    },
    FenceUnknown {
        participation: AgentParticipationIdV1,
        expected_revision: u64,
        key: moira_core::agent_participation::IdempotencyKeyV1,
    },
    MarkReattachRequired {
        participation: AgentParticipationIdV1,
        expected_revision: u64,
        session_revision: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentParticipationApplyReceiptV1 {
    pub storage_revision: u64,
    pub receipt: AgentParticipationReceiptV1,
    pub participation: AgentParticipationSnapshotV1,
    pub replayed: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum AgentParticipationApplyErrorV1<E> {
    Domain(AgentParticipationRefusalV1),
    Conflict {
        expected: Option<u64>,
        actual: Option<u64>,
    },
    Backend(E),
}

pub trait AgentParticipationStore {
    type Error;
    fn storage_revision(&self) -> Option<u64>;
    fn needs_reopen(&self) -> bool;
    fn participation(&self, id: AgentParticipationIdV1) -> Option<AgentParticipationSnapshotV1>;
    fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        mutation: AgentParticipationMutationV1,
    ) -> Result<AgentParticipationApplyReceiptV1, AgentParticipationApplyErrorV1<Self::Error>>;
}
use moira_core::model_saga::{
    CommandV1 as ModelSagaCommandV1, RefusalV1 as ModelSagaRefusalV1,
    SagaSnapshotV1 as ModelSagaSnapshotV1, TransitionV1 as ModelSagaTransitionV1,
};
use moira_core::saga::{
    EffectEvidencePageV1, OperatorActionIdV1, OperatorReceiptPageV1, OperatorReceiptSummaryV1,
    SagaCensusV1, SagaCommandV1, SagaEffectIdV1, SagaIdV1, SagaPageV1, SagaReconciliationViewV1,
    SagaRefusalV1, SagaSnapshotV1, SagaTransitionV1,
};
use moira_core::{
    CommandV1, RefusalV1, TransitionV1, WorkflowId, WorkflowRevision, WorkflowSnapshotV1,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplyReceiptV1 {
    pub storage_revision: u64,
    pub transition: TransitionV1,
    pub replayed: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ApplyErrorV1<E> {
    Domain(RefusalV1),
    Backend(E),
}

/// One serialized durable coordinator owner.
pub trait CoordinatorStore {
    type Error;

    fn revision(&self) -> WorkflowRevision;
    fn needs_reopen(&self) -> bool;
    fn workflow(&self, workflow: WorkflowId) -> Option<WorkflowSnapshotV1>;
    fn apply(
        &mut self,
        expected: WorkflowRevision,
        command: CommandV1,
    ) -> Result<ApplyReceiptV1, ApplyErrorV1<Self::Error>>;
}

/// Optional read port over the same retained workflow state as the coordinator.
pub trait AttemptReceiptStore: CoordinatorStore {
    fn workflow_for_attempt(
        &self,
        task: moira_core::ChronosTaskRefV1,
        attempt: u32,
    ) -> Result<Option<WorkflowSnapshotV1>, moira_core::AttemptReceiptConflict>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SagaApplyReceiptV1 {
    pub storage_revision: u64,
    pub transition: SagaTransitionV1,
    pub replayed: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum SagaApplyErrorV1<E> {
    Domain(SagaRefusalV1),
    Backend(E),
}

/// Separate durable owner for the closed saga profile. It does not dispatch
/// effects, schedule timers or decide operator policy.
pub trait SagaStore {
    type Error;

    fn revision(&self) -> u64;
    fn needs_reopen(&self) -> bool;
    fn saga(&self, saga: SagaIdV1) -> Option<SagaSnapshotV1>;
    fn inspect_page(
        &self,
        after: Option<SagaIdV1>,
        limit: usize,
    ) -> Result<SagaPageV1, SagaRefusalV1>;
    fn census(&self) -> SagaCensusV1;
    fn reconciliation_view(&self, saga: SagaIdV1) -> Option<SagaReconciliationViewV1>;
    fn evidence_page(
        &self,
        saga: SagaIdV1,
        after: Option<SagaEffectIdV1>,
        limit: usize,
    ) -> Result<EffectEvidencePageV1, SagaRefusalV1>;
    fn operator_receipt(&self, action: OperatorActionIdV1) -> Option<OperatorReceiptSummaryV1>;
    fn operator_receipt_page(
        &self,
        after: Option<OperatorActionIdV1>,
        limit: usize,
    ) -> Result<OperatorReceiptPageV1, SagaRefusalV1>;
    fn apply(
        &mut self,
        expected_revision: u64,
        command: SagaCommandV1,
    ) -> Result<SagaApplyReceiptV1, SagaApplyErrorV1<Self::Error>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSagaApplyReceiptV1 {
    pub storage_revision: u64,
    pub transition: ModelSagaTransitionV1,
    pub replayed: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ModelSagaApplyErrorV1<E> {
    Domain(ModelSagaRefusalV1),
    Conflict {
        expected: Option<u64>,
        actual: Option<u64>,
    },
    Backend(E),
}

/// Separate durable owner for model proposals. It coordinates effects but
/// never invokes Daimon, Agora or Vellum itself.
pub trait ModelSagaStore {
    type Error;

    fn storage_revision(&self) -> Option<u64>;
    fn needs_reopen(&self) -> bool;
    fn saga(&self, workflow: moira_core::WorkflowId) -> Option<ModelSagaSnapshotV1>;
    fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        command: ModelSagaCommandV1,
    ) -> Result<ModelSagaApplyReceiptV1, ModelSagaApplyErrorV1<Self::Error>>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum DeliberationApplyErrorV1<E> {
    Domain(RegistryRefusalV1),
    Backend(E),
}

/// Separate durable owner for methodical deliberations. Implementations must
/// preserve the registry's optimistic revision and matching-replay semantics.
pub trait DeliberationStore {
    type Error;

    fn revision(&self) -> RegistryRevisionV1;
    fn needs_reopen(&self) -> bool;
    fn deliberation(&self, id: DeliberationIdV1) -> Option<DeliberationCheckpointV1>;
    fn review(
        &self,
        id: DeliberationIdV1,
    ) -> Option<moira_core::deliberation::review::ReviewExchangeCheckpointV1>;
    fn apply(
        &mut self,
        expected_revision: RegistryRevisionV1,
        command: RegistryCommandV1,
    ) -> Result<RegistryReceiptV1, DeliberationApplyErrorV1<Self::Error>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolContinuationApplyReceiptV1 {
    pub storage_revision: u64,
    pub transition: ToolContinuationTransitionV1,
    pub replayed: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ToolContinuationApplyErrorV1<E> {
    Domain(ToolContinuationRefusalV1),
    Conflict {
        expected: Option<u64>,
        actual: Option<u64>,
    },
    Backend(E),
}

/// Separate durable owner for structured tool continuations. Implementations
/// coordinate retained facts only and never authorize or dispatch a tool.
pub trait ToolContinuationStore {
    type Error;

    fn storage_revision(&self) -> Option<u64>;
    fn needs_reopen(&self) -> bool;
    fn saga(&self, workflow: moira_core::WorkflowId) -> Option<ToolContinuationSnapshotV1>;
    fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        command: ToolContinuationCommandV1,
    ) -> Result<ToolContinuationApplyReceiptV1, ToolContinuationApplyErrorV1<Self::Error>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolInvocationV1 {
    pub workflow: moira_core::WorkflowId,
    pub effect: moira_core::EffectId,
    pub execution: moira_core::tool_continuation::ToolExecutionIdV1,
    pub gate: moira_core::tool_continuation::ToolGateV1,
    pub call: moira_core::tool_continuation::ToolCallV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExecutionOutcomeV1 {
    Succeeded {
        evidence_digest: [u8; 32],
        output: Vec<u8>,
    },
    Failed {
        evidence_digest: [u8; 32],
    },
    OutcomeUnknown,
}

/// One attempt to invoke an already durably marked in-flight tool effect.
/// Callers must never infer retry safety from this interface.
pub trait ToolExecutorV1 {
    type Error;

    fn execute(
        &mut self,
        invocation: &ToolInvocationV1,
    ) -> Result<ToolExecutionOutcomeV1, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolOutputRecordV1 {
    pub execution: moira_core::tool_continuation::ToolExecutionIdV1,
    pub capability: moira_core::tool_continuation::ToolCapabilityIdV1,
    pub evidence_digest: [u8; 32],
    pub output_digest: [u8; 32],
    pub output_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolOutputCommitV1 {
    pub record: ToolOutputRecordV1,
    pub replayed: bool,
}

/// Separately durable content-addressed tool output. Success may be admitted
/// into the continuation only after this port acknowledges the exact bytes.
pub trait ToolOutputStoreV1 {
    type Error;

    fn put(
        &mut self,
        execution: moira_core::tool_continuation::ToolExecutionIdV1,
        capability: moira_core::tool_continuation::ToolCapabilityIdV1,
        evidence_digest: [u8; 32],
        output: &[u8],
    ) -> Result<ToolOutputCommitV1, Self::Error>;

    fn get(&mut self, output_digest: [u8; 32]) -> Result<Option<Vec<u8>>, Self::Error>;

    fn find(
        &mut self,
        execution: moira_core::tool_continuation::ToolExecutionIdV1,
    ) -> Result<Option<(ToolOutputRecordV1, Vec<u8>)>, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentPromptRecordV1 {
    pub prompt_digest: [u8; 32],
    pub prompt_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentPromptCommitV1 {
    pub record: AgentPromptRecordV1,
    pub replayed: bool,
}

/// Separately durable content-addressed agent prompt bytes. A prompt operation
/// may refer to a digest only after this port acknowledges the exact bytes.
pub trait AgentPromptStoreV1 {
    type Error;

    fn put(&mut self, prompt: &[u8]) -> Result<AgentPromptCommitV1, Self::Error>;

    fn get(&mut self, prompt_digest: [u8; 32]) -> Result<Option<Vec<u8>>, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentOutputRecordV1 {
    pub participation: [u8; 16],
    pub turn: [u8; 16],
    pub last_cursor: u64,
    pub output_digest: [u8; 32],
    pub output_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentOutputCommitV1 {
    pub record: AgentOutputRecordV1,
    pub replayed: bool,
}

/// Durable cursor-ordered assistant output for one governed turn. Callers pass
/// only already validated contiguous event pages. Exact last-page replay is
/// idempotent; gaps, forks, and altered bytes must refuse.
pub trait AgentOutputStoreV1 {
    type Error;

    #[allow(clippy::too_many_arguments)]
    fn append(
        &mut self,
        participation: [u8; 16],
        turn: [u8; 16],
        after: Option<u64>,
        through: u64,
        page_digest: [u8; 32],
        assistant_bytes: &[u8],
    ) -> Result<AgentOutputCommitV1, Self::Error>;

    fn get(
        &mut self,
        participation: [u8; 16],
        turn: [u8; 16],
    ) -> Result<Option<(AgentOutputRecordV1, Vec<u8>)>, Self::Error>;
}
