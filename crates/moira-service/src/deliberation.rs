//! Native service mapping for the methodical-deliberation contract.

use crate::AccessV1;
use moira_core::deliberation::{self as domain, registry};
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};
use moira_protocol::deliberation as protocol;

pub struct DeliberationService<S> {
    store: S,
}

impl<S: DeliberationStore> DeliberationService<S> {
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    pub fn execute(&mut self, command: protocol::CommandV1) -> Result<protocol::ReplyV1, S::Error> {
        self.execute_authorized(
            AccessV1 {
                read: true,
                mutate: true,
            },
            command,
        )
    }

    pub fn execute_authorized(
        &mut self,
        access: AccessV1,
        command: protocol::CommandV1,
    ) -> Result<protocol::ReplyV1, S::Error> {
        let allowed = match command {
            protocol::CommandV1::Apply { .. } => access.mutate,
            protocol::CommandV1::Status | protocol::CommandV1::Deliberation { .. } => access.read,
        };
        if !allowed {
            return Ok(protocol::ReplyV1::Refused(
                protocol::RefusalV1::NotAuthorized,
            ));
        }
        match command {
            protocol::CommandV1::Apply {
                expected_revision,
                mutation,
            } => {
                match self.store.apply(
                    registry::RegistryRevisionV1(expected_revision),
                    mutation_domain(mutation),
                ) {
                    Ok(receipt) => Ok(protocol::ReplyV1::Accepted(protocol::OutcomeV1::Applied {
                        revision: receipt.revision.0,
                        deliberation: Box::new(checkpoint_view(receipt.deliberation)),
                        replayed: receipt.replayed,
                    })),
                    Err(DeliberationApplyErrorV1::Domain(value)) => {
                        Ok(protocol::ReplyV1::Refused(refusal(value)))
                    }
                    Err(DeliberationApplyErrorV1::Backend(error)) => Err(error),
                }
            }
            protocol::CommandV1::Status => Ok(protocol::ReplyV1::Accepted(
                protocol::OutcomeV1::Status(protocol::StatusV1 {
                    revision: self.store.revision().0,
                    needs_reopen: self.store.needs_reopen(),
                }),
            )),
            protocol::CommandV1::Deliberation { deliberation } => Ok(protocol::ReplyV1::Accepted(
                protocol::OutcomeV1::Deliberation(
                    self.store
                        .deliberation(domain::DeliberationIdV1(deliberation.0))
                        .map(checkpoint_view)
                        .map(Box::new),
                ),
            )),
        }
    }

    pub fn into_store(self) -> S {
        self.store
    }
}

fn mutation_domain(value: protocol::MutationV1) -> registry::RegistryCommandV1 {
    match value {
        protocol::MutationV1::Admit(plan) => registry::RegistryCommandV1::Admit(plan_domain(*plan)),
        protocol::MutationV1::SubmitReport {
            deliberation,
            report,
        } => registry::RegistryCommandV1::SubmitReport {
            deliberation: domain::DeliberationIdV1(deliberation.0),
            report: report_domain(report),
        },
        protocol::MutationV1::Resolve {
            deliberation,
            resolution,
        } => registry::RegistryCommandV1::Resolve {
            deliberation: domain::DeliberationIdV1(deliberation.0),
            resolution: resolution_domain(resolution),
        },
    }
}

fn plan_domain(value: protocol::DeliberationPlanV1) -> domain::DeliberationPlanV1 {
    domain::DeliberationPlanV1 {
        id: domain::DeliberationIdV1(value.id.0),
        question_digest: value.question_digest,
        method: domain::MethodRefV1 {
            machine_digest: value.method.machine_digest,
            state_digest: value.method.state_digest,
        },
        claims: value.claims.into_iter().map(claim_domain).collect(),
        criteria: value.criteria.into_iter().map(criterion_domain).collect(),
        candidates: value.candidates.into_iter().map(candidate_domain).collect(),
        members: value.members.into_iter().map(member_domain).collect(),
        orthogonality: domain::OrthogonalityEvidenceV1 {
            query_digest: value.orthogonality.query_digest,
            result_digest: value.orthogonality.result_digest,
            all_orthogonal: value.orthogonality.all_orthogonal,
        },
        fuel: value.fuel,
    }
}

fn claim_domain(value: protocol::ClaimKindV1) -> domain::ClaimKindV1 {
    match value {
        protocol::ClaimKindV1::Formal => domain::ClaimKindV1::Formal,
        protocol::ClaimKindV1::Empirical => domain::ClaimKindV1::Empirical,
        protocol::ClaimKindV1::Normative => domain::ClaimKindV1::Normative,
        protocol::ClaimKindV1::Historical => domain::ClaimKindV1::Historical,
        protocol::ClaimKindV1::Creative => domain::ClaimKindV1::Creative,
    }
}

fn criterion_domain(value: protocol::CriterionV1) -> domain::CriterionV1 {
    domain::CriterionV1 {
        id: domain::CriterionIdV1(value.id.0),
        direction: match value.direction {
            protocol::CriterionDirectionV1::Maximize => domain::CriterionDirectionV1::Maximize,
            protocol::CriterionDirectionV1::Minimize => domain::CriterionDirectionV1::Minimize,
        },
    }
}

fn candidate_domain(value: protocol::CandidateV1) -> domain::CandidateV1 {
    domain::CandidateV1 {
        id: domain::CandidateIdV1(value.id.0),
        scores: value.scores,
    }
}

fn member_domain(value: protocol::EnsembleMemberV1) -> domain::EnsembleMemberV1 {
    domain::EnsembleMemberV1 {
        id: domain::EnsembleMemberIdV1(value.id.0),
        lens: lens_domain(value.lens),
        concern_root_digest: value.concern_root_digest,
        grounding: domain::GroundingRefV1 {
            authority_digest: value.grounding.authority_digest,
            surface_digest: value.grounding.surface_digest,
            depth: match value.grounding.depth {
                protocol::SurfaceDepthV1::Ambient => domain::SurfaceDepthV1::Ambient,
                protocol::SurfaceDepthV1::Activated => domain::SurfaceDepthV1::Activated,
                protocol::SurfaceDepthV1::Reference => domain::SurfaceDepthV1::Reference,
            },
        },
    }
}

fn lens_domain(value: protocol::PersonaLensV1) -> domain::PersonaLensV1 {
    match value {
        protocol::PersonaLensV1::Historian => domain::PersonaLensV1::Historian,
        protocol::PersonaLensV1::IdeaStormer => domain::PersonaLensV1::IdeaStormer,
        protocol::PersonaLensV1::Theorist => domain::PersonaLensV1::Theorist,
        protocol::PersonaLensV1::Researcher => domain::PersonaLensV1::Researcher,
        protocol::PersonaLensV1::RigorReviewer => domain::PersonaLensV1::RigorReviewer,
        protocol::PersonaLensV1::ConvergenceFacilitator => {
            domain::PersonaLensV1::ConvergenceFacilitator
        }
    }
}

fn report_domain(value: protocol::MemberReportV1) -> domain::MemberReportV1 {
    domain::MemberReportV1 {
        id: domain::ReportIdV1(value.id.0),
        member: domain::EnsembleMemberIdV1(value.member.0),
        output_digest: value.output_digest,
        evidence_digests: value.evidence_digests,
        dissent_digests: value.dissent_digests,
    }
}

fn resolution_domain(value: protocol::ResolutionV1) -> domain::ResolutionV1 {
    match value {
        protocol::ResolutionV1::ProvenBool {
            value,
            validator,
            evidence_digest,
        } => domain::ResolutionV1::ProvenBool {
            value,
            validator,
            evidence_digest,
        },
        protocol::ResolutionV1::AuthorizedChoice {
            frontier,
            selected,
            authority,
        } => domain::ResolutionV1::AuthorizedChoice {
            frontier: frontier
                .into_iter()
                .map(|id| domain::CandidateIdV1(id.0))
                .collect(),
            selected: domain::CandidateIdV1(selected.0),
            authority: domain::AuthorityRefV1 {
                decision: domain::AuthorityDecisionIdV1(authority.decision.0),
                subject: domain::AuthoritySubjectIdV1(authority.subject.0),
                epoch: authority.epoch,
            },
        },
        protocol::ResolutionV1::Maybe(value) => domain::ResolutionV1::Maybe(maybe_domain(value)),
        protocol::ResolutionV1::Mu(value) => domain::ResolutionV1::Mu(mu_domain(value)),
    }
}

fn maybe_domain(value: protocol::MaybeValueV1) -> domain::MaybeValueV1 {
    match value {
        protocol::MaybeValueV1::TrueOnly => domain::MaybeValueV1::TrueOnly,
        protocol::MaybeValueV1::FalseOnly => domain::MaybeValueV1::FalseOnly,
        protocol::MaybeValueV1::Both => domain::MaybeValueV1::Both,
        protocol::MaybeValueV1::Neither => domain::MaybeValueV1::Neither,
    }
}

fn mu_domain(value: protocol::MuReasonV1) -> domain::MuReasonV1 {
    match value {
        protocol::MuReasonV1::FuelExhausted => domain::MuReasonV1::FuelExhausted,
        protocol::MuReasonV1::MissingEvidence => domain::MuReasonV1::MissingEvidence,
        protocol::MuReasonV1::ContradictoryEvidence => domain::MuReasonV1::ContradictoryEvidence,
        protocol::MuReasonV1::WrongQuestion => domain::MuReasonV1::WrongQuestion,
        protocol::MuReasonV1::WrongEnsemble => domain::MuReasonV1::WrongEnsemble,
        protocol::MuReasonV1::NoAuthorizedSelector => domain::MuReasonV1::NoAuthorizedSelector,
    }
}

fn checkpoint_view(value: domain::DeliberationCheckpointV1) -> protocol::DeliberationViewV1 {
    let phase = if value.resolution.is_some() {
        protocol::DeliberationPhaseV1::Resolved
    } else if value.reports.len() == value.plan.members.len() {
        protocol::DeliberationPhaseV1::ReadyToResolve
    } else {
        protocol::DeliberationPhaseV1::Collecting
    };
    protocol::DeliberationViewV1 {
        version: value.version,
        plan: plan_view(value.plan),
        limits: protocol::DeliberationLimitsV1 {
            max_claims: value.limits.max_claims,
            max_criteria: value.limits.max_criteria,
            max_candidates: value.limits.max_candidates,
            max_members: value.limits.max_members,
            max_evidence_per_report: value.limits.max_evidence_per_report,
            max_dissent_per_report: value.limits.max_dissent_per_report,
            max_fuel: value.limits.max_fuel,
        },
        reports: value.reports.into_iter().map(report_view).collect(),
        resolution: value.resolution.map(resolution_view),
        remaining_fuel: value.remaining_fuel,
        phase,
    }
}

fn plan_view(value: domain::DeliberationPlanV1) -> protocol::DeliberationPlanV1 {
    protocol::DeliberationPlanV1 {
        id: protocol::DeliberationIdV1(value.id.0),
        question_digest: value.question_digest,
        method: protocol::MethodRefV1 {
            machine_digest: value.method.machine_digest,
            state_digest: value.method.state_digest,
        },
        claims: value
            .claims
            .into_iter()
            .map(|v| match v {
                domain::ClaimKindV1::Formal => protocol::ClaimKindV1::Formal,
                domain::ClaimKindV1::Empirical => protocol::ClaimKindV1::Empirical,
                domain::ClaimKindV1::Normative => protocol::ClaimKindV1::Normative,
                domain::ClaimKindV1::Historical => protocol::ClaimKindV1::Historical,
                domain::ClaimKindV1::Creative => protocol::ClaimKindV1::Creative,
            })
            .collect(),
        criteria: value
            .criteria
            .into_iter()
            .map(|v| protocol::CriterionV1 {
                id: protocol::CriterionIdV1(v.id.0),
                direction: match v.direction {
                    domain::CriterionDirectionV1::Maximize => {
                        protocol::CriterionDirectionV1::Maximize
                    }
                    domain::CriterionDirectionV1::Minimize => {
                        protocol::CriterionDirectionV1::Minimize
                    }
                },
            })
            .collect(),
        candidates: value
            .candidates
            .into_iter()
            .map(|v| protocol::CandidateV1 {
                id: protocol::CandidateIdV1(v.id.0),
                scores: v.scores,
            })
            .collect(),
        members: value.members.into_iter().map(member_view).collect(),
        orthogonality: protocol::OrthogonalityEvidenceV1 {
            query_digest: value.orthogonality.query_digest,
            result_digest: value.orthogonality.result_digest,
            all_orthogonal: value.orthogonality.all_orthogonal,
        },
        fuel: value.fuel,
    }
}

fn member_view(value: domain::EnsembleMemberV1) -> protocol::EnsembleMemberV1 {
    protocol::EnsembleMemberV1 {
        id: protocol::EnsembleMemberIdV1(value.id.0),
        lens: match value.lens {
            domain::PersonaLensV1::Historian => protocol::PersonaLensV1::Historian,
            domain::PersonaLensV1::IdeaStormer => protocol::PersonaLensV1::IdeaStormer,
            domain::PersonaLensV1::Theorist => protocol::PersonaLensV1::Theorist,
            domain::PersonaLensV1::Researcher => protocol::PersonaLensV1::Researcher,
            domain::PersonaLensV1::RigorReviewer => protocol::PersonaLensV1::RigorReviewer,
            domain::PersonaLensV1::ConvergenceFacilitator => {
                protocol::PersonaLensV1::ConvergenceFacilitator
            }
        },
        concern_root_digest: value.concern_root_digest,
        grounding: protocol::GroundingRefV1 {
            authority_digest: value.grounding.authority_digest,
            surface_digest: value.grounding.surface_digest,
            depth: match value.grounding.depth {
                domain::SurfaceDepthV1::Ambient => protocol::SurfaceDepthV1::Ambient,
                domain::SurfaceDepthV1::Activated => protocol::SurfaceDepthV1::Activated,
                domain::SurfaceDepthV1::Reference => protocol::SurfaceDepthV1::Reference,
            },
        },
    }
}

fn report_view(value: domain::MemberReportV1) -> protocol::MemberReportV1 {
    protocol::MemberReportV1 {
        id: protocol::ReportIdV1(value.id.0),
        member: protocol::EnsembleMemberIdV1(value.member.0),
        output_digest: value.output_digest,
        evidence_digests: value.evidence_digests,
        dissent_digests: value.dissent_digests,
    }
}

fn resolution_view(value: domain::ResolutionV1) -> protocol::ResolutionV1 {
    match value {
        domain::ResolutionV1::ProvenBool {
            value,
            validator,
            evidence_digest,
        } => protocol::ResolutionV1::ProvenBool {
            value,
            validator,
            evidence_digest,
        },
        domain::ResolutionV1::AuthorizedChoice {
            frontier,
            selected,
            authority,
        } => protocol::ResolutionV1::AuthorizedChoice {
            frontier: frontier
                .into_iter()
                .map(|id| protocol::CandidateIdV1(id.0))
                .collect(),
            selected: protocol::CandidateIdV1(selected.0),
            authority: protocol::AuthorityRefV1 {
                decision: protocol::AuthorityDecisionIdV1(authority.decision.0),
                subject: protocol::AuthoritySubjectIdV1(authority.subject.0),
                epoch: authority.epoch,
            },
        },
        domain::ResolutionV1::Maybe(v) => protocol::ResolutionV1::Maybe(match v {
            domain::MaybeValueV1::TrueOnly => protocol::MaybeValueV1::TrueOnly,
            domain::MaybeValueV1::FalseOnly => protocol::MaybeValueV1::FalseOnly,
            domain::MaybeValueV1::Both => protocol::MaybeValueV1::Both,
            domain::MaybeValueV1::Neither => protocol::MaybeValueV1::Neither,
        }),
        domain::ResolutionV1::Mu(v) => protocol::ResolutionV1::Mu(match v {
            domain::MuReasonV1::FuelExhausted => protocol::MuReasonV1::FuelExhausted,
            domain::MuReasonV1::MissingEvidence => protocol::MuReasonV1::MissingEvidence,
            domain::MuReasonV1::ContradictoryEvidence => {
                protocol::MuReasonV1::ContradictoryEvidence
            }
            domain::MuReasonV1::WrongQuestion => protocol::MuReasonV1::WrongQuestion,
            domain::MuReasonV1::WrongEnsemble => protocol::MuReasonV1::WrongEnsemble,
            domain::MuReasonV1::NoAuthorizedSelector => protocol::MuReasonV1::NoAuthorizedSelector,
        }),
    }
}

fn refusal(value: registry::RegistryRefusalV1) -> protocol::RefusalV1 {
    match value {
        registry::RegistryRefusalV1::InvalidLimits => protocol::RefusalV1::InvalidLimits,
        registry::RegistryRefusalV1::UnsupportedVersion => protocol::RefusalV1::UnsupportedVersion,
        registry::RegistryRefusalV1::RevisionConflict { expected, actual } => {
            protocol::RefusalV1::RevisionConflict {
                expected: expected.0,
                actual: actual.0,
            }
        }
        registry::RegistryRefusalV1::RevisionExhausted => protocol::RefusalV1::RevisionExhausted,
        registry::RegistryRefusalV1::Capacity => protocol::RefusalV1::Capacity,
        registry::RegistryRefusalV1::UnknownDeliberation => {
            protocol::RefusalV1::UnknownDeliberation
        }
        registry::RegistryRefusalV1::InputConflict => protocol::RefusalV1::InputConflict,
        registry::RegistryRefusalV1::DuplicateDeliberation => {
            protocol::RefusalV1::DuplicateDeliberation
        }
        registry::RegistryRefusalV1::CheckpointMismatch => protocol::RefusalV1::CheckpointMismatch,
        registry::RegistryRefusalV1::Deliberation(v) => {
            protocol::RefusalV1::Deliberation(deliberation_refusal(v))
        }
    }
}

fn deliberation_refusal(value: domain::DeliberationRefusalV1) -> protocol::DeliberationRefusalV1 {
    use domain::DeliberationRefusalV1 as D;
    use protocol::DeliberationRefusalV1 as P;
    match value {
        D::UnsupportedVersion => P::UnsupportedVersion,
        D::InvalidLimits => P::InvalidLimits,
        D::EmptyClaims => P::EmptyClaims,
        D::ClaimLimit => P::ClaimLimit,
        D::CriterionLimit => P::CriterionLimit,
        D::CandidateLimit => P::CandidateLimit,
        D::EmptyEnsemble => P::EmptyEnsemble,
        D::MemberLimit => P::MemberLimit,
        D::DuplicateMember => P::DuplicateMember,
        D::NonOrthogonalEnsemble => P::NonOrthogonalEnsemble,
        D::FuelInvalid => P::FuelInvalid,
        D::Frontier(v) => P::Frontier(match v {
            domain::FrontierRefusalV1::NoCriteria => protocol::FrontierRefusalV1::NoCriteria,
            domain::FrontierRefusalV1::NoCandidates => protocol::FrontierRefusalV1::NoCandidates,
            domain::FrontierRefusalV1::DuplicateCriterion => {
                protocol::FrontierRefusalV1::DuplicateCriterion
            }
            domain::FrontierRefusalV1::DuplicateCandidate => {
                protocol::FrontierRefusalV1::DuplicateCandidate
            }
            domain::FrontierRefusalV1::ScoreDimensionMismatch => {
                protocol::FrontierRefusalV1::ScoreDimensionMismatch
            }
        }),
        D::UnknownMember => P::UnknownMember,
        D::ReportLimit => P::ReportLimit,
        D::ReportConflict => P::ReportConflict,
        D::AlreadyResolved => P::AlreadyResolved,
        D::ReportsIncomplete => P::ReportsIncomplete,
        D::ResolutionConflict => P::ResolutionConflict,
        D::FrontierMismatch => P::FrontierMismatch,
        D::SelectionOutsideFrontier => P::SelectionOutsideFrontier,
        D::FuelRemaining => P::FuelRemaining,
        D::CheckpointMismatch => P::CheckpointMismatch,
    }
}
