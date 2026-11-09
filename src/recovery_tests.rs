use super::*;

fn admit(engine: &mut Engine, sequence: u64) -> WorkflowId {
    let TransitionV1::ChronosSubmissionPrepared { workflow, .. } = engine
        .apply(
            engine.revision(),
            CommandV1::AdmitMessage {
                message: AgoraMessageRefV1::new([1; 16], 2, sequence),
                recipient: ParticipantRefV1::new([3; 16]),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    workflow
}
fn round_trip(engine: &Engine) {
    assert_eq!(
        Engine::recover(engine.checkpoint())
            .unwrap()
            .checkpoint()
            .revision,
        engine.revision()
    );
}
fn finish(engine: &mut Engine, workflow: WorkflowId, disposition: DaimonDispositionV1) {
    let task = ChronosTaskRefV1::new([4; 16], 1);
    let submission = engine.pending_effect(&workflow).unwrap();
    engine
        .apply(
            engine.revision(),
            CommandV1::AcceptChronosAdmission {
                workflow,
                receipt: ChronosAdmissionReceiptV1::new(submission, task, 7),
            },
        )
        .unwrap();
    round_trip(engine);
    let TransitionV1::DaimonExecutionPrepared { effect_id, .. } = engine
        .apply(
            engine.revision(),
            CommandV1::AcceptChronosAttempt {
                workflow,
                receipt: ChronosAttemptReceiptV1::new(task, 1, 8),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    round_trip(engine);
    engine
        .apply(
            engine.revision(),
            CommandV1::AcceptDaimonResult {
                workflow,
                receipt: DaimonResultReceiptV1::new(effect_id, [5; 16], task, 1, disposition),
            },
        )
        .unwrap();
    round_trip(engine);
    if disposition == DaimonDispositionV1::Uncertain {
        return;
    }
    let TransitionV1::AgoraResponsePrepared { effect_id, .. } = engine
        .apply(
            engine.revision(),
            CommandV1::PrepareAgoraResponse { workflow },
        )
        .unwrap()
    else {
        unreachable!()
    };
    round_trip(engine);
    engine
        .apply(
            engine.revision(),
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
    round_trip(engine);
}
fn checkpoint() -> (CheckpointV1, WorkflowId, WorkflowId) {
    let mut engine = Engine::try_new(Limits::default()).unwrap();
    let first = admit(&mut engine, 1);
    let second = admit(&mut engine, 2);
    finish(&mut engine, first, DaimonDispositionV1::Completed);
    (engine.checkpoint(), first, second)
}
fn invalid(checkpoint: CheckpointV1) {
    assert_eq!(
        Engine::recover(checkpoint).unwrap_err(),
        RecoveryErrorV1::InvalidSnapshot
    );
}

#[test]
fn recovery_checks_all_live_phase_shapes_and_result_dispositions() {
    for disposition in [
        DaimonDispositionV1::Completed,
        DaimonDispositionV1::Failed,
        DaimonDispositionV1::Cancelled,
        DaimonDispositionV1::TimedOut,
        DaimonDispositionV1::Uncertain,
    ] {
        let mut engine = Engine::try_new(Limits::default()).unwrap();
        round_trip(&engine);
        let first = admit(&mut engine, 1);
        round_trip(&engine);
        let second = admit(&mut engine, 2);
        round_trip(&engine);
        finish(&mut engine, first, disposition);
        finish(&mut engine, second, DaimonDispositionV1::Completed);
        let mut recovered = Engine::recover(engine.checkpoint()).unwrap();
        let third = admit(&mut recovered, 3);
        assert_ne!(third, first);
        assert_ne!(third, second);
        round_trip(&recovered);
    }
}

#[test]
fn late_chronos_admission_replay_observes_prior_acceptance_without_unfencing() {
    for disposition in [
        DaimonDispositionV1::Completed,
        DaimonDispositionV1::Uncertain,
    ] {
        let mut engine = Engine::try_new(Limits::default()).unwrap();
        let workflow = admit(&mut engine, 1);
        let task = ChronosTaskRefV1::new([4; 16], 1);
        let receipt =
            ChronosAdmissionReceiptV1::new(engine.pending_effect(&workflow).unwrap(), task, 7);
        let accepted = engine
            .apply(
                engine.revision(),
                CommandV1::AcceptChronosAdmission { workflow, receipt },
            )
            .unwrap();
        let TransitionV1::DaimonExecutionPrepared { effect_id, .. } = engine
            .apply(
                engine.revision(),
                CommandV1::AcceptChronosAttempt {
                    workflow,
                    receipt: ChronosAttemptReceiptV1::new(task, 1, 8),
                },
            )
            .unwrap()
        else {
            unreachable!()
        };
        let replay = |engine: &mut Engine| {
            let revision = engine.revision();
            let snapshot = *engine.workflow(&workflow).unwrap();
            assert_eq!(
                engine
                    .apply(
                        revision,
                        CommandV1::AcceptChronosAdmission { workflow, receipt }
                    )
                    .unwrap(),
                accepted
            );
            assert_eq!(engine.revision(), revision);
            assert_eq!(*engine.workflow(&workflow).unwrap(), snapshot);
            let conflict = ChronosAdmissionReceiptV1::new(
                receipt.effect_id,
                ChronosTaskRefV1::new([9; 16], 9),
                7,
            );
            assert_eq!(
                engine.apply(
                    revision,
                    CommandV1::AcceptChronosAdmission {
                        workflow,
                        receipt: conflict
                    }
                ),
                Err(RefusalV1::ReceiptConflict)
            );
            assert_eq!(*engine.workflow(&workflow).unwrap(), snapshot);
        };
        replay(&mut engine);
        engine
            .apply(
                engine.revision(),
                CommandV1::AcceptDaimonResult {
                    workflow,
                    receipt: DaimonResultReceiptV1::new(effect_id, [5; 16], task, 1, disposition),
                },
            )
            .unwrap();
        engine = Engine::recover(engine.checkpoint()).unwrap();
        replay(&mut engine);
        if disposition == DaimonDispositionV1::Uncertain {
            assert_eq!(
                engine.workflow(&workflow).unwrap().phase(),
                WorkflowPhaseV1::FencedUncertain
            );
            assert_eq!(
                engine.apply(
                    engine.revision(),
                    CommandV1::PrepareAgoraResponse { workflow }
                ),
                Err(RefusalV1::InvalidPhase)
            );
        } else {
            let TransitionV1::AgoraResponsePrepared { effect_id, .. } = engine
                .apply(
                    engine.revision(),
                    CommandV1::PrepareAgoraResponse { workflow },
                )
                .unwrap()
            else {
                unreachable!()
            };
            replay(&mut engine);
            engine
                .apply(
                    engine.revision(),
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
            replay(&mut engine);
        }
    }
}
#[test]
fn allocation_frontiers_cannot_rewind_or_skip_over_retained_identity_space() {
    let (checkpoint, _, _) = checkpoint();
    for change in [-1i64, 1] {
        let mut c = checkpoint.clone();
        c.next_workflow = c.next_workflow.checked_add_signed(change).unwrap();
        invalid(c);
        let mut c = checkpoint.clone();
        c.next_effect = c.next_effect.checked_add_signed(change).unwrap();
        invalid(c);
    }
}
#[test]
fn missing_input_index_and_duplicate_or_foreign_allocations_refuse_recovery() {
    let (checkpoint, first, second) = checkpoint();
    let mut c = checkpoint.clone();
    c.admitted_messages.remove(&c.workflows[&first].cause);
    assert_eq!(
        Engine::recover(c).unwrap_err(),
        RecoveryErrorV1::DanglingMessageIndex
    );
    for effect in [
        checkpoint.workflows[&first].submission_effect,
        EffectId::new([9; 16]),
    ] {
        let mut c = checkpoint.clone();
        let row = c.workflows.get_mut(&second).unwrap();
        row.submission_effect = effect;
        row.pending_effect = Some(effect);
        invalid(c);
    }
    let mut c = checkpoint.clone();
    let mut row = c.workflows.remove(&second).unwrap();
    let foreign = WorkflowId::new([9; 16]);
    row.workflow_id = foreign;
    c.admitted_messages.insert(row.cause, foreign);
    c.workflows.insert(foreign, row);
    invalid(c);
}
#[test]
fn revision_census_and_snapshot_frontiers_cannot_reuse_an_old_optimistic_revision() {
    let (checkpoint, first, second) = checkpoint();
    let mut c = checkpoint.clone();
    c.revision.0 += 1;
    invalid(c);
    let mut c = checkpoint.clone();
    c.workflows.get_mut(&first).unwrap().revision = WorkflowRevision(0);
    invalid(c);
    let mut c = checkpoint.clone();
    c.workflows.get_mut(&first).unwrap().revision = c.workflows[&second].revision;
    invalid(c);
}
#[test]
fn retained_receipts_must_match_the_original_task_attempt_and_effect() {
    let (checkpoint, first, second) = checkpoint();
    let mutations: [fn(&mut WorkflowSnapshotV1); 8] = [
        |s| s.chronos_attempt.as_mut().unwrap().task = ChronosTaskRefV1::new([9; 16], 9),
        |s| s.chronos_attempt.as_mut().unwrap().attempt_number = 0,
        |s| s.chronos_attempt.as_mut().unwrap().chronos_revision = 6,
        |s| s.daimon_result.as_mut().unwrap().effect_id = EffectId::new([9; 16]),
        |s| s.daimon_result.as_mut().unwrap().task = ChronosTaskRefV1::new([9; 16], 9),
        |s| s.daimon_result.as_mut().unwrap().attempt_number = 2,
        |s| s.daimon_result.as_mut().unwrap().disposition = DaimonDispositionV1::Uncertain,
        |s| s.agora_publication.as_mut().unwrap().effect_id = EffectId::new([9; 16]),
    ];
    for mutate in mutations {
        let mut c = checkpoint.clone();
        mutate(c.workflows.get_mut(&first).unwrap());
        invalid(c);
    }
    let mut c = checkpoint;
    c.workflows.get_mut(&second).unwrap().chronos_task = Some(ChronosTaskRefV1::new([8; 16], 1));
    invalid(c);
}
