//! Deterministic assembly of Dendrite evidence into a deliberation ensemble.
//!
//! Transport adapters parse MCP `structuredContent`; this module checks that
//! fold, surface and orthogonality evidence all describe the same cohort.

use std::collections::BTreeSet;

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FoldGroundingV1 {
    pub member: EnsembleMemberIdV1,
    pub authority_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceEvidenceV1 {
    pub member: EnsembleMemberIdV1,
    pub authority_digest: [u8; 32],
    pub surface_digest: [u8; 32],
    pub depth: SurfaceDepthV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemberIntentV1 {
    pub id: EnsembleMemberIdV1,
    pub stakeholder: StakeholderRefV1,
    pub lens: PersonaLensV1,
    pub concern_root_digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrthogonalityPacketV1 {
    pub members: Vec<EnsembleMemberIdV1>,
    pub query_digest: [u8; 32],
    pub result_digest: [u8; 32],
    pub all_orthogonal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroundedEnsembleV1 {
    pub members: Vec<EnsembleMemberV1>,
    pub orthogonality: OrthogonalityEvidenceV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroundingRefusalV1 {
    EmptyEnsemble,
    DuplicateMember,
    CohortMismatch,
    MissingFoldGrounding,
    MissingSurface,
    AuthorityMismatch,
    NonOrthogonal,
}

pub fn assemble(
    intents: &[MemberIntentV1],
    fold: &[FoldGroundingV1],
    surfaces: &[SurfaceEvidenceV1],
    orthogonality: OrthogonalityPacketV1,
) -> Result<GroundedEnsembleV1, GroundingRefusalV1> {
    if intents.is_empty() {
        return Err(GroundingRefusalV1::EmptyEnsemble);
    }
    let cohort = unique_ids(intents.iter().map(|value| value.id), intents.len())?;
    let fold_ids = unique_ids(fold.iter().map(|value| value.member), fold.len())?;
    let surface_ids = unique_ids(surfaces.iter().map(|value| value.member), surfaces.len())?;
    let orthogonal_ids = unique_ids(
        orthogonality.members.iter().copied(),
        orthogonality.members.len(),
    )?;
    if cohort != fold_ids || cohort != surface_ids || cohort != orthogonal_ids {
        return Err(GroundingRefusalV1::CohortMismatch);
    }
    if !orthogonality.all_orthogonal {
        return Err(GroundingRefusalV1::NonOrthogonal);
    }
    let mut members = Vec::with_capacity(intents.len());
    for intent in intents {
        let grounding = fold
            .iter()
            .find(|value| value.member == intent.id)
            .ok_or(GroundingRefusalV1::MissingFoldGrounding)?;
        let surface = surfaces
            .iter()
            .find(|value| value.member == intent.id)
            .ok_or(GroundingRefusalV1::MissingSurface)?;
        if grounding.authority_digest != surface.authority_digest {
            return Err(GroundingRefusalV1::AuthorityMismatch);
        }
        members.push(EnsembleMemberV1 {
            id: intent.id,
            stakeholder: intent.stakeholder,
            lens: intent.lens,
            concern_root_digest: intent.concern_root_digest,
            grounding: GroundingRefV1 {
                authority_digest: grounding.authority_digest,
                surface_digest: surface.surface_digest,
                depth: surface.depth,
            },
        });
    }
    Ok(GroundedEnsembleV1 {
        members,
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: orthogonality.query_digest,
            result_digest: orthogonality.result_digest,
            all_orthogonal: true,
        },
    })
}

fn unique_ids(
    ids: impl Iterator<Item = EnsembleMemberIdV1>,
    expected: usize,
) -> Result<BTreeSet<EnsembleMemberIdV1>, GroundingRefusalV1> {
    let ids = ids.collect::<BTreeSet<_>>();
    if ids.len() == expected {
        Ok(ids)
    } else {
        Err(GroundingRefusalV1::DuplicateMember)
    }
}
