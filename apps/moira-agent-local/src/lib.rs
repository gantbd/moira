//! Strict one-step composition for a governed local agent participation.
//!
//! The participation daemon remains the sole owner of prompt storage. A
//! worker materializes an exact pending prompt over its separately authorized
//! command, closes that connection, and reconnects before fencing dispatch.

#![forbid(unsafe_code)]

use daimon_wire::{LimitsV1 as DaimonLimitsV1, agent::AgentWireLimitsV1};
use moira_client::unix::ParticipationClientLimitsV1;
use moira_local_harness::{
    agent_runtime::{
        AgentDriveOutcomeV1, AgentWorkerOutcomeV1, FixedAgentCommandPolicyV1,
        FixedAgentCommandTemplateV1, RetainedPromptMaterializerV1, drive_current, load_prepared,
        materialize_pending_prompt,
    },
    unix::{connect_daimon_agent, connect_participation},
};
use moira_protocol::{agent, agent_participation as participation};
use moira_sqlite::agent_output::{AgentOutputSqliteStore, AgentOutputStoreLimitsV1};
use serde::Deserialize;
use std::{fmt, fs, path::PathBuf, time::Duration};

const MAX_CONFIG_BYTES: u64 = 1_048_576;
const MAX_CONTEXT_OR_TOOLS: usize = 64;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigV1 {
    pub participation_socket: PathBuf,
    pub daimon_agent_socket: PathBuf,
    pub participation_id: String,
    pub output: OutputConfigV1,
    pub policy: PolicyConfigV1,
    pub transport: TransportConfigV1,
    pub now: u64,
    pub max_event_observations: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfigV1 {
    pub path: PathBuf,
    pub owner: String,
    pub max_entries: usize,
    pub max_output_bytes: usize,
    pub max_page_assistant_bytes: usize,
    pub max_database_pages: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfigV1 {
    pub principal: String,
    pub workspace: String,
    #[serde(default)]
    pub context: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    pub execution_budget: ExecutionBudgetConfigV1,
    pub session_budget: SessionBudgetConfigV1,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBudgetConfigV1 {
    pub max_elapsed_millis: u64,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_context_bytes: usize,
    pub max_output_bytes: usize,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionBudgetConfigV1 {
    pub max_elapsed_millis: u64,
    pub max_turns: u32,
    pub max_events: u64,
    pub max_event_bytes: usize,
    pub max_prompt_bytes: usize,
    pub max_permission_requests: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportConfigV1 {
    pub participation_max_frame_bytes: usize,
    pub participation_max_requests: usize,
    pub daimon_handshake_max_frame_bytes: usize,
    pub agent_max_frame_bytes: usize,
    pub timeout_ms: u64,
}

pub struct ValidatedConfigV1 {
    participation_socket: PathBuf,
    daimon_agent_socket: PathBuf,
    participation: participation::ParticipationIdV1,
    output_path: PathBuf,
    output_owner: [u8; 16],
    output_limits: AgentOutputStoreLimitsV1,
    policy: FixedAgentCommandPolicyV1,
    participation_limits: ParticipationClientLimitsV1,
    handshake_limits: DaimonLimitsV1,
    agent_limits: AgentWireLimitsV1,
    timeout: Duration,
    now: u64,
    max_event_observations: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorV1 {
    ConfigIo,
    ConfigTooLarge,
    ConfigJson,
    InvalidConfig,
    Participation,
    PromptMaterialization,
    Daimon,
    OutputStore,
    Dispatch,
}

impl fmt::Display for ErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "moira agent local: {self:?}")
    }
}

impl std::error::Error for ErrorV1 {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SummaryV1 {
    Idle {
        participation: participation::ParticipationIdV1,
        revision: u64,
    },
    Observed {
        participation: participation::ParticipationIdV1,
        revision: u64,
    },
    Fenced {
        participation: participation::ParticipationIdV1,
        revision: u64,
    },
}

impl SummaryV1 {
    pub fn json(self) -> String {
        match self {
            Self::Idle {
                participation,
                revision,
            } => {
                format!(
                    r#"{{"outcome":"idle","participation":"{}","participation_revision":{revision}}}"#,
                    hex(&participation.0)
                )
            }
            Self::Observed {
                participation,
                revision,
            } => {
                format!(
                    r#"{{"outcome":"observed","participation":"{}","participation_revision":{revision}}}"#,
                    hex(&participation.0)
                )
            }
            Self::Fenced {
                participation,
                revision,
            } => {
                format!(
                    r#"{{"outcome":"fenced_unknown","participation":"{}","participation_revision":{revision}}}"#,
                    hex(&participation.0)
                )
            }
        }
    }
}

pub fn load(path: impl AsRef<std::path::Path>) -> Result<ConfigV1, ErrorV1> {
    let metadata = fs::metadata(path.as_ref()).map_err(|_| ErrorV1::ConfigIo)?;
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(ErrorV1::ConfigTooLarge);
    }
    let bytes = fs::read(path).map_err(|_| ErrorV1::ConfigIo)?;
    serde_json::from_slice(&bytes).map_err(|_| ErrorV1::ConfigJson)
}

pub fn validate(config: ConfigV1) -> Result<ValidatedConfigV1, ErrorV1> {
    let participation = participation::ParticipationIdV1(hex_id(&config.participation_id)?);
    let output_owner = hex_id(&config.output.owner)?;
    let principal = agent::PrincipalIdV1(hex_id(&config.policy.principal)?);
    let workspace = hex_id(&config.policy.workspace)?;
    if config.participation_socket.as_os_str().is_empty()
        || config.daimon_agent_socket.as_os_str().is_empty()
        || config.output.path.as_os_str().is_empty()
        || config.output.max_entries == 0
        || config.output.max_output_bytes == 0
        || config.output.max_page_assistant_bytes == 0
        || config.output.max_database_pages == 0
        || config.transport.participation_max_frame_bytes == 0
        || config.transport.participation_max_requests < 4
        || config.transport.daimon_handshake_max_frame_bytes == 0
        || config.transport.agent_max_frame_bytes == 0
        || config.transport.timeout_ms == 0
        || config.max_event_observations == 0
        || config.policy.context.len() > MAX_CONTEXT_OR_TOOLS
        || config.policy.tools.len() > MAX_CONTEXT_OR_TOOLS
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let context = config
        .policy
        .context
        .iter()
        .map(|value| hex_id(value).map(agent::ContextIdV1))
        .collect::<Result<Vec<_>, _>>()?;
    let tools = config
        .policy
        .tools
        .iter()
        .map(|value| hex_id(value).map(agent::ToolIdV1))
        .collect::<Result<Vec<_>, _>>()?;
    let execution_budget = agent::ExecutionBudgetV1 {
        max_elapsed_millis: config.policy.execution_budget.max_elapsed_millis,
        max_input_tokens: config.policy.execution_budget.max_input_tokens,
        max_output_tokens: config.policy.execution_budget.max_output_tokens,
        max_tool_calls: config.policy.execution_budget.max_tool_calls,
        max_context_bytes: config.policy.execution_budget.max_context_bytes,
        max_output_bytes: config.policy.execution_budget.max_output_bytes,
    };
    let session_budget = agent::SessionBudgetV1 {
        max_elapsed_millis: config.policy.session_budget.max_elapsed_millis,
        max_turns: config.policy.session_budget.max_turns,
        max_events: config.policy.session_budget.max_events,
        max_event_bytes: config.policy.session_budget.max_event_bytes,
        max_prompt_bytes: config.policy.session_budget.max_prompt_bytes,
        max_permission_requests: config.policy.session_budget.max_permission_requests,
    };
    let policy = FixedAgentCommandPolicyV1 {
        principal,
        workspace,
        context,
        tools,
        execution_budget,
        session_budget,
    };
    FixedAgentCommandTemplateV1::new(policy.clone()).map_err(|_| ErrorV1::InvalidConfig)?;
    let timeout = Duration::from_millis(config.transport.timeout_ms);
    let max_events =
        usize::try_from(config.max_event_observations).map_err(|_| ErrorV1::InvalidConfig)?;
    Ok(ValidatedConfigV1 {
        participation_socket: config.participation_socket,
        daimon_agent_socket: config.daimon_agent_socket,
        participation,
        output_path: config.output.path,
        output_owner,
        output_limits: AgentOutputStoreLimitsV1 {
            max_entries: config.output.max_entries,
            max_output_bytes: config.output.max_output_bytes,
            max_page_assistant_bytes: config.output.max_page_assistant_bytes,
            max_database_pages: config.output.max_database_pages,
        },
        policy,
        participation_limits: ParticipationClientLimitsV1 {
            max_frame_bytes: config.transport.participation_max_frame_bytes,
            max_requests: config.transport.participation_max_requests,
            io_timeout: timeout,
        },
        handshake_limits: DaimonLimitsV1 {
            max_frame_bytes: config.transport.daimon_handshake_max_frame_bytes,
            max_context_references: MAX_CONTEXT_OR_TOOLS,
            max_tools: MAX_CONTEXT_OR_TOOLS,
            max_artifacts: 1,
        },
        agent_limits: AgentWireLimitsV1 {
            max_frame_bytes: config.transport.agent_max_frame_bytes,
            max_context_references: MAX_CONTEXT_OR_TOOLS,
            max_tools: MAX_CONTEXT_OR_TOOLS,
            max_prompt_bytes: session_budget.max_prompt_bytes,
            max_events,
            max_event_bytes: session_budget.max_event_bytes,
            max_permission_options: 8,
        },
        timeout,
        now: config.now,
        max_event_observations: config.max_event_observations,
    })
}

pub fn execute(config: ValidatedConfigV1) -> Result<SummaryV1, ErrorV1> {
    let mut first =
        connect_participation(&config.participation_socket, config.participation_limits)
            .map_err(|_| ErrorV1::Participation)?;
    let prepared =
        load_prepared(&mut first, config.participation).map_err(|_| ErrorV1::Participation)?;
    let prompt = materialize_pending_prompt(&mut first, &prepared)
        .map_err(|_| ErrorV1::PromptMaterialization)?;
    drop(first);

    let mut service =
        connect_participation(&config.participation_socket, config.participation_limits)
            .map_err(|_| ErrorV1::Participation)?;
    let mut gateway = connect_daimon_agent(
        &config.daimon_agent_socket,
        config.handshake_limits,
        config.agent_limits,
        config.timeout,
    )
    .map_err(|_| ErrorV1::Daimon)?;
    let mut outputs = AgentOutputSqliteStore::open(
        &config.output_path,
        config.output_owner,
        config.output_limits,
    )
    .map_err(|_| ErrorV1::OutputStore)?;
    let template =
        FixedAgentCommandTemplateV1::new(config.policy).map_err(|_| ErrorV1::InvalidConfig)?;
    let mut materializer = RetainedPromptMaterializerV1::new(prompt, template);
    let outcome = drive_current(
        &mut service,
        &mut gateway,
        &mut materializer,
        &mut outputs,
        config.participation,
        config.now,
        config.max_event_observations,
    )
    .map_err(|_| ErrorV1::Dispatch)?;
    Ok(summary(outcome, config.participation))
}

fn summary<E>(
    outcome: AgentWorkerOutcomeV1<E>,
    participation: participation::ParticipationIdV1,
) -> SummaryV1 {
    match outcome {
        AgentWorkerOutcomeV1::Idle(value) => SummaryV1::Idle {
            participation,
            revision: value.participation.revision,
        },
        AgentWorkerOutcomeV1::Progress(AgentDriveOutcomeV1::Observed {
            participation: view,
            ..
        }) => SummaryV1::Observed {
            participation,
            revision: view.revision,
        },
        AgentWorkerOutcomeV1::Progress(AgentDriveOutcomeV1::Fenced {
            participation: view,
            ..
        }) => SummaryV1::Fenced {
            participation,
            revision: view.revision,
        },
    }
}

fn hex_id(value: &str) -> Result<[u8; 16], ErrorV1> {
    if value.len() != 32 || !value.is_ascii() {
        return Err(ErrorV1::InvalidConfig);
    }
    let mut bytes = [0_u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *byte =
            u8::from_str_radix(&value[start..start + 2], 16).map_err(|_| ErrorV1::InvalidConfig)?;
    }
    Ok(bytes)
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ConfigV1 {
        ConfigV1 {
            participation_socket: "/tmp/moira-participation.sock".into(),
            daimon_agent_socket: "/tmp/daimon-agent.sock".into(),
            participation_id: "01010101010101010101010101010101".into(),
            output: OutputConfigV1 {
                path: "/tmp/moira-agent-output.db".into(),
                owner: "02020202020202020202020202020202".into(),
                max_entries: 16,
                max_output_bytes: 65_536,
                max_page_assistant_bytes: 16_384,
                max_database_pages: 64,
            },
            policy: PolicyConfigV1 {
                principal: "03030303030303030303030303030303".into(),
                workspace: "04040404040404040404040404040404".into(),
                context: vec!["05050505050505050505050505050505".into()],
                tools: vec!["06060606060606060606060606060606".into()],
                execution_budget: ExecutionBudgetConfigV1 {
                    max_elapsed_millis: 1_000,
                    max_input_tokens: 100,
                    max_output_tokens: 100,
                    max_tool_calls: 1,
                    max_context_bytes: 4_096,
                    max_output_bytes: 4_096,
                },
                session_budget: SessionBudgetConfigV1 {
                    max_elapsed_millis: 10_000,
                    max_turns: 4,
                    max_events: 32,
                    max_event_bytes: 65_536,
                    max_prompt_bytes: 65_536,
                    max_permission_requests: 4,
                },
            },
            transport: TransportConfigV1 {
                participation_max_frame_bytes: 131_072,
                participation_max_requests: 8,
                daimon_handshake_max_frame_bytes: 1_048_576,
                agent_max_frame_bytes: 1_048_576,
                timeout_ms: 5_000,
            },
            now: 7,
            max_event_observations: 32,
        }
    }

    #[test]
    fn strict_configuration_validates_without_opening_external_resources() {
        assert!(validate(config()).is_ok());
        let mut invalid = config();
        invalid.transport.participation_max_requests = 3;
        assert!(matches!(validate(invalid), Err(ErrorV1::InvalidConfig)));
        let mut duplicate = config();
        duplicate
            .policy
            .tools
            .push(duplicate.policy.tools[0].clone());
        assert!(matches!(validate(duplicate), Err(ErrorV1::InvalidConfig)));
    }

    #[test]
    fn unknown_configuration_fields_and_noncanonical_ids_are_rejected() {
        let input = br#"{"participation_socket":"x","unknown":true}"#;
        assert!(serde_json::from_slice::<ConfigV1>(input).is_err());
        let mut invalid = config();
        invalid.participation_id = "01".into();
        assert!(matches!(validate(invalid), Err(ErrorV1::InvalidConfig)));
    }

    #[test]
    fn summaries_are_content_free() {
        assert_eq!(
            SummaryV1::Fenced {
                participation: participation::ParticipationIdV1([1; 16]),
                revision: 9,
            }
            .json(),
            r#"{"outcome":"fenced_unknown","participation":"01010101010101010101010101010101","participation_revision":9}"#
        );
    }
}
