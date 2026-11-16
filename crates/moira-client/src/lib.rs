//! Capability-shaped client facades for the native Moira contract.

use moira_protocol::OperatorOperationV1;
use moira_protocol::agent;
use moira_protocol::agent_participation as participation;
use moira_protocol::deliberation;
use moira_protocol::model_saga as model;
use moira_protocol::tool_continuation as tools;
use moira_protocol::{CommandV1, MutationV1, ReplyV1, WorkflowIdV1};
use moira_protocol::{SagaCommandV1, SagaIdV1, SagaMutationV1, SagaReplyV1};

pub trait AgentServicePort {
    type Error;
    fn execute(&mut self, command: agent::CommandV1) -> Result<agent::ReplyV1, Self::Error>;
}

pub trait AgentParticipationServicePort {
    type Error;
    fn execute_participation(
        &mut self,
        command: participation::CommandV1,
    ) -> Result<participation::ReplyV1, Self::Error>;
}

pub struct AgentParticipationMutationClient<P>(P);
impl<P: AgentParticipationServicePort> AgentParticipationMutationClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }
    pub fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        mutation: participation::MutationV1,
    ) -> Result<participation::ReplyV1, P::Error> {
        self.0
            .execute_participation(participation::CommandV1::Apply {
                expected_storage_revision,
                mutation: Box::new(mutation),
            })
    }
    pub fn into_port(self) -> P {
        self.0
    }
}
pub struct AgentParticipationReadOnlyClient<P>(P);
impl<P: AgentParticipationServicePort> AgentParticipationReadOnlyClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }
    pub fn status(&mut self) -> Result<participation::ReplyV1, P::Error> {
        self.0
            .execute_participation(participation::CommandV1::Status)
    }
    pub fn participation(
        &mut self,
        id: participation::ParticipationIdV1,
    ) -> Result<participation::ReplyV1, P::Error> {
        self.0
            .execute_participation(participation::CommandV1::Participation { participation: id })
    }
    pub fn permission_confirmation(
        &mut self,
        id: participation::ParticipationIdV1,
    ) -> Result<participation::ReplyV1, P::Error> {
        self.0
            .execute_participation(participation::CommandV1::PermissionConfirmation {
                participation: id,
            })
    }
    pub fn cancel_confirmation(
        &mut self,
        id: participation::ParticipationIdV1,
    ) -> Result<participation::ReplyV1, P::Error> {
        self.0
            .execute_participation(participation::CommandV1::CancelConfirmation {
                participation: id,
            })
    }
    pub fn into_port(self) -> P {
        self.0
    }
}

pub struct AgentParticipationCancellationClient<P>(P);
impl<P: AgentParticipationServicePort> AgentParticipationCancellationClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }
    pub fn confirm(
        &mut self,
        confirmation: participation::ConfirmedCancelV1,
    ) -> Result<participation::ReplyV1, P::Error> {
        self.0
            .execute_participation(participation::CommandV1::ConfirmCancel(confirmation))
    }
    pub fn into_port(self) -> P {
        self.0
    }
}

pub struct AgentParticipationPermissionClient<P>(P);
impl<P: AgentParticipationServicePort> AgentParticipationPermissionClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }
    pub fn select(
        &mut self,
        selection: participation::PermissionSelectionV1,
    ) -> Result<participation::ReplyV1, P::Error> {
        self.0
            .execute_participation(participation::CommandV1::SelectPermission(selection))
    }
    pub fn confirm(
        &mut self,
        selection: participation::ConfirmedPermissionSelectionV1,
    ) -> Result<participation::ReplyV1, P::Error> {
        self.0
            .execute_participation(participation::CommandV1::ConfirmPermission(selection))
    }
    pub fn into_port(self) -> P {
        self.0
    }
}

pub struct AgentMutationClient<P>(P);

impl<P: AgentServicePort> AgentMutationClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn open(
        &mut self,
        request: agent::OpenRequestV1,
        now: u64,
    ) -> Result<agent::ReplyV1, P::Error> {
        self.0.execute(agent::CommandV1::Open { request, now })
    }

    pub fn prompt(
        &mut self,
        request: agent::PromptRequestV1,
        now: u64,
    ) -> Result<agent::ReplyV1, P::Error> {
        self.0.execute(agent::CommandV1::Prompt { request, now })
    }

    pub fn decide_permission(
        &mut self,
        session: agent::SessionIdV1,
        expected_revision: u64,
        execution: agent::ExecutionV1,
        decision: agent::PermissionDecisionV1,
        now: u64,
    ) -> Result<agent::ReplyV1, P::Error> {
        self.0.execute(agent::CommandV1::DecidePermission {
            session,
            expected_revision,
            execution,
            decision,
            now,
        })
    }

    pub fn advance(
        &mut self,
        session: agent::SessionIdV1,
        turn: agent::TurnIdV1,
        expected_revision: u64,
        idempotency_key: agent::IdempotencyKeyV1,
        now: u64,
    ) -> Result<agent::ReplyV1, P::Error> {
        self.0.execute(agent::CommandV1::Advance {
            session,
            turn,
            expected_revision,
            idempotency_key,
            now,
        })
    }

    pub fn cancel(
        &mut self,
        session: agent::SessionIdV1,
        turn: agent::TurnIdV1,
        expected_revision: u64,
        idempotency_key: agent::IdempotencyKeyV1,
        now: u64,
    ) -> Result<agent::ReplyV1, P::Error> {
        self.0.execute(agent::CommandV1::Cancel {
            session,
            turn,
            expected_revision,
            idempotency_key,
            now,
        })
    }

    pub fn close(
        &mut self,
        session: agent::SessionIdV1,
        expected_revision: u64,
        idempotency_key: agent::IdempotencyKeyV1,
        now: u64,
    ) -> Result<agent::ReplyV1, P::Error> {
        self.0.execute(agent::CommandV1::Close {
            session,
            expected_revision,
            idempotency_key,
            now,
        })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub struct AgentReadOnlyClient<P>(P);

impl<P: AgentServicePort> AgentReadOnlyClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn inspect(&mut self, session: agent::SessionIdV1) -> Result<agent::ReplyV1, P::Error> {
        self.0.execute(agent::CommandV1::Inspect { session })
    }

    pub fn events(
        &mut self,
        session: agent::SessionIdV1,
        after: Option<u64>,
        limit: u64,
    ) -> Result<agent::ReplyV1, P::Error> {
        self.0.execute(agent::CommandV1::Events {
            session,
            after,
            limit,
        })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub trait ServicePort {
    type Error;
    fn execute(&mut self, command: CommandV1) -> Result<ReplyV1, Self::Error>;
}

pub struct MutationClient<P>(P);

impl<P: ServicePort> MutationClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn apply(
        &mut self,
        expected_revision: u64,
        mutation: MutationV1,
    ) -> Result<ReplyV1, P::Error> {
        self.0.execute(CommandV1::Apply {
            expected_revision,
            mutation,
        })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub struct ReadOnlyClient<P>(P);

impl<P: ServicePort> ReadOnlyClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn status(&mut self) -> Result<ReplyV1, P::Error> {
        self.0.execute(CommandV1::Status)
    }

    pub fn workflow(&mut self, workflow: WorkflowIdV1) -> Result<ReplyV1, P::Error> {
        self.0.execute(CommandV1::Workflow { workflow })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub trait SagaServicePort {
    type Error;
    fn execute(&mut self, command: SagaCommandV1) -> Result<SagaReplyV1, Self::Error>;
}

pub struct SagaMutationClient<P>(P);

impl<P: SagaServicePort> SagaMutationClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn apply(
        &mut self,
        expected_revision: u64,
        mutation: SagaMutationV1,
    ) -> Result<SagaReplyV1, P::Error> {
        self.0.execute(SagaCommandV1::Apply {
            expected_revision,
            mutation,
        })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub struct SagaReadOnlyClient<P>(P);

impl<P: SagaServicePort> SagaReadOnlyClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn status(&mut self) -> Result<SagaReplyV1, P::Error> {
        self.0.execute(SagaCommandV1::Status)
    }

    pub fn saga(&mut self, saga: SagaIdV1) -> Result<SagaReplyV1, P::Error> {
        self.0.execute(SagaCommandV1::Saga { saga })
    }

    pub fn inspect(
        &mut self,
        after: Option<SagaIdV1>,
        limit: u64,
    ) -> Result<SagaReplyV1, P::Error> {
        self.0.execute(SagaCommandV1::Inspect { after, limit })
    }

    pub fn census(&mut self) -> Result<SagaReplyV1, P::Error> {
        self.0.execute(SagaCommandV1::Census)
    }

    pub fn reconciliation(&mut self, saga: SagaIdV1) -> Result<SagaReplyV1, P::Error> {
        self.0.execute(SagaCommandV1::Reconciliation { saga })
    }

    pub fn evidence(
        &mut self,
        saga: SagaIdV1,
        after: Option<moira_protocol::SagaEffectIdV1>,
        limit: u64,
    ) -> Result<SagaReplyV1, P::Error> {
        self.0
            .execute(SagaCommandV1::Evidence { saga, after, limit })
    }

    pub fn operator_receipt(
        &mut self,
        action: moira_protocol::OperatorActionIdV1,
    ) -> Result<SagaReplyV1, P::Error> {
        self.0.execute(SagaCommandV1::OperatorReceipt { action })
    }

    pub fn operator_receipts(
        &mut self,
        after: Option<moira_protocol::OperatorActionIdV1>,
        limit: u64,
    ) -> Result<SagaReplyV1, P::Error> {
        self.0
            .execute(SagaCommandV1::OperatorReceipts { after, limit })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub struct SagaOperatorClient<P>(P);

impl<P: SagaServicePort> SagaOperatorClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn reconcile(
        &mut self,
        expected_revision: u64,
        operation: OperatorOperationV1,
    ) -> Result<SagaReplyV1, P::Error> {
        self.0.execute(SagaCommandV1::Reconcile {
            expected_revision,
            operation,
        })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub trait ModelSagaServicePort {
    type Error;
    fn execute(&mut self, command: model::CommandV1) -> Result<model::ReplyV1, Self::Error>;
}

pub struct ModelSagaMutationClient<P>(P);

impl<P: ModelSagaServicePort> ModelSagaMutationClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        mutation: model::MutationV1,
    ) -> Result<model::ReplyV1, P::Error> {
        self.0.execute(model::CommandV1::Apply {
            expected_storage_revision,
            mutation,
        })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub struct ModelSagaReadOnlyClient<P>(P);

impl<P: ModelSagaServicePort> ModelSagaReadOnlyClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn status(&mut self) -> Result<model::ReplyV1, P::Error> {
        self.0.execute(model::CommandV1::Status)
    }

    pub fn saga(&mut self, workflow: WorkflowIdV1) -> Result<model::ReplyV1, P::Error> {
        self.0.execute(model::CommandV1::Saga { workflow })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub trait DeliberationServicePort {
    type Error;
    fn execute(
        &mut self,
        command: deliberation::CommandV1,
    ) -> Result<deliberation::ReplyV1, Self::Error>;
}

pub struct DeliberationMutationClient<P>(P);

impl<P: DeliberationServicePort> DeliberationMutationClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn apply(
        &mut self,
        expected_revision: u64,
        mutation: deliberation::MutationV1,
    ) -> Result<deliberation::ReplyV1, P::Error> {
        self.0.execute(deliberation::CommandV1::Apply {
            expected_revision,
            mutation,
        })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub struct DeliberationReadOnlyClient<P>(P);

impl<P: DeliberationServicePort> DeliberationReadOnlyClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn status(&mut self) -> Result<deliberation::ReplyV1, P::Error> {
        self.0.execute(deliberation::CommandV1::Status)
    }

    pub fn deliberation(
        &mut self,
        id: deliberation::DeliberationIdV1,
    ) -> Result<deliberation::ReplyV1, P::Error> {
        self.0
            .execute(deliberation::CommandV1::Deliberation { deliberation: id })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub trait ToolContinuationServicePort {
    type Error;
    fn execute(&mut self, command: tools::CommandV1) -> Result<tools::ReplyV1, Self::Error>;
}

pub struct ToolContinuationMutationClient<P>(P);

impl<P: ToolContinuationServicePort> ToolContinuationMutationClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        mutation: tools::MutationV1,
    ) -> Result<tools::ReplyV1, P::Error> {
        self.0.execute(tools::CommandV1::Apply {
            expected_storage_revision,
            mutation,
        })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}

pub struct ToolContinuationReadOnlyClient<P>(P);

impl<P: ToolContinuationServicePort> ToolContinuationReadOnlyClient<P> {
    pub const fn new(port: P) -> Self {
        Self(port)
    }

    pub fn status(&mut self) -> Result<tools::ReplyV1, P::Error> {
        self.0.execute(tools::CommandV1::Status)
    }

    pub fn saga(&mut self, workflow: WorkflowIdV1) -> Result<tools::ReplyV1, P::Error> {
        self.0.execute(tools::CommandV1::Saga { workflow })
    }

    pub fn into_port(self) -> P {
        self.0
    }
}
