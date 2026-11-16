use moira_client::{ToolContinuationMutationClient, ToolContinuationServicePort};
use moira_core::tool_continuation::{LimitsV1, persistence::CodecLimitsV1};
use moira_host::{
    ToolExecutionOutcomeV1, ToolExecutorV1, ToolInvocationV1, ToolOutputCommitV1,
    ToolOutputRecordV1, ToolOutputStoreV1,
};
#[cfg(unix)]
use moira_local_harness::process_tool::{ProcessToolExecutorV1, ToolProcessProfileV1};
use moira_local_harness::tool_runtime::{
    ToolReconcileOutcomeV1, ToolRunOutcomeV1, ToolUncertaintyV1, dispatch_ready,
    reconcile_in_flight, resolve_completed_tool_results, resolve_succeeded_output,
};
use moira_protocol::{EffectIdV1, WorkflowIdV1, tool_continuation as tools};
use moira_service::tool_continuation::ToolContinuationService;
use moira_sqlite::{
    tool_continuation::{ToolContinuationSqliteStore, ToolContinuationStoreLimitsV1},
    tool_output::{ToolOutputSqliteStore, ToolOutputStoreLimitsV1},
};
#[cfg(unix)]
use std::time::Duration;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Port(ToolContinuationService<ToolContinuationSqliteStore>);

impl ToolContinuationServicePort for Port {
    type Error = moira_sqlite::tool_continuation::ToolContinuationStoreErrorV1;

    fn execute(&mut self, command: tools::CommandV1) -> Result<tools::ReplyV1, Self::Error> {
        self.0.execute(command)
    }
}

struct Executor {
    calls: usize,
    outcome: ToolExecutionOutcomeV1,
}

#[derive(Debug)]
enum LostReplyError {
    Lost,
    Store,
}

struct LostReplyOutputStore {
    inner: ToolOutputSqliteStore,
    lose_next_reply: bool,
}

impl ToolOutputStoreV1 for LostReplyOutputStore {
    type Error = LostReplyError;

    fn put(
        &mut self,
        execution: moira_core::tool_continuation::ToolExecutionIdV1,
        capability: moira_core::tool_continuation::ToolCapabilityIdV1,
        evidence_digest: [u8; 32],
        output: &[u8],
    ) -> Result<ToolOutputCommitV1, Self::Error> {
        let committed = self
            .inner
            .put(execution, capability, evidence_digest, output)
            .map_err(|_| LostReplyError::Store)?;
        if self.lose_next_reply {
            self.lose_next_reply = false;
            Err(LostReplyError::Lost)
        } else {
            Ok(committed)
        }
    }

    fn get(&mut self, output_digest: [u8; 32]) -> Result<Option<Vec<u8>>, Self::Error> {
        self.inner
            .get(output_digest)
            .map_err(|_| LostReplyError::Store)
    }

    fn find(
        &mut self,
        execution: moira_core::tool_continuation::ToolExecutionIdV1,
    ) -> Result<Option<(ToolOutputRecordV1, Vec<u8>)>, Self::Error> {
        self.inner
            .find(execution)
            .map_err(|_| LostReplyError::Store)
    }
}

impl ToolExecutorV1 for Executor {
    type Error = ();

    fn execute(
        &mut self,
        invocation: &ToolInvocationV1,
    ) -> Result<ToolExecutionOutcomeV1, Self::Error> {
        self.calls += 1;
        assert_eq!(invocation.call.capability.0, [4; 16]);
        assert_eq!(invocation.gate.attempt.task_key, [7; 16]);
        Ok(self.outcome.clone())
    }
}

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "moira-tool-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self { directory }
    }

    fn continuation(&self) -> ToolContinuationSqliteStore {
        ToolContinuationSqliteStore::open(
            self.directory.join("continuation.db"),
            [1; 16],
            continuation_limits(),
        )
        .unwrap()
    }

    fn outputs(&self) -> ToolOutputSqliteStore {
        ToolOutputSqliteStore::open(
            self.directory.join("outputs.db"),
            [2; 16],
            ToolOutputStoreLimitsV1 {
                max_entries: 4,
                max_output_bytes: 1_024,
                max_database_pages: 64,
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn continuation_limits() -> ToolContinuationStoreLimitsV1 {
    ToolContinuationStoreLimitsV1 {
        codec: CodecLimitsV1 {
            max_bytes: 64 * 1_024,
            domain: LimitsV1 {
                max_sagas: 4,
                max_calls_per_saga: 4,
                max_argument_bytes: 4 * 1_024,
                max_elapsed_millis: 10_000,
                max_output_bytes: 4 * 1_024,
            },
        },
        saga: LimitsV1 {
            max_sagas: 4,
            max_calls_per_saga: 4,
            max_argument_bytes: 4 * 1_024,
            max_elapsed_millis: 10_000,
            max_output_bytes: 4 * 1_024,
        },
        max_database_pages: 64,
    }
}

fn gate() -> tools::ToolGateV1 {
    tools::ToolGateV1 {
        authority: tools::AuthorityEvidenceV1 {
            decision: tools::AuthorityDecisionIdV1([3; 16]),
            subject: tools::AuthoritySubjectIdV1([5; 16]),
            capability: tools::ToolCapabilityIdV1([4; 16]),
            epoch: 6,
        },
        attempt: tools::AttemptEvidenceV1 {
            task_key: [7; 16],
            incarnation: 8,
            attempt: 1,
        },
    }
}

fn ready(
    fixture: &Fixture,
    workflow: WorkflowIdV1,
) -> (ToolContinuationMutationClient<Port>, tools::SagaViewV1) {
    let service = ToolContinuationService::new(fixture.continuation());
    let mut client = ToolContinuationMutationClient::new(Port(service));
    let tools::ReplyV1::Accepted(tools::OutcomeV1::Applied { saga, .. }) = client
        .apply(
            None,
            tools::MutationV1::Admit {
                workflow,
                source_execution: EffectIdV1([9; 16]),
                source_attempt: gate().attempt,
                calls: vec![tools::ToolCallV1 {
                    request: tools::ToolRequestIdV1([10; 16]),
                    capability: tools::ToolCapabilityIdV1([4; 16]),
                    provider_correlation: b"call-1".to_vec(),
                    arguments: br#"{"path":"README.md"}"#.to_vec(),
                    budget: tools::ToolBudgetV1 {
                        max_elapsed_millis: 100,
                        max_output_bytes: 1_024,
                    },
                }],
            },
        )
        .unwrap()
    else {
        panic!("admission expected")
    };
    let effect = saga.effects[0].effect;
    let tools::ReplyV1::Accepted(tools::OutcomeV1::Applied { saga, .. }) = client
        .apply(
            Some(0),
            tools::MutationV1::Authorize {
                workflow,
                effect,
                gate: gate(),
            },
        )
        .unwrap()
    else {
        panic!("authorization expected")
    };
    (client, saga)
}

fn dispatch_only(
    client: &mut ToolContinuationMutationClient<Port>,
    saga: &tools::SagaViewV1,
    execution: tools::ToolExecutionIdV1,
) -> tools::SagaViewV1 {
    let tools::ReplyV1::Accepted(tools::OutcomeV1::Applied { saga, .. }) = client
        .apply(
            Some(1),
            tools::MutationV1::Dispatch {
                workflow: saga.workflow,
                effect: saga.effects[0].effect,
                execution,
            },
        )
        .unwrap()
    else {
        panic!("dispatch expected")
    };
    saga
}

#[cfg(unix)]
#[test]
fn selected_process_profile_dispatches_into_the_durable_output_store() {
    let fixture = Fixture::new();
    let workflow = WorkflowIdV1([21; 16]);
    let (mut client, saga) = ready(&fixture, workflow);
    let effect = saga.effects[0].effect;
    let mut executor = ProcessToolExecutorV1::try_new(
        vec![ToolProcessProfileV1 {
            capability: moira_core::tool_continuation::ToolCapabilityIdV1([4; 16]),
            executable: PathBuf::from("/bin/cat"),
            arguments: Vec::new(),
            working_directory: fixture.directory.clone(),
            max_elapsed: Duration::from_millis(500),
            max_input_bytes: 1_024,
            max_output_bytes: 1_024,
            max_stderr_bytes: 1_024,
            max_executable_bytes: 2 * 1_024 * 1_024,
        }],
        Duration::from_millis(1),
    )
    .unwrap();
    let mut outputs = fixture.outputs();
    let ToolRunOutcomeV1::Succeeded { record, .. } = dispatch_ready(
        &mut client,
        &mut executor,
        &mut outputs,
        Some(1),
        &saga,
        effect,
        tools::ToolExecutionIdV1([22; 16]),
    )
    .unwrap() else {
        panic!("process success expected")
    };
    assert_ne!(record.evidence_digest, [0; 32]);
    assert_eq!(
        outputs.get(record.output_digest).unwrap().unwrap(),
        br#"{"path":"README.md"}"#
    );
}

#[test]
fn output_is_durable_before_success_and_reopens_for_model_resume() {
    let fixture = Fixture::new();
    let workflow = WorkflowIdV1([11; 16]);
    let (mut client, saga) = ready(&fixture, workflow);
    let effect = saga.effects[0].effect;
    let mut executor = Executor {
        calls: 0,
        outcome: ToolExecutionOutcomeV1::Succeeded {
            evidence_digest: [12; 32],
            output: b"durable tool result".to_vec(),
        },
    };
    let mut outputs = fixture.outputs();
    let ToolRunOutcomeV1::Succeeded {
        storage_revision,
        record,
        ..
    } = dispatch_ready(
        &mut client,
        &mut executor,
        &mut outputs,
        Some(1),
        &saga,
        effect,
        tools::ToolExecutionIdV1([13; 16]),
    )
    .unwrap()
    else {
        panic!("success expected")
    };
    assert_eq!(storage_revision, 3);
    assert_eq!(executor.calls, 1);
    assert_eq!(record.output_bytes, b"durable tool result".len());
    assert_eq!(
        outputs.get(record.output_digest).unwrap().unwrap(),
        b"durable tool result"
    );
    assert!(
        outputs
            .put(
                moira_core::tool_continuation::ToolExecutionIdV1([13; 16]),
                moira_core::tool_continuation::ToolCapabilityIdV1([4; 16]),
                [12; 32],
                b"durable tool result",
            )
            .unwrap()
            .replayed
    );
    assert!(matches!(
        outputs.put(
            moira_core::tool_continuation::ToolExecutionIdV1([13; 16]),
            moira_core::tool_continuation::ToolCapabilityIdV1([4; 16]),
            [12; 32],
            b"conflicting result",
        ),
        Err(moira_sqlite::tool_output::ToolOutputStoreErrorV1::Conflict)
    ));
    let tools::ReplyV1::Accepted(tools::OutcomeV1::Applied { saga, .. }) = client
        .apply(
            Some(3),
            tools::MutationV1::Accept {
                workflow,
                receipt: tools::ToolReceiptV1 {
                    effect,
                    capability: tools::ToolCapabilityIdV1([4; 16]),
                    execution: tools::ToolExecutionIdV1([13; 16]),
                    evidence_digest: record.evidence_digest,
                    output_digest: record.output_digest,
                    output_bytes: record.output_bytes,
                },
            },
        )
        .unwrap()
    else {
        panic!("replayed success expected")
    };
    let resolved = resolve_completed_tool_results(&mut outputs, &saga).unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].request, [10; 16]);
    assert_eq!(resolved[0].output, b"durable tool result");
    drop(outputs);
    drop(client);

    let mut reopened = fixture.outputs();
    assert_eq!(
        reopened.get(record.output_digest).unwrap().unwrap(),
        b"durable tool result"
    );
}

#[test]
fn provider_uncertainty_is_fenced_after_one_dispatch() {
    let fixture = Fixture::new();
    let workflow = WorkflowIdV1([21; 16]);
    let (mut client, saga) = ready(&fixture, workflow);
    let effect = saga.effects[0].effect;
    let mut executor = Executor {
        calls: 0,
        outcome: ToolExecutionOutcomeV1::OutcomeUnknown,
    };
    let mut outputs = fixture.outputs();
    assert!(matches!(
        dispatch_ready(
            &mut client,
            &mut executor,
            &mut outputs,
            Some(1),
            &saga,
            effect,
            tools::ToolExecutionIdV1([22; 16]),
        )
        .unwrap(),
        ToolRunOutcomeV1::Uncertain {
            storage_revision: 3,
            cause: ToolUncertaintyV1::Provider,
            ..
        }
    ));
    assert_eq!(executor.calls, 1);
}

#[test]
fn lost_output_commit_reply_reconciles_without_reinvoking_tool() {
    let fixture = Fixture::new();
    let workflow = WorkflowIdV1([31; 16]);
    let (mut client, saga) = ready(&fixture, workflow);
    let effect = saga.effects[0].effect;
    let mut executor = Executor {
        calls: 0,
        outcome: ToolExecutionOutcomeV1::Succeeded {
            evidence_digest: [32; 32],
            output: b"committed before lost reply".to_vec(),
        },
    };
    let mut outputs = LostReplyOutputStore {
        inner: fixture.outputs(),
        lose_next_reply: true,
    };
    assert!(matches!(
        dispatch_ready(
            &mut client,
            &mut executor,
            &mut outputs,
            Some(1),
            &saga,
            effect,
            tools::ToolExecutionIdV1([33; 16]),
        )
        .unwrap(),
        ToolRunOutcomeV1::Succeeded {
            storage_revision: 3,
            ..
        }
    ));
    assert_eq!(executor.calls, 1);
}

#[test]
fn retained_output_closes_inflight_recovery_and_supplies_resume_bytes() {
    let fixture = Fixture::new();
    let workflow = WorkflowIdV1([41; 16]);
    let (mut client, ready) = ready(&fixture, workflow);
    let effect = ready.effects[0].effect;
    let execution = tools::ToolExecutionIdV1([42; 16]);
    let inflight = dispatch_only(&mut client, &ready, execution);
    let mut outputs = fixture.outputs();
    outputs
        .put(
            moira_core::tool_continuation::ToolExecutionIdV1(execution.0),
            moira_core::tool_continuation::ToolCapabilityIdV1([4; 16]),
            [43; 32],
            b"recoverable output",
        )
        .unwrap();
    assert!(matches!(
        reconcile_in_flight(&mut client, &mut outputs, Some(2), &inflight, effect).unwrap(),
        ToolReconcileOutcomeV1::Succeeded {
            storage_revision: 3,
            ..
        }
    ));
    let mut port = client.into_port();
    let tools::ReplyV1::Accepted(tools::OutcomeV1::Saga(Some(succeeded))) =
        port.execute(tools::CommandV1::Saga { workflow }).unwrap()
    else {
        panic!("succeeded saga expected")
    };
    assert_eq!(
        resolve_succeeded_output(&mut outputs, &succeeded, effect).unwrap(),
        b"recoverable output"
    );
}

#[test]
fn inflight_recovery_without_durable_output_fences_instead_of_reinvoking() {
    let fixture = Fixture::new();
    let workflow = WorkflowIdV1([51; 16]);
    let (mut client, ready) = ready(&fixture, workflow);
    let effect = ready.effects[0].effect;
    let inflight = dispatch_only(&mut client, &ready, tools::ToolExecutionIdV1([52; 16]));
    let mut outputs = fixture.outputs();
    assert!(matches!(
        reconcile_in_flight(&mut client, &mut outputs, Some(2), &inflight, effect).unwrap(),
        ToolReconcileOutcomeV1::Uncertain {
            storage_revision: 3,
            ..
        }
    ));
}
