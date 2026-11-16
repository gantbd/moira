//! Local application composition for one governed deliberation run.
//!
//! The harness validates all execution identities before durable admission,
//! persists every valid attributed report, and never invents a terminal
//! resolution. Model execution remains in Daimon; durable semantics remain in
//! the injected Moira store.

#![forbid(unsafe_code)]

use daimon_client::ModelServicePort;
use moira_core::deliberation::{
    DeliberationCheckpointV1, DeliberationPlanV1, ResolutionV1,
    registry::{RegistryCommandV1, RegistryReceiptV1},
};
use moira_daimon::{
    CompletedToolResultV1, ModelExecutionAdapter,
    ensemble::{self, EnsembleRefusalV1, MemberContinuationV1, MemberOutcomeV1, MemberRunV1},
};
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};

pub mod action_selection;
pub mod agent_report;
pub mod agent_runtime;
pub mod peer_router;
pub mod peer_turn;
pub mod process_tool;
pub mod resolution;
pub mod tool_runtime;
pub mod tool_turn;

#[cfg(unix)]
pub mod unix {
    use daimon_client::unix::{ClientErrorV1, ModelUnixPort, connect_model};
    use daimon_wire::{LimitsV1, ModelLimitsV1};
    use moira_daimon::ModelExecutionAdapter;
    use std::{path::Path, time::Duration};

    /// Connect to Daimon's authenticated private model socket. Provider
    /// endpoint, credentials, attempt validation and authority stay in Daimon.
    pub fn connect_daimon(
        socket: impl AsRef<Path>,
        handshake_limits: LimitsV1,
        model_limits: ModelLimitsV1,
        timeout: Duration,
    ) -> Result<ModelExecutionAdapter<ModelUnixPort>, ClientErrorV1> {
        let (session, port) = connect_model(socket, handshake_limits, model_limits, timeout)?;
        Ok(ModelExecutionAdapter::new(session, port))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct GovernedRunV1<E> {
    pub outcomes: Vec<MemberOutcomeV1<E>>,
    pub retained: DeliberationCheckpointV1,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ReportCommitFailureV1<S, E> {
    pub error: DeliberationApplyErrorV1<S>,
    pub outcomes: Vec<MemberOutcomeV1<E>>,
    pub last_retained: Box<DeliberationCheckpointV1>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum GovernedRunErrorV1<S, E> {
    InvalidCohort(EnsembleRefusalV1),
    Admission(DeliberationApplyErrorV1<S>),
    ReportCommit(Box<ReportCommitFailureV1<S, E>>),
}

#[derive(Debug, Eq, PartialEq)]
pub struct GovernedMemberResumeV1<E> {
    pub outcome: MemberOutcomeV1<E>,
    pub retained: DeliberationCheckpointV1,
}

#[derive(Debug, Eq, PartialEq)]
pub enum GovernedMemberResumeErrorV1<S, E> {
    InvalidMember(EnsembleRefusalV1),
    MissingDeliberation,
    ReportCommit {
        error: DeliberationApplyErrorV1<S>,
        outcome: Box<MemberOutcomeV1<E>>,
        last_retained: Box<DeliberationCheckpointV1>,
    },
}

pub type GovernedMemberResumeResultV1<P, S> = Result<
    GovernedMemberResumeV1<<P as ModelServicePort>::Error>,
    GovernedMemberResumeErrorV1<<S as DeliberationStore>::Error, <P as ModelServicePort>::Error>,
>;

pub type GovernedRunResultV1<P, S> = Result<
    GovernedRunV1<<P as ModelServicePort>::Error>,
    GovernedRunErrorV1<<S as DeliberationStore>::Error, <P as ModelServicePort>::Error>,
>;

/// Admit, execute and retain one exact cohort. Matching replay is delegated to
/// the store and Daimon identities. Invalid model envelopes remain outcomes and
/// are never submitted as reports.
pub fn run<P, S>(
    store: &mut S,
    adapter: &mut ModelExecutionAdapter<P>,
    plan: DeliberationPlanV1,
    runs: Vec<MemberRunV1>,
    now: u64,
) -> GovernedRunResultV1<P, S>
where
    P: ModelServicePort,
    S: DeliberationStore,
{
    ensemble::validate_runs(&plan, &runs).map_err(GovernedRunErrorV1::InvalidCohort)?;
    let admitted = store
        .apply(store.revision(), RegistryCommandV1::Admit(plan.clone()))
        .map_err(GovernedRunErrorV1::Admission)?;
    let outcomes =
        ensemble::execute(adapter, &plan, runs, now).map_err(GovernedRunErrorV1::InvalidCohort)?;
    let reports = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            MemberOutcomeV1::Report { report, .. } => Some(report.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut retained = admitted.deliberation;
    for report in reports {
        match store.apply(
            store.revision(),
            RegistryCommandV1::SubmitReport {
                deliberation: plan.id,
                report,
            },
        ) {
            Ok(receipt) => retained = receipt.deliberation,
            Err(error) => {
                return Err(GovernedRunErrorV1::ReportCommit(Box::new(
                    ReportCommitFailureV1 {
                        error,
                        outcomes,
                        last_retained: Box::new(retained),
                    },
                )));
            }
        }
    }
    Ok(GovernedRunV1 { outcomes, retained })
}

/// Resume one member after a fully settled tool-continuation saga. Additional
/// native calls remain incomplete; only a terminal valid response is committed
/// as the member's attributed report.
#[allow(clippy::too_many_arguments)]
pub fn resume_member_and_retain<P, S>(
    store: &mut S,
    adapter: &mut ModelExecutionAdapter<P>,
    plan: &DeliberationPlanV1,
    run: MemberRunV1,
    continuation: MemberContinuationV1,
    results: Vec<CompletedToolResultV1>,
    now: u64,
) -> GovernedMemberResumeResultV1<P, S>
where
    P: ModelServicePort,
    S: DeliberationStore,
{
    let outcome = ensemble::resume_member(adapter, plan, run, continuation, results, now)
        .map_err(GovernedMemberResumeErrorV1::InvalidMember)?;
    let retained = store
        .deliberation(plan.id)
        .ok_or(GovernedMemberResumeErrorV1::MissingDeliberation)?;
    let MemberOutcomeV1::Report { report, .. } = &outcome else {
        return Ok(GovernedMemberResumeV1 { outcome, retained });
    };
    match store.apply(
        store.revision(),
        RegistryCommandV1::SubmitReport {
            deliberation: plan.id,
            report: report.clone(),
        },
    ) {
        Ok(receipt) => Ok(GovernedMemberResumeV1 {
            outcome,
            retained: receipt.deliberation,
        }),
        Err(error) => Err(GovernedMemberResumeErrorV1::ReportCommit {
            error,
            outcome: Box::new(outcome),
            last_retained: Box::new(retained),
        }),
    }
}

/// Apply an explicit externally supplied terminal value. This function does
/// not compute proof, choose a frontier member, or reinterpret model output.
pub fn resolve_explicit<S: DeliberationStore>(
    store: &mut S,
    deliberation: moira_core::deliberation::DeliberationIdV1,
    resolution: ResolutionV1,
) -> Result<RegistryReceiptV1, DeliberationApplyErrorV1<S::Error>> {
    store.apply(
        store.revision(),
        RegistryCommandV1::Resolve {
            deliberation,
            resolution,
        },
    )
}
