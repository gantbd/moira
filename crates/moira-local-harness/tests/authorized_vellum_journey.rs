#![cfg(unix)]

use aegis_authority::{EngineId, Limits as AegisLimits, PrincipalId, SessionId};
use aegis_client::{AuthorityClient, ServicePort as AegisServicePort};
use aegis_host::{DurableAuthority, HostError};
use aegis_protocol::{
    ActionV1, CommandV1 as AegisCommandV1, EngineNamespaceV1, OutcomeV1 as AegisOutcomeV1,
    ReplyV1 as AegisReplyV1, ScopeV1, SessionHandleV1,
};
use aegis_service::{AuthenticatedSessionV1, Service as AegisService, ServiceLimitsV1};
use agora_client::v2::{PortFailureV2, ServicePortV2};
use agora_protocol::v2::{ClientIdV2, ReplyV2, RequestV2};
use keystone_storage::{
    AcknowledgementStrength, Checkpoint, CheckpointStore, CommitOutcome, CommitReceipt, Revision,
    StoreRefusal,
};
use moira_aegis::{AegisActionSelectorV1, AegisChoiceAuthorityV1, TimeSourceV1, choice_resource};
use moira_agora::{
    PublicationLimitsV1,
    durable::{
        DurableResponsePublisher, PublisherCheckpointV1, PublisherRecordV1, PublisherStateStoreV1,
    },
};
use moira_client::{ModelSagaMutationClient, ModelSagaServicePort};
use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, DeliberationPlanV1, EnsembleMemberIdV1, EnsembleMemberV1, GroundingRefV1,
    MemberReportV1, MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1, ReportIdV1, ResolutionV1,
    SurfaceDepthV1,
    registry::{DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryRevisionV1},
};
use moira_core::model_saga::LimitsV1;
use moira_daimon::{
    ModelProposalV1, ModelResultV1,
    ensemble::{MemberClaimV1, MemberOutcomeV1, MemberResponseV1},
};
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};
use moira_local_harness::{
    action_selection::select_authorized,
    peer_router::FederatedPeerRouterV1,
    peer_turn::{PeerTurnOutcomeV1, execute_selected_peer_turn},
    resolution::resolve_authorized_choice,
};
use moira_protocol::{
    DispositionV1, EffectIdV1, ResultReceiptV1, TaskRefV1, WorkflowIdV1, model_saga as saga,
};
use moira_service::model_saga::ModelSagaService;
use moira_sqlite::model_saga::{
    ModelSagaSqliteStore, ModelSagaStoreErrorV1, ModelSagaStoreLimitsV1,
};
use moira_vellum::{DocumentRouteV1, EditAdapter, ExternalDocumentRefV1};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    convert::Infallible,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};
use vellum_protocol::{
    ClientId, DurableEffectIntentV1, DurableEffectReceiptV1, DurableExecuteOutcomeV1,
    DurableExecuteRequestV1, DurableReceiptLookupOutcomeV1, DurableReceiptLookupV1,
    DurableReplaceAndSaveReceiptV1, EditPersistenceV1, ReplyV1 as VellumReplyV1, RequestV1,
    SaveApplicationV1, SaveReceiptV1, SaveSynchronizationV1, ServiceAccessErrorV1,
    ServiceControlErrorV1, ServicePort, WorkspaceDocumentIdV1, WorkspaceIdV1,
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
            mechanism: "authorized-vellum-test",
            strength: AcknowledgementStrength::Durable,
        }))
    }
}

struct AegisPort(AegisService<AuthorityStore>);

impl AegisServicePort for AegisPort {
    type Error = HostError<Infallible>;

    fn execute(
        &mut self,
        session: SessionHandleV1,
        command: AegisCommandV1,
    ) -> Result<AegisReplyV1, Self::Error> {
        self.0.execute(session, command)
    }
}

struct FixedTime(u64);

impl TimeSourceV1 for FixedTime {
    fn now(&mut self) -> u64 {
        self.0
    }
}

struct Deliberations(DeliberationRegistryV1);

impl DeliberationStore for Deliberations {
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

#[derive(Clone)]
struct PublisherStore(PublisherRecordV1);

impl PublisherStateStoreV1 for PublisherStore {
    type Error = Infallible;

    fn load(&mut self) -> Result<PublisherRecordV1, Self::Error> {
        Ok(self.0.clone())
    }

    fn compare_exchange(
        &mut self,
        expected_revision: u64,
        checkpoint: &PublisherCheckpointV1,
    ) -> Result<PublisherRecordV1, Self::Error> {
        assert_eq!(self.0.revision, expected_revision);
        self.0 = PublisherRecordV1 {
            revision: expected_revision + 1,
            checkpoint: checkpoint.clone(),
        };
        Ok(self.0.clone())
    }
}

struct UnusedAgoraPort;

impl ServicePortV2 for UnusedAgoraPort {
    type Error = Infallible;

    fn execute(
        &mut self,
        _: ClientIdV2,
        _: RequestV2,
    ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
        unreachable!("authorized Vellum journey has no Agora effect")
    }
}

#[derive(Default)]
struct VellumCalls {
    executions: Vec<[u8; 16]>,
    lookups: Vec<[u8; 16]>,
}

struct VellumPort {
    calls: Rc<RefCell<VellumCalls>>,
    outcome: DurableExecuteOutcomeV1,
    lose_next: bool,
}

impl ServicePort for VellumPort {
    fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
        Ok(ClientId::new(1).unwrap())
    }

    fn execute(&mut self, _: RequestV1) -> Result<VellumReplyV1, ServiceAccessErrorV1> {
        unreachable!("durable path only")
    }

    fn disconnect(&mut self, _: ClientId) -> Result<(), ServiceControlErrorV1> {
        Ok(())
    }

    fn execute_durable(
        &mut self,
        request: DurableExecuteRequestV1,
    ) -> Result<DurableExecuteOutcomeV1, ServiceAccessErrorV1> {
        self.calls
            .borrow_mut()
            .executions
            .push(request.operation.to_bytes());
        assert!(matches!(
            request.intent,
            DurableEffectIntentV1::ReplaceDocumentAndSave {
                expected_revision: 7,
                start: 0,
                end: 12,
                ref inserted,
                ..
            } if inserted == "replacement"
        ));
        if self.lose_next {
            self.lose_next = false;
            Err(ServiceAccessErrorV1::OutcomeUnknown)
        } else {
            Ok(self.outcome.clone())
        }
    }

    fn lookup_durable_receipt(
        &mut self,
        lookup: DurableReceiptLookupV1,
    ) -> Result<DurableReceiptLookupOutcomeV1, ServiceAccessErrorV1> {
        self.calls
            .borrow_mut()
            .lookups
            .push(lookup.operation.to_bytes());
        Ok(DurableReceiptLookupOutcomeV1::Complete(
            self.outcome.clone(),
        ))
    }
}

struct SagaPort(ModelSagaService<ModelSagaSqliteStore>);

impl ModelSagaServicePort for SagaPort {
    type Error = ModelSagaStoreErrorV1;

    fn execute(&mut self, command: saga::CommandV1) -> Result<saga::ReplyV1, Self::Error> {
        self.0.execute(command)
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moira-authorized-vellum-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn report(id: u8, member: u8, output: &[u8]) -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([id; 16]),
        member: EnsembleMemberIdV1([member; 16]),
        output_digest: Sha256::digest(output).into(),
        evidence_digests: Vec::new(),
        dissent_digests: Vec::new(),
    }
}

fn unresolved_outcome() -> (DeliberationPlanV1, Vec<MemberReportV1>, MemberOutcomeV1<()>) {
    let selected_output = br#"{"schema":"moira.member-response.v1","claims":[{"kind":"normative","statement":"edit","evidence_digests":[],"dissent_digests":[]}]}"#.to_vec();
    let other_output = b"other".to_vec();
    let selected_report = report(21, 11, &selected_output);
    let other_report = report(22, 12, &other_output);
    let deliberation_id = DeliberationIdV1([1; 16]);
    let candidate = CandidateIdV1([2; 16]);
    let member = |id, lens| EnsembleMemberV1 {
        id: EnsembleMemberIdV1([id; 16]),
        stakeholder: moira_core::deliberation::StakeholderRefV1::technical_sme(
            [id; 16],
            [id + 20; 32],
        ),
        lens,
        concern_root_digest: [id + 20; 32],
        grounding: GroundingRefV1 {
            authority_digest: [id + 30; 32],
            surface_digest: [id + 40; 32],
            depth: SurfaceDepthV1::Ambient,
        },
    };
    let plan = DeliberationPlanV1 {
        id: deliberation_id,
        question_digest: [3; 32],
        method: MethodRefV1 {
            machine_digest: [4; 32],
            state_digest: [5; 32],
        },
        claims: vec![ClaimKindV1::Normative],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([6; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: candidate,
            scores: vec![1],
        }],
        admission: moira_core::deliberation::EnsembleAdmissionV1::affirmed(
            [7; 32], [8; 32], [9; 32], [10; 32], [13; 16],
        ),
        members: vec![
            member(11, PersonaLensV1::Theorist),
            member(12, PersonaLensV1::RigorReviewer),
        ],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [7; 32],
            result_digest: [8; 32],
            all_orthogonal: true,
        },
        fuel: 4,
    };
    let result = ModelResultV1 {
        receipt: ResultReceiptV1 {
            effect: EffectIdV1([13; 16]),
            execution_id: [14; 16],
            task: TaskRefV1 {
                external_task_key: [15; 16],
                incarnation: 16,
            },
            attempt_number: 17,
            disposition: DispositionV1::Completed,
        },
        evidence_digest: [18; 32],
        input_tokens: 1,
        output_tokens: 1,
        output: selected_output,
        proposals: vec![ModelProposalV1::VellumEdit {
            workspace: [19; 16],
            document: [20; 16],
            expected_revision: 7,
            replacement: b"replacement".to_vec(),
        }],
    };
    (
        plan,
        vec![selected_report.clone(), other_report],
        MemberOutcomeV1::Report {
            member: EnsembleMemberIdV1([11; 16]),
            report: selected_report,
            response: MemberResponseV1 {
                claims: vec![MemberClaimV1 {
                    kind: ClaimKindV1::Normative,
                    statement: "edit".into(),
                    statement_digest: Sha256::digest(b"edit").into(),
                    evidence_digests: Vec::new(),
                    dissent_digests: Vec::new(),
                }],
            },
            result: Box::new(result),
        },
    )
}

#[test]
fn authorized_ensemble_edit_crosses_live_aegis_and_lookup_only_vellum_recovery() {
    let fixture = Fixture::new();
    let (plan, reports, outcome) = unresolved_outcome();
    let mut deliberations =
        Deliberations(DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap());
    deliberations
        .apply(
            deliberations.revision(),
            RegistryCommandV1::Admit(Box::new(plan.clone())),
        )
        .unwrap();
    for report in reports {
        deliberations
            .apply(
                deliberations.revision(),
                RegistryCommandV1::SubmitReport {
                    deliberation: plan.id,
                    report,
                },
            )
            .unwrap();
    }
    let authority_id = EngineId::new([23; 16]);
    let authority = DurableAuthority::create_new(
        AuthorityStore::default(),
        authority_id,
        AegisLimits::default(),
    )
    .unwrap();
    let mut service =
        AegisService::new(authority, authority_id, ServiceLimitsV1::default()).unwrap();
    let principal = [24; 16];
    let session = service
        .open_session(AuthenticatedSessionV1 {
            principal: PrincipalId::new(principal),
            session: SessionId::new([25; 16]),
            administrator: true,
        })
        .unwrap();
    let AegisReplyV1::Accepted(AegisOutcomeV1::GrantIssued(grant)) = service
        .execute(
            session,
            AegisCommandV1::Issue {
                expected_epoch: 0,
                now: 1,
                expires_at: 10,
                scopes: vec![ScopeV1 {
                    engine: EngineNamespaceV1::Moira,
                    resource: choice_resource(plan.id),
                    actions: 1 << ActionV1::Administer as u8,
                }],
            },
        )
        .unwrap()
    else {
        panic!("action-selection grant expected")
    };
    let mut choice_authority = AegisChoiceAuthorityV1::new(
        AuthorityClient::new(session, AegisPort(service)),
        grant.handle,
        principal,
        CandidateIdV1([2; 16]),
        FixedTime(2),
    );
    let resolved =
        resolve_authorized_choice(&mut deliberations, plan.id, &mut choice_authority).unwrap();
    let checkpoint = resolved.deliberation;
    let Some(ResolutionV1::AuthorizedChoice {
        authority: choice_decision,
        ..
    }) = checkpoint.resolution
    else {
        panic!("authorized choice expected")
    };
    let (port, _) = choice_authority.into_parts();
    let mut selector = AegisActionSelectorV1::new(
        AuthorityClient::new(session, port),
        grant.handle,
        principal,
        FixedTime(3),
    );
    let selected = select_authorized(&checkpoint, &outcome, &mut selector).unwrap();
    let moira_local_harness::action_selection::ActionSelectionEvidenceV1::Authorized {
        authority: action_decision,
    } = selected.evidence()
    else {
        panic!("authorized action selection expected")
    };
    assert_ne!(choice_decision.decision, action_decision.decision);

    let limits = LimitsV1 {
        max_sagas: 4,
        max_proposals_per_saga: 4,
        max_proposal_bytes: 4_096,
    };
    let store = ModelSagaSqliteStore::open(
        fixture.0.join("saga.db"),
        [26; 16],
        ModelSagaStoreLimitsV1 {
            codec: moira_core::model_saga::persistence::CodecLimitsV1 {
                max_bytes: 64 * 1_024,
                max_sagas: 4,
                max_proposals_per_saga: 4,
                max_proposal_bytes: 4_096,
            },
            saga: limits,
            max_database_pages: 64,
        },
    )
    .unwrap();
    let mut saga_client = ModelSagaMutationClient::new(SagaPort(ModelSagaService::new(store)));
    let publisher = DurableResponsePublisher::new(
        PublisherStore(PublisherRecordV1 {
            revision: 0,
            checkpoint: PublisherCheckpointV1::initial(ClientIdV2(1)),
        }),
        UnusedAgoraPort,
        PublicationLimitsV1::default(),
    )
    .unwrap();
    let workspace = WorkspaceIdV1::new(1).unwrap();
    let document = WorkspaceDocumentIdV1::new(workspace, 2).unwrap();
    let save = SaveReceiptV1 {
        application: SaveApplicationV1::Written,
        synchronization: SaveSynchronizationV1::Succeeded,
        no_future_effects: true,
        quiescent: true,
        adapter_failure: false,
        cleanup_failed: false,
        baseline_published: true,
        outcome_unknown: false,
    };
    let vellum_calls = Rc::new(RefCell::new(VellumCalls::default()));
    let editor = EditAdapter::connect(
        VellumPort {
            calls: Rc::clone(&vellum_calls),
            outcome: DurableExecuteOutcomeV1::Completed(
                DurableEffectReceiptV1::DocumentReplacedAndSaved(DurableReplaceAndSaveReceiptV1 {
                    document,
                    source_revision: 8,
                    text: Arc::from("replacement"),
                    persistence: EditPersistenceV1::Durable(save),
                }),
            ),
            lose_next: true,
        },
        BTreeMap::from([(
            ExternalDocumentRefV1 {
                workspace: [19; 16],
                document: [20; 16],
            },
            DocumentRouteV1 {
                workspace,
                document,
            },
        )]),
        1_024,
    )
    .unwrap();
    let mut router = FederatedPeerRouterV1::new(publisher, BTreeMap::new(), editor);
    let workflow = WorkflowIdV1([27; 16]);
    let first = execute_selected_peer_turn(
        &mut saga_client,
        &mut router,
        None,
        workflow,
        selected,
        |workspace_ref, document_ref, revision| {
            assert_eq!(
                (workspace_ref, document_ref, revision),
                ([19; 16], [20; 16], 7)
            );
            Ok::<_, ()>(12)
        },
    )
    .unwrap();
    let PeerTurnOutcomeV1::Completed {
        storage_revision: 2,
        ..
    } = first
    else {
        panic!("completed Vellum turn expected: {first:?}")
    };
    {
        let calls = vellum_calls.borrow();
        assert_eq!(calls.executions.len(), 1);
        assert_eq!(calls.lookups.len(), 1);
        assert_eq!(calls.lookups, calls.executions);
    }

    assert!(matches!(
        execute_selected_peer_turn(
            &mut saga_client,
            &mut router,
            Some(2),
            workflow,
            selected,
            |_, _, _| Ok::<_, ()>(12),
        )
        .unwrap(),
        PeerTurnOutcomeV1::Completed { .. }
    ));
    assert_eq!(vellum_calls.borrow().executions.len(), 1);
    assert_eq!(vellum_calls.borrow().lookups.len(), 1);
}
