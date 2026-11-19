use moira_client::{
    DeliberationMutationClient, DeliberationReadOnlyClient, DeliberationServicePort,
};
use moira_core::deliberation::{
    DeliberationLimitsV1,
    registry::{RegistryLimitsV1, persistence::CodecLimitsV1},
};
use moira_protocol::deliberation::*;
use moira_service::deliberation::DeliberationService;
use moira_sqlite::deliberation::{
    DeliberationSqliteStore, DeliberationStoreErrorV1, DeliberationStoreLimitsV1,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Port(DeliberationService<DeliberationSqliteStore>);

impl DeliberationServicePort for Port {
    type Error = DeliberationStoreErrorV1;

    fn execute(&mut self, command: CommandV1) -> Result<ReplyV1, Self::Error> {
        self.0.execute(command)
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "moira-deliberation-public-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self(directory)
    }

    fn path(&self) -> PathBuf {
        self.0.join("deliberation.db")
    }

    fn open(&self) -> DeliberationSqliteStore {
        DeliberationSqliteStore::open(
            self.path(),
            [31; 16],
            DeliberationStoreLimitsV1 {
                codec: CodecLimitsV1 {
                    max_bytes: 65_536,
                    max_deliberation_bytes: 32_768,
                    registry: RegistryLimitsV1 {
                        max_deliberations: 8,
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

fn plan() -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([1; 16]),
        question_digest: [20; 32],
        method: MethodRefV1 {
            machine_digest: [2; 32],
            state_digest: [3; 32],
        },
        claims: vec![ClaimKindV1::Historical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([4; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([5; 16]),
            scores: vec![1],
        }],
        admission: EnsembleAdmissionV1::affirmed([12; 32], [13; 32], [14; 32], [15; 32], [16; 16]),
        members: vec![
            EnsembleMemberV1 {
                id: EnsembleMemberIdV1([6; 16]),
                stakeholder: StakeholderRefV1::technical_sme([6; 16], [7; 32]),
                lens: PersonaLensV1::Historian,
                concern_root_digest: [7; 32],
                grounding: GroundingRefV1 {
                    authority_digest: [8; 32],
                    surface_digest: [9; 32],
                    depth: SurfaceDepthV1::Activated,
                },
            },
            EnsembleMemberV1 {
                id: EnsembleMemberIdV1([17; 16]),
                stakeholder: StakeholderRefV1::technical_sme([17; 16], [18; 32]),
                lens: PersonaLensV1::RigorReviewer,
                concern_root_digest: [18; 32],
                grounding: GroundingRefV1 {
                    authority_digest: [19; 32],
                    surface_digest: [20; 32],
                    depth: SurfaceDepthV1::Reference,
                },
            },
        ],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [10; 32],
            result_digest: [11; 32],
            all_orthogonal: true,
        },
        fuel: 2,
    }
}

fn report(id: u8, member: u8) -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([id; 16]),
        member: EnsembleMemberIdV1([member; 16]),
        output_digest: [id; 32],
        evidence_digests: vec![[id.wrapping_add(1); 32]],
        dissent_digests: vec![[id.wrapping_add(2); 32]],
    }
}

#[test]
fn public_client_service_store_journey_survives_reopen() {
    let fixture = Fixture::new();
    {
        let mut client =
            DeliberationMutationClient::new(Port(DeliberationService::new(fixture.open())));
        assert!(matches!(
            client
                .apply(0, MutationV1::Admit(Box::new(plan())))
                .unwrap(),
            ReplyV1::Accepted(OutcomeV1::Applied {
                revision: 1,
                replayed: false,
                ..
            })
        ));
    }

    let mut reader =
        DeliberationReadOnlyClient::new(Port(DeliberationService::new(fixture.open())));
    assert!(matches!(
        reader.status().unwrap(),
        ReplyV1::Accepted(OutcomeV1::Status(StatusV1 {
            revision: 1,
            needs_reopen: false
        }))
    ));
    let ReplyV1::Accepted(OutcomeV1::Deliberation(Some(view))) =
        reader.deliberation(DeliberationIdV1([1; 16])).unwrap()
    else {
        panic!("durable public read expected");
    };
    assert_eq!(view.plan.method.machine_digest, [2; 32]);
    assert_eq!(view.plan.members[0].grounding.surface_digest, [9; 32]);
    assert_eq!(view.phase, DeliberationPhaseV1::Collecting);
}

#[test]
fn public_review_journey_survives_owner_reopen() {
    let fixture = Fixture::new();
    {
        let mut client =
            DeliberationMutationClient::new(Port(DeliberationService::new(fixture.open())));
        client
            .apply(0, MutationV1::Admit(Box::new(plan())))
            .unwrap();
        client
            .apply(
                1,
                MutationV1::SubmitReport {
                    deliberation: DeliberationIdV1([1; 16]),
                    report: report(21, 6),
                },
            )
            .unwrap();
        client
            .apply(
                2,
                MutationV1::SubmitReport {
                    deliberation: DeliberationIdV1([1; 16]),
                    report: report(24, 17),
                },
            )
            .unwrap();
        client
            .apply(
                3,
                MutationV1::BeginReview {
                    deliberation: DeliberationIdV1([1; 16]),
                },
            )
            .unwrap();
        client
            .apply(
                4,
                MutationV1::SubmitFinding {
                    deliberation: DeliberationIdV1([1; 16]),
                    finding: ReviewFindingV1 {
                        id: FindingIdV1([27; 16]),
                        source_report: ReportIdV1([21; 16]),
                        owner: [28; 16],
                        finding_digest: [29; 32],
                        evidence_digests: vec![[30; 32]],
                        requires_human_gate: false,
                    },
                },
            )
            .unwrap();
        client
            .apply(
                5,
                MutationV1::SubmitRebuttal {
                    deliberation: DeliberationIdV1([1; 16]),
                    rebuttal: RebuttalV1 {
                        id: RebuttalIdV1([31; 16]),
                        finding: FindingIdV1([27; 16]),
                        member: EnsembleMemberIdV1([17; 16]),
                        rebuttal_digest: [32; 32],
                        evidence_digests: vec![[33; 32]],
                    },
                },
            )
            .unwrap();
        assert!(matches!(
            client
                .apply(
                    6,
                    MutationV1::DisposeFinding {
                        deliberation: DeliberationIdV1([1; 16]),
                        disposition: FindingDispositionV1 {
                            id: FindingDispositionIdV1([34; 16]),
                            finding: FindingIdV1([27; 16]),
                            owner: [28; 16],
                            kind: FindingDispositionKindV1::Tested,
                            rationale_digest: [35; 32],
                            evidence_digests: vec![[36; 32]],
                            human_gate_response: None,
                            decided_at: 37,
                        },
                    },
                )
                .unwrap(),
            ReplyV1::Accepted(OutcomeV1::ReviewApplied {
                revision: 7,
                ref review,
                ..
            }) if review.phase == ReviewPhaseV1::Disposed
        ));
    }

    let mut reader =
        DeliberationReadOnlyClient::new(Port(DeliberationService::new(fixture.open())));
    let ReplyV1::Accepted(OutcomeV1::Review(Some(review))) =
        reader.review(DeliberationIdV1([1; 16])).unwrap()
    else {
        panic!("durable public review expected");
    };
    assert_eq!(review.phase, ReviewPhaseV1::Disposed);
    assert_eq!(review.reports.len(), 2);
    assert_eq!(review.findings[0].finding_digest, [29; 32]);
    assert_eq!(review.rebuttals[0].rebuttal_digest, [32; 32]);
    assert_eq!(
        review.dispositions[0].kind,
        FindingDispositionKindV1::Tested
    );
}
