use moira_core::deliberation::{
    CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1, DeliberationIdV1,
    DeliberationLimitsV1, DeliberationPlanV1, DeliberationV1, EnsembleAdmissionV1,
    EnsembleMemberIdV1, EnsembleMemberV1, FindingDispositionIdV1, FindingIdV1, GroundingRefV1,
    HumanGateAuthorityRefV1, HumanGateDispositionV1, HumanGateMaterialRefV1, HumanGateRequestIdV1,
    HumanGateRequestV1, HumanGateResponseIdV1, HumanGateResponseV1, MaybeValueV1, MemberReportV1,
    MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1, RebuttalIdV1, ReportIdV1, ResolutionV1,
    StakeholderRefV1, SurfaceDepthV1,
    review::{
        FindingDispositionKindV1, FindingDispositionV1, RebuttalV1, ReviewExchangeV1,
        ReviewFindingV1, ReviewLimitsV1, ReviewPhaseV1, ReviewRefusalV1,
    },
};

fn member(seed: u8) -> EnsembleMemberV1 {
    EnsembleMemberV1 {
        id: EnsembleMemberIdV1([seed; 16]),
        stakeholder: StakeholderRefV1::technical_sme([seed; 16], [seed; 32]),
        lens: if seed == 1 {
            PersonaLensV1::Researcher
        } else {
            PersonaLensV1::RigorReviewer
        },
        concern_root_digest: [seed; 32],
        grounding: GroundingRefV1 {
            authority_digest: [seed.wrapping_add(1); 32],
            surface_digest: [seed.wrapping_add(2); 32],
            depth: SurfaceDepthV1::Activated,
        },
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

fn plan() -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([1; 16]),
        question_digest: [2; 32],
        method: MethodRefV1 {
            machine_digest: [3; 32],
            state_digest: [4; 32],
        },
        claims: vec![ClaimKindV1::Empirical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([5; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: moira_core::deliberation::CandidateIdV1([6; 16]),
            scores: vec![1],
        }],
        admission: EnsembleAdmissionV1::affirmed([7; 32], [8; 32], [9; 32], [10; 32], [11; 16]),
        members: vec![member(1), member(2)],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [12; 32],
            result_digest: [13; 32],
            all_orthogonal: true,
        },
        fuel: 4,
    }
}

fn resolved(gate: Option<HumanGateDispositionV1>) -> DeliberationV1 {
    let mut value = DeliberationV1::try_new(plan(), DeliberationLimitsV1::default()).unwrap();
    value.submit_report(report(1)).unwrap();
    value.submit_report(report(2)).unwrap();
    value
        .resolve(ResolutionV1::Maybe(MaybeValueV1::Both))
        .unwrap();
    if let Some(gate) = gate {
        let material = |seed| HumanGateMaterialRefV1 {
            owner_route: [seed; 16],
            artifact_route: [seed + 1; 16],
            expected_revision: u64::from(seed),
            expected_sha256: [seed + 2; 32],
        };
        value
            .request_human_gate(HumanGateRequestV1 {
                id: HumanGateRequestIdV1([20; 16]),
                reviewer: [21; 16],
                finding_digest: [22; 32],
                decision_digest: [23; 32],
                material_revision_digest: [24; 32],
                policy_digest: [25; 32],
                decision_packet: material(26),
                requested_at: 29,
            })
            .unwrap();
        value
            .respond_human_gate(HumanGateResponseV1 {
                id: HumanGateResponseIdV1([30; 16]),
                request: HumanGateRequestIdV1([20; 16]),
                disposition: gate,
                rationale_digest: [31; 32],
                rationale: material(32),
                responded_at: 35,
                authority: HumanGateAuthorityRefV1 {
                    decision: [36; 16],
                    subject: [21; 16],
                    epoch: 37,
                },
            })
            .unwrap();
    }
    value
}

fn finding(requires_human_gate: bool) -> ReviewFindingV1 {
    ReviewFindingV1 {
        id: FindingIdV1([40; 16]),
        source_report: ReportIdV1([1; 16]),
        owner: [41; 16],
        finding_digest: [42; 32],
        evidence_digests: vec![[43; 32]],
        requires_human_gate,
    }
}

fn rebuttal(member: u8) -> RebuttalV1 {
    RebuttalV1 {
        id: RebuttalIdV1([44; 16]),
        finding: FindingIdV1([40; 16]),
        member: EnsembleMemberIdV1([member; 16]),
        rebuttal_digest: [45; 32],
        evidence_digests: vec![[46; 32]],
    }
}

fn disposition(gate: Option<HumanGateResponseIdV1>) -> FindingDispositionV1 {
    FindingDispositionV1 {
        id: FindingDispositionIdV1([47; 16]),
        finding: FindingIdV1([40; 16]),
        owner: [41; 16],
        kind: FindingDispositionKindV1::Mitigated,
        rationale_digest: [48; 32],
        evidence_digests: vec![[49; 32]],
        human_gate_response: gate,
        decided_at: 50,
    }
}

#[test]
fn review_waits_for_independent_first_passes() {
    let value = DeliberationV1::try_new(plan(), DeliberationLimitsV1::default()).unwrap();
    assert_eq!(
        ReviewExchangeV1::try_new(&value, ReviewLimitsV1::default()),
        Err(ReviewRefusalV1::FirstPassIncomplete)
    );
}

#[test]
fn cross_member_challenge_precedes_owner_disposition_and_replays_exactly() {
    let mut exchange = ReviewExchangeV1::try_new(&resolved(None), ReviewLimitsV1::default())
        .expect("complete first passes admit review");
    exchange.submit_finding(finding(false)).unwrap();
    assert_eq!(
        exchange.submit_rebuttal(rebuttal(1)),
        Err(ReviewRefusalV1::SelfRebuttal)
    );
    assert_eq!(
        exchange.dispose_finding(disposition(None)),
        Err(ReviewRefusalV1::RebuttalRequired)
    );
    exchange.submit_rebuttal(rebuttal(2)).unwrap();
    exchange.dispose_finding(disposition(None)).unwrap();
    assert_eq!(exchange.phase(), ReviewPhaseV1::Disposed);

    let checkpoint = exchange.checkpoint();
    assert_eq!(
        ReviewExchangeV1::recover(checkpoint.clone())
            .unwrap()
            .checkpoint(),
        checkpoint
    );
}

#[test]
fn gated_finding_requires_the_exact_admitted_human_response() {
    let mut exchange = ReviewExchangeV1::try_new(
        &resolved(Some(HumanGateDispositionV1::Approved)),
        ReviewLimitsV1::default(),
    )
    .unwrap();
    exchange.submit_finding(finding(true)).unwrap();
    exchange.submit_rebuttal(rebuttal(2)).unwrap();
    assert_eq!(
        exchange.dispose_finding(disposition(None)),
        Err(ReviewRefusalV1::HumanGateRequired)
    );
    assert_eq!(
        exchange.dispose_finding(disposition(Some(HumanGateResponseIdV1([99; 16])))),
        Err(ReviewRefusalV1::HumanGateMismatch)
    );
    exchange
        .dispose_finding(disposition(Some(HumanGateResponseIdV1([30; 16]))))
        .unwrap();
}

#[test]
fn non_approved_human_responses_never_authorize_disposition() {
    for response in [
        HumanGateDispositionV1::Rejected,
        HumanGateDispositionV1::Deferred,
        HumanGateDispositionV1::ChangesRequested,
    ] {
        let mut exchange =
            ReviewExchangeV1::try_new(&resolved(Some(response)), ReviewLimitsV1::default())
                .unwrap();
        exchange.submit_finding(finding(true)).unwrap();
        exchange.submit_rebuttal(rebuttal(2)).unwrap();
        assert_eq!(
            exchange.dispose_finding(disposition(Some(HumanGateResponseIdV1([30; 16])))),
            Err(ReviewRefusalV1::HumanGateNotApproved)
        );
    }
}
