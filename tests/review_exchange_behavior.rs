use moira_core::deliberation::{
    CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1, DeliberationIdV1,
    DeliberationLimitsV1, DeliberationPlanV1, DeliberationV1, EnsembleAdmissionV1,
    EnsembleMemberIdV1, EnsembleMemberV1, FindingDispositionIdV1, FindingIdV1, GroundingRefV1,
    HumanGateAuthorityRefV1, HumanGateDispositionV1, HumanGateMaterialRefV1, HumanGateRequestIdV1,
    HumanGateRequestV1, HumanGateResponseIdV1, HumanGateResponseV1, MaybeValueV1, MemberReportV1,
    MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1, RebuttalIdV1, ReportIdV1, ResolutionV1,
    StakeholderRefV1, SurfaceDepthV1,
    review::{
        CorrelationDimensionEvidenceV1, CorrelationDimensionV1, EnsembleCorrelationEvidenceV1,
        FindingDispositionKindV1, FindingDispositionV1, FirstPassExposureV1,
        IndependenceAssessmentV1, RebuttalV1, ReviewExchangeV1, ReviewFindingV1, ReviewLimitsV1,
        ReviewPhaseV1, ReviewRefusalV1,
        persistence::{ReviewCodecLimitsV1, decode_review_checkpoint, encode_review_checkpoint},
    },
};

fn correlation() -> EnsembleCorrelationEvidenceV1 {
    let mut evidence = EnsembleCorrelationEvidenceV1::undisclosed(2);
    evidence.dimensions[0] = CorrelationDimensionEvidenceV1 {
        dimension: CorrelationDimensionV1::ExecutionPrincipal,
        observed_members: 2,
        distinct_values: 1,
    };
    evidence.first_pass = FirstPassExposureV1 {
        committed_before_cross_review: 2,
        exposed_before_commit: 0,
        unknown: 0,
    };
    evidence
}

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
        grounding_provenance: None,
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

fn legacy_review_bytes(
    checkpoint: &moira_core::deliberation::review::ReviewExchangeCheckpointV1,
) -> Vec<u8> {
    let mut bytes = b"MOIRARV1".to_vec();
    bytes.push(1);
    let number = |bytes: &mut Vec<u8>, value: usize| {
        bytes.extend_from_slice(&u64::try_from(value).unwrap().to_be_bytes());
    };
    for value in [
        checkpoint.limits.max_findings,
        checkpoint.limits.max_rebuttals,
        checkpoint.limits.max_evidence_per_item,
    ] {
        number(&mut bytes, value);
    }
    bytes.extend_from_slice(&checkpoint.deliberation.0);
    number(&mut bytes, checkpoint.reports.len());
    for report in &checkpoint.reports {
        bytes.extend_from_slice(&report.id.0);
        bytes.extend_from_slice(&report.member.0);
        bytes.extend_from_slice(&report.output_digest);
        number(&mut bytes, report.evidence_digests.len());
        for digest in &report.evidence_digests {
            bytes.extend_from_slice(digest);
        }
        number(&mut bytes, report.dissent_digests.len());
        for digest in &report.dissent_digests {
            bytes.extend_from_slice(digest);
        }
    }
    bytes.push(0);
    number(&mut bytes, 0);
    number(&mut bytes, 0);
    number(&mut bytes, 0);
    bytes
}

#[test]
fn review_waits_for_independent_first_passes() {
    let value = DeliberationV1::try_new(plan(), DeliberationLimitsV1::default()).unwrap();
    assert_eq!(
        ReviewExchangeV1::try_new(&value, ReviewLimitsV1::default(), correlation()),
        Err(ReviewRefusalV1::FirstPassIncomplete)
    );
}

#[test]
fn cross_member_challenge_precedes_owner_disposition_and_replays_exactly() {
    let mut exchange =
        ReviewExchangeV1::try_new(&resolved(None), ReviewLimitsV1::default(), correlation())
            .expect("complete first passes admit review");
    assert_eq!(
        exchange.correlation().independence(),
        IndependenceAssessmentV1::Correlated
    );
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
        correlation(),
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
        let mut exchange = ReviewExchangeV1::try_new(
            &resolved(Some(response)),
            ReviewLimitsV1::default(),
            correlation(),
        )
        .unwrap();
        exchange.submit_finding(finding(true)).unwrap();
        exchange.submit_rebuttal(rebuttal(2)).unwrap();
        assert_eq!(
            exchange.dispose_finding(disposition(Some(HumanGateResponseIdV1([30; 16])))),
            Err(ReviewRefusalV1::HumanGateNotApproved)
        );
    }
}

#[test]
fn canonical_review_bytes_roundtrip_and_reject_truncation() {
    let mut exchange = ReviewExchangeV1::try_new(
        &resolved(Some(HumanGateDispositionV1::Approved)),
        ReviewLimitsV1::default(),
        correlation(),
    )
    .unwrap();
    exchange.submit_finding(finding(true)).unwrap();
    exchange.submit_rebuttal(rebuttal(2)).unwrap();
    exchange
        .dispose_finding(disposition(Some(HumanGateResponseIdV1([30; 16]))))
        .unwrap();
    let checkpoint = exchange.checkpoint();
    let limits = ReviewCodecLimitsV1::default();
    let bytes = encode_review_checkpoint(&checkpoint, limits).unwrap();
    assert_eq!(
        decode_review_checkpoint(&bytes, limits).unwrap(),
        checkpoint
    );
    assert!(decode_review_checkpoint(&bytes[..bytes.len() - 1], limits).is_err());
}

#[test]
fn legacy_review_migrates_with_explicitly_undisclosed_correlation() {
    let exchange =
        ReviewExchangeV1::try_new(&resolved(None), ReviewLimitsV1::default(), correlation())
            .unwrap();
    let legacy = legacy_review_bytes(&exchange.checkpoint());
    let migrated = decode_review_checkpoint(&legacy, ReviewCodecLimitsV1::default()).unwrap();
    assert_eq!(migrated.version, 2);
    assert_eq!(
        migrated.correlation,
        EnsembleCorrelationEvidenceV1::undisclosed(2)
    );
    assert_ne!(
        encode_review_checkpoint(&migrated, ReviewCodecLimitsV1::default()).unwrap(),
        legacy
    );
}

#[test]
fn distinct_observed_values_never_claim_independence() {
    let mut evidence = EnsembleCorrelationEvidenceV1::undisclosed(2);
    evidence.dimensions[0] = CorrelationDimensionEvidenceV1 {
        dimension: CorrelationDimensionV1::ExecutionPrincipal,
        observed_members: 2,
        distinct_values: 2,
    };
    assert_eq!(
        evidence.independence(),
        IndependenceAssessmentV1::NotEstablished
    );
}

#[test]
fn correlation_requires_complete_explicit_dimension_disclosure() {
    let mut evidence = correlation();
    evidence.dimensions.pop();
    assert_eq!(
        ReviewExchangeV1::try_new(&resolved(None), ReviewLimitsV1::default(), evidence),
        Err(ReviewRefusalV1::InvalidCorrelationEvidence)
    );
}
