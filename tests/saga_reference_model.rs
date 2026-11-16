use moira_core::saga::*;
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum Forward {
    Succeed,
    FailRetry,
    Uncertain,
}
#[derive(Clone, Copy)]
enum Terminal {
    Succeed,
    FailRetry,
    FailCancel,
    DeadlineCompensate,
}
#[derive(Clone, Copy)]
enum Compensation {
    Succeed,
    Fail,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Expected {
    phase: SagaPhaseV1,
    retries: u8,
}

/// Independent phase model for the selected closed saga family. It knows no
/// engine identities, transitions, checkpoints or implementation helpers.
fn reference(forward: Forward, terminal: Terminal, compensation: Compensation) -> Expected {
    match forward {
        Forward::Uncertain => Expected {
            phase: SagaPhaseV1::FencedUncertain,
            retries: 0,
        },
        Forward::FailRetry => Expected {
            phase: SagaPhaseV1::Completed,
            retries: 1,
        },
        Forward::Succeed => match terminal {
            Terminal::Succeed => Expected {
                phase: SagaPhaseV1::Completed,
                retries: 0,
            },
            Terminal::FailRetry => Expected {
                phase: SagaPhaseV1::Completed,
                retries: 1,
            },
            Terminal::FailCancel | Terminal::DeadlineCompensate => Expected {
                phase: match compensation {
                    Compensation::Succeed => SagaPhaseV1::Cancelled,
                    Compensation::Fail => SagaPhaseV1::CompensationFailed,
                    Compensation::Uncertain => SagaPhaseV1::FencedUncertain,
                },
                retries: 0,
            },
        },
    }
}

fn apply_recover_replay(engine: &mut SagaEngineV1, command: SagaCommandV1) -> SagaTransitionV1 {
    let before = engine.revision();
    let transition = engine.apply(before, command).unwrap();
    assert_eq!(engine.revision(), before + 1);
    *engine = SagaEngineV1::recover(engine.checkpoint()).unwrap_or_else(|error| {
        panic!(
            "recovery failed at revision {} with {:?}: {error:?}",
            engine.revision(),
            engine.saga(SagaIdV1(0))
        )
    });
    assert_eq!(engine.apply(engine.revision(), command), Ok(transition));
    assert_eq!(engine.revision(), before + 1);
    transition
}

fn effect(transition: SagaTransitionV1) -> SagaEffectIdV1 {
    match transition {
        SagaTransitionV1::Admitted { effect, .. }
        | SagaTransitionV1::EffectPrepared { effect, .. } => effect,
        SagaTransitionV1::Cancelled {
            compensation: Some(effect),
            ..
        }
        | SagaTransitionV1::DeadlineApplied {
            compensation: Some(effect),
            ..
        } => effect,
        _ => panic!("transition did not prepare an effect"),
    }
}

#[test]
fn closed_saga_family_matches_independent_model_across_restart_and_lost_reply() {
    let forwards = [Forward::Succeed, Forward::FailRetry, Forward::Uncertain];
    let terminals = [
        Terminal::Succeed,
        Terminal::FailRetry,
        Terminal::FailCancel,
        Terminal::DeadlineCompensate,
    ];
    let compensations = [
        Compensation::Succeed,
        Compensation::Fail,
        Compensation::Uncertain,
    ];
    for forward in forwards {
        for terminal in terminals {
            for compensation in compensations {
                let expected = reference(forward, terminal, compensation);
                let mut engine = SagaEngineV1::try_new(SagaLimitsV1 {
                    max_sagas: 1,
                    max_retries_per_saga: 1,
                    max_inspection_page: 1,
                    max_operator_actions: 4,
                })
                .unwrap();
                let admitted = apply_recover_replay(
                    &mut engine,
                    SagaCommandV1::Admit {
                        source: [1; 16],
                        now: 1,
                        deadline: 10,
                        retry_budget: 1,
                    },
                );
                let SagaTransitionV1::Admitted { saga, timer, .. } = admitted else {
                    unreachable!()
                };
                let mut current = effect(admitted);
                let mut identities = BTreeSet::from([current]);

                match forward {
                    Forward::Uncertain => {
                        apply_recover_replay(
                            &mut engine,
                            SagaCommandV1::AcceptEffect {
                                saga,
                                effect: current,
                                outcome: EffectOutcomeV1::Uncertain,
                                observed_at: 3,
                            },
                        );
                    }
                    Forward::FailRetry => {
                        apply_recover_replay(
                            &mut engine,
                            SagaCommandV1::AcceptEffect {
                                saga,
                                effect: current,
                                outcome: EffectOutcomeV1::Failed,
                                observed_at: 3,
                            },
                        );
                        current = effect(apply_recover_replay(
                            &mut engine,
                            SagaCommandV1::Retry {
                                saga,
                                failed_effect: current,
                                now: 4,
                            },
                        ));
                        assert!(identities.insert(current));
                        apply_recover_replay(
                            &mut engine,
                            SagaCommandV1::AcceptEffect {
                                saga,
                                effect: current,
                                outcome: EffectOutcomeV1::Succeeded,
                                observed_at: 5,
                            },
                        );
                        current = effect(apply_recover_replay(
                            &mut engine,
                            SagaCommandV1::PrepareTerminal { saga, now: 6 },
                        ));
                        assert!(identities.insert(current));
                        apply_recover_replay(
                            &mut engine,
                            SagaCommandV1::AcceptEffect {
                                saga,
                                effect: current,
                                outcome: EffectOutcomeV1::Succeeded,
                                observed_at: 7,
                            },
                        );
                    }
                    Forward::Succeed => {
                        apply_recover_replay(
                            &mut engine,
                            SagaCommandV1::AcceptEffect {
                                saga,
                                effect: current,
                                outcome: EffectOutcomeV1::Succeeded,
                                observed_at: 3,
                            },
                        );
                        current = effect(apply_recover_replay(
                            &mut engine,
                            SagaCommandV1::PrepareTerminal { saga, now: 4 },
                        ));
                        assert!(identities.insert(current));
                        match terminal {
                            Terminal::Succeed => {
                                apply_recover_replay(
                                    &mut engine,
                                    SagaCommandV1::AcceptEffect {
                                        saga,
                                        effect: current,
                                        outcome: EffectOutcomeV1::Succeeded,
                                        observed_at: 5,
                                    },
                                );
                            }
                            Terminal::FailRetry => {
                                apply_recover_replay(
                                    &mut engine,
                                    SagaCommandV1::AcceptEffect {
                                        saga,
                                        effect: current,
                                        outcome: EffectOutcomeV1::Failed,
                                        observed_at: 5,
                                    },
                                );
                                current = effect(apply_recover_replay(
                                    &mut engine,
                                    SagaCommandV1::Retry {
                                        saga,
                                        failed_effect: current,
                                        now: 6,
                                    },
                                ));
                                assert!(identities.insert(current));
                                apply_recover_replay(
                                    &mut engine,
                                    SagaCommandV1::AcceptEffect {
                                        saga,
                                        effect: current,
                                        outcome: EffectOutcomeV1::Succeeded,
                                        observed_at: 7,
                                    },
                                );
                            }
                            Terminal::FailCancel => {
                                apply_recover_replay(
                                    &mut engine,
                                    SagaCommandV1::AcceptEffect {
                                        saga,
                                        effect: current,
                                        outcome: EffectOutcomeV1::Failed,
                                        observed_at: 5,
                                    },
                                );
                                current = effect(apply_recover_replay(
                                    &mut engine,
                                    SagaCommandV1::Cancel { saga, now: 6 },
                                ));
                                assert!(identities.insert(current));
                                accept_compensation(&mut engine, saga, current, compensation);
                            }
                            Terminal::DeadlineCompensate => {
                                current = effect(apply_recover_replay(
                                    &mut engine,
                                    SagaCommandV1::FireDeadline {
                                        saga,
                                        timer,
                                        observed_at: 10,
                                        action: DeadlineActionV1::Compensate,
                                    },
                                ));
                                assert!(identities.insert(current));
                                accept_compensation(&mut engine, saga, current, compensation);
                            }
                        }
                    }
                }
                let actual = engine.saga(saga).unwrap();
                assert_eq!(actual.phase, expected.phase);
                assert_eq!(actual.retries_used, expected.retries);
            }
        }
    }
}

fn accept_compensation(
    engine: &mut SagaEngineV1,
    saga: SagaIdV1,
    effect: SagaEffectIdV1,
    value: Compensation,
) {
    apply_recover_replay(
        engine,
        SagaCommandV1::AcceptEffect {
            saga,
            effect,
            outcome: match value {
                Compensation::Succeed => EffectOutcomeV1::Succeeded,
                Compensation::Fail => EffectOutcomeV1::Failed,
                Compensation::Uncertain => EffectOutcomeV1::Uncertain,
            },
            observed_at: 11,
        },
    );
}
