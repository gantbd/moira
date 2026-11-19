use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, DeliberationLimitsV1, DeliberationPlanV1, EnsembleMemberIdV1,
    EnsembleMemberV1, FindingIdV1, GroundingRefV1, MaybeValueV1, MemberReportV1, MethodRefV1,
    OrthogonalityEvidenceV1, PersonaLensV1, RebuttalIdV1, ReportIdV1, ResolutionV1, SurfaceDepthV1,
    registry::{
        RegistryCommandV1, RegistryLimitsV1, RegistryRevisionV1, persistence::CodecLimitsV1,
    },
    review::{RebuttalV1, ReviewFindingV1},
};
use moira_sqlite::deliberation::{
    ApplyFailureV1, DeliberationSqliteStore, DeliberationStoreErrorV1, DeliberationStoreLimitsV1,
};
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
            "moira-deliberation-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self { directory }
    }

    fn path(&self) -> PathBuf {
        self.directory.join("deliberation.db")
    }

    fn limits(&self) -> DeliberationStoreLimitsV1 {
        let deliberation = DeliberationLimitsV1 {
            max_claims: 8,
            max_criteria: 8,
            max_candidates: 8,
            max_members: 4,
            max_evidence_per_report: 8,
            max_dissent_per_report: 8,
            max_fuel: 32,
        };
        DeliberationStoreLimitsV1 {
            codec: CodecLimitsV1 {
                max_bytes: 65_536,
                max_deliberation_bytes: 32_768,
                registry: RegistryLimitsV1 {
                    max_deliberations: 8,
                    deliberation,
                },
            },
            max_database_pages: 64,
        }
    }

    fn open(&self) -> DeliberationSqliteStore {
        DeliberationSqliteStore::open(self.path(), [9; 16], self.limits()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in [
            "deliberation.db",
            "deliberation.db-journal",
            "deliberation.db.owner.lock",
        ] {
            let _ = fs::remove_file(self.directory.join(name));
        }
        let _ = fs::remove_dir(&self.directory);
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
            [12; 32], [13; 32], [14; 32], [15; 32], [16; 16],
        ),
        members: vec![EnsembleMemberV1 {
            id: EnsembleMemberIdV1([6; 16]),
            stakeholder: moira_core::deliberation::StakeholderRefV1::technical_sme(
                [6; 16], [7; 32],
            ),
            lens: PersonaLensV1::Researcher,
            concern_root_digest: [7; 32],
            grounding: GroundingRefV1 {
                authority_digest: [8; 32],
                surface_digest: [9; 32],
                depth: SurfaceDepthV1::Reference,
            },
        }],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [10; 32],
            result_digest: [11; 32],
            all_orthogonal: true,
        },
        fuel: 2,
    }
}

fn report() -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([12; 16]),
        member: EnsembleMemberIdV1([6; 16]),
        output_digest: [13; 32],
        evidence_digests: vec![[14; 32]],
        dissent_digests: vec![[15; 32]],
    }
}

fn review_plan() -> DeliberationPlanV1 {
    let mut value = plan();
    value.members.push(EnsembleMemberV1 {
        id: EnsembleMemberIdV1([17; 16]),
        stakeholder: moira_core::deliberation::StakeholderRefV1::technical_sme([17; 16], [18; 32]),
        lens: PersonaLensV1::RigorReviewer,
        concern_root_digest: [18; 32],
        grounding: GroundingRefV1 {
            authority_digest: [19; 32],
            surface_digest: [20; 32],
            depth: SurfaceDepthV1::Activated,
        },
    });
    value.fuel = 4;
    value
}

fn second_report() -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([21; 16]),
        member: EnsembleMemberIdV1([17; 16]),
        output_digest: [22; 32],
        evidence_digests: vec![[23; 32]],
        dissent_digests: vec![[24; 32]],
    }
}

#[test]
fn registry_reopens_and_matching_replay_does_not_advance_revision() {
    let fixture = Fixture::new();
    {
        let mut store = fixture.open();
        let admitted = store
            .apply(
                RegistryRevisionV1(0),
                RegistryCommandV1::Admit(Box::new(plan())),
            )
            .unwrap();
        assert_eq!(admitted.revision, RegistryRevisionV1(1));
        store
            .apply(
                RegistryRevisionV1(1),
                RegistryCommandV1::SubmitReport {
                    deliberation: DeliberationIdV1([1; 16]),
                    report: report(),
                },
            )
            .unwrap();
    }

    let mut reopened = fixture.open();
    assert_eq!(reopened.revision(), RegistryRevisionV1(2));
    let retained = reopened.deliberation(DeliberationIdV1([1; 16])).unwrap();
    assert_eq!(retained.reports[0].dissent_digests, vec![[15; 32]]);
    assert_eq!(retained.plan.members[0].grounding.surface_digest, [9; 32]);

    let replay = reopened
        .apply(
            RegistryRevisionV1(2),
            RegistryCommandV1::SubmitReport {
                deliberation: DeliberationIdV1([1; 16]),
                report: report(),
            },
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.revision, RegistryRevisionV1(2));
    reopened
        .apply(
            RegistryRevisionV1(2),
            RegistryCommandV1::Resolve {
                deliberation: DeliberationIdV1([1; 16]),
                resolution: ResolutionV1::Maybe(MaybeValueV1::Neither),
            },
        )
        .unwrap();
    drop(reopened);

    let reopened = fixture.open();
    assert_eq!(reopened.revision(), RegistryRevisionV1(3));
    assert_eq!(
        reopened
            .deliberation(DeliberationIdV1([1; 16]))
            .unwrap()
            .resolution,
        Some(ResolutionV1::Maybe(MaybeValueV1::Neither))
    );
}

#[test]
fn stale_revision_remains_a_domain_refusal_without_persistent_change() {
    let fixture = Fixture::new();
    let mut store = fixture.open();
    store
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan())),
        )
        .unwrap();
    assert!(matches!(
        store.apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan())),
        ),
        Err(ApplyFailureV1::Domain(
            moira_core::deliberation::registry::RegistryRefusalV1::RevisionConflict { .. }
        ))
    ));
    drop(store);
    assert_eq!(fixture.open().revision(), RegistryRevisionV1(1));
}

#[test]
fn review_findings_and_rebuttals_survive_owner_restart() {
    let fixture = Fixture::new();
    let id = DeliberationIdV1([1; 16]);
    {
        let mut store = fixture.open();
        store
            .apply(
                store.revision(),
                RegistryCommandV1::Admit(Box::new(review_plan())),
            )
            .unwrap();
        for retained in [report(), second_report()] {
            store
                .apply(
                    store.revision(),
                    RegistryCommandV1::SubmitReport {
                        deliberation: id,
                        report: retained,
                    },
                )
                .unwrap();
        }
        store
            .apply(
                store.revision(),
                RegistryCommandV1::BeginReview {
                    deliberation: id,
                    correlation: moira_core::deliberation::review::EnsembleCorrelationEvidenceV1::undisclosed(2),
                },
            )
            .unwrap();
        store
            .apply(
                store.revision(),
                RegistryCommandV1::SubmitFinding {
                    deliberation: id,
                    finding: ReviewFindingV1 {
                        id: FindingIdV1([25; 16]),
                        source_report: ReportIdV1([12; 16]),
                        owner: [26; 16],
                        finding_digest: [27; 32],
                        evidence_digests: vec![[28; 32]],
                        requires_human_gate: false,
                    },
                },
            )
            .unwrap();
        store
            .apply(
                store.revision(),
                RegistryCommandV1::SubmitRebuttal {
                    deliberation: id,
                    rebuttal: RebuttalV1 {
                        id: RebuttalIdV1([29; 16]),
                        finding: FindingIdV1([25; 16]),
                        member: EnsembleMemberIdV1([17; 16]),
                        rebuttal_digest: [30; 32],
                        evidence_digests: vec![[31; 32]],
                    },
                },
            )
            .unwrap();
        assert_eq!(store.revision(), RegistryRevisionV1(6));
    }

    let reopened = fixture.open();
    let review = reopened.review(id).expect("review survives restart");
    assert_eq!(review.findings.len(), 1);
    assert_eq!(review.rebuttals.len(), 1);
    assert_eq!(review.reports.len(), 2);
}

#[test]
fn profile_drift_owner_mismatch_and_foreign_database_are_refused() {
    let fixture = Fixture::new();
    drop(fixture.open());

    let mut changed = fixture.limits();
    changed.codec.registry.max_deliberations = 7;
    assert!(matches!(
        DeliberationSqliteStore::open(fixture.path(), [9; 16], changed),
        Err(DeliberationStoreErrorV1::InvalidConfig)
    ));
    assert!(matches!(
        DeliberationSqliteStore::open(fixture.path(), [8; 16], fixture.limits()),
        Err(DeliberationStoreErrorV1::OwnerMismatch)
    ));

    let foreign = fixture.directory.join("foreign.db");
    let connection = rusqlite::Connection::open(&foreign).unwrap();
    connection
        .execute("CREATE TABLE foreign_state(value INTEGER)", [])
        .unwrap();
    drop(connection);
    assert!(matches!(
        DeliberationSqliteStore::open(&foreign, [9; 16], fixture.limits()),
        Err(DeliberationStoreErrorV1::ForeignDatabase)
    ));
    for name in ["foreign.db", "foreign.db-journal", "foreign.db.owner.lock"] {
        let _ = fs::remove_file(fixture.directory.join(name));
    }
}
