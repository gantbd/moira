use agora_client::v2::{PortFailureV2, ServicePortV2};
use agora_protocol::v2::{
    ClientIdV2, CommandV2, MessageIdV1, OutcomeV2, ReplyV2, RequestIdV2, RequestV2,
};
use moira_agora::{
    PublicationLimitsV1,
    durable::{
        DurableResponsePublisher, PublisherCheckpointV1, PublisherRecordV1, PublisherStateStoreV1,
    },
};
use moira_local_harness::{
    peer_router::{AgoraRouteV1, ExternalConversationRefV1, FederatedPeerRouterV1},
    peer_turn::{PeerEffectOutcomeV1, PeerEffectPortV1, PeerReconcileOutcomeV1},
};
use moira_protocol::{EffectIdV1, model_saga as saga};
use moira_vellum::{DocumentRouteV1, EditAdapter, ExternalDocumentRefV1};
use std::{cell::RefCell, collections::BTreeMap, convert::Infallible, rc::Rc, sync::Arc};
use vellum_protocol::{
    ClientId, DurableEffectIntentV1, DurableEffectReceiptV1, DurableExecuteOutcomeV1,
    DurableExecuteRequestV1, DurableReceiptLookupOutcomeV1, DurableReceiptLookupV1,
    DurableReplaceAndSaveReceiptV1, EditPersistenceV1, ReplyV1 as VellumReplyV1, RequestV1,
    SaveApplicationV1, SaveReceiptV1, SaveSynchronizationV1, ServiceAccessErrorV1,
    ServiceControlErrorV1, ServicePort, WorkspaceDocumentIdV1, WorkspaceIdV1,
};

#[derive(Clone)]
struct StateStore(PublisherRecordV1);

impl PublisherStateStoreV1 for StateStore {
    type Error = Infallible;

    fn load(&mut self) -> Result<PublisherRecordV1, Self::Error> {
        Ok(self.0.clone())
    }

    fn compare_exchange(
        &mut self,
        expected_revision: u64,
        checkpoint: &PublisherCheckpointV1,
    ) -> Result<PublisherRecordV1, Self::Error> {
        assert_eq!(self.0.revision, expected_revision);
        self.0 = PublisherRecordV1 {
            revision: expected_revision + 1,
            checkpoint: checkpoint.clone(),
        };
        Ok(self.0.clone())
    }
}

#[derive(Default)]
struct AgoraCalls {
    calls: usize,
    messages: usize,
    retained: BTreeMap<(ClientIdV2, RequestIdV2), (RequestV2, ReplyV2)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Lost;

struct AgoraPort {
    calls: Rc<RefCell<AgoraCalls>>,
    lose_next: bool,
}

impl ServicePortV2 for AgoraPort {
    type Error = Lost;

    fn execute(
        &mut self,
        client: ClientIdV2,
        request: RequestV2,
    ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
        let mut calls = self.calls.borrow_mut();
        calls.calls += 1;
        let key = (client, request.request_id);
        let reply = if let Some((retained, reply)) = calls.retained.get(&key) {
            assert_eq!(retained, &request);
            reply.clone()
        } else {
            assert!(matches!(
                &request.command,
                CommandV2::SpeakIdentified {
                    room_id,
                    expected_revision: 4,
                    logical_time: 12,
                    body,
                    addressed_to,
                } if room_id == "deployment-room"
                    && body == "answer"
                    && addressed_to == &["owner".to_owned()]
            ));
            calls.messages += 1;
            let reply = ReplyV2::Accepted(OutcomeV2::IdentifiedMessageAccepted {
                revision: 5,
                message_id: MessageIdV1::new([6; 16]),
                sequence: 7,
            });
            calls.retained.insert(key, (request, reply.clone()));
            reply
        };
        if self.lose_next {
            self.lose_next = false;
            Err(PortFailureV2::OutcomeUnknown(Lost))
        } else {
            Ok(reply)
        }
    }
}

#[derive(Default)]
struct VellumCalls {
    executions: usize,
    lookups: usize,
}

struct VellumPort {
    calls: Rc<RefCell<VellumCalls>>,
    outcome: DurableExecuteOutcomeV1,
    lose_next: bool,
}

impl ServicePort for VellumPort {
    fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
        Ok(ClientId::new(1).unwrap())
    }

    fn execute(&mut self, _: RequestV1) -> Result<VellumReplyV1, ServiceAccessErrorV1> {
        unreachable!("durable operations only")
    }

    fn disconnect(&mut self, _: ClientId) -> Result<(), ServiceControlErrorV1> {
        Ok(())
    }

    fn execute_durable(
        &mut self,
        request: DurableExecuteRequestV1,
    ) -> Result<DurableExecuteOutcomeV1, ServiceAccessErrorV1> {
        self.calls.borrow_mut().executions += 1;
        assert_eq!(request.operation.to_bytes(), [8; 16]);
        assert!(matches!(
            request.intent,
            DurableEffectIntentV1::ReplaceDocumentAndSave {
                expected_revision: 8,
                start: 0,
                end: 7,
                ref inserted,
                ..
            } if inserted == "replacement"
        ));
        if self.lose_next {
            self.lose_next = false;
            Err(ServiceAccessErrorV1::OutcomeUnknown)
        } else {
            Ok(self.outcome.clone())
        }
    }

    fn lookup_durable_receipt(
        &mut self,
        _: DurableReceiptLookupV1,
    ) -> Result<DurableReceiptLookupOutcomeV1, ServiceAccessErrorV1> {
        self.calls.borrow_mut().lookups += 1;
        Ok(DurableReceiptLookupOutcomeV1::Complete(
            self.outcome.clone(),
        ))
    }
}

fn router(
    agora_calls: Rc<RefCell<AgoraCalls>>,
    vellum_calls: Rc<RefCell<VellumCalls>>,
) -> FederatedPeerRouterV1<StateStore, AgoraPort, VellumPort> {
    let publisher = DurableResponsePublisher::new(
        StateStore(PublisherRecordV1 {
            revision: 0,
            checkpoint: PublisherCheckpointV1::initial(ClientIdV2(1)),
        }),
        AgoraPort {
            calls: agora_calls,
            lose_next: true,
        },
        PublicationLimitsV1::default(),
    )
    .unwrap();
    let workspace = WorkspaceIdV1::new(1).unwrap();
    let document = WorkspaceDocumentIdV1::new(workspace, 2).unwrap();
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
            source_revision: 9,
            text: Arc::from("replacement"),
            persistence: EditPersistenceV1::Durable(save),
        }),
    );
    let editor = EditAdapter::connect(
        VellumPort {
            calls: vellum_calls,
            outcome,
            lose_next: true,
        },
        BTreeMap::from([(
            ExternalDocumentRefV1 {
                workspace: [10; 16],
                document: [11; 16],
            },
            DocumentRouteV1 {
                workspace,
                document,
            },
        )]),
        1_024,
    )
    .unwrap();
    FederatedPeerRouterV1::new(
        publisher,
        BTreeMap::from([(
            ExternalConversationRefV1 {
                room: [2; 16],
                reply_to: [3; 16],
            },
            AgoraRouteV1 {
                source: [4; 16],
                room_sequence: 1,
                room_id: "deployment-room".into(),
                expected_agora_revision: 4,
                logical_time: 12,
                addressed_to: vec!["owner".into()],
            },
        )]),
        editor,
    )
}

#[test]
fn agora_uncertainty_replays_the_prepared_request_and_vellum_uses_lookup_only() {
    let agora_calls = Rc::new(RefCell::new(AgoraCalls::default()));
    let vellum_calls = Rc::new(RefCell::new(VellumCalls::default()));
    let mut router = router(Rc::clone(&agora_calls), Rc::clone(&vellum_calls));
    let agora = saga::ProposalEffectV1 {
        effect: EffectIdV1([1; 16]),
        proposal: saga::ProposalV1::AgoraMessage {
            room: [2; 16],
            reply_to: [3; 16],
            body: b"answer".to_vec(),
        },
        state: saga::EffectStateV1::Pending,
    };
    assert_eq!(
        router.execute(&agora).unwrap(),
        PeerEffectOutcomeV1::Uncertain
    );
    assert!(matches!(
        router.reconcile(&agora).unwrap(),
        PeerReconcileOutcomeV1::Succeeded(saga::PeerReceiptV1::AgoraMessage { .. })
    ));
    assert_eq!(agora_calls.borrow().calls, 2);
    assert_eq!(agora_calls.borrow().messages, 1);

    let vellum = saga::ProposalEffectV1 {
        effect: EffectIdV1([8; 16]),
        proposal: saga::ProposalV1::VellumEdit {
            workspace: [10; 16],
            document: [11; 16],
            expected_revision: 8,
            expected_extent: 7,
            replacement: b"replacement".to_vec(),
        },
        state: saga::EffectStateV1::Pending,
    };
    assert_eq!(
        router.execute(&vellum).unwrap(),
        PeerEffectOutcomeV1::Uncertain
    );
    assert!(matches!(
        router.reconcile(&vellum).unwrap(),
        PeerReconcileOutcomeV1::Succeeded(saga::PeerReceiptV1::VellumEdit { durable: true, .. })
    ));
    assert_eq!(vellum_calls.borrow().executions, 1);
    assert_eq!(vellum_calls.borrow().lookups, 1);
}
