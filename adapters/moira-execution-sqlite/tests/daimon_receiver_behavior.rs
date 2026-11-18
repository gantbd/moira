#![cfg(unix)]
use chronos_engine::{Agent, EngineLimits, StateRevision, TaskSpecV2};
use daimon_harness::*;
use moira_core::EffectId;
use moira_execution_sqlite::{
    chronos::{ChronosReceiver, ReceiverLimits},
    daimon::*,
};
use moira_sqlite::Timing;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "moira-daimon-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn receiver(&self) -> FakeDaimonReceiver {
        FakeDaimonReceiver::open(self.0.join("daimon.db"), [7; 16], limits()).unwrap()
    }
    fn agora(&self) -> moira_agora_sqlite::AgoraReceiver {
        moira_agora_sqlite::AgoraReceiver::open(
            self.0.join("agora.db"),
            moira_agora_sqlite::AgoraIdentity {
                owner: [7; 16],
                source: [10; 16],
            },
            moira_agora_sqlite::AgoraReceiverLimits {
                engine: agora_engine::Limits::default(),
                max_snapshot_bytes: 65536,
                max_database_pages: 64,
                timing: Timing::Off,
            },
        )
        .unwrap()
    }
    fn chronos(&self) -> ChronosReceiver {
        ChronosReceiver::open(
            self.0.join("chronos.db"),
            [7; 16],
            ReceiverLimits {
                engine: EngineLimits {
                    max_tasks: 2,
                    ..EngineLimits::default()
                },
                max_receipts: 2,
                max_snapshot_bytes: 16384,
                max_intent_bytes: 2048,
                max_database_pages: 64,
                timing: Timing::Off,
            },
        )
        .unwrap()
    }
    fn claim(&self) -> ChronosReceiver {
        let mut r = self.chronos();
        r.admit(submission(), StateRevision::ZERO, TaskSpecV2::new("work"))
            .unwrap();
        let grant = r
            .authorize_worker(Agent::new("worker"), now(), Duration::from_secs(60), 1)
            .unwrap();
        r.claim(
            submission(),
            r.revision(),
            &grant,
            now(),
            Duration::from_secs(30),
        )
        .unwrap();
        r
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for name in [
            "daimon.db",
            "daimon.db-journal",
            "daimon.db.owner.lock",
            "chronos.db",
            "chronos.db-journal",
            "chronos.db.owner.lock",
            "origin.db",
            "origin.db-journal",
            "origin.db.owner.lock",
            "agora.db",
            "agora.db-journal",
            "agora.db.owner.lock",
        ] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}
fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(10)
}
fn submission() -> EffectId {
    EffectId::new([1; 16])
}
fn effect() -> EffectId {
    EffectId::new([2; 16])
}
fn limits() -> FakeReceiverLimits {
    FakeReceiverLimits {
        harness: Limits::default(),
        max_entries: 2,
        max_record_bytes: 4096,
        max_database_pages: 64,
        timing: Timing::Off,
    }
}
fn request() -> HarnessRequestV1 {
    HarnessRequestV1::new(
        ExecutionId::new([2; 16]),
        TaskAttemptRefV1::new([1; 16], 0, 1),
        PrincipalId::new([3; 16]),
        HarnessProfileId::new([4; 16]),
        IdempotencyKey::new([5; 16]),
        vec![],
        vec![ToolCapabilityId::new([6; 16])],
        ExecutionBudgetV1 {
            max_elapsed_millis: 100,
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 1,
            max_context_bytes: 100,
            max_output_bytes: 100,
        },
    )
}
struct HostAuthority {
    engine: aegis_authority::Engine,
    grant: aegis_authority::GrantV1,
}
impl HostAuthority {
    fn borrow(&mut self) -> ExecutionAuthority<'_> {
        ExecutionAuthority {
            engine: &mut self.engine,
            grant: &self.grant,
            subject: aegis_authority::PrincipalId::new([3; 16]),
            session: None,
        }
    }
}
fn authority_until(expiry: u64) -> HostAuthority {
    authority_for(expiry, &[4, 6])
}
fn authority_for(expiry: u64, resources: &[u8]) -> HostAuthority {
    use aegis_authority::{
        ActionSetV1, ActionV1, AuthorityEpoch, Engine, EngineId, EngineNamespaceV1,
        GrantDescriptorV1, Limits, LogicalTime, PrincipalId, ResourceId, ResourceScopeV1,
    };
    let id = EngineId::new([8; 16]);
    let mut engine = Engine::try_new(id, Limits::default()).unwrap();
    let scopes = resources
        .iter()
        .copied()
        .map(|n| {
            ResourceScopeV1::new(
                EngineNamespaceV1::Daimon,
                ResourceId::new([n; 16]),
                ActionSetV1::singleton(ActionV1::Execute),
            )
        })
        .collect();
    let grant = engine
        .issue(
            GrantDescriptorV1::new(
                PrincipalId::new([3; 16]),
                None,
                id,
                AuthorityEpoch(0),
                LogicalTime(expiry),
                scopes,
            ),
            LogicalTime(1),
        )
        .unwrap();
    HostAuthority { engine, grant }
}
fn authority() -> HostAuthority {
    authority_until(30)
}
fn reply() -> FakeReply {
    FakeReply {
        disposition: ExecutionDispositionV1::Completed,
        output_bytes: 5,
        artifacts: vec![],
    }
}
fn execute(
    r: &mut FakeDaimonReceiver,
    c: &mut ChronosReceiver,
) -> Result<Execution, FakeReceiverError> {
    r.execute(
        effect(),
        request(),
        reply(),
        authority().borrow(),
        LogicalTime(10),
        ExecutionAttempt {
            chronos: c,
            submission: submission(),
            now: now(),
        },
    )
}

#[test]
fn durable_fake_result_replays_after_both_receivers_restart_without_dispatch() {
    let f = Fixture::new();
    let mut c = f.claim();
    let mut r = f.receiver();
    let first = execute(&mut r, &mut c).unwrap();
    assert!(!first.replayed);
    assert!(matches!(
        first.evidence,
        ExecutionEvidence::ProviderResult(_)
    ));
    assert_eq!(r.usage().provider_calls, 1);
    drop(r);
    drop(c);
    let mut c = f.claim();
    let mut r = f.receiver();
    let replay = execute(&mut r, &mut c).unwrap();
    assert!(replay.replayed);
    assert_eq!(
        replay,
        Execution {
            replayed: true,
            ..first
        }
    );
    assert_eq!(r.usage().provider_calls, 0);
}

#[test]
fn current_authority_and_exact_live_attempt_are_required_before_new_dispatch() {
    let f = Fixture::new();
    let mut c = f.claim();
    let mut r = f.receiver();
    let mut expired = authority_until(10);
    assert!(matches!(
        r.execute(
            effect(),
            request(),
            reply(),
            expired.borrow(),
            LogicalTime(10),
            ExecutionAttempt {
                chronos: &mut c,
                submission: submission(),
                now: now()
            }
        ),
        Err(FakeReceiverError::AuthorityRefused)
    ));
    drop(c);
    let mut c = f.claim();
    assert!(matches!(
        execute(&mut r, &mut c),
        Err(FakeReceiverError::AttemptRefused)
    ));
    assert_eq!(r.usage().provider_calls, 0);
}

#[test]
fn conflicting_request_or_fake_receipt_cannot_replace_retained_result() {
    let f = Fixture::new();
    let mut c = f.claim();
    let mut r = f.receiver();
    execute(&mut r, &mut c).unwrap();
    let mut changed = request();
    changed.budget.max_input_tokens = 99;
    assert!(matches!(
        r.execute(
            effect(),
            changed,
            reply(),
            authority().borrow(),
            LogicalTime(10),
            ExecutionAttempt {
                chronos: &mut c,
                submission: submission(),
                now: now()
            }
        ),
        Err(FakeReceiverError::IntentConflict)
    ));
    let mut changed = reply();
    changed.disposition = ExecutionDispositionV1::Failed;
    assert!(matches!(
        r.execute(
            effect(),
            request(),
            changed,
            authority().borrow(),
            LogicalTime(10),
            ExecutionAttempt {
                chronos: &mut c,
                submission: submission(),
                now: now()
            }
        ),
        Err(FakeReceiverError::IntentConflict)
    ));
    assert_eq!(r.usage().provider_calls, 1);
}

#[test]
fn profile_and_each_tool_are_required_and_revocation_blocks_retained_delivery() {
    for (resources, empty_tools, allowed) in [
        (&[4][..], true, true),
        (&[6][..], false, false),
        (&[4][..], false, false),
    ] {
        let f = Fixture::new();
        let mut c = f.claim();
        let mut r = f.receiver();
        let mut auth = authority_for(30, resources);
        let mut req = request();
        if empty_tools {
            req.tools.clear()
        }
        let result = r.execute(
            effect(),
            req,
            reply(),
            auth.borrow(),
            LogicalTime(10),
            ExecutionAttempt {
                chronos: &mut c,
                submission: submission(),
                now: now(),
            },
        );
        assert_eq!(result.is_ok(), allowed);
        assert_eq!(r.usage().provider_calls, if allowed { 1 } else { 0 });
    }
    let f = Fixture::new();
    let mut c = f.claim();
    let mut r = f.receiver();
    execute(&mut r, &mut c).unwrap();
    let mut auth = authority();
    auth.engine.revoke_epoch().unwrap();
    assert!(matches!(
        r.execute(
            effect(),
            request(),
            reply(),
            auth.borrow(),
            LogicalTime(10),
            ExecutionAttempt {
                chronos: &mut c,
                submission: submission(),
                now: now()
            }
        ),
        Err(FakeReceiverError::AuthorityRefused)
    ));
    assert_eq!(r.usage().provider_calls, 1);
}

#[test]
fn four_store_workflow_completes_or_fences_without_duplicate_effects() {
    use moira_core::persistence::CodecLimits;
    use moira_core::{
        CommandV1, Engine, ParticipantRefV1, TransitionV1, WorkflowPhaseV1, WorkflowRevision,
    };
    use moira_sqlite::{SqliteStore, StoreLimits};
    for disposition in [
        ExecutionDispositionV1::Completed,
        ExecutionDispositionV1::Uncertain,
    ] {
        let f = Fixture::new();
        let mut agora = f.agora();
        for (id, kind) in [
            ("human", agora_engine::ParticipantKind::Human),
            ("agent", agora_engine::ParticipantKind::Agent),
        ] {
            agora
                .apply(agora_engine::RevisionedCommandV2 {
                    expected_revision: agora.revision(),
                    command: agora_engine::CommandV2::RegisterParticipant(
                        agora_engine::Participant::new(id, id, kind),
                    ),
                })
                .unwrap();
        }
        for command in [
            agora_engine::CommandV2::CreateRoom {
                room_id: "room".into(),
                name: "Room".into(),
                owner: "human".into(),
                admission: agora_engine::AdmissionPolicy::Open,
            },
            agora_engine::CommandV2::Join {
                room_id: "room".into(),
                participant: "agent".into(),
            },
            agora_engine::CommandV2::Speak {
                room_id: "room".into(),
                author: "human".into(),
                at: agora_engine::LogicalTime(9),
                body: "question".into(),
            },
        ] {
            agora
                .apply(agora_engine::RevisionedCommandV2 {
                    expected_revision: agora.revision(),
                    command,
                })
                .unwrap();
        }
        let input = agora.message_ref(&"room".into(), 0).unwrap();
        let config = StoreLimits {
            codec: CodecLimits {
                max_bytes: 8192,
                max_workflows: 2,
            },
            max_database_pages: 64,
            timing: Timing::Off,
        };
        let path = f.0.join("origin.db");
        let mut origin = SqliteStore::open(&path, [7; 16], config).unwrap();
        let TransitionV1::ChronosSubmissionPrepared {
            workflow,
            effect_id: submission,
        } = origin
            .apply(
                WorkflowRevision(0),
                CommandV1::AdmitMessage {
                    message: input,
                    recipient: ParticipantRefV1::new([3; 16]),
                },
            )
            .unwrap()
            .transition
        else {
            panic!("submission")
        };
        let mut c = f.chronos();
        let admitted = c
            .admit(submission, c.revision(), TaskSpecV2::new("work"))
            .unwrap();
        origin
            .apply(
                origin.revision(),
                CommandV1::AcceptChronosAdmission {
                    workflow,
                    receipt: admitted.receipt,
                },
            )
            .unwrap();
        let grant = c
            .authorize_worker(Agent::new("worker"), now(), Duration::from_secs(60), 1)
            .unwrap();
        let attempt = c
            .claim(
                submission,
                c.revision(),
                &grant,
                now(),
                Duration::from_secs(30),
            )
            .unwrap();
        let TransitionV1::DaimonExecutionPrepared { effect_id, .. } = origin
            .apply(
                origin.revision(),
                CommandV1::AcceptChronosAttempt {
                    workflow,
                    receipt: attempt.receipt,
                },
            )
            .unwrap()
            .transition
        else {
            panic!("execution")
        };
        let mut req = request();
        req.execution_id = ExecutionId::new(effect_id.into_bytes());
        req.idempotency_key = IdempotencyKey::new(effect_id.into_bytes());
        req.task_attempt = TaskAttemptRefV1::new(
            submission.into_bytes(),
            admitted.incarnation,
            attempt.number,
        );
        let mut r = f.receiver();
        let mut fake_reply = reply();
        fake_reply.disposition = disposition;
        let first = r
            .execute(
                effect_id,
                req.clone(),
                fake_reply.clone(),
                authority().borrow(),
                LogicalTime(10),
                ExecutionAttempt {
                    chronos: &mut c,
                    submission,
                    now: now(),
                },
            )
            .unwrap();
        drop(r);
        drop(c);
        drop(origin);
        drop(agora);
        let mut origin = SqliteStore::open(&path, [7; 16], config).unwrap();
        let mut c = f.chronos();
        let mut r = f.receiver();
        let retained = r
            .execute(
                effect_id,
                req.clone(),
                fake_reply.clone(),
                authority().borrow(),
                LogicalTime(10),
                ExecutionAttempt {
                    chronos: &mut c,
                    submission,
                    now: now(),
                },
            )
            .unwrap();
        assert!(retained.replayed);
        assert_eq!(retained.receipt, first.receipt);
        assert_eq!(r.usage().provider_calls, 0);
        origin
            .apply(
                origin.revision(),
                CommandV1::AcceptDaimonResult {
                    workflow,
                    receipt: retained.receipt,
                },
            )
            .unwrap();
        let admitted_result = Engine::recover(origin.load().unwrap().unwrap().checkpoint)
            .unwrap()
            .workflow(&workflow)
            .unwrap()
            .daimon_result()
            .unwrap();
        let settled = c
            .settle_admitted_result(
                submission,
                c.revision(),
                admitted_result,
                if disposition == ExecutionDispositionV1::Completed {
                    moira_execution_sqlite::chronos::SettlementAction::Complete
                } else {
                    moira_execution_sqlite::chronos::SettlementAction::KeepUncertain
                },
                moira_execution_sqlite::chronos::SettlementContext {
                    source_owner: [7; 16],
                    now: now(),
                },
            )
            .unwrap();
        assert_eq!(
            settled.phase,
            if disposition == ExecutionDispositionV1::Completed {
                chronos_engine::TaskPhase::Succeeded
            } else {
                chronos_engine::TaskPhase::Uncertain
            }
        );
        if disposition == ExecutionDispositionV1::Uncertain {
            assert!(
                origin
                    .apply(
                        origin.revision(),
                        CommandV1::PrepareAgoraResponse { workflow }
                    )
                    .is_err()
            );
            drop(origin);
            let mut origin = SqliteStore::open(&path, [7; 16], config).unwrap();
            let e = Engine::recover(origin.load().unwrap().unwrap().checkpoint).unwrap();
            assert_eq!(
                e.workflow(&workflow).unwrap().phase(),
                WorkflowPhaseV1::FencedUncertain
            );
            assert_eq!(
                f.agora()
                    .room(&"room".into())
                    .unwrap()
                    .unwrap()
                    .messages
                    .len(),
                1
            );
            continue;
        }
        let prepared = origin
            .apply(
                origin.revision(),
                CommandV1::PrepareAgoraResponse { workflow },
            )
            .unwrap();
        let TransitionV1::AgoraResponsePrepared {
            effect_id: publication,
            ..
        } = prepared.transition
        else {
            panic!("publication")
        };
        let mut agora = f.agora();
        let expected = agora.revision();
        // Integration-owned fake status text, not a claim of generated model text.
        let response = moira_agora_sqlite::ReplyMessage {
            room_id: "room".into(),
            author: "agent".into(),
            at: agora_engine::LogicalTime(10),
            body: "Fake execution completed".into(),
        };
        let first_reply = agora
            .publish(publication, expected, response.clone())
            .unwrap();
        drop(agora);
        drop(c);
        drop(r);
        drop(origin);
        let mut agora = f.agora();
        let replay = agora.publish(publication, expected, response).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.receipt, first_reply.receipt);
        assert_eq!(agora.publication_usage().committed, 0);
        assert_eq!(
            agora.room(&"room".into()).unwrap().unwrap().messages.len(),
            2
        );
        let mut origin = SqliteStore::open(&path, [7; 16], config).unwrap();
        origin
            .apply(
                origin.revision(),
                CommandV1::AcceptAgoraPublication {
                    workflow,
                    receipt: replay.receipt,
                },
            )
            .unwrap();
        drop(origin);
        let mut origin = SqliteStore::open(&path, [7; 16], config).unwrap();
        let e = Engine::recover(origin.load().unwrap().unwrap().checkpoint).unwrap();
        assert_eq!(
            e.workflow(&workflow).unwrap().phase(),
            WorkflowPhaseV1::Completed
        );
        assert_eq!(e.revision(), WorkflowRevision(6));
        let mut c = f.chronos();
        assert_eq!(c.task_count(), 1);
        assert!(
            c.settle(
                submission,
                StateRevision::ZERO,
                &retained.settlement,
                moira_execution_sqlite::chronos::SettlementAction::Complete,
                moira_execution_sqlite::chronos::SettlementContext {
                    source_owner: [7; 16],
                    now: now()
                }
            )
            .unwrap()
            .replayed
        );
        let mut r = f.receiver();
        assert!(
            r.execute(
                effect_id,
                req,
                fake_reply,
                authority().borrow(),
                LogicalTime(10),
                ExecutionAttempt {
                    chronos: &mut c,
                    submission,
                    now: now()
                }
            )
            .unwrap()
            .replayed
        );
        assert_eq!(r.usage().provider_calls, 0);
    }
}

#[test]
fn exact_fake_result_settles_live_or_recovered_attempt_and_replays_without_new_transition() {
    use moira_execution_sqlite::chronos::{SettlementAction, SettlementContext};
    for recover in [false, true] {
        let f = Fixture::new();
        let mut c = f.claim();
        let mut r = f.receiver();
        let result = execute(&mut r, &mut c).unwrap();
        if recover {
            drop(c);
            c = f.chronos()
        }
        let ctx = SettlementContext {
            source_owner: [7; 16],
            now: now(),
        };
        let first = c
            .settle(
                submission(),
                c.revision(),
                &result.settlement,
                SettlementAction::Complete,
                ctx,
            )
            .unwrap();
        assert!(!first.replayed);
        assert_eq!(first.phase, chronos_engine::TaskPhase::Succeeded);
        drop(c);
        let mut c = f.chronos();
        let replay = c
            .settle(
                submission(),
                StateRevision::ZERO,
                &result.settlement,
                SettlementAction::Complete,
                ctx,
            )
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.revision, first.revision);
        assert_eq!(c.settlement_usage().committed, 0);
        assert_eq!(c.settlement_usage().replayed, 1);
    }
}

#[test]
fn first_slice_settlement_actions_are_explicit_and_do_not_schedule_retries() {
    use chronos_engine::TaskPhase;
    use moira_execution_sqlite::chronos::{SettlementAction, SettlementContext};
    for (disposition, action, phase) in [
        (
            ExecutionDispositionV1::Failed,
            SettlementAction::FailTerminal,
            TaskPhase::Failed,
        ),
        (
            ExecutionDispositionV1::Cancelled,
            SettlementAction::Cancel,
            TaskPhase::Cancelled,
        ),
        (
            ExecutionDispositionV1::TimedOut,
            SettlementAction::KeepUncertain,
            TaskPhase::Uncertain,
        ),
        (
            ExecutionDispositionV1::Uncertain,
            SettlementAction::KeepUncertain,
            TaskPhase::Uncertain,
        ),
    ] {
        let f = Fixture::new();
        let mut c = f.claim();
        let mut r = f.receiver();
        let mut response = reply();
        response.disposition = disposition;
        let result = r
            .execute(
                effect(),
                request(),
                response,
                authority().borrow(),
                LogicalTime(10),
                ExecutionAttempt {
                    chronos: &mut c,
                    submission: submission(),
                    now: now(),
                },
            )
            .unwrap();
        let ctx = SettlementContext {
            source_owner: [7; 16],
            now: now(),
        };
        let first = c
            .settle(submission(), c.revision(), &result.settlement, action, ctx)
            .unwrap();
        assert_eq!(first.phase, phase);
        drop(c);
        let mut c = f.chronos();
        let again = c
            .settle(submission(), c.revision(), &result.settlement, action, ctx)
            .unwrap();
        assert!(again.replayed);
        assert_eq!(again.phase, phase);
        assert_eq!(again.revision, first.revision);
    }
}
