use agora_client::v2::{PortFailureV2, ServicePortV2};
use agora_protocol::v2::{
    ClientIdV2, CommandV2, MessageIdV1, OutcomeV2, ReplyV2, RequestIdV2, RequestV2,
};
use moira_agora::{PublicationLimitsV1, durable::DurableResponsePublisher};
use moira_agora_publisher_sqlite::{PublisherSqliteStore, PublisherStoreLimitsV1};
use moira_client::{ModelSagaMutationClient, ModelSagaServicePort};
use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, DeliberationLimitsV1, DeliberationPlanV1, DeliberationV1, EnsembleMemberIdV1,
    EnsembleMemberV1, GroundingRefV1, MemberReportV1, MethodRefV1, OrthogonalityEvidenceV1,
    PersonaLensV1, ReportIdV1, SurfaceDepthV1,
};
use moira_core::model_saga::LimitsV1;
use moira_daimon::{
    ModelProposalV1, ModelResultV1,
    ensemble::{MemberClaimV1, MemberOutcomeV1, MemberResponseV1},
};
use moira_local_harness::{
    action_selection::select_single_member,
    peer_router::{AgoraRouteV1, ExternalConversationRefV1, FederatedPeerRouterV1},
    peer_turn::{PeerTurnOutcomeV1, execute_selected_peer_turn},
};
use moira_protocol::{
    DispositionV1, EffectIdV1, ResultReceiptV1, TaskRefV1, WorkflowIdV1, model_saga as saga,
};
use moira_service::model_saga::ModelSagaService;
use moira_sqlite::model_saga::{
    ModelSagaSqliteStore, ModelSagaStoreErrorV1, ModelSagaStoreLimitsV1,
};
use moira_vellum::EditAdapter;
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};
use vellum_protocol::{
    ClientId, DurableExecuteOutcomeV1, DurableExecuteRequestV1, DurableReceiptLookupOutcomeV1,
    DurableReceiptLookupV1, ReplyV1 as VellumReplyV1, RequestV1, ServiceAccessErrorV1,
    ServiceControlErrorV1, ServicePort,
};

struct SagaPort(ModelSagaService<ModelSagaSqliteStore>);

impl ModelSagaServicePort for SagaPort {
    type Error = ModelSagaStoreErrorV1;

    fn execute(&mut self, command: saga::CommandV1) -> Result<saga::ReplyV1, Self::Error> {
        self.0.execute(command)
    }
}

#[derive(Default)]
struct AgoraState {
    calls: usize,
    messages: usize,
    retained: BTreeMap<(ClientIdV2, RequestIdV2), (RequestV2, ReplyV2)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LostReply;

struct AgoraPort {
    state: Rc<RefCell<AgoraState>>,
    lose_next: bool,
}

impl ServicePortV2 for AgoraPort {
    type Error = LostReply;

    fn execute(
        &mut self,
        client: ClientIdV2,
        request: RequestV2,
    ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
        let mut state = self.state.borrow_mut();
        state.calls += 1;
        let key = (client, request.request_id);
        let reply = if let Some((retained, reply)) = state.retained.get(&key) {
            assert_eq!(retained, &request);
            reply.clone()
        } else {
            assert!(matches!(
                &request.command,
                CommandV2::SpeakIdentified {
                    room_id,
                    expected_revision: 4,
                    logical_time: 20,
                    body,
                    addressed_to,
                } if room_id == "chat-room"
                    && body == "answer"
                    && addressed_to == &["human".to_owned()]
            ));
            state.messages += 1;
            let reply = ReplyV2::Accepted(OutcomeV2::IdentifiedMessageAccepted {
                revision: 5,
                message_id: MessageIdV1::new([30; 16]),
                sequence: 6,
            });
            state.retained.insert(key, (request, reply.clone()));
            reply
        };
        if self.lose_next {
            self.lose_next = false;
            Err(PortFailureV2::OutcomeUnknown(LostReply))
        } else {
            Ok(reply)
        }
    }
}

struct UnusedVellumPort;

impl ServicePort for UnusedVellumPort {
    fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
        Ok(ClientId::new(1).unwrap())
    }

    fn execute(&mut self, _: RequestV1) -> Result<VellumReplyV1, ServiceAccessErrorV1> {
        unreachable!("chatbot journey has no Vellum operation")
    }

    fn disconnect(&mut self, _: ClientId) -> Result<(), ServiceControlErrorV1> {
        Ok(())
    }

    fn execute_durable(
        &mut self,
        _: DurableExecuteRequestV1,
    ) -> Result<DurableExecuteOutcomeV1, ServiceAccessErrorV1> {
        unreachable!("chatbot journey has no Vellum operation")
    }

    fn lookup_durable_receipt(
        &mut self,
        _: DurableReceiptLookupV1,
    ) -> Result<DurableReceiptLookupOutcomeV1, ServiceAccessErrorV1> {
        unreachable!("chatbot journey has no Vellum operation")
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moira-chatbot-peer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn outcome() -> (
    moira_core::deliberation::DeliberationCheckpointV1,
    MemberOutcomeV1<()>,
) {
    let member = EnsembleMemberIdV1([1; 16]);
    let report_id = ReportIdV1([2; 16]);
    let output = br#"{"schema":"moira.member-response.v1","claims":[{"kind":"empirical","statement":"answer","evidence_digests":[],"dissent_digests":[]}]}"#.to_vec();
    let report = MemberReportV1 {
        id: report_id,
        member,
        output_digest: Sha256::digest(&output).into(),
        evidence_digests: Vec::new(),
        dissent_digests: Vec::new(),
    };
    let plan = DeliberationPlanV1 {
        id: DeliberationIdV1([3; 16]),
        question_digest: [4; 32],
        method: MethodRefV1 {
            machine_digest: [5; 32],
            state_digest: [6; 32],
        },
        grounding_provenance: None,
        claims: vec![ClaimKindV1::Empirical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([7; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([8; 16]),
            scores: vec![1],
        }],
        admission: moira_core::deliberation::EnsembleAdmissionV1::affirmed(
            [14; 32], [15; 32], [16; 32], [17; 32], [18; 16],
        ),
        members: vec![EnsembleMemberV1 {
            id: member,
            stakeholder: moira_core::deliberation::StakeholderRefV1::technical_sme(
                member.0, [9; 32],
            ),
            lens: PersonaLensV1::Researcher,
            concern_root_digest: [9; 32],
            grounding: GroundingRefV1 {
                authority_digest: [10; 32],
                surface_digest: [11; 32],
                depth: SurfaceDepthV1::Ambient,
            },
        }],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [12; 32],
            result_digest: [13; 32],
            all_orthogonal: true,
        },
        fuel: 2,
    };
    let mut deliberation = DeliberationV1::try_new(plan, DeliberationLimitsV1::default()).unwrap();
    deliberation.submit_report(report.clone()).unwrap();
    let result = ModelResultV1 {
        receipt: ResultReceiptV1 {
            effect: EffectIdV1([14; 16]),
            execution_id: [15; 16],
            task: TaskRefV1 {
                external_task_key: [16; 16],
                incarnation: 17,
            },
            attempt_number: 18,
            disposition: DispositionV1::Completed,
        },
        evidence_digest: [19; 32],
        invocation_evidence: None,
        input_tokens: 1,
        output_tokens: 1,
        output,
        proposals: vec![ModelProposalV1::AgoraMessage {
            room: [20; 16],
            reply_to: [21; 16],
            body: b"answer".to_vec(),
        }],
    };
    (
        deliberation.checkpoint(),
        MemberOutcomeV1::Report {
            member,
            report,
            response: MemberResponseV1 {
                claims: vec![MemberClaimV1 {
                    kind: ClaimKindV1::Empirical,
                    statement: "answer".into(),
                    statement_digest: Sha256::digest(b"answer").into(),
                    evidence_digests: Vec::new(),
                    dissent_digests: Vec::new(),
                }],
            },
            result: Box::new(result),
        },
    )
}

#[test]
fn selected_chatbot_reply_crosses_durable_saga_and_agora_recovery_once() {
    let fixture = Fixture::new();
    let (checkpoint, outcome) = outcome();
    let selected = select_single_member::<_, ()>(&checkpoint, &outcome, [22; 32]).unwrap();
    let saga_limits = LimitsV1 {
        max_sagas: 4,
        max_proposals_per_saga: 4,
        max_proposal_bytes: 4_096,
    };
    let saga_store = ModelSagaSqliteStore::open(
        fixture.0.join("saga.db"),
        [23; 16],
        ModelSagaStoreLimitsV1 {
            codec: moira_core::model_saga::persistence::CodecLimitsV1 {
                max_bytes: 64 * 1_024,
                max_sagas: 4,
                max_proposals_per_saga: 4,
                max_proposal_bytes: 4_096,
            },
            saga: saga_limits,
            max_database_pages: 64,
        },
    )
    .unwrap();
    let mut saga_client = ModelSagaMutationClient::new(SagaPort(ModelSagaService::new(saga_store)));
    let publisher_store = PublisherSqliteStore::open(
        fixture.0.join("publisher.db"),
        [24; 16],
        ClientIdV2(1),
        PublisherStoreLimitsV1::default(),
    )
    .unwrap();
    let agora_state = Rc::new(RefCell::new(AgoraState::default()));
    let publisher = DurableResponsePublisher::new(
        publisher_store,
        AgoraPort {
            state: Rc::clone(&agora_state),
            lose_next: true,
        },
        PublicationLimitsV1::default(),
    )
    .unwrap();
    let editor = EditAdapter::connect(UnusedVellumPort, BTreeMap::new(), 1_024).unwrap();
    let mut router = FederatedPeerRouterV1::new(
        publisher,
        BTreeMap::from([(
            ExternalConversationRefV1 {
                room: [20; 16],
                reply_to: [21; 16],
            },
            AgoraRouteV1 {
                source: [25; 16],
                room_sequence: 1,
                room_id: "chat-room".into(),
                expected_agora_revision: 4,
                logical_time: 20,
                addressed_to: vec!["human".into()],
            },
        )]),
        editor,
    );
    let workflow = WorkflowIdV1([26; 16]);
    let first = execute_selected_peer_turn(
        &mut saga_client,
        &mut router,
        None,
        workflow,
        selected,
        |_, _, _| Ok::<_, ()>(0),
    )
    .unwrap();
    let PeerTurnOutcomeV1::Completed {
        storage_revision: 2,
        ref saga,
    } = first
    else {
        panic!("completed peer turn expected: {first:?}")
    };
    assert_ne!(saga.selection_digest, [0; 32]);
    assert_eq!(agora_state.borrow().calls, 2);
    assert_eq!(agora_state.borrow().messages, 1);

    let selected = select_single_member::<_, ()>(&checkpoint, &outcome, [22; 32]).unwrap();
    assert!(matches!(
        execute_selected_peer_turn(
            &mut saga_client,
            &mut router,
            Some(2),
            workflow,
            selected,
            |_, _, _| Ok::<_, ()>(0),
        )
        .unwrap(),
        PeerTurnOutcomeV1::Completed { .. }
    ));
    assert_eq!(agora_state.borrow().calls, 2);
    assert_eq!(agora_state.borrow().messages, 1);
}
