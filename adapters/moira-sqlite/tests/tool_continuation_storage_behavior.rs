#![cfg(unix)]

use moira_core::{
    EffectId, WorkflowId,
    tool_continuation::{
        AttemptEvidenceV1, AuthorityDecisionIdV1, AuthorityEvidenceV1, AuthoritySubjectIdV1,
        CommandV1, LimitsV1, PhaseV1, ToolBudgetV1, ToolCallV1, ToolCapabilityIdV1,
        ToolEffectStateV1, ToolExecutionIdV1, ToolGateV1, ToolReceiptV1, ToolRequestIdV1,
        persistence::CodecLimitsV1,
    },
};
use moira_sqlite::{
    StoreRevision,
    tool_continuation::{
        ToolContinuationSqliteStore, ToolContinuationStoreErrorV1, ToolContinuationStoreLimitsV1,
    },
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "moira-tool-continuation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self { directory }
    }

    fn path(&self) -> PathBuf {
        self.directory.join("tools.db")
    }

    fn limits(&self) -> ToolContinuationStoreLimitsV1 {
        let saga = LimitsV1 {
            max_sagas: 4,
            max_calls_per_saga: 4,
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

    fn open(&self) -> ToolContinuationSqliteStore {
        ToolContinuationSqliteStore::open(self.path(), [1; 16], self.limits()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["tools.db", "tools.db-journal", "tools.db.owner.lock"] {
            let _ = fs::remove_file(self.directory.join(name));
        }
        let _ = fs::remove_dir(&self.directory);
    }
}

fn call() -> ToolCallV1 {
    ToolCallV1 {
        request: ToolRequestIdV1([2; 16]),
        capability: ToolCapabilityIdV1([3; 16]),
        provider_correlation: b"call-1".to_vec(),
        arguments: b"bounded-native-arguments".to_vec(),
        budget: ToolBudgetV1 {
            max_elapsed_millis: 500,
            max_output_bytes: 1_024,
        },
    }
}

fn gate() -> ToolGateV1 {
    ToolGateV1 {
        authority: AuthorityEvidenceV1 {
            decision: AuthorityDecisionIdV1([4; 16]),
            subject: AuthoritySubjectIdV1([5; 16]),
            capability: ToolCapabilityIdV1([3; 16]),
            epoch: 6,
        },
        attempt: AttemptEvidenceV1 {
            task_key: [7; 16],
            incarnation: 8,
            attempt: 1,
        },
    }
}

#[test]
fn authority_attempt_inflight_and_receipt_survive_sqlite_reopen() {
    let fixture = Fixture::new();
    let workflow = WorkflowId::new([9; 16]);
    let execution = ToolExecutionIdV1([10; 16]);
    let (effect, receipt) = {
        let mut store = fixture.open();
        let admitted = store
            .apply(
                None,
                CommandV1::Admit {
                    workflow,
                    source_execution: EffectId::new([11; 16]),
                    source_attempt: gate().attempt,
                    calls: vec![call()],
                },
            )
            .unwrap();
        let effect = admitted.transition.saga.effects[0].effect;
        store
            .apply(
                Some(StoreRevision(0)),
                CommandV1::Authorize {
                    workflow,
                    effect,
                    gate: gate(),
                },
            )
            .unwrap();
        store
            .apply(
                Some(StoreRevision(1)),
                CommandV1::Dispatch {
                    workflow,
                    effect,
                    execution,
                },
            )
            .unwrap();
        (
            effect,
            ToolReceiptV1 {
                effect,
                capability: ToolCapabilityIdV1([3; 16]),
                execution,
                evidence_digest: [12; 32],
                output_digest: [13; 32],
                output_bytes: 16,
            },
        )
    };

    let mut reopened = fixture.open();
    assert_eq!(reopened.storage_revision(), Some(StoreRevision(2)));
    assert!(matches!(
        reopened.saga(workflow).unwrap().effects[0].state,
        ToolEffectStateV1::InFlight { gate: retained, execution: retained_execution }
            if retained == gate() && retained_execution == execution
    ));
    let applied = reopened
        .apply(
            Some(StoreRevision(2)),
            CommandV1::Accept { workflow, receipt },
        )
        .unwrap();
    assert_eq!(applied.transition.saga.phase, PhaseV1::Completed);
    assert!(!applied.replayed);
    let replay = reopened
        .apply(
            Some(StoreRevision(3)),
            CommandV1::Accept { workflow, receipt },
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.storage_revision, StoreRevision(3));
    drop(reopened);

    let reopened = fixture.open();
    assert_eq!(reopened.storage_revision(), Some(StoreRevision(3)));
    assert_eq!(reopened.saga(workflow).unwrap().phase, PhaseV1::Completed);
    assert!(matches!(
        reopened.saga(workflow).unwrap().effects[0].state,
        ToolEffectStateV1::Succeeded { receipt: retained, .. } if retained == receipt
    ));
    assert_eq!(effect, receipt.effect);
}

#[test]
fn stale_revision_owner_and_profile_drift_refuse_without_reinitializing() {
    let fixture = Fixture::new();
    let workflow = WorkflowId::new([20; 16]);
    let mut store = fixture.open();
    store
        .apply(
            None,
            CommandV1::Admit {
                workflow,
                source_execution: EffectId::new([21; 16]),
                source_attempt: gate().attempt,
                calls: vec![call()],
            },
        )
        .unwrap();
    assert!(matches!(
        store.apply(
            None,
            CommandV1::Admit {
                workflow,
                source_execution: EffectId::new([21; 16]),
                source_attempt: gate().attempt,
                calls: vec![call()],
            }
        ),
        Err(ToolContinuationStoreErrorV1::Conflict { .. })
    ));
    drop(store);

    assert!(matches!(
        ToolContinuationSqliteStore::open(fixture.path(), [99; 16], fixture.limits()),
        Err(ToolContinuationStoreErrorV1::OwnerMismatch)
    ));
    let mut changed = fixture.limits();
    changed.saga.max_calls_per_saga = 3;
    assert!(matches!(
        ToolContinuationSqliteStore::open(fixture.path(), [1; 16], changed),
        Err(ToolContinuationStoreErrorV1::InvalidConfig)
    ));
    assert_eq!(fixture.open().storage_revision(), Some(StoreRevision(0)));
}
