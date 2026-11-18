#![cfg(unix)]

use aegis_authority::{EngineId, Limits, PrincipalId, SessionId};
use aegis_client::{AuthorityClient, ServicePort};
use aegis_host::{DurableAuthority, HostError};
use aegis_protocol::{
    ActionV1, CommandV1, EngineNamespaceV1, OutcomeV1, ReplyV1, ScopeV1, SessionHandleV1,
};
use aegis_service::{AuthenticatedSessionV1, Service, ServiceLimitsV1};
use keystone_storage::{
    AcknowledgementStrength, Checkpoint, CheckpointStore, CommitOutcome, CommitReceipt, Revision,
    StoreRefusal,
};
use moira_aegis::{
    AegisActionSelectorV1, AegisChoiceAuthorityV1, AegisOperatorAuthorizerV1,
    AegisPermissionAuthorizerV1, AegisToolAuthorityV1, OperatorAdapter, ReconcileError,
    TimeSourceV1, ToolAuthorizationError, choice_resource, permission_resource, saga_resource,
};
use moira_core::EffectId;
use moira_core::deliberation::{
    AuthoritySubjectIdV1, CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1,
    CriterionIdV1, CriterionV1, DeliberationIdV1, DeliberationLimitsV1, DeliberationPlanV1,
    EnsembleAdmissionV1, EnsembleMemberIdV1, EnsembleMemberV1, GroundingRefV1, MemberReportV1,
    MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1, ReportIdV1, ResolutionV1,
    StakeholderRefV1, SurfaceDepthV1,
    registry::{RegistryCommandV1, RegistryLimitsV1, RegistryRevisionV1},
};
use moira_core::saga::{SagaLimitsV1, persistence::SagaCodecLimitsV1};
use moira_host::{
    ActionSelectionSubjectV1, ActionSelectorV1, ChoiceAuthorityV1, ChoiceRequestV1,
    ResolutionSubjectV1,
};
use moira_local_harness::resolution::resolve_authorized_choice;
use moira_protocol::tool_continuation::ToolCapabilityIdV1;
use moira_protocol::{
    OperatorActionIdV1, OperatorDispositionV1, OperatorOperationV1, SagaCommandV1,
    SagaEffectOutcomeV1, SagaMutationV1, SagaOutcomeV1, SagaReplyV1, SagaSourceV1,
    SagaTransitionV1, agent_participation as participation,
};
use moira_service::{
    agent_participation::PermissionAuthorizer,
    saga::{OperatorAuthorizer, SagaService},
};
use moira_sqlite::deliberation::{DeliberationSqliteStore, DeliberationStoreLimitsV1};
use moira_sqlite::saga::{SagaSqliteStore, SagaStoreLimitsV1};
use std::{
    convert::Infallible,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
struct AuthorityStore(Option<Checkpoint>);

impl CheckpointStore for AuthorityStore {
    type Error = Infallible;

    fn load(&self, _: &str) -> Result<Option<Checkpoint>, Self::Error> {
        Ok(self.0.clone())
    }

    fn compare_exchange(
        &mut self,
        _: &str,
        expected: Option<Revision>,
        bytes: &[u8],
    ) -> Result<CommitOutcome, Self::Error> {
        let actual = self.0.as_ref().map(|checkpoint| checkpoint.revision);
        if actual != expected {
            return Ok(CommitOutcome::Refused(StoreRefusal::RevisionConflict {
                expected,
                actual,
            }));
        }
        let revision = Revision(actual.map_or(0, |revision| revision.0 + 1));
        self.0 = Some(Checkpoint {
            revision,
            bytes: bytes.to_vec(),
            strength: AcknowledgementStrength::Durable,
        });
        Ok(CommitOutcome::Committed(CommitReceipt {
            revision,
            mechanism: "moira-aegis-test-store",
            strength: AcknowledgementStrength::Durable,
        }))
    }
}

struct AegisPort {
    service: Service<AuthorityStore>,
}

impl ServicePort for AegisPort {
    type Error = HostError<Infallible>;

    fn execute(
        &mut self,
        session: SessionHandleV1,
        command: CommandV1,
    ) -> Result<ReplyV1, Self::Error> {
        self.service.execute(session, command)
    }
}

fn path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "moira-aegis-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn saga_limits() -> SagaStoreLimitsV1 {
    SagaStoreLimitsV1 {
        codec: SagaCodecLimitsV1 {
            max_bytes: 16_384,
            max_sagas: 2,
            max_operator_actions: 4,
        },
        saga: SagaLimitsV1 {
            max_sagas: 2,
            max_retries_per_saga: 1,
            max_inspection_page: 2,
            max_operator_actions: 4,
        },
        max_database_pages: 64,
    }
}

fn deliberation_limits() -> DeliberationStoreLimitsV1 {
    let domain = DeliberationLimitsV1 {
        max_claims: 4,
        max_criteria: 4,
        max_candidates: 4,
        max_members: 4,
        max_evidence_per_report: 8,
        max_dissent_per_report: 8,
        max_fuel: 8,
    };
    DeliberationStoreLimitsV1 {
        codec: moira_core::deliberation::registry::persistence::CodecLimitsV1 {
            max_bytes: 65_536,
            max_deliberation_bytes: 32_768,
            registry: RegistryLimitsV1 {
                max_deliberations: 2,
                deliberation: domain,
            },
        },
        max_database_pages: 64,
    }
}

fn choice_plan(deliberation: DeliberationIdV1, selected: CandidateIdV1) -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: deliberation,
        question_digest: [61; 32],
        method: MethodRefV1 {
            machine_digest: [62; 32],
            state_digest: [63; 32],
        },
        claims: vec![ClaimKindV1::Normative],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([64; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: selected,
            scores: vec![1],
        }],
        admission: EnsembleAdmissionV1::affirmed([71; 32], [72; 32], [73; 32], [74; 32], [75; 16]),
        members: vec![EnsembleMemberV1 {
            id: EnsembleMemberIdV1([65; 16]),
            stakeholder: StakeholderRefV1::technical_sme([65; 16], [66; 32]),
            lens: PersonaLensV1::Historian,
            concern_root_digest: [66; 32],
            grounding: GroundingRefV1 {
                authority_digest: [67; 32],
                surface_digest: [68; 32],
                depth: SurfaceDepthV1::Reference,
            },
        }],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [69; 32],
            result_digest: [70; 32],
            all_orthogonal: true,
        },
        fuel: 2,
    }
}

fn choice_report() -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([71; 16]),
        member: EnsembleMemberIdV1([65; 16]),
        output_digest: [72; 32],
        evidence_digests: vec![[73; 32]],
        dissent_digests: vec![[74; 32]],
    }
}

struct FixedTime(u64);

impl TimeSourceV1 for FixedTime {
    fn now(&mut self) -> u64 {
        self.0
    }
}

#[test]
fn exact_daimon_capability_produces_attributed_tool_authority_evidence() {
    let authority_id = EngineId::new([30; 16]);
    let authority =
        DurableAuthority::create_new(AuthorityStore::default(), authority_id, Limits::default())
            .unwrap();
    let mut aegis = Service::new(authority, authority_id, ServiceLimitsV1::default()).unwrap();
    let subject = [31; 16];
    let session = aegis
        .open_session(AuthenticatedSessionV1 {
            principal: PrincipalId::new(subject),
            session: SessionId::new([32; 16]),
            administrator: true,
        })
        .unwrap();
    let capability = ToolCapabilityIdV1([33; 16]);
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(grant)) = aegis
        .execute(
            session,
            CommandV1::Issue {
                expected_epoch: 0,
                now: 1,
                expires_at: 10,
                scopes: vec![ScopeV1 {
                    engine: EngineNamespaceV1::Daimon,
                    resource: capability.0,
                    actions: 1 << ActionV1::Execute as u8,
                }],
            },
        )
        .unwrap()
    else {
        panic!("tool grant expected")
    };
    let mut tools = AegisToolAuthorityV1::new(
        AuthorityClient::new(session, AegisPort { service: aegis }),
        grant.handle,
        subject,
        FixedTime(2),
    );
    let evidence = tools.authorize(capability).unwrap();
    assert_eq!(evidence.subject.0, subject);
    assert_eq!(evidence.capability, capability);
    assert_eq!(evidence.epoch, 0);
    assert_ne!(evidence.decision.0, [0; 16]);
    assert!(matches!(
        tools.authorize(ToolCapabilityIdV1([34; 16])),
        Err(ToolAuthorizationError::AuthorityRefused(_))
    ));
}

#[test]
fn exact_deliberation_scope_authorizes_one_attributed_choice() {
    let authority_id = EngineId::new([40; 16]);
    let authority =
        DurableAuthority::create_new(AuthorityStore::default(), authority_id, Limits::default())
            .unwrap();
    let mut aegis = Service::new(authority, authority_id, ServiceLimitsV1::default()).unwrap();
    let subject = [41; 16];
    let session = aegis
        .open_session(AuthenticatedSessionV1 {
            principal: PrincipalId::new(subject),
            session: SessionId::new([42; 16]),
            administrator: true,
        })
        .unwrap();
    let deliberation = DeliberationIdV1([43; 16]);
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(grant)) = aegis
        .execute(
            session,
            CommandV1::Issue {
                expected_epoch: 0,
                now: 1,
                expires_at: 10,
                scopes: vec![ScopeV1 {
                    engine: EngineNamespaceV1::Moira,
                    resource: choice_resource(deliberation),
                    actions: 1 << ActionV1::Administer as u8,
                }],
            },
        )
        .unwrap()
    else {
        panic!("choice grant expected")
    };
    let request = ChoiceRequestV1 {
        subject: ResolutionSubjectV1 {
            deliberation,
            question_digest: [44; 32],
            checkpoint_digest: [45; 32],
        },
        frontier: vec![CandidateIdV1([46; 16])],
    };
    let mut chooser = AegisChoiceAuthorityV1::new(
        AuthorityClient::new(session, AegisPort { service: aegis }),
        grant.handle,
        subject,
        CandidateIdV1([46; 16]),
        FixedTime(2),
    );
    let decision = chooser.choose(request.clone()).unwrap();
    assert_eq!(decision.subject, request.subject);
    assert_eq!(decision.selected, request.frontier[0]);
    assert_eq!(decision.authority.subject.0, subject);
    assert_eq!(decision.authority.epoch, 0);

    let (port, _) = chooser.into_parts();
    let action_subject = ActionSelectionSubjectV1 {
        deliberation,
        checkpoint_digest: [47; 32],
        member: EnsembleMemberIdV1([48; 16]),
        report: ReportIdV1([49; 16]),
        output_digest: [50; 32],
        daimon_effect: EffectId::new([51; 16]),
    };
    let mut selector = AegisActionSelectorV1::new(
        AuthorityClient::new(session, port),
        grant.handle,
        subject,
        FixedTime(3),
    );
    let selected_action = selector.select(action_subject).unwrap();
    assert_eq!(selected_action.subject, action_subject);
    assert_eq!(selected_action.authority.subject.0, subject);
    assert_eq!(selected_action.authority.epoch, 0);
}

#[test]
fn aegis_authorized_choice_is_resolved_and_reopened_with_attribution() {
    let path = path();
    let owner = [50; 16];
    let deliberation = DeliberationIdV1([51; 16]);
    let selected = CandidateIdV1([52; 16]);
    let mut store = DeliberationSqliteStore::open(&path, owner, deliberation_limits()).unwrap();
    store
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(choice_plan(deliberation, selected)),
        )
        .unwrap();
    store
        .apply(
            RegistryRevisionV1(1),
            RegistryCommandV1::SubmitReport {
                deliberation,
                report: choice_report(),
            },
        )
        .unwrap();

    let authority_id = EngineId::new([53; 16]);
    let authority =
        DurableAuthority::create_new(AuthorityStore::default(), authority_id, Limits::default())
            .unwrap();
    let mut aegis = Service::new(authority, authority_id, ServiceLimitsV1::default()).unwrap();
    let subject = [54; 16];
    let session = aegis
        .open_session(AuthenticatedSessionV1 {
            principal: PrincipalId::new(subject),
            session: SessionId::new([55; 16]),
            administrator: true,
        })
        .unwrap();
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(grant)) = aegis
        .execute(
            session,
            CommandV1::Issue {
                expected_epoch: 0,
                now: 1,
                expires_at: 10,
                scopes: vec![ScopeV1 {
                    engine: EngineNamespaceV1::Moira,
                    resource: choice_resource(deliberation),
                    actions: 1 << ActionV1::Administer as u8,
                }],
            },
        )
        .unwrap()
    else {
        panic!("choice grant expected")
    };
    let mut chooser = AegisChoiceAuthorityV1::new(
        AuthorityClient::new(session, AegisPort { service: aegis }),
        grant.handle,
        subject,
        selected,
        FixedTime(2),
    );
    let receipt = resolve_authorized_choice(&mut store, deliberation, &mut chooser).unwrap();
    let retained = receipt.deliberation.resolution.unwrap();
    let ResolutionV1::AuthorizedChoice { authority, .. } = &retained else {
        panic!("authorized choice expected")
    };
    assert_eq!(authority.subject, AuthoritySubjectIdV1(subject));
    assert_eq!(authority.epoch, 0);
    assert_eq!(&authority.decision.0[..8], b"AEGISD01");
    drop(store);

    let reopened = DeliberationSqliteStore::open(&path, owner, deliberation_limits()).unwrap();
    assert_eq!(
        reopened.deliberation(deliberation).unwrap().resolution,
        Some(retained)
    );
    drop(reopened);
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

#[test]
fn daemon_authorizer_returns_only_current_exact_saga_aegis_facts() {
    let authority_id = EngineId::new([8; 16]);
    let authority =
        DurableAuthority::create_new(AuthorityStore::default(), authority_id, Limits::default())
            .unwrap();
    let mut aegis = Service::new(authority, authority_id, ServiceLimitsV1::default()).unwrap();
    let subject = [9; 16];
    let session = aegis
        .open_session(AuthenticatedSessionV1 {
            principal: PrincipalId::new(subject),
            session: SessionId::new([10; 16]),
            administrator: true,
        })
        .unwrap();
    let saga = moira_protocol::SagaIdV1(11);
    let issue = |action| CommandV1::Issue {
        expected_epoch: 0,
        now: 1,
        expires_at: 10,
        scopes: vec![ScopeV1 {
            engine: EngineNamespaceV1::Moira,
            resource: saga_resource(saga.0),
            actions: 1 << action as u8,
        }],
    };
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(allowed)) =
        aegis.execute(session, issue(ActionV1::Administer)).unwrap()
    else {
        panic!("administer grant expected")
    };
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(denied)) =
        aegis.execute(session, issue(ActionV1::Observe)).unwrap()
    else {
        panic!("observe grant expected")
    };
    let operation = OperatorOperationV1 {
        action: OperatorActionIdV1([12; 16]),
        saga,
        disposition: OperatorDispositionV1::KeepUncertain,
    };
    let mut authorizer = AegisOperatorAuthorizerV1::new(
        AuthorityClient::new(session, AegisPort { service: aegis }),
        allowed.handle,
        subject,
        FixedTime(2),
    );
    let access = authorizer.authorize_operator(operation).unwrap().unwrap();
    assert_eq!(access.subject, subject);
    assert_eq!(access.authority_epoch, 0);

    let (port, time) = authorizer.into_parts();
    let mut denied_authorizer = AegisOperatorAuthorizerV1::new(
        AuthorityClient::new(session, port),
        denied.handle,
        subject,
        time,
    );
    assert_eq!(
        denied_authorizer.authorize_operator(operation).unwrap(),
        None
    );
}

#[test]
fn permission_authorizer_scopes_aegis_to_exact_request_and_capability() {
    let authority_id = EngineId::new([31; 16]);
    let authority =
        DurableAuthority::create_new(AuthorityStore::default(), authority_id, Limits::default())
            .unwrap();
    let mut aegis = Service::new(authority, authority_id, ServiceLimitsV1::default()).unwrap();
    let subject = [32; 16];
    let session = aegis
        .open_session(AuthenticatedSessionV1 {
            principal: PrincipalId::new(subject),
            session: SessionId::new([33; 16]),
            administrator: true,
        })
        .unwrap();
    let selection = participation::PermissionSelectionV1 {
        expected_storage_revision: Some(4),
        participation: participation::ParticipationIdV1([34; 16]),
        expected_revision: 5,
        key: participation::IdempotencyKeyV1([35; 16]),
        execution: participation::ExecutionIdV1([41; 16]),
        request: participation::PermissionRequestIdV1([36; 16]),
        option: participation::PermissionOptionIdV1([37; 16]),
        capability_digest: [38; 32],
        attempt: participation::AttemptV1 {
            task_key: [39; 16],
            incarnation: 1,
            attempt: 2,
        },
    };
    let resource = permission_resource(
        selection.participation,
        selection.execution,
        selection.request,
        selection.option,
        selection.capability_digest,
    );
    assert_ne!(
        resource,
        permission_resource(
            selection.participation,
            selection.execution,
            selection.request,
            selection.option,
            [40; 32],
        )
    );
    assert_ne!(
        resource,
        permission_resource(
            selection.participation,
            selection.execution,
            selection.request,
            participation::PermissionOptionIdV1([42; 16]),
            selection.capability_digest,
        )
    );
    let issue = |action| CommandV1::Issue {
        expected_epoch: 0,
        now: 1,
        expires_at: 10,
        scopes: vec![ScopeV1 {
            engine: EngineNamespaceV1::Moira,
            resource,
            actions: 1 << action as u8,
        }],
    };
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(allowed)) =
        aegis.execute(session, issue(ActionV1::Execute)).unwrap()
    else {
        panic!("execute grant expected")
    };
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(denied)) =
        aegis.execute(session, issue(ActionV1::Observe)).unwrap()
    else {
        panic!("observe grant expected")
    };
    let mut authorizer = AegisPermissionAuthorizerV1::new(
        AuthorityClient::new(session, AegisPort { service: aegis }),
        allowed.handle,
        subject,
        FixedTime(2),
    );
    let access = authorizer.authorize_permission(selection).unwrap().unwrap();
    assert_eq!(access.subject, subject);
    assert_eq!(access.capability_digest, selection.capability_digest);
    assert_eq!(&access.decision[..8], b"AEGISD01");

    let (port, time) = authorizer.into_parts();
    let mut denied_authorizer = AegisPermissionAuthorizerV1::new(
        AuthorityClient::new(session, port),
        denied.handle,
        subject,
        time,
    );
    assert_eq!(
        denied_authorizer.authorize_permission(selection).unwrap(),
        None
    );
}

#[test]
fn public_aegis_client_authorizes_durable_reconciliation_and_denies_wrong_scope() {
    let path = path();
    let owner = [1; 16];
    let mut moira = SagaService::new(SagaSqliteStore::open(&path, owner, saga_limits()).unwrap());
    let SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
        transition: SagaTransitionV1::Admitted { saga, effect, .. },
        ..
    }) = moira
        .execute(SagaCommandV1::Apply {
            expected_revision: 0,
            mutation: SagaMutationV1::Admit {
                source: SagaSourceV1([2; 16]),
                now: 1,
                deadline: 20,
                retry_budget: 1,
            },
        })
        .unwrap()
    else {
        panic!("admission expected")
    };
    moira
        .execute(SagaCommandV1::Apply {
            expected_revision: 1,
            mutation: SagaMutationV1::AcceptEffect {
                saga,
                effect,
                outcome: SagaEffectOutcomeV1::Uncertain,
                observed_at: 2,
            },
        })
        .unwrap();

    let authority_id = EngineId::new([3; 16]);
    let authority =
        DurableAuthority::create_new(AuthorityStore::default(), authority_id, Limits::default())
            .unwrap();
    let mut aegis = Service::new(authority, authority_id, ServiceLimitsV1::default()).unwrap();
    let subject = [4; 16];
    let session = aegis
        .open_session(AuthenticatedSessionV1 {
            principal: PrincipalId::new(subject),
            session: SessionId::new([5; 16]),
            administrator: true,
        })
        .unwrap();
    let issue = |resource, action| CommandV1::Issue {
        expected_epoch: 0,
        now: 1,
        expires_at: 10,
        scopes: vec![ScopeV1 {
            engine: EngineNamespaceV1::Moira,
            resource,
            actions: 1 << action as u8,
        }],
    };
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(allowed)) = aegis
        .execute(session, issue(saga_resource(saga.0), ActionV1::Administer))
        .unwrap()
    else {
        panic!("administer grant expected")
    };
    let ReplyV1::Accepted(OutcomeV1::GrantIssued(denied)) = aegis
        .execute(session, issue(saga_resource(saga.0), ActionV1::Observe))
        .unwrap()
    else {
        panic!("observe grant expected")
    };
    let command = SagaCommandV1::Reconcile {
        expected_revision: 2,
        operation: OperatorOperationV1 {
            action: OperatorActionIdV1([6; 16]),
            saga,
            disposition: OperatorDispositionV1::KeepUncertain,
        },
    };
    let mut adapter = OperatorAdapter::new(
        AuthorityClient::new(session, AegisPort { service: aegis }),
        allowed.handle,
        subject,
    );
    assert!(matches!(
        adapter.reconcile(&mut moira, 2, command).unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            transition: SagaTransitionV1::Reconciled { .. },
            replayed: false,
            ..
        })
    ));
    drop(moira);

    let mut reopened =
        SagaService::new(SagaSqliteStore::open(&path, owner, saga_limits()).unwrap());
    assert!(matches!(
        adapter.reconcile(&mut reopened, 3, command).unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Applied { replayed: true, .. })
    ));
    let port = adapter.into_port();
    let mut denied_adapter =
        OperatorAdapter::new(AuthorityClient::new(session, port), denied.handle, subject);
    let denied_command = SagaCommandV1::Reconcile {
        expected_revision: 3,
        operation: OperatorOperationV1 {
            action: OperatorActionIdV1([7; 16]),
            saga,
            disposition: OperatorDispositionV1::FailTerminal,
        },
    };
    assert!(matches!(
        denied_adapter.reconcile(&mut reopened, 3, denied_command),
        Err(ReconcileError::AuthorityRefused(_))
    ));
    assert!(matches!(
        reopened.execute(SagaCommandV1::Status).unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Status(status)) if status.revision == 3
    ));
    drop(reopened);
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}
