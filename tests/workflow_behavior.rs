use moira_core::{
    AgoraMessageRefV1, AgoraPublicationReceiptV1, ChronosAdmissionReceiptV1,
    ChronosAttemptReceiptV1, ChronosTaskRefV1, CommandV1, DaimonDispositionV1,
    DaimonResultReceiptV1, EffectId, Engine, Limits, ParticipantRefV1, RefusalV1, TransitionV1,
    WorkflowPhaseV1, WorkflowRevision,
};

fn message() -> AgoraMessageRefV1 {
    AgoraMessageRefV1::new([1; 16], 7, 11)
}

fn admitted_workflow(engine: &mut Engine) -> (moira_core::WorkflowId, ChronosTaskRefV1) {
    let TransitionV1::ChronosSubmissionPrepared {
        workflow,
        effect_id,
    } = engine
        .apply(
            engine.revision(),
            CommandV1::AdmitMessage {
                message: message(),
                recipient: ParticipantRefV1::new([2; 16]),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    let task = ChronosTaskRefV1::new([3; 16], 4);
    engine
        .apply(
            engine.revision(),
            CommandV1::AcceptChronosAdmission {
                workflow,
                receipt: ChronosAdmissionReceiptV1::new(effect_id, task, 1),
            },
        )
        .unwrap();
    (workflow, task)
}

#[test]
fn one_message_prepares_one_stable_chronos_submission_intent() {
    let mut engine = Engine::try_new(Limits::default()).unwrap();
    let first = engine
        .apply(
            WorkflowRevision(0),
            CommandV1::AdmitMessage {
                message: message(),
                recipient: ParticipantRefV1::new([2; 16]),
            },
        )
        .unwrap();
    let TransitionV1::ChronosSubmissionPrepared {
        workflow,
        effect_id,
    } = first
    else {
        panic!("expected submission intent")
    };

    assert_eq!(
        engine.workflow(&workflow).unwrap().phase(),
        WorkflowPhaseV1::TaskSubmissionPending
    );
    assert_eq!(engine.pending_effect(&workflow), Some(effect_id));
    assert_eq!(
        engine
            .apply(
                WorkflowRevision(1),
                CommandV1::AdmitMessage {
                    message: message(),
                    recipient: ParticipantRefV1::new([2; 16]),
                },
            )
            .unwrap(),
        first
    );
    assert_eq!(engine.revision(), WorkflowRevision(1));
}

#[test]
fn only_the_exact_pending_effect_receipt_advances_the_workflow() {
    let mut engine = Engine::try_new(Limits::default()).unwrap();
    let TransitionV1::ChronosSubmissionPrepared {
        workflow,
        effect_id,
    } = engine
        .apply(
            WorkflowRevision(0),
            CommandV1::AdmitMessage {
                message: message(),
                recipient: ParticipantRefV1::new([2; 16]),
            },
        )
        .unwrap()
    else {
        panic!("expected submission intent")
    };

    let wrong = EffectId::new([9; 16]);
    assert_eq!(
        engine.apply(
            WorkflowRevision(1),
            CommandV1::AcceptChronosAdmission {
                workflow,
                receipt: ChronosAdmissionReceiptV1::new(
                    wrong,
                    ChronosTaskRefV1::new([3; 16], 4),
                    1,
                ),
            },
        ),
        Err(RefusalV1::EffectMismatch)
    );
    assert_eq!(engine.revision(), WorkflowRevision(1));

    let receipt = ChronosAdmissionReceiptV1::new(effect_id, ChronosTaskRefV1::new([3; 16], 4), 1);
    let accepted = engine
        .apply(
            WorkflowRevision(1),
            CommandV1::AcceptChronosAdmission { workflow, receipt },
        )
        .unwrap();
    assert!(matches!(
        accepted,
        TransitionV1::ChronosSubmissionAccepted { .. }
    ));

    assert_eq!(
        engine
            .apply(
                WorkflowRevision(2),
                CommandV1::AcceptChronosAdmission { workflow, receipt },
            )
            .unwrap(),
        accepted
    );
    assert_eq!(engine.revision(), WorkflowRevision(2));
}

#[test]
fn replay_with_the_same_source_identity_but_different_content_is_a_conflict() {
    let mut engine = Engine::try_new(Limits::default()).unwrap();
    engine
        .apply(
            WorkflowRevision(0),
            CommandV1::AdmitMessage {
                message: message(),
                recipient: ParticipantRefV1::new([2; 16]),
            },
        )
        .unwrap();

    assert_eq!(
        engine.apply(
            WorkflowRevision(1),
            CommandV1::AdmitMessage {
                message: message(),
                recipient: ParticipantRefV1::new([8; 16]),
            },
        ),
        Err(RefusalV1::InputConflict)
    );
    assert_eq!(engine.revision(), WorkflowRevision(1));
}

#[test]
fn stale_revision_refuses_before_workflow_or_effect_identity_mutation() {
    let mut engine = Engine::try_new(Limits::default()).unwrap();
    engine
        .apply(
            WorkflowRevision(0),
            CommandV1::AdmitMessage {
                message: message(),
                recipient: ParticipantRefV1::new([2; 16]),
            },
        )
        .unwrap();

    assert_eq!(
        engine.apply(
            WorkflowRevision(0),
            CommandV1::AdmitMessage {
                message: AgoraMessageRefV1::new([8; 16], 9, 10),
                recipient: ParticipantRefV1::new([2; 16]),
            },
        ),
        Err(RefusalV1::RevisionConflict {
            expected: WorkflowRevision(0),
            actual: WorkflowRevision(1),
        })
    );
    assert_eq!(engine.revision(), WorkflowRevision(1));

    let TransitionV1::ChronosSubmissionPrepared { effect_id, .. } = engine
        .apply(
            WorkflowRevision(1),
            CommandV1::AdmitMessage {
                message: AgoraMessageRefV1::new([8; 16], 9, 10),
                recipient: ParticipantRefV1::new([2; 16]),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(effect_id.into_bytes()[8], 1);
}

#[test]
fn exact_chronos_attempt_prepares_one_replay_safe_daimon_execution() {
    let mut engine = Engine::try_new(Limits::default()).unwrap();
    let (workflow, task) = admitted_workflow(&mut engine);
    let receipt = ChronosAttemptReceiptV1::new(task, 1, 3);

    let first = engine
        .apply(
            WorkflowRevision(2),
            CommandV1::AcceptChronosAttempt { workflow, receipt },
        )
        .unwrap();
    let TransitionV1::DaimonExecutionPrepared { effect_id, .. } = first else {
        panic!("expected Daimon intent")
    };
    assert_eq!(
        engine.workflow(&workflow).unwrap().phase(),
        WorkflowPhaseV1::HarnessPending
    );
    assert_eq!(engine.pending_effect(&workflow), Some(effect_id));
    assert_eq!(
        engine
            .apply(
                WorkflowRevision(3),
                CommandV1::AcceptChronosAttempt { workflow, receipt },
            )
            .unwrap(),
        first
    );
    assert_eq!(engine.revision(), WorkflowRevision(3));
}

#[test]
fn uncertain_daimon_evidence_fences_the_workflow_and_replays_without_progress() {
    let mut engine = Engine::try_new(Limits::default()).unwrap();
    let (workflow, task) = admitted_workflow(&mut engine);
    let attempt = ChronosAttemptReceiptV1::new(task, 1, 3);
    let TransitionV1::DaimonExecutionPrepared { effect_id, .. } = engine
        .apply(
            WorkflowRevision(2),
            CommandV1::AcceptChronosAttempt {
                workflow,
                receipt: attempt,
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    let receipt =
        DaimonResultReceiptV1::new(effect_id, [7; 16], task, 1, DaimonDispositionV1::Uncertain);

    let first = engine
        .apply(
            WorkflowRevision(3),
            CommandV1::AcceptDaimonResult { workflow, receipt },
        )
        .unwrap();
    assert_eq!(
        engine.workflow(&workflow).unwrap().phase(),
        WorkflowPhaseV1::FencedUncertain
    );
    assert_eq!(
        engine
            .apply(
                WorkflowRevision(4),
                CommandV1::AcceptDaimonResult { workflow, receipt },
            )
            .unwrap(),
        first
    );
    assert_eq!(engine.revision(), WorkflowRevision(4));
}

#[test]
fn completed_harness_result_prepares_and_accepts_one_replay_safe_agora_response() {
    let mut engine = Engine::try_new(Limits::default()).unwrap();
    let (workflow, task) = admitted_workflow(&mut engine);
    let TransitionV1::DaimonExecutionPrepared { effect_id, .. } = engine
        .apply(
            WorkflowRevision(2),
            CommandV1::AcceptChronosAttempt {
                workflow,
                receipt: ChronosAttemptReceiptV1::new(task, 1, 3),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    engine
        .apply(
            WorkflowRevision(3),
            CommandV1::AcceptDaimonResult {
                workflow,
                receipt: DaimonResultReceiptV1::new(
                    effect_id,
                    [7; 16],
                    task,
                    1,
                    DaimonDispositionV1::Completed,
                ),
            },
        )
        .unwrap();

    let prepared = engine
        .apply(
            WorkflowRevision(4),
            CommandV1::PrepareAgoraResponse { workflow },
        )
        .unwrap();
    let TransitionV1::AgoraResponsePrepared {
        effect_id: publication_effect,
        ..
    } = prepared
    else {
        panic!("expected Agora publication intent")
    };
    assert_eq!(
        engine
            .apply(
                WorkflowRevision(5),
                CommandV1::PrepareAgoraResponse { workflow },
            )
            .unwrap(),
        prepared
    );
    assert_eq!(engine.revision(), WorkflowRevision(5));

    let receipt = AgoraPublicationReceiptV1::new(
        publication_effect,
        AgoraMessageRefV1::new([8; 16], 9, 10),
        11,
    );
    let accepted = engine
        .apply(
            WorkflowRevision(5),
            CommandV1::AcceptAgoraPublication { workflow, receipt },
        )
        .unwrap();
    assert_eq!(
        engine.workflow(&workflow).unwrap().phase(),
        WorkflowPhaseV1::Completed
    );
    assert_eq!(
        engine
            .apply(
                WorkflowRevision(6),
                CommandV1::AcceptAgoraPublication { workflow, receipt },
            )
            .unwrap(),
        accepted
    );
    assert_eq!(engine.revision(), WorkflowRevision(6));
}
