use chronos_adapter_native::{coordination_v3 as native_v3, coordination_v4 as native_v4};
use chronos_client::{WireClientV3, WireClientV4};
use chronos_engine::{AcknowledgementStrength, EngineLimits, RecoveryLimits};
use chronos_host::{
    StoreError,
    native::{RequestV1, ResponseV1},
    native_allocation::AllocationReceiptV1,
    native_v2::RequestV2,
    native_v3::{InvocationRequestV1, InvocationV1, MutationV1},
    service_envelope::{EnvelopeReceipt, LoadedEnvelope, ServiceEnvelopeStore},
};
use chronos_protocol as wire;
use chronos_service::{
    Authorizer, Limits,
    atomic::{AllocationAuthorizer, AtomicLimits, AtomicService, InvocationAuthorizer},
    coordination_v2::AuthorizerV2,
};
use moira_chronos::{
    AdapterErrorV1, AdmissionAdapter, AdmissionIntentV1, AdmissionReplyV1, PreparedAdmissionV1,
    RetrySafetyV1,
};
use moira_protocol::EffectIdV1;
use std::sync::{Arc, Mutex};

type Register = Arc<Mutex<Option<(u64, Vec<u8>)>>>;

#[derive(Clone, Default)]
struct Store(Register);
impl ServiceEnvelopeStore for Store {
    fn load(&self) -> Result<Option<LoadedEnvelope>, StoreError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .as_ref()
            .map(|(revision, bytes)| LoadedEnvelope::from_storage(*revision, bytes.clone())))
    }
    fn commit(
        &mut self,
        expected: u64,
        revision: u64,
        bytes: &[u8],
    ) -> Result<EnvelopeReceipt, StoreError> {
        let mut state = self.0.lock().unwrap();
        assert_eq!(expected, state.as_ref().map_or(0, |value| value.0));
        *state = Some((revision, bytes.to_vec()));
        Ok(EnvelopeReceipt {
            revision,
            strength: AcknowledgementStrength::Durable,
        })
    }
}

struct Policy;
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}
impl AuthorizerV2 for Policy {
    fn allows_v2(&mut self, _: &RequestV2) -> bool {
        true
    }
}
impl InvocationAuthorizer for Policy {
    fn allows_invocation(&mut self, _: &InvocationRequestV1) -> bool {
        true
    }
    fn allows_retained_result(&mut self, _: &InvocationV1, _: &MutationV1) -> bool {
        true
    }
}

impl AllocationAuthorizer for Policy {
    fn allows_allocation(&mut self, _: &InvocationV1) -> bool {
        true
    }
    fn allows_allocation_result(&mut self, _: &InvocationV1, _: &AllocationReceiptV1) -> bool {
        true
    }
}

fn protocol_limits() -> wire::Limits {
    wire::Limits {
        max_frame_bytes: 4096,
        max_id_bytes: 128,
        max_graph_tasks: 4,
        max_dependencies_per_task: 4,
        max_capabilities_per_task: 4,
        max_capability_bytes: 64,
        max_page_entries: 4,
    }
}
fn recovery() -> RecoveryLimits {
    RecoveryLimits::new(900_000, EngineLimits::default())
}
fn atomic_limits() -> AtomicLimits {
    AtomicLimits {
        max_envelope_bytes: 1_000_000,
        max_retained_bytes: 4096,
        max_command_bytes: 2048,
        max_key_bytes: 64,
        max_invocations: 4,
    }
}
fn create(store: Store) -> AtomicService<Policy, Store> {
    AtomicService::create_new(
        store,
        Policy,
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        atomic_limits(),
    )
    .unwrap()
}
fn open(store: Store) -> AtomicService<Policy, Store> {
    AtomicService::open_existing(
        store,
        Policy,
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        atomic_limits(),
    )
    .unwrap()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortError {
    LostReply,
    Service,
}

struct Port {
    service: AtomicService<Policy, Store>,
    lose_execute_reply: bool,
    calls: usize,
}
impl WireClientV3 for Port {
    type Error = PortError;
    fn request_v3(
        &mut self,
        request: wire::coordination_v3::Request,
    ) -> Result<wire::coordination_v3::Response, Self::Error> {
        self.calls += 1;
        request
            .validate(protocol_limits())
            .map_err(|_| PortError::Service)?;
        let execute = matches!(
            &request.command,
            wire::coordination_v3::Command::ExecuteInvocation { .. }
        );
        let native = native_v3::invocation(request.command, protocol_limits())
            .map_err(|_| PortError::Service)?;
        let reply = self
            .service
            .invoke(native)
            .map_err(|_| PortError::Service)?;
        let response = native_v3::invocation_response(reply);
        if execute && self.lose_execute_reply {
            self.lose_execute_reply = false;
            return Err(PortError::LostReply);
        }
        Ok(response)
    }
}
impl WireClientV4 for Port {
    type Error = PortError;
    fn request_v4(
        &mut self,
        request: wire::coordination_v4::Request,
    ) -> Result<wire::coordination_v4::Response, PortError> {
        self.calls += 1;
        let invocation =
            native_v4::invocation(request, protocol_limits()).map_err(|_| PortError::Service)?;
        let serial = invocation.serial.to_string();
        let reply = self
            .service
            .allocation(invocation)
            .map_err(|_| PortError::Service)?;
        Ok(native_v4::response(&serial, reply, protocol_limits()))
    }
}

fn intent() -> AdmissionIntentV1 {
    AdmissionIntentV1 {
        effect: EffectIdV1([1; 16]),
        external_task_key: [2; 16],
        task_id: "moira-task".to_owned(),
        expected_chronos_revision: 0,
        priority: 3,
        capabilities: vec!["agent".to_owned()],
        max_attempts: 1,
        retry_safety: RetrySafetyV1::FenceUnknown,
    }
}

#[test]
fn lost_execute_reply_reconciles_after_service_restart_without_resubmission() {
    let store = Store::default();
    let mut service = create(store.clone());
    service.enable_allocation_receipts().unwrap();
    let port = Port {
        service,
        lose_execute_reply: true,
        calls: 0,
    };
    let limits = protocol_limits();
    let mut adapter = AdmissionAdapter::new(port, limits).unwrap();
    let prepared = adapter.prepare(intent()).unwrap();
    assert_eq!(prepared.serial(), 1);
    assert!(matches!(
        adapter.execute(&prepared),
        Err(AdapterErrorV1::OutcomeUnknown(PortError::LostReply))
    ));
    let port = adapter.into_port();
    assert_eq!(port.calls, 2);
    drop(port);

    let restored =
        PreparedAdmissionV1::restore(prepared.intent().clone(), prepared.serial(), limits).unwrap();
    let port = Port {
        service: open(store),
        lose_execute_reply: false,
        calls: 0,
    };
    let mut adapter = AdmissionAdapter::new(port, limits).unwrap();
    assert_eq!(
        adapter.reconcile(&restored).unwrap(),
        AdmissionReplyV1::Admitted(moira_protocol::AdmissionReceiptV1 {
            effect: EffectIdV1([1; 16]),
            task: moira_protocol::TaskRefV1 {
                external_task_key: [2; 16],
                incarnation: 0,
            },
            chronos_revision: 1,
        })
    );
    let mut port = adapter.into_port();
    assert_eq!(port.calls, 2);
    assert_eq!(port.service.signals().lookups, 1);
    assert_eq!(port.service.signals().executions, 0);
    assert_eq!(port.service.usage().retained_entries, 1);
    let Ok(ResponseV1::Snapshot(Some(snapshot))) = port
        .service
        .dispatch(RequestV1::Snapshot {
            task: "moira-task".into(),
        })
        .unwrap()
    else {
        panic!("retained task expected")
    };
    assert_eq!(snapshot.reference.incarnation().get(), 0);
}

#[test]
fn malformed_or_unfunded_prepared_identity_refuses_before_dispatch() {
    let store = Store::default();
    let port = Port {
        service: create(store),
        lose_execute_reply: false,
        calls: 0,
    };
    let limits = protocol_limits();
    let mut adapter = AdmissionAdapter::new(port, limits).unwrap();
    let mut invalid = intent();
    invalid.task_id = "x".repeat(129);
    assert!(matches!(
        adapter.prepare(invalid),
        Err(AdapterErrorV1::InvalidInput)
    ));
    assert!(matches!(
        PreparedAdmissionV1::restore(intent(), 0, limits),
        Err(AdapterErrorV1::InvalidInput)
    ));
    assert_eq!(adapter.into_port().calls, 0);
}

#[test]
fn legacy_success_without_original_allocation_refuses_receipt() {
    let port = Port {
        service: create(Store::default()),
        lose_execute_reply: false,
        calls: 0,
    };
    let mut adapter = AdmissionAdapter::new(port, protocol_limits()).unwrap();
    let prepared = adapter.prepare(intent()).unwrap();
    assert_eq!(
        adapter.execute(&prepared).unwrap(),
        AdmissionReplyV1::HistoryLost
    );
    assert_eq!(adapter.into_port().calls, 3);
}
