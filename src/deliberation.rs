//! Pure deliberation values shared by future Moira workflow profiles.
//!
//! This module performs no model invocation and grants no authority. Pareto
//! membership is not proof and does not select a winner.

use std::collections::BTreeSet;

pub mod grounding;
pub mod persistence;
pub mod registry;

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub [u8; 16]);
    };
}

id!(CandidateIdV1);
id!(CriterionIdV1);
id!(AuthorityDecisionIdV1);
id!(AuthoritySubjectIdV1);
id!(DeliberationIdV1);
id!(EnsembleMemberIdV1);
id!(ReportIdV1);
id!(HumanGateRequestIdV1);
id!(HumanGateResponseIdV1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimKindV1 {
    Formal,
    Empirical,
    Normative,
    Historical,
    Creative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonaLensV1 {
    Historian,
    IdeaStormer,
    Theorist,
    Researcher,
    RigorReviewer,
    ConvergenceFacilitator,
}

/// How this member is entitled to speak in the cohort. This is deliberately
/// separate from `PersonaLensV1`: a way of examining a question is not
/// stakeholder identity, lived experience, mandate, or authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StakeholderRepresentationV1 {
    ActualStakeholder,
    AuthorizedRepresentative,
    AgentModeledPerspective,
    TechnicalSme,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StakeholderRefV1 {
    pub principal: [u8; 16],
    pub constituency_digest: [u8; 32],
    pub representation: StakeholderRepresentationV1,
    pub accountable_principal: Option<[u8; 16]>,
    pub mandate_digest: Option<[u8; 32]>,
    pub conflict_digest: Option<[u8; 32]>,
}

impl StakeholderRefV1 {
    pub fn technical_sme(principal: [u8; 16], domain_digest: [u8; 32]) -> Self {
        Self {
            principal,
            constituency_digest: domain_digest,
            representation: StakeholderRepresentationV1::TechnicalSme,
            accountable_principal: None,
            mandate_digest: None,
            conflict_digest: None,
        }
    }

    pub fn agent_modeled(principal: [u8; 16], constituency_digest: [u8; 32]) -> Self {
        Self {
            principal,
            constituency_digest,
            representation: StakeholderRepresentationV1::AgentModeledPerspective,
            accountable_principal: None,
            mandate_digest: None,
            conflict_digest: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnsembleFitnessV1 {
    Yes,
    No,
    Contested,
}

/// Retained answer to the admission question “Do we have the right ensemble?”
/// Digests name exact owner artifacts; they are evidence, not proof that the
/// cohort is representative or equitable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnsembleAdmissionV1 {
    pub affected_population_digest: [u8; 32],
    pub stakeholder_map_digest: [u8; 32],
    pub coverage_manifest_digest: [u8; 32],
    pub equity_assessment_digest: [u8; 32],
    pub assessor_principal: [u8; 16],
    pub fitness: EnsembleFitnessV1,
}

impl EnsembleAdmissionV1 {
    pub fn affirmed(
        affected_population_digest: [u8; 32],
        stakeholder_map_digest: [u8; 32],
        coverage_manifest_digest: [u8; 32],
        equity_assessment_digest: [u8; 32],
        assessor_principal: [u8; 16],
    ) -> Self {
        Self {
            affected_population_digest,
            stakeholder_map_digest,
            coverage_manifest_digest,
            equity_assessment_digest,
            assessor_principal,
            fitness: EnsembleFitnessV1::Yes,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceDepthV1 {
    Ambient,
    Activated,
    Reference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroundingRefV1 {
    pub authority_digest: [u8; 32],
    pub surface_digest: [u8; 32],
    pub depth: SurfaceDepthV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MethodRefV1 {
    pub machine_digest: [u8; 32],
    pub state_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnsembleMemberV1 {
    pub id: EnsembleMemberIdV1,
    pub stakeholder: StakeholderRefV1,
    pub lens: PersonaLensV1,
    pub concern_root_digest: [u8; 32],
    pub grounding: GroundingRefV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrthogonalityEvidenceV1 {
    pub query_digest: [u8; 32],
    pub result_digest: [u8; 32],
    pub all_orthogonal: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CriterionDirectionV1 {
    Maximize,
    Minimize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CriterionV1 {
    pub id: CriterionIdV1,
    pub direction: CriterionDirectionV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateV1 {
    pub id: CandidateIdV1,
    /// One score per criterion, in the criterion order supplied to the fold.
    pub scores: Vec<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaybeValueV1 {
    TrueOnly,
    FalseOnly,
    Both,
    Neither,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MuReasonV1 {
    FuelExhausted,
    MissingEvidence,
    ContradictoryEvidence,
    WrongQuestion,
    WrongEnsemble,
    NoAuthorizedSelector,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolutionV1 {
    ProvenBool {
        value: bool,
        validator: [u8; 16],
        evidence_digest: [u8; 32],
    },
    AuthorizedChoice {
        frontier: Vec<CandidateIdV1>,
        selected: CandidateIdV1,
        authority: AuthorityRefV1,
    },
    Maybe(MaybeValueV1),
    Mu(MuReasonV1),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorityRefV1 {
    pub decision: AuthorityDecisionIdV1,
    pub subject: AuthoritySubjectIdV1,
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrontierRefusalV1 {
    NoCriteria,
    NoCandidates,
    DuplicateCriterion,
    DuplicateCandidate,
    ScoreDimensionMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliberationLimitsV1 {
    pub max_claims: usize,
    pub max_criteria: usize,
    pub max_candidates: usize,
    pub max_members: usize,
    pub max_evidence_per_report: usize,
    pub max_dissent_per_report: usize,
    pub max_fuel: u32,
}

impl Default for DeliberationLimitsV1 {
    fn default() -> Self {
        Self {
            max_claims: 64,
            max_criteria: 32,
            max_candidates: 64,
            max_members: 16,
            max_evidence_per_report: 64,
            max_dissent_per_report: 32,
            max_fuel: 1_024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliberationPlanV1 {
    pub id: DeliberationIdV1,
    /// Exact identity of the UTF-8 question framed by the host.
    pub question_digest: [u8; 32],
    pub method: MethodRefV1,
    pub claims: Vec<ClaimKindV1>,
    pub criteria: Vec<CriterionV1>,
    pub candidates: Vec<CandidateV1>,
    pub admission: EnsembleAdmissionV1,
    pub members: Vec<EnsembleMemberV1>,
    pub orthogonality: OrthogonalityEvidenceV1,
    pub fuel: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberReportV1 {
    pub id: ReportIdV1,
    pub member: EnsembleMemberIdV1,
    pub output_digest: [u8; 32],
    pub evidence_digests: Vec<[u8; 32]>,
    pub dissent_digests: Vec<[u8; 32]>,
}

/// One exact request for a named human to adjudicate a resolved finding.
/// Digests bind external owner artifacts without importing their content or
/// granting the reviewer authority merely because the request exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HumanGateRequestV1 {
    pub id: HumanGateRequestIdV1,
    pub reviewer: [u8; 16],
    pub finding_digest: [u8; 32],
    pub decision_digest: [u8; 32],
    pub material_revision_digest: [u8; 32],
    pub policy_digest: [u8; 32],
    pub requested_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HumanGateDispositionV1 {
    Approved,
    Rejected,
    Deferred,
    ChangesRequested,
}

/// Verified attribution facts injected by the command-scoped authorization
/// adapter. The human does not claim these facts in the response payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HumanGateAuthorityRefV1 {
    pub decision: [u8; 16],
    pub subject: [u8; 16],
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HumanGateResponseV1 {
    pub id: HumanGateResponseIdV1,
    pub request: HumanGateRequestIdV1,
    pub disposition: HumanGateDispositionV1,
    pub rationale_digest: [u8; 32],
    pub responded_at: u64,
    pub authority: HumanGateAuthorityRefV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliberationPhaseV1 {
    Collecting,
    ReadyToResolve,
    Resolved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliberationRefusalV1 {
    UnsupportedVersion,
    InvalidLimits,
    EmptyClaims,
    ClaimLimit,
    CriterionLimit,
    CandidateLimit,
    EmptyEnsemble,
    MemberLimit,
    DuplicateMember,
    InvalidStakeholder,
    InvalidEnsembleAdmission,
    EnsembleNotAdmitted,
    NonOrthogonalEnsemble,
    FuelInvalid,
    Frontier(FrontierRefusalV1),
    UnknownMember,
    ReportLimit,
    ReportConflict,
    AlreadyResolved,
    ReportsIncomplete,
    GateBeforeResolution,
    InvalidHumanGateRequest,
    HumanGateRequestConflict,
    HumanGateNotRequested,
    InvalidHumanGateResponse,
    HumanGateReviewerMismatch,
    HumanGateResponseConflict,
    ResolutionConflict,
    FrontierMismatch,
    SelectionOutsideFrontier,
    FuelRemaining,
    CheckpointMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliberationCheckpointV1 {
    pub version: u8,
    pub plan: DeliberationPlanV1,
    pub limits: DeliberationLimitsV1,
    pub reports: Vec<MemberReportV1>,
    pub resolution: Option<ResolutionV1>,
    pub human_gate_request: Option<HumanGateRequestV1>,
    pub human_gate_response: Option<HumanGateResponseV1>,
    pub remaining_fuel: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliberationV1 {
    plan: DeliberationPlanV1,
    limits: DeliberationLimitsV1,
    reports: Vec<MemberReportV1>,
    resolution: Option<ResolutionV1>,
    human_gate_request: Option<HumanGateRequestV1>,
    human_gate_response: Option<HumanGateResponseV1>,
    remaining_fuel: u32,
}

impl DeliberationV1 {
    pub fn try_new(
        plan: DeliberationPlanV1,
        limits: DeliberationLimitsV1,
    ) -> Result<Self, DeliberationRefusalV1> {
        validate_limits(limits)?;
        validate_plan(&plan, limits)?;
        let remaining_fuel = plan.fuel;
        Ok(Self {
            plan,
            limits,
            reports: Vec::new(),
            resolution: None,
            human_gate_request: None,
            human_gate_response: None,
            remaining_fuel,
        })
    }

    pub fn phase(&self) -> DeliberationPhaseV1 {
        if self.resolution.is_some() {
            DeliberationPhaseV1::Resolved
        } else if self.reports.len() == self.plan.members.len() {
            DeliberationPhaseV1::ReadyToResolve
        } else {
            DeliberationPhaseV1::Collecting
        }
    }

    pub fn remaining_fuel(&self) -> u32 {
        self.remaining_fuel
    }
    pub fn reports(&self) -> &[MemberReportV1] {
        &self.reports
    }
    pub fn resolution(&self) -> Option<&ResolutionV1> {
        self.resolution.as_ref()
    }

    pub fn checkpoint(&self) -> DeliberationCheckpointV1 {
        DeliberationCheckpointV1 {
            version: 4,
            plan: self.plan.clone(),
            limits: self.limits,
            reports: self.reports.clone(),
            resolution: self.resolution.clone(),
            human_gate_request: self.human_gate_request,
            human_gate_response: self.human_gate_response,
            remaining_fuel: self.remaining_fuel,
        }
    }

    pub fn recover(checkpoint: DeliberationCheckpointV1) -> Result<Self, DeliberationRefusalV1> {
        if checkpoint.version != 4 {
            return Err(DeliberationRefusalV1::UnsupportedVersion);
        }
        let expected = checkpoint.clone();
        let mut recovered = Self::try_new(checkpoint.plan, checkpoint.limits)?;
        for report in checkpoint.reports {
            recovered.submit_report(report)?;
        }
        if let Some(resolution) = checkpoint.resolution {
            recovered.resolve(resolution)?;
        }
        if let Some(request) = checkpoint.human_gate_request {
            recovered.request_human_gate(request)?;
        }
        if let Some(response) = checkpoint.human_gate_response {
            recovered.respond_human_gate(response)?;
        }
        if recovered.checkpoint() != expected {
            return Err(DeliberationRefusalV1::CheckpointMismatch);
        }
        Ok(recovered)
    }

    pub fn submit_report(&mut self, report: MemberReportV1) -> Result<(), DeliberationRefusalV1> {
        if self.resolution.is_some() {
            return Err(DeliberationRefusalV1::AlreadyResolved);
        }
        if self.plan.admission.fitness != EnsembleFitnessV1::Yes {
            return Err(DeliberationRefusalV1::EnsembleNotAdmitted);
        }
        if let Some(existing) = self
            .reports
            .iter()
            .find(|item| item.member == report.member || item.id == report.id)
        {
            return if existing == &report {
                Ok(())
            } else {
                Err(DeliberationRefusalV1::ReportConflict)
            };
        }
        if !self
            .plan
            .members
            .iter()
            .any(|member| member.id == report.member)
        {
            return Err(DeliberationRefusalV1::UnknownMember);
        }
        if report.evidence_digests.len() > self.limits.max_evidence_per_report
            || report.dissent_digests.len() > self.limits.max_dissent_per_report
        {
            return Err(DeliberationRefusalV1::ReportLimit);
        }
        self.spend_fuel()?;
        self.reports.push(report);
        Ok(())
    }

    pub fn resolve(&mut self, resolution: ResolutionV1) -> Result<(), DeliberationRefusalV1> {
        if let Some(existing) = &self.resolution {
            return if existing == &resolution {
                Ok(())
            } else {
                Err(DeliberationRefusalV1::ResolutionConflict)
            };
        }
        match resolution {
            ResolutionV1::Mu(MuReasonV1::FuelExhausted) => {
                if self.remaining_fuel != 0 {
                    return Err(DeliberationRefusalV1::FuelRemaining);
                }
            }
            ResolutionV1::Mu(_) => {
                self.spend_fuel()?;
            }
            _ => {
                if self.reports.len() != self.plan.members.len() {
                    return Err(DeliberationRefusalV1::ReportsIncomplete);
                }
                validate_resolution(&self.plan, &resolution)?;
                self.spend_fuel()?;
            }
        }
        self.resolution = Some(resolution);
        Ok(())
    }

    pub fn request_human_gate(
        &mut self,
        request: HumanGateRequestV1,
    ) -> Result<(), DeliberationRefusalV1> {
        if self.resolution.is_none() {
            return Err(DeliberationRefusalV1::GateBeforeResolution);
        }
        if !valid_human_gate_request(request) {
            return Err(DeliberationRefusalV1::InvalidHumanGateRequest);
        }
        if let Some(existing) = self.human_gate_request {
            return if existing == request {
                Ok(())
            } else {
                Err(DeliberationRefusalV1::HumanGateRequestConflict)
            };
        }
        self.human_gate_request = Some(request);
        Ok(())
    }

    pub fn respond_human_gate(
        &mut self,
        response: HumanGateResponseV1,
    ) -> Result<(), DeliberationRefusalV1> {
        let request = self
            .human_gate_request
            .ok_or(DeliberationRefusalV1::HumanGateNotRequested)?;
        if !valid_human_gate_response(response) {
            return Err(DeliberationRefusalV1::InvalidHumanGateResponse);
        }
        if response.request != request.id || response.authority.subject != request.reviewer {
            return Err(DeliberationRefusalV1::HumanGateReviewerMismatch);
        }
        if let Some(existing) = self.human_gate_response {
            return if existing == response {
                Ok(())
            } else {
                Err(DeliberationRefusalV1::HumanGateResponseConflict)
            };
        }
        self.human_gate_response = Some(response);
        Ok(())
    }

    fn spend_fuel(&mut self) -> Result<(), DeliberationRefusalV1> {
        match self.remaining_fuel.checked_sub(1) {
            Some(remaining) => {
                self.remaining_fuel = remaining;
                Ok(())
            }
            None => Err(DeliberationRefusalV1::FuelInvalid),
        }
    }
}

fn validate_limits(limits: DeliberationLimitsV1) -> Result<(), DeliberationRefusalV1> {
    if limits.max_claims == 0
        || limits.max_criteria == 0
        || limits.max_candidates == 0
        || limits.max_members == 0
        || limits.max_evidence_per_report == 0
        || limits.max_dissent_per_report == 0
        || limits.max_fuel == 0
    {
        Err(DeliberationRefusalV1::InvalidLimits)
    } else {
        Ok(())
    }
}

fn validate_plan(
    plan: &DeliberationPlanV1,
    limits: DeliberationLimitsV1,
) -> Result<(), DeliberationRefusalV1> {
    if plan.claims.is_empty() {
        return Err(DeliberationRefusalV1::EmptyClaims);
    }
    if plan.claims.len() > limits.max_claims {
        return Err(DeliberationRefusalV1::ClaimLimit);
    }
    if plan.criteria.len() > limits.max_criteria {
        return Err(DeliberationRefusalV1::CriterionLimit);
    }
    if plan.candidates.len() > limits.max_candidates {
        return Err(DeliberationRefusalV1::CandidateLimit);
    }
    pareto_frontier(&plan.criteria, &plan.candidates).map_err(DeliberationRefusalV1::Frontier)?;
    if plan.members.is_empty() {
        return Err(DeliberationRefusalV1::EmptyEnsemble);
    }
    if plan.members.len() > limits.max_members {
        return Err(DeliberationRefusalV1::MemberLimit);
    }
    if plan
        .members
        .iter()
        .map(|item| item.id)
        .collect::<BTreeSet<_>>()
        .len()
        != plan.members.len()
    {
        return Err(DeliberationRefusalV1::DuplicateMember);
    }
    if !valid_admission(plan.admission) {
        return Err(DeliberationRefusalV1::InvalidEnsembleAdmission);
    }
    if plan
        .members
        .iter()
        .any(|member| !valid_stakeholder(member.stakeholder))
    {
        return Err(DeliberationRefusalV1::InvalidStakeholder);
    }
    if !plan.orthogonality.all_orthogonal {
        return Err(DeliberationRefusalV1::NonOrthogonalEnsemble);
    }
    if plan.fuel == 0 || plan.fuel > limits.max_fuel {
        return Err(DeliberationRefusalV1::FuelInvalid);
    }
    Ok(())
}

fn valid_admission(value: EnsembleAdmissionV1) -> bool {
    value.affected_population_digest != [0; 32]
        && value.stakeholder_map_digest != [0; 32]
        && value.coverage_manifest_digest != [0; 32]
        && value.equity_assessment_digest != [0; 32]
        && value.assessor_principal != [0; 16]
}

fn valid_stakeholder(value: StakeholderRefV1) -> bool {
    if value.principal == [0; 16] || value.constituency_digest == [0; 32] {
        return false;
    }
    match value.representation {
        StakeholderRepresentationV1::ActualStakeholder => {
            value.accountable_principal.is_some() && value.mandate_digest.is_none()
        }
        StakeholderRepresentationV1::AuthorizedRepresentative => {
            value.accountable_principal.is_some() && value.mandate_digest.is_some()
        }
        StakeholderRepresentationV1::AgentModeledPerspective
        | StakeholderRepresentationV1::TechnicalSme => value.mandate_digest.is_none(),
    }
}

fn valid_human_gate_request(value: HumanGateRequestV1) -> bool {
    value.id.0 != [0; 16]
        && value.reviewer != [0; 16]
        && value.finding_digest != [0; 32]
        && value.decision_digest != [0; 32]
        && value.material_revision_digest != [0; 32]
        && value.policy_digest != [0; 32]
}

fn valid_human_gate_response(value: HumanGateResponseV1) -> bool {
    value.id.0 != [0; 16]
        && value.request.0 != [0; 16]
        && value.rationale_digest != [0; 32]
        && value.authority.decision != [0; 16]
        && value.authority.subject != [0; 16]
}

fn validate_resolution(
    plan: &DeliberationPlanV1,
    resolution: &ResolutionV1,
) -> Result<(), DeliberationRefusalV1> {
    if let ResolutionV1::AuthorizedChoice {
        frontier, selected, ..
    } = resolution
    {
        let computed = pareto_frontier(&plan.criteria, &plan.candidates)
            .map_err(DeliberationRefusalV1::Frontier)?;
        if frontier != &computed {
            return Err(DeliberationRefusalV1::FrontierMismatch);
        }
        if !frontier.contains(selected) {
            return Err(DeliberationRefusalV1::SelectionOutsideFrontier);
        }
    }
    Ok(())
}

/// Returns non-dominated candidate identities in caller-supplied order.
pub fn pareto_frontier(
    criteria: &[CriterionV1],
    candidates: &[CandidateV1],
) -> Result<Vec<CandidateIdV1>, FrontierRefusalV1> {
    validate(criteria, candidates)?;
    Ok(candidates
        .iter()
        .enumerate()
        .filter(|(index, candidate)| {
            !candidates.iter().enumerate().any(|(other_index, other)| {
                *index != other_index && dominates(criteria, other, candidate)
            })
        })
        .map(|(_, candidate)| candidate.id)
        .collect())
}

fn validate(criteria: &[CriterionV1], candidates: &[CandidateV1]) -> Result<(), FrontierRefusalV1> {
    if criteria.is_empty() {
        return Err(FrontierRefusalV1::NoCriteria);
    }
    if candidates.is_empty() {
        return Err(FrontierRefusalV1::NoCandidates);
    }
    if criteria
        .iter()
        .map(|item| item.id)
        .collect::<BTreeSet<_>>()
        .len()
        != criteria.len()
    {
        return Err(FrontierRefusalV1::DuplicateCriterion);
    }
    if candidates
        .iter()
        .map(|item| item.id)
        .collect::<BTreeSet<_>>()
        .len()
        != candidates.len()
    {
        return Err(FrontierRefusalV1::DuplicateCandidate);
    }
    if candidates
        .iter()
        .any(|item| item.scores.len() != criteria.len())
    {
        return Err(FrontierRefusalV1::ScoreDimensionMismatch);
    }
    Ok(())
}

fn dominates(criteria: &[CriterionV1], left: &CandidateV1, right: &CandidateV1) -> bool {
    let mut strictly_better = false;
    for ((criterion, left_score), right_score) in
        criteria.iter().zip(&left.scores).zip(&right.scores)
    {
        match (criterion.direction, left_score.cmp(right_score)) {
            (CriterionDirectionV1::Maximize, std::cmp::Ordering::Less)
            | (CriterionDirectionV1::Minimize, std::cmp::Ordering::Greater) => return false,
            (CriterionDirectionV1::Maximize, std::cmp::Ordering::Greater)
            | (CriterionDirectionV1::Minimize, std::cmp::Ordering::Less) => strictly_better = true,
            _ => {}
        }
    }
    strictly_better
}
