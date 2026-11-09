use agora_engine::{
    AdmissionPolicy, CommandV1 as AgoraCommand, Engine as Agora, LogicalTime,
    OutcomeV1 as AgoraOutcome, Participant, ParticipantId, ParticipantKind, PublicationCommandV1,
    PublicationKeyV1, RoomId,
};
use moira_core::{
    AgoraMessageRefV1, AgoraPublicationReceiptV1, ChronosAdmissionReceiptV1,
    ChronosAttemptReceiptV1, ChronosTaskRefV1, CommandV1 as MoiraCommand, DaimonDispositionV1,
    DaimonResultReceiptV1, Engine as Moira, Limits, ParticipantRefV1,
    TransitionV1 as MoiraTransition, WorkflowPhaseV1,
};

#[test]
fn lost_agora_reply_is_closed_by_keyed_replay_without_a_duplicate_message() {
    let mut moira = Moira::try_new(Limits::default()).unwrap();
    let MoiraTransition::ChronosSubmissionPrepared {
        workflow,
        effect_id: submission_effect,
    } = moira
        .apply(
            moira.revision(),
            MoiraCommand::AdmitMessage {
                message: AgoraMessageRefV1::new([1; 16], 2, 3),
                recipient: ParticipantRefV1::new([4; 16]),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    let task = ChronosTaskRefV1::new([5; 16], 6);
    moira
        .apply(
            moira.revision(),
            MoiraCommand::AcceptChronosAdmission {
                workflow,
                receipt: ChronosAdmissionReceiptV1::new(submission_effect, task, 7),
            },
        )
        .unwrap();
    let MoiraTransition::DaimonExecutionPrepared {
        effect_id: harness_effect,
        ..
    } = moira
        .apply(
            moira.revision(),
            MoiraCommand::AcceptChronosAttempt {
                workflow,
                receipt: ChronosAttemptReceiptV1::new(task, 1, 8),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    moira
        .apply(
            moira.revision(),
            MoiraCommand::AcceptDaimonResult {
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
    let MoiraTransition::AgoraResponsePrepared {
        effect_id: publication_effect,
        ..
    } = moira
        .apply(
            moira.revision(),
            MoiraCommand::PrepareAgoraResponse { workflow },
        )
        .unwrap()
    else {
        unreachable!()
    };

    let mut agora = Agora::new();
    for (id, kind) in [
        ("human", ParticipantKind::Human),
        ("agent", ParticipantKind::Agent),
    ] {
        agora
            .apply(AgoraCommand::RegisterParticipant(Participant::new(
                id, id, kind,
            )))
            .unwrap();
    }
    agora
        .apply(AgoraCommand::CreateRoom {
            room_id: RoomId::from("room"),
            name: "Room".into(),
            owner: ParticipantId::from("human"),
            admission: AdmissionPolicy::Open,
        })
        .unwrap();
    agora
        .apply(AgoraCommand::Join {
            room_id: RoomId::from("room"),
            participant: ParticipantId::from("agent"),
        })
        .unwrap();
    let publication = PublicationCommandV1::new(
        agora.revision(),
        PublicationKeyV1::new(publication_effect.into_bytes()),
        RoomId::from("room"),
        ParticipantId::from("agent"),
        LogicalTime(10),
        "answer",
    );

    let first = agora.publish(publication.clone()).unwrap();
    let replay = agora.publish(publication).unwrap();
    assert_eq!(replay, first);
    assert_eq!(agora.room(&RoomId::from("room")).unwrap().messages.len(), 1);
    let AgoraOutcome::MessageAccepted { sequence, .. } = first.outcome else {
        unreachable!()
    };

    let receipt = AgoraPublicationReceiptV1::new(
        publication_effect,
        AgoraMessageRefV1::new([10; 16], 0, sequence),
        first.revision.0,
    );
    let accepted = moira
        .apply(
            moira.revision(),
            MoiraCommand::AcceptAgoraPublication { workflow, receipt },
        )
        .unwrap();
    let revision = moira.revision();
    assert_eq!(
        moira.workflow(&workflow).unwrap().phase(),
        WorkflowPhaseV1::Completed
    );
    assert_eq!(
        moira
            .apply(
                revision,
                MoiraCommand::AcceptAgoraPublication { workflow, receipt },
            )
            .unwrap(),
        accepted
    );
    assert_eq!(moira.revision(), revision);
}
