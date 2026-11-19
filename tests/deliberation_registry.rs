use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, DeliberationLimitsV1, DeliberationPlanV1, DeliberationRefusalV1,
    EnsembleAdmissionV1, EnsembleMemberIdV1, EnsembleMemberV1, FindingDispositionIdV1, FindingIdV1,
    GroundingRefV1, MaybeValueV1, MemberReportV1, MethodRefV1, OrthogonalityEvidenceV1,
    PersonaLensV1, RebuttalIdV1, ReportIdV1, ResolutionV1, StakeholderRefV1, SurfaceDepthV1,
    registry::{
        DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryRefusalV1,
        RegistryRevisionV1,
    },
    review::{
        FindingDispositionKindV1, FindingDispositionV1, RebuttalV1, ReviewFindingV1, ReviewPhaseV1,
    },
};

fn plan(seed: u8) -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([seed; 16]),
        question_digest: [seed.wrapping_add(9); 32],
        method: MethodRefV1 {
            machine_digest: [seed; 32],
            state_digest: [seed.wrapping_add(1); 32],
        },
        grounding_provenance: None,
        claims: vec![ClaimKindV1::Empirical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([1; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([1; 16]),
            scores: vec![1],
        }],
        admission: EnsembleAdmissionV1::affirmed([10; 32], [11; 32], [12; 32], [13; 32], [14; 16]),
        members: vec![EnsembleMemberV1 {
            id: EnsembleMemberIdV1([1; 16]),
            stakeholder: StakeholderRefV1::technical_sme([1; 16], [2; 32]),
            lens: PersonaLensV1::Researcher,
            concern_root_digest: [2; 32],
            grounding: GroundingRefV1 {
                authority_digest: [3; 32],
                surface_digest: [4; 32],
                depth: SurfaceDepthV1::Activated,
            },
        }],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [5; 32],
            result_digest: [6; 32],
            all_orthogonal: true,
        },
        fuel: 2,
    }
}

fn report(seed: u8) -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([seed; 16]),
        member: EnsembleMemberIdV1([1; 16]),
        output_digest: [7; 32],
        evidence_digests: vec![[8; 32]],
        dissent_digests: vec![[9; 32]],
    }
}

fn review_plan() -> DeliberationPlanV1 {
    let mut value = plan(1);
    value.members.push(EnsembleMemberV1 {
        id: EnsembleMemberIdV1([2; 16]),
        stakeholder: StakeholderRefV1::technical_sme([2; 16], [3; 32]),
        lens: PersonaLensV1::RigorReviewer,
        concern_root_digest: [3; 32],
        grounding: GroundingRefV1 {
            authority_digest: [4; 32],
            surface_digest: [5; 32],
            depth: SurfaceDepthV1::Activated,
        },
    });
    value.fuel = 5;
    value
}

fn member_report(seed: u8) -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([seed; 16]),
        member: EnsembleMemberIdV1([seed; 16]),
        output_digest: [seed; 32],
        evidence_digests: vec![[seed.wrapping_add(1); 32]],
        dissent_digests: vec![[seed.wrapping_add(2); 32]],
    }
}

#[test]
fn admission_reports_and_resolution_are_revisioned_and_replay_safe() {
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    let admitted = registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan(1))),
        )
        .unwrap();
    assert_eq!(admitted.revision, RegistryRevisionV1(1));
    assert!(!admitted.replayed);

    let replay = registry
        .apply(
            RegistryRevisionV1(1),
            RegistryCommandV1::Admit(Box::new(plan(1))),
        )
        .unwrap();
    assert_eq!(replay.revision, RegistryRevisionV1(1));
    assert!(replay.replayed);

    let accepted = registry
        .apply(
            RegistryRevisionV1(1),
            RegistryCommandV1::SubmitReport {
                deliberation: DeliberationIdV1([1; 16]),
                report: report(1),
            },
        )
        .unwrap();
    assert_eq!(accepted.revision, RegistryRevisionV1(2));
    assert!(!accepted.replayed);

    let report_replay = registry
        .apply(
            RegistryRevisionV1(2),
            RegistryCommandV1::SubmitReport {
                deliberation: DeliberationIdV1([1; 16]),
                report: report(1),
            },
        )
        .unwrap();
    assert!(report_replay.replayed);
    assert_eq!(report_replay.revision, RegistryRevisionV1(2));

    registry
        .apply(
            RegistryRevisionV1(2),
            RegistryCommandV1::Resolve {
                deliberation: DeliberationIdV1([1; 16]),
                resolution: ResolutionV1::Maybe(MaybeValueV1::TrueOnly),
            },
        )
        .unwrap();
    assert_eq!(registry.revision(), RegistryRevisionV1(3));
}

#[test]
fn stale_revision_and_conflicting_identity_refuse_before_mutation() {
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan(1))),
        )
        .unwrap();
    let before = registry.checkpoint();

    assert_eq!(
        registry.apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan(2))),
        ),
        Err(RegistryRefusalV1::RevisionConflict {
            expected: RegistryRevisionV1(0),
            actual: RegistryRevisionV1(1),
        })
    );
    let mut changed = plan(1);
    changed.question_digest = [99; 32];
    assert_eq!(
        registry.apply(
            RegistryRevisionV1(1),
            RegistryCommandV1::Admit(Box::new(changed)),
        ),
        Err(RegistryRefusalV1::InputConflict)
    );
    assert_eq!(registry.checkpoint(), before);
}

#[test]
fn registry_checkpoint_recovery_checks_transition_census_and_capacity() {
    let limits = RegistryLimitsV1 {
        max_deliberations: 1,
        deliberation: DeliberationLimitsV1::default(),
    };
    let mut registry = DeliberationRegistryV1::try_new(limits).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan(1))),
        )
        .unwrap();
    assert_eq!(
        registry.apply(
            RegistryRevisionV1(1),
            RegistryCommandV1::Admit(Box::new(plan(2))),
        ),
        Err(RegistryRefusalV1::Capacity)
    );

    let checkpoint = registry.checkpoint();
    assert_eq!(
        DeliberationRegistryV1::recover(checkpoint.clone())
            .unwrap()
            .checkpoint(),
        checkpoint
    );
    let mut corrupted = checkpoint;
    corrupted.revision = RegistryRevisionV1(2);
    assert_eq!(
        DeliberationRegistryV1::recover(corrupted),
        Err(RegistryRefusalV1::CheckpointMismatch)
    );
}

#[test]
fn registry_recovery_refuses_noncanonical_deliberation_order() {
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan(2))),
        )
        .unwrap();
    registry
        .apply(
            RegistryRevisionV1(1),
            RegistryCommandV1::Admit(Box::new(plan(1))),
        )
        .unwrap();
    let mut checkpoint = registry.checkpoint();
    checkpoint.deliberations.reverse();
    assert_eq!(
        DeliberationRegistryV1::recover(checkpoint),
        Err(RegistryRefusalV1::CheckpointMismatch)
    );
}

#[test]
fn conflicting_report_replay_is_a_typed_deliberation_refusal() {
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan(1))),
        )
        .unwrap();
    registry
        .apply(
            RegistryRevisionV1(1),
            RegistryCommandV1::SubmitReport {
                deliberation: DeliberationIdV1([1; 16]),
                report: report(1),
            },
        )
        .unwrap();
    let mut changed = report(1);
    changed.output_digest = [42; 32];
    assert_eq!(
        registry.apply(
            RegistryRevisionV1(2),
            RegistryCommandV1::SubmitReport {
                deliberation: DeliberationIdV1([1; 16]),
                report: changed,
            },
        ),
        Err(RegistryRefusalV1::Deliberation(
            DeliberationRefusalV1::ReportConflict
        ))
    );
}

#[test]
fn review_exchange_is_revisioned_with_its_deliberation_and_recovers_exactly() {
    let id = DeliberationIdV1([1; 16]);
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(review_plan())),
        )
        .unwrap();
    for seed in [1, 2] {
        registry
            .apply(
                registry.revision(),
                RegistryCommandV1::SubmitReport {
                    deliberation: id,
                    report: member_report(seed),
                },
            )
            .unwrap();
    }
    let begun = registry
        .apply(
            registry.revision(),
            RegistryCommandV1::BeginReview {
                deliberation: id,
                correlation:
                    moira_core::deliberation::review::EnsembleCorrelationEvidenceV1::undisclosed(2),
            },
        )
        .unwrap();
    assert_eq!(begun.revision, RegistryRevisionV1(4));
    assert_eq!(
        begun.review.as_ref().unwrap().findings.len(),
        0,
        "beginning review retains first passes without fabricating findings"
    );
    registry
        .apply(
            registry.revision(),
            RegistryCommandV1::SubmitFinding {
                deliberation: id,
                finding: ReviewFindingV1 {
                    id: FindingIdV1([20; 16]),
                    source_report: ReportIdV1([1; 16]),
                    owner: [21; 16],
                    finding_digest: [22; 32],
                    evidence_digests: vec![[23; 32]],
                    requires_human_gate: false,
                },
            },
        )
        .unwrap();
    registry
        .apply(
            registry.revision(),
            RegistryCommandV1::SubmitRebuttal {
                deliberation: id,
                rebuttal: RebuttalV1 {
                    id: RebuttalIdV1([24; 16]),
                    finding: FindingIdV1([20; 16]),
                    member: EnsembleMemberIdV1([2; 16]),
                    rebuttal_digest: [25; 32],
                    evidence_digests: vec![[26; 32]],
                },
            },
        )
        .unwrap();
    registry
        .apply(
            registry.revision(),
            RegistryCommandV1::DisposeFinding {
                deliberation: id,
                disposition: FindingDispositionV1 {
                    id: FindingDispositionIdV1([27; 16]),
                    finding: FindingIdV1([20; 16]),
                    owner: [21; 16],
                    kind: FindingDispositionKindV1::Mitigated,
                    rationale_digest: [28; 32],
                    evidence_digests: vec![[29; 32]],
                    human_gate_response: None,
                    decided_at: 30,
                },
            },
        )
        .unwrap();
    assert_eq!(registry.revision(), RegistryRevisionV1(7));
    let checkpoint = registry.checkpoint();
    assert_eq!(registry.review(id).unwrap().dispositions.len(), 1);
    assert_eq!(
        moira_core::deliberation::review::ReviewExchangeV1::recover(registry.review(id).unwrap())
            .unwrap()
            .phase(),
        ReviewPhaseV1::Disposed
    );
    assert_eq!(
        DeliberationRegistryV1::recover(checkpoint.clone())
            .unwrap()
            .checkpoint(),
        checkpoint
    );
}
