use hermes_ledger::{
    AppendOutcomeV1, CorrelationId, Engine as Hermes, EventEnvelopeV1, EventSequence,
    IdempotencyKey, Limits as HermesLimits, ProducerId, SchemaId, StreamId,
};
use moira_core::{
    AgoraMessageRefV1, CommandV1, Engine as Moira, Limits as MoiraLimits, ParticipantRefV1,
    TransitionV1,
};

#[test]
fn lost_hermes_append_reply_replays_the_moira_effect_without_a_second_event() {
    let mut moira = Moira::try_new(MoiraLimits::default()).unwrap();
    let TransitionV1::ChronosSubmissionPrepared {
        workflow: _,
        effect_id,
    } = moira
        .apply(
            moira.revision(),
            CommandV1::AdmitMessage {
                message: AgoraMessageRefV1::new([1; 16], 2, 3),
                recipient: ParticipantRefV1::new([4; 16]),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    let effect_bytes = effect_id.into_bytes();
    let stream = StreamId::new([5; 16]);
    let event = EventEnvelopeV1::new(
        ProducerId::new([6; 16]),
        SchemaId::new([7; 16]),
        IdempotencyKey::new(effect_bytes),
        CorrelationId::new(effect_bytes),
        None,
        b"moira.chronos-submission-prepared.v1".to_vec(),
    );
    let mut hermes = Hermes::try_new(HermesLimits::default()).unwrap();

    assert_eq!(
        hermes.append(stream, None, event.clone()).unwrap(),
        AppendOutcomeV1::Appended(EventSequence(0))
    );
    assert_eq!(
        hermes.append(stream, None, event).unwrap(),
        AppendOutcomeV1::Replayed(EventSequence(0))
    );
    assert_eq!(
        hermes
            .read(stream, EventSequence(0), 2)
            .unwrap()
            .events
            .len(),
        1
    );
}
