use moira_client::{
    DeliberationHumanGateClient, DeliberationMutationClient, DeliberationReadOnlyClient,
    DeliberationServicePort,
};
use moira_protocol::deliberation::{
    CommandV1, DeliberationIdV1, HumanGateDispositionV1, HumanGateMaterialRefV1,
    HumanGateOperationV1, HumanGateRequestIdV1, HumanGateResponseIdV1,
    HumanGateResponseSubmissionV1, MutationV1, RefusalV1, ReplyV1,
};

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
    reader.review(DeliberationIdV1([1; 16])).unwrap();
    let port = reader.into_port();
    assert!(matches!(port.0[1], CommandV1::Status));
    assert!(matches!(port.0[2], CommandV1::Deliberation { .. }));
    assert!(matches!(port.0[3], CommandV1::Review { .. }));

    let mut gate = DeliberationHumanGateClient::new(port);
    gate.respond(HumanGateOperationV1 {
        deliberation: DeliberationIdV1([1; 16]),
        expected_revision: 8,
        response: HumanGateResponseSubmissionV1 {
            id: HumanGateResponseIdV1([2; 16]),
            request: HumanGateRequestIdV1([3; 16]),
            disposition: HumanGateDispositionV1::ChangesRequested,
            rationale_digest: [4; 32],
            rationale: HumanGateMaterialRefV1 {
                owner_route: [5; 16],
                artifact_route: [6; 16],
                expected_revision: 7,
                expected_sha256: [8; 32],
            },
            responded_at: 9,
        },
    })
    .unwrap();
    let port = gate.into_port();
    assert!(matches!(port.0[4], CommandV1::RespondHumanGate { .. }));
}
