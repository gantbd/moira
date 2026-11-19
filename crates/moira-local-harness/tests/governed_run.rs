use daimon_client::ModelServicePort;
use daimon_host::AuthenticatedSessionV1;
use daimon_local_daemon::unix::{Authenticator, ModelDaemon, ModelSessionPort};
use daimon_protocol as daimon;
use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, DeliberationLimitsV1, DeliberationPlanV1, EnsembleMemberIdV1,
    EnsembleMemberV1, GroundingRefV1, MaybeValueV1, MethodRefV1, OrthogonalityEvidenceV1,
    PersonaLensV1, ReportIdV1, ResolutionV1, SurfaceDepthV1,
    registry::{
        DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryRevisionV1,
        persistence::CodecLimitsV1,
    },
};
use moira_daimon::{
    CompletedToolResultV1, ExecutionIntentV1, ModelExecutionAdapter,
    ensemble::{MemberContinuationV1, MemberOutcomeV1, MemberResponseRefusalV1, MemberRunV1},
};
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};
use moira_local_harness::{
    GovernedRunErrorV1, resolve_explicit, resume_member_and_retain, run, unix::connect_daimon,
};
use moira_protocol::{EffectIdV1, TaskRefV1};
use moira_sqlite::deliberation::{DeliberationSqliteStore, DeliberationStoreLimitsV1};
use std::{
    cell::Cell,
    convert::Infallible,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

const RESPONSE: &[u8] = br#"{"schema":"moira.member-response.v1","claims":[{"kind":"empirical","statement":"local model completed","evidence_digests":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"dissent_digests":["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]}]}"#;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "moira-local-harness-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn open(&self) -> DeliberationSqliteStore {
        DeliberationSqliteStore::open(
            self.0.join("deliberation.db"),
            [50; 16],
            DeliberationStoreLimitsV1 {
                codec: CodecLimitsV1 {
                    max_bytes: 65_536,
                    max_deliberation_bytes: 32_768,
                    registry: RegistryLimitsV1 {
                        max_deliberations: 4,
                        deliberation: DeliberationLimitsV1 {
                            max_claims: 8,
                            max_criteria: 8,
                            max_candidates: 8,
                            max_members: 4,
                            max_evidence_per_report: 8,
                            max_dissent_per_report: 8,
                            max_fuel: 32,
                        },
                    },
                },
                max_database_pages: 64,
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in [
            "deliberation.db",
            "deliberation.db-journal",
            "deliberation.db.owner.lock",
        ] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}

struct Store(DeliberationRegistryV1);

impl Store {
    fn new() -> Self {
        Self(DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap())
    }
}

impl DeliberationStore for Store {
    type Error = Infallible;

    fn revision(&self) -> RegistryRevisionV1 {
        self.0.revision()
    }

    fn needs_reopen(&self) -> bool {
        false
    }

    fn deliberation(
        &self,
        id: DeliberationIdV1,
    ) -> Option<moira_core::deliberation::DeliberationCheckpointV1> {
        self.0.deliberation(id)
    }

    fn review(
        &self,
        id: DeliberationIdV1,
    ) -> Option<moira_core::deliberation::review::ReviewExchangeCheckpointV1> {
        self.0.review(id)
    }

    fn apply(
        &mut self,
        expected_revision: RegistryRevisionV1,
        command: RegistryCommandV1,
    ) -> Result<
        moira_core::deliberation::registry::RegistryReceiptV1,
        DeliberationApplyErrorV1<Self::Error>,
    > {
        self.0
            .apply(expected_revision, command)
            .map_err(DeliberationApplyErrorV1::Domain)
    }
}

struct Port {
    calls: Rc<Cell<usize>>,
    output: Vec<u8>,
}

struct ResumePort {
    tool_call: bool,
}

impl ModelServicePort for ResumePort {
    type Error = Infallible;

    fn execute_model(
        &mut self,
        _session: daimon::SessionHandleV1,
        command: daimon::ModelCommandV1,
    ) -> Result<daimon::ModelReplyV1, Self::Error> {
        let daimon::ModelCommandV1::Resume { request, .. } = command else {
            panic!("only resume is used")
        };
        let proposals = self.tool_call.then(|| {
            daimon::OutputProposalV1::ToolCall(daimon::ToolCallProposalV1 {
                request: [80; 16],
                capability: daimon::ToolCapabilityIdV1::new([81; 16]),
                provider_correlation: b"call-80".to_vec(),
                arguments: b"{}".to_vec(),
            })
        });
        Ok(daimon::ModelReplyV1::Accepted(
            daimon::ModelOutcomeV1::Executed(daimon::ModelExecutionResultV1 {
                execution_id: request.execution_id,
                disposition: daimon::ExecutionDispositionV1::Completed,
                evidence_digest: daimon::ContentDigestV1::new([24; 32]),
                input_tokens: 2,
                output_tokens: 3,
                output: if self.tool_call {
                    Vec::new()
                } else {
                    RESPONSE.to_vec()
                },
                proposals: proposals.into_iter().collect(),
            }),
        ))
    }
}

impl ModelServicePort for Port {
    type Error = Infallible;

    fn execute_model(
        &mut self,
        _session: daimon::SessionHandleV1,
        command: daimon::ModelCommandV1,
    ) -> Result<daimon::ModelReplyV1, Self::Error> {
        let daimon::ModelCommandV1::Execute { request, .. } = command else {
            panic!("only execution is used")
        };
        self.calls.set(self.calls.get() + 1);
        Ok(daimon::ModelReplyV1::Accepted(
            daimon::ModelOutcomeV1::Executed(daimon::ModelExecutionResultV1 {
                execution_id: request.execution_id,
                disposition: daimon::ExecutionDispositionV1::Completed,
                evidence_digest: daimon::ContentDigestV1::new([24; 32]),
                input_tokens: 2,
                output_tokens: 3,
                output: self.output.clone(),
                proposals: Vec::new(),
            }),
        ))
    }
}

struct SocketService {
    calls: Arc<AtomicUsize>,
}

impl daimon_local_daemon::ModelServicePort for SocketService {
    type Error = Infallible;

    fn execute_model(
        &mut self,
        _session: daimon::SessionHandleV1,
        command: daimon::ModelCommandV1,
    ) -> Result<daimon::ModelReplyV1, Self::Error> {
        let daimon::ModelCommandV1::Execute { request, .. } = command else {
            panic!("only execution is used")
        };
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(daimon::ModelReplyV1::Accepted(
            daimon::ModelOutcomeV1::Executed(daimon::ModelExecutionResultV1 {
                execution_id: request.execution_id,
                disposition: daimon::ExecutionDispositionV1::Completed,
                evidence_digest: daimon::ContentDigestV1::new([24; 32]),
                input_tokens: 2,
                output_tokens: 3,
                output: RESPONSE.to_vec(),
                proposals: Vec::new(),
            }),
        ))
    }
}

impl ModelSessionPort for SocketService {
    fn open_model_session(
        &mut self,
        _facts: AuthenticatedSessionV1,
    ) -> Result<daimon::SessionHandleV1, daimon::RefusalV1> {
        Ok(daimon::SessionHandleV1(70))
    }

    fn close_model_session(&mut self, session: daimon::SessionHandleV1) -> bool {
        session == daimon::SessionHandleV1(70)
    }
}

struct Authenticate;

impl Authenticator for Authenticate {
    type Error = Infallible;

    fn authenticate(
        &mut self,
        _stream: &std::os::unix::net::UnixStream,
    ) -> Result<AuthenticatedSessionV1, Self::Error> {
        Ok(AuthenticatedSessionV1 {
            principal: [20; 16],
            authority_context: [71; 16],
        })
    }
}

fn plan() -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([1; 16]),
        question_digest: [19; 32],
        method: MethodRefV1 {
            machine_digest: [2; 32],
            state_digest: [3; 32],
        },
        claims: vec![ClaimKindV1::Empirical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([4; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([5; 16]),
            scores: vec![1],
        }],
        admission: moira_core::deliberation::EnsembleAdmissionV1::affirmed(
            [16; 32], [17; 32], [18; 32], [19; 32], [20; 16],
        ),
        members: vec![member(6), member(10)],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [14; 32],
            result_digest: [15; 32],
            all_orthogonal: true,
        },
        fuel: 3,
    }
}

fn member(seed: u8) -> EnsembleMemberV1 {
    EnsembleMemberV1 {
        id: EnsembleMemberIdV1([seed; 16]),
        stakeholder: moira_core::deliberation::StakeholderRefV1::technical_sme(
            [seed; 16],
            [seed + 1; 32],
        ),
        lens: PersonaLensV1::RigorReviewer,
        concern_root_digest: [seed + 1; 32],
        grounding: GroundingRefV1 {
            authority_digest: [seed + 2; 32],
            surface_digest: [seed + 3; 32],
            depth: SurfaceDepthV1::Ambient,
        },
    }
}

fn runs(plan: &DeliberationPlanV1) -> Vec<MemberRunV1> {
    plan.members
        .iter()
        .enumerate()
        .map(|(index, member)| {
            let seed = u8::try_from(index).unwrap() * 10 + 30;
            MemberRunV1 {
                member: member.id,
                report: ReportIdV1([seed; 16]),
                execution: ExecutionIntentV1 {
                    effect: EffectIdV1([seed + 1; 16]),
                    execution_id: [seed + 2; 16],
                    task: TaskRefV1 {
                        external_task_key: [seed + 3; 16],
                        incarnation: 1,
                    },
                    attempt_number: 1,
                    principal: [20; 16],
                    profile: [21; 16],
                    idempotency_key: [seed + 4; 16],
                    context: vec![[seed + 5; 16]],
                    tools: Vec::new(),
                    budget: daimon::ExecutionBudgetV1 {
                        max_elapsed_millis: 1_000,
                        max_input_tokens: 100,
                        max_output_tokens: 100,
                        max_tool_calls: 1,
                        max_context_bytes: 1_024,
                        max_output_bytes: 1_024,
                    },
                },
            }
        })
        .collect()
}

#[test]
fn resumed_member_remains_incomplete_until_terminal_report_is_durably_retained() {
    let mut store = Store::new();
    let plan = plan();
    store
        .apply(
            store.revision(),
            RegistryCommandV1::Admit(Box::new(plan.clone())),
        )
        .unwrap();
    let run = runs(&plan).remove(0);
    let result = CompletedToolResultV1 {
        request: [70; 16],
        evidence_digest: [71; 32],
        output_digest: [72; 32],
        output: b"tool output".to_vec(),
    };
    let mut continuation =
        ModelExecutionAdapter::new(daimon::SessionHandleV1(1), ResumePort { tool_call: true });
    let incomplete = resume_member_and_retain(
        &mut store,
        &mut continuation,
        &plan,
        run.clone(),
        MemberContinuationV1 {
            member: run.member,
            report: run.report,
            source_execution: [69; 16],
        },
        vec![result.clone()],
        10,
    )
    .unwrap();
    let MemberOutcomeV1::ToolContinuation { continuation, .. } = incomplete.outcome else {
        panic!("continuation expected")
    };
    assert!(incomplete.retained.reports.is_empty());

    let mut terminal =
        ModelExecutionAdapter::new(daimon::SessionHandleV1(1), ResumePort { tool_call: false });
    let completed = resume_member_and_retain(
        &mut store,
        &mut terminal,
        &plan,
        run,
        continuation,
        vec![result],
        11,
    )
    .unwrap();
    assert!(matches!(completed.outcome, MemberOutcomeV1::Report { .. }));
    assert_eq!(completed.retained.reports.len(), 1);
}

#[test]
fn exact_cohort_is_admitted_executed_retained_then_explicitly_resolved() {
    let calls = Rc::new(Cell::new(0));
    let mut store = Store::new();
    let mut adapter = ModelExecutionAdapter::new(
        daimon::SessionHandleV1(1),
        Port {
            calls: Rc::clone(&calls),
            output: RESPONSE.to_vec(),
        },
    );
    let plan = plan();
    let completed = run(&mut store, &mut adapter, plan.clone(), runs(&plan), 10).unwrap();
    assert_eq!(calls.get(), 2);
    assert_eq!(completed.retained.reports.len(), 2);
    assert!(completed.retained.resolution.is_none());
    assert_eq!(store.revision(), RegistryRevisionV1(3));

    let resolved =
        resolve_explicit(&mut store, plan.id, ResolutionV1::Maybe(MaybeValueV1::Both)).unwrap();
    assert_eq!(resolved.revision, RegistryRevisionV1(4));
    assert_eq!(
        resolved.deliberation.resolution,
        Some(ResolutionV1::Maybe(MaybeValueV1::Both))
    );
}

#[test]
fn completed_prose_is_retained_as_outcome_but_cannot_make_reports_or_resolution() {
    let calls = Rc::new(Cell::new(0));
    let mut store = Store::new();
    let mut adapter = ModelExecutionAdapter::new(
        daimon::SessionHandleV1(1),
        Port {
            calls: Rc::clone(&calls),
            output: b"confident prose".to_vec(),
        },
    );
    let plan = plan();
    let completed = run(&mut store, &mut adapter, plan.clone(), runs(&plan), 10).unwrap();
    assert_eq!(calls.get(), 2);
    assert!(completed.outcomes.iter().all(|outcome| matches!(
        outcome,
        MemberOutcomeV1::InvalidReport {
            refusal: MemberResponseRefusalV1::InvalidJson,
            ..
        }
    )));
    assert!(completed.retained.reports.is_empty());
    assert!(
        resolve_explicit(
            &mut store,
            plan.id,
            ResolutionV1::Maybe(MaybeValueV1::Neither)
        )
        .is_err()
    );
}

#[test]
fn duplicate_execution_identity_refuses_before_plan_admission_or_dispatch() {
    let calls = Rc::new(Cell::new(0));
    let mut store = Store::new();
    let mut adapter = ModelExecutionAdapter::new(
        daimon::SessionHandleV1(1),
        Port {
            calls: Rc::clone(&calls),
            output: RESPONSE.to_vec(),
        },
    );
    let plan = plan();
    let mut runs = runs(&plan);
    runs[1].execution.execution_id = runs[0].execution.execution_id;
    assert!(matches!(
        run(&mut store, &mut adapter, plan, runs, 10),
        Err(GovernedRunErrorV1::InvalidCohort(_))
    ));
    assert_eq!(calls.get(), 0);
    assert_eq!(store.revision(), RegistryRevisionV1(0));
}

#[test]
fn governed_reports_survive_sqlite_close_and_reopen() {
    let fixture = Fixture::new();
    let calls = Rc::new(Cell::new(0));
    let plan = plan();
    {
        let mut store = fixture.open();
        let mut adapter = ModelExecutionAdapter::new(
            daimon::SessionHandleV1(1),
            Port {
                calls: Rc::clone(&calls),
                output: RESPONSE.to_vec(),
            },
        );
        let completed = run(&mut store, &mut adapter, plan.clone(), runs(&plan), 10).unwrap();
        assert_eq!(completed.retained.reports.len(), 2);
    }
    let reopened = fixture.open();
    assert_eq!(reopened.revision(), RegistryRevisionV1(3));
    let retained = reopened.deliberation(plan.id).unwrap();
    assert_eq!(retained.reports.len(), 2);
    assert_eq!(retained.reports[0].dissent_digests, vec![[0xbb; 32]]);
}

#[test]
fn governed_run_uses_daimons_authenticated_private_model_socket() {
    if std::env::var_os("MOIRA_LOCAL_SOCKET_TEST").is_none() {
        return;
    }
    let fixture = Fixture::new();
    let socket = fixture.0.join("daimon.sock");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut daemon = ModelDaemon::bind_new(
        &socket,
        SocketService {
            calls: Arc::clone(&calls),
        },
        Authenticate,
        daimon_wire::LimitsV1::default(),
        daimon_wire::ModelLimitsV1::default(),
        Duration::from_secs(2),
        2,
    )
    .unwrap();
    let server = thread::spawn(move || daemon.serve_next().unwrap());
    let mut adapter = connect_daimon(
        &socket,
        daimon_wire::LimitsV1::default(),
        daimon_wire::ModelLimitsV1::default(),
        Duration::from_secs(2),
    )
    .unwrap();
    let plan = plan();
    let mut store = Store::new();
    let completed = run(&mut store, &mut adapter, plan.clone(), runs(&plan), 10).unwrap();
    assert_eq!(completed.retained.reports.len(), 2);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    drop(adapter);
    server.join().unwrap();
}
