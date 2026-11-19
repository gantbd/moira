//! Transport-neutral contract for methodical deliberation.
//!
//! These DTOs intentionally do not depend on `moira-core`. Digests identify
//! exact Rune methods and Dendrite surfaces; they do not grant authority.

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub [u8; 16]);

        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(<opaque>)"))
            }
        }
    };
}

opaque_id!(CandidateIdV1);
opaque_id!(CriterionIdV1);
opaque_id!(AuthorityDecisionIdV1);
opaque_id!(AuthoritySubjectIdV1);
opaque_id!(DeliberationIdV1);
opaque_id!(EnsembleMemberIdV1);
opaque_id!(ReportIdV1);
opaque_id!(HumanGateRequestIdV1);
opaque_id!(HumanGateResponseIdV1);
opaque_id!(FindingIdV1);
opaque_id!(RebuttalIdV1);
opaque_id!(FindingDispositionIdV1);

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliberationPlanV1 {
    pub id: DeliberationIdV1,
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

/// One content-free dissent identity that remains unresolved in Moira's
/// current deliberation contract. Reports are immutable and no command settles
/// a dissent digest; findings and their dispositions are separate review facts
/// and therefore cannot silently erase this reference.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct UnresolvedDissentRefV1 {
    pub source_report: ReportIdV1,
    pub dissent_digest: [u8; 32],
}

pub fn unresolved_dissent(
    reports: &[MemberReportV1],
) -> impl Iterator<Item = UnresolvedDissentRefV1> + '_ {
    reports.iter().flat_map(|report| {
        report
            .dissent_digests
            .iter()
            .copied()
            .map(|dissent_digest| UnresolvedDissentRefV1 {
                source_report: report.id,
                dissent_digest,
            })
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HumanGateMaterialRefV1 {
    pub owner_route: [u8; 16],
    pub artifact_route: [u8; 16],
    pub expected_revision: u64,
    pub expected_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HumanGateRequestV1 {
    pub id: HumanGateRequestIdV1,
    pub reviewer: [u8; 16],
    pub finding_digest: [u8; 32],
    pub decision_digest: [u8; 32],
    pub material_revision_digest: [u8; 32],
    pub policy_digest: [u8; 32],
    pub decision_packet: HumanGateMaterialRefV1,
    pub requested_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HumanGateDispositionV1 {
    Approved,
    Rejected,
    Deferred,
    ChangesRequested,
}

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
    pub rationale: HumanGateMaterialRefV1,
    pub responded_at: u64,
    pub authority: HumanGateAuthorityRefV1,
}

/// Payload proposed by a terminal. Verified reviewer and authority facts are
/// supplied separately by the command-scoped authorization adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HumanGateResponseSubmissionV1 {
    pub id: HumanGateResponseIdV1,
    pub request: HumanGateRequestIdV1,
    pub disposition: HumanGateDispositionV1,
    pub rationale_digest: [u8; 32],
    pub rationale: HumanGateMaterialRefV1,
    pub responded_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HumanGateOperationV1 {
    pub deliberation: DeliberationIdV1,
    pub expected_revision: u64,
    pub response: HumanGateResponseSubmissionV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliberationPhaseV1 {
    Collecting,
    ReadyToResolve,
    Resolved,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliberationViewV1 {
    pub version: u8,
    pub plan: DeliberationPlanV1,
    pub limits: DeliberationLimitsV1,
    pub reports: Vec<MemberReportV1>,
    pub resolution: Option<ResolutionV1>,
    pub human_gate_request: Option<HumanGateRequestV1>,
    pub human_gate_response: Option<HumanGateResponseV1>,
    pub remaining_fuel: u32,
    pub phase: DeliberationPhaseV1,
}

impl DeliberationViewV1 {
    pub fn unresolved_dissent(&self) -> impl Iterator<Item = UnresolvedDissentRefV1> + '_ {
        unresolved_dissent(&self.reports)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewLimitsV1 {
    pub max_findings: usize,
    pub max_rebuttals: usize,
    pub max_evidence_per_item: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CorrelationDimensionV1 {
    ExecutionPrincipal,
    ExecutionProfile,
    ContextManifest,
    Model,
    Provider,
    Prompt,
    Grounding,
    EvidenceScope,
    Organization,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CorrelationDimensionEvidenceV1 {
    pub dimension: CorrelationDimensionV1,
    pub observed_members: usize,
    pub distinct_values: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirstPassExposureV1 {
    pub committed_before_cross_review: usize,
    pub exposed_before_commit: usize,
    pub unknown: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndependenceAssessmentV1 {
    Correlated,
    NotEstablished,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnsembleCorrelationEvidenceV1 {
    pub members: usize,
    pub dimensions: Vec<CorrelationDimensionEvidenceV1>,
    pub first_pass: FirstPassExposureV1,
}

impl EnsembleCorrelationEvidenceV1 {
    pub fn undisclosed(members: usize) -> Self {
        let dimensions = [
            CorrelationDimensionV1::ExecutionPrincipal,
            CorrelationDimensionV1::ExecutionProfile,
            CorrelationDimensionV1::ContextManifest,
            CorrelationDimensionV1::Model,
            CorrelationDimensionV1::Provider,
            CorrelationDimensionV1::Prompt,
            CorrelationDimensionV1::Grounding,
            CorrelationDimensionV1::EvidenceScope,
            CorrelationDimensionV1::Organization,
        ]
        .into_iter()
        .map(|dimension| CorrelationDimensionEvidenceV1 {
            dimension,
            observed_members: 0,
            distinct_values: 0,
        })
        .collect();
        Self {
            members,
            dimensions,
            first_pass: FirstPassExposureV1 {
                committed_before_cross_review: 0,
                exposed_before_commit: 0,
                unknown: members,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewFindingV1 {
    pub id: FindingIdV1,
    pub source_report: ReportIdV1,
    pub owner: [u8; 16],
    pub finding_digest: [u8; 32],
    pub evidence_digests: Vec<[u8; 32]>,
    pub requires_human_gate: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebuttalV1 {
    pub id: RebuttalIdV1,
    pub finding: FindingIdV1,
    pub member: EnsembleMemberIdV1,
    pub rebuttal_digest: [u8; 32],
    pub evidence_digests: Vec<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindingDispositionKindV1 {
    Accepted,
    Mitigated,
    Tested,
    Deferred,
    RiskAccepted,
    Disputed,
    OutOfScope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindingDispositionV1 {
    pub id: FindingDispositionIdV1,
    pub finding: FindingIdV1,
    pub owner: [u8; 16],
    pub kind: FindingDispositionKindV1,
    pub rationale_digest: [u8; 32],
    pub evidence_digests: Vec<[u8; 32]>,
    pub human_gate_response: Option<HumanGateResponseIdV1>,
    pub decided_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewPhaseV1 {
    FirstPassComplete,
    Challenged,
    Disposed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewViewV1 {
    pub version: u8,
    pub deliberation: DeliberationIdV1,
    pub limits: ReviewLimitsV1,
    pub reports: Vec<MemberReportV1>,
    pub correlation: EnsembleCorrelationEvidenceV1,
    pub independence: IndependenceAssessmentV1,
    pub admitted_human_gate_response: Option<HumanGateResponseV1>,
    pub findings: Vec<ReviewFindingV1>,
    pub rebuttals: Vec<RebuttalV1>,
    pub dispositions: Vec<FindingDispositionV1>,
    pub phase: ReviewPhaseV1,
}

impl ReviewViewV1 {
    pub fn unresolved_dissent(&self) -> impl Iterator<Item = UnresolvedDissentRefV1> + '_ {
        unresolved_dissent(&self.reports)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationV1 {
    Admit(Box<DeliberationPlanV1>),
    SubmitReport {
        deliberation: DeliberationIdV1,
        report: MemberReportV1,
    },
    Resolve {
        deliberation: DeliberationIdV1,
        resolution: ResolutionV1,
    },
    RequestHumanGate {
        deliberation: DeliberationIdV1,
        request: HumanGateRequestV1,
    },
    BeginReview {
        deliberation: DeliberationIdV1,
        correlation: EnsembleCorrelationEvidenceV1,
    },
    SubmitFinding {
        deliberation: DeliberationIdV1,
        finding: ReviewFindingV1,
    },
    SubmitRebuttal {
        deliberation: DeliberationIdV1,
        rebuttal: RebuttalV1,
    },
    DisposeFinding {
        deliberation: DeliberationIdV1,
        disposition: FindingDispositionV1,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandV1 {
    Apply {
        expected_revision: u64,
        mutation: MutationV1,
    },
    Status,
    Deliberation {
        deliberation: DeliberationIdV1,
    },
    Review {
        deliberation: DeliberationIdV1,
    },
    RespondHumanGate {
        operation: HumanGateOperationV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusV1 {
    pub revision: u64,
    pub needs_reopen: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutcomeV1 {
    Applied {
        revision: u64,
        deliberation: Box<DeliberationViewV1>,
        replayed: bool,
    },
    ReviewApplied {
        revision: u64,
        deliberation: Box<DeliberationViewV1>,
        review: Box<ReviewViewV1>,
        replayed: bool,
    },
    Status(StatusV1),
    Deliberation(Option<Box<DeliberationViewV1>>),
    Review(Option<Box<ReviewViewV1>>),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
    NotAuthorized,
    InvalidLimits,
    UnsupportedVersion,
    RevisionConflict { expected: u64, actual: u64 },
    RevisionExhausted,
    Capacity,
    UnknownDeliberation,
    InputConflict,
    DuplicateDeliberation,
    CheckpointMismatch,
    Deliberation(DeliberationRefusalV1),
    Review(ReviewRefusalV1),
    DuplicateReview,
    ReviewNotBegun,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewRefusalV1 {
    InvalidLimits,
    FirstPassIncomplete,
    FindingLimit,
    RebuttalLimit,
    EvidenceLimit,
    InvalidFinding,
    InvalidRebuttal,
    InvalidDisposition,
    UnknownReport,
    UnknownFinding,
    UnknownMember,
    SelfRebuttal,
    FindingConflict,
    RebuttalConflict,
    DispositionConflict,
    AlreadyDisposed,
    RebuttalRequired,
    OwnerMismatch,
    HumanGateRequired,
    HumanGateNotApproved,
    HumanGateMismatch,
    InvalidReports,
    DeliberationMismatch,
    InvalidCorrelationEvidence,
    UnsupportedVersion,
    CheckpointMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplyV1 {
    Accepted(OutcomeV1),
    Refused(RefusalV1),
}
