use moira_core::deliberation::{
    AuthorityDecisionIdV1, AuthorityRefV1, AuthoritySubjectIdV1, CandidateIdV1, CandidateV1,
    ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1, DeliberationIdV1,
    DeliberationLimitsV1, DeliberationPlanV1, DeliberationV1, EnsembleMemberIdV1, EnsembleMemberV1,
    GroundingRefV1, MemberReportV1, MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1,
    ReportIdV1, ResolutionV1, SurfaceDepthV1,
    persistence::{CodecErrorV1, CodecLimitsV1, decode, encode},
};

fn plan() -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([1; 16]),
        question_digest: [20; 32],
        method: MethodRefV1 {
            machine_digest: [2; 32],
            state_digest: [3; 32],
        },
        claims: vec![ClaimKindV1::Historical, ClaimKindV1::Normative],
        criteria: vec![
            CriterionV1 {
                id: CriterionIdV1([1; 16]),
                direction: CriterionDirectionV1::Maximize,
            },
            CriterionV1 {
                id: CriterionIdV1([2; 16]),
                direction: CriterionDirectionV1::Minimize,
            },
        ],
        candidates: vec![
            CandidateV1 {
                id: CandidateIdV1([1; 16]),
                scores: vec![10, -2],
            },
            CandidateV1 {
                id: CandidateIdV1([2; 16]),
                scores: vec![5, -7],
            },
        ],
        members: vec![
            member(1, PersonaLensV1::Historian, SurfaceDepthV1::Activated),
            member(2, PersonaLensV1::RigorReviewer, SurfaceDepthV1::Reference),
        ],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [8; 32],
            result_digest: [9; 32],
            all_orthogonal: true,
        },
        fuel: 3,
    }
}

fn member(seed: u8, lens: PersonaLensV1, depth: SurfaceDepthV1) -> EnsembleMemberV1 {
    EnsembleMemberV1 {
        id: EnsembleMemberIdV1([seed; 16]),
        lens,
        concern_root_digest: [seed; 32],
        grounding: GroundingRefV1 {
            authority_digest: [seed.wrapping_add(1); 32],
            surface_digest: [seed.wrapping_add(2); 32],
            depth,
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

fn checkpoint() -> moira_core::deliberation::DeliberationCheckpointV1 {
    let mut deliberation =
        DeliberationV1::try_new(plan(), DeliberationLimitsV1::default()).unwrap();
    deliberation.submit_report(report(1)).unwrap();
    deliberation.submit_report(report(2)).unwrap();
    deliberation
        .resolve(ResolutionV1::AuthorizedChoice {
            frontier: vec![CandidateIdV1([1; 16]), CandidateIdV1([2; 16])],
            selected: CandidateIdV1([2; 16]),
            authority: AuthorityRefV1 {
                decision: AuthorityDecisionIdV1([7; 16]),
                subject: AuthoritySubjectIdV1([8; 16]),
                epoch: 9,
            },
        })
        .unwrap();
    deliberation.checkpoint()
}

#[test]
fn canonical_bytes_roundtrip_the_complete_deliberation() {
    let checkpoint = checkpoint();
    let limits = CodecLimitsV1::default();
    let bytes = encode(&checkpoint, limits).unwrap();
    let decoded = decode(&bytes, limits).unwrap();
    assert_eq!(decoded, checkpoint);
    assert_eq!(encode(&decoded, limits).unwrap(), bytes);
}

#[test]
fn prior_codec_identity_is_refused_instead_of_reinterpreted() {
    let limits = CodecLimitsV1::default();
    let mut bytes = encode(&checkpoint(), limits).unwrap();
    bytes[..8].copy_from_slice(b"MOIRADL1");
    bytes[8] = 1;
    assert_eq!(decode(&bytes, limits), Err(CodecErrorV1::Invalid));
}

#[test]
fn truncation_trailing_data_and_host_limit_reduction_refuse() {
    let limits = CodecLimitsV1::default();
    let bytes = encode(&checkpoint(), limits).unwrap();
    for end in 0..bytes.len() {
        assert!(decode(&bytes[..end], limits).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(decode(&trailing, limits), Err(CodecErrorV1::Invalid));

    let mut narrow = limits;
    narrow.domain.max_members = 1;
    assert_eq!(decode(&bytes, narrow), Err(CodecErrorV1::Limit));
}

#[test]
fn accepted_single_byte_mutations_remain_canonical() {
    let limits = CodecLimitsV1::default();
    let bytes = encode(&checkpoint(), limits).unwrap();
    for index in 0..bytes.len() {
        let mut mutated = bytes.clone();
        mutated[index] ^= 1;
        if let Ok(decoded) = decode(&mutated, limits) {
            assert_eq!(encode(&decoded, limits).unwrap(), mutated);
        }
    }
}
