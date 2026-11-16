use moira_core::saga::{
    EffectEvidencePageV1 as CoreEvidencePage, SagaCensusV1 as CoreCensus,
    SagaCommandV1 as CoreCommand, SagaEffectIdV1 as CoreEffectId, SagaEngineV1, SagaIdV1 as CoreId,
    SagaLimitsV1, SagaPageV1 as CorePage, SagaReconciliationViewV1 as CoreReconciliation,
    SagaRefusalV1 as CoreRefusal, SagaSnapshotV1 as CoreSnapshot,
};
use moira_host::{SagaApplyErrorV1, SagaApplyReceiptV1, SagaStore};
use moira_protocol::{
    ConfirmedReconciliationV1, OperatorActionIdV1, OperatorDispositionV1, OperatorOperationV1,
    ReconciliationProposalV1, SagaCommandV1, SagaEffectOutcomeV1, SagaIdV1, SagaMutationV1,
    SagaOutcomeV1, SagaRefusalV1, SagaReplyV1, SagaSourceV1, SagaTransitionV1,
};
use moira_service::{
    AccessV1,
    saga::{OperatorAccessV1, SagaService},
};
use std::time::{Duration, Instant};

struct Store {
    engine: SagaEngineV1,
    calls: u64,
}

impl Store {
    fn new() -> Self {
        Self {
            engine: SagaEngineV1::try_new(SagaLimitsV1 {
                max_sagas: 4,
                max_retries_per_saga: 2,
                max_inspection_page: 2,
                max_operator_actions: 8,
            })
            .unwrap(),
            calls: 0,
        }
    }
}

impl SagaStore for Store {
    type Error = ();

    fn revision(&self) -> u64 {
        self.engine.revision()
    }

    fn needs_reopen(&self) -> bool {
        false
    }

    fn saga(&self, saga: CoreId) -> Option<CoreSnapshot> {
        self.engine.saga(saga)
    }

    fn inspect_page(&self, after: Option<CoreId>, limit: usize) -> Result<CorePage, CoreRefusal> {
        self.engine.inspect_page(after, limit)
    }

    fn census(&self) -> CoreCensus {
        self.engine.census()
    }

    fn reconciliation_view(&self, saga: CoreId) -> Option<CoreReconciliation> {
        self.engine.reconciliation_view(saga)
    }

    fn evidence_page(
        &self,
        saga: CoreId,
        after: Option<CoreEffectId>,
        limit: usize,
    ) -> Result<CoreEvidencePage, CoreRefusal> {
        self.engine.evidence_page(saga, after, limit)
    }

    fn operator_receipt(
        &self,
        action: moira_core::saga::OperatorActionIdV1,
    ) -> Option<moira_core::saga::OperatorReceiptSummaryV1> {
        self.engine.operator_receipt(action)
    }

    fn operator_receipt_page(
        &self,
        after: Option<moira_core::saga::OperatorActionIdV1>,
        limit: usize,
    ) -> Result<moira_core::saga::OperatorReceiptPageV1, CoreRefusal> {
        self.engine.operator_receipt_page(after, limit)
    }

    fn apply(
        &mut self,
        expected_revision: u64,
        command: CoreCommand,
    ) -> Result<SagaApplyReceiptV1, SagaApplyErrorV1<Self::Error>> {
        self.calls += 1;
        let before = self.engine.revision();
        let transition = self
            .engine
            .apply(expected_revision, command)
            .map_err(SagaApplyErrorV1::Domain)?;
        Ok(SagaApplyReceiptV1 {
            storage_revision: self.engine.revision().saturating_sub(1),
            transition,
            replayed: before == self.engine.revision(),
        })
    }
}

fn admission() -> SagaMutationV1 {
    SagaMutationV1::Admit {
        source: SagaSourceV1([1; 16]),
        now: 10,
        deadline: 20,
        retry_budget: 2,
    }
}

#[test]
fn saga_service_preserves_replay_refusal_and_bounded_read_models() {
    let mut service = SagaService::new(Store::new());
    let SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
        transition,
        replayed: false,
        ..
    }) = service
        .execute(SagaCommandV1::Apply {
            expected_revision: 0,
            mutation: admission(),
        })
        .unwrap()
    else {
        panic!("new saga expected")
    };
    let SagaTransitionV1::Admitted { saga, .. } = transition else {
        panic!("admission expected")
    };
    assert!(matches!(
        service.execute(SagaCommandV1::Status).unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Status(status))
            if status.revision == 1 && !status.needs_reopen
    ));
    assert!(matches!(
        service.execute(SagaCommandV1::Saga { saga }).unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Saga(Some(view))) if view.saga == saga
    ));
    assert!(matches!(
        service
            .execute(SagaCommandV1::Inspect {
                after: None,
                limit: 2,
            })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Page(page)) if page.entries.len() == 1
    ));
    assert!(matches!(
        service.execute(SagaCommandV1::Census).unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Census(census))
            if census.total == 1 && census.runnable_effects == 1
    ));
    assert!(matches!(
        service
            .execute(SagaCommandV1::Reconciliation { saga })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Reconciliation(Some(view)))
            if view.last_confirmed_revision == 1
    ));
    assert!(matches!(
        service
            .execute(SagaCommandV1::Evidence {
                saga,
                after: None,
                limit: 2,
            })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Evidence(page)) if page.entries.is_empty()
    ));
    assert!(matches!(
        service
            .execute(SagaCommandV1::Apply {
                expected_revision: 1,
                mutation: admission(),
            })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Applied { replayed: true, .. })
    ));
    assert_eq!(
        service
            .execute(SagaCommandV1::Apply {
                expected_revision: 0,
                mutation: admission(),
            })
            .unwrap(),
        SagaReplyV1::Refused(SagaRefusalV1::RevisionConflict {
            expected: 0,
            actual: 1,
        })
    );
    assert_eq!(
        service
            .execute(SagaCommandV1::Inspect {
                after: Some(SagaIdV1(0)),
                limit: 3,
            })
            .unwrap(),
        SagaReplyV1::Refused(SagaRefusalV1::InvalidPageLimit)
    );
}

#[test]
fn saga_authorization_refuses_before_store_dispatch() {
    let mut service = SagaService::new(Store::new());
    assert_eq!(
        service
            .execute_authorized(
                AccessV1 {
                    read: true,
                    mutate: false,
                },
                SagaCommandV1::Apply {
                    expected_revision: 0,
                    mutation: admission(),
                },
            )
            .unwrap(),
        SagaReplyV1::Refused(SagaRefusalV1::NotAuthorized)
    );
    assert_eq!(service.into_store().calls, 0);
}

#[test]
fn operator_reconciliation_requires_verified_aegis_facts_and_replays_exactly() {
    let mut service = SagaService::new(Store::new());
    let SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
        transition: SagaTransitionV1::Admitted { saga, effect, .. },
        ..
    }) = service
        .execute(SagaCommandV1::Apply {
            expected_revision: 0,
            mutation: admission(),
        })
        .unwrap()
    else {
        panic!("admission expected")
    };
    service
        .execute(SagaCommandV1::Apply {
            expected_revision: 1,
            mutation: SagaMutationV1::AcceptEffect {
                saga,
                effect,
                outcome: SagaEffectOutcomeV1::Uncertain,
                observed_at: 11,
            },
        })
        .unwrap();
    let proposal = ReconciliationProposalV1 {
        saga,
        disposition: OperatorDispositionV1::KeepUncertain,
    };
    let SagaReplyV1::Accepted(SagaOutcomeV1::ReconciliationConfirmation(Some(confirmation))) =
        service
            .execute(SagaCommandV1::ReconciliationConfirmation { proposal })
            .unwrap()
    else {
        panic!("confirmation expected")
    };
    let selected = ConfirmedReconciliationV1 {
        storage_revision: confirmation.storage_revision,
        saga,
        disposition: proposal.disposition,
        action: confirmation.action,
        confirmation_digest: confirmation.confirmation_digest,
    };
    let (expected_revision, prepared) = service.prepare_confirmed_reconciliation(selected).unwrap();
    assert_eq!(expected_revision, 2);
    assert_eq!(prepared.action, confirmation.action);
    assert_eq!(prepared.saga, saga);
    let operation = OperatorOperationV1 {
        action: OperatorActionIdV1([7; 16]),
        saga,
        disposition: OperatorDispositionV1::KeepUncertain,
    };
    let command = SagaCommandV1::Reconcile {
        expected_revision: 2,
        operation,
    };
    assert_eq!(
        service.execute(command).unwrap(),
        SagaReplyV1::Refused(SagaRefusalV1::NotAuthorized)
    );
    let store = service.into_store();
    assert_eq!(store.engine.revision(), 2);
    let mut service = SagaService::new(store);
    let authority = OperatorAccessV1 {
        subject: [8; 16],
        authority_decision: 9,
        authority_epoch: 10,
    };
    assert!(matches!(
        service
            .execute_operator_authorized(authority, command)
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            transition: SagaTransitionV1::Reconciled { .. },
            replayed: false,
            ..
        })
    ));
    assert!(matches!(
        service
            .execute_operator_authorized(authority, command)
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Applied { replayed: true, .. })
    ));
    assert!(matches!(
        service
            .execute(SagaCommandV1::OperatorReceipt {
                action: operation.action,
            })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipt(Some(receipt)))
            if receipt.action == operation.action
                && receipt.saga == saga
                && receipt.authority_decision == 9
                && receipt.authority_epoch == 10
                && receipt.disposition == OperatorDispositionV1::KeepUncertain
                && receipt.prepared.is_none()
    ));
    assert!(matches!(
        service
            .execute(SagaCommandV1::OperatorReceipts {
                after: None,
                limit: 2,
            })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipts(page))
            if page.entries.len() == 1 && page.next_after.is_none()
    ));
    assert_eq!(
        service
            .execute_authorized(
                AccessV1 {
                    read: false,
                    mutate: true,
                },
                SagaCommandV1::OperatorReceipt {
                    action: operation.action,
                },
            )
            .unwrap(),
        SagaReplyV1::Refused(SagaRefusalV1::NotAuthorized)
    );
    assert_eq!(
        service
            .execute_operator_authorized(
                OperatorAccessV1 {
                    subject: [11; 16],
                    ..authority
                },
                command,
            )
            .unwrap(),
        SagaReplyV1::Refused(SagaRefusalV1::OperatorActionConflict)
    );
}

#[test]
#[ignore = "opt-in deterministic closed-saga service soak"]
fn five_thousand_saga_lifecycles_preserve_effect_identity_and_exact_replay() {
    for cycle in 0_u64..5_000 {
        let mut service = SagaService::new(Store::new());
        let command = SagaCommandV1::Apply {
            expected_revision: 0,
            mutation: admission(),
        };
        let SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            transition: SagaTransitionV1::Admitted { saga, effect, .. },
            ..
        }) = service.execute(command).unwrap()
        else {
            panic!("cycle {cycle}: admission expected")
        };
        service
            .execute(SagaCommandV1::Apply {
                expected_revision: 1,
                mutation: SagaMutationV1::AcceptEffect {
                    saga,
                    effect,
                    outcome: SagaEffectOutcomeV1::Failed,
                    observed_at: 11,
                },
            })
            .unwrap();
        let SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            transition:
                SagaTransitionV1::EffectPrepared {
                    effect: retry_effect,
                    ..
                },
            ..
        }) = service
            .execute(SagaCommandV1::Apply {
                expected_revision: 2,
                mutation: SagaMutationV1::Retry {
                    saga,
                    failed_effect: effect,
                    now: 12,
                },
            })
            .unwrap()
        else {
            panic!("cycle {cycle}: retry effect expected")
        };
        service
            .execute(SagaCommandV1::Apply {
                expected_revision: 3,
                mutation: SagaMutationV1::AcceptEffect {
                    saga,
                    effect: retry_effect,
                    outcome: SagaEffectOutcomeV1::Succeeded,
                    observed_at: 13,
                },
            })
            .unwrap();
        let SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            transition:
                SagaTransitionV1::EffectPrepared {
                    effect: terminal_effect,
                    ..
                },
            ..
        }) = service
            .execute(SagaCommandV1::Apply {
                expected_revision: 4,
                mutation: SagaMutationV1::PrepareTerminal { saga, now: 14 },
            })
            .unwrap()
        else {
            panic!("cycle {cycle}: terminal effect expected")
        };
        service
            .execute(SagaCommandV1::Apply {
                expected_revision: 5,
                mutation: SagaMutationV1::AcceptEffect {
                    saga,
                    effect: terminal_effect,
                    outcome: SagaEffectOutcomeV1::Succeeded,
                    observed_at: 15,
                },
            })
            .unwrap();
        assert!(
            matches!(
                service
                    .execute(SagaCommandV1::Apply {
                        expected_revision: 6,
                        mutation: admission(),
                    })
                    .unwrap(),
                SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
                    transition: SagaTransitionV1::Admitted {
                        effect: replayed_effect,
                        ..
                    },
                    replayed: true,
                    ..
                }) if replayed_effect == effect
            ),
            "cycle {cycle}"
        );
    }
}

fn percentile(samples: &mut [Duration], percent: usize) -> Duration {
    assert!(!samples.is_empty());
    assert!((1..=100).contains(&percent));
    samples.sort_unstable();
    samples[(samples.len() * percent).div_ceil(100) - 1]
}

fn report(operation: &str, samples: &mut [Duration]) {
    let p50 = percentile(samples, 50);
    let p95 = percentile(samples, 95);
    let p99 = percentile(samples, 99);
    println!(
        "operation={operation} count={} p50_ns={} p95_ns={} p99_ns={} max_ns={}",
        samples.len(),
        p50.as_nanos(),
        p95.as_nanos(),
        p99.as_nanos(),
        samples.last().unwrap().as_nanos()
    );
    assert!(
        p95 <= Duration::from_millis(10),
        "{operation}: provisional p95 engineering budget exceeded"
    );
    assert!(
        p99 <= Duration::from_millis(25),
        "{operation}: provisional p99 engineering budget exceeded"
    );
}

#[test]
#[ignore = "explicit optimized reference-host performance acceptance gate"]
fn local_candidate_performance_gate() {
    if cfg!(debug_assertions) {
        panic!("qualification requires --release");
    }
    const SAMPLES: usize = 64;
    println!(
        "profile=in_memory_saga_service samples={SAMPLES} os={} arch={} optimized=true p95_budget_ms=10 p99_budget_ms=25",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let mut admissions = Vec::with_capacity(SAMPLES);
    let mut reads = Vec::with_capacity(SAMPLES);
    let mut accepts = Vec::with_capacity(SAMPLES);
    let mut retries = Vec::with_capacity(SAMPLES);

    for _ in 0..SAMPLES {
        let mut service = SagaService::new(Store::new());
        let started = Instant::now();
        let SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            transition: SagaTransitionV1::Admitted { saga, effect, .. },
            ..
        }) = service
            .execute(SagaCommandV1::Apply {
                expected_revision: 0,
                mutation: admission(),
            })
            .unwrap()
        else {
            panic!("admission expected")
        };
        admissions.push(started.elapsed());

        let started = Instant::now();
        assert!(matches!(
            service.execute(SagaCommandV1::Saga { saga }).unwrap(),
            SagaReplyV1::Accepted(SagaOutcomeV1::Saga(Some(_)))
        ));
        reads.push(started.elapsed());

        let started = Instant::now();
        assert!(matches!(
            service
                .execute(SagaCommandV1::Apply {
                    expected_revision: 1,
                    mutation: SagaMutationV1::AcceptEffect {
                        saga,
                        effect,
                        outcome: SagaEffectOutcomeV1::Failed,
                        observed_at: 11,
                    },
                })
                .unwrap(),
            SagaReplyV1::Accepted(SagaOutcomeV1::Applied { .. })
        ));
        accepts.push(started.elapsed());

        let started = Instant::now();
        assert!(matches!(
            service
                .execute(SagaCommandV1::Apply {
                    expected_revision: 2,
                    mutation: SagaMutationV1::Retry {
                        saga,
                        failed_effect: effect,
                        now: 12,
                    },
                })
                .unwrap(),
            SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
                transition: SagaTransitionV1::EffectPrepared { .. },
                ..
            })
        ));
        retries.push(started.elapsed());
    }

    report("admit", &mut admissions);
    report("saga_read", &mut reads);
    report("accept_effect", &mut accepts);
    report("retry", &mut retries);
}
