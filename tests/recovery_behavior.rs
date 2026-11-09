use moira_core::{
    AgoraMessageRefV1, AgoraPublicationReceiptV1, ChronosAdmissionReceiptV1,
    ChronosAttemptReceiptV1, ChronosTaskRefV1, CommandV1, DaimonDispositionV1,
    DaimonResultReceiptV1, Engine, Limits, ParticipantRefV1, TransitionV1, WorkflowPhaseV1,
};

#[test]
fn checkpoint_recovery_preserves_every_effect_identity_and_closes_replays() {
    let mut engine = Engine::try_new(Limits::default()).unwrap();
    let message = AgoraMessageRefV1::new([1; 16], 2, 3);
    let recipient = ParticipantRefV1::new([4; 16]);
    let admitted = engine
        .apply(
            engine.revision(),
            CommandV1::AdmitMessage { message, recipient },
        )
        .unwrap();
    let TransitionV1::ChronosSubmissionPrepared {
        workflow,
        effect_id: submission_effect,
    } = admitted
    else {
        unreachable!()
    };

    let mut engine = Engine::recover(engine.checkpoint()).unwrap();
    assert_eq!(
        engine
            .apply(
                engine.revision(),
                CommandV1::AdmitMessage { message, recipient },
            )
            .unwrap(),
        admitted
    );
    let task = ChronosTaskRefV1::new([5; 16], 6);
    engine
        .apply(
            engine.revision(),
            CommandV1::AcceptChronosAdmission {
                workflow,
                receipt: ChronosAdmissionReceiptV1::new(submission_effect, task, 7),
            },
        )
        .unwrap();
    let attempt = ChronosAttemptReceiptV1::new(task, 1, 8);
    let prepared = engine
        .apply(
            engine.revision(),
            CommandV1::AcceptChronosAttempt {
                workflow,
                receipt: attempt,
            },
        )
        .unwrap();
    let TransitionV1::DaimonExecutionPrepared {
        effect_id: harness_effect,
        ..
    } = prepared
    else {
        unreachable!()
    };

    let mut engine = Engine::recover(engine.checkpoint()).unwrap();
    assert_eq!(
        engine
            .apply(
                engine.revision(),
                CommandV1::AcceptChronosAttempt {
                    workflow,
                    receipt: attempt,
                },
            )
            .unwrap(),
        prepared
    );
    engine
        .apply(
            engine.revision(),
            CommandV1::AcceptDaimonResult {
                workflow,
                receipt: DaimonResultReceiptV1::new(
                    harness_effect,
                    [9; 16],
                    task,
                    1,
                    DaimonDispositionV1::Completed,
                ),
            },
        )
        .unwrap();
    let publication = engine
        .apply(
            engine.revision(),
            CommandV1::PrepareAgoraResponse { workflow },
        )
        .unwrap();
    let TransitionV1::AgoraResponsePrepared {
        effect_id: publication_effect,
        ..
    } = publication
    else {
        unreachable!()
    };

    let mut engine = Engine::recover(engine.checkpoint()).unwrap();
    assert_eq!(
        engine
            .apply(
                engine.revision(),
                CommandV1::PrepareAgoraResponse { workflow },
            )
            .unwrap(),
        publication
    );
    let receipt = AgoraPublicationReceiptV1::new(
        publication_effect,
        AgoraMessageRefV1::new([10; 16], 11, 12),
        13,
    );
    engine
        .apply(
            engine.revision(),
            CommandV1::AcceptAgoraPublication { workflow, receipt },
        )
        .unwrap();
    let revision = engine.revision();

    let mut engine = Engine::recover(engine.checkpoint()).unwrap();
    assert_eq!(
        engine.workflow(&workflow).unwrap().phase(),
        WorkflowPhaseV1::Completed
    );
    engine
        .apply(
            engine.revision(),
            CommandV1::AcceptAgoraPublication { workflow, receipt },
        )
        .unwrap();
    assert_eq!(engine.revision(), revision);
}
