use moira_protocol::EffectIdV1;
use moira_vellum::{
    DocumentRouteV1, EditAdapter, EditIntentV1, EditReplyV1, ExternalDocumentRefV1,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::Arc};
use vellum_protocol::{
    ClientId, DurableEffectIntentV1, DurableEffectReceiptV1, DurableExecuteOutcomeV1,
    DurableExecuteRequestV1, DurableReceiptLookupOutcomeV1, DurableReceiptLookupV1,
    DurableReplaceAndSaveReceiptV1, EditPersistenceV1, ReplyV1, RequestV1, SaveApplicationV1,
    SaveReceiptV1, SaveSynchronizationV1, ServiceAccessErrorV1, ServiceControlErrorV1, ServicePort,
    WorkspaceDocumentIdV1, WorkspaceIdV1,
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
