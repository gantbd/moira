use moira_client::{
    ToolContinuationMutationClient, ToolContinuationReadOnlyClient, ToolContinuationServicePort,
};
use moira_protocol::{
    EffectIdV1, WorkflowIdV1,
    tool_continuation::{CommandV1, MutationV1, OutcomeV1, ReplyV1, StatusV1},
};

#[derive(Default)]
struct Port(Vec<CommandV1>);

impl ToolContinuationServicePort for Port {
    type Error = ();

    fn execute(&mut self, command: CommandV1) -> Result<ReplyV1, Self::Error> {
        self.0.push(command);
        Ok(ReplyV1::Accepted(OutcomeV1::Status(StatusV1 {
            storage_revision: None,
            needs_reopen: false,
        })))
    }
}

#[test]
fn mutation_and_read_facades_expose_disjoint_tool_capabilities() {
    let workflow = WorkflowIdV1([1; 16]);
    let mutation = MutationV1::MarkUncertain {
        workflow,
        effect: EffectIdV1([2; 16]),
    };
    let mut writer = ToolContinuationMutationClient::new(Port::default());
    writer.apply(Some(3), mutation.clone()).unwrap();
    assert_eq!(
        writer.into_port().0,
        vec![CommandV1::Apply {
            expected_storage_revision: Some(3),
            mutation,
        }]
    );

    let mut reader = ToolContinuationReadOnlyClient::new(Port::default());
    reader.status().unwrap();
    reader.saga(workflow).unwrap();
    assert_eq!(
        reader.into_port().0,
        vec![CommandV1::Status, CommandV1::Saga { workflow }]
    );
}
