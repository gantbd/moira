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
    pub remaining_fuel: u32,
    pub phase: DeliberationPhaseV1,
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
    Status(StatusV1),
    Deliberation(Option<Box<DeliberationViewV1>>),
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
    NonOrthogonalEnsemble,
    FuelInvalid,
    Frontier(FrontierRefusalV1),
    UnknownMember,
    ReportLimit,
    ReportConflict,
    AlreadyResolved,
    ReportsIncomplete,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplyV1 {
    Accepted(OutcomeV1),
    Refused(RefusalV1),
}
