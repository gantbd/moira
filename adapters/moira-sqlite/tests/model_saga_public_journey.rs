use moira_client::{ModelSagaMutationClient, ModelSagaReadOnlyClient, ModelSagaServicePort};
use moira_core::model_saga::LimitsV1;
use moira_protocol::{EffectIdV1, WorkflowIdV1, model_saga as protocol};
use moira_service::{AccessV1, model_saga::ModelSagaService};
use moira_sqlite::model_saga::{
    ModelSagaSqliteStore, ModelSagaStoreErrorV1, ModelSagaStoreLimitsV1,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Port(ModelSagaService<ModelSagaSqliteStore>);

impl ModelSagaServicePort for Port {
    type Error = ModelSagaStoreErrorV1;

    fn execute(&mut self, command: protocol::CommandV1) -> Result<protocol::ReplyV1, Self::Error> {
        self.0.execute(command)
    }
}

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "moira-model-saga-public-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self { directory }
    }

    fn open(&self) -> ModelSagaSqliteStore {
        ModelSagaSqliteStore::open(
            self.directory.join("saga.db"),
            [8; 16],
            ModelSagaStoreLimitsV1 {
                codec: moira_core::model_saga::persistence::CodecLimitsV1 {
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
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["saga.db", "saga.db-journal", "saga.db.owner.lock"] {
            let _ = fs::remove_file(self.directory.join(name));
        }
        let _ = fs::remove_dir(&self.directory);
    }
}

fn admission() -> protocol::MutationV1 {
    protocol::MutationV1::Admit {
        workflow: WorkflowIdV1([1; 16]),
        daimon_effect: EffectIdV1([2; 16]),
        selection_digest: [3; 32],
        proposals: vec![protocol::ProposalV1::AgoraMessage {
            room: [3; 16],
            reply_to: [4; 16],
            body: b"public reply".to_vec(),
        }],
    }
}

#[test]
fn public_client_replays_exact_admission_after_sqlite_reopen() {
    let fixture = Fixture::new();
    let first = {
        let service = ModelSagaService::new(fixture.open());
        let mut client = ModelSagaMutationClient::new(Port(service));
        client.apply(None, admission()).unwrap()
    };
    let protocol::ReplyV1::Accepted(protocol::OutcomeV1::Applied {
        storage_revision: 0,
        saga: first_saga,
        replayed: false,
    }) = first
    else {
        panic!("unexpected first reply")
    };

    let service = ModelSagaService::new(fixture.open());
    let mut client = ModelSagaMutationClient::new(Port(service));
    let replay = client.apply(Some(0), admission()).unwrap();
    let protocol::ReplyV1::Accepted(protocol::OutcomeV1::Applied {
        storage_revision: 0,
        saga,
        replayed: true,
    }) = replay
    else {
        panic!("unexpected replay reply")
    };
    assert_eq!(saga, first_saga);

    let mut reader = ModelSagaReadOnlyClient::new(client.into_port());
    assert_eq!(
        reader.saga(WorkflowIdV1([1; 16])).unwrap(),
        protocol::ReplyV1::Accepted(protocol::OutcomeV1::Saga(Some(first_saga)))
    );
}

#[test]
fn authorization_and_storage_revision_conflict_refuse_without_mutation() {
    let fixture = Fixture::new();
    let mut service = ModelSagaService::new(fixture.open());
    let denied = service
        .execute_authorized(
            AccessV1 {
                read: true,
                mutate: false,
            },
            protocol::CommandV1::Apply {
                expected_storage_revision: None,
                mutation: admission(),
            },
        )
        .unwrap();
    assert_eq!(
        denied,
        protocol::ReplyV1::Refused(protocol::RefusalV1::NotAuthorized)
    );
    assert_eq!(
        service.execute(protocol::CommandV1::Status).unwrap(),
        protocol::ReplyV1::Accepted(protocol::OutcomeV1::Status(protocol::StatusV1 {
            storage_revision: None,
            needs_reopen: false,
        }))
    );

    let protocol::MutationV1::Admit {
        workflow,
        daimon_effect,
        proposals,
        ..
    } = admission()
    else {
        unreachable!()
    };
    assert_eq!(
        service
            .execute(protocol::CommandV1::Apply {
                expected_storage_revision: None,
                mutation: protocol::MutationV1::Admit {
                    workflow,
                    daimon_effect,
                    selection_digest: [0; 32],
                    proposals,
                },
            })
            .unwrap(),
        protocol::ReplyV1::Refused(protocol::RefusalV1::InvalidSelection)
    );

    service
        .execute(protocol::CommandV1::Apply {
            expected_storage_revision: None,
            mutation: admission(),
        })
        .unwrap();
    assert_eq!(
        service
            .execute(protocol::CommandV1::Apply {
                expected_storage_revision: None,
                mutation: admission(),
            })
            .unwrap(),
        protocol::ReplyV1::Refused(protocol::RefusalV1::StorageRevisionConflict {
            expected: None,
            actual: Some(0),
        })
    );
}
