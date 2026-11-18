use moira_core::deliberation::{
    AuthorityDecisionIdV1, AuthorityRefV1, AuthoritySubjectIdV1, CandidateIdV1, CandidateV1,
    ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1, DeliberationIdV1,
    DeliberationLimitsV1, DeliberationPhaseV1, DeliberationPlanV1, DeliberationRefusalV1,
    DeliberationV1, EnsembleAdmissionV1, EnsembleFitnessV1, EnsembleMemberIdV1, EnsembleMemberV1,
    FrontierRefusalV1, GroundingRefV1, MemberReportV1, MethodRefV1, MuReasonV1,
    OrthogonalityEvidenceV1, PersonaLensV1, ReportIdV1, ResolutionV1, StakeholderRefV1,
    StakeholderRepresentationV1, SurfaceDepthV1, pareto_frontier,
};

fn criterion(seed: u8, direction: CriterionDirectionV1) -> CriterionV1 {
    CriterionV1 {
        id: CriterionIdV1([seed; 16]),
        direction,
    }
}

fn member(seed: u8, lens: PersonaLensV1) -> EnsembleMemberV1 {
    EnsembleMemberV1 {
        id: EnsembleMemberIdV1([seed; 16]),
        stakeholder: StakeholderRefV1::technical_sme([seed; 16], [seed; 32]),
        lens,
        concern_root_digest: [seed; 32],
        grounding: GroundingRefV1 {
            authority_digest: [seed.wrapping_add(1); 32],
            surface_digest: [seed.wrapping_add(2); 32],
            depth: SurfaceDepthV1::Activated,
        },
    }
}

fn plan(fuel: u32) -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([1; 16]),
        question_digest: [20; 32],
        method: MethodRefV1 {
            machine_digest: [2; 32],
            state_digest: [3; 32],
        },
        claims: vec![ClaimKindV1::Normative],
        criteria: vec![
            criterion(1, CriterionDirectionV1::Maximize),
            criterion(2, CriterionDirectionV1::Minimize),
        ],
        candidates: vec![candidate(1, &[10, 10]), candidate(2, &[5, 1])],
        admission: EnsembleAdmissionV1::affirmed([10; 32], [11; 32], [12; 32], [13; 32], [14; 16]),
        members: vec![
            member(1, PersonaLensV1::Historian),
            member(2, PersonaLensV1::RigorReviewer),
        ],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [4; 32],
            result_digest: [5; 32],
            all_orthogonal: true,
        },
        fuel,
    }
}

fn report(seed: u8) -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([seed; 16]),
        member: EnsembleMemberIdV1([seed; 16]),
        output_digest: [seed; 32],
        evidence_digests: vec![[seed.wrapping_add(1); 32]],
        dissent_digests: vec![[seed.wrapping_add(2); 32]],
    }
}

fn candidate(seed: u8, scores: &[i64]) -> CandidateV1 {
    CandidateV1 {
        id: CandidateIdV1([seed; 16]),
        scores: scores.to_vec(),
    }
}

#[test]
fn unfit_or_contested_ensemble_is_retained_but_cannot_submit_substantive_reports() {
    for fitness in [EnsembleFitnessV1::No, EnsembleFitnessV1::Contested] {
        let mut value = plan(3);
        value.admission.fitness = fitness;
        let mut deliberation = DeliberationV1::try_new(value, DeliberationLimitsV1::default())
            .expect("the cohort problem must remain inspectable");
        assert_eq!(
            deliberation.submit_report(report(1)),
            Err(DeliberationRefusalV1::EnsembleNotAdmitted)
        );
        deliberation
            .resolve(ResolutionV1::Mu(MuReasonV1::WrongEnsemble))
            .expect("the retained review can terminate honestly as the wrong ensemble");
    }
}

#[test]
fn stakeholder_standing_cannot_be_fabricated_from_a_persona() {
    let mut value = plan(3);
    value.members[0].stakeholder = StakeholderRefV1 {
        principal: [1; 16],
        constituency_digest: [2; 32],
        representation: StakeholderRepresentationV1::AgentModeledPerspective,
        accountable_principal: None,
        mandate_digest: Some([3; 32]),
        conflict_digest: None,
    };
    assert_eq!(
        DeliberationV1::try_new(value, DeliberationLimitsV1::default()),
        Err(DeliberationRefusalV1::InvalidStakeholder)
    );
}

#[test]
fn ensemble_admission_requires_exact_nonzero_equity_evidence() {
    let mut value = plan(3);
    value.admission.equity_assessment_digest = [0; 32];
    assert_eq!(
        DeliberationV1::try_new(value, DeliberationLimitsV1::default()),
        Err(DeliberationRefusalV1::InvalidEnsembleAdmission)
    );
}

#[test]
fn frontier_removes_only_dominated_candidates_and_preserves_survivor_order() {
    let criteria = [
        criterion(1, CriterionDirectionV1::Maximize),
        criterion(2, CriterionDirectionV1::Minimize),
    ];
    let candidates = [
        candidate(1, &[8, 8]),
        candidate(2, &[9, 8]),
        candidate(3, &[7, 3]),
        candidate(4, &[6, 9]),
    ];
    assert_eq!(
        pareto_frontier(&criteria, &candidates).unwrap(),
        vec![CandidateIdV1([2; 16]), CandidateIdV1([3; 16])]
    );
}

#[test]
fn incomparable_candidates_remain_choices_instead_of_becoming_a_boolean_winner() {
    let criteria = [
        criterion(1, CriterionDirectionV1::Maximize),
        criterion(2, CriterionDirectionV1::Minimize),
    ];
    let candidates = [candidate(1, &[10, 10]), candidate(2, &[5, 1])];
    assert_eq!(
        pareto_frontier(&criteria, &candidates).unwrap(),
        vec![CandidateIdV1([1; 16]), CandidateIdV1([2; 16])]
    );
}

#[test]
fn malformed_frontier_inputs_refuse_without_guessing() {
    let c = criterion(1, CriterionDirectionV1::Maximize);
    assert_eq!(
        pareto_frontier(&[], &[candidate(1, &[])]),
        Err(FrontierRefusalV1::NoCriteria)
    );
    assert_eq!(
        pareto_frontier(&[c], &[]),
        Err(FrontierRefusalV1::NoCandidates)
    );
    assert_eq!(
        pareto_frontier(&[c, c], &[candidate(1, &[1, 1])]),
        Err(FrontierRefusalV1::DuplicateCriterion)
    );
    assert_eq!(
        pareto_frontier(&[c], &[candidate(1, &[1]), candidate(1, &[2])]),
        Err(FrontierRefusalV1::DuplicateCandidate)
    );
    assert_eq!(
        pareto_frontier(&[c], &[candidate(1, &[1, 2])]),
        Err(FrontierRefusalV1::ScoreDimensionMismatch)
    );
}

#[test]
fn correlated_ensemble_is_refused_before_reports_can_create_false_consensus() {
    let mut candidate = plan(2);
    candidate.orthogonality.all_orthogonal = false;
    assert_eq!(
        DeliberationV1::try_new(candidate, DeliberationLimitsV1::default()),
        Err(DeliberationRefusalV1::NonOrthogonalEnsemble)
    );
}

#[test]
fn reports_are_attributed_replay_safe_and_dissent_survives_resolution() {
    let mut deliberation =
        DeliberationV1::try_new(plan(3), DeliberationLimitsV1::default()).unwrap();
    let first = report(1);
    deliberation.submit_report(first.clone()).unwrap();
    assert_eq!(deliberation.remaining_fuel(), 2);
    deliberation.submit_report(first).unwrap();
    assert_eq!(deliberation.remaining_fuel(), 2);
    deliberation.submit_report(report(2)).unwrap();
    assert_eq!(deliberation.phase(), DeliberationPhaseV1::ReadyToResolve);

    let resolution = ResolutionV1::AuthorizedChoice {
        frontier: vec![CandidateIdV1([1; 16]), CandidateIdV1([2; 16])],
        selected: CandidateIdV1([2; 16]),
        authority: AuthorityRefV1 {
            decision: AuthorityDecisionIdV1([9; 16]),
            subject: AuthoritySubjectIdV1([10; 16]),
            epoch: 11,
        },
    };
    deliberation.resolve(resolution.clone()).unwrap();
    assert_eq!(deliberation.phase(), DeliberationPhaseV1::Resolved);
    assert_eq!(deliberation.resolution(), Some(&resolution));
    assert_eq!(deliberation.reports()[0].dissent_digests, vec![[3; 32]]);
}

#[test]
fn malformed_choice_refuses_without_spending_resolution_fuel() {
    let mut deliberation =
        DeliberationV1::try_new(plan(4), DeliberationLimitsV1::default()).unwrap();
    deliberation.submit_report(report(1)).unwrap();
    deliberation.submit_report(report(2)).unwrap();
    let before = deliberation.remaining_fuel();

    assert_eq!(
        deliberation.resolve(ResolutionV1::AuthorizedChoice {
            frontier: vec![CandidateIdV1([1; 16])],
            selected: CandidateIdV1([1; 16]),
            authority: AuthorityRefV1 {
                decision: AuthorityDecisionIdV1([9; 16]),
                subject: AuthoritySubjectIdV1([10; 16]),
                epoch: 11,
            },
        }),
        Err(DeliberationRefusalV1::FrontierMismatch)
    );
    assert_eq!(deliberation.remaining_fuel(), before);
}

#[test]
fn exhausted_fuel_resolves_to_mu_without_completing_the_ensemble() {
    let mut deliberation =
        DeliberationV1::try_new(plan(1), DeliberationLimitsV1::default()).unwrap();
    deliberation.submit_report(report(1)).unwrap();
    assert_eq!(deliberation.remaining_fuel(), 0);
    assert_eq!(
        deliberation.submit_report(report(2)),
        Err(DeliberationRefusalV1::FuelInvalid)
    );
    deliberation
        .resolve(ResolutionV1::Mu(MuReasonV1::FuelExhausted))
        .unwrap();
    assert_eq!(
        deliberation.resolution(),
        Some(&ResolutionV1::Mu(MuReasonV1::FuelExhausted))
    );
}

#[test]
fn semantic_checkpoint_recovers_exactly_and_refuses_mutated_fuel() {
    let mut deliberation =
        DeliberationV1::try_new(plan(3), DeliberationLimitsV1::default()).unwrap();
    deliberation.submit_report(report(1)).unwrap();
    deliberation.submit_report(report(2)).unwrap();
    deliberation
        .resolve(ResolutionV1::Maybe(
            moira_core::deliberation::MaybeValueV1::Both,
        ))
        .unwrap();

    let checkpoint = deliberation.checkpoint();
    let recovered = DeliberationV1::recover(checkpoint.clone()).unwrap();
    assert_eq!(recovered.checkpoint(), checkpoint);

    let mut mutated = checkpoint;
    mutated.remaining_fuel = mutated.remaining_fuel.saturating_add(1);
    assert_eq!(
        DeliberationV1::recover(mutated),
        Err(DeliberationRefusalV1::CheckpointMismatch)
    );
}
