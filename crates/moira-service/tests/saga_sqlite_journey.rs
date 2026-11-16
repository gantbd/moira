#![cfg(unix)]

use moira_core::saga::{SagaLimitsV1, persistence::SagaCodecLimitsV1};
use moira_protocol::{
    AutomaticProgressV1, FenceCauseV1, OperatorActionIdV1, OperatorDispositionV1,
    OperatorOperationV1, SagaCommandV1, SagaEffectOutcomeV1, SagaMutationV1, SagaOutcomeV1,
    SagaReplyV1, SagaSourceV1, SagaTransitionV1,
};
use moira_service::saga::{OperatorAccessV1, SagaService};
use moira_sqlite::saga::{SagaSqliteStore, SagaStoreLimitsV1};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "moira-saga-service-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn limits() -> SagaStoreLimitsV1 {
    SagaStoreLimitsV1 {
        codec: SagaCodecLimitsV1 {
            max_bytes: 16_384,
            max_sagas: 8,
            max_operator_actions: 16,
        },
        saga: SagaLimitsV1 {
            max_sagas: 8,
            max_retries_per_saga: 2,
            max_inspection_page: 4,
            max_operator_actions: 16,
        },
        max_database_pages: 64,
    }
}

#[test]
fn native_saga_service_retains_transition_and_read_models_across_sqlite_reopen() {
    let path = path();
    let owner = [8; 16];
    let mutation = SagaMutationV1::Admit {
        source: SagaSourceV1([2; 16]),
        now: 10,
        deadline: 20,
        retry_budget: 2,
    };
    let mut service = SagaService::new(SagaSqliteStore::open(&path, owner, limits()).unwrap());
    let SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
        transition,
        replayed: false,
        ..
    }) = service
        .execute(SagaCommandV1::Apply {
            expected_revision: 0,
            mutation,
        })
        .unwrap()
    else {
        panic!("durable saga expected")
    };
    let SagaTransitionV1::Admitted { saga, effect, .. } = transition else {
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
    let authority = OperatorAccessV1 {
        subject: [3; 16],
        authority_decision: 4,
        authority_epoch: 5,
    };
    let reconciliation = SagaCommandV1::Reconcile {
        expected_revision: 2,
        operation: OperatorOperationV1 {
            action: OperatorActionIdV1([6; 16]),
            saga,
            disposition: OperatorDispositionV1::KeepUncertain,
        },
    };
    service
        .execute_operator_authorized(authority, reconciliation)
        .unwrap();
    drop(service);

    let mut reopened = SagaService::new(SagaSqliteStore::open(&path, owner, limits()).unwrap());
    assert!(matches!(
        reopened.execute(SagaCommandV1::Status).unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Status(status))
            if status.revision == 3 && !status.needs_reopen
    ));
    assert!(matches!(
        reopened.execute(SagaCommandV1::Saga { saga }).unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Saga(Some(view))) if view.saga == saga
    ));
    assert!(matches!(
        reopened.execute(SagaCommandV1::Census).unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Census(census))
            if census.total == 1 && census.fenced == 1
    ));
    assert!(matches!(
        reopened
            .execute(SagaCommandV1::Reconciliation { saga })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Reconciliation(Some(view)))
            if view.last_confirmed_revision == 3
                && view.automatic_progress == AutomaticProgressV1::BlockedUncertain
                && matches!(view.fence, Some(FenceCauseV1::EffectUncertain { effect: retained, observed_at: 11, .. }) if retained == effect)
    ));
    assert!(matches!(
        reopened
            .execute(SagaCommandV1::Evidence {
                saga,
                after: None,
                limit: 4,
            })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Evidence(page))
            if page.entries.len() == 1 && page.entries[0].effect == effect
    ));
    assert!(matches!(
        reopened
            .execute(SagaCommandV1::OperatorReceipt {
                action: OperatorActionIdV1([6; 16]),
            })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipt(Some(receipt)))
            if receipt.saga == saga
                && receipt.authority_decision == 4
                && receipt.authority_epoch == 5
                && receipt.disposition == OperatorDispositionV1::KeepUncertain
                && receipt.prepared.is_none()
    ));
    assert!(matches!(
        reopened
            .execute(SagaCommandV1::OperatorReceipts {
                after: None,
                limit: 4,
            })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipts(page))
            if page.entries.len() == 1 && page.next_after.is_none()
    ));
    assert!(matches!(
        reopened
            .execute_operator_authorized(authority, reconciliation)
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Applied { replayed: true, .. })
    ));
    assert!(matches!(
        reopened
            .execute(SagaCommandV1::Apply {
                expected_revision: 3,
                mutation,
            })
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Applied { replayed: true, .. })
    ));
    drop(reopened);
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}
