use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, DeliberationLimitsV1, DeliberationPlanV1, DeliberationRefusalV1,
    EnsembleAdmissionV1, EnsembleMemberIdV1, EnsembleMemberV1, GroundingRefV1, MaybeValueV1,
    MemberReportV1, MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1, ReportIdV1, ResolutionV1,
    StakeholderRefV1, SurfaceDepthV1,
    registry::{
        DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryRefusalV1,
        RegistryRevisionV1,
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
