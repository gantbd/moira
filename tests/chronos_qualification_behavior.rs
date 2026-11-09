use chronos_engine::{Engine as Chronos, IntegrationEffectIdV1, StateRevision, TaskSpecV2};
use moira_core::{
    AgoraMessageRefV1, ChronosAdmissionReceiptV1, ChronosTaskRefV1, CommandV1, Engine as Moira,
    Limits, ParticipantRefV1, TransitionV1, WorkflowRevision,
};

#[test]
fn a_retained_chronos_receipt_closes_lost_reply_replay_without_a_second_task() {
    let mut moira = Moira::try_new(Limits::default()).unwrap();
    let TransitionV1::ChronosSubmissionPrepared {
        workflow,
        effect_id,
    } = moira
        .apply(
            WorkflowRevision(0),
            CommandV1::AdmitMessage {
                message: AgoraMessageRefV1::new([1; 16], 2, 3),
                recipient: ParticipantRefV1::new([4; 16]),
            },
        )
        .unwrap()
    else {
        panic!("expected Chronos submission intent")
    };

    let mut chronos = Chronos::new();
    let chronos_receipt = chronos
        .submit_coordination_graph_with_receipt(
            StateRevision::ZERO,
            IntegrationEffectIdV1::new(effect_id.into_bytes()),
            vec![TaskSpecV2::new("moira-workflow-task")],
        )
        .unwrap();
    let task = &chronos_receipt.tasks()[0];
    let StateRevision::Finite(chronos_revision) = chronos_receipt.revision() else {
        panic!("a first admission has a finite revision")
    };
    let retained = ChronosAdmissionReceiptV1::new(
        effect_id,
        ChronosTaskRefV1::new(effect_id.into_bytes(), task.incarnation().get()),
        chronos_revision,
    );

    let accepted = moira
        .apply(
            WorkflowRevision(1),
            CommandV1::AcceptChronosAdmission {
                workflow,
                receipt: retained,
            },
        )
        .unwrap();
    assert_eq!(moira.revision(), WorkflowRevision(2));

    // A lost Moira reply replays retained evidence; it never resubmits to
    // Chronos and therefore cannot create a fresh task incarnation.
    assert_eq!(
        moira
            .apply(
                WorkflowRevision(2),
                CommandV1::AcceptChronosAdmission {
                    workflow,
                    receipt: retained,
                },
            )
            .unwrap(),
        accepted
    );
    assert_eq!(moira.revision(), WorkflowRevision(2));
    assert_eq!(chronos.tasks().count(), 1);
}
