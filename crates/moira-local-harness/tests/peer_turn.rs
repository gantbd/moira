use moira_client::{ModelSagaMutationClient, ModelSagaServicePort};
use moira_core::model_saga::LimitsV1;
use moira_daimon::{ModelProposalV1, ModelResultV1};
use moira_local_harness::peer_turn::{
    PeerEffectOutcomeV1, PeerEffectPortV1, PeerProjectionErrorV1, PeerReconcileOutcomeV1,
    PeerTurnErrorV1, PeerTurnOutcomeV1, execute_peer_turn, project_peer_proposals,
    resume_peer_turn,
};
use moira_protocol::{
    DispositionV1, EffectIdV1, ResultReceiptV1, TaskRefV1, WorkflowIdV1, model_saga as saga,
};
use moira_service::model_saga::ModelSagaService;
use moira_sqlite::model_saga::{
    ModelSagaSqliteStore, ModelSagaStoreErrorV1, ModelSagaStoreLimitsV1,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Port(ModelSagaService<ModelSagaSqliteStore>);

impl ModelSagaServicePort for Port {
    type Error = ModelSagaStoreErrorV1;

    fn execute(&mut self, command: saga::CommandV1) -> Result<saga::ReplyV1, Self::Error> {
        self.0.execute(command)
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moira-peer-turn-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn port(&self) -> Port {
        let limits = LimitsV1 {
            max_sagas: 4,
            max_proposals_per_saga: 4,
            max_proposal_bytes: 4_096,
        };
        let store = ModelSagaSqliteStore::open(
            self.0.join("peer.db"),
            [1; 16],
            ModelSagaStoreLimitsV1 {
                codec: moira_core::model_saga::persistence::CodecLimitsV1 {
                    max_bytes: 64 * 1_024,
                    max_sagas: 4,
                    max_proposals_per_saga: 4,
                    max_proposal_bytes: 4_096,
                },
                saga: limits,
                max_database_pages: 64,
            },
        )
        .unwrap();
        Port(ModelSagaService::new(store))
    }

    fn client(&self) -> ModelSagaMutationClient<Port> {
        ModelSagaMutationClient::new(self.port())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn result() -> ModelResultV1 {
    ModelResultV1 {
        receipt: ResultReceiptV1 {
            effect: EffectIdV1([2; 16]),
            execution_id: [3; 16],
            task: TaskRefV1 {
                external_task_key: [4; 16],
                incarnation: 5,
            },
            attempt_number: 6,
            disposition: DispositionV1::Completed,
        },
        evidence_digest: [7; 32],
        invocation_evidence: None,
        input_tokens: 8,
        output_tokens: 9,
        output: b"answer".to_vec(),
        proposals: vec![
            ModelProposalV1::AgoraMessage {
                room: [10; 16],
                reply_to: [11; 16],
                body: b"reply".to_vec(),
            },
            ModelProposalV1::ToolCall {
                request: [12; 16],
                capability: [13; 16],
                provider_correlation: Vec::new(),
                arguments: Vec::new(),
            },
            ModelProposalV1::VellumEdit {
                workspace: [14; 16],
                document: [15; 16],
                expected_revision: 16,
                replacement: b"replacement".to_vec(),
            },
        ],
    }
}

struct Peers {
    execute_calls: usize,
    reconcile_calls: usize,
    reconciliation_ready: bool,
}

impl PeerEffectPortV1 for Peers {
    type Error = ();

    fn execute(
        &mut self,
        effect: &saga::ProposalEffectV1,
    ) -> Result<PeerEffectOutcomeV1, Self::Error> {
        self.execute_calls += 1;
        Ok(match &effect.proposal {
            saga::ProposalV1::AgoraMessage { room, reply_to, .. } => {
                PeerEffectOutcomeV1::Succeeded(saga::PeerReceiptV1::AgoraMessage {
                    effect: effect.effect,
                    room: *room,
                    reply_to: *reply_to,
                    message: [17; 16],
                    agora_revision: 18,
                })
            }
            saga::ProposalV1::VellumEdit { .. } => PeerEffectOutcomeV1::Uncertain,
        })
    }

    fn reconcile(
        &mut self,
        effect: &saga::ProposalEffectV1,
    ) -> Result<PeerReconcileOutcomeV1, Self::Error> {
        self.reconcile_calls += 1;
        if !self.reconciliation_ready {
            return Ok(PeerReconcileOutcomeV1::Uncertain);
        }
        let saga::ProposalV1::VellumEdit {
            workspace,
            document,
            ..
        } = effect.proposal
        else {
            panic!("only the uncertain Vellum effect may be reconciled")
        };
        Ok(PeerReconcileOutcomeV1::Succeeded(
            saga::PeerReceiptV1::VellumEdit {
                effect: effect.effect,
                workspace,
                document,
                source_revision: 17,
                durable: true,
            },
        ))
    }
}

#[test]
fn projection_requires_trusted_extent_and_excludes_tool_calls() {
    let mut requests = Vec::new();
    let projected = project_peer_proposals(&result(), |workspace, document, revision| {
        requests.push((workspace, document, revision));
        Ok::<_, ()>(42)
    })
    .unwrap();

    assert_eq!(requests, vec![([14; 16], [15; 16], 16)]);
    assert_eq!(projected.len(), 2);
    assert!(matches!(
        projected[1],
        saga::ProposalV1::VellumEdit {
            expected_extent: 42,
            ..
        }
    ));
}

#[test]
fn projection_rejects_a_noncompleted_model_result_before_resolving_routes() {
    let mut result = result();
    result.receipt.disposition = DispositionV1::Failed;
    let mut called = false;

    assert_eq!(
        project_peer_proposals(&result, |_, _, _| {
            called = true;
            Ok::<_, ()>(42)
        }),
        Err(PeerProjectionErrorV1::NonCompleted)
    );
    assert!(!called);
}

#[test]
fn recovery_never_dispatches_without_a_saga_read_from_the_store() {
    let fixture = Fixture::new();
    let mut port = fixture.port();
    let mut peers = Peers {
        execute_calls: 0,
        reconcile_calls: 0,
        reconciliation_ready: true,
    };

    assert!(matches!(
        resume_peer_turn(&mut port, &mut peers, WorkflowIdV1([99; 16])),
        Err(PeerTurnErrorV1::InvalidState)
    ));
    assert_eq!(peers.execute_calls, 0);
    assert_eq!(peers.reconcile_calls, 0);
}

#[test]
fn uncertain_effect_is_reconciled_without_reexecution_and_completion_replays_quietly() {
    let fixture = Fixture::new();
    let result = result();
    let proposals = project_peer_proposals(&result, |_, _, _| Ok::<_, ()>(42)).unwrap();
    let mut client = fixture.client();
    let mut peers = Peers {
        execute_calls: 0,
        reconcile_calls: 0,
        reconciliation_ready: false,
    };
    let workflow = WorkflowIdV1([19; 16]);

    let first = execute_peer_turn(
        &mut client,
        &mut peers,
        None,
        workflow,
        [20; 32],
        &result,
        proposals.clone(),
    )
    .unwrap();
    let PeerTurnOutcomeV1::Uncertain {
        storage_revision: 2,
        ..
    } = first
    else {
        panic!("uncertain outcome expected: {first:?}")
    };
    assert_eq!(peers.execute_calls, 2);
    assert_eq!(peers.reconcile_calls, 1);

    drop(client);
    let mut reopened = fixture.port();
    peers.reconciliation_ready = true;
    let second = resume_peer_turn(&mut reopened, &mut peers, workflow).unwrap();
    let PeerTurnOutcomeV1::Completed {
        storage_revision: 3,
        ref saga,
    } = second
    else {
        panic!("completed outcome expected: {second:?}")
    };
    assert_eq!(saga.selection_digest, [20; 32]);
    let mut client = ModelSagaMutationClient::new(reopened);
    assert_eq!(peers.execute_calls, 2);
    assert_eq!(peers.reconcile_calls, 2);

    let replay = execute_peer_turn(
        &mut client,
        &mut peers,
        Some(3),
        workflow,
        [20; 32],
        &result,
        proposals,
    )
    .unwrap();
    assert!(matches!(replay, PeerTurnOutcomeV1::Completed { .. }));
    assert_eq!(peers.execute_calls, 2);
    assert_eq!(peers.reconcile_calls, 2);
}
