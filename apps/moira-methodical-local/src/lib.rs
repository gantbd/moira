//! Strict one-shot composition for the local methodical harness.

#![forbid(unsafe_code)]

use aegis_client::AuthorityClient;
use aegis_protocol::{
    ActionV1, EngineNamespaceV1, OutcomeV1 as AegisOutcomeV1, ReplyV1 as AegisReplyV1, ScopeV1,
};
use daimon_protocol::ExecutionBudgetV1;
use moira_aegis::{AegisChoiceAuthorityV1, AegisToolAuthorityV1, TimeSourceV1, choice_resource};
use moira_chronos::{AttemptBindingV1, CurrentAttemptObserverV1};
use moira_client::{ToolContinuationMutationClient, ToolContinuationServicePort};
use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, DeliberationLimitsV1, EnsembleAdmissionV1, EnsembleFitnessV1,
    EnsembleMemberIdV1, MaybeValueV1, MuReasonV1, PersonaLensV1, ReportIdV1, ResolutionV1,
    StakeholderRefV1, StakeholderRepresentationV1, SurfaceDepthV1,
    registry::{RegistryLimitsV1, persistence::CodecLimitsV1},
    review::CorrelationDimensionV1,
};
use moira_daimon::{
    ExecutionIntentV1,
    ensemble::{IndependenceAssessmentV1, MemberOutcomeV1, MemberRunV1},
};
use moira_dendrite::{
    ClientLimitsV1, DendriteClient, GroundedMemberRequestV1, PlanGroundingRequestV1,
};
use moira_host::{ToolExecutionOutcomeV1, ToolExecutorV1, ToolInvocationV1};
use moira_local_harness::{
    begin_review_from_governed_run, observe_member_resume,
    process_tool::{ProcessToolErrorV1, ProcessToolExecutorV1, ToolProcessProfileV1},
    resolution::{resolve_authorized_choice, resolve_proven_bool},
    resolve_explicit, resume_member_and_retain, run,
    tool_turn::{ToolTurnOutcomeV1, execute_tool_turn},
    unix::connect_daimon,
};
use moira_protocol::{EffectIdV1, TaskRefV1, tool_continuation as tools};
use moira_rune_validator::{RuneTheoremValidatorConfigV1, RuneTheoremValidatorV1};
use moira_service::tool_continuation::ToolContinuationService;
use moira_sqlite::{
    deliberation::{DeliberationSqliteStore, DeliberationStoreLimitsV1},
    tool_continuation::{ToolContinuationSqliteStore, ToolContinuationStoreLimitsV1},
    tool_output::{ToolOutputSqliteStore, ToolOutputStoreLimitsV1},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fmt, fs,
    path::{Path, PathBuf},
    time::Duration,
};

mod peer;
pub use peer::{
    AgoraPeerConfigV1, AgoraRouteConfigV1, PeerActionConfigV1, PeerExecutionV1,
    PeerSagaStoreConfigV1, PeerSelectionConfigV1, VellumPeerConfigV1, VellumRouteConfigV1,
    execute_config as execute_peer_action,
};

const MAX_CONFIG_BYTES: u64 = 1_048_576;

#[derive(Debug, Eq, PartialEq)]
pub enum ErrorV1 {
    ConfigIo,
    ConfigTooLarge,
    ConfigJson,
    InvalidConfig,
    MissingSecret,
    Dendrite,
    Store,
    Daimon,
    GovernedRun,
    Tooling,
    Resolution,
    Peer,
    Review,
}

impl fmt::Display for ErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "moira methodical local: {self:?}")
    }
}

impl std::error::Error for ErrorV1 {}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigV1 {
    #[serde(default)]
    pub product: ProductConfigV1,
    pub dendrite: DendriteConfigV1,
    pub daimon: DaimonConfigV1,
    pub store: StoreConfigV1,
    pub plan: PlanConfigV1,
    pub runs: Vec<RunConfigV1>,
    #[serde(default)]
    pub tooling: Option<ToolingConfigV1>,
    #[serde(default)]
    pub resolution: Option<ResolutionConfigV1>,
    #[serde(default)]
    pub begin_review: bool,
    #[serde(default)]
    pub peer_action: Option<PeerActionConfigV1>,
    pub now: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProductConfigV1 {
    Chatbot,
    #[default]
    Workflow,
    Agent,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DendriteConfigV1 {
    pub port: u16,
    #[serde(default)]
    pub bearer_token_env: Option<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaimonConfigV1 {
    pub socket: PathBuf,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreConfigV1 {
    pub path: PathBuf,
    pub owner: String,
    #[serde(default = "default_database_pages")]
    pub max_database_pages: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanConfigV1 {
    pub id: String,
    pub question: String,
    pub method_load: String,
    pub method_state: String,
    pub fold_load: String,
    pub fold_tags: Vec<String>,
    pub orthogonality_load: String,
    pub claims: Vec<ClaimConfigV1>,
    pub criteria: Vec<CriterionConfigV1>,
    pub candidates: Vec<CandidateConfigV1>,
    pub admission: AdmissionConfigV1,
    pub members: Vec<MemberConfigV1>,
    pub fuel: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum ClaimConfigV1 {
    Formal,
    Empirical,
    Normative,
    Historical,
    Creative,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriterionConfigV1 {
    pub id: String,
    pub direction: DirectionConfigV1,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionConfigV1 {
    Maximize,
    Minimize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateConfigV1 {
    pub id: String,
    pub scores: Vec<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberConfigV1 {
    pub id: String,
    pub principal: String,
    pub constituency_digest: String,
    pub representation: RepresentationConfigV1,
    #[serde(default)]
    pub accountable_principal: Option<String>,
    #[serde(default)]
    pub mandate_digest: Option<String>,
    #[serde(default)]
    pub conflict_digest: Option<String>,
    pub lens: LensConfigV1,
    pub fold_target: String,
    pub orthogonality_role: String,
    pub depth: DepthConfigV1,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionConfigV1 {
    pub affected_population_digest: String,
    pub stakeholder_map_digest: String,
    pub coverage_manifest_digest: String,
    pub equity_assessment_digest: String,
    pub assessor_principal: String,
    pub fitness: FitnessConfigV1,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitnessConfigV1 {
    Yes,
    No,
    Contested,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepresentationConfigV1 {
    ActualStakeholder,
    AuthorizedRepresentative,
    AgentModeledPerspective,
    TechnicalSme,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LensConfigV1 {
    Historian,
    IdeaStormer,
    Theorist,
    Researcher,
    RigorReviewer,
    ConvergenceFacilitator,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepthConfigV1 {
    Ambient,
    Activated,
    Reference,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfigV1 {
    pub member: String,
    pub report: String,
    pub effect: String,
    pub execution: String,
    pub task_key: String,
    pub task_incarnation: u64,
    pub attempt_number: u32,
    pub principal: String,
    pub profile: String,
    pub idempotency_key: String,
    pub context: Vec<String>,
    pub tools: Vec<String>,
    pub budget: BudgetConfigV1,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetConfigV1 {
    pub max_elapsed_millis: u64,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_context_bytes: usize,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolingConfigV1 {
    pub continuation_store: StoreConfigV1,
    pub output_store: ToolOutputStoreConfigV1,
    pub aegis: AegisToolConfigV1,
    pub chronos: ChronosToolConfigV1,
    pub profiles: Vec<ToolProcessConfigV1>,
    pub budget: ToolBudgetConfigV1,
    pub max_turns: u32,
    #[serde(default = "default_tool_poll_ms")]
    pub poll_interval_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolOutputStoreConfigV1 {
    pub path: PathBuf,
    pub owner: String,
    pub max_entries: usize,
    pub max_output_bytes: usize,
    #[serde(default = "default_database_pages")]
    pub max_database_pages: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AegisToolConfigV1 {
    pub socket: PathBuf,
    pub subject: String,
    pub expected_epoch: u64,
    pub expires_at: u64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChronosToolConfigV1 {
    pub directory: PathBuf,
    pub bindings: Vec<ChronosTaskBindingConfigV1>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChronosTaskBindingConfigV1 {
    pub task_key: String,
    pub task_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolProcessConfigV1 {
    pub capability: String,
    pub executable: PathBuf,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub max_elapsed_millis: u64,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_stderr_bytes: usize,
    pub max_executable_bytes: u64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolBudgetConfigV1 {
    pub max_elapsed_millis: u64,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolutionConfigV1 {
    ProvenBool {
        program: PathBuf,
        #[serde(default)]
        arguments: Vec<String>,
        validator: String,
        max_output_bytes: usize,
        timeout_ms: u64,
    },
    AuthorizedChoice {
        selected_candidate: String,
        aegis: AegisResolutionConfigV1,
    },
    Maybe {
        value: MaybeConfigV1,
    },
    Mu {
        reason: MuConfigV1,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AegisResolutionConfigV1 {
    pub socket: PathBuf,
    pub subject: String,
    pub expected_epoch: u64,
    pub expires_at: u64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaybeConfigV1 {
    TrueOnly,
    FalseOnly,
    Both,
    Neither,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MuConfigV1 {
    FuelExhausted,
    MissingEvidence,
    ContradictoryEvidence,
    WrongQuestion,
    WrongEnsemble,
    NoAuthorizedSelector,
}

#[derive(Debug)]
pub struct ValidatedConfigV1 {
    dendrite: DendriteConfigV1,
    daimon: DaimonConfigV1,
    store: StoreConfigV1,
    owner: [u8; 16],
    plan: PlanGroundingRequestV1,
    runs: Vec<MemberRunV1>,
    tooling: Option<ValidatedToolingConfigV1>,
    resolution: Option<ValidatedResolutionV1>,
    begin_review: bool,
    peer_action: Option<peer::ValidatedPeerActionV1>,
    now: u64,
}

#[derive(Debug)]
enum ValidatedResolutionV1 {
    ProvenBool(RuneTheoremValidatorConfigV1),
    AuthorizedChoice {
        selected: CandidateIdV1,
        aegis: AegisResolutionConfigV1,
        subject: [u8; 16],
    },
    Maybe(MaybeValueV1),
    Mu(MuReasonV1),
}

#[derive(Debug)]
struct ValidatedToolingConfigV1 {
    continuation_store: StoreConfigV1,
    continuation_owner: [u8; 16],
    output_store: ToolOutputStoreConfigV1,
    output_owner: [u8; 16],
    aegis: AegisToolConfigV1,
    aegis_subject: [u8; 16],
    chronos: ChronosToolConfigV1,
    bindings: Vec<([u8; 16], String)>,
    profiles: Vec<([u8; 16], ToolProcessConfigV1)>,
    budget: ToolBudgetConfigV1,
    max_turns: u32,
    poll_interval: Duration,
}

struct ToolPort(ToolContinuationService<ToolContinuationSqliteStore>);

impl ToolContinuationServicePort for ToolPort {
    type Error = moira_sqlite::tool_continuation::ToolContinuationStoreErrorV1;

    fn execute(&mut self, command: tools::CommandV1) -> Result<tools::ReplyV1, Self::Error> {
        self.0.execute(command)
    }
}

#[derive(Clone, Copy)]
struct FixedTime(u64);

impl TimeSourceV1 for FixedTime {
    fn now(&mut self) -> u64 {
        self.0
    }
}

struct LazyAegisToolAuthorityV1 {
    config: AegisToolConfigV1,
    subject: [u8; 16],
    capabilities: Vec<[u8; 16]>,
    now: u64,
    connected: Option<AegisToolAuthorityV1<aegis_local_daemon::unix::UnixPort, FixedTime>>,
}

impl LazyAegisToolAuthorityV1 {
    fn authorize(
        &mut self,
        capability: tools::ToolCapabilityIdV1,
    ) -> Result<tools::AuthorityEvidenceV1, ()> {
        if self.connected.is_none() {
            let (session, port) = aegis_local_daemon::unix::connect(
                &self.config.socket,
                aegis_wire::LimitsV1::default(),
                Duration::from_millis(self.config.timeout_ms),
            )
            .map_err(|_| ())?;
            let mut client = AuthorityClient::new(session, port);
            let scopes = self
                .capabilities
                .iter()
                .copied()
                .map(|resource| ScopeV1 {
                    engine: EngineNamespaceV1::Daimon,
                    resource,
                    actions: 1 << ActionV1::Execute as u8,
                })
                .collect();
            let AegisReplyV1::Accepted(AegisOutcomeV1::GrantIssued(grant)) = client
                .issue(
                    self.config.expected_epoch,
                    self.now,
                    self.config.expires_at,
                    scopes,
                )
                .map_err(|_| ())?
            else {
                return Err(());
            };
            if grant.epoch != self.config.expected_epoch
                || grant.expires_at != self.config.expires_at
            {
                return Err(());
            }
            self.connected = Some(AegisToolAuthorityV1::new(
                client,
                grant.handle,
                self.subject,
                FixedTime(self.now),
            ));
        }
        self.connected
            .as_mut()
            .ok_or(())?
            .authorize(capability)
            .map_err(|_| ())
    }
}

struct ToolRuntimeV1 {
    client: ToolContinuationMutationClient<ToolPort>,
    executor: LazyProcessToolExecutorV1,
    outputs: ToolOutputSqliteStore,
    authority: LazyAegisToolAuthorityV1,
    attempts: CurrentAttemptObserverV1<chronos_local::LocalClient>,
    storage_revision: Option<u64>,
    budget: tools::ToolBudgetV1,
    max_turns: u32,
}

#[derive(Debug, Eq, PartialEq)]
enum LazyProcessToolErrorV1 {
    Configuration,
    Execution(ProcessToolErrorV1),
}

struct LazyProcessToolExecutorV1 {
    profiles: Vec<ToolProcessProfileV1>,
    poll_interval: Duration,
    connected: Option<ProcessToolExecutorV1>,
}

impl ToolExecutorV1 for LazyProcessToolExecutorV1 {
    type Error = LazyProcessToolErrorV1;

    fn execute(
        &mut self,
        invocation: &ToolInvocationV1,
    ) -> Result<ToolExecutionOutcomeV1, Self::Error> {
        if self.connected.is_none() {
            self.connected = Some(
                ProcessToolExecutorV1::try_new(self.profiles.clone(), self.poll_interval)
                    .map_err(|_| LazyProcessToolErrorV1::Configuration)?,
            );
        }
        self.connected
            .as_mut()
            .ok_or(LazyProcessToolErrorV1::Configuration)?
            .execute(invocation)
            .map_err(LazyProcessToolErrorV1::Execution)
    }
}

impl ToolRuntimeV1 {
    fn open(
        config: ValidatedToolingConfigV1,
        runs: &[MemberRunV1],
        now: u64,
    ) -> Result<Self, ErrorV1> {
        let max_calls = runs
            .iter()
            .map(|run| usize::try_from(run.execution.budget.max_tool_calls).unwrap_or(usize::MAX))
            .max()
            .unwrap_or(0);
        let max_sagas = runs
            .iter()
            .filter(|run| !run.execution.tools.is_empty())
            .count()
            .checked_mul(usize::try_from(config.max_turns).unwrap_or(usize::MAX))
            .ok_or(ErrorV1::Tooling)?;
        let max_argument_bytes = config
            .profiles
            .iter()
            .map(|(_, profile)| profile.max_input_bytes)
            .max()
            .ok_or(ErrorV1::Tooling)?;
        let codec_bytes = max_sagas
            .checked_mul(max_calls)
            .and_then(|effects| effects.checked_mul(max_argument_bytes.checked_add(2_048)?))
            .and_then(|bytes| bytes.checked_add(1_048_576))
            .filter(|bytes| *bytes <= 64 * 1_048_576)
            .ok_or(ErrorV1::Tooling)?;
        let limits = moira_core::tool_continuation::LimitsV1 {
            max_sagas,
            max_calls_per_saga: max_calls,
            max_argument_bytes,
            max_elapsed_millis: config.budget.max_elapsed_millis,
            max_output_bytes: config.budget.max_output_bytes,
        };
        let store = ToolContinuationSqliteStore::open(
            config.continuation_store.path,
            config.continuation_owner,
            ToolContinuationStoreLimitsV1 {
                codec: moira_core::tool_continuation::persistence::CodecLimitsV1 {
                    max_bytes: codec_bytes,
                    domain: limits,
                },
                saga: limits,
                max_database_pages: config.continuation_store.max_database_pages,
            },
        )
        .map_err(|_| ErrorV1::Tooling)?;
        let mut service = ToolContinuationService::new(store);
        let storage_revision = match service
            .execute(tools::CommandV1::Status)
            .map_err(|_| ErrorV1::Tooling)?
        {
            tools::ReplyV1::Accepted(tools::OutcomeV1::Status(status)) if !status.needs_reopen => {
                status.storage_revision
            }
            _ => return Err(ErrorV1::Tooling),
        };
        let outputs = ToolOutputSqliteStore::open(
            config.output_store.path,
            config.output_owner,
            ToolOutputStoreLimitsV1 {
                max_entries: config.output_store.max_entries,
                max_output_bytes: config.output_store.max_output_bytes,
                max_database_pages: config.output_store.max_database_pages,
            },
        )
        .map_err(|_| ErrorV1::Tooling)?;
        let profiles = config
            .profiles
            .iter()
            .map(|(capability, profile)| ToolProcessProfileV1 {
                capability: moira_core::tool_continuation::ToolCapabilityIdV1(*capability),
                executable: profile.executable.clone(),
                arguments: profile
                    .arguments
                    .iter()
                    .map(std::ffi::OsString::from)
                    .collect(),
                working_directory: profile.working_directory.clone(),
                max_elapsed: Duration::from_millis(profile.max_elapsed_millis),
                max_input_bytes: profile.max_input_bytes,
                max_output_bytes: profile.max_output_bytes,
                max_stderr_bytes: profile.max_stderr_bytes,
                max_executable_bytes: profile.max_executable_bytes,
            })
            .collect();
        let executor = LazyProcessToolExecutorV1 {
            profiles,
            poll_interval: config.poll_interval,
            connected: None,
        };

        let authority = LazyAegisToolAuthorityV1 {
            config: config.aegis,
            subject: config.aegis_subject,
            capabilities: config
                .profiles
                .iter()
                .map(|(capability, _)| *capability)
                .collect(),
            now,
            connected: None,
        };
        let attempts = CurrentAttemptObserverV1::new(
            chronos_local::LocalClient::new(config.chronos.directory),
            chronos_local::wire_limits(),
            config.bindings.len(),
            config
                .bindings
                .into_iter()
                .map(|(external_task_key, chronos_task_id)| AttemptBindingV1 {
                    external_task_key,
                    chronos_task_id,
                })
                .collect(),
        )
        .map_err(|_| ErrorV1::Tooling)?;
        Ok(Self {
            client: ToolContinuationMutationClient::new(ToolPort(service)),
            executor,
            outputs,
            authority,
            attempts,
            storage_revision,
            budget: tools::ToolBudgetV1 {
                max_elapsed_millis: config.budget.max_elapsed_millis,
                max_output_bytes: config.budget.max_output_bytes,
            },
            max_turns: config.max_turns,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SummaryV1 {
    pub cohort_members: usize,
    pub distinct_principals: usize,
    pub distinct_profiles: usize,
    pub distinct_context_manifests: usize,
    pub shared_principal: bool,
    pub shared_profile: bool,
    pub shared_context_manifest: bool,
    pub independence: IndependenceAssessmentV1,
    pub revision: u64,
    pub reports: usize,
    pub invalid_reports: usize,
    pub tool_continuations: usize,
    pub uncertain: usize,
    pub non_completed: usize,
    pub refused: usize,
    pub failed: usize,
    pub ready_to_resolve: bool,
    pub resolved: bool,
    pub review_begun: bool,
    pub peer_completed: bool,
    pub peer_uncertain: bool,
    pub peer_failed: bool,
}

impl SummaryV1 {
    pub fn json(self) -> String {
        serde_json::json!({
            "cohort_members": self.cohort_members,
            "distinct_principals": self.distinct_principals,
            "distinct_profiles": self.distinct_profiles,
            "distinct_context_manifests": self.distinct_context_manifests,
            "shared_principal": self.shared_principal,
            "shared_profile": self.shared_profile,
            "shared_context_manifest": self.shared_context_manifest,
            "independence": match self.independence {
                IndependenceAssessmentV1::Correlated => "correlated",
                IndependenceAssessmentV1::NotEstablished => "not_established",
            },
            "revision": self.revision,
            "reports": self.reports,
            "invalid_reports": self.invalid_reports,
            "tool_continuations": self.tool_continuations,
            "uncertain": self.uncertain,
            "non_completed": self.non_completed,
            "refused": self.refused,
            "failed": self.failed,
            "ready_to_resolve": self.ready_to_resolve,
            "resolved": self.resolved,
            "review_begun": self.review_begun,
            "peer_completed": self.peer_completed,
            "peer_uncertain": self.peer_uncertain,
            "peer_failed": self.peer_failed
        })
        .to_string()
    }
}

pub fn load(path: impl AsRef<Path>) -> Result<ConfigV1, ErrorV1> {
    let metadata = fs::metadata(path.as_ref()).map_err(|_| ErrorV1::ConfigIo)?;
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(ErrorV1::ConfigTooLarge);
    }
    let bytes = fs::read(path).map_err(|_| ErrorV1::ConfigIo)?;
    serde_json::from_slice(&bytes).map_err(|_| ErrorV1::ConfigJson)
}

pub fn validate(raw: ConfigV1) -> Result<ValidatedConfigV1, ErrorV1> {
    if raw.dendrite.port == 0
        || !(1..=300_000).contains(&raw.dendrite.timeout_ms)
        || !(1..=300_000).contains(&raw.daimon.timeout_ms)
        || empty_path(&raw.daimon.socket)
        || empty_path(&raw.store.path)
        || !(64..=1_048_576).contains(&raw.store.max_database_pages)
        || raw.plan.method_load.is_empty()
        || raw.plan.question.is_empty()
        || raw.plan.question.len() > 65_536
        || raw.plan.method_state.is_empty()
        || raw.plan.fold_load.is_empty()
        || raw.plan.fold_tags.is_empty()
        || raw.plan.orthogonality_load.is_empty()
        || raw.plan.claims.is_empty()
        || raw.plan.claims.len() > 64
        || raw.plan.criteria.is_empty()
        || raw.plan.criteria.len() > 32
        || raw.plan.candidates.is_empty()
        || raw.plan.candidates.len() > 64
        || raw.plan.members.is_empty()
        || raw.plan.members.len() > 16
        || raw.plan.fuel == 0
        || raw.plan.fuel > 1_024
        || usize::try_from(raw.plan.fuel).map_or(true, |fuel| fuel < raw.plan.members.len())
        || raw.runs.len() != raw.plan.members.len()
        || (raw.begin_review && raw.resolution.is_some())
    {
        return Err(ErrorV1::InvalidConfig);
    }
    if raw.plan.claims.iter().collect::<BTreeSet<_>>().len() != raw.plan.claims.len() {
        return Err(ErrorV1::InvalidConfig);
    }
    if raw.dendrite.bearer_token_env.as_ref().is_some_and(|name| {
        name.is_empty()
            || name.len() > 256
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    }) {
        return Err(ErrorV1::InvalidConfig);
    }
    let owner = id(&raw.store.owner)?;
    let plan_id = DeliberationIdV1(id(&raw.plan.id)?);
    let criteria = raw
        .plan
        .criteria
        .iter()
        .map(|value| {
            Ok(CriterionV1 {
                id: CriterionIdV1(id(&value.id)?),
                direction: match value.direction {
                    DirectionConfigV1::Maximize => CriterionDirectionV1::Maximize,
                    DirectionConfigV1::Minimize => CriterionDirectionV1::Minimize,
                },
            })
        })
        .collect::<Result<Vec<_>, ErrorV1>>()?;
    if criteria
        .iter()
        .map(|value| value.id)
        .collect::<BTreeSet<_>>()
        .len()
        != criteria.len()
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let candidates = raw
        .plan
        .candidates
        .iter()
        .map(|value| {
            if value.scores.len() != criteria.len() {
                return Err(ErrorV1::InvalidConfig);
            }
            Ok(CandidateV1 {
                id: CandidateIdV1(id(&value.id)?),
                scores: value.scores.clone(),
            })
        })
        .collect::<Result<Vec<_>, ErrorV1>>()?;
    if candidates
        .iter()
        .map(|value| value.id)
        .collect::<BTreeSet<_>>()
        .len()
        != candidates.len()
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let admission = EnsembleAdmissionV1 {
        affected_population_digest: digest(&raw.plan.admission.affected_population_digest)?,
        stakeholder_map_digest: digest(&raw.plan.admission.stakeholder_map_digest)?,
        coverage_manifest_digest: digest(&raw.plan.admission.coverage_manifest_digest)?,
        equity_assessment_digest: digest(&raw.plan.admission.equity_assessment_digest)?,
        assessor_principal: id(&raw.plan.admission.assessor_principal)?,
        fitness: match raw.plan.admission.fitness {
            FitnessConfigV1::Yes => EnsembleFitnessV1::Yes,
            FitnessConfigV1::No => EnsembleFitnessV1::No,
            FitnessConfigV1::Contested => EnsembleFitnessV1::Contested,
        },
    };
    let members = raw
        .plan
        .members
        .iter()
        .map(|value| {
            if value.fold_target.is_empty() || value.orthogonality_role.is_empty() {
                return Err(ErrorV1::InvalidConfig);
            }
            Ok(GroundedMemberRequestV1 {
                id: EnsembleMemberIdV1(id(&value.id)?),
                stakeholder: StakeholderRefV1 {
                    principal: id(&value.principal)?,
                    constituency_digest: digest(&value.constituency_digest)?,
                    representation: match value.representation {
                        RepresentationConfigV1::ActualStakeholder => {
                            StakeholderRepresentationV1::ActualStakeholder
                        }
                        RepresentationConfigV1::AuthorizedRepresentative => {
                            StakeholderRepresentationV1::AuthorizedRepresentative
                        }
                        RepresentationConfigV1::AgentModeledPerspective => {
                            StakeholderRepresentationV1::AgentModeledPerspective
                        }
                        RepresentationConfigV1::TechnicalSme => {
                            StakeholderRepresentationV1::TechnicalSme
                        }
                    },
                    accountable_principal: value
                        .accountable_principal
                        .as_deref()
                        .map(id)
                        .transpose()?,
                    mandate_digest: value.mandate_digest.as_deref().map(digest).transpose()?,
                    conflict_digest: value.conflict_digest.as_deref().map(digest).transpose()?,
                },
                lens: lens(value.lens),
                fold_target: value.fold_target.clone(),
                orthogonality_role: value.orthogonality_role.clone(),
                depth: depth(value.depth),
            })
        })
        .collect::<Result<Vec<_>, ErrorV1>>()?;
    let member_ids = members
        .iter()
        .map(|value| value.id)
        .collect::<BTreeSet<_>>();
    if member_ids.len() != members.len()
        || raw.plan.fold_tags.iter().collect::<BTreeSet<_>>().len() != raw.plan.fold_tags.len()
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let runs = raw
        .runs
        .iter()
        .map(run_config)
        .collect::<Result<Vec<_>, _>>()?;
    if runs
        .iter()
        .map(|value| value.member)
        .collect::<BTreeSet<_>>()
        != member_ids
        || !unique(&runs, |value| value.report.0)
        || !unique(&runs, |value| value.execution.effect.0)
        || !unique(&runs, |value| value.execution.execution_id)
        || !unique(&runs, |value| value.execution.idempotency_key)
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let advertises_tools = runs.iter().any(|run| !run.execution.tools.is_empty());
    match raw.product {
        ProductConfigV1::Chatbot if members.len() != 1 => return Err(ErrorV1::InvalidConfig),
        ProductConfigV1::Workflow if members.len() < 2 => return Err(ErrorV1::InvalidConfig),
        ProductConfigV1::Agent if members.len() < 2 || !advertises_tools => {
            return Err(ErrorV1::InvalidConfig);
        }
        _ => {}
    }
    let tooling = validate_tooling(raw.tooling, &runs, raw.now, &raw.store.path, owner)?;
    let question_digest: [u8; 32] = Sha256::digest(raw.plan.question.as_bytes()).into();
    let resolution = validate_resolution(raw.resolution, question_digest, &candidates, raw.now)?;
    let peer_action = peer::validate(
        raw.peer_action,
        &member_ids,
        raw.product,
        matches!(
            resolution,
            Some(ValidatedResolutionV1::AuthorizedChoice { .. })
        ),
        &raw.store.path,
        owner,
        raw.now,
    )?;
    Ok(ValidatedConfigV1 {
        dendrite: raw.dendrite,
        daimon: raw.daimon,
        store: raw.store,
        owner,
        plan: PlanGroundingRequestV1 {
            id: plan_id,
            question_digest,
            method_load: raw.plan.method_load,
            method_state: raw.plan.method_state,
            fold_load: raw.plan.fold_load,
            fold_tags: raw.plan.fold_tags,
            orthogonality_load: raw.plan.orthogonality_load,
            claims: raw.plan.claims.into_iter().map(claim).collect(),
            criteria,
            candidates,
            admission,
            members,
            fuel: raw.plan.fuel,
        },
        runs,
        tooling,
        resolution,
        begin_review: raw.begin_review,
        peer_action,
        now: raw.now,
    })
}

pub fn execute(config: ValidatedConfigV1) -> Result<SummaryV1, ErrorV1> {
    let ValidatedConfigV1 {
        dendrite,
        daimon,
        store: store_config,
        owner,
        plan: grounding_request,
        runs,
        tooling,
        resolution,
        begin_review,
        peer_action,
        now,
    } = config;
    let bearer = dendrite
        .bearer_token_env
        .as_ref()
        .map(|name| std::env::var(name).map_err(|_| ErrorV1::MissingSecret))
        .transpose()?;
    let mut dendrite = DendriteClient::connect_loopback(
        dendrite.port,
        bearer,
        ClientLimitsV1 {
            timeout: Duration::from_millis(dendrite.timeout_ms),
            ..ClientLimitsV1::default()
        },
    )
    .map_err(|_| ErrorV1::Dendrite)?;
    let plan = dendrite
        .ground_plan(grounding_request)
        .map_err(|_| ErrorV1::Dendrite)?;
    let mut daimon = connect_daimon(
        daimon.socket,
        daimon_wire::LimitsV1::default(),
        daimon_wire::ModelLimitsV1::default(),
        Duration::from_millis(daimon.timeout_ms),
    )
    .map_err(|_| ErrorV1::Daimon)?;
    let mut store = DeliberationSqliteStore::open(
        store_config.path,
        owner,
        store_limits(store_config.max_database_pages),
    )
    .map_err(|_| ErrorV1::Store)?;
    let mut completed = run(&mut store, &mut daimon, plan.clone(), runs.clone(), now)
        .map_err(|_| ErrorV1::GovernedRun)?;
    let needs_tool_runtime = completed
        .outcomes
        .iter()
        .any(|outcome| matches!(outcome, MemberOutcomeV1::ToolContinuation { .. }));
    let mut tool_runtime = if needs_tool_runtime {
        Some(ToolRuntimeV1::open(
            tooling.ok_or(ErrorV1::Tooling)?,
            &runs,
            now,
        )?)
    } else {
        None
    };
    let principal = completed
        .correlation
        .dimension(CorrelationDimensionV1::ExecutionPrincipal)
        .expect("adapter discloses every correlation dimension");
    let profile = completed
        .correlation
        .dimension(CorrelationDimensionV1::ExecutionProfile)
        .expect("adapter discloses every correlation dimension");
    let context = completed
        .correlation
        .dimension(CorrelationDimensionV1::ContextManifest)
        .expect("adapter discloses every correlation dimension");
    let mut summary = SummaryV1 {
        cohort_members: completed.correlation.members,
        distinct_principals: principal.distinct_values,
        distinct_profiles: profile.distinct_values,
        distinct_context_manifests: context.distinct_values,
        shared_principal: principal.distinct_values < principal.observed_members,
        shared_profile: profile.distinct_values < profile.observed_members,
        shared_context_manifest: context.distinct_values < context.observed_members,
        independence: completed.correlation.independence(),
        revision: store.revision().0,
        reports: 0,
        invalid_reports: 0,
        tool_continuations: 0,
        uncertain: 0,
        non_completed: 0,
        refused: 0,
        failed: 0,
        ready_to_resolve: false,
        resolved: false,
        review_begun: false,
        peer_completed: false,
        peer_uncertain: false,
        peer_failed: false,
    };
    let mut terminal_outcomes = Vec::new();
    let initial_outcomes = std::mem::take(&mut completed.outcomes);
    for outcome in initial_outcomes {
        let MemberOutcomeV1::ToolContinuation {
            member,
            continuation,
            result,
        } = outcome
        else {
            record_outcome(&mut summary, &outcome);
            terminal_outcomes.push(outcome);
            continue;
        };
        summary.tool_continuations += 1;
        let base_run = runs
            .iter()
            .find(|run| run.member == member)
            .cloned()
            .ok_or(ErrorV1::Tooling)?;
        let runtime = tool_runtime.as_mut().ok_or(ErrorV1::Tooling)?;
        let mut continuation = continuation;
        let mut result = *result;
        let mut turn = 0_u32;
        let mut used_tool_calls = 0_u32;
        loop {
            if continuation.source_execution != result.receipt.execution_id {
                return Err(ErrorV1::Tooling);
            }
            turn = turn.checked_add(1).ok_or(ErrorV1::Tooling)?;
            if turn > runtime.max_turns {
                summary.non_completed += 1;
                break;
            }
            let workflow = tool_workflow(plan.id, member, result.receipt.execution_id);
            let tool_outcome = {
                let ToolRuntimeV1 {
                    client,
                    executor,
                    outputs,
                    authority,
                    attempts,
                    storage_revision,
                    budget,
                    ..
                } = runtime;
                let expected = *storage_revision;
                let mut authorize = |capability| authority.authorize(capability);
                let mut observe = |task_key, incarnation, attempt| {
                    attempts.observe(task_key, incarnation, attempt)
                };
                execute_tool_turn(
                    client,
                    executor,
                    outputs,
                    expected,
                    workflow,
                    &result,
                    *budget,
                    &mut authorize,
                    &mut observe,
                )
                .map_err(|_| ErrorV1::Tooling)?
            };
            match tool_outcome {
                ToolTurnOutcomeV1::Completed {
                    storage_revision,
                    results,
                    ..
                } => {
                    runtime.storage_revision = Some(storage_revision);
                    used_tool_calls = used_tool_calls
                        .checked_add(u32::try_from(results.len()).map_err(|_| ErrorV1::Tooling)?)
                        .ok_or(ErrorV1::Tooling)?;
                    let resumed = resume_member_and_retain(
                        &mut store,
                        &mut daimon,
                        &plan,
                        resumed_run(
                            &base_run,
                            continuation.source_execution,
                            turn,
                            used_tool_calls,
                        ),
                        continuation,
                        results,
                        now,
                    )
                    .map_err(|_| ErrorV1::GovernedRun)?;
                    observe_member_resume(
                        &mut completed,
                        &plan,
                        &runs,
                        &resumed.outcome,
                        &resumed.retained,
                    )
                    .map_err(|_| ErrorV1::GovernedRun)?;
                    match resumed.outcome {
                        MemberOutcomeV1::ToolContinuation {
                            continuation: next,
                            result: next_result,
                            ..
                        } => {
                            summary.tool_continuations += 1;
                            continuation = next;
                            result = *next_result;
                        }
                        terminal => {
                            record_outcome(&mut summary, &terminal);
                            terminal_outcomes.push(terminal);
                            break;
                        }
                    }
                }
                ToolTurnOutcomeV1::Failed {
                    storage_revision, ..
                } => {
                    runtime.storage_revision = Some(storage_revision);
                    summary.failed += 1;
                    break;
                }
                ToolTurnOutcomeV1::Uncertain {
                    storage_revision, ..
                } => {
                    runtime.storage_revision = Some(storage_revision);
                    summary.uncertain += 1;
                    break;
                }
            }
        }
    }
    let principal = completed
        .correlation
        .dimension(CorrelationDimensionV1::ExecutionPrincipal)
        .expect("adapter discloses every correlation dimension");
    let profile = completed
        .correlation
        .dimension(CorrelationDimensionV1::ExecutionProfile)
        .expect("adapter discloses every correlation dimension");
    let context = completed
        .correlation
        .dimension(CorrelationDimensionV1::ContextManifest)
        .expect("adapter discloses every correlation dimension");
    summary.distinct_principals = principal.distinct_values;
    summary.distinct_profiles = profile.distinct_values;
    summary.distinct_context_manifests = context.distinct_values;
    summary.shared_principal = principal.distinct_values < principal.observed_members;
    summary.shared_profile = profile.distinct_values < profile.observed_members;
    summary.shared_context_manifest = context.distinct_values < context.observed_members;
    summary.independence = completed.correlation.independence();
    let reports_complete =
        completed.retained.reports.len() == completed.retained.plan.members.len();
    if begin_review {
        if !reports_complete {
            return Err(ErrorV1::Review);
        }
        begin_review_from_governed_run(&mut store, &completed).map_err(|_| ErrorV1::Review)?;
        summary.review_begun = true;
    }
    if let Some(resolution) = resolution {
        if !reports_complete {
            return Err(ErrorV1::Resolution);
        }
        apply_resolution(&mut store, plan.id, resolution, now)?;
        summary.resolved = true;
    }
    if let Some(peer_action) = peer_action {
        let checkpoint = store.deliberation(plan.id).ok_or(ErrorV1::Peer)?;
        match peer::execute(peer_action, &checkpoint, &terminal_outcomes, now)? {
            peer::PeerExecutionV1::Completed => summary.peer_completed = true,
            peer::PeerExecutionV1::Uncertain => summary.peer_uncertain = true,
            peer::PeerExecutionV1::Failed => summary.peer_failed = true,
        }
    }
    summary.revision = store.revision().0;
    summary.ready_to_resolve = reports_complete && !summary.resolved && !summary.review_begun;
    Ok(summary)
}

fn apply_resolution(
    store: &mut DeliberationSqliteStore,
    deliberation: DeliberationIdV1,
    resolution: ValidatedResolutionV1,
    now: u64,
) -> Result<(), ErrorV1> {
    if let Some(existing) = store
        .deliberation(deliberation)
        .ok_or(ErrorV1::Resolution)?
        .resolution
    {
        return if resolution_matches(&resolution, &existing) {
            Ok(())
        } else {
            Err(ErrorV1::Resolution)
        };
    }
    match resolution {
        ValidatedResolutionV1::ProvenBool(config) => {
            let mut validator =
                RuneTheoremValidatorV1::new(config).map_err(|_| ErrorV1::Resolution)?;
            resolve_proven_bool(store, deliberation, &mut validator)
                .map_err(|_| ErrorV1::Resolution)?;
        }
        ValidatedResolutionV1::AuthorizedChoice {
            selected,
            aegis,
            subject,
        } => {
            let (session, port) = aegis_local_daemon::unix::connect(
                &aegis.socket,
                aegis_wire::LimitsV1::default(),
                Duration::from_millis(aegis.timeout_ms),
            )
            .map_err(|_| ErrorV1::Resolution)?;
            let mut client = AuthorityClient::new(session, port);
            let AegisReplyV1::Accepted(AegisOutcomeV1::GrantIssued(grant)) = client
                .issue(
                    aegis.expected_epoch,
                    now,
                    aegis.expires_at,
                    vec![ScopeV1 {
                        engine: EngineNamespaceV1::Moira,
                        resource: choice_resource(deliberation),
                        actions: 1 << ActionV1::Administer as u8,
                    }],
                )
                .map_err(|_| ErrorV1::Resolution)?
            else {
                return Err(ErrorV1::Resolution);
            };
            if grant.epoch != aegis.expected_epoch || grant.expires_at != aegis.expires_at {
                return Err(ErrorV1::Resolution);
            }
            let mut authority = AegisChoiceAuthorityV1::new(
                client,
                grant.handle,
                subject,
                selected,
                FixedTime(now),
            );
            resolve_authorized_choice(store, deliberation, &mut authority)
                .map_err(|_| ErrorV1::Resolution)?;
        }
        ValidatedResolutionV1::Maybe(value) => {
            resolve_explicit(store, deliberation, ResolutionV1::Maybe(value))
                .map_err(|_| ErrorV1::Resolution)?;
        }
        ValidatedResolutionV1::Mu(reason) => {
            resolve_explicit(store, deliberation, ResolutionV1::Mu(reason))
                .map_err(|_| ErrorV1::Resolution)?;
        }
    }
    Ok(())
}

fn resolution_matches(config: &ValidatedResolutionV1, existing: &ResolutionV1) -> bool {
    match (config, existing) {
        (ValidatedResolutionV1::ProvenBool(config), ResolutionV1::ProvenBool { validator, .. }) => {
            config.validator == *validator
        }
        (
            ValidatedResolutionV1::AuthorizedChoice {
                selected,
                aegis,
                subject,
            },
            ResolutionV1::AuthorizedChoice {
                selected: retained,
                authority,
                ..
            },
        ) => {
            selected == retained
                && authority.subject.0 == *subject
                && authority.epoch == aegis.expected_epoch
        }
        (ValidatedResolutionV1::Maybe(expected), ResolutionV1::Maybe(retained)) => {
            expected == retained
        }
        (ValidatedResolutionV1::Mu(expected), ResolutionV1::Mu(retained)) => expected == retained,
        _ => false,
    }
}

fn record_outcome<E>(summary: &mut SummaryV1, outcome: &MemberOutcomeV1<E>) {
    match outcome {
        MemberOutcomeV1::Report { .. } => summary.reports += 1,
        MemberOutcomeV1::InvalidReport { .. } => summary.invalid_reports += 1,
        MemberOutcomeV1::ToolContinuation { .. } => summary.tool_continuations += 1,
        MemberOutcomeV1::Uncertain { .. } => summary.uncertain += 1,
        MemberOutcomeV1::NonCompleted { .. } => summary.non_completed += 1,
        MemberOutcomeV1::Refused { .. } => summary.refused += 1,
        MemberOutcomeV1::Failed { .. } => summary.failed += 1,
    }
}

fn tool_workflow(
    deliberation: DeliberationIdV1,
    member: EnsembleMemberIdV1,
    source_execution: [u8; 16],
) -> moira_protocol::WorkflowIdV1 {
    moira_protocol::WorkflowIdV1(derived_identity(
        b"moira-methodical-tool-workflow-v1\0",
        &[&deliberation.0, &member.0, &source_execution],
    ))
}

fn resumed_run(
    base: &MemberRunV1,
    source_execution: [u8; 16],
    turn: u32,
    used_tool_calls: u32,
) -> MemberRunV1 {
    let mut resumed = base.clone();
    let turn = turn.to_be_bytes();
    resumed.execution.execution_id = derived_identity(
        b"moira-methodical-model-resume-v1\0",
        &[&base.member.0, &source_execution, &turn],
    );
    resumed.execution.idempotency_key = derived_identity(
        b"moira-methodical-model-resume-idempotency-v1\0",
        &[&base.member.0, &source_execution, &turn],
    );
    if used_tool_calls >= resumed.execution.budget.max_tool_calls {
        resumed.execution.tools.clear();
    }
    resumed
}

fn derived_identity(domain: &[u8], parts: &[&[u8]]) -> [u8; 16] {
    let mut hash = Sha256::new();
    hash.update(domain);
    for part in parts {
        hash.update(u64::try_from(part.len()).unwrap_or(u64::MAX).to_be_bytes());
        hash.update(part);
    }
    let digest: [u8; 32] = hash.finalize().into();
    let mut identity = [0_u8; 16];
    identity.copy_from_slice(&digest[..16]);
    identity[0] |= 0x80;
    identity
}

fn run_config(value: &RunConfigV1) -> Result<MemberRunV1, ErrorV1> {
    let budget = value.budget;
    if value.attempt_number == 0
        || budget.max_elapsed_millis == 0
        || budget.max_input_tokens == 0
        || budget.max_output_tokens == 0
        || budget.max_context_bytes == 0
        || budget.max_output_bytes == 0
        || value.context.len() > 64
        || value.tools.len() > 64
        || value.tools.is_empty() != (budget.max_tool_calls == 0)
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let context = value
        .context
        .iter()
        .map(|value| id(value))
        .collect::<Result<Vec<_>, _>>()?;
    let tools = value
        .tools
        .iter()
        .map(|value| id(value))
        .collect::<Result<Vec<_>, _>>()?;
    if context.iter().collect::<BTreeSet<_>>().len() != context.len()
        || tools.iter().collect::<BTreeSet<_>>().len() != tools.len()
    {
        return Err(ErrorV1::InvalidConfig);
    }
    Ok(MemberRunV1 {
        member: EnsembleMemberIdV1(id(&value.member)?),
        report: ReportIdV1(id(&value.report)?),
        execution: ExecutionIntentV1 {
            effect: EffectIdV1(id(&value.effect)?),
            execution_id: id(&value.execution)?,
            task: TaskRefV1 {
                external_task_key: id(&value.task_key)?,
                incarnation: value.task_incarnation,
            },
            attempt_number: value.attempt_number,
            principal: id(&value.principal)?,
            profile: id(&value.profile)?,
            idempotency_key: id(&value.idempotency_key)?,
            context,
            tools,
            budget: ExecutionBudgetV1 {
                max_elapsed_millis: budget.max_elapsed_millis,
                max_input_tokens: budget.max_input_tokens,
                max_output_tokens: budget.max_output_tokens,
                max_tool_calls: budget.max_tool_calls,
                max_context_bytes: budget.max_context_bytes,
                max_output_bytes: budget.max_output_bytes,
            },
        },
    })
}

fn validate_resolution(
    raw: Option<ResolutionConfigV1>,
    question_digest: [u8; 32],
    candidates: &[CandidateV1],
    now: u64,
) -> Result<Option<ValidatedResolutionV1>, ErrorV1> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let resolution = match raw {
        ResolutionConfigV1::ProvenBool {
            program,
            arguments,
            validator,
            max_output_bytes,
            timeout_ms,
        } => {
            if !program.is_absolute() {
                return Err(ErrorV1::InvalidConfig);
            }
            let config = RuneTheoremValidatorConfigV1 {
                program,
                arguments: arguments.into_iter().map(Into::into).collect(),
                expected_question_digest: question_digest,
                validator: id(&validator)?,
                max_output_bytes,
                timeout: Duration::from_millis(timeout_ms),
            };
            RuneTheoremValidatorV1::new(config.clone()).map_err(|_| ErrorV1::InvalidConfig)?;
            ValidatedResolutionV1::ProvenBool(config)
        }
        ResolutionConfigV1::AuthorizedChoice {
            selected_candidate,
            aegis,
        } => {
            let selected = CandidateIdV1(id(&selected_candidate)?);
            if !candidates.iter().any(|candidate| candidate.id == selected)
                || empty_path(&aegis.socket)
                || !(1..=300_000).contains(&aegis.timeout_ms)
                || aegis.expires_at <= now
            {
                return Err(ErrorV1::InvalidConfig);
            }
            let subject = id(&aegis.subject)?;
            ValidatedResolutionV1::AuthorizedChoice {
                selected,
                aegis,
                subject,
            }
        }
        ResolutionConfigV1::Maybe { value } => ValidatedResolutionV1::Maybe(match value {
            MaybeConfigV1::TrueOnly => MaybeValueV1::TrueOnly,
            MaybeConfigV1::FalseOnly => MaybeValueV1::FalseOnly,
            MaybeConfigV1::Both => MaybeValueV1::Both,
            MaybeConfigV1::Neither => MaybeValueV1::Neither,
        }),
        ResolutionConfigV1::Mu { reason } => ValidatedResolutionV1::Mu(match reason {
            MuConfigV1::FuelExhausted => MuReasonV1::FuelExhausted,
            MuConfigV1::MissingEvidence => MuReasonV1::MissingEvidence,
            MuConfigV1::ContradictoryEvidence => MuReasonV1::ContradictoryEvidence,
            MuConfigV1::WrongQuestion => MuReasonV1::WrongQuestion,
            MuConfigV1::WrongEnsemble => MuReasonV1::WrongEnsemble,
            MuConfigV1::NoAuthorizedSelector => MuReasonV1::NoAuthorizedSelector,
        }),
    };
    Ok(Some(resolution))
}

fn validate_tooling(
    raw: Option<ToolingConfigV1>,
    runs: &[MemberRunV1],
    now: u64,
    deliberation_path: &Path,
    deliberation_owner: [u8; 16],
) -> Result<Option<ValidatedToolingConfigV1>, ErrorV1> {
    let advertised = runs
        .iter()
        .flat_map(|run| run.execution.tools.iter().copied())
        .collect::<BTreeSet<_>>();
    let tool_tasks = runs
        .iter()
        .filter(|run| !run.execution.tools.is_empty())
        .map(|run| run.execution.task.external_task_key)
        .collect::<BTreeSet<_>>();
    let Some(raw) = raw else {
        return if advertised.is_empty() {
            Ok(None)
        } else {
            Err(ErrorV1::InvalidConfig)
        };
    };
    if advertised.is_empty()
        || raw.profiles.is_empty()
        || raw.profiles.len() > 64
        || raw.chronos.bindings.is_empty()
        || raw.chronos.bindings.len() > 64
        || !(1..=32).contains(&raw.max_turns)
        || !(1..=1_000).contains(&raw.poll_interval_ms)
        || !(1..=300_000).contains(&raw.aegis.timeout_ms)
        || raw.aegis.expires_at <= now
        || empty_path(&raw.aegis.socket)
        || empty_path(&raw.chronos.directory)
        || empty_path(&raw.continuation_store.path)
        || empty_path(&raw.output_store.path)
        || raw.continuation_store.path == raw.output_store.path
        || raw.continuation_store.path == deliberation_path
        || raw.output_store.path == deliberation_path
        || !(64..=1_048_576).contains(&raw.continuation_store.max_database_pages)
        || !(64..=1_048_576).contains(&raw.output_store.max_database_pages)
        || raw.output_store.max_entries == 0
        || raw.output_store.max_entries > 4_096
        || raw.output_store.max_output_bytes == 0
        || raw.output_store.max_output_bytes > 16 * 1_048_576
        || raw.budget.max_elapsed_millis == 0
        || raw.budget.max_elapsed_millis > 300_000
        || raw.budget.max_output_bytes == 0
        || raw.budget.max_output_bytes > 16 * 1_048_576
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let continuation_owner = id(&raw.continuation_store.owner)?;
    let output_owner = id(&raw.output_store.owner)?;
    let aegis_subject = id(&raw.aegis.subject)?;
    if continuation_owner == output_owner
        || continuation_owner == deliberation_owner
        || output_owner == deliberation_owner
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let mut profiles = Vec::new();
    let mut capabilities = BTreeSet::new();
    for profile in raw.profiles {
        let capability = id(&profile.capability)?;
        let argument_bytes = profile
            .arguments
            .iter()
            .try_fold(0_usize, |total, value| total.checked_add(value.len()))
            .ok_or(ErrorV1::InvalidConfig)?;
        if !capabilities.insert(capability)
            || !profile.executable.is_absolute()
            || !profile.working_directory.is_absolute()
            || profile.arguments.len() > 128
            || argument_bytes > 16 * 1_024
            || profile.max_elapsed_millis == 0
            || profile.max_elapsed_millis > 300_000
            || profile.max_input_bytes == 0
            || profile.max_input_bytes > 1_048_576
            || profile.max_output_bytes == 0
            || profile.max_output_bytes > 16 * 1_048_576
            || profile.max_stderr_bytes == 0
            || profile.max_stderr_bytes > 16 * 1_048_576
            || profile.max_executable_bytes == 0
            || profile.max_executable_bytes > 256 * 1_048_576
            || raw.budget.max_elapsed_millis > profile.max_elapsed_millis
            || raw.budget.max_output_bytes > profile.max_output_bytes
        {
            return Err(ErrorV1::InvalidConfig);
        }
        profiles.push((capability, profile));
    }
    if capabilities != advertised || raw.output_store.max_output_bytes < raw.budget.max_output_bytes
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let mut bindings = Vec::new();
    let mut bound_keys = BTreeSet::new();
    let mut task_ids = BTreeSet::new();
    for binding in raw.chronos.bindings.iter().cloned() {
        let key = id(&binding.task_key)?;
        if binding.task_id.is_empty()
            || binding.task_id.len() > 256
            || !bound_keys.insert(key)
            || !task_ids.insert(binding.task_id.clone())
        {
            return Err(ErrorV1::InvalidConfig);
        }
        bindings.push((key, binding.task_id));
    }
    if bound_keys != tool_tasks {
        return Err(ErrorV1::InvalidConfig);
    }
    let maximum_effects = runs
        .iter()
        .map(|run| usize::try_from(run.execution.budget.max_tool_calls).unwrap_or(usize::MAX))
        .try_fold(0_usize, usize::checked_add)
        .and_then(|per_turn| {
            per_turn.checked_mul(usize::try_from(raw.max_turns).unwrap_or(usize::MAX))
        })
        .ok_or(ErrorV1::InvalidConfig)?;
    let maximum_input_storage = maximum_effects
        .checked_mul(
            profiles
                .iter()
                .map(|(_, profile)| profile.max_input_bytes)
                .max()
                .ok_or(ErrorV1::InvalidConfig)?,
        )
        .ok_or(ErrorV1::InvalidConfig)?;
    if maximum_effects == 0
        || maximum_effects > 4_096
        || maximum_input_storage > 32 * 1_048_576
        || raw.output_store.max_entries < maximum_effects
    {
        return Err(ErrorV1::InvalidConfig);
    }
    Ok(Some(ValidatedToolingConfigV1 {
        continuation_store: raw.continuation_store,
        continuation_owner,
        output_store: raw.output_store,
        output_owner,
        aegis: raw.aegis,
        aegis_subject,
        chronos: raw.chronos,
        bindings,
        profiles,
        budget: raw.budget,
        max_turns: raw.max_turns,
        poll_interval: Duration::from_millis(raw.poll_interval_ms),
    }))
}

fn store_limits(max_database_pages: u32) -> DeliberationStoreLimitsV1 {
    DeliberationStoreLimitsV1 {
        codec: CodecLimitsV1 {
            max_bytes: 8 * 1_024 * 1_024,
            max_deliberation_bytes: 2 * 1_024 * 1_024,
            registry: RegistryLimitsV1 {
                max_deliberations: 128,
                deliberation: DeliberationLimitsV1::default(),
            },
        },
        max_database_pages,
    }
}

fn id(value: &str) -> Result<[u8; 16], ErrorV1> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let mut result = [0_u8; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        result[index] = (hex(pair[0])? << 4) | hex(pair[1])?;
    }
    Ok(result)
}

fn digest(value: &str) -> Result<[u8; 32], ErrorV1> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let mut result = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        result[index] = (hex(pair[0])? << 4) | hex(pair[1])?;
    }
    Ok(result)
}

fn hex(value: u8) -> Result<u8, ErrorV1> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(ErrorV1::InvalidConfig),
    }
}

fn unique<T: Ord>(values: &[MemberRunV1], key: impl Fn(&MemberRunV1) -> T) -> bool {
    values.iter().map(key).collect::<BTreeSet<_>>().len() == values.len()
}

fn empty_path(value: &Path) -> bool {
    value.as_os_str() == OsStr::new("")
}

const fn claim(value: ClaimConfigV1) -> ClaimKindV1 {
    match value {
        ClaimConfigV1::Formal => ClaimKindV1::Formal,
        ClaimConfigV1::Empirical => ClaimKindV1::Empirical,
        ClaimConfigV1::Normative => ClaimKindV1::Normative,
        ClaimConfigV1::Historical => ClaimKindV1::Historical,
        ClaimConfigV1::Creative => ClaimKindV1::Creative,
    }
}

const fn lens(value: LensConfigV1) -> PersonaLensV1 {
    match value {
        LensConfigV1::Historian => PersonaLensV1::Historian,
        LensConfigV1::IdeaStormer => PersonaLensV1::IdeaStormer,
        LensConfigV1::Theorist => PersonaLensV1::Theorist,
        LensConfigV1::Researcher => PersonaLensV1::Researcher,
        LensConfigV1::RigorReviewer => PersonaLensV1::RigorReviewer,
        LensConfigV1::ConvergenceFacilitator => PersonaLensV1::ConvergenceFacilitator,
    }
}

const fn depth(value: DepthConfigV1) -> SurfaceDepthV1 {
    match value {
        DepthConfigV1::Ambient => SurfaceDepthV1::Ambient,
        DepthConfigV1::Activated => SurfaceDepthV1::Activated,
        DepthConfigV1::Reference => SurfaceDepthV1::Reference,
    }
}

const fn default_timeout_ms() -> u64 {
    5_000
}

const fn default_database_pages() -> u32 {
    4_096
}

const fn default_tool_poll_ms() -> u64 {
    5
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis_authority::{EngineId, Limits as AegisLimits, PrincipalId, SessionId};
    use aegis_host::DurableAuthority;
    use aegis_local_daemon::unix::{Authenticator as AegisAuthenticator, Daemon as AegisDaemon};
    use aegis_service::{
        AuthenticatedSessionV1 as AegisSession, Service as AegisService,
        ServiceLimitsV1 as AegisServiceLimits,
    };
    use daimon_host::AuthenticatedSessionV1;
    use daimon_local_daemon::unix::{Authenticator, ModelDaemon, ModelSessionPort};
    use keystone_storage::{
        AcknowledgementStrength, Checkpoint, CheckpointStore, CommitOutcome, CommitReceipt,
        Revision, StoreRefusal,
    };
    use moira_core::deliberation::{
        CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
        DeliberationPlanV1, EnsembleMemberV1, GroundingRefV1, MemberReportV1, MethodRefV1,
        OrthogonalityEvidenceV1, registry::RegistryCommandV1,
    };
    use std::{
        convert::Infallible,
        io::{BufRead, BufReader},
        os::unix::fs::PermissionsExt,
        os::unix::net::UnixListener,
        os::unix::net::UnixStream,
        path::Path,
        sync::{
            Arc,
            atomic::{AtomicU64, AtomicUsize, Ordering},
        },
        thread,
    };

    static LIVE_NEXT: AtomicU64 = AtomicU64::new(0);
    const RESPONSE: &[u8] = br#"{"schema":"moira.member-response.v1","claims":[{"kind":"empirical","statement":"local model completed","evidence_digests":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"dissent_digests":["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]}]}"#;

    fn invocation_evidence() -> Option<Box<daimon_protocol::ModelInvocationEvidenceV1>> {
        Some(Box::new(daimon_protocol::ModelInvocationEvidenceV1 {
            provider: daimon_protocol::ProviderProfileIdV1::new([81; 16]),
            provider_document_digest: daimon_protocol::ContentDigestV1::new([82; 32]),
            model_digest: daimon_protocol::ContentDigestV1::new([83; 32]),
            context_manifest_digest: daimon_protocol::ContentDigestV1::new([84; 32]),
            message_manifest_digest: daimon_protocol::ContentDigestV1::new([85; 32]),
            evidence_scope_digest: daimon_protocol::ContentDigestV1::new([86; 32]),
        }))
    }

    struct CleanupDirectory(PathBuf);

    impl Drop for CleanupDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct AegisStore(Option<Checkpoint>);

    impl CheckpointStore for AegisStore {
        type Error = Infallible;

        fn load(&self, _: &str) -> Result<Option<Checkpoint>, Self::Error> {
            Ok(self.0.clone())
        }

        fn compare_exchange(
            &mut self,
            _: &str,
            expected: Option<Revision>,
            bytes: &[u8],
        ) -> Result<CommitOutcome, Self::Error> {
            let actual = self.0.as_ref().map(|checkpoint| checkpoint.revision);
            if actual != expected {
                return Ok(CommitOutcome::Refused(StoreRefusal::RevisionConflict {
                    expected,
                    actual,
                }));
            }
            let revision = Revision(actual.map_or(0, |value| value.0 + 1));
            self.0 = Some(Checkpoint {
                revision,
                bytes: bytes.to_vec(),
                strength: AcknowledgementStrength::Durable,
            });
            Ok(CommitOutcome::Committed(CommitReceipt {
                revision,
                mechanism: "methodical-live-tool",
                strength: AcknowledgementStrength::Durable,
            }))
        }
    }

    struct AegisAuth;

    impl AegisAuthenticator for AegisAuth {
        type Error = Infallible;

        fn authenticate(&mut self, _: &UnixStream) -> Result<AegisSession, Self::Error> {
            Ok(AegisSession {
                principal: PrincipalId::new([93; 16]),
                session: SessionId::new([94; 16]),
                administrator: true,
            })
        }
    }

    fn spawn_chronos_snapshot(directory: &Path, task_id: String) -> thread::JoinHandle<()> {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = chronos_local::socket_path(directory);
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut frame = Vec::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_until(b'\n', &mut frame)
                .unwrap();
            assert_eq!(frame.pop(), Some(b'\n'));
            let request =
                chronos_wire::decode_request(&frame, chronos_local::wire_limits()).unwrap();
            let chronos_wire::Command::Snapshot { task } = request.command else {
                panic!("snapshot expected")
            };
            assert_eq!(task, task_id);
            let response = chronos_wire::Response {
                version: 1,
                outcome: chronos_wire::Outcome::Snapshot {
                    value: Some(chronos_wire::Entry {
                        reference: chronos_wire::Reference {
                            task,
                            incarnation: "1".into(),
                        },
                        snapshot: chronos_wire::Snapshot {
                            phase: chronos_wire::Phase::Running,
                            attempt_number: Some(1),
                            dependency_count: 0,
                            required_capability_count: 1,
                            priority: 0,
                            max_attempts: 1,
                        },
                    }),
                    revision: Some(chronos_wire::Revision::Finite("1".into())),
                },
            };
            chronos_wire::write_frame(&mut stream, &response, chronos_local::MAX_RESPONSE_BYTES)
                .unwrap();
        })
    }

    struct LiveService(Arc<AtomicUsize>);

    impl daimon_local_daemon::ModelServicePort for LiveService {
        type Error = Infallible;

        fn execute_model(
            &mut self,
            _session: daimon_protocol::SessionHandleV1,
            command: daimon_protocol::ModelCommandV1,
        ) -> Result<daimon_protocol::ModelReplyV1, Self::Error> {
            let daimon_protocol::ModelCommandV1::Execute { request, .. } = command else {
                panic!("only execution is used")
            };
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(daimon_protocol::ModelReplyV1::Accepted(
                daimon_protocol::ModelOutcomeV1::Executed(
                    daimon_protocol::ModelExecutionResultV1 {
                        execution_id: request.execution_id,
                        disposition: daimon_protocol::ExecutionDispositionV1::Completed,
                        evidence_digest: daimon_protocol::ContentDigestV1::new([90; 32]),
                        invocation_evidence: invocation_evidence(),
                        input_tokens: 2,
                        output_tokens: 3,
                        output: RESPONSE.to_vec(),
                        proposals: Vec::new(),
                    },
                ),
            ))
        }
    }

    struct ToolLiveService(Arc<AtomicUsize>);

    impl daimon_local_daemon::ModelServicePort for ToolLiveService {
        type Error = Infallible;

        fn execute_model(
            &mut self,
            _session: daimon_protocol::SessionHandleV1,
            command: daimon_protocol::ModelCommandV1,
        ) -> Result<daimon_protocol::ModelReplyV1, Self::Error> {
            self.0.fetch_add(1, Ordering::Relaxed);
            let (execution_id, output, proposals) = match command {
                daimon_protocol::ModelCommandV1::Execute { request, .. }
                    if request.execution_id.into_bytes() == [0x16; 16] =>
                {
                    (
                        request.execution_id,
                        Vec::new(),
                        vec![daimon_protocol::OutputProposalV1::ToolCall(
                            daimon_protocol::ToolCallProposalV1 {
                                request: [0xab; 16],
                                capability: daimon_protocol::ToolCapabilityIdV1::new([0xaa; 16]),
                                provider_correlation: b"live-call-ab".to_vec(),
                                arguments: br#"{"query":"grounded architecture"}"#.to_vec(),
                            },
                        )],
                    )
                }
                daimon_protocol::ModelCommandV1::Execute { request, .. } => {
                    (request.execution_id, RESPONSE.to_vec(), Vec::new())
                }
                daimon_protocol::ModelCommandV1::Resume {
                    request,
                    source_execution,
                    results,
                    ..
                } => {
                    assert_eq!(source_execution.into_bytes(), [0x16; 16]);
                    assert_eq!(results.len(), 1);
                    assert_eq!(results[0].request, [0xab; 16]);
                    assert_eq!(results[0].output, br#"{"query":"grounded architecture"}"#);
                    (request.execution_id, RESPONSE.to_vec(), Vec::new())
                }
                daimon_protocol::ModelCommandV1::Inspect { .. } => {
                    panic!("inspection is not used")
                }
            };
            Ok(daimon_protocol::ModelReplyV1::Accepted(
                daimon_protocol::ModelOutcomeV1::Executed(
                    daimon_protocol::ModelExecutionResultV1 {
                        execution_id,
                        disposition: daimon_protocol::ExecutionDispositionV1::Completed,
                        evidence_digest: daimon_protocol::ContentDigestV1::new([95; 32]),
                        invocation_evidence: invocation_evidence(),
                        input_tokens: 2,
                        output_tokens: 3,
                        output,
                        proposals,
                    },
                ),
            ))
        }
    }

    impl ModelSessionPort for ToolLiveService {
        fn open_model_session(
            &mut self,
            _facts: AuthenticatedSessionV1,
        ) -> Result<daimon_protocol::SessionHandleV1, daimon_protocol::RefusalV1> {
            Ok(daimon_protocol::SessionHandleV1(96))
        }

        fn close_model_session(&mut self, session: daimon_protocol::SessionHandleV1) -> bool {
            session == daimon_protocol::SessionHandleV1(96)
        }
    }

    impl ModelSessionPort for LiveService {
        fn open_model_session(
            &mut self,
            _facts: AuthenticatedSessionV1,
        ) -> Result<daimon_protocol::SessionHandleV1, daimon_protocol::RefusalV1> {
            Ok(daimon_protocol::SessionHandleV1(91))
        }

        fn close_model_session(&mut self, session: daimon_protocol::SessionHandleV1) -> bool {
            session == daimon_protocol::SessionHandleV1(91)
        }
    }

    struct LiveAuth;

    impl Authenticator for LiveAuth {
        type Error = Infallible;

        fn authenticate(
            &mut self,
            _stream: &UnixStream,
        ) -> Result<AuthenticatedSessionV1, Self::Error> {
            Ok(AuthenticatedSessionV1 {
                principal: [64; 16],
                authority_context: [92; 16],
            })
        }
    }

    fn ready_resolution_store(path: &Path) -> DeliberationSqliteStore {
        let member = |value| EnsembleMemberV1 {
            id: EnsembleMemberIdV1([value; 16]),
            stakeholder: StakeholderRefV1::technical_sme([value; 16], [value; 32]),
            lens: PersonaLensV1::Theorist,
            concern_root_digest: [value; 32],
            grounding: GroundingRefV1 {
                authority_digest: [value + 1; 32],
                surface_digest: [value + 2; 32],
                depth: SurfaceDepthV1::Ambient,
            },
        };
        let plan = DeliberationPlanV1 {
            id: DeliberationIdV1([1; 16]),
            question_digest: [2; 32],
            method: MethodRefV1 {
                machine_digest: [3; 32],
                state_digest: [4; 32],
            },
            grounding_provenance: None,
            claims: vec![ClaimKindV1::Empirical],
            criteria: vec![CriterionV1 {
                id: CriterionIdV1([5; 16]),
                direction: CriterionDirectionV1::Maximize,
            }],
            candidates: vec![CandidateV1 {
                id: CandidateIdV1([6; 16]),
                scores: vec![1],
            }],
            admission: EnsembleAdmissionV1::affirmed(
                [12; 32], [13; 32], [14; 32], [15; 32], [16; 16],
            ),
            members: vec![member(7), member(8)],
            orthogonality: OrthogonalityEvidenceV1 {
                query_digest: [9; 32],
                result_digest: [10; 32],
                all_orthogonal: true,
            },
            fuel: 3,
        };
        let mut store = DeliberationSqliteStore::open(path, [11; 16], store_limits(64)).unwrap();
        store
            .apply(store.revision(), RegistryCommandV1::Admit(Box::new(plan)))
            .unwrap();
        for (report, member) in [(12, 7), (13, 8)] {
            store
                .apply(
                    store.revision(),
                    RegistryCommandV1::SubmitReport {
                        deliberation: DeliberationIdV1([1; 16]),
                        report: MemberReportV1 {
                            id: ReportIdV1([report; 16]),
                            member: EnsembleMemberIdV1([member; 16]),
                            output_digest: [report; 32],
                            evidence_digests: Vec::new(),
                            dissent_digests: Vec::new(),
                        },
                    },
                )
                .unwrap();
        }
        store
    }

    fn config() -> serde_json::Value {
        serde_json::json!({
            "dendrite": {"port": 4499},
            "daimon": {"socket": "/tmp/daimon-model.sock"},
            "store": {
                "path": "/tmp/moira-methodical.db",
                "owner": "01010101010101010101010101010101"
            },
            "plan": {
                "id": "02020202020202020202020202020202",
                "question": "Which architecture boundary should be reviewed?",
                "method_load": "rune/fsm/behavior-driven-development",
                "method_state": "discovery",
                "fold_load": "rune/sme/triggers",
                "fold_tags": ["architecture-flow-review"],
                "orthogonality_load": "axon/sme/axon-concerns",
                "claims": ["empirical"],
                "criteria": [{
                    "id": "03030303030303030303030303030303",
                    "direction": "maximize"
                }],
                "candidates": [{
                    "id": "04040404040404040404040404040404",
                    "scores": [1]
                }],
                "admission": {
                    "affected_population_digest": "1111111111111111111111111111111111111111111111111111111111111111",
                    "stakeholder_map_digest": "1212121212121212121212121212121212121212121212121212121212121212",
                    "coverage_manifest_digest": "1313131313131313131313131313131313131313131313131313131313131313",
                    "equity_assessment_digest": "1414141414141414141414141414141414141414141414141414141414141414",
                    "assessor_principal": "15151515151515151515151515151515",
                    "fitness": "yes"
                },
                "members": [
                    {
                        "id": "05050505050505050505050505050505",
                        "principal": "40404040404040404040404040404040",
                        "constituency_digest": "5151515151515151515151515151515151515151515151515151515151515151",
                        "representation": "technical_sme",
                        "lens": "theorist",
                        "fold_target": "rune:architecture",
                        "orthogonality_role": "rune:architecture",
                        "depth": "ambient"
                    },
                    {
                        "id": "06060606060606060606060606060606",
                        "principal": "40404040404040404040404040404040",
                        "constituency_digest": "5252525252525252525252525252525252525252525252525252525252525252",
                        "representation": "technical_sme",
                        "lens": "rigor_reviewer",
                        "fold_target": "rune:network",
                        "orthogonality_role": "rune:network",
                        "depth": "ambient"
                    },
                    {
                        "id": "07070707070707070707070707070707",
                        "principal": "40404040404040404040404040404040",
                        "constituency_digest": "5353535353535353535353535353535353535353535353535353535353535353",
                        "representation": "technical_sme",
                        "lens": "researcher",
                        "fold_target": "rune:identity",
                        "orthogonality_role": "rune:identity",
                        "depth": "ambient"
                    },
                    {
                        "id": "08080808080808080808080808080808",
                        "principal": "40404040404040404040404040404040",
                        "constituency_digest": "5454545454545454545454545454545454545454545454545454545454545454",
                        "representation": "technical_sme",
                        "lens": "historian",
                        "fold_target": "rune:data",
                        "orthogonality_role": "rune:data",
                        "depth": "ambient"
                    }
                ],
                "fuel": 5
            },
            "runs": [run(5, 20), run(6, 30), run(7, 40), run(8, 50)],
            "now": 10
        })
    }

    fn run(member: u8, seed: u8) -> serde_json::Value {
        let hex = |value: u8| format!("{value:02x}").repeat(16);
        serde_json::json!({
            "member": hex(member),
            "report": hex(seed),
            "effect": hex(seed + 1),
            "execution": hex(seed + 2),
            "task_key": hex(seed + 3),
            "task_incarnation": 1,
            "attempt_number": 1,
            "principal": "40404040404040404040404040404040",
            "profile": "41414141414141414141414141414141",
            "idempotency_key": hex(seed + 4),
            "context": [hex(seed + 5)],
            "tools": [],
            "budget": {
                "max_elapsed_millis": 1000,
                "max_input_tokens": 1024,
                "max_output_tokens": 512,
                "max_tool_calls": 0,
                "max_context_bytes": 65536,
                "max_output_bytes": 65536
            }
        })
    }

    fn tool_config() -> serde_json::Value {
        let mut value = config();
        value["product"] = serde_json::json!("agent");
        value["runs"][0]["tools"] = serde_json::json!(["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]);
        value["runs"][0]["budget"]["max_tool_calls"] = serde_json::json!(1);
        value["tooling"] = serde_json::json!({
            "continuation_store": {
                "path": "/tmp/moira-methodical-tools.db",
                "owner": "11111111111111111111111111111111"
            },
            "output_store": {
                "path": "/tmp/moira-methodical-tool-output.db",
                "owner": "12121212121212121212121212121212",
                "max_entries": 2,
                "max_output_bytes": 4096
            },
            "aegis": {
                "socket": "/tmp/aegis.sock",
                "subject": "13131313131313131313131313131313",
                "expected_epoch": 0,
                "expires_at": 20
            },
            "chronos": {
                "directory": "/tmp/chronos-private",
                "bindings": [{
                    "task_key": "17171717171717171717171717171717",
                    "task_id": "methodical-member-5"
                }]
            },
            "profiles": [{
                "capability": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "executable": "/bin/cat",
                "arguments": [],
                "working_directory": "/tmp",
                "max_elapsed_millis": 1000,
                "max_input_bytes": 4096,
                "max_output_bytes": 4096,
                "max_stderr_bytes": 1024,
                "max_executable_bytes": 2097152
            }],
            "budget": {
                "max_elapsed_millis": 500,
                "max_output_bytes": 4096
            },
            "max_turns": 2
        });
        value
    }

    fn chatbot_config() -> serde_json::Value {
        let mut value = config();
        value["product"] = serde_json::json!("chatbot");
        value["plan"]["method_state"] = serde_json::json!("formulation");
        value["plan"]["fold_tags"] = serde_json::json!(["test-oracle-design"]);
        value["plan"]["members"] = serde_json::json!([{
            "id": "05050505050505050505050505050505",
            "principal": "40404040404040404040404040404040",
            "constituency_digest": "5151515151515151515151515151515151515151515151515151515151515151",
            "representation": "technical_sme",
            "lens": "rigor_reviewer",
            "fold_target": "rune:tester",
            "orthogonality_role": "rune:tester",
            "depth": "ambient"
        }]);
        value["plan"]["fuel"] = serde_json::json!(1);
        value["runs"] = serde_json::Value::Array(vec![value["runs"][0].clone()]);
        value
    }

    fn peer_config() -> serde_json::Value {
        let mut value = config();
        value["resolution"] = serde_json::json!({
            "kind": "authorized_choice",
            "selected_candidate": "04040404040404040404040404040404",
            "aegis": {
                "socket": "/tmp/aegis-choice.sock",
                "subject": "31313131313131313131313131313131",
                "expected_epoch": 0,
                "expires_at": 20
            }
        });
        value["peer_action"] = serde_json::json!({
            "member": "05050505050505050505050505050505",
            "workflow": "32323232323232323232323232323232",
            "saga_store": {
                "path": "/tmp/moira-methodical-peer.db",
                "owner": "33333333333333333333333333333333",
                "max_sagas": 4,
                "max_proposals_per_saga": 4,
                "max_proposal_bytes": 4096
            },
            "aegis": {
                "socket": "/tmp/aegis-action.sock",
                "subject": "34343434343434343434343434343434",
                "expected_epoch": 0,
                "expires_at": 20
            },
            "agora": {
                "socket": "/tmp/agora.sock",
                "participant": "moira-methodical",
                "publisher_store": {
                    "path": "/tmp/moira-methodical-publisher.db",
                    "owner": "35353535353535353535353535353535"
                },
                "routes": [{
                    "room": "36363636363636363636363636363636",
                    "reply_to": "37373737373737373737373737373737",
                    "source": "38383838383838383838383838383838",
                    "room_sequence": 1,
                    "room_id": "methodical-room",
                    "expected_agora_revision": 1,
                    "logical_time": 10,
                    "addressed_to": ["operator"]
                }]
            },
            "vellum": {
                "socket": "/tmp/vellum.sock",
                "claim_env": "MOIRA_VELLUM_LOCAL_CLAIM",
                "max_replacement_bytes": 4096,
                "routes": [{
                    "workspace": "39393939393939393939393939393939",
                    "document": "3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a",
                    "vellum_workspace": 1,
                    "vellum_document": 2,
                    "source_revision": 3,
                    "source_extent": 128
                }]
            }
        });
        value
    }

    #[test]
    fn strict_configuration_validates_without_contacting_peers() {
        let parsed: ConfigV1 = serde_json::from_value(config()).unwrap();
        let validated = validate(parsed).unwrap();
        assert_eq!(validated.runs.len(), 4);
        assert_eq!(validated.plan.members.len(), 4);
    }

    #[test]
    fn resumed_run_stops_exposing_tools_when_budget_is_spent() {
        let parsed: ConfigV1 = serde_json::from_value(tool_config()).unwrap();
        let validated = validate(parsed).unwrap();
        let source = &validated.runs[0];

        let resumed = resumed_run(source, source.execution.execution_id, 1, 1);

        assert_eq!(resumed.execution.budget.max_tool_calls, 1);
        assert!(resumed.execution.tools.is_empty());
    }

    #[test]
    fn peer_action_requires_authorized_resolution_and_distinct_durable_owners() {
        let validated = validate(serde_json::from_value(peer_config()).unwrap()).unwrap();
        assert!(validated.peer_action.is_some());

        let mut unresolved = peer_config();
        unresolved.as_object_mut().unwrap().remove("resolution");
        assert_eq!(
            validate(serde_json::from_value(unresolved).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );

        let mut shared_owner = peer_config();
        shared_owner["peer_action"]["saga_store"]["owner"] = shared_owner["store"]["owner"].clone();
        assert_eq!(
            validate(serde_json::from_value(shared_owner).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );
    }

    #[test]
    fn product_profiles_preserve_chatbot_workflow_and_tool_agent_boundaries() {
        let mut chatbot = config();
        chatbot["product"] = serde_json::json!("chatbot");
        chatbot["plan"]["members"] =
            serde_json::Value::Array(vec![chatbot["plan"]["members"][0].clone()]);
        chatbot["runs"] = serde_json::Value::Array(vec![chatbot["runs"][0].clone()]);
        assert!(validate(serde_json::from_value(chatbot.clone()).unwrap()).is_ok());

        let mut implicit_workflow = chatbot.clone();
        implicit_workflow.as_object_mut().unwrap().remove("product");
        assert_eq!(
            validate(serde_json::from_value(implicit_workflow).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );

        let mut agent = tool_config();
        agent["product"] = serde_json::json!("agent");
        assert!(validate(serde_json::from_value(agent).unwrap()).is_ok());

        let mut tool_free_agent = config();
        tool_free_agent["product"] = serde_json::json!("agent");
        assert_eq!(
            validate(serde_json::from_value(tool_free_agent).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );

        chatbot["peer_action"] = peer_config()["peer_action"].clone();
        chatbot["peer_action"]
            .as_object_mut()
            .unwrap()
            .remove("aegis");
        chatbot["peer_action"]["selection"] = serde_json::json!({
            "kind": "single_member",
            "policy_digest": "42".repeat(32)
        });
        assert!(validate(serde_json::from_value(chatbot.clone()).unwrap()).is_ok());

        chatbot["peer_action"]["selection"]["policy_digest"] = serde_json::json!("00".repeat(32));
        assert_eq!(
            validate(serde_json::from_value(chatbot).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );
    }

    #[test]
    fn resolution_configuration_preserves_all_four_terminal_families() {
        let mut maybe = config();
        maybe["resolution"] = serde_json::json!({"kind": "maybe", "value": "both"});
        assert!(matches!(
            validate(serde_json::from_value(maybe).unwrap())
                .unwrap()
                .resolution,
            Some(ValidatedResolutionV1::Maybe(MaybeValueV1::Both))
        ));

        let mut mu = config();
        mu["resolution"] = serde_json::json!({"kind": "mu", "reason": "missing_evidence"});
        assert!(matches!(
            validate(serde_json::from_value(mu).unwrap())
                .unwrap()
                .resolution,
            Some(ValidatedResolutionV1::Mu(MuReasonV1::MissingEvidence))
        ));

        let mut proof = config();
        proof["resolution"] = serde_json::json!({
            "kind": "proven_bool",
            "program": "/bin/true",
            "arguments": [],
            "validator": "31313131313131313131313131313131",
            "max_output_bytes": 4096,
            "timeout_ms": 1000
        });
        assert!(matches!(
            validate(serde_json::from_value(proof).unwrap())
                .unwrap()
                .resolution,
            Some(ValidatedResolutionV1::ProvenBool(_))
        ));

        let mut choice = config();
        choice["resolution"] = serde_json::json!({
            "kind": "authorized_choice",
            "selected_candidate": "04040404040404040404040404040404",
            "aegis": {
                "socket": "/tmp/aegis.sock",
                "subject": "32323232323232323232323232323232",
                "expected_epoch": 0,
                "expires_at": 20
            }
        });
        assert!(matches!(
            validate(serde_json::from_value(choice.clone()).unwrap())
                .unwrap()
                .resolution,
            Some(ValidatedResolutionV1::AuthorizedChoice { .. })
        ));
        choice["resolution"]["selected_candidate"] =
            serde_json::json!("99999999999999999999999999999999");
        assert_eq!(
            validate(serde_json::from_value(choice).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );
    }

    #[test]
    fn explicit_operator_resolution_is_durable_replay_safe_and_conflict_strict() {
        let directory = std::env::temp_dir().join(format!(
            "moira-methodical-resolution-{}-{}",
            std::process::id(),
            LIVE_NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let mut store = ready_resolution_store(&directory.join("resolution.db"));

        apply_resolution(
            &mut store,
            DeliberationIdV1([1; 16]),
            ValidatedResolutionV1::Maybe(MaybeValueV1::Both),
            10,
        )
        .unwrap();
        assert_eq!(store.revision().0, 4);
        drop(store);

        let mut reopened = DeliberationSqliteStore::open(
            directory.join("resolution.db"),
            [11; 16],
            store_limits(64),
        )
        .unwrap();
        apply_resolution(
            &mut reopened,
            DeliberationIdV1([1; 16]),
            ValidatedResolutionV1::Maybe(MaybeValueV1::Both),
            10,
        )
        .unwrap();
        assert_eq!(reopened.revision().0, 4);
        assert_eq!(
            apply_resolution(
                &mut reopened,
                DeliberationIdV1([1; 16]),
                ValidatedResolutionV1::Mu(MuReasonV1::MissingEvidence),
                10,
            ),
            Err(ErrorV1::Resolution)
        );
        assert_eq!(reopened.revision().0, 4);
        drop(reopened);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "requires local Unix socket support"]
    fn operator_authorized_choice_crosses_aegis_once_and_replays_offline() {
        let directory = PathBuf::from(format!(
            "/tmp/mmr-{}-{}",
            std::process::id(),
            LIVE_NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let cleanup = CleanupDirectory(directory.clone());
        let socket = directory.join("aegis.sock");
        let authority_id = EngineId::new([71; 16]);
        let authority = DurableAuthority::create_new(
            AegisStore::default(),
            authority_id,
            AegisLimits::default(),
        )
        .unwrap();
        let service =
            AegisService::new(authority, authority_id, AegisServiceLimits::default()).unwrap();
        let mut daemon = AegisDaemon::bind_new(
            &socket,
            service,
            AegisAuth,
            aegis_wire::LimitsV1::default(),
            Duration::from_secs(2),
            4,
        )
        .unwrap();
        let server = thread::spawn(move || daemon.serve_next().unwrap());
        let mut store = ready_resolution_store(&directory.join("choice.db"));
        let resolution = || ValidatedResolutionV1::AuthorizedChoice {
            selected: CandidateIdV1([6; 16]),
            aegis: AegisResolutionConfigV1 {
                socket: socket.clone(),
                subject: "48484848484848484848484848484848".into(),
                expected_epoch: 0,
                expires_at: 20,
                timeout_ms: 2_000,
            },
            subject: [72; 16],
        };

        apply_resolution(&mut store, DeliberationIdV1([1; 16]), resolution(), 10).unwrap();
        server.join().unwrap();
        assert!(matches!(
            store
                .deliberation(DeliberationIdV1([1; 16]))
                .unwrap()
                .resolution,
            Some(ResolutionV1::AuthorizedChoice {
                selected,
                ..
            }) if selected == CandidateIdV1([6; 16])
        ));
        assert_eq!(store.revision().0, 4);

        apply_resolution(&mut store, DeliberationIdV1([1; 16]), resolution(), 10).unwrap();
        assert_eq!(store.revision().0, 4);
        drop(store);
        drop(cleanup);
    }

    #[test]
    fn tool_configuration_is_closed_and_requires_exact_advertised_capabilities_and_tasks() {
        let validated = validate(serde_json::from_value(tool_config()).unwrap()).unwrap();
        assert!(validated.tooling.is_some());

        let mut missing = tool_config();
        missing.as_object_mut().unwrap().remove("tooling");
        assert_eq!(
            validate(serde_json::from_value(missing).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );

        let mut extra_profile = tool_config();
        extra_profile["tooling"]["profiles"][0]["capability"] =
            serde_json::json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert_eq!(
            validate(serde_json::from_value(extra_profile).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );

        let mut wrong_task = tool_config();
        wrong_task["tooling"]["chronos"]["bindings"][0]["task_key"] =
            serde_json::json!("cccccccccccccccccccccccccccccccc");
        assert_eq!(
            validate(serde_json::from_value(wrong_task).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );
    }

    #[test]
    fn tool_runtime_opens_local_profiles_and_stores_without_contacting_authority_or_time() {
        let directory = std::env::temp_dir().join(format!(
            "moira-methodical-tools-{}-{}",
            std::process::id(),
            LIVE_NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let mut raw = tool_config();
        raw["tooling"]["continuation_store"]["path"] =
            serde_json::json!(directory.join("continuation.db"));
        raw["tooling"]["output_store"]["path"] = serde_json::json!(directory.join("outputs.db"));
        raw["tooling"]["profiles"][0]["working_directory"] = serde_json::json!(&directory);
        raw["tooling"]["profiles"][0]["executable"] =
            serde_json::json!(directory.join("absent-tool"));
        raw["tooling"]["aegis"]["socket"] = serde_json::json!(directory.join("absent-aegis.sock"));
        raw["tooling"]["chronos"]["directory"] =
            serde_json::json!(directory.join("absent-chronos"));
        let validated = validate(serde_json::from_value(raw).unwrap()).unwrap();
        let runtime =
            ToolRuntimeV1::open(validated.tooling.unwrap(), &validated.runs, validated.now)
                .unwrap();
        assert_eq!(runtime.storage_revision, None);
        assert!(runtime.authority.connected.is_none());
        assert!(runtime.executor.connected.is_none());
        drop(runtime);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unknown_fields_and_uppercase_or_duplicate_identities_refuse() {
        let mut unknown = config();
        unknown["surprise"] = serde_json::Value::Bool(true);
        assert!(serde_json::from_value::<ConfigV1>(unknown).is_err());

        let mut uppercase = config();
        uppercase["store"]["owner"] = serde_json::Value::String("AA".repeat(16));
        assert_eq!(
            validate(serde_json::from_value(uppercase).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );

        let mut duplicate = config();
        duplicate["runs"][1]["execution"] = duplicate["runs"][0]["execution"].clone();
        assert_eq!(
            validate(serde_json::from_value(duplicate).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );
    }

    #[test]
    fn review_request_cannot_share_a_run_with_resolution() {
        let mut value = config();
        value["begin_review"] = serde_json::json!(true);
        value["resolution"] = serde_json::json!({"kind": "maybe", "value": "both"});
        assert_eq!(
            validate(serde_json::from_value(value).unwrap()).unwrap_err(),
            ErrorV1::InvalidConfig
        );
    }

    #[test]
    fn summary_is_content_free_and_stably_named() {
        let summary = SummaryV1 {
            cohort_members: 2,
            distinct_principals: 1,
            distinct_profiles: 1,
            distinct_context_manifests: 2,
            shared_principal: true,
            shared_profile: true,
            shared_context_manifest: false,
            independence: IndependenceAssessmentV1::Correlated,
            revision: 3,
            reports: 2,
            invalid_reports: 0,
            tool_continuations: 0,
            uncertain: 0,
            non_completed: 0,
            refused: 0,
            failed: 0,
            ready_to_resolve: true,
            resolved: false,
            review_begun: false,
            peer_completed: false,
            peer_uncertain: false,
            peer_failed: false,
        };
        assert_eq!(
            summary.json(),
            r#"{"cohort_members":2,"distinct_context_manifests":2,"distinct_principals":1,"distinct_profiles":1,"failed":0,"independence":"correlated","invalid_reports":0,"non_completed":0,"peer_completed":false,"peer_failed":false,"peer_uncertain":false,"ready_to_resolve":true,"refused":0,"reports":2,"resolved":false,"review_begun":false,"revision":3,"shared_context_manifest":false,"shared_principal":true,"shared_profile":true,"tool_continuations":0,"uncertain":0}"#
        );
    }

    #[test]
    fn live_real_corpus_to_private_model_socket_to_sqlite_journey() {
        let Some(port) = std::env::var_os("MOIRA_METHODICAL_LIVE_DENDRITE_PORT") else {
            return;
        };
        let directory = std::env::temp_dir().join(format!(
            "moira-methodical-app-{}-{}",
            std::process::id(),
            LIVE_NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let socket = directory.join("daimon.sock");
        let database = directory.join("moira.db");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut daemon = ModelDaemon::bind_new(
            &socket,
            LiveService(Arc::clone(&calls)),
            LiveAuth,
            daimon_wire::LimitsV1::default(),
            daimon_wire::ModelLimitsV1::default(),
            Duration::from_secs(2),
            4,
        )
        .unwrap();
        let server = thread::spawn(move || daemon.serve_next().unwrap());
        let mut raw = config();
        raw["dendrite"]["port"] = serde_json::json!(port.to_string_lossy().parse::<u16>().unwrap());
        if std::env::var_os("MOIRA_DENDRITE_BEARER").is_some() {
            raw["dendrite"]["bearer_token_env"] = serde_json::json!("MOIRA_DENDRITE_BEARER");
        }
        raw["daimon"]["socket"] = serde_json::json!(socket);
        raw["store"]["path"] = serde_json::json!(database);
        raw["begin_review"] = serde_json::json!(true);
        let summary = execute(validate(serde_json::from_value(raw).unwrap()).unwrap()).unwrap();
        assert_eq!(summary.revision, 6);
        assert_eq!(summary.reports, 4);
        assert!(summary.review_begun);
        assert!(!summary.ready_to_resolve);
        assert_eq!(calls.load(Ordering::Relaxed), 4);
        let reopened = DeliberationSqliteStore::open(
            &database,
            [1; 16],
            store_limits(default_database_pages()),
        )
        .unwrap();
        let review = reopened.review(DeliberationIdV1([2; 16])).unwrap();
        assert_eq!(
            review.correlation.first_pass.committed_before_cross_review,
            4
        );
        assert_eq!(review.correlation.first_pass.exposed_before_commit, 0);
        assert_eq!(review.correlation.first_pass.unknown, 0);
        for dimension in [
            CorrelationDimensionV1::Model,
            CorrelationDimensionV1::Provider,
            CorrelationDimensionV1::Prompt,
            CorrelationDimensionV1::EvidenceScope,
        ] {
            let evidence = review.correlation.dimension(dimension).unwrap();
            assert_eq!(evidence.observed_members, 4);
            assert_eq!(evidence.distinct_values, 1);
        }
        drop(reopened);
        server.join().unwrap();
        for name in ["moira.db", "moira.db-journal", "moira.db.owner.lock"] {
            let _ = fs::remove_file(directory.join(name));
        }
        let _ = fs::remove_dir(directory);
    }

    #[test]
    fn live_grounded_chatbot_profile_uses_one_authored_expert_and_private_model_socket() {
        let Some(port) = std::env::var_os("MOIRA_METHODICAL_LIVE_DENDRITE_PORT") else {
            return;
        };
        let directory = std::env::temp_dir().join(format!(
            "moira-methodical-chatbot-{}-{}",
            std::process::id(),
            LIVE_NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let cleanup = CleanupDirectory(directory.clone());
        let socket = directory.join("daimon.sock");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut daemon = ModelDaemon::bind_new(
            &socket,
            LiveService(Arc::clone(&calls)),
            LiveAuth,
            daimon_wire::LimitsV1::default(),
            daimon_wire::ModelLimitsV1::default(),
            Duration::from_secs(2),
            2,
        )
        .unwrap();
        let server = thread::spawn(move || daemon.serve_next().unwrap());
        let mut raw = chatbot_config();
        raw["dendrite"]["port"] = serde_json::json!(port.to_string_lossy().parse::<u16>().unwrap());
        if std::env::var_os("MOIRA_DENDRITE_BEARER").is_some() {
            raw["dendrite"]["bearer_token_env"] = serde_json::json!("MOIRA_DENDRITE_BEARER");
        }
        raw["daimon"]["socket"] = serde_json::json!(socket);
        raw["store"]["path"] = serde_json::json!(directory.join("deliberation.db"));

        let summary = execute(validate(serde_json::from_value(raw).unwrap()).unwrap()).unwrap();
        assert_eq!(summary.revision, 2);
        assert_eq!(summary.reports, 1);
        assert_eq!(summary.tool_continuations, 0);
        assert!(summary.ready_to_resolve);
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        server.join().unwrap();
        drop(cleanup);
    }

    #[test]
    fn live_grounded_tool_turn_crosses_aegis_chronos_daimon_and_durable_resume() {
        let Some(port) = std::env::var_os("MOIRA_METHODICAL_LIVE_DENDRITE_PORT") else {
            return;
        };
        let directory = PathBuf::from(format!(
            "/private/tmp/mm-tool-{}-{}",
            std::process::id(),
            LIVE_NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let chronos_directory = directory.join("chronos");
        fs::create_dir_all(&chronos_directory).unwrap();
        let cleanup = CleanupDirectory(directory.clone());

        let aegis_socket = directory.join("aegis.sock");
        let authority_id = EngineId::new([97; 16]);
        let authority = DurableAuthority::create_new(
            AegisStore::default(),
            authority_id,
            AegisLimits::default(),
        )
        .unwrap();
        let aegis =
            AegisService::new(authority, authority_id, AegisServiceLimits::default()).unwrap();
        let mut aegis_daemon = AegisDaemon::bind_new(
            &aegis_socket,
            aegis,
            AegisAuth,
            aegis_wire::LimitsV1::default(),
            Duration::from_secs(2),
            4,
        )
        .unwrap();
        let aegis_server = thread::spawn(move || aegis_daemon.serve_next().unwrap());
        let chronos_server =
            spawn_chronos_snapshot(&chronos_directory, "methodical-member-5".into());

        let daimon_socket = directory.join("daimon.sock");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut daimon = ModelDaemon::bind_new(
            &daimon_socket,
            ToolLiveService(Arc::clone(&calls)),
            LiveAuth,
            daimon_wire::LimitsV1::default(),
            daimon_wire::ModelLimitsV1::default(),
            Duration::from_secs(2),
            8,
        )
        .unwrap();
        let daimon_server = thread::spawn(move || daimon.serve_next().unwrap());

        let mut raw = tool_config();
        raw["dendrite"]["port"] = serde_json::json!(port.to_string_lossy().parse::<u16>().unwrap());
        if std::env::var_os("MOIRA_DENDRITE_BEARER").is_some() {
            raw["dendrite"]["bearer_token_env"] = serde_json::json!("MOIRA_DENDRITE_BEARER");
        }
        raw["daimon"]["socket"] = serde_json::json!(daimon_socket);
        let database = directory.join("deliberation.db");
        raw["store"]["path"] = serde_json::json!(&database);
        raw["begin_review"] = serde_json::json!(true);
        raw["tooling"]["continuation_store"]["path"] =
            serde_json::json!(directory.join("continuation.db"));
        raw["tooling"]["output_store"]["path"] = serde_json::json!(directory.join("outputs.db"));
        raw["tooling"]["aegis"]["socket"] = serde_json::json!(aegis_socket);
        raw["tooling"]["chronos"]["directory"] = serde_json::json!(chronos_directory);
        raw["tooling"]["profiles"][0]["working_directory"] = serde_json::json!(&directory);

        let summary = execute(validate(serde_json::from_value(raw).unwrap()).unwrap()).unwrap();
        assert_eq!(summary.revision, 6);
        assert_eq!(summary.reports, 4);
        assert_eq!(summary.tool_continuations, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.uncertain, 0);
        assert!(summary.review_begun);
        assert!(!summary.ready_to_resolve);
        assert_eq!(calls.load(Ordering::Relaxed), 5);

        let reopened = DeliberationSqliteStore::open(
            &database,
            [1; 16],
            store_limits(default_database_pages()),
        )
        .unwrap();
        let review = reopened.review(DeliberationIdV1([2; 16])).unwrap();
        assert_eq!(
            review.correlation.first_pass.committed_before_cross_review,
            4
        );
        assert_eq!(review.correlation.first_pass.unknown, 0);
        assert_eq!(
            review
                .correlation
                .dimension(CorrelationDimensionV1::Provider)
                .unwrap()
                .observed_members,
            4
        );
        drop(reopened);

        daimon_server.join().unwrap();
        aegis_server.join().unwrap();
        chronos_server.join().unwrap();
        drop(cleanup);
    }
}
