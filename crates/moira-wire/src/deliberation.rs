//! Canonical bounded wire family for methodical deliberation.

use super::{CodecErrorV1, LimitsV1, Reader, Writer};
use moira_protocol::deliberation as d;

/// Deliberation profile V2 adds retained stakeholder standing and cohort
/// admission evidence. Distinct frame kinds make V1 peers fail closed instead
/// of decoding the changed plan shape under the old profile.
pub const DELIBERATION_PROFILE_VERSION: u8 = 4;
const REQUEST: u8 = 14;
const REPLY: u8 = 15;

pub fn encode_deliberation_request(
    value: &d::CommandV1,
    limits: LimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    let mut w = Writer::new(REQUEST, limits)?;
    command(&mut w, value)?;
    Ok(w.bytes)
}

pub fn decode_deliberation_request(
    bytes: &[u8],
    limits: LimitsV1,
) -> Result<d::CommandV1, CodecErrorV1> {
    let mut r = Reader::new(bytes, REQUEST, limits)?;
    let value = read_command(&mut r)?;
    r.finish()?;
    Ok(value)
}

pub fn encode_deliberation_reply(
    value: &d::ReplyV1,
    limits: LimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    let mut w = Writer::new(REPLY, limits)?;
    match value {
        d::ReplyV1::Accepted(value) => {
            w.byte(0)?;
            outcome(&mut w, value)?;
        }
        d::ReplyV1::Refused(value) => {
            w.byte(1)?;
            refusal(&mut w, *value)?;
        }
    }
    Ok(w.bytes)
}

pub fn decode_deliberation_reply(
    bytes: &[u8],
    limits: LimitsV1,
) -> Result<d::ReplyV1, CodecErrorV1> {
    let mut r = Reader::new(bytes, REPLY, limits)?;
    let value = match r.byte()? {
        0 => d::ReplyV1::Accepted(read_outcome(&mut r)?),
        1 => d::ReplyV1::Refused(read_refusal(&mut r)?),
        _ => return Err(CodecErrorV1::Unsupported),
    };
    r.finish()?;
    Ok(value)
}

fn command(w: &mut Writer, value: &d::CommandV1) -> Result<(), CodecErrorV1> {
    match value {
        d::CommandV1::Apply {
            expected_revision,
            mutation: value,
        } => {
            w.byte(0)?;
            w.u64(*expected_revision)?;
            mutation(w, value)
        }
        d::CommandV1::Status => w.byte(1),
        d::CommandV1::Deliberation { deliberation } => {
            w.byte(2)?;
            w.raw(&deliberation.0)
        }
        d::CommandV1::RespondHumanGate { operation } => {
            w.byte(3)?;
            human_gate_operation(w, *operation)
        }
        d::CommandV1::Review { deliberation } => {
            w.byte(4)?;
            w.raw(&deliberation.0)
        }
    }
}

fn read_command(r: &mut Reader<'_>) -> Result<d::CommandV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(d::CommandV1::Apply {
            expected_revision: r.u64()?,
            mutation: read_mutation(r)?,
        }),
        1 => Ok(d::CommandV1::Status),
        2 => Ok(d::CommandV1::Deliberation {
            deliberation: d::DeliberationIdV1(r.raw()?),
        }),
        3 => Ok(d::CommandV1::RespondHumanGate {
            operation: read_human_gate_operation(r)?,
        }),
        4 => Ok(d::CommandV1::Review {
            deliberation: d::DeliberationIdV1(r.raw()?),
        }),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn mutation(w: &mut Writer, value: &d::MutationV1) -> Result<(), CodecErrorV1> {
    match value {
        d::MutationV1::Admit(value) => {
            w.byte(0)?;
            plan(w, value)
        }
        d::MutationV1::SubmitReport {
            deliberation,
            report: value,
        } => {
            w.byte(1)?;
            w.raw(&deliberation.0)?;
            report(w, value)
        }
        d::MutationV1::Resolve {
            deliberation,
            resolution: value,
        } => {
            w.byte(2)?;
            w.raw(&deliberation.0)?;
            resolution(w, value)
        }
        d::MutationV1::RequestHumanGate {
            deliberation,
            request,
        } => {
            w.byte(3)?;
            w.raw(&deliberation.0)?;
            human_gate_request(w, *request)
        }
        d::MutationV1::BeginReview {
            deliberation,
            correlation: value,
        } => {
            w.byte(4)?;
            w.raw(&deliberation.0)?;
            correlation(w, value)
        }
        d::MutationV1::SubmitFinding {
            deliberation,
            finding: value,
        } => {
            w.byte(5)?;
            w.raw(&deliberation.0)?;
            review_finding(w, value)
        }
        d::MutationV1::SubmitRebuttal {
            deliberation,
            rebuttal: value,
        } => {
            w.byte(6)?;
            w.raw(&deliberation.0)?;
            rebuttal(w, value)
        }
        d::MutationV1::DisposeFinding {
            deliberation,
            disposition: value,
        } => {
            w.byte(7)?;
            w.raw(&deliberation.0)?;
            finding_disposition(w, value)
        }
    }
}

fn read_mutation(r: &mut Reader<'_>) -> Result<d::MutationV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(d::MutationV1::Admit(Box::new(read_plan(r)?))),
        1 => Ok(d::MutationV1::SubmitReport {
            deliberation: d::DeliberationIdV1(r.raw()?),
            report: read_report(r)?,
        }),
        2 => Ok(d::MutationV1::Resolve {
            deliberation: d::DeliberationIdV1(r.raw()?),
            resolution: read_resolution(r)?,
        }),
        3 => Ok(d::MutationV1::RequestHumanGate {
            deliberation: d::DeliberationIdV1(r.raw()?),
            request: read_human_gate_request(r)?,
        }),
        4 => Ok(d::MutationV1::BeginReview {
            deliberation: d::DeliberationIdV1(r.raw()?),
            correlation: read_correlation(r)?,
        }),
        5 => Ok(d::MutationV1::SubmitFinding {
            deliberation: d::DeliberationIdV1(r.raw()?),
            finding: read_review_finding(r)?,
        }),
        6 => Ok(d::MutationV1::SubmitRebuttal {
            deliberation: d::DeliberationIdV1(r.raw()?),
            rebuttal: read_rebuttal(r)?,
        }),
        7 => Ok(d::MutationV1::DisposeFinding {
            deliberation: d::DeliberationIdV1(r.raw()?),
            disposition: read_finding_disposition(r)?,
        }),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn outcome(w: &mut Writer, value: &d::OutcomeV1) -> Result<(), CodecErrorV1> {
    match value {
        d::OutcomeV1::Applied {
            revision,
            deliberation,
            replayed,
        } => {
            w.byte(0)?;
            w.u64(*revision)?;
            view(w, deliberation)?;
            boolean(w, *replayed)
        }
        d::OutcomeV1::Status(value) => {
            w.byte(1)?;
            w.u64(value.revision)?;
            boolean(w, value.needs_reopen)
        }
        d::OutcomeV1::Deliberation(value) => {
            w.byte(2)?;
            w.option(value.as_deref(), view)
        }
        d::OutcomeV1::ReviewApplied {
            revision,
            deliberation,
            review,
            replayed,
        } => {
            w.byte(3)?;
            w.u64(*revision)?;
            view(w, deliberation)?;
            review_view(w, review)?;
            boolean(w, *replayed)
        }
        d::OutcomeV1::Review(value) => {
            w.byte(4)?;
            w.option(value.as_deref(), review_view)
        }
    }
}

fn read_outcome(r: &mut Reader<'_>) -> Result<d::OutcomeV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(d::OutcomeV1::Applied {
            revision: r.u64()?,
            deliberation: Box::new(read_view(r)?),
            replayed: read_boolean(r)?,
        }),
        1 => Ok(d::OutcomeV1::Status(d::StatusV1 {
            revision: r.u64()?,
            needs_reopen: read_boolean(r)?,
        })),
        2 => Ok(d::OutcomeV1::Deliberation(
            r.option(read_view)?.map(Box::new),
        )),
        3 => Ok(d::OutcomeV1::ReviewApplied {
            revision: r.u64()?,
            deliberation: Box::new(read_view(r)?),
            review: Box::new(read_review_view(r)?),
            replayed: read_boolean(r)?,
        }),
        4 => Ok(d::OutcomeV1::Review(
            r.option(read_review_view)?.map(Box::new),
        )),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn view(w: &mut Writer, value: &d::DeliberationViewV1) -> Result<(), CodecErrorV1> {
    w.byte(value.version)?;
    plan(w, &value.plan)?;
    limits(w, value.limits)?;
    write_len(w, value.reports.len())?;
    for value in &value.reports {
        report(w, value)?;
    }
    w.option(value.resolution.as_ref(), resolution)?;
    w.option(value.human_gate_request.as_ref(), |w, value| {
        human_gate_request(w, *value)
    })?;
    w.option(value.human_gate_response.as_ref(), |w, value| {
        human_gate_response(w, *value)
    })?;
    w.u32(value.remaining_fuel)?;
    w.byte(phase_tag(value.phase))
}

fn read_view(r: &mut Reader<'_>) -> Result<d::DeliberationViewV1, CodecErrorV1> {
    Ok(d::DeliberationViewV1 {
        version: r.byte()?,
        plan: read_plan(r)?,
        limits: read_limits(r)?,
        reports: read_many(r, read_report)?,
        resolution: r.option(read_resolution)?,
        human_gate_request: r.option(read_human_gate_request)?,
        human_gate_response: r.option(read_human_gate_response)?,
        remaining_fuel: r.u32()?,
        phase: read_phase(r.byte()?)?,
    })
}

fn correlation(
    w: &mut Writer,
    value: &d::EnsembleCorrelationEvidenceV1,
) -> Result<(), CodecErrorV1> {
    w.usize(value.members)?;
    write_len(w, value.dimensions.len())?;
    for evidence in &value.dimensions {
        w.byte(correlation_dimension_tag(evidence.dimension))?;
        w.usize(evidence.observed_members)?;
        w.usize(evidence.distinct_values)?;
    }
    w.usize(value.first_pass.committed_before_cross_review)?;
    w.usize(value.first_pass.exposed_before_commit)?;
    w.usize(value.first_pass.unknown)
}

fn read_correlation(r: &mut Reader<'_>) -> Result<d::EnsembleCorrelationEvidenceV1, CodecErrorV1> {
    Ok(d::EnsembleCorrelationEvidenceV1 {
        members: r.usize()?,
        dimensions: read_many(r, |r| {
            Ok(d::CorrelationDimensionEvidenceV1 {
                dimension: read_correlation_dimension(r.byte()?)?,
                observed_members: r.usize()?,
                distinct_values: r.usize()?,
            })
        })?,
        first_pass: d::FirstPassExposureV1 {
            committed_before_cross_review: r.usize()?,
            exposed_before_commit: r.usize()?,
            unknown: r.usize()?,
        },
    })
}

fn correlation_dimension_tag(value: d::CorrelationDimensionV1) -> u8 {
    match value {
        d::CorrelationDimensionV1::ExecutionPrincipal => 0,
        d::CorrelationDimensionV1::ExecutionProfile => 1,
        d::CorrelationDimensionV1::ContextManifest => 2,
        d::CorrelationDimensionV1::Model => 3,
        d::CorrelationDimensionV1::Provider => 4,
        d::CorrelationDimensionV1::Prompt => 5,
        d::CorrelationDimensionV1::Grounding => 6,
        d::CorrelationDimensionV1::EvidenceScope => 7,
        d::CorrelationDimensionV1::Organization => 8,
    }
}

fn read_correlation_dimension(value: u8) -> Result<d::CorrelationDimensionV1, CodecErrorV1> {
    match value {
        0 => Ok(d::CorrelationDimensionV1::ExecutionPrincipal),
        1 => Ok(d::CorrelationDimensionV1::ExecutionProfile),
        2 => Ok(d::CorrelationDimensionV1::ContextManifest),
        3 => Ok(d::CorrelationDimensionV1::Model),
        4 => Ok(d::CorrelationDimensionV1::Provider),
        5 => Ok(d::CorrelationDimensionV1::Prompt),
        6 => Ok(d::CorrelationDimensionV1::Grounding),
        7 => Ok(d::CorrelationDimensionV1::EvidenceScope),
        8 => Ok(d::CorrelationDimensionV1::Organization),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn independence_tag(value: d::IndependenceAssessmentV1) -> u8 {
    match value {
        d::IndependenceAssessmentV1::Correlated => 0,
        d::IndependenceAssessmentV1::NotEstablished => 1,
    }
}

fn read_independence(value: u8) -> Result<d::IndependenceAssessmentV1, CodecErrorV1> {
    match value {
        0 => Ok(d::IndependenceAssessmentV1::Correlated),
        1 => Ok(d::IndependenceAssessmentV1::NotEstablished),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn review_view(w: &mut Writer, value: &d::ReviewViewV1) -> Result<(), CodecErrorV1> {
    w.byte(value.version)?;
    w.raw(&value.deliberation.0)?;
    review_limits(w, value.limits)?;
    write_len(w, value.reports.len())?;
    for value in &value.reports {
        report(w, value)?;
    }
    correlation(w, &value.correlation)?;
    w.byte(independence_tag(value.independence))?;
    w.option(value.admitted_human_gate_response.as_ref(), |w, value| {
        human_gate_response(w, *value)
    })?;
    write_len(w, value.findings.len())?;
    for value in &value.findings {
        review_finding(w, value)?;
    }
    write_len(w, value.rebuttals.len())?;
    for value in &value.rebuttals {
        rebuttal(w, value)?;
    }
    write_len(w, value.dispositions.len())?;
    for value in &value.dispositions {
        finding_disposition(w, value)?;
    }
    w.byte(review_phase_tag(value.phase))
}

fn read_review_view(r: &mut Reader<'_>) -> Result<d::ReviewViewV1, CodecErrorV1> {
    Ok(d::ReviewViewV1 {
        version: r.byte()?,
        deliberation: d::DeliberationIdV1(r.raw()?),
        limits: read_review_limits(r)?,
        reports: read_many(r, read_report)?,
        correlation: read_correlation(r)?,
        independence: read_independence(r.byte()?)?,
        admitted_human_gate_response: r.option(read_human_gate_response)?,
        findings: read_many(r, read_review_finding)?,
        rebuttals: read_many(r, read_rebuttal)?,
        dispositions: read_many(r, read_finding_disposition)?,
        phase: read_review_phase(r.byte()?)?,
    })
}

fn review_limits(w: &mut Writer, value: d::ReviewLimitsV1) -> Result<(), CodecErrorV1> {
    w.usize(value.max_findings)?;
    w.usize(value.max_rebuttals)?;
    w.usize(value.max_evidence_per_item)
}

fn read_review_limits(r: &mut Reader<'_>) -> Result<d::ReviewLimitsV1, CodecErrorV1> {
    Ok(d::ReviewLimitsV1 {
        max_findings: r.usize()?,
        max_rebuttals: r.usize()?,
        max_evidence_per_item: r.usize()?,
    })
}

fn review_finding(w: &mut Writer, value: &d::ReviewFindingV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    w.raw(&value.source_report.0)?;
    w.raw(&value.owner)?;
    w.raw(&value.finding_digest)?;
    digests(w, &value.evidence_digests)?;
    boolean(w, value.requires_human_gate)
}

fn read_review_finding(r: &mut Reader<'_>) -> Result<d::ReviewFindingV1, CodecErrorV1> {
    Ok(d::ReviewFindingV1 {
        id: d::FindingIdV1(r.raw()?),
        source_report: d::ReportIdV1(r.raw()?),
        owner: r.raw()?,
        finding_digest: r.raw()?,
        evidence_digests: read_many(r, |r| r.raw())?,
        requires_human_gate: read_boolean(r)?,
    })
}

fn rebuttal(w: &mut Writer, value: &d::RebuttalV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    w.raw(&value.finding.0)?;
    w.raw(&value.member.0)?;
    w.raw(&value.rebuttal_digest)?;
    digests(w, &value.evidence_digests)
}

fn read_rebuttal(r: &mut Reader<'_>) -> Result<d::RebuttalV1, CodecErrorV1> {
    Ok(d::RebuttalV1 {
        id: d::RebuttalIdV1(r.raw()?),
        finding: d::FindingIdV1(r.raw()?),
        member: d::EnsembleMemberIdV1(r.raw()?),
        rebuttal_digest: r.raw()?,
        evidence_digests: read_many(r, |r| r.raw())?,
    })
}

fn finding_disposition(
    w: &mut Writer,
    value: &d::FindingDispositionV1,
) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    w.raw(&value.finding.0)?;
    w.raw(&value.owner)?;
    w.byte(finding_disposition_kind_tag(value.kind))?;
    w.raw(&value.rationale_digest)?;
    digests(w, &value.evidence_digests)?;
    w.option(value.human_gate_response.as_ref(), |w, value| {
        w.raw(&value.0)
    })?;
    w.u64(value.decided_at)
}

fn read_finding_disposition(r: &mut Reader<'_>) -> Result<d::FindingDispositionV1, CodecErrorV1> {
    Ok(d::FindingDispositionV1 {
        id: d::FindingDispositionIdV1(r.raw()?),
        finding: d::FindingIdV1(r.raw()?),
        owner: r.raw()?,
        kind: read_finding_disposition_kind(r.byte()?)?,
        rationale_digest: r.raw()?,
        evidence_digests: read_many(r, |r| r.raw())?,
        human_gate_response: r.option(|r| r.raw().map(d::HumanGateResponseIdV1))?,
        decided_at: r.u64()?,
    })
}

fn finding_disposition_kind_tag(value: d::FindingDispositionKindV1) -> u8 {
    match value {
        d::FindingDispositionKindV1::Accepted => 0,
        d::FindingDispositionKindV1::Mitigated => 1,
        d::FindingDispositionKindV1::Tested => 2,
        d::FindingDispositionKindV1::Deferred => 3,
        d::FindingDispositionKindV1::RiskAccepted => 4,
        d::FindingDispositionKindV1::Disputed => 5,
        d::FindingDispositionKindV1::OutOfScope => 6,
    }
}

fn read_finding_disposition_kind(value: u8) -> Result<d::FindingDispositionKindV1, CodecErrorV1> {
    match value {
        0 => Ok(d::FindingDispositionKindV1::Accepted),
        1 => Ok(d::FindingDispositionKindV1::Mitigated),
        2 => Ok(d::FindingDispositionKindV1::Tested),
        3 => Ok(d::FindingDispositionKindV1::Deferred),
        4 => Ok(d::FindingDispositionKindV1::RiskAccepted),
        5 => Ok(d::FindingDispositionKindV1::Disputed),
        6 => Ok(d::FindingDispositionKindV1::OutOfScope),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn review_phase_tag(value: d::ReviewPhaseV1) -> u8 {
    match value {
        d::ReviewPhaseV1::FirstPassComplete => 0,
        d::ReviewPhaseV1::Challenged => 1,
        d::ReviewPhaseV1::Disposed => 2,
    }
}

fn read_review_phase(value: u8) -> Result<d::ReviewPhaseV1, CodecErrorV1> {
    match value {
        0 => Ok(d::ReviewPhaseV1::FirstPassComplete),
        1 => Ok(d::ReviewPhaseV1::Challenged),
        2 => Ok(d::ReviewPhaseV1::Disposed),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn human_gate_request(w: &mut Writer, value: d::HumanGateRequestV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    w.raw(&value.reviewer)?;
    w.raw(&value.finding_digest)?;
    w.raw(&value.decision_digest)?;
    w.raw(&value.material_revision_digest)?;
    w.raw(&value.policy_digest)?;
    human_gate_material_ref(w, value.decision_packet)?;
    w.u64(value.requested_at)
}

fn read_human_gate_request(r: &mut Reader<'_>) -> Result<d::HumanGateRequestV1, CodecErrorV1> {
    Ok(d::HumanGateRequestV1 {
        id: d::HumanGateRequestIdV1(r.raw()?),
        reviewer: r.raw()?,
        finding_digest: r.raw()?,
        decision_digest: r.raw()?,
        material_revision_digest: r.raw()?,
        policy_digest: r.raw()?,
        decision_packet: read_human_gate_material_ref(r)?,
        requested_at: r.u64()?,
    })
}

fn human_gate_material_ref(
    w: &mut Writer,
    value: d::HumanGateMaterialRefV1,
) -> Result<(), CodecErrorV1> {
    w.raw(&value.owner_route)?;
    w.raw(&value.artifact_route)?;
    w.u64(value.expected_revision)?;
    w.raw(&value.expected_sha256)
}

fn read_human_gate_material_ref(
    r: &mut Reader<'_>,
) -> Result<d::HumanGateMaterialRefV1, CodecErrorV1> {
    Ok(d::HumanGateMaterialRefV1 {
        owner_route: r.raw()?,
        artifact_route: r.raw()?,
        expected_revision: r.u64()?,
        expected_sha256: r.raw()?,
    })
}

fn human_gate_submission(
    w: &mut Writer,
    value: d::HumanGateResponseSubmissionV1,
) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    w.raw(&value.request.0)?;
    w.byte(human_gate_disposition_tag(value.disposition))?;
    w.raw(&value.rationale_digest)?;
    human_gate_material_ref(w, value.rationale)?;
    w.u64(value.responded_at)
}

fn read_human_gate_submission(
    r: &mut Reader<'_>,
) -> Result<d::HumanGateResponseSubmissionV1, CodecErrorV1> {
    Ok(d::HumanGateResponseSubmissionV1 {
        id: d::HumanGateResponseIdV1(r.raw()?),
        request: d::HumanGateRequestIdV1(r.raw()?),
        disposition: read_human_gate_disposition(r.byte()?)?,
        rationale_digest: r.raw()?,
        rationale: read_human_gate_material_ref(r)?,
        responded_at: r.u64()?,
    })
}

fn human_gate_operation(
    w: &mut Writer,
    value: d::HumanGateOperationV1,
) -> Result<(), CodecErrorV1> {
    w.raw(&value.deliberation.0)?;
    w.u64(value.expected_revision)?;
    human_gate_submission(w, value.response)
}

fn read_human_gate_operation(r: &mut Reader<'_>) -> Result<d::HumanGateOperationV1, CodecErrorV1> {
    Ok(d::HumanGateOperationV1 {
        deliberation: d::DeliberationIdV1(r.raw()?),
        expected_revision: r.u64()?,
        response: read_human_gate_submission(r)?,
    })
}

fn human_gate_response(w: &mut Writer, value: d::HumanGateResponseV1) -> Result<(), CodecErrorV1> {
    human_gate_submission(
        w,
        d::HumanGateResponseSubmissionV1 {
            id: value.id,
            request: value.request,
            disposition: value.disposition,
            rationale_digest: value.rationale_digest,
            rationale: value.rationale,
            responded_at: value.responded_at,
        },
    )?;
    w.raw(&value.authority.decision)?;
    w.raw(&value.authority.subject)?;
    w.u64(value.authority.epoch)
}

fn read_human_gate_response(r: &mut Reader<'_>) -> Result<d::HumanGateResponseV1, CodecErrorV1> {
    let value = read_human_gate_submission(r)?;
    Ok(d::HumanGateResponseV1 {
        id: value.id,
        request: value.request,
        disposition: value.disposition,
        rationale_digest: value.rationale_digest,
        rationale: value.rationale,
        responded_at: value.responded_at,
        authority: d::HumanGateAuthorityRefV1 {
            decision: r.raw()?,
            subject: r.raw()?,
            epoch: r.u64()?,
        },
    })
}

fn human_gate_disposition_tag(value: d::HumanGateDispositionV1) -> u8 {
    match value {
        d::HumanGateDispositionV1::Approved => 0,
        d::HumanGateDispositionV1::Rejected => 1,
        d::HumanGateDispositionV1::Deferred => 2,
        d::HumanGateDispositionV1::ChangesRequested => 3,
    }
}

fn read_human_gate_disposition(value: u8) -> Result<d::HumanGateDispositionV1, CodecErrorV1> {
    match value {
        0 => Ok(d::HumanGateDispositionV1::Approved),
        1 => Ok(d::HumanGateDispositionV1::Rejected),
        2 => Ok(d::HumanGateDispositionV1::Deferred),
        3 => Ok(d::HumanGateDispositionV1::ChangesRequested),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn plan(w: &mut Writer, value: &d::DeliberationPlanV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    w.raw(&value.question_digest)?;
    method(w, value.method)?;
    write_len(w, value.claims.len())?;
    for value in &value.claims {
        w.byte(claim_tag(*value))?;
    }
    write_len(w, value.criteria.len())?;
    for value in &value.criteria {
        criterion(w, *value)?;
    }
    write_len(w, value.candidates.len())?;
    for value in &value.candidates {
        candidate(w, value)?;
    }
    admission(w, value.admission)?;
    write_len(w, value.members.len())?;
    for value in &value.members {
        member(w, *value)?;
    }
    w.raw(&value.orthogonality.query_digest)?;
    w.raw(&value.orthogonality.result_digest)?;
    boolean(w, value.orthogonality.all_orthogonal)?;
    w.u32(value.fuel)
}

fn read_plan(r: &mut Reader<'_>) -> Result<d::DeliberationPlanV1, CodecErrorV1> {
    let id = d::DeliberationIdV1(r.raw()?);
    let question_digest = r.raw()?;
    let method = read_method(r)?;
    let claims = read_many(r, |r| read_claim(r.byte()?))?;
    let criteria = read_many(r, read_criterion)?;
    let candidates = read_many(r, read_candidate)?;
    let admission = read_admission(r)?;
    let members = read_many(r, read_member)?;
    Ok(d::DeliberationPlanV1 {
        id,
        question_digest,
        method,
        claims,
        criteria,
        candidates,
        admission,
        members,
        orthogonality: d::OrthogonalityEvidenceV1 {
            query_digest: r.raw()?,
            result_digest: r.raw()?,
            all_orthogonal: read_boolean(r)?,
        },
        fuel: r.u32()?,
    })
}

fn report(w: &mut Writer, value: &d::MemberReportV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    w.raw(&value.member.0)?;
    w.raw(&value.output_digest)?;
    digests(w, &value.evidence_digests)?;
    digests(w, &value.dissent_digests)
}

fn read_report(r: &mut Reader<'_>) -> Result<d::MemberReportV1, CodecErrorV1> {
    Ok(d::MemberReportV1 {
        id: d::ReportIdV1(r.raw()?),
        member: d::EnsembleMemberIdV1(r.raw()?),
        output_digest: r.raw()?,
        evidence_digests: read_many(r, |r| r.raw())?,
        dissent_digests: read_many(r, |r| r.raw())?,
    })
}

fn resolution(w: &mut Writer, value: &d::ResolutionV1) -> Result<(), CodecErrorV1> {
    match value {
        d::ResolutionV1::ProvenBool {
            value,
            validator,
            evidence_digest,
        } => {
            w.byte(0)?;
            boolean(w, *value)?;
            w.raw(validator)?;
            w.raw(evidence_digest)
        }
        d::ResolutionV1::AuthorizedChoice {
            frontier,
            selected,
            authority,
        } => {
            w.byte(1)?;
            write_len(w, frontier.len())?;
            for value in frontier {
                w.raw(&value.0)?;
            }
            w.raw(&selected.0)?;
            w.raw(&authority.decision.0)?;
            w.raw(&authority.subject.0)?;
            w.u64(authority.epoch)
        }
        d::ResolutionV1::Maybe(value) => {
            w.byte(2)?;
            w.byte(maybe_tag(*value))
        }
        d::ResolutionV1::Mu(value) => {
            w.byte(3)?;
            w.byte(mu_tag(*value))
        }
    }
}

fn read_resolution(r: &mut Reader<'_>) -> Result<d::ResolutionV1, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(d::ResolutionV1::ProvenBool {
            value: read_boolean(r)?,
            validator: r.raw()?,
            evidence_digest: r.raw()?,
        }),
        1 => Ok(d::ResolutionV1::AuthorizedChoice {
            frontier: read_many(r, |r| r.raw().map(d::CandidateIdV1))?,
            selected: d::CandidateIdV1(r.raw()?),
            authority: d::AuthorityRefV1 {
                decision: d::AuthorityDecisionIdV1(r.raw()?),
                subject: d::AuthoritySubjectIdV1(r.raw()?),
                epoch: r.u64()?,
            },
        }),
        2 => Ok(d::ResolutionV1::Maybe(read_maybe(r.byte()?)?)),
        3 => Ok(d::ResolutionV1::Mu(read_mu(r.byte()?)?)),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn criterion(w: &mut Writer, value: d::CriterionV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    w.byte(match value.direction {
        d::CriterionDirectionV1::Maximize => 0,
        d::CriterionDirectionV1::Minimize => 1,
    })
}

fn read_criterion(r: &mut Reader<'_>) -> Result<d::CriterionV1, CodecErrorV1> {
    Ok(d::CriterionV1 {
        id: d::CriterionIdV1(r.raw()?),
        direction: match r.byte()? {
            0 => d::CriterionDirectionV1::Maximize,
            1 => d::CriterionDirectionV1::Minimize,
            _ => return Err(CodecErrorV1::Unsupported),
        },
    })
}

fn candidate(w: &mut Writer, value: &d::CandidateV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    write_len(w, value.scores.len())?;
    for value in &value.scores {
        w.raw(&value.to_be_bytes())?;
    }
    Ok(())
}

fn read_candidate(r: &mut Reader<'_>) -> Result<d::CandidateV1, CodecErrorV1> {
    Ok(d::CandidateV1 {
        id: d::CandidateIdV1(r.raw()?),
        scores: read_many(r, |r| Ok(i64::from_be_bytes(r.raw()?)))?,
    })
}

fn member(w: &mut Writer, value: d::EnsembleMemberV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.id.0)?;
    stakeholder(w, value.stakeholder)?;
    w.byte(persona_tag(value.lens))?;
    w.raw(&value.concern_root_digest)?;
    grounding(w, value.grounding)
}

fn read_member(r: &mut Reader<'_>) -> Result<d::EnsembleMemberV1, CodecErrorV1> {
    Ok(d::EnsembleMemberV1 {
        id: d::EnsembleMemberIdV1(r.raw()?),
        stakeholder: read_stakeholder(r)?,
        lens: read_persona(r.byte()?)?,
        concern_root_digest: r.raw()?,
        grounding: read_grounding(r)?,
    })
}

fn admission(w: &mut Writer, value: d::EnsembleAdmissionV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.affected_population_digest)?;
    w.raw(&value.stakeholder_map_digest)?;
    w.raw(&value.coverage_manifest_digest)?;
    w.raw(&value.equity_assessment_digest)?;
    w.raw(&value.assessor_principal)?;
    w.byte(fitness_tag(value.fitness))
}

fn read_admission(r: &mut Reader<'_>) -> Result<d::EnsembleAdmissionV1, CodecErrorV1> {
    Ok(d::EnsembleAdmissionV1 {
        affected_population_digest: r.raw()?,
        stakeholder_map_digest: r.raw()?,
        coverage_manifest_digest: r.raw()?,
        equity_assessment_digest: r.raw()?,
        assessor_principal: r.raw()?,
        fitness: read_fitness(r.byte()?)?,
    })
}

fn stakeholder(w: &mut Writer, value: d::StakeholderRefV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.principal)?;
    w.raw(&value.constituency_digest)?;
    w.byte(representation_tag(value.representation))?;
    w.option(value.accountable_principal.as_ref(), |w, value| {
        w.raw(value)
    })?;
    w.option(value.mandate_digest.as_ref(), |w, value| w.raw(value))?;
    w.option(value.conflict_digest.as_ref(), |w, value| w.raw(value))
}

fn read_stakeholder(r: &mut Reader<'_>) -> Result<d::StakeholderRefV1, CodecErrorV1> {
    Ok(d::StakeholderRefV1 {
        principal: r.raw()?,
        constituency_digest: r.raw()?,
        representation: read_representation(r.byte()?)?,
        accountable_principal: r.option(|r| r.raw())?,
        mandate_digest: r.option(|r| r.raw())?,
        conflict_digest: r.option(|r| r.raw())?,
    })
}

fn grounding(w: &mut Writer, value: d::GroundingRefV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.authority_digest)?;
    w.raw(&value.surface_digest)?;
    w.byte(match value.depth {
        d::SurfaceDepthV1::Ambient => 0,
        d::SurfaceDepthV1::Activated => 1,
        d::SurfaceDepthV1::Reference => 2,
    })
}

fn read_grounding(r: &mut Reader<'_>) -> Result<d::GroundingRefV1, CodecErrorV1> {
    Ok(d::GroundingRefV1 {
        authority_digest: r.raw()?,
        surface_digest: r.raw()?,
        depth: match r.byte()? {
            0 => d::SurfaceDepthV1::Ambient,
            1 => d::SurfaceDepthV1::Activated,
            2 => d::SurfaceDepthV1::Reference,
            _ => return Err(CodecErrorV1::Unsupported),
        },
    })
}

fn method(w: &mut Writer, value: d::MethodRefV1) -> Result<(), CodecErrorV1> {
    w.raw(&value.machine_digest)?;
    w.raw(&value.state_digest)
}

fn read_method(r: &mut Reader<'_>) -> Result<d::MethodRefV1, CodecErrorV1> {
    Ok(d::MethodRefV1 {
        machine_digest: r.raw()?,
        state_digest: r.raw()?,
    })
}

fn limits(w: &mut Writer, value: d::DeliberationLimitsV1) -> Result<(), CodecErrorV1> {
    w.usize(value.max_claims)?;
    w.usize(value.max_criteria)?;
    w.usize(value.max_candidates)?;
    w.usize(value.max_members)?;
    w.usize(value.max_evidence_per_report)?;
    w.usize(value.max_dissent_per_report)?;
    w.u32(value.max_fuel)
}

fn read_limits(r: &mut Reader<'_>) -> Result<d::DeliberationLimitsV1, CodecErrorV1> {
    Ok(d::DeliberationLimitsV1 {
        max_claims: r.usize()?,
        max_criteria: r.usize()?,
        max_candidates: r.usize()?,
        max_members: r.usize()?,
        max_evidence_per_report: r.usize()?,
        max_dissent_per_report: r.usize()?,
        max_fuel: r.u32()?,
    })
}

fn digests(w: &mut Writer, values: &[[u8; 32]]) -> Result<(), CodecErrorV1> {
    write_len(w, values.len())?;
    for value in values {
        w.raw(value)?;
    }
    Ok(())
}

fn read_many<T>(
    r: &mut Reader<'_>,
    mut read: impl FnMut(&mut Reader<'_>) -> Result<T, CodecErrorV1>,
) -> Result<Vec<T>, CodecErrorV1> {
    let count = usize::try_from(r.u32()?).map_err(|_| CodecErrorV1::Limit)?;
    if count > r.bytes.len().saturating_sub(r.position) {
        return Err(CodecErrorV1::Limit);
    }
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..count {
        values.push(read(r)?);
    }
    Ok(values)
}

fn write_len(w: &mut Writer, value: usize) -> Result<(), CodecErrorV1> {
    w.u32(u32::try_from(value).map_err(|_| CodecErrorV1::Limit)?)
}

fn boolean(w: &mut Writer, value: bool) -> Result<(), CodecErrorV1> {
    w.byte(u8::from(value))
}

fn read_boolean(r: &mut Reader<'_>) -> Result<bool, CodecErrorV1> {
    match r.byte()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CodecErrorV1::Malformed),
    }
}

fn refusal(w: &mut Writer, value: d::RefusalV1) -> Result<(), CodecErrorV1> {
    match value {
        d::RefusalV1::NotAuthorized => w.byte(0),
        d::RefusalV1::InvalidLimits => w.byte(1),
        d::RefusalV1::UnsupportedVersion => w.byte(2),
        d::RefusalV1::RevisionConflict { expected, actual } => {
            w.byte(3)?;
            w.u64(expected)?;
            w.u64(actual)
        }
        d::RefusalV1::RevisionExhausted => w.byte(4),
        d::RefusalV1::Capacity => w.byte(5),
        d::RefusalV1::UnknownDeliberation => w.byte(6),
        d::RefusalV1::InputConflict => w.byte(7),
        d::RefusalV1::DuplicateDeliberation => w.byte(8),
        d::RefusalV1::CheckpointMismatch => w.byte(9),
        d::RefusalV1::Deliberation(value) => {
            w.byte(10)?;
            deliberation_refusal(w, value)
        }
        d::RefusalV1::Review(value) => {
            w.byte(11)?;
            w.byte(review_refusal_tag(value))
        }
        d::RefusalV1::DuplicateReview => w.byte(12),
        d::RefusalV1::ReviewNotBegun => w.byte(13),
    }
}

fn read_refusal(r: &mut Reader<'_>) -> Result<d::RefusalV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => d::RefusalV1::NotAuthorized,
        1 => d::RefusalV1::InvalidLimits,
        2 => d::RefusalV1::UnsupportedVersion,
        3 => d::RefusalV1::RevisionConflict {
            expected: r.u64()?,
            actual: r.u64()?,
        },
        4 => d::RefusalV1::RevisionExhausted,
        5 => d::RefusalV1::Capacity,
        6 => d::RefusalV1::UnknownDeliberation,
        7 => d::RefusalV1::InputConflict,
        8 => d::RefusalV1::DuplicateDeliberation,
        9 => d::RefusalV1::CheckpointMismatch,
        10 => d::RefusalV1::Deliberation(read_deliberation_refusal(r)?),
        11 => d::RefusalV1::Review(read_review_refusal(r.byte()?)?),
        12 => d::RefusalV1::DuplicateReview,
        13 => d::RefusalV1::ReviewNotBegun,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn review_refusal_tag(value: d::ReviewRefusalV1) -> u8 {
    match value {
        d::ReviewRefusalV1::InvalidLimits => 0,
        d::ReviewRefusalV1::FirstPassIncomplete => 1,
        d::ReviewRefusalV1::FindingLimit => 2,
        d::ReviewRefusalV1::RebuttalLimit => 3,
        d::ReviewRefusalV1::EvidenceLimit => 4,
        d::ReviewRefusalV1::InvalidFinding => 5,
        d::ReviewRefusalV1::InvalidRebuttal => 6,
        d::ReviewRefusalV1::InvalidDisposition => 7,
        d::ReviewRefusalV1::UnknownReport => 8,
        d::ReviewRefusalV1::UnknownFinding => 9,
        d::ReviewRefusalV1::UnknownMember => 10,
        d::ReviewRefusalV1::SelfRebuttal => 11,
        d::ReviewRefusalV1::FindingConflict => 12,
        d::ReviewRefusalV1::RebuttalConflict => 13,
        d::ReviewRefusalV1::DispositionConflict => 14,
        d::ReviewRefusalV1::AlreadyDisposed => 15,
        d::ReviewRefusalV1::RebuttalRequired => 16,
        d::ReviewRefusalV1::OwnerMismatch => 17,
        d::ReviewRefusalV1::HumanGateRequired => 18,
        d::ReviewRefusalV1::HumanGateNotApproved => 19,
        d::ReviewRefusalV1::HumanGateMismatch => 20,
        d::ReviewRefusalV1::InvalidReports => 21,
        d::ReviewRefusalV1::DeliberationMismatch => 22,
        d::ReviewRefusalV1::UnsupportedVersion => 23,
        d::ReviewRefusalV1::CheckpointMismatch => 24,
        d::ReviewRefusalV1::InvalidCorrelationEvidence => 25,
    }
}

fn read_review_refusal(value: u8) -> Result<d::ReviewRefusalV1, CodecErrorV1> {
    match value {
        0 => Ok(d::ReviewRefusalV1::InvalidLimits),
        1 => Ok(d::ReviewRefusalV1::FirstPassIncomplete),
        2 => Ok(d::ReviewRefusalV1::FindingLimit),
        3 => Ok(d::ReviewRefusalV1::RebuttalLimit),
        4 => Ok(d::ReviewRefusalV1::EvidenceLimit),
        5 => Ok(d::ReviewRefusalV1::InvalidFinding),
        6 => Ok(d::ReviewRefusalV1::InvalidRebuttal),
        7 => Ok(d::ReviewRefusalV1::InvalidDisposition),
        8 => Ok(d::ReviewRefusalV1::UnknownReport),
        9 => Ok(d::ReviewRefusalV1::UnknownFinding),
        10 => Ok(d::ReviewRefusalV1::UnknownMember),
        11 => Ok(d::ReviewRefusalV1::SelfRebuttal),
        12 => Ok(d::ReviewRefusalV1::FindingConflict),
        13 => Ok(d::ReviewRefusalV1::RebuttalConflict),
        14 => Ok(d::ReviewRefusalV1::DispositionConflict),
        15 => Ok(d::ReviewRefusalV1::AlreadyDisposed),
        16 => Ok(d::ReviewRefusalV1::RebuttalRequired),
        17 => Ok(d::ReviewRefusalV1::OwnerMismatch),
        18 => Ok(d::ReviewRefusalV1::HumanGateRequired),
        19 => Ok(d::ReviewRefusalV1::HumanGateNotApproved),
        20 => Ok(d::ReviewRefusalV1::HumanGateMismatch),
        21 => Ok(d::ReviewRefusalV1::InvalidReports),
        22 => Ok(d::ReviewRefusalV1::DeliberationMismatch),
        23 => Ok(d::ReviewRefusalV1::UnsupportedVersion),
        24 => Ok(d::ReviewRefusalV1::CheckpointMismatch),
        25 => Ok(d::ReviewRefusalV1::InvalidCorrelationEvidence),
        _ => Err(CodecErrorV1::Unsupported),
    }
}

fn deliberation_refusal(
    w: &mut Writer,
    value: d::DeliberationRefusalV1,
) -> Result<(), CodecErrorV1> {
    let tag = match value {
        d::DeliberationRefusalV1::UnsupportedVersion => 0,
        d::DeliberationRefusalV1::InvalidLimits => 1,
        d::DeliberationRefusalV1::EmptyClaims => 2,
        d::DeliberationRefusalV1::ClaimLimit => 3,
        d::DeliberationRefusalV1::CriterionLimit => 4,
        d::DeliberationRefusalV1::CandidateLimit => 5,
        d::DeliberationRefusalV1::EmptyEnsemble => 6,
        d::DeliberationRefusalV1::MemberLimit => 7,
        d::DeliberationRefusalV1::DuplicateMember => 8,
        d::DeliberationRefusalV1::InvalidStakeholder => 22,
        d::DeliberationRefusalV1::InvalidEnsembleAdmission => 23,
        d::DeliberationRefusalV1::EnsembleNotAdmitted => 24,
        d::DeliberationRefusalV1::NonOrthogonalEnsemble => 9,
        d::DeliberationRefusalV1::FuelInvalid => 10,
        d::DeliberationRefusalV1::Frontier(value) => {
            w.byte(11)?;
            return w.byte(frontier_refusal_tag(value));
        }
        d::DeliberationRefusalV1::UnknownMember => 12,
        d::DeliberationRefusalV1::ReportLimit => 13,
        d::DeliberationRefusalV1::ReportConflict => 14,
        d::DeliberationRefusalV1::AlreadyResolved => 15,
        d::DeliberationRefusalV1::ReportsIncomplete => 16,
        d::DeliberationRefusalV1::GateBeforeResolution => 25,
        d::DeliberationRefusalV1::InvalidHumanGateRequest => 26,
        d::DeliberationRefusalV1::HumanGateRequestConflict => 27,
        d::DeliberationRefusalV1::HumanGateNotRequested => 28,
        d::DeliberationRefusalV1::InvalidHumanGateResponse => 29,
        d::DeliberationRefusalV1::HumanGateReviewerMismatch => 30,
        d::DeliberationRefusalV1::HumanGateResponseConflict => 31,
        d::DeliberationRefusalV1::ResolutionConflict => 17,
        d::DeliberationRefusalV1::FrontierMismatch => 18,
        d::DeliberationRefusalV1::SelectionOutsideFrontier => 19,
        d::DeliberationRefusalV1::FuelRemaining => 20,
        d::DeliberationRefusalV1::CheckpointMismatch => 21,
    };
    w.byte(tag)
}

fn read_deliberation_refusal(r: &mut Reader<'_>) -> Result<d::DeliberationRefusalV1, CodecErrorV1> {
    Ok(match r.byte()? {
        0 => d::DeliberationRefusalV1::UnsupportedVersion,
        1 => d::DeliberationRefusalV1::InvalidLimits,
        2 => d::DeliberationRefusalV1::EmptyClaims,
        3 => d::DeliberationRefusalV1::ClaimLimit,
        4 => d::DeliberationRefusalV1::CriterionLimit,
        5 => d::DeliberationRefusalV1::CandidateLimit,
        6 => d::DeliberationRefusalV1::EmptyEnsemble,
        7 => d::DeliberationRefusalV1::MemberLimit,
        8 => d::DeliberationRefusalV1::DuplicateMember,
        22 => d::DeliberationRefusalV1::InvalidStakeholder,
        23 => d::DeliberationRefusalV1::InvalidEnsembleAdmission,
        24 => d::DeliberationRefusalV1::EnsembleNotAdmitted,
        9 => d::DeliberationRefusalV1::NonOrthogonalEnsemble,
        10 => d::DeliberationRefusalV1::FuelInvalid,
        11 => d::DeliberationRefusalV1::Frontier(read_frontier_refusal(r.byte()?)?),
        12 => d::DeliberationRefusalV1::UnknownMember,
        13 => d::DeliberationRefusalV1::ReportLimit,
        14 => d::DeliberationRefusalV1::ReportConflict,
        15 => d::DeliberationRefusalV1::AlreadyResolved,
        16 => d::DeliberationRefusalV1::ReportsIncomplete,
        25 => d::DeliberationRefusalV1::GateBeforeResolution,
        26 => d::DeliberationRefusalV1::InvalidHumanGateRequest,
        27 => d::DeliberationRefusalV1::HumanGateRequestConflict,
        28 => d::DeliberationRefusalV1::HumanGateNotRequested,
        29 => d::DeliberationRefusalV1::InvalidHumanGateResponse,
        30 => d::DeliberationRefusalV1::HumanGateReviewerMismatch,
        31 => d::DeliberationRefusalV1::HumanGateResponseConflict,
        17 => d::DeliberationRefusalV1::ResolutionConflict,
        18 => d::DeliberationRefusalV1::FrontierMismatch,
        19 => d::DeliberationRefusalV1::SelectionOutsideFrontier,
        20 => d::DeliberationRefusalV1::FuelRemaining,
        21 => d::DeliberationRefusalV1::CheckpointMismatch,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn claim_tag(value: d::ClaimKindV1) -> u8 {
    match value {
        d::ClaimKindV1::Formal => 0,
        d::ClaimKindV1::Empirical => 1,
        d::ClaimKindV1::Normative => 2,
        d::ClaimKindV1::Historical => 3,
        d::ClaimKindV1::Creative => 4,
    }
}

fn read_claim(value: u8) -> Result<d::ClaimKindV1, CodecErrorV1> {
    Ok(match value {
        0 => d::ClaimKindV1::Formal,
        1 => d::ClaimKindV1::Empirical,
        2 => d::ClaimKindV1::Normative,
        3 => d::ClaimKindV1::Historical,
        4 => d::ClaimKindV1::Creative,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn persona_tag(value: d::PersonaLensV1) -> u8 {
    match value {
        d::PersonaLensV1::Historian => 0,
        d::PersonaLensV1::IdeaStormer => 1,
        d::PersonaLensV1::Theorist => 2,
        d::PersonaLensV1::Researcher => 3,
        d::PersonaLensV1::RigorReviewer => 4,
        d::PersonaLensV1::ConvergenceFacilitator => 5,
    }
}

fn representation_tag(value: d::StakeholderRepresentationV1) -> u8 {
    match value {
        d::StakeholderRepresentationV1::ActualStakeholder => 0,
        d::StakeholderRepresentationV1::AuthorizedRepresentative => 1,
        d::StakeholderRepresentationV1::AgentModeledPerspective => 2,
        d::StakeholderRepresentationV1::TechnicalSme => 3,
    }
}

fn read_representation(value: u8) -> Result<d::StakeholderRepresentationV1, CodecErrorV1> {
    Ok(match value {
        0 => d::StakeholderRepresentationV1::ActualStakeholder,
        1 => d::StakeholderRepresentationV1::AuthorizedRepresentative,
        2 => d::StakeholderRepresentationV1::AgentModeledPerspective,
        3 => d::StakeholderRepresentationV1::TechnicalSme,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn fitness_tag(value: d::EnsembleFitnessV1) -> u8 {
    match value {
        d::EnsembleFitnessV1::Yes => 0,
        d::EnsembleFitnessV1::No => 1,
        d::EnsembleFitnessV1::Contested => 2,
    }
}

fn read_fitness(value: u8) -> Result<d::EnsembleFitnessV1, CodecErrorV1> {
    Ok(match value {
        0 => d::EnsembleFitnessV1::Yes,
        1 => d::EnsembleFitnessV1::No,
        2 => d::EnsembleFitnessV1::Contested,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn read_persona(value: u8) -> Result<d::PersonaLensV1, CodecErrorV1> {
    Ok(match value {
        0 => d::PersonaLensV1::Historian,
        1 => d::PersonaLensV1::IdeaStormer,
        2 => d::PersonaLensV1::Theorist,
        3 => d::PersonaLensV1::Researcher,
        4 => d::PersonaLensV1::RigorReviewer,
        5 => d::PersonaLensV1::ConvergenceFacilitator,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn phase_tag(value: d::DeliberationPhaseV1) -> u8 {
    match value {
        d::DeliberationPhaseV1::Collecting => 0,
        d::DeliberationPhaseV1::ReadyToResolve => 1,
        d::DeliberationPhaseV1::Resolved => 2,
    }
}

fn read_phase(value: u8) -> Result<d::DeliberationPhaseV1, CodecErrorV1> {
    Ok(match value {
        0 => d::DeliberationPhaseV1::Collecting,
        1 => d::DeliberationPhaseV1::ReadyToResolve,
        2 => d::DeliberationPhaseV1::Resolved,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn maybe_tag(value: d::MaybeValueV1) -> u8 {
    match value {
        d::MaybeValueV1::TrueOnly => 0,
        d::MaybeValueV1::FalseOnly => 1,
        d::MaybeValueV1::Both => 2,
        d::MaybeValueV1::Neither => 3,
    }
}

fn read_maybe(value: u8) -> Result<d::MaybeValueV1, CodecErrorV1> {
    Ok(match value {
        0 => d::MaybeValueV1::TrueOnly,
        1 => d::MaybeValueV1::FalseOnly,
        2 => d::MaybeValueV1::Both,
        3 => d::MaybeValueV1::Neither,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn mu_tag(value: d::MuReasonV1) -> u8 {
    match value {
        d::MuReasonV1::FuelExhausted => 0,
        d::MuReasonV1::MissingEvidence => 1,
        d::MuReasonV1::ContradictoryEvidence => 2,
        d::MuReasonV1::WrongQuestion => 3,
        d::MuReasonV1::WrongEnsemble => 4,
        d::MuReasonV1::NoAuthorizedSelector => 5,
    }
}

fn read_mu(value: u8) -> Result<d::MuReasonV1, CodecErrorV1> {
    Ok(match value {
        0 => d::MuReasonV1::FuelExhausted,
        1 => d::MuReasonV1::MissingEvidence,
        2 => d::MuReasonV1::ContradictoryEvidence,
        3 => d::MuReasonV1::WrongQuestion,
        4 => d::MuReasonV1::WrongEnsemble,
        5 => d::MuReasonV1::NoAuthorizedSelector,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}

fn frontier_refusal_tag(value: d::FrontierRefusalV1) -> u8 {
    match value {
        d::FrontierRefusalV1::NoCriteria => 0,
        d::FrontierRefusalV1::NoCandidates => 1,
        d::FrontierRefusalV1::DuplicateCriterion => 2,
        d::FrontierRefusalV1::DuplicateCandidate => 3,
        d::FrontierRefusalV1::ScoreDimensionMismatch => 4,
    }
}

fn read_frontier_refusal(value: u8) -> Result<d::FrontierRefusalV1, CodecErrorV1> {
    Ok(match value {
        0 => d::FrontierRefusalV1::NoCriteria,
        1 => d::FrontierRefusalV1::NoCandidates,
        2 => d::FrontierRefusalV1::DuplicateCriterion,
        3 => d::FrontierRefusalV1::DuplicateCandidate,
        4 => d::FrontierRefusalV1::ScoreDimensionMismatch,
        _ => return Err(CodecErrorV1::Unsupported),
    })
}
