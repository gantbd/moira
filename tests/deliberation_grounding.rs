use moira_core::deliberation::{
    EnsembleMemberIdV1, PersonaLensV1, StakeholderRefV1, SurfaceDepthV1,
    grounding::{self, *},
};

fn intent(id: u8) -> MemberIntentV1 {
    MemberIntentV1 {
        id: EnsembleMemberIdV1([id; 16]),
        stakeholder: StakeholderRefV1::technical_sme([id; 16], [id; 32]),
        lens: PersonaLensV1::Researcher,
        concern_root_digest: [id; 32],
    }
}

fn fold(id: u8) -> FoldGroundingV1 {
    FoldGroundingV1 {
        member: EnsembleMemberIdV1([id; 16]),
        authority_digest: [id + 10; 32],
    }
}

fn surface(id: u8) -> SurfaceEvidenceV1 {
    SurfaceEvidenceV1 {
        member: EnsembleMemberIdV1([id; 16]),
        authority_digest: [id + 10; 32],
        surface_digest: [id + 20; 32],
        depth: SurfaceDepthV1::Activated,
    }
}

fn orthogonality(ids: &[u8]) -> OrthogonalityPacketV1 {
    OrthogonalityPacketV1 {
        members: ids.iter().map(|id| EnsembleMemberIdV1([*id; 16])).collect(),
        query_digest: [40; 32],
        result_digest: [41; 32],
        all_orthogonal: true,
    }
}

#[test]
fn exact_fold_surface_and_orthogonality_cohort_assembles() {
    let result = grounding::assemble(
        &[intent(1), intent(2)],
        &[fold(1), fold(2)],
        &[surface(1), surface(2)],
        orthogonality(&[2, 1]),
    )
    .unwrap();
    assert_eq!(result.members[0].grounding.authority_digest, [11; 32]);
    assert_eq!(result.members[1].grounding.surface_digest, [22; 32]);
    assert!(result.orthogonality.all_orthogonal);
}

#[test]
fn singleton_cohort_accepts_vacuous_pairwise_orthogonality() {
    let result =
        grounding::assemble(&[intent(1)], &[fold(1)], &[surface(1)], orthogonality(&[1])).unwrap();
    assert_eq!(result.members.len(), 1);
    assert!(result.orthogonality.all_orthogonal);
}

#[test]
fn evidence_for_another_cohort_and_authority_substitution_refuse() {
    assert_eq!(
        grounding::assemble(
            &[intent(1), intent(2)],
            &[fold(1), fold(2)],
            &[surface(1), surface(2)],
            orthogonality(&[1, 3]),
        ),
        Err(GroundingRefusalV1::CohortMismatch)
    );
    let mut replaced = surface(2);
    replaced.authority_digest = [99; 32];
    assert_eq!(
        grounding::assemble(
            &[intent(1), intent(2)],
            &[fold(1), fold(2)],
            &[surface(1), replaced],
            orthogonality(&[1, 2]),
        ),
        Err(GroundingRefusalV1::AuthorityMismatch)
    );
}

#[test]
fn correlated_or_duplicate_evidence_never_assembles() {
    let mut verdict = orthogonality(&[1, 2]);
    verdict.all_orthogonal = false;
    assert_eq!(
        grounding::assemble(
            &[intent(1), intent(2)],
            &[fold(1), fold(2)],
            &[surface(1), surface(2)],
            verdict,
        ),
        Err(GroundingRefusalV1::NonOrthogonal)
    );
    assert_eq!(
        grounding::assemble(
            &[intent(1), intent(1)],
            &[fold(1), fold(1)],
            &[surface(1), surface(1)],
            orthogonality(&[1, 1]),
        ),
        Err(GroundingRefusalV1::DuplicateMember)
    );
}
