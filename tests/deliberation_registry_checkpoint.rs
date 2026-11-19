use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, DeliberationPlanV1, EnsembleAdmissionV1, EnsembleMemberIdV1,
    EnsembleMemberV1, GroundingRefV1, MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1,
    StakeholderRefV1, SurfaceDepthV1,
    registry::{
        DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryRevisionV1,
        persistence::{CodecErrorV1, CodecLimitsV1, decode, encode},
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
        claims: vec![ClaimKindV1::Formal],
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
            lens: PersonaLensV1::Theorist,
            concern_root_digest: [2; 32],
            grounding: GroundingRefV1 {
                authority_digest: [3; 32],
                surface_digest: [4; 32],
                depth: SurfaceDepthV1::Reference,
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

fn checkpoint() -> moira_core::deliberation::registry::RegistryCheckpointV1 {
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
    registry.checkpoint()
}

#[test]
fn registry_bytes_roundtrip_in_canonical_deliberation_order() {
    let checkpoint = checkpoint();
    assert_eq!(
        checkpoint.deliberations[0].plan.id,
        DeliberationIdV1([1; 16])
    );
    let limits = CodecLimitsV1::default();
    let bytes = encode(&checkpoint, limits).unwrap();
    let decoded = decode(&bytes, limits).unwrap();
    assert_eq!(decoded, checkpoint);
    assert_eq!(encode(&decoded, limits).unwrap(), bytes);
}

#[test]
fn prior_registry_codec_identity_is_refused_instead_of_reinterpreted() {
    let limits = CodecLimitsV1::default();
    let mut bytes = encode(&checkpoint(), limits).unwrap();
    bytes[..8].copy_from_slice(b"MOIRADR1");
    bytes[8] = 1;
    assert_eq!(decode(&bytes, limits), Err(CodecErrorV1::Invalid));
}

#[test]
fn version_two_registry_and_version_three_child_migrate_without_gate_state() {
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan(1))),
        )
        .unwrap();
    let expected = registry.checkpoint();
    let limits = CodecLimitsV1::default();
    let current = encode(&expected, limits).unwrap();
    let child_start = current
        .windows(8)
        .position(|window| window == b"MOIRADL6")
        .unwrap();
    let length_start = child_start - 8;
    let child_length = usize::try_from(u64::from_be_bytes(
        current[length_start..child_start].try_into().unwrap(),
    ))
    .unwrap();
    assert_eq!(child_start + child_length + 8, current.len());

    let mut child = current[child_start..child_start + child_length].to_vec();
    assert_eq!(&child[child.len() - 10..child.len() - 8], &[0, 0]);
    child[..8].copy_from_slice(b"MOIRADL3");
    child[8] = 3;
    // This fixture has no provenance payload, so remove V6's absent marker.
    child.drain(177..178);
    let gate_options = child.len() - 10;
    child.drain(gate_options..gate_options + 2);

    let mut legacy = current[..length_start].to_vec();
    legacy[..8].copy_from_slice(b"MOIRADR2");
    legacy[8] = 2;
    legacy.extend_from_slice(&u64::try_from(child.len()).unwrap().to_be_bytes());
    legacy.extend_from_slice(&child);

    let migrated = decode(&legacy, limits).unwrap();
    assert_eq!(migrated.version, 5);
    assert!(migrated.reviews.is_empty());
    assert_eq!(migrated.deliberations[0].version, 5);
    assert_eq!(migrated.deliberations[0].human_gate_request, None);
    assert_eq!(migrated.deliberations[0].human_gate_response, None);
    assert_eq!(migrated, expected);
    assert_ne!(encode(&migrated, limits).unwrap(), legacy);
}

#[test]
fn registry_codec_refuses_truncation_trailing_and_narrow_host_bounds() {
    let limits = CodecLimitsV1::default();
    let bytes = encode(&checkpoint(), limits).unwrap();
    for end in 0..bytes.len() {
        assert!(decode(&bytes[..end], limits).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(decode(&trailing, limits), Err(CodecErrorV1::Invalid));

    let mut narrow = limits;
    narrow.registry.max_deliberations = 1;
    assert_eq!(decode(&bytes, narrow), Err(CodecErrorV1::Limit));
}

#[test]
fn accepted_registry_mutations_are_canonical() {
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
