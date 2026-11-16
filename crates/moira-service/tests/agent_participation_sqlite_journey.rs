#![cfg(unix)]

use moira_core::agent_participation::{LimitsV1 as DomainLimits, persistence::CodecLimitsV1};
use moira_host::AgentPromptStoreV1;
use moira_protocol::{WorkflowIdV1, agent_participation as p, deliberation as d};
use moira_service::{
    AccessV1,
    agent_participation::{AgentParticipationService, PermissionAccessV1},
};
use moira_sqlite::agent_participation::{
    AgentParticipationSqliteStore, AgentParticipationStoreLimitsV1,
};
use moira_sqlite::agent_prompt::{AgentPromptSqliteStore, AgentPromptStoreLimitsV1};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "moira-agent-participation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self { directory }
    }
    fn path(&self) -> PathBuf {
        self.directory.join("agent.db")
    }
    fn prompt_path(&self) -> PathBuf {
        self.directory.join("prompts.db")
    }
    fn limits(&self) -> AgentParticipationStoreLimitsV1 {
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
    fn service(&self) -> AgentParticipationService<AgentParticipationSqliteStore> {
        AgentParticipationService::new(
            AgentParticipationSqliteStore::open(self.path(), [1; 16], self.limits()).unwrap(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for name in [
            "agent.db",
            "agent.db-journal",
            "agent.db.owner.lock",
            "prompts.db",
            "prompts.db-journal",
            "prompts.db.owner.lock",
        ] {
            let _ = fs::remove_file(self.directory.join(name));
        }
        let _ = fs::remove_dir(&self.directory);
    }
}

#[test]
fn prompt_bytes_commit_before_server_derived_operation_and_replay_exactly() {
    let fixture = Fixture::new();
    let binding = binding();
    let mut service = fixture.service();
    apply(&mut service, None, p::MutationV1::Admit(binding));
    let open = p::OperationV1::Open {
        key: p::IdempotencyKeyV1([71; 16]),
        execution: p::ExecutionIdV1([72; 16]),
    };
    apply(
        &mut service,
        Some(0),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: 0,
            operation: open,
        },
    );
    apply(
        &mut service,
        Some(1),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 1,
            key: key(open),
            observation: observation(p::PhaseV1::Ready, 1, None, None, None),
        },
    );
    let mut prompts = AgentPromptSqliteStore::open(
        fixture.prompt_path(),
        [2; 16],
        AgentPromptStoreLimitsV1 {
            max_entries: 4,
            max_prompt_bytes: 128,
            max_database_pages: 64,
        },
    )
    .unwrap();
    let submission = p::PromptSubmissionV1 {
        participation: binding.participation,
        prompt: b"answer from the grounded record".to_vec(),
    };
    let first = service
        .execute_prompt_authorized(
            AccessV1 {
                read: true,
                mutate: true,
            },
            submission.clone(),
            &mut prompts,
        )
        .unwrap();
    let (storage_revision, prepared) = match first {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::Applied {
                storage_revision,
                participation,
                replayed: false,
            } => (storage_revision, participation),
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    };
    let operation = prepared.pending.unwrap();
    let digest = p::agent_prompt_digest(&submission.prompt).unwrap();
    assert!(matches!(
        operation,
        p::OperationV1::Prompt {
            prompt_digest,
            attempt,
            ..
        } if prompt_digest == digest && attempt == binding.attempt
    ));
    assert_eq!(prompts.get(digest).unwrap().unwrap(), submission.prompt);

    let replay = service
        .execute_prompt_authorized(
            AccessV1 {
                read: true,
                mutate: true,
            },
            submission.clone(),
            &mut prompts,
        )
        .unwrap();
    match replay {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::Applied {
                storage_revision: revision,
                participation,
                replayed: true,
            } => {
                assert_eq!(revision, storage_revision);
                assert_eq!(participation.pending, Some(operation));
            }
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
    let materialized = service
        .execute_prompt_materialization(
            p::PromptMaterializationRequestV1 {
                participation: binding.participation,
                expected_revision: prepared.revision,
                prompt_digest: digest,
            },
            &mut prompts,
        )
        .unwrap();
    assert!(matches!(
        materialized,
        p::ReplyV1::Accepted(value)
            if matches!(&*value, p::OutcomeV1::PromptMaterialization(value)
                if value.participation == binding.participation
                    && value.participation_revision == prepared.revision
                    && value.prompt_digest == digest
                    && value.prompt == submission.prompt)
    ));
    assert_eq!(
        service
            .execute_prompt_materialization(
                p::PromptMaterializationRequestV1 {
                    participation: binding.participation,
                    expected_revision: prepared.revision - 1,
                    prompt_digest: digest,
                },
                &mut prompts,
            )
            .unwrap(),
        p::ReplyV1::Refused(p::RefusalV1::RevisionConflict)
    );
    assert_eq!(
        service
            .execute_prompt_materialization(
                p::PromptMaterializationRequestV1 {
                    participation: binding.participation,
                    expected_revision: prepared.revision,
                    prompt_digest: [99; 32],
                },
                &mut prompts,
            )
            .unwrap(),
        p::ReplyV1::Refused(p::RefusalV1::NoPendingOperation)
    );
    assert_eq!(
        service
            .execute_prompt_authorized(
                AccessV1 {
                    read: true,
                    mutate: true,
                },
                p::PromptSubmissionV1 {
                    participation: binding.participation,
                    prompt: b"a conflicting prompt".to_vec(),
                },
                &mut prompts,
            )
            .unwrap(),
        p::ReplyV1::Refused(p::RefusalV1::OperationConflict)
    );
    assert_eq!(
        service
            .execute(p::CommandV1::SubmitPrompt(submission))
            .unwrap(),
        p::ReplyV1::Refused(p::RefusalV1::NotAuthorized)
    );
    assert_eq!(
        service
            .execute(p::CommandV1::MaterializePrompt(
                p::PromptMaterializationRequestV1 {
                    participation: binding.participation,
                    expected_revision: prepared.revision,
                    prompt_digest: digest,
                }
            ))
            .unwrap(),
        p::ReplyV1::Refused(p::RefusalV1::NotAuthorized)
    );
}

#[test]
fn authority_fence_reconciliation_terminal_and_replay_survive_reopen() {
    let fixture = Fixture::new();
    let binding = binding();
    let mut service = fixture.service();
    let admitted = apply(&mut service, None, p::MutationV1::Admit(binding));
    assert_eq!(admitted.0, 0);
    assert_eq!(admitted.1.revision, 0);
    let open = p::OperationV1::Open {
        key: p::IdempotencyKeyV1([12; 16]),
        execution: p::ExecutionIdV1([13; 16]),
    };
    let prepared = apply(
        &mut service,
        Some(0),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: 0,
            operation: open,
        },
    );
    assert_eq!(prepared.1.pending, Some(open));
    let fenced = apply(
        &mut service,
        Some(1),
        p::MutationV1::FenceUnknown {
            participation: binding.participation,
            expected_revision: 1,
            key: key(open),
        },
    );
    assert_eq!(fenced.1.phase, p::PhaseV1::FencedUnknown);
    drop(service);
    let mut service = fixture.service();
    let ready = observation(p::PhaseV1::Ready, 1, None, None, None);
    let reconciled = apply(
        &mut service,
        Some(2),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 2,
            key: key(open),
            observation: ready,
        },
    );
    assert_eq!(reconciled.1.phase, p::PhaseV1::Ready);
    let turn = p::AgentTurnIdV1([14; 16]);
    let prompt = p::OperationV1::Prompt {
        key: p::IdempotencyKeyV1([15; 16]),
        turn,
        execution: p::ExecutionIdV1([16; 16]),
        prompt_digest: [17; 32],
        attempt: binding.attempt,
    };
    apply(
        &mut service,
        Some(3),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: 3,
            operation: prompt,
        },
    );
    let permission = p::PermissionEvidenceV1 {
        request: p::PermissionRequestIdV1([18; 16]),
        turn,
        capability_digest: [19; 32],
        options_digest: [20; 32],
        option_count: 1,
        option_ids: [
            p::PermissionOptionIdV1([22; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
        ],
    };
    apply(
        &mut service,
        Some(4),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 4,
            key: key(prompt),
            observation: observation(
                p::PhaseV1::AwaitingPermission,
                2,
                Some(turn),
                Some(permission),
                None,
            ),
        },
    );
    let decision = p::OperationV1::DecidePermission {
        key: p::IdempotencyKeyV1([21; 16]),
        execution: p::ExecutionIdV1([31; 16]),
        request: permission.request,
        option: p::PermissionOptionIdV1([22; 16]),
        authority: p::PermissionAuthorityV1 {
            decision: p::AuthorityDecisionIdV1([23; 16]),
            subject: p::AuthoritySubjectIdV1([24; 16]),
            capability_digest: permission.capability_digest,
            epoch: 1,
        },
        attempt: binding.attempt,
    };
    let forged = service
        .execute(p::CommandV1::Apply {
            expected_storage_revision: Some(5),
            mutation: Box::new(p::MutationV1::Prepare {
                participation: binding.participation,
                expected_revision: 5,
                operation: decision,
            }),
        })
        .unwrap();
    assert_eq!(forged, p::ReplyV1::Refused(p::RefusalV1::NotAuthorized));
    let confirmation = match service
        .execute(p::CommandV1::PermissionConfirmation {
            participation: binding.participation,
        })
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::PermissionConfirmation(Some(value)) => value,
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    };
    let confirmed = p::ConfirmedPermissionSelectionV1 {
        participation: binding.participation,
        confirmation_digest: confirmation.confirmation_digest,
        option: p::PermissionOptionIdV1([22; 16]),
    };
    let selection = service.prepare_confirmed_permission(confirmed).unwrap();
    assert_eq!(selection.expected_storage_revision, Some(5));
    assert_eq!(selection.expected_revision, 5);
    assert_eq!(selection.request, permission.request);
    assert_eq!(selection.attempt, binding.attempt);
    let authorized = service
        .execute_permission_authorized(
            PermissionAccessV1 {
                decision: [23; 16],
                subject: [24; 16],
                capability_digest: permission.capability_digest,
                epoch: 1,
            },
            p::CommandV1::SelectPermission(selection),
        )
        .unwrap();
    assert!(matches!(authorized, p::ReplyV1::Accepted(_)));
    apply(
        &mut service,
        Some(6),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 6,
            key: selection.key,
            observation: observation(p::PhaseV1::TurnActive, 3, Some(turn), None, None),
        },
    );
    let advance = p::OperationV1::Advance {
        key: p::IdempotencyKeyV1([25; 16]),
        turn,
    };
    apply(
        &mut service,
        Some(7),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: 7,
            operation: advance,
        },
    );
    let terminal = p::TerminalEvidenceV1 {
        turn,
        disposition: p::DispositionV1::Completed,
        stop_reason: p::StopReasonV1::EndTurn,
        output_digest: [26; 32],
        provider_evidence_digest: [27; 32],
    };
    let done = apply(
        &mut service,
        Some(8),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 8,
            key: key(advance),
            observation: observation(p::PhaseV1::Terminal, 4, None, None, Some(terminal)),
        },
    );
    assert_eq!(done.1.terminal, Some(terminal));
    drop(service);
    let mut service = fixture.service();
    let view = match service
        .execute(p::CommandV1::Participation {
            participation: binding.participation,
        })
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::Participation(Some(v)) => v,
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(view.binding, binding);
    assert_eq!(view.terminal, Some(terminal));
    let replay = apply(
        &mut service,
        Some(9),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 8,
            key: key(advance),
            observation: observation(p::PhaseV1::Terminal, 4, None, None, Some(terminal)),
        },
    );
    assert!(replay.2);
    assert_eq!(replay.0, 9);
}

fn apply(
    service: &mut AgentParticipationService<AgentParticipationSqliteStore>,
    revision: Option<u64>,
    mutation: p::MutationV1,
) -> (u64, p::ParticipationViewV1, bool) {
    match service
        .execute(p::CommandV1::Apply {
            expected_storage_revision: revision,
            mutation: Box::new(mutation),
        })
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::Applied {
                storage_revision,
                participation,
                replayed,
            } => (storage_revision, participation, replayed),
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
}
fn key(value: p::OperationV1) -> p::IdempotencyKeyV1 {
    match value {
        p::OperationV1::Open { key, .. }
        | p::OperationV1::Prompt { key, .. }
        | p::OperationV1::Advance { key, .. }
        | p::OperationV1::DecidePermission { key, .. }
        | p::OperationV1::Cancel { key, .. }
        | p::OperationV1::Close { key }
        | p::OperationV1::Reattach { key, .. } => key,
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
fn observation(
    phase: p::PhaseV1,
    session_revision: u64,
    active_turn: Option<p::AgentTurnIdV1>,
    permission: Option<p::PermissionEvidenceV1>,
    terminal: Option<p::TerminalEvidenceV1>,
) -> p::ObservationV1 {
    p::ObservationV1 {
        phase,
        session_revision,
        active_turn,
        last_event_cursor: Some(session_revision),
        profile_document_digest: [30; 32],
        capabilities: p::CapabilitiesV1 {
            permissions: true,
            terminal: true,
            ..p::CapabilitiesV1::default()
        },
        permission,
        terminal,
    }
}
