use moira_client::{ToolContinuationMutationClient, ToolContinuationServicePort};
use moira_core::tool_continuation::{LimitsV1, persistence::CodecLimitsV1};
use moira_daimon::{ModelProposalV1, ModelResultV1};
use moira_host::{ToolExecutionOutcomeV1, ToolExecutorV1, ToolInvocationV1};
use moira_local_harness::tool_turn::{ToolTurnOutcomeV1, execute_tool_turn};
use moira_protocol::{
    DispositionV1, EffectIdV1, ResultReceiptV1, TaskRefV1, tool_continuation as tools,
};
use moira_service::tool_continuation::ToolContinuationService;
use moira_sqlite::{
    tool_continuation::{ToolContinuationSqliteStore, ToolContinuationStoreLimitsV1},
    tool_output::{ToolOutputSqliteStore, ToolOutputStoreLimitsV1},
};
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
}

impl ToolExecutorV1 for Executor {
    type Error = ();

    fn execute(
        &mut self,
        invocation: &ToolInvocationV1,
    ) -> Result<ToolExecutionOutcomeV1, Self::Error> {
        self.calls += 1;
        Ok(ToolExecutionOutcomeV1::Succeeded {
            evidence_digest: [u8::try_from(self.calls).unwrap(); 32],
            output: invocation.call.arguments.clone(),
        })
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moira-tool-turn-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn client(&self) -> ToolContinuationMutationClient<Port> {
        let store = ToolContinuationSqliteStore::open(
            self.0.join("continuation.db"),
            [1; 16],
            ToolContinuationStoreLimitsV1 {
                codec: CodecLimitsV1 {
                    max_bytes: 64 * 1_024,
                    domain: limits(),
                },
                saga: limits(),
                max_database_pages: 64,
            },
        )
        .unwrap();
        ToolContinuationMutationClient::new(Port(ToolContinuationService::new(store)))
    }

    fn outputs(&self) -> ToolOutputSqliteStore {
        ToolOutputSqliteStore::open(
            self.0.join("outputs.db"),
            [2; 16],
            ToolOutputStoreLimitsV1 {
                max_entries: 8,
                max_output_bytes: 1_024,
                max_database_pages: 64,
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn limits() -> LimitsV1 {
    LimitsV1 {
        max_sagas: 4,
        max_calls_per_saga: 4,
        max_argument_bytes: 1_024,
        max_elapsed_millis: 10_000,
        max_output_bytes: 1_024,
    }
}

fn result() -> ModelResultV1 {
    ModelResultV1 {
        receipt: ResultReceiptV1 {
            effect: EffectIdV1([3; 16]),
            execution_id: [4; 16],
            task: TaskRefV1 {
                external_task_key: [5; 16],
                incarnation: 6,
            },
            attempt_number: 1,
            disposition: DispositionV1::Completed,
        },
        evidence_digest: [7; 32],
        input_tokens: 8,
        output_tokens: 9,
        output: Vec::new(),
        proposals: vec![
            ModelProposalV1::ToolCall {
                request: [10; 16],
                capability: [11; 16],
                provider_correlation: b"provider-call-1".to_vec(),
                arguments: b"first result".to_vec(),
            },
            ModelProposalV1::ToolCall {
                request: [12; 16],
                capability: [13; 16],
                provider_correlation: b"provider-call-2".to_vec(),
                arguments: b"second result".to_vec(),
            },
        ],
    }
}

#[test]
fn multi_call_turn_is_gated_in_order_and_replays_without_effects() {
    let fixture = Fixture::new();
    let mut client = fixture.client();
    let mut outputs = fixture.outputs();
    let mut executor = Executor { calls: 0 };
    let mut authorized = Vec::new();
    let mut observed = 0;
    let mut authorize = |capability: tools::ToolCapabilityIdV1| {
        authorized.push(capability);
        Ok::<_, ()>(tools::AuthorityEvidenceV1 {
            decision: tools::AuthorityDecisionIdV1(capability.0),
            subject: tools::AuthoritySubjectIdV1([14; 16]),
            capability,
            epoch: 15,
        })
    };
    let mut observe = |task_key, incarnation, attempt| {
        observed += 1;
        Ok::<_, ()>(tools::AttemptEvidenceV1 {
            task_key,
            incarnation,
            attempt,
        })
    };
    let outcome = execute_tool_turn(
        &mut client,
        &mut executor,
        &mut outputs,
        None,
        moira_protocol::WorkflowIdV1([16; 16]),
        &result(),
        tools::ToolBudgetV1 {
            max_elapsed_millis: 100,
            max_output_bytes: 1_024,
        },
        &mut authorize,
        &mut observe,
    )
    .unwrap();
    let ToolTurnOutcomeV1::Completed {
        storage_revision,
        results,
        ..
    } = outcome
    else {
        panic!("completed tool turn expected")
    };
    assert_eq!(storage_revision, 6);
    assert_eq!(executor.calls, 2);
    assert_eq!(observed, 2);
    assert_eq!(
        authorized,
        vec![
            tools::ToolCapabilityIdV1([11; 16]),
            tools::ToolCapabilityIdV1([13; 16])
        ]
    );
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].request, [10; 16]);
    assert_eq!(results[0].output, b"first result");
    assert_eq!(results[1].request, [12; 16]);
    assert_eq!(results[1].output, b"second result");

    drop(client);
    drop(outputs);
    let mut client = fixture.client();
    let mut outputs = fixture.outputs();
    let mut no_authority = |_| Err::<tools::AuthorityEvidenceV1, _>(());
    let mut no_attempt = |_, _, _| Err::<tools::AttemptEvidenceV1, _>(());
    let replay = execute_tool_turn(
        &mut client,
        &mut executor,
        &mut outputs,
        Some(storage_revision),
        moira_protocol::WorkflowIdV1([16; 16]),
        &result(),
        tools::ToolBudgetV1 {
            max_elapsed_millis: 100,
            max_output_bytes: 1_024,
        },
        &mut no_authority,
        &mut no_attempt,
    )
    .unwrap();
    assert!(matches!(replay, ToolTurnOutcomeV1::Completed { .. }));
    assert_eq!(executor.calls, 2);
}

#[test]
fn unavailable_authority_stops_before_process_dispatch() {
    let fixture = Fixture::new();
    let mut client = fixture.client();
    let mut outputs = fixture.outputs();
    let mut executor = Executor { calls: 0 };
    let mut no_authority = |_| Err::<tools::AuthorityEvidenceV1, _>("authority unavailable");
    let mut observe = |task_key, incarnation, attempt| {
        Ok::<_, ()>(tools::AttemptEvidenceV1 {
            task_key,
            incarnation,
            attempt,
        })
    };
    let error = execute_tool_turn(
        &mut client,
        &mut executor,
        &mut outputs,
        None,
        moira_protocol::WorkflowIdV1([17; 16]),
        &result(),
        tools::ToolBudgetV1 {
            max_elapsed_millis: 100,
            max_output_bytes: 1_024,
        },
        &mut no_authority,
        &mut observe,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        moira_local_harness::tool_turn::ToolTurnErrorV1::Authority("authority unavailable")
    ));
    assert_eq!(executor.calls, 0);
}
