use moira_client::{
    AgentReadOnlyClient, AgentServicePort, MutationClient, ReadOnlyClient,
    SagaConfirmedReconciliationClient, SagaMutationClient, SagaOperatorClient, SagaReadOnlyClient,
    SagaServicePort, ServicePort, TaskAgentJourneyReadOnlyClient, TaskAgentJourneyServicePort,
};
use moira_protocol::agent;
use moira_protocol::task_agent_journey as journey;
use moira_protocol::{
    CommandV1, MessageRefV1, MutationV1, OperatorActionIdV1, OperatorDispositionV1,
    OperatorOperationV1, OutcomeV1, ParticipantRefV1, ReplyV1, SagaCommandV1, SagaIdV1,
    SagaMutationV1, SagaOutcomeV1, SagaReplyV1, SagaSourceV1, SagaStatusV1, StatusV1, WorkflowIdV1,
};

#[derive(Default)]
struct Port(Vec<CommandV1>);

impl ServicePort for Port {
    type Error = ();

    fn execute(&mut self, command: CommandV1) -> Result<ReplyV1, Self::Error> {
        self.0.push(command);
        Ok(ReplyV1::Accepted(OutcomeV1::Status(StatusV1 {
            revision: 0,
            needs_reopen: false,
        })))
    }
}

#[derive(Default)]
struct SagaPort(Vec<SagaCommandV1>);

#[derive(Default)]
struct AgentPort(Vec<agent::CommandV1>);

#[derive(Default)]
struct JourneyPort(Vec<journey::CommandV1>);

impl TaskAgentJourneyServicePort for JourneyPort {
    type Error = ();

    fn execute_task_agent_journey(
        &mut self,
        command: journey::CommandV1,
    ) -> Result<journey::ReplyV1, Self::Error> {
        self.0.push(command);
        Ok(journey::ReplyV1::Refused(journey::RefusalV1::Storage))
    }
}

impl AgentServicePort for AgentPort {
    type Error = ();

    fn execute(&mut self, command: agent::CommandV1) -> Result<agent::ReplyV1, Self::Error> {
        self.0.push(command);
        Ok(agent::ReplyV1::Refused(agent::RefusalV1::Unsupported))
    }
}

impl SagaServicePort for SagaPort {
    type Error = ();

    fn execute(&mut self, command: SagaCommandV1) -> Result<SagaReplyV1, Self::Error> {
        self.0.push(command);
        Ok(SagaReplyV1::Accepted(SagaOutcomeV1::Status(SagaStatusV1 {
            revision: 0,
            needs_reopen: false,
        })))
    }
}

#[test]
fn mutation_and_read_only_facades_are_disjoint() {
    let command = MutationV1::AdmitMessage {
        message: MessageRefV1 {
            source: [1; 16],
            room_sequence: 2,
            message_sequence: 3,
        },
        recipient: ParticipantRefV1([4; 16]),
    };
    let mut mutation = MutationClient::new(Port::default());
    mutation.apply(0, command).unwrap();
    assert_eq!(
        mutation.into_port().0,
        vec![CommandV1::Apply {
            expected_revision: 0,
            mutation: command,
        }]
    );

    let workflow = WorkflowIdV1([5; 16]);
    let mut reader = ReadOnlyClient::new(Port::default());
    reader.status().unwrap();
    reader.workflow(workflow).unwrap();
    assert_eq!(
        reader.into_port().0,
        vec![CommandV1::Status, CommandV1::Workflow { workflow }]
    );
}

#[test]
fn agent_result_observation_is_read_only_and_exactly_scoped() {
    let session = agent::SessionIdV1([1; 16]);
    let turn = agent::TurnIdV1([2; 16]);
    let mut reader = AgentReadOnlyClient::new(AgentPort::default());
    reader.result(session, turn).unwrap();
    assert_eq!(
        reader.into_port().0,
        vec![agent::CommandV1::Result { session, turn }]
    );
}

#[test]
fn task_namespace_observation_is_read_only_and_journey_scoped() {
    let journey = journey::JourneyIdV1 {
        owner: journey::OwnerIdV1([3; 16]),
        sequence: 4,
    };
    let mut reader = TaskAgentJourneyReadOnlyClient::new(JourneyPort::default());
    reader.task_attempt_namespace(journey).unwrap();
    assert_eq!(
        reader.into_port().0,
        vec![journey::CommandV1::TaskAttemptNamespace { journey }]
    );
}

#[test]
fn saga_mutation_and_read_only_facades_are_disjoint() {
    let mutation = SagaMutationV1::Admit {
        source: SagaSourceV1([6; 16]),
        now: 1,
        deadline: 2,
        retry_budget: 1,
    };
    let mut writer = SagaMutationClient::new(SagaPort::default());
    writer.apply(3, mutation).unwrap();
    assert_eq!(
        writer.into_port().0,
        vec![SagaCommandV1::Apply {
            expected_revision: 3,
            mutation,
        }]
    );

    let saga = SagaIdV1(7);
    let mut reader = SagaReadOnlyClient::new(SagaPort::default());
    reader.status().unwrap();
    reader.saga(saga).unwrap();
    reader.inspect(Some(saga), 8).unwrap();
    reader.census().unwrap();
    reader.reconciliation(saga).unwrap();
    reader.evidence(saga, None, 8).unwrap();
    let action = OperatorActionIdV1([5; 16]);
    reader.operator_receipt(action).unwrap();
    reader.operator_receipts(Some(action), 8).unwrap();
    assert_eq!(
        reader.into_port().0,
        vec![
            SagaCommandV1::Status,
            SagaCommandV1::Saga { saga },
            SagaCommandV1::Inspect {
                after: Some(saga),
                limit: 8,
            },
            SagaCommandV1::Census,
            SagaCommandV1::Reconciliation { saga },
            SagaCommandV1::Evidence {
                saga,
                after: None,
                limit: 8,
            },
            SagaCommandV1::OperatorReceipt { action },
            SagaCommandV1::OperatorReceipts {
                after: Some(action),
                limit: 8,
            },
        ]
    );
}

#[test]
fn saga_operator_facade_carries_only_operation_not_authority_facts() {
    let operation = OperatorOperationV1 {
        action: OperatorActionIdV1([9; 16]),
        saga: SagaIdV1(3),
        disposition: OperatorDispositionV1::KeepUncertain,
    };
    let mut client = SagaOperatorClient::new(SagaPort::default());
    client.reconcile(4, operation).unwrap();
    assert_eq!(
        client.into_port().0,
        vec![SagaCommandV1::Reconcile {
            expected_revision: 4,
            operation,
        }]
    );
}

#[test]
fn confirmed_reconciliation_facade_carries_only_the_server_challenge() {
    let confirmation = moira_protocol::ConfirmedReconciliationV1 {
        storage_revision: 4,
        saga: SagaIdV1(3),
        disposition: OperatorDispositionV1::KeepUncertain,
        action: OperatorActionIdV1([8; 16]),
        confirmation_digest: [7; 32],
    };
    let mut client = SagaConfirmedReconciliationClient::new(SagaPort::default());
    client.confirm(confirmation).unwrap();
    assert_eq!(
        client.into_port().0,
        vec![SagaCommandV1::ConfirmReconciliation(confirmation)]
    );
}
