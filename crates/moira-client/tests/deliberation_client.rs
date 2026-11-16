use moira_client::{
    DeliberationMutationClient, DeliberationReadOnlyClient, DeliberationServicePort,
};
use moira_protocol::deliberation::{CommandV1, DeliberationIdV1, MutationV1, RefusalV1, ReplyV1};

#[derive(Default)]
struct RecordingPort(Vec<CommandV1>);

impl DeliberationServicePort for RecordingPort {
    type Error = ();

    fn execute(&mut self, command: CommandV1) -> Result<ReplyV1, Self::Error> {
        self.0.push(command);
        Ok(ReplyV1::Refused(RefusalV1::NotAuthorized))
    }
}

#[test]
fn capability_clients_emit_only_their_closed_command_shapes() {
    let mut mutation = DeliberationMutationClient::new(RecordingPort::default());
    mutation
        .apply(
            7,
            MutationV1::Resolve {
                deliberation: DeliberationIdV1([1; 16]),
                resolution: moira_protocol::deliberation::ResolutionV1::Mu(
                    moira_protocol::deliberation::MuReasonV1::MissingEvidence,
                ),
            },
        )
        .unwrap();
    let port = mutation.into_port();
    assert!(matches!(
        port.0.as_slice(),
        [CommandV1::Apply {
            expected_revision: 7,
            ..
        }]
    ));

    let mut reader = DeliberationReadOnlyClient::new(port);
    reader.status().unwrap();
    reader.deliberation(DeliberationIdV1([1; 16])).unwrap();
    let port = reader.into_port();
    assert!(matches!(port.0[1], CommandV1::Status));
    assert!(matches!(port.0[2], CommandV1::Deliberation { .. }));
}
