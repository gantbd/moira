use agora_client::v2::{PortFailureV2, ServicePortV2};
use agora_protocol::v2::{
    ClientIdV2, CommandV2, MessageIdV1, OutcomeV2, ReplyV2, RequestIdV2, RequestV2,
};
use chronos_client::WireClientV1;
use chronos_protocol as chronos;
use daimon_chronos::{ChronosAttemptValidator, TaskBindingV1};
use daimon_client::ModelServicePort;
use daimon_host::{
    AssembledModelInputV1, AuthenticatedSessionV1, AuthorityGrantV1, AuthorityRefusalV1,
    ContextAssembler, ContextRefusalV1, ExecutionAuthorizer, ModelProvider, ModelProviderOutcomeV1,
    ProfileRefusalV1, ProviderProfileResolver,
};
use daimon_protocol as daimon;
use daimon_service::{ServiceLimitsV1, model::ModelService};
use moira_agora::{
    PublicationIntentV1, PublicationLimitsV1, PublicationReplyV1, ResponsePublisher,
};
use moira_daimon::{
    ExecutionIntentV1, ModelExecutionAdapter, ModelExecutionReplyV1, ModelProposalV1,
};
use moira_protocol::{EffectIdV1, TaskRefV1, WorkflowIdV1, model_saga as saga};
use moira_service::model_saga::ModelSagaService;
use moira_sqlite::model_saga::{ModelSagaSqliteStore, ModelSagaStoreLimitsV1};
use moira_vellum::{
    DocumentRouteV1, EditAdapter, EditIntentV1, EditReplyV1, ExternalDocumentRefV1,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    convert::Infallible,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};
use vellum_protocol::{
    ClientId, DurableEffectIntentV1, DurableEffectReceiptV1, DurableExecuteOutcomeV1,
    DurableExecuteRequestV1, DurableReceiptLookupOutcomeV1, DurableReceiptLookupV1,
    DurableReplaceAndSaveReceiptV1, EditPersistenceV1, ReplyV1 as VellumReplyV1, RequestV1,
    SaveApplicationV1, SaveReceiptV1, SaveSynchronizationV1, ServiceAccessErrorV1,
    ServiceControlErrorV1, ServicePort, WorkspaceDocumentIdV1, WorkspaceIdV1,
};

const WORKFLOW: WorkflowIdV1 = WorkflowIdV1([40; 16]);
const DAIMON_EFFECT: EffectIdV1 = EffectIdV1([41; 16]);

struct Allow;

impl ExecutionAuthorizer for Allow {
    fn authorize(
        &mut self,
        _session: &AuthenticatedSessionV1,
        request: &daimon::ExecuteRequestV1,
        _now: u64,
    ) -> Result<AuthorityGrantV1, AuthorityRefusalV1> {
        Ok(AuthorityGrantV1 {
            decision_id: [42; 16],
            expires_at: 100,
            tools: request.tools.clone(),
        })
    }
}

struct ChronosPort {
    calls: Rc<Cell<usize>>,
}

impl WireClientV1 for ChronosPort {
    type Error = Infallible;

    fn request(&mut self, request: chronos::Request) -> Result<chronos::Response, Self::Error> {
        self.calls.set(self.calls.get() + 1);
        let chronos::Command::Snapshot { task } = request.command else {
            panic!("snapshot expected")
        };
        Ok(chronos::Response {
            version: 1,
            outcome: chronos::Outcome::Snapshot {
                value: Some(chronos::Entry {
                    reference: chronos::Reference {
                        task,
                        incarnation: "7".into(),
                    },
                    snapshot: chronos::Snapshot {
                        phase: chronos::Phase::Running,
                        attempt_number: Some(3),
                        dependency_count: 0,
                        required_capability_count: 0,
                        priority: 0,
                        max_attempts: 4,
                    },
                }),
                revision: Some(chronos::Revision::Finite("9".into())),
            },
        })
    }
}

struct Profile;

impl ProviderProfileResolver for Profile {
    fn resolve_profile(
        &mut self,
        _session: &AuthenticatedSessionV1,
        request: &daimon::ExecuteRequestV1,
    ) -> Result<daimon::ProviderProfileSnapshotV1, ProfileRefusalV1> {
        Ok(daimon::ProviderProfileSnapshotV1 {
            provider: daimon::ProviderProfileIdV1::new([43; 16]),
            harness_profile: request.profile,
            wire: daimon::ProviderWireProfileV1::OpenAiChatCompletions,
            model: b"local-model".to_vec(),
            structured_tools: false,
            document_digest: daimon::ContentDigestV1::new([44; 32]),
        })
    }
}

struct Context;

impl ContextAssembler for Context {
    fn assemble(
        &mut self,
        _session: &AuthenticatedSessionV1,
        request: &daimon::ExecuteRequestV1,
        _profile: &daimon::ProviderProfileSnapshotV1,
    ) -> Result<AssembledModelInputV1, ContextRefusalV1> {
        Ok(AssembledModelInputV1 {
            context: vec![daimon::ContextEvidenceV1 {
                reference: request.context[0],
                kind: daimon::ContextKindV1::AgoraMessage,
                bytes: b"question".to_vec(),
                source_digest: daimon::ContentDigestV1::new([45; 32]),
                grounding: None,
            }],
            messages: vec![daimon::ModelMessageV1 {
                role: daimon::MessageRoleV1::User,
                content: b"question".to_vec(),
            }],
            estimated_input_tokens: 2,
        })
    }
}

struct Provider {
    calls: Rc<Cell<usize>>,
}

impl ModelProvider for Provider {
    fn execute_model(&mut self, invocation: &daimon::ModelInvocationV1) -> ModelProviderOutcomeV1 {
        self.calls.set(self.calls.get() + 1);
        ModelProviderOutcomeV1::Receipt(daimon::ModelProviderReceiptV1 {
            execution_id: invocation.execution_id,
            provider_document_digest: invocation.provider.document_digest,
            evidence_digest: daimon::ContentDigestV1::new([46; 32]),
            disposition: daimon::ExecutionDispositionV1::Completed,
            input_tokens: 2,
            output_tokens: 4,
            output: b"answer".to_vec(),
            proposals: vec![
                daimon::OutputProposalV1::AgoraMessage(daimon::AgoraMessageProposalV1 {
                    room: [47; 16],
                    reply_to: [48; 16],
                    body: b"answer".to_vec(),
                }),
                daimon::OutputProposalV1::VellumEdit(daimon::VellumEditProposalV1 {
                    workspace: [49; 16],
                    document: [50; 16],
                    expected_revision: 7,
                    replacement: b"replacement".to_vec(),
                }),
            ],
        })
    }
}

struct ModelPort(
    ModelService<Allow, ChronosAttemptValidator<ChronosPort>, Profile, Context, Provider>,
);

impl ModelServicePort for ModelPort {
    type Error = Infallible;

    fn execute_model(
        &mut self,
        session: daimon::SessionHandleV1,
        command: daimon::ModelCommandV1,
    ) -> Result<daimon::ModelReplyV1, Self::Error> {
        Ok(self.0.execute(session, command))
    }
}

fn model_intent() -> ExecutionIntentV1 {
    ExecutionIntentV1 {
        effect: DAIMON_EFFECT,
        execution_id: [51; 16],
        task: TaskRefV1 {
            external_task_key: [52; 16],
            incarnation: 7,
        },
        attempt_number: 3,
        principal: [53; 16],
        profile: [54; 16],
        idempotency_key: [55; 16],
        context: vec![[56; 16]],
        tools: vec![],
        budget: daimon::ExecutionBudgetV1 {
            max_elapsed_millis: 1_000,
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 1,
            max_context_bytes: 1_024,
            max_output_bytes: 1_024,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AgoraError {
    LostReply,
}

struct AgoraPort {
    lose_next: bool,
    calls: usize,
    retained: BTreeMap<(ClientIdV2, RequestIdV2), (RequestV2, ReplyV2)>,
    messages: usize,
}

impl ServicePortV2 for AgoraPort {
    type Error = AgoraError;

    fn execute(
        &mut self,
        client: ClientIdV2,
        request: RequestV2,
    ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
        self.calls += 1;
        let key = (client, request.request_id);
        let reply = if let Some((retained, reply)) = self.retained.get(&key) {
            assert_eq!(retained, &request);
            reply.clone()
        } else {
            assert!(matches!(
                &request.command,
                CommandV2::SpeakIdentified { room_id, body, .. }
                    if room_id == "room" && body == "answer"
            ));
            self.messages += 1;
            let reply = ReplyV2::Accepted(OutcomeV2::IdentifiedMessageAccepted {
                revision: 5,
                message_id: MessageIdV1::new([57; 16]),
                sequence: 6,
            });
            self.retained.insert(key, (request.clone(), reply.clone()));
            reply
        };
        if self.lose_next {
            self.lose_next = false;
            Err(PortFailureV2::OutcomeUnknown(AgoraError::LostReply))
        } else {
            Ok(reply)
        }
    }
}

#[derive(Default)]
struct VellumCalls {
    executions: Vec<DurableExecuteRequestV1>,
    lookups: Vec<DurableReceiptLookupV1>,
}

struct VellumPort {
    calls: Rc<RefCell<VellumCalls>>,
    outcome: DurableExecuteOutcomeV1,
    lose_execute_reply: bool,
}

impl ServicePort for VellumPort {
    fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
        Ok(ClientId::new(1).unwrap())
    }

    fn execute(&mut self, _request: RequestV1) -> Result<VellumReplyV1, ServiceAccessErrorV1> {
        unreachable!("durable path only")
    }

    fn disconnect(&mut self, _client: ClientId) -> Result<(), ServiceControlErrorV1> {
        Ok(())
    }

    fn execute_durable(
        &mut self,
        request: DurableExecuteRequestV1,
    ) -> Result<DurableExecuteOutcomeV1, ServiceAccessErrorV1> {
        self.calls.borrow_mut().executions.push(request);
        if self.lose_execute_reply {
            self.lose_execute_reply = false;
            Err(ServiceAccessErrorV1::OutcomeUnknown)
        } else {
            Ok(self.outcome.clone())
        }
    }

    fn lookup_durable_receipt(
        &mut self,
        lookup: DurableReceiptLookupV1,
    ) -> Result<DurableReceiptLookupOutcomeV1, ServiceAccessErrorV1> {
        self.calls.borrow_mut().lookups.push(lookup);
        Ok(DurableReceiptLookupOutcomeV1::Complete(
            self.outcome.clone(),
        ))
    }
}

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "moira-model-federation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self { directory }
    }

    fn open(&self) -> ModelSagaSqliteStore {
        ModelSagaSqliteStore::open(
            self.directory.join("saga.db"),
            [58; 16],
            ModelSagaStoreLimitsV1 {
                codec: moira_core::model_saga::persistence::CodecLimitsV1 {
                    max_bytes: 65_536,
                    max_sagas: 4,
                    max_proposals_per_saga: 4,
                    max_proposal_bytes: 4_096,
                },
                saga: moira_core::model_saga::LimitsV1 {
                    max_sagas: 4,
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

fn apply(
    service: &mut ModelSagaService<ModelSagaSqliteStore>,
    revision: Option<u64>,
    mutation: saga::MutationV1,
) -> (u64, saga::SagaViewV1, bool) {
    let saga::ReplyV1::Accepted(saga::OutcomeV1::Applied {
        storage_revision,
        saga,
        replayed,
    }) = service
        .execute(saga::CommandV1::Apply {
            expected_storage_revision: revision,
            mutation,
        })
        .unwrap()
    else {
        panic!("model saga mutation refused")
    };
    (storage_revision, saga, replayed)
}

#[test]
fn chronos_gated_model_and_peer_reply_loss_do_not_duplicate_cross_engine_effects() {
    let provider_calls = Rc::new(Cell::new(0));
    let chronos_calls = Rc::new(Cell::new(0));
    let attempts = ChronosAttemptValidator::new(
        ChronosPort {
            calls: Rc::clone(&chronos_calls),
        },
        chronos::Limits {
            max_frame_bytes: 4_096,
            max_id_bytes: 64,
            max_graph_tasks: 4,
            max_dependencies_per_task: 4,
            max_capabilities_per_task: 4,
            max_capability_bytes: 64,
            max_page_entries: 4,
        },
        1,
        vec![TaskBindingV1 {
            external_task_key: [52; 16],
            chronos_task_id: "model-task".into(),
        }],
    )
    .unwrap();
    let mut model_service = ModelService::new(
        daimon_harness::Limits::default(),
        ServiceLimitsV1::default(),
        Allow,
        attempts,
        Profile,
        Context,
        Provider {
            calls: Rc::clone(&provider_calls),
        },
    )
    .unwrap();
    let session = model_service
        .open_session(AuthenticatedSessionV1 {
            principal: [53; 16],
            authority_context: [59; 16],
        })
        .unwrap();
    let mut model = ModelExecutionAdapter::new(session, ModelPort(model_service));
    let first = model.execute(model_intent(), 10).unwrap();
    assert_eq!(model.execute(model_intent(), 10).unwrap(), first);
    let ModelExecutionReplyV1::Result(result) = first else {
        panic!("model result expected: {first:?}")
    };
    assert_eq!(provider_calls.get(), 1);
    assert_eq!(chronos_calls.get(), 1);
    assert_eq!(result.evidence_digest, [46; 32]);

    let proposals = result
        .proposals
        .into_iter()
        .filter_map(|proposal| match proposal {
            ModelProposalV1::AgoraMessage {
                room,
                reply_to,
                body,
            } => Some(saga::ProposalV1::AgoraMessage {
                room,
                reply_to,
                body,
            }),
            ModelProposalV1::VellumEdit {
                workspace,
                document,
                expected_revision,
                replacement,
            } => Some(saga::ProposalV1::VellumEdit {
                workspace,
                document,
                expected_revision,
                expected_extent: 12,
                replacement,
            }),
            ModelProposalV1::ToolCall { .. } => None,
        })
        .collect();
    let fixture = Fixture::new();
    let mut saga_service = ModelSagaService::new(fixture.open());
    let (revision, admitted, replayed) = apply(
        &mut saga_service,
        None,
        saga::MutationV1::Admit {
            workflow: WORKFLOW,
            daimon_effect: DAIMON_EFFECT,
            selection_digest: [61; 32],
            proposals,
        },
    );
    assert_eq!(revision, 0);
    assert!(!replayed);
    let agora_effect = admitted.effects[0].effect;
    let vellum_effect = admitted.effects[1].effect;
    assert_ne!(agora_effect, vellum_effect);

    let mut publisher = ResponsePublisher::new(
        ClientIdV2(1),
        AgoraPort {
            lose_next: true,
            calls: 0,
            retained: BTreeMap::new(),
            messages: 0,
        },
        PublicationLimitsV1::default(),
    )
    .unwrap();
    let publication = PublicationIntentV1 {
        effect: agora_effect,
        source: [60; 16],
        room_sequence: 1,
        room_id: "room".into(),
        expected_agora_revision: 4,
        logical_time: 10,
        body: "answer".into(),
        addressed_to: vec!["owner".into()],
    };
    assert!(publisher.publish(publication.clone()).is_err());
    let state = publisher.state();
    let (port, _, _) = publisher.into_parts();
    let mut publisher =
        ResponsePublisher::restore(port, PublicationLimitsV1::default(), state).unwrap();
    let PublicationReplyV1::Published(published) = publisher.publish(publication).unwrap() else {
        panic!("Agora publication expected")
    };
    let (agora_port, _, _) = publisher.into_parts();
    assert_eq!(agora_port.calls, 2);
    assert_eq!(agora_port.messages, 1);
    let (revision, advanced, _) = apply(
        &mut saga_service,
        Some(0),
        saga::MutationV1::Accept {
            workflow: WORKFLOW,
            receipt: saga::PeerReceiptV1::AgoraMessage {
                effect: agora_effect,
                room: [47; 16],
                reply_to: [48; 16],
                message: published.message_id.into_bytes(),
                agora_revision: published.receipt.agora_revision,
            },
        },
    );
    assert_eq!(revision, 1);
    assert_eq!(advanced.effects[1].state, saga::EffectStateV1::Pending);

    let workspace = WorkspaceIdV1::new(1).unwrap();
    let document = WorkspaceDocumentIdV1::new(workspace, 2).unwrap();
    let target = ExternalDocumentRefV1 {
        workspace: [49; 16],
        document: [50; 16],
    };
    let save = SaveReceiptV1 {
        application: SaveApplicationV1::Written,
        synchronization: SaveSynchronizationV1::Succeeded,
        no_future_effects: true,
        quiescent: true,
        adapter_failure: false,
        cleanup_failed: false,
        baseline_published: true,
        outcome_unknown: false,
    };
    let outcome = DurableExecuteOutcomeV1::Completed(
        DurableEffectReceiptV1::DocumentReplacedAndSaved(DurableReplaceAndSaveReceiptV1 {
            document,
            source_revision: 8,
            text: Arc::from("replacement"),
            persistence: EditPersistenceV1::Durable(save),
        }),
    );
    let vellum_calls = Rc::new(RefCell::new(VellumCalls::default()));
    let routes = BTreeMap::from([(
        target,
        DocumentRouteV1 {
            workspace,
            document,
        },
    )]);
    let edit = EditIntentV1 {
        effect: vellum_effect,
        target,
        expected_revision: 7,
        expected_extent: 12,
        replacement: b"replacement".to_vec(),
    };
    let mut editor = EditAdapter::connect(
        VellumPort {
            calls: Rc::clone(&vellum_calls),
            outcome: outcome.clone(),
            lose_execute_reply: true,
        },
        routes.clone(),
        1_024,
    )
    .unwrap();
    assert_eq!(editor.execute(&edit).unwrap(), EditReplyV1::OutcomeUnknown);
    let (revision, fenced, _) = apply(
        &mut saga_service,
        Some(1),
        saga::MutationV1::MarkUncertain {
            workflow: WORKFLOW,
            effect: vellum_effect,
        },
    );
    assert_eq!(revision, 2);
    assert_eq!(
        fenced.phase,
        saga::PhaseV1::FencedUncertain {
            effect: vellum_effect
        }
    );
    drop(saga_service);

    let mut editor = EditAdapter::connect(
        VellumPort {
            calls: Rc::clone(&vellum_calls),
            outcome,
            lose_execute_reply: false,
        },
        routes,
        1_024,
    )
    .unwrap();
    let EditReplyV1::Completed { receipt, .. } = editor.lookup(&edit).unwrap() else {
        panic!("historical Vellum receipt expected")
    };
    let calls = vellum_calls.borrow();
    assert_eq!(calls.executions.len(), 1);
    assert_eq!(calls.lookups.len(), 1);
    assert_eq!(calls.executions[0].operation.to_bytes(), vellum_effect.0);
    assert_eq!(calls.lookups[0].operation.to_bytes(), vellum_effect.0);
    assert!(matches!(
        calls.executions[0].intent,
        DurableEffectIntentV1::ReplaceDocumentAndSave { .. }
    ));
    drop(calls);

    let mut reopened = ModelSagaService::new(fixture.open());
    let (revision, completed, replayed) = apply(
        &mut reopened,
        Some(2),
        saga::MutationV1::Reconcile {
            workflow: WORKFLOW,
            receipt: saga::PeerReceiptV1::VellumEdit {
                effect: vellum_effect,
                workspace: [49; 16],
                document: [50; 16],
                source_revision: receipt.source_revision,
                durable: true,
            },
        },
    );
    assert_eq!(revision, 3);
    assert!(!replayed);
    assert_eq!(completed.phase, saga::PhaseV1::Completed);
    let (same_revision, same, replayed) = apply(
        &mut reopened,
        Some(3),
        saga::MutationV1::Reconcile {
            workflow: WORKFLOW,
            receipt: saga::PeerReceiptV1::VellumEdit {
                effect: vellum_effect,
                workspace: [49; 16],
                document: [50; 16],
                source_revision: receipt.source_revision,
                durable: true,
            },
        },
    );
    assert_eq!(same_revision, 3);
    assert!(replayed);
    assert_eq!(same, completed);
}
