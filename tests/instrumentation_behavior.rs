use moira_core::instrumentation::{Disposition, Operation, Recorder, RefusalKind};
use moira_core::*;

fn both(
    direct: &mut Engine,
    observed: &mut Engine,
    recorder: &mut Recorder,
    expected: WorkflowRevision,
    command: CommandV1,
) -> Result<TransitionV1, RefusalV1> {
    let a = direct.apply(expected, command);
    let b = observed.apply_observed(expected, command, recorder);
    assert_eq!(a, b);
    assert_eq!(direct.revision(), observed.revision());
    b
}

#[test]
fn off_available_saturated_and_zero_capacity_keep_actual_workflow_results_identical() {
    for capacity in [None, Some(64), Some(1), Some(0)] {
        let mut r = capacity.map_or_else(Recorder::disabled, |n| {
            Recorder::try_with_capacity(n).unwrap()
        });
        let mut direct = Engine::try_new(Limits { max_workflows: 1 }).unwrap();
        let mut observed = Engine::try_new(Limits { max_workflows: 1 }).unwrap();
        let message = AgoraMessageRefV1::new(*b"private-content!", 2, 3);
        let recipient = ParticipantRefV1::new(*b"private-content!");
        let admit = CommandV1::AdmitMessage { message, recipient };
        let TransitionV1::ChronosSubmissionPrepared {
            workflow,
            effect_id,
        } = both(
            &mut direct,
            &mut observed,
            &mut r,
            WorkflowRevision(0),
            admit,
        )
        .unwrap()
        else {
            unreachable!()
        };
        both(
            &mut direct,
            &mut observed,
            &mut r,
            WorkflowRevision(1),
            admit,
        )
        .unwrap();
        assert!(
            both(
                &mut direct,
                &mut observed,
                &mut r,
                WorkflowRevision(1),
                CommandV1::AdmitMessage {
                    message,
                    recipient: ParticipantRefV1::new([9; 16])
                }
            )
            .is_err()
        );
        assert!(
            both(
                &mut direct,
                &mut observed,
                &mut r,
                WorkflowRevision(0),
                admit
            )
            .is_err()
        );
        assert!(
            both(
                &mut direct,
                &mut observed,
                &mut r,
                WorkflowRevision(1),
                CommandV1::AdmitMessage {
                    message: AgoraMessageRefV1::new([8; 16], 9, 10),
                    recipient
                }
            )
            .is_err()
        );
        let task = ChronosTaskRefV1::new([4; 16], 1);
        assert!(
            both(
                &mut direct,
                &mut observed,
                &mut r,
                WorkflowRevision(1),
                CommandV1::AcceptChronosAdmission {
                    workflow,
                    receipt: ChronosAdmissionReceiptV1::new(EffectId::new([9; 16]), task, 7)
                }
            )
            .is_err()
        );
        let admission = CommandV1::AcceptChronosAdmission {
            workflow,
            receipt: ChronosAdmissionReceiptV1::new(effect_id, task, 7),
        };
        both(
            &mut direct,
            &mut observed,
            &mut r,
            WorkflowRevision(1),
            admission,
        )
        .unwrap();
        let TransitionV1::DaimonExecutionPrepared { effect_id, .. } = both(
            &mut direct,
            &mut observed,
            &mut r,
            WorkflowRevision(2),
            CommandV1::AcceptChronosAttempt {
                workflow,
                receipt: ChronosAttemptReceiptV1::new(task, 1, 8),
            },
        )
        .unwrap() else {
            unreachable!()
        };
        let result = CommandV1::AcceptDaimonResult {
            workflow,
            receipt: DaimonResultReceiptV1::new(
                effect_id,
                [5; 16],
                task,
                1,
                DaimonDispositionV1::Completed,
            ),
        };
        both(
            &mut direct,
            &mut observed,
            &mut r,
            WorkflowRevision(3),
            result,
        )
        .unwrap();
        let TransitionV1::AgoraResponsePrepared { effect_id, .. } = both(
            &mut direct,
            &mut observed,
            &mut r,
            WorkflowRevision(4),
            CommandV1::PrepareAgoraResponse { workflow },
        )
        .unwrap() else {
            unreachable!()
        };
        both(
            &mut direct,
            &mut observed,
            &mut r,
            WorkflowRevision(5),
            CommandV1::AcceptAgoraPublication {
                workflow,
                receipt: AgoraPublicationReceiptV1::new(
                    effect_id,
                    AgoraMessageRefV1::new([6; 16], 2, 9),
                    10,
                ),
            },
        )
        .unwrap();
        both(
            &mut direct,
            &mut observed,
            &mut r,
            WorkflowRevision(6),
            admission,
        )
        .unwrap();
        assert!(
            both(
                &mut direct,
                &mut observed,
                &mut r,
                WorkflowRevision(6),
                CommandV1::PrepareAgoraResponse {
                    workflow: WorkflowId::new([9; 16])
                }
            )
            .is_err()
        );
        both(
            &mut direct,
            &mut observed,
            &mut r,
            WorkflowRevision(6),
            result,
        )
        .unwrap();
        assert_eq!(observed.workflow(&workflow), direct.workflow(&workflow));
        assert_eq!(
            (
                r.counts().offered,
                r.counts().accepted,
                r.counts().replayed,
                r.counts().refused
            ),
            (14, 6, 3, 5)
        );
        assert_eq!(r.counts().for_operation(Operation::AdmitMessage).offered, 5);
        assert_eq!(r.counts().harness_receipts.completed, 1);
        assert_eq!(r.counts().for_refusal(RefusalKind::Attribution), 1);
        let usage = observed.usage();
        assert_eq!(
            (
                usage.workflows,
                usage.workflow_capacity,
                usage.completed,
                usage.pending_effects
            ),
            (1, 1, 1, 0)
        );
        let capture = r.usage();
        assert_eq!(capture.enabled, capacity.is_some());
        assert_eq!(
            capture.dropped,
            match capacity {
                Some(1) => 13,
                Some(0) => 14,
                _ => 0,
            }
        );
        assert_eq!(capture.saturated, matches!(capacity, Some(0 | 1)));
        let events: Vec<_> = std::iter::from_fn(|| r.pop_front()).collect();
        assert!(!format!("{r:?} {events:?}").contains("private-content!"));
        assert!(events.iter().all(|e| e.service_time.is_none()));
        if capacity == Some(64) {
            assert_eq!(events.len(), 14);
            assert_eq!(events[0].disposition, Disposition::Accepted);
            assert_eq!(events[1].disposition, Disposition::Replayed);
            assert_eq!(
                events[2].disposition,
                Disposition::Refused(RefusalKind::Conflict)
            );
        }
        assert_eq!(r.usage().dropped, capture.dropped);
        let mut recovered = Engine::recover(observed.checkpoint()).unwrap();
        let mut fresh = Recorder::disabled();
        recovered
            .apply_observed(WorkflowRevision(6), admit, &mut fresh)
            .unwrap();
        assert_eq!((fresh.counts().accepted, fresh.counts().replayed), (0, 1));
    }
}

#[test]
fn uncertain_receipt_and_refused_progress_are_different_observation_populations() {
    let mut engine = Engine::try_new(Limits::default()).unwrap();
    let mut r = Recorder::try_with_capacity(8).unwrap();
    let TransitionV1::ChronosSubmissionPrepared {
        workflow,
        effect_id,
    } = engine
        .apply_observed(
            engine.revision(),
            CommandV1::AdmitMessage {
                message: AgoraMessageRefV1::new([1; 16], 2, 3),
                recipient: ParticipantRefV1::new([2; 16]),
            },
            &mut r,
        )
        .unwrap()
    else {
        unreachable!()
    };
    let task = ChronosTaskRefV1::new([3; 16], 1);
    engine
        .apply_observed(
            engine.revision(),
            CommandV1::AcceptChronosAdmission {
                workflow,
                receipt: ChronosAdmissionReceiptV1::new(effect_id, task, 7),
            },
            &mut r,
        )
        .unwrap();
    let TransitionV1::DaimonExecutionPrepared { effect_id, .. } = engine
        .apply_observed(
            engine.revision(),
            CommandV1::AcceptChronosAttempt {
                workflow,
                receipt: ChronosAttemptReceiptV1::new(task, 1, 8),
            },
            &mut r,
        )
        .unwrap()
    else {
        unreachable!()
    };
    let result = CommandV1::AcceptDaimonResult {
        workflow,
        receipt: DaimonResultReceiptV1::new(
            effect_id,
            [4; 16],
            task,
            1,
            DaimonDispositionV1::Uncertain,
        ),
    };
    engine
        .apply_observed(engine.revision(), result, &mut r)
        .unwrap();
    engine
        .apply_observed(engine.revision(), result, &mut r)
        .unwrap();
    assert!(
        engine
            .apply_observed(
                engine.revision(),
                CommandV1::PrepareAgoraResponse { workflow },
                &mut r
            )
            .is_err()
    );
    assert_eq!(
        (
            r.counts().accepted,
            r.counts().replayed,
            r.counts().refused,
            r.counts().harness_receipts.uncertain
        ),
        (4, 1, 1, 1)
    );
    assert_eq!(
        (
            engine.usage().fenced_uncertain,
            engine.usage().pending_effects
        ),
        (1, 0)
    );
}

#[test]
fn impossible_observation_reservation_is_explicit_and_does_not_admit_work() {
    assert!(Recorder::try_with_capacity(usize::MAX).is_err());
}
