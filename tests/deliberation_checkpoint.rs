use moira_core::deliberation::{
    AuthorityDecisionIdV1, AuthorityRefV1, AuthoritySubjectIdV1, CandidateIdV1, CandidateV1,
    ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1, DeliberationIdV1,
    DeliberationLimitsV1, DeliberationPlanV1, DeliberationV1, EnsembleAdmissionV1,
    EnsembleMemberIdV1, EnsembleMemberV1, GroundingRefV1, GroundingSourceProvenanceV1,
    HumanGateAuthorityRefV1, HumanGateDispositionV1, HumanGateMaterialRefV1, HumanGateRequestIdV1,
    HumanGateRequestV1, HumanGateResponseIdV1, HumanGateResponseV1, MemberReportV1, MethodRefV1,
    OrthogonalityEvidenceV1, PersonaLensV1, ReportIdV1, ResolutionV1, StakeholderRefV1,
    SurfaceDepthV1,
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
        grounding_provenance: Some(GroundingSourceProvenanceV1 {
            fold_content_digest: [4; 32],
            orthogonality_content_digest: [5; 32],
        }),
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
        admission: EnsembleAdmissionV1::affirmed([10; 32], [11; 32], [12; 32], [13; 32], [14; 16]),
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
        stakeholder: StakeholderRefV1::technical_sme([seed; 16], [seed; 32]),
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

fn material(seed: u8, revision: u64) -> HumanGateMaterialRefV1 {
    HumanGateMaterialRefV1 {
        owner_route: [seed; 16],
        artifact_route: [seed.wrapping_add(1); 16],
        expected_revision: revision,
        expected_sha256: [seed.wrapping_add(2); 32],
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
    deliberation
        .request_human_gate(HumanGateRequestV1 {
            id: HumanGateRequestIdV1([30; 16]),
            reviewer: [31; 16],
            finding_digest: [32; 32],
            decision_digest: [33; 32],
            material_revision_digest: [34; 32],
            policy_digest: [35; 32],
            decision_packet: material(42, 7),
            requested_at: 36,
        })
        .unwrap();
    deliberation
        .respond_human_gate(HumanGateResponseV1 {
            id: HumanGateResponseIdV1([37; 16]),
            request: HumanGateRequestIdV1([30; 16]),
            disposition: HumanGateDispositionV1::Deferred,
            rationale_digest: [38; 32],
            rationale: material(45, 8),
            responded_at: 39,
            authority: HumanGateAuthorityRefV1 {
                decision: [40; 16],
                subject: [31; 16],
                epoch: 41,
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
    bytes[..8].copy_from_slice(b"MOIRADL4");
    bytes[8] = 4;
    assert_eq!(decode(&bytes, limits), Err(CodecErrorV1::Invalid));
}

#[test]
fn version_three_bytes_migrate_without_fabricating_a_human_gate() {
    let mut checkpoint = checkpoint();
    checkpoint.human_gate_request = None;
    checkpoint.human_gate_response = None;
    let limits = CodecLimitsV1::default();
    let current = encode(&checkpoint, limits).unwrap();
    assert_eq!(&current[current.len() - 10..current.len() - 8], &[0, 0]);

    let mut legacy = current;
    legacy[..8].copy_from_slice(b"MOIRADL3");
    legacy[8] = 3;
    // V3 ended after the method identity. Remove V6's provenance marker and
    // two digests instead of allowing them to be reinterpreted as claim bytes.
    legacy.drain(177..242);
    legacy.drain(legacy.len() - 10..legacy.len() - 8);

    let migrated = decode(&legacy, limits).unwrap();
    assert_eq!(migrated.version, 5);
    assert_eq!(migrated.human_gate_request, None);
    assert_eq!(migrated.human_gate_response, None);
    assert_ne!(encode(&migrated, limits).unwrap(), legacy);
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
