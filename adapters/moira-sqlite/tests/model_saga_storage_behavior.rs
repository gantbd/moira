use moira_core::{
    EffectId, WorkflowId,
    model_saga::{
        CommandV1, EffectStateV1, LimitsV1, PeerReceiptV1, PhaseV1, ProposalV1,
        persistence::CodecLimitsV1,
    },
};
use moira_sqlite::{
    StoreRevision,
    model_saga::{ModelSagaSqliteStore, ModelSagaStoreErrorV1, ModelSagaStoreLimitsV1},
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
            "moira-model-saga-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self { directory }
    }

    fn path(&self) -> PathBuf {
        self.directory.join("model-saga.db")
    }

    fn limits(&self) -> ModelSagaStoreLimitsV1 {
        ModelSagaStoreLimitsV1 {
            codec: CodecLimitsV1 {
                max_bytes: 65_536,
                max_sagas: 8,
                max_proposals_per_saga: 4,
                max_proposal_bytes: 4_096,
            },
            saga: LimitsV1 {
                max_sagas: 8,
                max_proposals_per_saga: 4,
                max_proposal_bytes: 4_096,
            },
            max_database_pages: 64,
        }
    }

    fn open(&self) -> ModelSagaSqliteStore {
        ModelSagaSqliteStore::open(self.path(), [9; 16], self.limits()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in [
            "model-saga.db",
            "model-saga.db-journal",
            "model-saga.db.owner.lock",
        ] {
            let _ = fs::remove_file(self.directory.join(name));
        }
        let _ = fs::remove_dir(&self.directory);
    }
}

fn workflow() -> WorkflowId {
    WorkflowId::new([1; 16])
}

fn daimon_effect() -> EffectId {
    EffectId::new([2; 16])
}

fn admit() -> CommandV1 {
    CommandV1::Admit {
        workflow: workflow(),
        daimon_effect: daimon_effect(),
        selection_digest: [9; 32],
        proposals: vec![
            ProposalV1::AgoraMessage {
                room: [3; 16],
                reply_to: [4; 16],
                body: b"hello".to_vec(),
            },
            ProposalV1::VellumEdit {
                workspace: [5; 16],
                document: [6; 16],
                expected_revision: 7,
                expected_extent: 8,
                replacement: b"replacement".to_vec(),
            },
        ],
    }
}

#[test]
fn model_saga_store_reopens_and_replays_without_duplicate_progress() {
    let fixture = Fixture::new();
    let first_effect = {
        let mut store = fixture.open();
        let admitted = store.apply(None, admit()).unwrap();
        assert!(!admitted.replayed);
        assert_eq!(admitted.storage_revision, StoreRevision(0));
        let effect = admitted.transition.saga.effects[0].effect;
        let accepted = store
            .apply(
                Some(StoreRevision(0)),
                CommandV1::Accept {
                    workflow: workflow(),
                    receipt: PeerReceiptV1::AgoraMessage {
                        effect,
                        room: [3; 16],
                        reply_to: [4; 16],
                        message: [10; 16],
                        agora_revision: 11,
                    },
                },
            )
            .unwrap();
        assert!(!accepted.replayed);
        assert_eq!(accepted.storage_revision, StoreRevision(1));
        effect
    };

    let mut reopened = fixture.open();
    let saga = reopened.saga(workflow()).unwrap();
    assert!(matches!(saga.effects[0].state, EffectStateV1::Succeeded(_)));
    assert_eq!(saga.effects[1].state, EffectStateV1::Pending);
    let replay = reopened
        .apply(
            Some(StoreRevision(1)),
            CommandV1::Accept {
                workflow: workflow(),
                receipt: PeerReceiptV1::AgoraMessage {
                    effect: first_effect,
                    room: [3; 16],
                    reply_to: [4; 16],
                    message: [10; 16],
                    agora_revision: 11,
                },
            },
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.storage_revision, StoreRevision(1));
}

#[test]
fn uncertain_effect_and_queued_tail_survive_reopen() {
    let fixture = Fixture::new();
    let pending = {
        let mut store = fixture.open();
        let admitted = store.apply(None, admit()).unwrap();
        let effect = admitted.transition.saga.effects[0].effect;
        store
            .apply(
                Some(StoreRevision(0)),
                CommandV1::MarkUncertain {
                    workflow: workflow(),
                    effect,
                },
            )
            .unwrap();
        effect
    };
    let mut reopened = fixture.open();
    let saga = reopened.saga(workflow()).unwrap();
    assert_eq!(saga.phase, PhaseV1::FencedUncertain { effect: pending });
    assert_eq!(saga.effects[0].state, EffectStateV1::Uncertain);
    assert_eq!(saga.effects[1].state, EffectStateV1::Queued);
    let reconciled = reopened
        .apply(
            Some(StoreRevision(1)),
            CommandV1::Reconcile {
                workflow: workflow(),
                receipt: PeerReceiptV1::AgoraMessage {
                    effect: pending,
                    room: [3; 16],
                    reply_to: [4; 16],
                    message: [12; 16],
                    agora_revision: 13,
                },
            },
        )
        .unwrap();
    assert_eq!(reconciled.storage_revision, StoreRevision(2));
    assert_eq!(reconciled.transition.saga.phase, PhaseV1::Pending);
    assert_eq!(
        reconciled.transition.saga.effects[1].state,
        EffectStateV1::Pending
    );
}

#[test]
fn profile_drift_and_foreign_databases_are_refused() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    store.apply(None, admit()).unwrap();
    drop(store);

    let mut changed = fixture.limits();
    changed.saga.max_proposals_per_saga = 3;
    assert!(matches!(
        ModelSagaSqliteStore::open(fixture.path(), [9; 16], changed),
        Err(ModelSagaStoreErrorV1::InvalidConfig)
    ));

    let foreign = fixture.directory.join("foreign.db");
    let connection = rusqlite::Connection::open(&foreign).unwrap();
    connection
        .execute("CREATE TABLE foreign_state(value INTEGER)", [])
        .unwrap();
    drop(connection);
    assert!(matches!(
        ModelSagaSqliteStore::open(&foreign, [9; 16], fixture.limits()),
        Err(ModelSagaStoreErrorV1::ForeignDatabase)
    ));
    for name in ["foreign.db", "foreign.db-journal", "foreign.db.owner.lock"] {
        let _ = fs::remove_file(fixture.directory.join(name));
    }
}
