#![cfg(unix)]

use moira_client::AgentParticipationServicePort;
use moira_core::agent_participation::{LimitsV1 as DomainLimits, persistence::CodecLimitsV1};
use moira_host::{AgentGateway, AgentOutputStoreV1};
use moira_local_harness::agent_runtime::{
    AgentCommandMaterializerV1, AgentDriveErrorV1, AgentDriveOutcomeV1, AgentFenceCauseV1,
    AgentWorkerOutcomeV1, PreparedParticipationV1, dispatch_prepared, drive_current, load_prepared,
    reconcile_fenced,
};
use moira_protocol::{WorkflowIdV1, agent, agent_participation as p, deliberation as d};
use moira_service::agent_participation::{AgentParticipationService, PermissionAccessV1};
use moira_sqlite::agent_output::{AgentOutputSqliteStore, AgentOutputStoreLimitsV1};
use moira_sqlite::agent_participation::{
    AgentParticipationSqliteStore, AgentParticipationStoreErrorV1, AgentParticipationStoreLimitsV1,
};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Port(AgentParticipationService<AgentParticipationSqliteStore>);

impl AgentParticipationServicePort for Port {
    type Error = AgentParticipationStoreErrorV1;

    fn execute_participation(&mut self, command: p::CommandV1) -> Result<p::ReplyV1, Self::Error> {
        self.0.execute(command)
    }
}

struct Gateway {
    replies: VecDeque<Result<agent::ReplyV1, &'static str>>,
    mutations: usize,
}

impl AgentGateway for Gateway {
    type Error = &'static str;

    fn execute_agent(&mut self, command: agent::CommandV1) -> Result<agent::ReplyV1, Self::Error> {
        if matches!(
            command,
            agent::CommandV1::Open { .. }
                | agent::CommandV1::Prompt { .. }
                | agent::CommandV1::DecidePermission { .. }
                | agent::CommandV1::Advance { .. }
                | agent::CommandV1::Cancel { .. }
                | agent::CommandV1::Close { .. }
                | agent::CommandV1::Reattach { .. }
        ) {
            self.mutations += 1;
        }
        self.replies.pop_front().expect("planned gateway reply")
    }
}

struct Materializer {
    prompt: Vec<u8>,
    corrupt_execution: bool,
}

impl AgentCommandMaterializerV1 for Materializer {
    type Error = ();

    fn materialize(
        &mut self,
        view: &p::ParticipationViewV1,
        operation: p::OperationV1,
        now: u64,
    ) -> Result<agent::CommandV1, Self::Error> {
        let session = agent::SessionIdV1(view.binding.session.0);
        Ok(match operation {
            p::OperationV1::Open { key, execution } => agent::CommandV1::Open {
                request: agent::OpenRequestV1 {
                    session,
                    execution: execution_view(
                        view,
                        if self.corrupt_execution {
                            p::ExecutionIdV1([99; 16])
                        } else {
                            execution
                        },
                        key,
                    ),
                    workspace: [40; 16],
                    budget: agent::SessionBudgetV1 {
                        max_elapsed_millis: 1_000,
                        max_turns: 4,
                        max_events: 32,
                        max_event_bytes: 4_096,
                        max_prompt_bytes: 4_096,
                        max_permission_requests: 4,
                    },
                },
                now,
            },
            p::OperationV1::Prompt {
                key,
                turn,
                execution,
                prompt_digest,
                ..
            } => agent::CommandV1::Prompt {
                request: agent::PromptRequestV1 {
                    session,
                    turn: agent::TurnIdV1(turn.0),
                    expected_revision: view.session_revision.unwrap(),
                    execution: execution_view(view, execution, key),
                    prompt_digest: agent::DigestV1(prompt_digest),
                    prompt: self.prompt.clone(),
                },
                now,
            },
            p::OperationV1::DecidePermission {
                key,
                execution,
                request,
                option,
                ..
            } => agent::CommandV1::DecidePermission {
                session,
                expected_revision: view.session_revision.unwrap(),
                execution: execution_view(view, execution, key),
                decision: agent::PermissionDecisionV1 {
                    request: agent::PermissionRequestIdV1(request.0),
                    option: agent::PermissionOptionIdV1(option.0),
                },
                now,
            },
            p::OperationV1::Advance { key, turn } => agent::CommandV1::Advance {
                session,
                turn: agent::TurnIdV1(turn.0),
                expected_revision: view.session_revision.unwrap(),
                idempotency_key: agent::IdempotencyKeyV1(key.0),
                now,
            },
            p::OperationV1::Reattach { key, execution, .. } => agent::CommandV1::Reattach {
                session,
                expected_revision: view.session_revision.unwrap(),
                execution: execution_view(view, execution, key),
                now,
            },
            _ => return Err(()),
        })
    }
}

struct Fixture {
    database: PathBuf,
    output_database: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let identity = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir();
        Self {
            database: directory.join(format!(
                "moira-agent-runtime-{}-{identity}.db",
                std::process::id(),
            )),
            output_database: directory.join(format!(
                "moira-agent-runtime-output-{}-{identity}.db",
                std::process::id(),
            )),
        }
    }

    fn port(&self) -> Port {
        Port(AgentParticipationService::new(
            AgentParticipationSqliteStore::open(&self.database, [1; 16], limits()).unwrap(),
        ))
    }

    fn outputs(&self) -> AgentOutputSqliteStore {
        AgentOutputSqliteStore::open(
            &self.output_database,
            [2; 16],
            AgentOutputStoreLimitsV1 {
                max_entries: 4,
                max_output_bytes: 4_096,
                max_page_assistant_bytes: 1_024,
                max_database_pages: 64,
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-journal", ".owner.lock"] {
            let _ = fs::remove_file(format!("{}{}", self.database.display(), suffix));
            let _ = fs::remove_file(format!("{}{}", self.output_database.display(), suffix));
        }
    }
}

#[test]
fn prepared_open_and_prompt_are_observed_with_exact_permission_evidence() {
    let fixture = Fixture::new();
    let mut port = fixture.port();
    let mut outputs = fixture.outputs();
    let binding = binding();
    let admitted = apply(&mut port, None, p::MutationV1::Admit(binding));
    let open = p::OperationV1::Open {
        key: p::IdempotencyKeyV1([20; 16]),
        execution: p::ExecutionIdV1([21; 16]),
    };
    let prepared = apply(
        &mut port,
        Some(admitted.storage_revision),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: admitted.participation.revision,
            operation: open,
        },
    );
    assert_eq!(
        load_prepared(&mut port, binding.participation).unwrap(),
        prepared
    );
    let mut gateway = Gateway {
        replies: VecDeque::from([
            Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(
                session(agent::PhaseV1::Ready, 0, None, None),
            ))),
            Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Events(
                agent::EventPageV1 {
                    session: agent::SessionIdV1(binding.session.0),
                    entries: vec![],
                    next_after: None,
                },
            ))),
        ]),
        mutations: 0,
    };
    let mut materializer = Materializer {
        prompt: b"grounded prompt".to_vec(),
        corrupt_execution: false,
    };
    let AgentWorkerOutcomeV1::Progress(ready) = drive_current(
        &mut port,
        &mut gateway,
        &mut materializer,
        &mut outputs,
        binding.participation,
        10,
        16,
    )
    .unwrap() else {
        panic!("prepared operation must progress")
    };
    let (storage_revision, ready) = observed(ready);
    assert_eq!(ready.phase, p::PhaseV1::Ready);

    let prompt_bytes = b"grounded prompt".to_vec();
    let turn = p::AgentTurnIdV1([22; 16]);
    let prompt = p::OperationV1::Prompt {
        key: p::IdempotencyKeyV1([23; 16]),
        turn,
        execution: p::ExecutionIdV1([24; 16]),
        prompt_digest: Sha256::digest(&prompt_bytes).into(),
        attempt: binding.attempt,
    };
    let prepared = apply(
        &mut port,
        Some(storage_revision),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: ready.revision,
            operation: prompt,
        },
    );
    let permission = agent::PermissionRequestV1 {
        request: agent::PermissionRequestIdV1([25; 16]),
        turn: agent::TurnIdV1(turn.0),
        capability: b"terminal.execute".to_vec(),
        description: b"run exact tests".to_vec(),
        options: vec![agent::PermissionOptionV1 {
            id: agent::PermissionOptionIdV1([26; 16]),
            kind: agent::PermissionOptionKindV1::AllowOnce,
            label: b"allow once".to_vec(),
        }],
    };
    gateway.replies = VecDeque::from([
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(
            session(
                agent::PhaseV1::AwaitingPermission,
                1,
                Some(agent::TurnIdV1(turn.0)),
                Some(1),
            ),
        ))),
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Events(
            agent::EventPageV1 {
                session: agent::SessionIdV1(binding.session.0),
                entries: vec![
                    agent::EventRecordV1 {
                        cursor: 0,
                        event: agent::EventV1::AssistantMessage {
                            bytes: b"hel".to_vec(),
                        },
                    },
                    agent::EventRecordV1 {
                        cursor: 1,
                        event: agent::EventV1::Permission(permission.clone()),
                    },
                ],
                next_after: None,
            },
        ))),
    ]);
    let outcome = dispatch_prepared(
        &mut port,
        &mut gateway,
        &mut materializer,
        &mut outputs,
        prepared,
        11,
        16,
    )
    .unwrap();
    let (storage_revision, awaiting) = observed(outcome);
    assert_eq!(awaiting.phase, p::PhaseV1::AwaitingPermission);
    let retained = awaiting.permission.unwrap();
    assert_eq!(retained.request.0, permission.request.0);
    assert_eq!(retained.turn.0, permission.turn.0);
    assert_eq!(
        retained.capability_digest,
        <[u8; 32]>::from(Sha256::digest(&permission.capability))
    );
    assert_ne!(retained.options_digest, [0; 32]);

    let selection = p::PermissionSelectionV1 {
        expected_storage_revision: Some(storage_revision),
        participation: binding.participation,
        expected_revision: awaiting.revision,
        key: p::IdempotencyKeyV1([27; 16]),
        execution: p::ExecutionIdV1([28; 16]),
        request: retained.request,
        option: retained.option_ids[0],
        capability_digest: retained.capability_digest,
        attempt: binding.attempt,
    };
    let prepared = match port
        .0
        .execute_permission_authorized(
            PermissionAccessV1 {
                decision: [31; 16],
                subject: [32; 16],
                capability_digest: retained.capability_digest,
                epoch: 1,
            },
            p::CommandV1::SelectPermission(selection),
        )
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::Applied {
                storage_revision,
                participation,
                ..
            } => PreparedParticipationV1 {
                storage_revision,
                participation,
            },
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    };
    gateway.replies = VecDeque::from([
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(
            session(
                agent::PhaseV1::TurnActive,
                2,
                Some(agent::TurnIdV1(turn.0)),
                Some(2),
            ),
        ))),
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Events(
            agent::EventPageV1 {
                session: agent::SessionIdV1(binding.session.0),
                entries: vec![agent::EventRecordV1 {
                    cursor: 2,
                    event: agent::EventV1::AssistantMessage {
                        bytes: b"lo ".to_vec(),
                    },
                }],
                next_after: None,
            },
        ))),
    ]);
    let outcome = dispatch_prepared(
        &mut port,
        &mut gateway,
        &mut materializer,
        &mut outputs,
        prepared,
        12,
        16,
    )
    .unwrap();
    let (storage_revision, active) = observed(outcome);
    assert_eq!(active.phase, p::PhaseV1::TurnActive);
    let prepared = apply(
        &mut port,
        Some(storage_revision),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: active.revision,
            operation: p::OperationV1::Advance {
                key: p::IdempotencyKeyV1([29; 16]),
                turn,
            },
        },
    );
    gateway.replies = VecDeque::from([
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(
            session(agent::PhaseV1::Ready, 3, None, Some(4)),
        ))),
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Events(
            agent::EventPageV1 {
                session: agent::SessionIdV1(binding.session.0),
                entries: vec![
                    agent::EventRecordV1 {
                        cursor: 3,
                        event: agent::EventV1::AssistantMessage {
                            bytes: b"world".to_vec(),
                        },
                    },
                    agent::EventRecordV1 {
                        cursor: 4,
                        event: agent::EventV1::TurnFinished {
                            turn: agent::TurnIdV1(turn.0),
                            disposition: agent::DispositionV1::Completed,
                            stop_reason: agent::StopReasonV1::EndTurn,
                            output_digest: None,
                        },
                    },
                ],
                next_after: None,
            },
        ))),
    ]);
    let outcome = dispatch_prepared(
        &mut port,
        &mut gateway,
        &mut materializer,
        &mut outputs,
        prepared,
        13,
        16,
    )
    .unwrap();
    let (_, terminal) = observed(outcome);
    assert_eq!(terminal.phase, p::PhaseV1::Terminal);
    let expected_digest = <[u8; 32]>::from(Sha256::digest(b"hello world"));
    assert_eq!(terminal.terminal.unwrap().output_digest, expected_digest);
    let (record, bytes) = outputs
        .get(binding.participation.0, turn.0)
        .unwrap()
        .unwrap();
    assert_eq!(record.output_digest, expected_digest);
    assert_eq!(bytes, b"hello world");
    assert_eq!(gateway.mutations, 4);
}

#[test]
fn transport_ambiguity_fences_and_stale_prepared_receipt_cannot_redispatch() {
    let fixture = Fixture::new();
    let mut port = fixture.port();
    let mut outputs = fixture.outputs();
    let binding = binding();
    let admitted = apply(&mut port, None, p::MutationV1::Admit(binding));
    let operation = p::OperationV1::Open {
        key: p::IdempotencyKeyV1([30; 16]),
        execution: p::ExecutionIdV1([31; 16]),
    };
    let prepared = apply(
        &mut port,
        Some(admitted.storage_revision),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: admitted.participation.revision,
            operation,
        },
    );
    let receipt = PreparedParticipationV1 {
        storage_revision: prepared.storage_revision,
        participation: prepared.participation.clone(),
    };
    let mut gateway = Gateway {
        replies: VecDeque::from([Err("lost reply")]),
        mutations: 0,
    };
    let mut materializer = Materializer {
        prompt: vec![],
        corrupt_execution: false,
    };
    let outcome = dispatch_prepared(
        &mut port,
        &mut gateway,
        &mut materializer,
        &mut outputs,
        receipt.clone(),
        10,
        4,
    )
    .unwrap();
    let fenced_receipt = match outcome {
        AgentDriveOutcomeV1::Fenced {
            storage_revision,
            participation,
            cause: AgentFenceCauseV1::Gateway("lost reply"),
        } if participation.phase == p::PhaseV1::FencedUnknown => PreparedParticipationV1 {
            storage_revision,
            participation,
        },
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(gateway.mutations, 1);

    let mut second = Gateway {
        replies: VecDeque::new(),
        mutations: 0,
    };
    assert!(matches!(
        dispatch_prepared(
            &mut port,
            &mut second,
            &mut materializer,
            &mut outputs,
            receipt,
            10,
            4,
        ),
        Err(AgentDriveErrorV1::InvalidState)
    ));
    assert_eq!(second.mutations, 0);

    let mut recovery = Gateway {
        replies: VecDeque::from([
            Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(
                session(agent::PhaseV1::Ready, 0, None, None),
            ))),
            Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Events(
                agent::EventPageV1 {
                    session: agent::SessionIdV1(binding.session.0),
                    entries: vec![],
                    next_after: None,
                },
            ))),
        ]),
        mutations: 0,
    };
    let recovered =
        reconcile_fenced(&mut port, &mut recovery, &mut outputs, fenced_receipt, 4).unwrap();
    assert!(matches!(
        recovered,
        AgentDriveOutcomeV1::Observed { participation, .. }
            if participation.phase == p::PhaseV1::Ready
    ));
    assert_eq!(recovery.mutations, 0);
}

#[test]
fn mismatched_materialized_identity_is_refused_before_gateway_dispatch() {
    let fixture = Fixture::new();
    let mut port = fixture.port();
    let mut outputs = fixture.outputs();
    let binding = binding();
    let admitted = apply(&mut port, None, p::MutationV1::Admit(binding));
    let prepared = apply(
        &mut port,
        Some(admitted.storage_revision),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: admitted.participation.revision,
            operation: p::OperationV1::Open {
                key: p::IdempotencyKeyV1([32; 16]),
                execution: p::ExecutionIdV1([33; 16]),
            },
        },
    );
    let mut gateway = Gateway {
        replies: VecDeque::new(),
        mutations: 0,
    };
    let mut materializer = Materializer {
        prompt: vec![],
        corrupt_execution: true,
    };
    assert!(matches!(
        dispatch_prepared(
            &mut port,
            &mut gateway,
            &mut materializer,
            &mut outputs,
            prepared,
            10,
            4,
        ),
        Err(AgentDriveErrorV1::InvalidProjection)
    ));
    assert_eq!(gateway.mutations, 0);
}

#[test]
fn idle_detachment_is_marked_then_confirmed_and_reattached_without_terminal_identity_input() {
    let fixture = Fixture::new();
    let mut port = fixture.port();
    let mut outputs = fixture.outputs();
    let binding = binding();
    let admitted = apply(&mut port, None, p::MutationV1::Admit(binding));
    let prepared = apply(
        &mut port,
        Some(admitted.storage_revision),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: admitted.participation.revision,
            operation: p::OperationV1::Open {
                key: p::IdempotencyKeyV1([50; 16]),
                execution: p::ExecutionIdV1([51; 16]),
            },
        },
    );
    let mut gateway = Gateway {
        replies: VecDeque::from([
            Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(
                session(agent::PhaseV1::Ready, 0, None, None),
            ))),
            Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Events(
                agent::EventPageV1 {
                    session: agent::SessionIdV1(binding.session.0),
                    entries: vec![],
                    next_after: None,
                },
            ))),
        ]),
        mutations: 0,
    };
    let mut materializer = Materializer {
        prompt: vec![],
        corrupt_execution: false,
    };
    let ready = dispatch_prepared(
        &mut port,
        &mut gateway,
        &mut materializer,
        &mut outputs,
        prepared,
        10,
        4,
    )
    .unwrap();
    let (_, ready) = observed(ready);
    assert_eq!(ready.phase, p::PhaseV1::Ready);

    gateway.replies = VecDeque::from([Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(
        session(agent::PhaseV1::ReattachRequired, 0, None, None),
    )))]);
    let AgentWorkerOutcomeV1::Progress(marked) = drive_current(
        &mut port,
        &mut gateway,
        &mut materializer,
        &mut outputs,
        binding.participation,
        11,
        4,
    )
    .unwrap() else {
        panic!("detachment must be durably marked")
    };
    let (storage_revision, marked) = observed(marked);
    assert_eq!(marked.phase, p::PhaseV1::ReattachRequired);
    assert_eq!(gateway.mutations, 1);

    let confirmation = match port
        .execute_participation(p::CommandV1::ReattachConfirmation {
            participation: binding.participation,
        })
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::ReattachConfirmation(Some(value)) => value,
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(confirmation.storage_revision, storage_revision);
    let prepared = match port
        .execute_participation(p::CommandV1::ConfirmReattach(p::ConfirmedReattachV1 {
            participation: binding.participation,
            confirmation_digest: confirmation.confirmation_digest,
        }))
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::Applied {
                storage_revision,
                participation,
                ..
            } => PreparedParticipationV1 {
                storage_revision,
                participation,
            },
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    };
    assert!(matches!(
        prepared.participation.pending,
        Some(p::OperationV1::Reattach { attempt, .. }) if attempt == binding.attempt
    ));

    gateway.replies = VecDeque::from([
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Session(
            session(agent::PhaseV1::Ready, 1, None, None),
        ))),
        Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::Events(
            agent::EventPageV1 {
                session: agent::SessionIdV1(binding.session.0),
                entries: vec![],
                next_after: None,
            },
        ))),
    ]);
    let reattached = dispatch_prepared(
        &mut port,
        &mut gateway,
        &mut materializer,
        &mut outputs,
        prepared,
        12,
        4,
    )
    .unwrap();
    assert!(matches!(
        reattached,
        AgentDriveOutcomeV1::Observed { participation, .. }
            if participation.phase == p::PhaseV1::Ready
                && participation.pending.is_none()
    ));
    assert_eq!(gateway.mutations, 2);
}

fn apply(
    port: &mut Port,
    revision: Option<u64>,
    mutation: p::MutationV1,
) -> PreparedParticipationV1 {
    match port
        .execute_participation(p::CommandV1::Apply {
            expected_storage_revision: revision,
            mutation: Box::new(mutation),
        })
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::Applied {
                storage_revision,
                participation,
                ..
            } => PreparedParticipationV1 {
                storage_revision,
                participation,
            },
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
}

fn observed(outcome: AgentDriveOutcomeV1<&'static str>) -> (u64, p::ParticipationViewV1) {
    match outcome {
        AgentDriveOutcomeV1::Observed {
            storage_revision,
            participation,
        } => (storage_revision, participation),
        other => panic!("unexpected {other:?}"),
    }
}

fn execution_view(
    view: &p::ParticipationViewV1,
    execution: p::ExecutionIdV1,
    key: p::IdempotencyKeyV1,
) -> agent::ExecutionV1 {
    agent::ExecutionV1 {
        id: agent::ExecutionIdV1(execution.0),
        attempt: agent::AttemptV1 {
            task_key: view.binding.attempt.task_key,
            incarnation: view.binding.attempt.incarnation,
            attempt: view.binding.attempt.attempt,
        },
        principal: agent::PrincipalIdV1([41; 16]),
        profile: agent::ProfileIdV1(view.binding.profile.0),
        idempotency_key: agent::IdempotencyKeyV1(key.0),
        context: vec![agent::ContextIdV1([42; 16])],
        tools: vec![agent::ToolIdV1([43; 16])],
        budget: agent::ExecutionBudgetV1 {
            max_elapsed_millis: 1_000,
            max_input_tokens: 1_000,
            max_output_tokens: 1_000,
            max_tool_calls: 4,
            max_context_bytes: 4_096,
            max_output_bytes: 4_096,
        },
    }
}

fn session(
    phase: agent::PhaseV1,
    revision: u64,
    active_turn: Option<agent::TurnIdV1>,
    last_event_cursor: Option<u64>,
) -> agent::SessionViewV1 {
    agent::SessionViewV1 {
        session: agent::SessionIdV1([6; 16]),
        revision,
        phase,
        profile_document_digest: agent::DigestV1([44; 32]),
        active_turn,
        last_event_cursor,
        last_context: None,
        capabilities: agent::CapabilitiesV1 {
            resume_session: true,
            permissions: true,
            terminal: true,
            ..agent::CapabilitiesV1::default()
        },
    }
}

fn binding() -> p::BindingV1 {
    p::BindingV1 {
        participation: p::ParticipationIdV1([2; 16]),
        workflow: WorkflowIdV1([3; 16]),
        deliberation: d::DeliberationIdV1([4; 16]),
        member: d::EnsembleMemberIdV1([5; 16]),
        session: p::AgentSessionIdV1([6; 16]),
        profile: p::ProfileIdV1([7; 16]),
        attempt: p::AttemptV1 {
            task_key: [8; 16],
            incarnation: 1,
            attempt: 1,
        },
        method: d::MethodRefV1 {
            machine_digest: [9; 32],
            state_digest: [10; 32],
        },
        grounding: d::GroundingRefV1 {
            authority_digest: [11; 32],
            surface_digest: [12; 32],
            depth: d::SurfaceDepthV1::Activated,
        },
        persona: d::PersonaLensV1::Researcher,
    }
}

fn limits() -> AgentParticipationStoreLimitsV1 {
    AgentParticipationStoreLimitsV1 {
        codec: CodecLimitsV1 {
            max_bytes: 65_536,
            domain: DomainLimits {
                max_participations: 4,
            },
        },
        max_database_pages: 64,
    }
}
