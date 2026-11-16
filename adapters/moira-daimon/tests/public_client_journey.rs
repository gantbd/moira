use daimon_client::ServicePort;
use daimon_harness::Limits;
use daimon_host::{
    AttemptRefusalV1, AttemptValidator, AuthenticatedSessionV1, AuthorityGrantV1,
    AuthorityRefusalV1, ExecutionAuthorizer, Provider, ProviderOutcomeV1, ProviderReceiptV1,
};
use daimon_protocol as daimon;
use daimon_service::{Service, ServiceLimitsV1};
use moira_daimon::{ExecutionAdapter, ExecutionIntentV1, ExecutionReplyV1};
use moira_protocol::{DispositionV1, EffectIdV1, ResultReceiptV1, TaskRefV1};
use std::cell::{Cell, RefCell};
use std::convert::Infallible;
use std::rc::Rc;

struct Authorizer;

impl ExecutionAuthorizer for Authorizer {
    fn authorize(
        &mut self,
        _session: &AuthenticatedSessionV1,
        request: &daimon::ExecuteRequestV1,
        _now: u64,
    ) -> Result<AuthorityGrantV1, AuthorityRefusalV1> {
        Ok(AuthorityGrantV1 {
            decision_id: [8; 16],
            expires_at: 20,
            tools: request.tools.clone(),
        })
    }
}

struct Attempts;

impl AttemptValidator for Attempts {
    fn validate(
        &mut self,
        _session: &AuthenticatedSessionV1,
        _request: &daimon::ExecuteRequestV1,
    ) -> Result<(), AttemptRefusalV1> {
        Ok(())
    }
}

struct FakeProvider {
    calls: Rc<Cell<usize>>,
    unknown: bool,
}

impl Provider for FakeProvider {
    fn execute(&mut self, request: &daimon::ExecuteRequestV1) -> ProviderOutcomeV1 {
        self.calls.set(self.calls.get() + 1);
        if self.unknown {
            ProviderOutcomeV1::OutcomeUnknown
        } else {
            ProviderOutcomeV1::Receipt(ProviderReceiptV1 {
                execution_id: request.execution_id,
                disposition: daimon::ExecutionDispositionV1::Completed,
                output_bytes: 12,
                artifacts: vec![],
            })
        }
    }
}

struct Port {
    service: Rc<RefCell<Service<Authorizer, Attempts, FakeProvider>>>,
}

impl ServicePort for Port {
    type Error = Infallible;

    fn execute(
        &mut self,
        session: daimon::SessionHandleV1,
        command: daimon::CommandV1,
    ) -> Result<daimon::ReplyV1, Self::Error> {
        Ok(self.service.borrow_mut().execute(session, command))
    }
}

fn intent() -> ExecutionIntentV1 {
    ExecutionIntentV1 {
        effect: EffectIdV1([1; 16]),
        execution_id: [2; 16],
        task: TaskRefV1 {
            external_task_key: [3; 16],
            incarnation: 4,
        },
        attempt_number: 1,
        principal: [5; 16],
        profile: [6; 16],
        idempotency_key: [7; 16],
        context: vec![[9; 16]],
        tools: vec![[10; 16]],
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

fn adapter(unknown: bool) -> (ExecutionAdapter<Port>, Rc<Cell<usize>>) {
    let calls = Rc::new(Cell::new(0));
    let mut service = Service::new(
        Limits::default(),
        ServiceLimitsV1::default(),
        Authorizer,
        Attempts,
        FakeProvider {
            calls: calls.clone(),
            unknown,
        },
    )
    .unwrap();
    let session = service
        .open_session(AuthenticatedSessionV1 {
            principal: [5; 16],
            authority_context: [11; 16],
        })
        .unwrap();
    let service = Rc::new(RefCell::new(service));
    (ExecutionAdapter::new(session, Port { service }), calls)
}

#[test]
fn public_daimon_service_maps_one_execution_to_one_exact_moira_receipt() {
    let (mut adapter, calls) = adapter(false);
    let expected = ExecutionReplyV1::Result(ResultReceiptV1 {
        effect: EffectIdV1([1; 16]),
        execution_id: [2; 16],
        task: TaskRefV1 {
            external_task_key: [3; 16],
            incarnation: 4,
        },
        attempt_number: 1,
        disposition: DispositionV1::Completed,
    });

    assert_eq!(adapter.execute(intent(), 10).unwrap(), expected);
    assert_eq!(adapter.execute(intent(), 10).unwrap(), expected);
    assert_eq!(calls.get(), 1);
}

#[test]
fn public_daimon_uncertainty_maps_to_attributable_moira_uncertainty_without_retry() {
    let (mut adapter, calls) = adapter(true);
    let first = adapter.execute(intent(), 10).unwrap();
    let replay = adapter.execute(intent(), 10).unwrap();

    assert_eq!(first, replay);
    let ExecutionReplyV1::Result(receipt) = first else {
        panic!("expected attributable uncertain receipt")
    };
    assert_eq!(receipt.disposition, DispositionV1::Uncertain);
    assert_eq!(receipt.effect, EffectIdV1([1; 16]));
    assert_eq!(receipt.execution_id, [2; 16]);
    assert_eq!(calls.get(), 1);
}

#[test]
fn read_only_status_checks_daimon_session_and_does_not_replay_execution() {
    use daimon_client::ReadOnlyClient;
    use moira_daimon::{ExecutionStatusV1, inspect_execution};
    let (mut adapter, calls) = adapter(true);
    let ExecutionReplyV1::Result(receipt) = adapter.execute(intent(), 10).unwrap() else {
        panic!("receipt expected");
    };
    let port = adapter.into_port();
    let reader = |principal| {
        let session = port
            .service
            .borrow_mut()
            .open_session(AuthenticatedSessionV1 {
                principal,
                authority_context: [11; 16],
            })
            .unwrap();
        ReadOnlyClient::new(
            session,
            Port {
                service: port.service.clone(),
            },
        )
    };
    assert_eq!(
        inspect_execution(&mut reader([5; 16]), receipt.execution_id),
        ExecutionStatusV1::Observed(daimon::ExecutionPhaseV1::OutcomeUnknown),
    );
    assert_eq!(
        inspect_execution(&mut reader([6; 16]), receipt.execution_id),
        ExecutionStatusV1::Denied,
    );
    assert_eq!(
        inspect_execution(&mut reader([5; 16]), [0; 16]),
        ExecutionStatusV1::Missing,
    );
    assert_eq!(calls.get(), 1);
}
