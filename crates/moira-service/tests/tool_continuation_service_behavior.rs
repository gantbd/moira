#![cfg(unix)]

use moira_core::tool_continuation::{LimitsV1, persistence::CodecLimitsV1};
use moira_protocol::{
    EffectIdV1, WorkflowIdV1,
    tool_continuation::{
        AttemptEvidenceV1, AuthorityDecisionIdV1, AuthorityEvidenceV1, AuthoritySubjectIdV1,
        CommandV1, MutationV1, OutcomeV1, PhaseV1, RefusalV1, ReplyV1, ToolBudgetV1, ToolCallV1,
        ToolCapabilityIdV1, ToolEffectStateV1, ToolExecutionIdV1, ToolGateV1, ToolReceiptV1,
        ToolRequestIdV1,
    },
};
use moira_service::{AccessV1, tool_continuation::ToolContinuationService};
use moira_sqlite::tool_continuation::{ToolContinuationSqliteStore, ToolContinuationStoreLimitsV1};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

fn limits() -> ToolContinuationStoreLimitsV1 {
    let saga = LimitsV1 {
        max_sagas: 2,
        max_calls_per_saga: 2,
        max_argument_bytes: 4_096,
        max_elapsed_millis: 10_000,
        max_output_bytes: 4_096,
    };
    ToolContinuationStoreLimitsV1 {
        codec: CodecLimitsV1 {
            max_bytes: 65_536,
            domain: saga,
        },
        saga,
        max_database_pages: 64,
    }
}

fn path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "moira-tool-service-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn cleanup(path: &Path) {
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

fn gate() -> ToolGateV1 {
    ToolGateV1 {
        authority: AuthorityEvidenceV1 {
            decision: AuthorityDecisionIdV1([5; 16]),
            subject: AuthoritySubjectIdV1([6; 16]),
            capability: ToolCapabilityIdV1([4; 16]),
            epoch: 7,
        },
        attempt: AttemptEvidenceV1 {
            task_key: [8; 16],
            incarnation: 9,
            attempt: 1,
        },
    }
}

#[test]
fn public_contract_preserves_authorization_attempt_and_replay_across_reopen() {
    let path = path();
    let owner = [1; 16];
    let workflow = WorkflowIdV1([2; 16]);
    let admit = CommandV1::Apply {
        expected_storage_revision: None,
        mutation: MutationV1::Admit {
            workflow,
            source_execution: EffectIdV1([3; 16]),
            source_attempt: gate().attempt,
            calls: vec![ToolCallV1 {
                request: ToolRequestIdV1([10; 16]),
                capability: ToolCapabilityIdV1([4; 16]),
                provider_correlation: b"call-1".to_vec(),
                arguments: b"native".to_vec(),
                budget: ToolBudgetV1 {
                    max_elapsed_millis: 100,
                    max_output_bytes: 1_024,
                },
            }],
        },
    };
    let store = ToolContinuationSqliteStore::open(&path, owner, limits()).unwrap();
    let mut service = ToolContinuationService::new(store);
    assert_eq!(
        service
            .execute_authorized(
                AccessV1 {
                    read: true,
                    mutate: false,
                },
                admit.clone(),
            )
            .unwrap(),
        ReplyV1::Refused(RefusalV1::NotAuthorized)
    );
    let ReplyV1::Accepted(OutcomeV1::Applied { saga, .. }) = service.execute(admit).unwrap() else {
        panic!("admission expected")
    };
    let effect = saga.effects[0].effect;
    let mut wrong_gate = gate();
    wrong_gate.attempt.attempt = 2;
    assert_eq!(
        service
            .execute(CommandV1::Apply {
                expected_storage_revision: Some(0),
                mutation: MutationV1::Authorize {
                    workflow,
                    effect,
                    gate: wrong_gate,
                },
            })
            .unwrap(),
        ReplyV1::Refused(RefusalV1::InvalidEvidence)
    );
    service
        .execute(CommandV1::Apply {
            expected_storage_revision: Some(0),
            mutation: MutationV1::Authorize {
                workflow,
                effect,
                gate: gate(),
            },
        })
        .unwrap();
    let execution = ToolExecutionIdV1([11; 16]);
    service
        .execute(CommandV1::Apply {
            expected_storage_revision: Some(1),
            mutation: MutationV1::Dispatch {
                workflow,
                effect,
                execution,
            },
        })
        .unwrap();
    drop(service.into_store());

    let store = ToolContinuationSqliteStore::open(&path, owner, limits()).unwrap();
    let mut service = ToolContinuationService::new(store);
    let ReplyV1::Accepted(OutcomeV1::Saga(Some(retained))) =
        service.execute(CommandV1::Saga { workflow }).unwrap()
    else {
        panic!("retained saga expected")
    };
    assert!(matches!(
        retained.effects[0].state,
        ToolEffectStateV1::InFlight { gate: retained_gate, execution: retained_execution }
            if retained_gate == gate() && retained_execution == execution
    ));
    let receipt = ToolReceiptV1 {
        effect,
        capability: ToolCapabilityIdV1([4; 16]),
        execution,
        evidence_digest: [12; 32],
        output_digest: [13; 32],
        output_bytes: 16,
    };
    let complete = CommandV1::Apply {
        expected_storage_revision: Some(2),
        mutation: MutationV1::Accept { workflow, receipt },
    };
    let ReplyV1::Accepted(OutcomeV1::Applied {
        storage_revision,
        saga,
        replayed,
    }) = service.execute(complete).unwrap()
    else {
        panic!("completion expected")
    };
    assert_eq!(storage_revision, 3);
    assert_eq!(saga.phase, PhaseV1::Completed);
    assert!(!replayed);
    let ReplyV1::Accepted(OutcomeV1::Applied { replayed, .. }) = service
        .execute(CommandV1::Apply {
            expected_storage_revision: Some(3),
            mutation: MutationV1::Accept { workflow, receipt },
        })
        .unwrap()
    else {
        panic!("replay expected")
    };
    assert!(replayed);
    drop(service.into_store());

    let store = ToolContinuationSqliteStore::open(&path, owner, limits()).unwrap();
    let mut service = ToolContinuationService::new(store);
    assert!(matches!(
        service.execute(CommandV1::Saga { workflow }).unwrap(),
        ReplyV1::Accepted(OutcomeV1::Saga(Some(saga))) if saga.phase == PhaseV1::Completed
    ));
    drop(service.into_store());
    cleanup(&path);
}
