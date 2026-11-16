//! Authority-preserving terminal resolution ports.
//!
//! Validators and selectors receive an exact digest of the unresolved retained
//! checkpoint. Their response must echo that subject. Model output and Pareto
//! membership cannot manufacture either witness.

use moira_core::deliberation::{
    DeliberationCheckpointV1, DeliberationIdV1, FrontierRefusalV1, ResolutionV1, pareto_frontier,
    persistence::{CodecLimitsV1, encode},
    registry::{RegistryCommandV1, RegistryReceiptV1},
};
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};
use sha2::{Digest, Sha256};

pub use moira_host::{
    BooleanValidatorV1, ChoiceAuthorityV1, ChoiceDecisionV1, ChoiceRequestV1, ProofRequestV1,
    ProofWitnessV1, ResolutionSubjectV1,
};

#[derive(Debug, Eq, PartialEq)]
pub enum ProofResolutionErrorV1<S, V> {
    Missing,
    AlreadyResolved,
    ReportsIncomplete,
    SubjectEncoding,
    Validator(V),
    InvalidWitness,
    Store(DeliberationApplyErrorV1<S>),
}

#[derive(Debug, Eq, PartialEq)]
pub enum ChoiceResolutionErrorV1<S, A> {
    Missing,
    AlreadyResolved,
    ReportsIncomplete,
    SubjectEncoding,
    Frontier(FrontierRefusalV1),
    Authority(A),
    InvalidDecision,
    Store(DeliberationApplyErrorV1<S>),
}

pub fn resolve_proven_bool<S, V>(
    store: &mut S,
    deliberation: DeliberationIdV1,
    validator: &mut V,
) -> Result<RegistryReceiptV1, ProofResolutionErrorV1<S::Error, V::Error>>
where
    S: DeliberationStore,
    V: BooleanValidatorV1,
{
    let checkpoint = ready(store, deliberation).map_err(|error| match error {
        ReadinessErrorV1::Missing => ProofResolutionErrorV1::Missing,
        ReadinessErrorV1::AlreadyResolved => ProofResolutionErrorV1::AlreadyResolved,
        ReadinessErrorV1::ReportsIncomplete => ProofResolutionErrorV1::ReportsIncomplete,
    })?;
    let subject = subject(&checkpoint).map_err(|()| ProofResolutionErrorV1::SubjectEncoding)?;
    let witness = validator
        .validate(ProofRequestV1 { subject })
        .map_err(ProofResolutionErrorV1::Validator)?;
    if witness.subject != subject {
        return Err(ProofResolutionErrorV1::InvalidWitness);
    }
    store
        .apply(
            store.revision(),
            RegistryCommandV1::Resolve {
                deliberation,
                resolution: ResolutionV1::ProvenBool {
                    value: witness.value,
                    validator: witness.validator,
                    evidence_digest: witness.evidence_digest,
                },
            },
        )
        .map_err(ProofResolutionErrorV1::Store)
}

pub fn resolve_authorized_choice<S, A>(
    store: &mut S,
    deliberation: DeliberationIdV1,
    authority: &mut A,
) -> Result<RegistryReceiptV1, ChoiceResolutionErrorV1<S::Error, A::Error>>
where
    S: DeliberationStore,
    A: ChoiceAuthorityV1,
{
    let checkpoint = ready(store, deliberation).map_err(|error| match error {
        ReadinessErrorV1::Missing => ChoiceResolutionErrorV1::Missing,
        ReadinessErrorV1::AlreadyResolved => ChoiceResolutionErrorV1::AlreadyResolved,
        ReadinessErrorV1::ReportsIncomplete => ChoiceResolutionErrorV1::ReportsIncomplete,
    })?;
    let subject = subject(&checkpoint).map_err(|()| ChoiceResolutionErrorV1::SubjectEncoding)?;
    let frontier = pareto_frontier(&checkpoint.plan.criteria, &checkpoint.plan.candidates)
        .map_err(ChoiceResolutionErrorV1::Frontier)?;
    let decision = authority
        .choose(ChoiceRequestV1 {
            subject,
            frontier: frontier.clone(),
        })
        .map_err(ChoiceResolutionErrorV1::Authority)?;
    if decision.subject != subject || !frontier.contains(&decision.selected) {
        return Err(ChoiceResolutionErrorV1::InvalidDecision);
    }
    store
        .apply(
            store.revision(),
            RegistryCommandV1::Resolve {
                deliberation,
                resolution: ResolutionV1::AuthorizedChoice {
                    frontier,
                    selected: decision.selected,
                    authority: decision.authority,
                },
            },
        )
        .map_err(ChoiceResolutionErrorV1::Store)
}

fn subject(checkpoint: &DeliberationCheckpointV1) -> Result<ResolutionSubjectV1, ()> {
    let bytes = encode(
        checkpoint,
        CodecLimitsV1 {
            max_bytes: 8 * 1_024 * 1_024,
            domain: checkpoint.limits,
        },
    )
    .map_err(|_| ())?;
    let mut hash = Sha256::new();
    hash.update(b"MOIRAWIT1");
    hash.update(bytes);
    Ok(ResolutionSubjectV1 {
        deliberation: checkpoint.plan.id,
        question_digest: checkpoint.plan.question_digest,
        checkpoint_digest: hash.finalize().into(),
    })
}

#[derive(Clone, Copy)]
enum ReadinessErrorV1 {
    Missing,
    AlreadyResolved,
    ReportsIncomplete,
}

fn ready<S: DeliberationStore>(
    store: &S,
    deliberation: DeliberationIdV1,
) -> Result<DeliberationCheckpointV1, ReadinessErrorV1> {
    let checkpoint = store
        .deliberation(deliberation)
        .ok_or(ReadinessErrorV1::Missing)?;
    if checkpoint.resolution.is_some() {
        return Err(ReadinessErrorV1::AlreadyResolved);
    }
    if checkpoint.reports.len() != checkpoint.plan.members.len() {
        return Err(ReadinessErrorV1::ReportsIncomplete);
    }
    Ok(checkpoint)
}
