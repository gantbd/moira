use moira_core::saga::*;

fn admit(engine: &mut SagaEngineV1, budget: u8) -> (SagaIdV1, SagaEffectIdV1, TimerIdV1) {
    let SagaTransitionV1::Admitted {
        saga,
        effect,
        timer,
    } = engine
        .apply(
            0,
            SagaCommandV1::Admit {
                source: [1; 16],
                now: 1,
                deadline: 10,
                retry_budget: budget,
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    (saga, effect, timer)
}

#[test]
fn cancellation_after_forward_success_prepares_attributable_compensation() {
    let mut engine = SagaEngineV1::try_new(SagaLimitsV1::default()).unwrap();
    let (saga, forward, _) = admit(&mut engine, 1);
    engine
        .apply(
            1,
            SagaCommandV1::AcceptEffect {
                saga,
                effect: forward,
                outcome: EffectOutcomeV1::Succeeded,
                observed_at: 3,
            },
        )
        .unwrap();
    let SagaTransitionV1::EffectPrepared {
        effect: terminal, ..
    } = engine
        .apply(2, SagaCommandV1::PrepareTerminal { saga, now: 4 })
        .unwrap()
    else {
        unreachable!()
    };
    let SagaTransitionV1::Cancelled {
        compensation: Some(compensation),
        ..
    } = engine
        .apply(3, SagaCommandV1::Cancel { saga, now: 5 })
        .unwrap()
    else {
        panic!("known forward success requires compensation");
    };
    assert_ne!(compensation, forward);
    assert_ne!(compensation, terminal);
    assert_eq!(
        engine.apply(
            4,
            SagaCommandV1::AcceptEffect {
                saga,
                effect: terminal,
                outcome: EffectOutcomeV1::Succeeded,
                observed_at: 6,
            }
        ),
        Err(SagaRefusalV1::LateEvidence)
    );
    engine
        .apply(
            4,
            SagaCommandV1::AcceptEffect {
                saga,
                effect: compensation,
                outcome: EffectOutcomeV1::Succeeded,
                observed_at: 6,
            },
        )
        .unwrap();
    assert_eq!(engine.saga(saga).unwrap().phase, SagaPhaseV1::Cancelled);
}

#[test]
fn deadlines_and_retries_require_explicit_commands_and_never_imply_success() {
    let mut engine = SagaEngineV1::try_new(SagaLimitsV1::default()).unwrap();
    let (saga, forward, timer) = admit(&mut engine, 1);
    assert_eq!(
        engine.apply(
            1,
            SagaCommandV1::FireDeadline {
                saga,
                timer,
                observed_at: 9,
                action: DeadlineActionV1::Fail,
            }
        ),
        Err(SagaRefusalV1::DeadlineNotReached)
    );
    engine
        .apply(
            1,
            SagaCommandV1::AcceptEffect {
                saga,
                effect: forward,
                outcome: EffectOutcomeV1::Failed,
                observed_at: 4,
            },
        )
        .unwrap();
    let SagaTransitionV1::EffectPrepared { effect: retry, .. } = engine
        .apply(
            2,
            SagaCommandV1::Retry {
                saga,
                failed_effect: forward,
                now: 5,
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    engine
        .apply(
            3,
            SagaCommandV1::AcceptEffect {
                saga,
                effect: retry,
                outcome: EffectOutcomeV1::Failed,
                observed_at: 6,
            },
        )
        .unwrap();
    assert_eq!(
        engine.apply(
            4,
            SagaCommandV1::Retry {
                saga,
                failed_effect: retry,
                now: 7,
            }
        ),
        Err(SagaRefusalV1::RetryBudget)
    );
    let transition = engine
        .apply(
            4,
            SagaCommandV1::FireDeadline {
                saga,
                timer,
                observed_at: 10,
                action: DeadlineActionV1::Fail,
            },
        )
        .unwrap();
    assert!(matches!(
        transition,
        SagaTransitionV1::DeadlineApplied {
            phase: SagaPhaseV1::Failed,
            ..
        }
    ));
}

#[test]
fn exact_evidence_replay_survives_progress_and_checkpoint_recovery() {
    let mut engine = SagaEngineV1::try_new(SagaLimitsV1::default()).unwrap();
    let (saga, forward, _) = admit(&mut engine, 1);
    let command = SagaCommandV1::AcceptEffect {
        saga,
        effect: forward,
        outcome: EffectOutcomeV1::Succeeded,
        observed_at: 3,
    };
    let accepted = engine.apply(1, command).unwrap();
    engine
        .apply(2, SagaCommandV1::PrepareTerminal { saga, now: 4 })
        .unwrap();
    engine = SagaEngineV1::recover(engine.checkpoint()).unwrap();
    assert_eq!(engine.apply(3, command), Ok(accepted));
    assert_eq!(engine.revision(), 3);
}

#[test]
fn multi_saga_inspection_is_bounded_ordered_and_non_authoritative() {
    let mut engine = SagaEngineV1::try_new(SagaLimitsV1 {
        max_sagas: 3,
        max_retries_per_saga: 1,
        max_inspection_page: 2,
        max_operator_actions: 8,
    })
    .unwrap();
    let mut sagas = Vec::new();
    for source in 1..=3 {
        let SagaTransitionV1::Admitted { saga, .. } = engine
            .apply(
                engine.revision(),
                SagaCommandV1::Admit {
                    source: [source; 16],
                    now: 1,
                    deadline: 10,
                    retry_budget: 1,
                },
            )
            .unwrap()
        else {
            unreachable!()
        };
        sagas.push(saga);
    }
    let revision = engine.revision();
    let first = engine.inspect_page(None, 2).unwrap();
    assert_eq!(
        first
            .entries
            .iter()
            .map(|entry| entry.saga)
            .collect::<Vec<_>>(),
        sagas[..2]
    );
    let second = engine.inspect_page(first.next_after, 2).unwrap();
    assert_eq!(second.entries[0].saga, sagas[2]);
    assert_eq!(engine.revision(), revision);
    assert_eq!(
        engine.inspect_page(None, 0),
        Err(SagaRefusalV1::InvalidPageLimit)
    );
    assert_eq!(
        engine.inspect_page(None, 3),
        Err(SagaRefusalV1::InvalidPageLimit)
    );
    assert_eq!(
        engine.census(),
        SagaCensusV1 {
            total: 3,
            runnable_effects: 3,
            awaiting_decision: 0,
            fenced: 0,
            terminal: 0,
            retries_consumed: 0,
            saturated: false,
        }
    );
    assert_eq!(engine.revision(), revision);
}

#[test]
fn reconciliation_reads_explain_last_revision_evidence_and_fence_without_mutation() {
    let mut engine = SagaEngineV1::try_new(SagaLimitsV1 {
        max_sagas: 2,
        max_retries_per_saga: 1,
        max_inspection_page: 2,
        max_operator_actions: 8,
    })
    .unwrap();
    let (saga, effect, _) = admit(&mut engine, 1);
    let admitted = engine.reconciliation_view(saga).unwrap();
    assert_eq!(admitted.last_confirmed_revision, 1);
    assert_eq!(admitted.retained_evidence, 0);
    assert_eq!(admitted.automatic_progress, AutomaticProgressV1::Runnable);
    assert_eq!(admitted.fence, None);

    engine
        .apply(
            1,
            SagaCommandV1::AcceptEffect {
                saga,
                effect,
                outcome: EffectOutcomeV1::Uncertain,
                observed_at: 4,
            },
        )
        .unwrap();
    let revision = engine.revision();
    let fenced = engine.reconciliation_view(saga).unwrap();
    assert_eq!(fenced.last_confirmed_revision, 2);
    assert_eq!(fenced.retained_evidence, 1);
    assert_eq!(
        fenced.automatic_progress,
        AutomaticProgressV1::BlockedUncertain
    );
    assert_eq!(
        fenced.fence,
        Some(FenceCauseV1::EffectUncertain {
            effect,
            kind: EffectKindV1::Forward,
            observed_at: 4,
        })
    );
    assert_eq!(
        engine.evidence_page(saga, None, 2).unwrap(),
        EffectEvidencePageV1 {
            entries: vec![EffectEvidenceSummaryV1 {
                effect,
                outcome: EffectOutcomeV1::Uncertain,
                observed_at: 4,
            }],
            next_after: None,
        }
    );
    assert_eq!(
        engine.evidence_page(saga, None, 0),
        Err(SagaRefusalV1::InvalidPageLimit)
    );
    assert_eq!(engine.revision(), revision);
}

#[test]
fn operator_actions_are_attributed_revisioned_and_exactly_replayable_after_recovery() {
    let mut engine = SagaEngineV1::try_new(SagaLimitsV1 {
        max_sagas: 1,
        max_retries_per_saga: 1,
        max_inspection_page: 2,
        max_operator_actions: 4,
    })
    .unwrap();
    let (saga, effect, _) = admit(&mut engine, 1);
    engine
        .apply(
            1,
            SagaCommandV1::AcceptEffect {
                saga,
                effect,
                outcome: EffectOutcomeV1::Uncertain,
                observed_at: 4,
            },
        )
        .unwrap();
    let keep_cause = OperatorCauseV1 {
        action: OperatorActionIdV1([1; 16]),
        subject: [2; 16],
        authority_decision: 3,
        authority_epoch: 4,
    };
    let keep = SagaCommandV1::Reconcile {
        cause: keep_cause,
        saga,
        disposition: OperatorDispositionV1::KeepUncertain,
    };
    let kept = engine.apply(2, keep).unwrap();
    assert_eq!(engine.revision(), 3);
    assert!(matches!(
        kept,
        SagaTransitionV1::Reconciled {
            phase: SagaPhaseV1::FencedUncertain,
            prepared: None,
            ..
        }
    ));
    let summary = engine.operator_receipt(keep_cause.action).unwrap();
    assert_eq!(summary.saga, saga);
    assert_eq!(summary.authority_decision, 3);
    assert_eq!(summary.authority_epoch, 4);
    assert_eq!(summary.disposition, OperatorDispositionV1::KeepUncertain);
    assert_eq!(summary.phase, SagaPhaseV1::FencedUncertain);
    assert_eq!(summary.prepared, None);
    assert_eq!(engine.revision(), 3);

    let mut recovered = SagaEngineV1::recover(engine.checkpoint()).unwrap();
    assert_eq!(recovered.apply(2, keep).unwrap(), kept);
    assert_eq!(recovered.revision(), 3);
    assert_eq!(
        recovered.apply(
            3,
            SagaCommandV1::Reconcile {
                cause: keep_cause,
                saga,
                disposition: OperatorDispositionV1::FailTerminal,
            },
        ),
        Err(SagaRefusalV1::OperatorActionConflict)
    );

    let evidence = SagaCommandV1::Reconcile {
        cause: OperatorCauseV1 {
            action: OperatorActionIdV1([5; 16]),
            subject: [2; 16],
            authority_decision: 6,
            authority_epoch: 4,
        },
        saga,
        disposition: OperatorDispositionV1::AdmitEvidence {
            effect,
            outcome: EffectOutcomeV1::Failed,
            observed_at: 7,
        },
    };
    recovered.apply(3, evidence).unwrap();
    assert_eq!(
        recovered.saga(saga).unwrap().phase,
        SagaPhaseV1::ForwardFailed
    );
    let retried = recovered
        .apply(
            4,
            SagaCommandV1::Reconcile {
                cause: OperatorCauseV1 {
                    action: OperatorActionIdV1([8; 16]),
                    subject: [2; 16],
                    authority_decision: 9,
                    authority_epoch: 4,
                },
                saga,
                disposition: OperatorDispositionV1::Retry {
                    failed_effect: effect,
                    now: 8,
                },
            },
        )
        .unwrap();
    assert!(matches!(
        retried,
        SagaTransitionV1::Reconciled {
            phase: SagaPhaseV1::ForwardPending,
            prepared: Some(fresh),
            ..
        } if fresh != effect
    ));
    let first_page = recovered.operator_receipt_page(None, 2).unwrap();
    assert_eq!(first_page.entries.len(), 2);
    assert_eq!(first_page.entries[0].action, keep_cause.action);
    let second_page = recovered
        .operator_receipt_page(first_page.next_after, 2)
        .unwrap();
    assert_eq!(second_page.entries.len(), 1);
    assert_eq!(second_page.entries[0].authority_decision, 9);
    assert_eq!(
        second_page.entries[0].prepared,
        match retried {
            SagaTransitionV1::Reconciled { prepared, .. } => prepared,
            _ => unreachable!(),
        }
    );
    assert_eq!(
        recovered.operator_receipt_page(None, 0),
        Err(SagaRefusalV1::InvalidPageLimit)
    );
    assert_eq!(recovered.revision(), 6);
    let reopened = SagaEngineV1::recover(recovered.checkpoint()).unwrap();
    assert_eq!(
        reopened.operator_receipt(keep_cause.action),
        recovered.operator_receipt(keep_cause.action)
    );
    assert_eq!(
        reopened.operator_receipt(OperatorActionIdV1([5; 16])),
        recovered.operator_receipt(OperatorActionIdV1([5; 16]))
    );
    assert_eq!(
        reopened.operator_receipt(OperatorActionIdV1([8; 16])),
        recovered.operator_receipt(OperatorActionIdV1([8; 16]))
    );
}

#[test]
fn operator_dispositions_never_collapse_uncertainty_into_unattributed_success() {
    fn fenced() -> (SagaEngineV1, SagaIdV1, SagaEffectIdV1) {
        let mut engine = SagaEngineV1::try_new(SagaLimitsV1 {
            max_sagas: 1,
            max_retries_per_saga: 1,
            max_inspection_page: 2,
            max_operator_actions: 4,
        })
        .unwrap();
        let (saga, effect, _) = admit(&mut engine, 1);
        engine
            .apply(
                1,
                SagaCommandV1::AcceptEffect {
                    saga,
                    effect,
                    outcome: EffectOutcomeV1::Uncertain,
                    observed_at: 2,
                },
            )
            .unwrap();
        (engine, saga, effect)
    }

    let cause = |byte| OperatorCauseV1 {
        action: OperatorActionIdV1([byte; 16]),
        subject: [9; 16],
        authority_decision: u64::from(byte),
        authority_epoch: 1,
    };

    let (mut evidence, saga, effect) = fenced();
    evidence
        .apply(
            2,
            SagaCommandV1::Reconcile {
                cause: cause(1),
                saga,
                disposition: OperatorDispositionV1::AdmitEvidence {
                    effect,
                    outcome: EffectOutcomeV1::Succeeded,
                    observed_at: 3,
                },
            },
        )
        .unwrap();
    assert_eq!(
        evidence.saga(saga).unwrap().phase,
        SagaPhaseV1::ForwardSucceeded
    );
    let recovered = SagaEngineV1::recover(evidence.checkpoint()).unwrap();
    assert_eq!(
        recovered.operator_receipt(cause(1).action),
        evidence.operator_receipt(cause(1).action)
    );
    assert_eq!(
        recovered.saga(saga).unwrap().phase,
        SagaPhaseV1::ForwardSucceeded
    );

    for (byte, disposition) in [
        (2, OperatorDispositionV1::Compensate { now: 3 }),
        (3, OperatorDispositionV1::Cancel { now: 3 }),
    ] {
        let (mut engine, saga, original) = fenced();
        let transition = engine
            .apply(
                2,
                SagaCommandV1::Reconcile {
                    cause: cause(byte),
                    saga,
                    disposition,
                },
            )
            .unwrap();
        assert!(matches!(
            transition,
            SagaTransitionV1::Reconciled {
                phase: SagaPhaseV1::CompensationPending,
                prepared: Some(compensation),
                ..
            } if compensation != original
        ));
        assert_eq!(engine.revision(), 4);
        let recovered = SagaEngineV1::recover(engine.checkpoint()).unwrap();
        assert_eq!(
            recovered.operator_receipt(cause(byte).action),
            engine.operator_receipt(cause(byte).action)
        );
        assert_eq!(
            recovered.saga(saga).unwrap().phase,
            SagaPhaseV1::CompensationPending
        );
    }

    let (mut failed, saga, _) = fenced();
    failed
        .apply(
            2,
            SagaCommandV1::Reconcile {
                cause: cause(4),
                saga,
                disposition: OperatorDispositionV1::FailTerminal,
            },
        )
        .unwrap();
    assert_eq!(failed.saga(saga).unwrap().phase, SagaPhaseV1::Failed);
    assert_eq!(failed.revision(), 3);
    let recovered = SagaEngineV1::recover(failed.checkpoint()).unwrap();
    assert_eq!(
        recovered.operator_receipt(cause(4).action),
        failed.operator_receipt(cause(4).action)
    );
    assert_eq!(recovered.saga(saga).unwrap().phase, SagaPhaseV1::Failed);
}
