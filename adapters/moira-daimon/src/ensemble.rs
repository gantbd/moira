//! Independent member execution for one already-admitted deliberation plan.
//!
//! This layer creates reports, never resolutions. Partial execution remains a
//! vector of attributed outcomes and cannot become proof or choice.

use std::collections::BTreeSet;

use daimon_client::ModelServicePort;
use daimon_protocol as daimon;
pub use moira_core::deliberation::review::{
    EnsembleCorrelationEvidenceV1 as EnsembleCorrelationV1, IndependenceAssessmentV1,
};
use moira_core::deliberation::{
    DeliberationPlanV1, EnsembleMemberIdV1, MemberReportV1, ReportIdV1,
    review::{
        CorrelationDimensionEvidenceV1, CorrelationDimensionV1, EnsembleCorrelationEvidenceV1,
        FirstPassExposureV1,
    },
};
use moira_protocol::DispositionV1;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    AdapterErrorV1, CompletedToolResultV1, ExecutionIntentV1, ModelExecutionAdapter,
    ModelExecutionReplyV1, ModelProposalV1, ModelResultV1,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberRunV1 {
    pub member: EnsembleMemberIdV1,
    pub report: ReportIdV1,
    pub execution: ExecutionIntentV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemberContinuationV1 {
    pub member: EnsembleMemberIdV1,
    pub report: ReportIdV1,
    pub source_execution: [u8; 16],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemberResponseLimitsV1 {
    pub max_bytes: usize,
    pub max_claims: usize,
    pub max_evidence_digests: usize,
    pub max_dissent_digests: usize,
}

impl Default for MemberResponseLimitsV1 {
    fn default() -> Self {
        Self {
            max_bytes: 1_048_576,
            max_claims: 64,
            max_evidence_digests: 64,
            max_dissent_digests: 32,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberClaimV1 {
    pub kind: moira_core::deliberation::ClaimKindV1,
    pub statement: String,
    pub statement_digest: [u8; 32],
    pub evidence_digests: Vec<[u8; 32]>,
    pub dissent_digests: Vec<[u8; 32]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberResponseV1 {
    pub claims: Vec<MemberClaimV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberResponseRefusalV1 {
    TooLarge,
    InvalidJson,
    UnsupportedSchema,
    EmptyClaims,
    ClaimLimit,
    UnadmittedClaimKind,
    InvalidDigest,
    DuplicateClaim,
    EvidenceLimit,
    DuplicateEvidence,
    DissentLimit,
    DuplicateDissent,
}

#[derive(Debug, Eq, PartialEq)]
pub enum EnsembleRefusalV1 {
    Empty,
    CohortMismatch,
    DuplicateReport,
    DuplicateExecution,
    DuplicateIdempotencyKey,
    DuplicateEffect,
    OutcomeMismatch,
}

#[derive(Debug, Eq, PartialEq)]
pub enum MemberOutcomeV1<E> {
    Report {
        member: EnsembleMemberIdV1,
        report: MemberReportV1,
        response: MemberResponseV1,
        result: Box<ModelResultV1>,
    },
    InvalidReport {
        member: EnsembleMemberIdV1,
        refusal: MemberResponseRefusalV1,
        result: Box<ModelResultV1>,
    },
    ToolContinuation {
        member: EnsembleMemberIdV1,
        continuation: MemberContinuationV1,
        result: Box<ModelResultV1>,
    },
    Uncertain {
        member: EnsembleMemberIdV1,
        receipt: moira_protocol::ResultReceiptV1,
    },
    NonCompleted {
        member: EnsembleMemberIdV1,
        result: Box<ModelResultV1>,
    },
    Refused {
        member: EnsembleMemberIdV1,
        refusal: daimon::RefusalV1,
    },
    Failed {
        member: EnsembleMemberIdV1,
        error: AdapterErrorV1<E>,
    },
}

pub fn execute<P: ModelServicePort>(
    adapter: &mut ModelExecutionAdapter<P>,
    plan: &DeliberationPlanV1,
    runs: Vec<MemberRunV1>,
    now: u64,
) -> Result<Vec<MemberOutcomeV1<P::Error>>, EnsembleRefusalV1> {
    execute_with_limits(adapter, plan, runs, now, MemberResponseLimitsV1::default())
}

pub fn execute_with_limits<P: ModelServicePort>(
    adapter: &mut ModelExecutionAdapter<P>,
    plan: &DeliberationPlanV1,
    runs: Vec<MemberRunV1>,
    now: u64,
    response_limits: MemberResponseLimitsV1,
) -> Result<Vec<MemberOutcomeV1<P::Error>>, EnsembleRefusalV1> {
    validate_runs(plan, &runs)?;
    Ok(runs
        .into_iter()
        .map(|run| {
            let reply = adapter.execute(run.execution.clone(), now);
            classify_member_reply(plan, &run, reply, response_limits)
        })
        .collect())
}

pub fn resume_member<P: ModelServicePort>(
    adapter: &mut ModelExecutionAdapter<P>,
    plan: &DeliberationPlanV1,
    run: MemberRunV1,
    continuation: MemberContinuationV1,
    results: Vec<CompletedToolResultV1>,
    now: u64,
) -> Result<MemberOutcomeV1<P::Error>, EnsembleRefusalV1> {
    resume_member_with_limits(
        adapter,
        plan,
        run,
        continuation,
        results,
        now,
        MemberResponseLimitsV1::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn resume_member_with_limits<P: ModelServicePort>(
    adapter: &mut ModelExecutionAdapter<P>,
    plan: &DeliberationPlanV1,
    run: MemberRunV1,
    continuation: MemberContinuationV1,
    results: Vec<CompletedToolResultV1>,
    now: u64,
    response_limits: MemberResponseLimitsV1,
) -> Result<MemberOutcomeV1<P::Error>, EnsembleRefusalV1> {
    if !plan.members.iter().any(|member| member.id == run.member)
        || continuation.member != run.member
        || continuation.report != run.report
        || continuation.source_execution == [0; 16]
    {
        return Err(EnsembleRefusalV1::CohortMismatch);
    }
    let reply = adapter.resume(
        run.execution.clone(),
        continuation.source_execution,
        results,
        now,
    );
    Ok(classify_member_reply(plan, &run, reply, response_limits))
}

fn classify_member_reply<E>(
    plan: &DeliberationPlanV1,
    run: &MemberRunV1,
    reply: Result<ModelExecutionReplyV1, AdapterErrorV1<E>>,
    response_limits: MemberResponseLimitsV1,
) -> MemberOutcomeV1<E> {
    let member = plan
        .members
        .iter()
        .find(|candidate| candidate.id == run.member)
        .expect("member validated");
    match reply {
        Ok(ModelExecutionReplyV1::Result(result))
            if result.receipt.disposition == DispositionV1::Completed =>
        {
            if result
                .proposals
                .iter()
                .any(|proposal| matches!(proposal, ModelProposalV1::ToolCall { .. }))
            {
                return MemberOutcomeV1::ToolContinuation {
                    member: run.member,
                    continuation: MemberContinuationV1 {
                        member: run.member,
                        report: run.report,
                        source_execution: result.receipt.execution_id,
                    },
                    result: Box::new(result),
                };
            }
            let response = match parse_member_response(plan, &result.output, response_limits) {
                Ok(response) => response,
                Err(refusal) => {
                    return MemberOutcomeV1::InvalidReport {
                        member: run.member,
                        refusal,
                        result: Box::new(result),
                    };
                }
            };
            let mut evidence_digests = vec![
                plan.question_digest,
                plan.method.machine_digest,
                plan.method.state_digest,
                member.grounding.authority_digest,
                member.grounding.surface_digest,
                result.evidence_digest,
            ];
            let mut dissent_digests = Vec::new();
            for claim in &response.claims {
                evidence_digests.extend_from_slice(&claim.evidence_digests);
                dissent_digests.extend_from_slice(&claim.dissent_digests);
            }
            if evidence_digests.len() > response_limits.max_evidence_digests {
                return MemberOutcomeV1::InvalidReport {
                    member: run.member,
                    refusal: MemberResponseRefusalV1::EvidenceLimit,
                    result: Box::new(result),
                };
            }
            if evidence_digests.iter().collect::<BTreeSet<_>>().len() != evidence_digests.len() {
                return MemberOutcomeV1::InvalidReport {
                    member: run.member,
                    refusal: MemberResponseRefusalV1::DuplicateEvidence,
                    result: Box::new(result),
                };
            }
            let report = MemberReportV1 {
                id: run.report,
                member: run.member,
                output_digest: Sha256::digest(&result.output).into(),
                evidence_digests,
                dissent_digests,
            };
            MemberOutcomeV1::Report {
                member: run.member,
                report,
                response,
                result: Box::new(result),
            }
        }
        Ok(ModelExecutionReplyV1::Result(result)) => MemberOutcomeV1::NonCompleted {
            member: run.member,
            result: Box::new(result),
        },
        Ok(ModelExecutionReplyV1::Uncertain(receipt)) => MemberOutcomeV1::Uncertain {
            member: run.member,
            receipt,
        },
        Ok(ModelExecutionReplyV1::Refused(refusal)) => MemberOutcomeV1::Refused {
            member: run.member,
            refusal,
        },
        Err(error) => MemberOutcomeV1::Failed {
            member: run.member,
            error,
        },
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberResponseWireV1 {
    schema: String,
    claims: Vec<MemberClaimWireV1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberClaimWireV1 {
    kind: ClaimKindWireV1,
    statement: String,
    evidence_digests: Vec<String>,
    dissent_digests: Vec<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClaimKindWireV1 {
    Formal,
    Empirical,
    Normative,
    Historical,
    Creative,
}

impl From<ClaimKindWireV1> for moira_core::deliberation::ClaimKindV1 {
    fn from(value: ClaimKindWireV1) -> Self {
        use moira_core::deliberation::ClaimKindV1;
        match value {
            ClaimKindWireV1::Formal => ClaimKindV1::Formal,
            ClaimKindWireV1::Empirical => ClaimKindV1::Empirical,
            ClaimKindWireV1::Normative => ClaimKindV1::Normative,
            ClaimKindWireV1::Historical => ClaimKindV1::Historical,
            ClaimKindWireV1::Creative => ClaimKindV1::Creative,
        }
    }
}

pub fn parse_member_response(
    plan: &DeliberationPlanV1,
    bytes: &[u8],
    limits: MemberResponseLimitsV1,
) -> Result<MemberResponseV1, MemberResponseRefusalV1> {
    if bytes.len() > limits.max_bytes {
        return Err(MemberResponseRefusalV1::TooLarge);
    }
    let wire: MemberResponseWireV1 =
        serde_json::from_slice(bytes).map_err(|_| MemberResponseRefusalV1::InvalidJson)?;
    if wire.schema != "moira.member-response.v1" {
        return Err(MemberResponseRefusalV1::UnsupportedSchema);
    }
    if wire.claims.is_empty() {
        return Err(MemberResponseRefusalV1::EmptyClaims);
    }
    if wire.claims.len() > limits.max_claims {
        return Err(MemberResponseRefusalV1::ClaimLimit);
    }

    let mut claims = Vec::with_capacity(wire.claims.len());
    let mut claim_ids = BTreeSet::new();
    let mut evidence_ids = BTreeSet::new();
    let mut dissent_ids = BTreeSet::new();
    for claim in wire.claims {
        let kind = claim.kind.into();
        if !plan.claims.contains(&kind) {
            return Err(MemberResponseRefusalV1::UnadmittedClaimKind);
        }
        let statement_digest = Sha256::digest(claim.statement.as_bytes()).into();
        if !claim_ids.insert(statement_digest) {
            return Err(MemberResponseRefusalV1::DuplicateClaim);
        }
        let evidence_digests = parse_unique_digests(
            claim.evidence_digests,
            &mut evidence_ids,
            MemberResponseRefusalV1::DuplicateEvidence,
        )?;
        if evidence_ids.len() > limits.max_evidence_digests.saturating_sub(6) {
            return Err(MemberResponseRefusalV1::EvidenceLimit);
        }
        let dissent_digests = parse_unique_digests(
            claim.dissent_digests,
            &mut dissent_ids,
            MemberResponseRefusalV1::DuplicateDissent,
        )?;
        if dissent_ids.len() > limits.max_dissent_digests {
            return Err(MemberResponseRefusalV1::DissentLimit);
        }
        claims.push(MemberClaimV1 {
            kind,
            statement: claim.statement,
            statement_digest,
            evidence_digests,
            dissent_digests,
        });
    }
    Ok(MemberResponseV1 { claims })
}

fn parse_unique_digests(
    values: Vec<String>,
    seen: &mut BTreeSet<[u8; 32]>,
    duplicate: MemberResponseRefusalV1,
) -> Result<Vec<[u8; 32]>, MemberResponseRefusalV1> {
    values
        .into_iter()
        .map(|value| {
            let digest = parse_digest(&value)?;
            if !seen.insert(digest) {
                return Err(duplicate);
            }
            Ok(digest)
        })
        .collect()
}

fn parse_digest(value: &str) -> Result<[u8; 32], MemberResponseRefusalV1> {
    if value.len() != 64 {
        return Err(MemberResponseRefusalV1::InvalidDigest);
    }
    let mut digest = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0]).ok_or(MemberResponseRefusalV1::InvalidDigest)?;
        let low = hex_nibble(pair[1]).ok_or(MemberResponseRefusalV1::InvalidDigest)?;
        digest[index] = (high << 4) | low;
    }
    Ok(digest)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

pub fn validate_runs(
    plan: &DeliberationPlanV1,
    runs: &[MemberRunV1],
) -> Result<(), EnsembleRefusalV1> {
    if runs.is_empty() {
        return Err(EnsembleRefusalV1::Empty);
    }
    let planned = plan
        .members
        .iter()
        .map(|member| member.id)
        .collect::<BTreeSet<_>>();
    let supplied = runs.iter().map(|run| run.member).collect::<BTreeSet<_>>();
    if supplied.len() != runs.len() || supplied != planned {
        return Err(EnsembleRefusalV1::CohortMismatch);
    }
    unique(
        runs.iter().map(|run| run.report),
        EnsembleRefusalV1::DuplicateReport,
    )?;
    unique(
        runs.iter().map(|run| run.execution.execution_id),
        EnsembleRefusalV1::DuplicateExecution,
    )?;
    unique(
        runs.iter().map(|run| run.execution.idempotency_key),
        EnsembleRefusalV1::DuplicateIdempotencyKey,
    )?;
    unique(
        runs.iter().map(|run| run.execution.effect),
        EnsembleRefusalV1::DuplicateEffect,
    )?;
    Ok(())
}

pub fn analyze_correlation(
    plan: &DeliberationPlanV1,
    runs: &[MemberRunV1],
) -> Result<EnsembleCorrelationV1, EnsembleRefusalV1> {
    validate_runs(plan, runs)?;
    let principals = runs
        .iter()
        .map(|run| run.execution.principal)
        .collect::<BTreeSet<_>>();
    let profiles = runs
        .iter()
        .map(|run| run.execution.profile)
        .collect::<BTreeSet<_>>();
    let contexts = runs
        .iter()
        .map(|run| context_manifest_digest(&run.execution.context))
        .collect::<BTreeSet<_>>();
    let grounding = plan
        .members
        .iter()
        .map(|member| member.grounding.surface_digest)
        .collect::<BTreeSet<_>>();
    let mut correlation = EnsembleCorrelationEvidenceV1::undisclosed(runs.len());
    for (dimension, distinct_values) in [
        (CorrelationDimensionV1::ExecutionPrincipal, principals.len()),
        (CorrelationDimensionV1::ExecutionProfile, profiles.len()),
        (CorrelationDimensionV1::ContextManifest, contexts.len()),
        (CorrelationDimensionV1::Grounding, grounding.len()),
    ] {
        let evidence = correlation
            .dimensions
            .iter_mut()
            .find(|evidence| evidence.dimension == dimension)
            .expect("all correlation dimensions are constructed above");
        *evidence = CorrelationDimensionEvidenceV1 {
            dimension,
            observed_members: runs.len(),
            distinct_values,
        };
    }
    correlation.first_pass = FirstPassExposureV1 {
        committed_before_cross_review: 0,
        exposed_before_commit: 0,
        unknown: runs.len(),
    };
    Ok(correlation)
}

/// Add only invocation facts that Daimon published with member outcomes.
/// Legacy or non-dispatched outcomes leave those dimensions explicitly
/// unobserved; no provider/model/prompt identity is inferred from output.
pub fn analyze_observed_correlation<E>(
    plan: &DeliberationPlanV1,
    runs: &[MemberRunV1],
    outcomes: &[MemberOutcomeV1<E>],
) -> Result<EnsembleCorrelationV1, EnsembleRefusalV1> {
    validate_runs(plan, runs)?;
    let expected = runs.iter().map(|run| run.member).collect::<BTreeSet<_>>();
    let observed = outcomes.iter().map(outcome_member).collect::<BTreeSet<_>>();
    if outcomes.len() != runs.len() || observed.len() != outcomes.len() || observed != expected {
        return Err(EnsembleRefusalV1::OutcomeMismatch);
    }
    let evidence = outcomes
        .iter()
        .filter_map(outcome_result)
        .filter_map(|result| result.invocation_evidence.as_deref())
        .collect::<Vec<_>>();
    let contexts = evidence
        .iter()
        .map(|value| value.context_manifest_digest)
        .collect::<BTreeSet<_>>();
    let models = evidence
        .iter()
        .map(|value| value.model_digest)
        .collect::<BTreeSet<_>>();
    let providers = evidence
        .iter()
        .map(|value| (value.provider, value.provider_document_digest))
        .collect::<BTreeSet<_>>();
    let prompts = evidence
        .iter()
        .map(|value| value.prompt_digest)
        .collect::<BTreeSet<_>>();
    let scopes = evidence
        .iter()
        .map(|value| value.evidence_scope_digest)
        .collect::<BTreeSet<_>>();
    let mut correlation = analyze_correlation(plan, runs)?;
    for (dimension, distinct_values) in [
        (CorrelationDimensionV1::ContextManifest, contexts.len()),
        (CorrelationDimensionV1::Model, models.len()),
        (CorrelationDimensionV1::Provider, providers.len()),
        (CorrelationDimensionV1::Prompt, prompts.len()),
        (CorrelationDimensionV1::EvidenceScope, scopes.len()),
    ] {
        let dimension_evidence = correlation
            .dimensions
            .iter_mut()
            .find(|evidence| evidence.dimension == dimension)
            .expect("all correlation dimensions are constructed above");
        *dimension_evidence = CorrelationDimensionEvidenceV1 {
            dimension,
            observed_members: evidence.len(),
            distinct_values,
        };
    }
    Ok(correlation)
}

fn outcome_member<E>(outcome: &MemberOutcomeV1<E>) -> EnsembleMemberIdV1 {
    match outcome {
        MemberOutcomeV1::Report { member, .. }
        | MemberOutcomeV1::InvalidReport { member, .. }
        | MemberOutcomeV1::ToolContinuation { member, .. }
        | MemberOutcomeV1::Uncertain { member, .. }
        | MemberOutcomeV1::NonCompleted { member, .. }
        | MemberOutcomeV1::Refused { member, .. }
        | MemberOutcomeV1::Failed { member, .. } => *member,
    }
}

fn outcome_result<E>(outcome: &MemberOutcomeV1<E>) -> Option<&ModelResultV1> {
    match outcome {
        MemberOutcomeV1::Report { result, .. }
        | MemberOutcomeV1::InvalidReport { result, .. }
        | MemberOutcomeV1::ToolContinuation { result, .. }
        | MemberOutcomeV1::NonCompleted { result, .. } => Some(result),
        MemberOutcomeV1::Uncertain { .. }
        | MemberOutcomeV1::Refused { .. }
        | MemberOutcomeV1::Failed { .. } => None,
    }
}

fn context_manifest_digest(context: &[[u8; 16]]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRA_MEMBER_CONTEXT_V1\0");
    hash.update((context.len() as u64).to_be_bytes());
    for reference in context {
        hash.update(reference);
    }
    hash.finalize().into()
}

fn unique<T: Ord>(
    values: impl Iterator<Item = T>,
    refusal: EnsembleRefusalV1,
) -> Result<(), EnsembleRefusalV1> {
    let values = values.collect::<Vec<_>>();
    if values.iter().collect::<BTreeSet<_>>().len() == values.len() {
        Ok(())
    } else {
        Err(refusal)
    }
}
