use moira_protocol::EffectIdV1;
use moira_vellum::{
    DecisionPacketAdapter, DecisionPacketErrorV1, DecisionPacketRefV1, DocumentRouteV1,
    EditAdapter, EditIntentV1, EditReplyV1, ExternalDocumentRefV1,
};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::Arc};
use vellum_protocol::{
    ClientId, CommandV1, DurableEffectIntentV1, DurableEffectReceiptV1, DurableExecuteOutcomeV1,
    DurableExecuteRequestV1, DurableReceiptLookupOutcomeV1, DurableReceiptLookupV1,
    DurableReplaceAndSaveReceiptV1, EditPersistenceV1, ReplyV1, RequestV1, SaveApplicationV1,
    SaveReceiptV1, SaveSynchronizationV1, ServiceAccessErrorV1, ServiceControlErrorV1, ServicePort,
    SnapshotV1, VersionWitness, ViewOpenReceiptV1, ViewOpenStatusV1, WorkspaceDocumentIdV1,
    WorkspaceIdV1,
};

#[derive(Clone, Copy)]
enum Mode {
    Execute,
    Lookup,
}

#[derive(Default)]
struct Calls {
    executions: Vec<DurableExecuteRequestV1>,
    lookups: Vec<DurableReceiptLookupV1>,
}

struct Port {
    calls: Rc<RefCell<Calls>>,
    mode: Mode,
    outcome: DurableExecuteOutcomeV1,
}

impl ServicePort for Port {
    fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
        Ok(ClientId::new(1).unwrap())
    }

    fn execute(&mut self, _request: RequestV1) -> Result<ReplyV1, ServiceAccessErrorV1> {
        unreachable!("durable adapter never uses ordinary execution")
    }

    fn disconnect(&mut self, _client: ClientId) -> Result<(), ServiceControlErrorV1> {
        Ok(())
    }

    fn execute_durable(
        &mut self,
        request: DurableExecuteRequestV1,
    ) -> Result<DurableExecuteOutcomeV1, ServiceAccessErrorV1> {
        assert!(matches!(self.mode, Mode::Execute));
        self.calls.borrow_mut().executions.push(request);
        Ok(self.outcome.clone())
    }

    fn lookup_durable_receipt(
        &mut self,
        lookup: DurableReceiptLookupV1,
    ) -> Result<DurableReceiptLookupOutcomeV1, ServiceAccessErrorV1> {
        assert!(matches!(self.mode, Mode::Lookup));
        self.calls.borrow_mut().lookups.push(lookup);
        Ok(DurableReceiptLookupOutcomeV1::Complete(
            self.outcome.clone(),
        ))
    }
}

fn save_receipt() -> SaveReceiptV1 {
    SaveReceiptV1 {
        application: SaveApplicationV1::Written,
        synchronization: SaveSynchronizationV1::Succeeded,
        no_future_effects: true,
        quiescent: true,
        adapter_failure: false,
        cleanup_failed: false,
        baseline_published: true,
        outcome_unknown: false,
    }
}

fn fixture(mode: Mode) -> (EditAdapter<Port>, Rc<RefCell<Calls>>, EditIntentV1) {
    let workspace = WorkspaceIdV1::new(1).unwrap();
    let document = WorkspaceDocumentIdV1::new(workspace, 2).unwrap();
    let target = ExternalDocumentRefV1 {
        workspace: [3; 16],
        document: [4; 16],
    };
    let calls = Rc::new(RefCell::new(Calls::default()));
    let outcome = DurableExecuteOutcomeV1::Completed(
        DurableEffectReceiptV1::DocumentReplacedAndSaved(DurableReplaceAndSaveReceiptV1 {
            document,
            source_revision: 8,
            text: Arc::from("replacement"),
            persistence: EditPersistenceV1::Durable(save_receipt()),
        }),
    );
    let adapter = EditAdapter::connect(
        Port {
            calls: Rc::clone(&calls),
            mode,
            outcome,
        },
        BTreeMap::from([(
            target,
            DocumentRouteV1 {
                workspace,
                document,
            },
        )]),
        1_024,
    )
    .unwrap();
    let intent = EditIntentV1 {
        effect: EffectIdV1([5; 16]),
        target,
        expected_revision: 7,
        expected_extent: 12,
        replacement: b"replacement".to_vec(),
    };
    (adapter, calls, intent)
}

#[test]
fn moira_effect_becomes_one_exact_durable_vellum_operation() {
    let (mut adapter, calls, intent) = fixture(Mode::Execute);
    let EditReplyV1::Completed { effect, receipt } = adapter.execute(&intent).unwrap() else {
        panic!("completed edit expected")
    };
    assert_eq!(effect, intent.effect);
    assert_eq!(receipt.source_revision, 8);
    let calls = calls.borrow();
    assert_eq!(calls.executions.len(), 1);
    assert_eq!(calls.executions[0].operation.to_bytes(), intent.effect.0);
    assert!(matches!(
        &calls.executions[0].intent,
        DurableEffectIntentV1::ReplaceDocumentAndSave {
            expected_revision: 7,
            start: 0,
            end: 12,
            inserted,
            ..
        } if inserted == "replacement"
    ));
}

#[test]
fn historical_reconciliation_never_falls_through_to_execution() {
    let (mut adapter, calls, intent) = fixture(Mode::Lookup);
    assert!(matches!(
        adapter.lookup(&intent).unwrap(),
        EditReplyV1::Completed { .. }
    ));
    let calls = calls.borrow();
    assert!(calls.executions.is_empty());
    assert_eq!(calls.lookups.len(), 1);
    assert_eq!(calls.lookups[0].operation.to_bytes(), intent.effect.0);
}

struct PacketPort {
    document: WorkspaceDocumentIdV1,
    revision: u64,
    text: Arc<str>,
    calls: Rc<RefCell<Vec<&'static str>>>,
}

impl ServicePort for PacketPort {
    fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
        Ok(ClientId::new(2).unwrap())
    }

    fn execute(&mut self, request: RequestV1) -> Result<ReplyV1, ServiceAccessErrorV1> {
        let outcome = match request.command {
            CommandV1::OpenView => {
                self.calls.borrow_mut().push("open");
                vellum_protocol::OutcomeV1::ViewOpened(ViewOpenReceiptV1 {
                    document: self.document,
                    status: ViewOpenStatusV1::Opened,
                })
            }
            CommandV1::SelectDocument { document } if document == self.document => {
                self.calls.borrow_mut().push("select");
                vellum_protocol::OutcomeV1::DocumentSelected(document)
            }
            CommandV1::Snapshot => {
                self.calls.borrow_mut().push("snapshot");
                vellum_protocol::OutcomeV1::Snapshot(SnapshotV1 {
                    witness: VersionWitness::new(3).unwrap(),
                    revision: self.revision,
                    text: Arc::clone(&self.text),
                })
            }
            _ => {
                return Ok(ReplyV1::Refused(
                    vellum_protocol::RefusalV1::UnknownDocument,
                ));
            }
        };
        Ok(ReplyV1::Accepted(outcome))
    }

    fn disconnect(&mut self, _client: ClientId) -> Result<(), ServiceControlErrorV1> {
        Ok(())
    }
}

fn packet_fixture(
    revision: u64,
    text: &str,
) -> (
    DecisionPacketAdapter<PacketPort>,
    DecisionPacketRefV1,
    Rc<RefCell<Vec<&'static str>>>,
) {
    let workspace = WorkspaceIdV1::new(7).unwrap();
    let document = WorkspaceDocumentIdV1::new(workspace, 8).unwrap();
    let target = ExternalDocumentRefV1 {
        workspace: [9; 16],
        document: [10; 16],
    };
    let calls = Rc::new(RefCell::new(Vec::new()));
    let adapter = DecisionPacketAdapter::connect(
        PacketPort {
            document,
            revision,
            text: Arc::from(text),
            calls: Rc::clone(&calls),
        },
        BTreeMap::from([(
            target,
            DocumentRouteV1 {
                workspace,
                document,
            },
        )]),
        4_096,
    )
    .unwrap();
    let reference = DecisionPacketRefV1 {
        owner_route: target.workspace,
        artifact_route: target.document,
        expected_revision: revision,
        expected_sha256: Sha256::digest(text.as_bytes()).into(),
    };
    (adapter, reference, calls)
}

#[test]
fn decision_packet_resolves_one_exact_revision_and_digest_without_edit_authority() {
    let (mut adapter, reference, calls) = packet_fixture(11, "decision packet");
    let packet = adapter.read(reference).unwrap();
    assert_eq!(packet.reference, reference);
    assert_eq!(packet.text, "decision packet");
    assert_eq!(&*calls.borrow(), &["open", "select", "snapshot"]);
}

#[test]
fn decision_packet_refuses_stale_or_digest_mismatched_material() {
    let (mut stale, mut reference, _) = packet_fixture(11, "decision packet");
    reference.expected_revision = 10;
    assert_eq!(
        stale.read(reference),
        Err(DecisionPacketErrorV1::RevisionMismatch {
            expected: 10,
            actual: 11,
        })
    );

    let (mut changed, mut reference, _) = packet_fixture(11, "decision packet");
    reference.expected_sha256 = [0; 32];
    assert_eq!(
        changed.read(reference),
        Err(DecisionPacketErrorV1::DigestMismatch)
    );
}
