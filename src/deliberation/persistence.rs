//! Canonical bounded bytes for one methodical deliberation checkpoint.

use super::*;

const MAGIC: &[u8; 8] = b"MOIRADL5";
const CODEC_VERSION: u8 = 5;
const LEGACY_MAGIC: &[u8; 8] = b"MOIRADL3";
const LEGACY_CODEC_VERSION: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecLimitsV1 {
    pub max_bytes: usize,
    pub domain: DeliberationLimitsV1,
}

impl Default for CodecLimitsV1 {
    fn default() -> Self {
        Self {
            max_bytes: 8 * 1024 * 1024,
            domain: DeliberationLimitsV1::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecErrorV1 {
    InvalidLimits,
    Limit,
    Capacity,
    Truncated,
    Invalid,
    NonCanonical,
}

struct Writer {
    bytes: Vec<u8>,
    maximum: usize,
}

impl Writer {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
        }
    }
    fn raw(&mut self, value: &[u8]) -> Result<(), CodecErrorV1> {
        if value.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(CodecErrorV1::Limit);
        }
        self.bytes
            .try_reserve(value.len())
            .map_err(|_| CodecErrorV1::Capacity)?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }
    fn byte(&mut self, value: u8) -> Result<(), CodecErrorV1> {
        self.raw(&[value])
    }
    fn number(&mut self, value: u64) -> Result<(), CodecErrorV1> {
        self.raw(&value.to_be_bytes())
    }
    fn signed(&mut self, value: i64) -> Result<(), CodecErrorV1> {
        self.raw(&value.to_be_bytes())
    }
    fn id(&mut self, value: [u8; 16]) -> Result<(), CodecErrorV1> {
        self.raw(&value)
    }
    fn digest(&mut self, value: [u8; 32]) -> Result<(), CodecErrorV1> {
        self.raw(&value)
    }
    fn count(&mut self, value: usize) -> Result<(), CodecErrorV1> {
        self.number(u64::try_from(value).map_err(|_| CodecErrorV1::Limit)?)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], maximum: usize) -> Result<Self, CodecErrorV1> {
        if bytes.len() > maximum {
            return Err(CodecErrorV1::Limit);
        }
        Ok(Self { bytes, position: 0 })
    }
    fn raw<const N: usize>(&mut self) -> Result<[u8; N], CodecErrorV1> {
        let end = self.position.checked_add(N).ok_or(CodecErrorV1::Invalid)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(CodecErrorV1::Truncated)?;
        self.position = end;
        value.try_into().map_err(|_| CodecErrorV1::Invalid)
    }
    fn byte(&mut self) -> Result<u8, CodecErrorV1> {
        Ok(self.raw::<1>()?[0])
    }
    fn number(&mut self) -> Result<u64, CodecErrorV1> {
        Ok(u64::from_be_bytes(self.raw()?))
    }
    fn signed(&mut self) -> Result<i64, CodecErrorV1> {
        Ok(i64::from_be_bytes(self.raw()?))
    }
    fn id(&mut self) -> Result<[u8; 16], CodecErrorV1> {
        self.raw()
    }
    fn digest(&mut self) -> Result<[u8; 32], CodecErrorV1> {
        self.raw()
    }
    fn count(&mut self, maximum: usize) -> Result<usize, CodecErrorV1> {
        let count = usize::try_from(self.number()?).map_err(|_| CodecErrorV1::Limit)?;
        if count > maximum {
            return Err(CodecErrorV1::Limit);
        }
        Ok(count)
    }
    fn finish(self) -> Result<(), CodecErrorV1> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(CodecErrorV1::Invalid)
        }
    }
}

pub fn encode(
    checkpoint: &DeliberationCheckpointV1,
    limits: CodecLimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    validate_codec_limits(limits)?;
    validate_host_bounds(checkpoint, limits)?;
    DeliberationV1::recover(checkpoint.clone()).map_err(|_| CodecErrorV1::Invalid)?;
    let mut writer = Writer::new(limits.max_bytes);
    writer.raw(MAGIC)?;
    writer.byte(CODEC_VERSION)?;
    encode_limits(&mut writer, checkpoint.limits)?;
    encode_plan(&mut writer, &checkpoint.plan)?;
    writer.count(checkpoint.reports.len())?;
    for report in &checkpoint.reports {
        encode_report(&mut writer, report)?;
    }
    encode_resolution(&mut writer, checkpoint.resolution.as_ref())?;
    encode_human_gate_request(&mut writer, checkpoint.human_gate_request.as_ref())?;
    encode_human_gate_response(&mut writer, checkpoint.human_gate_response.as_ref())?;
    writer.number(checkpoint.remaining_fuel.into())?;
    Ok(writer.bytes)
}

pub fn decode(
    bytes: &[u8],
    limits: CodecLimitsV1,
) -> Result<DeliberationCheckpointV1, CodecErrorV1> {
    validate_codec_limits(limits)?;
    let mut reader = Reader::new(bytes, limits.max_bytes)?;
    let magic = reader.raw::<8>()?;
    let version = reader.byte()?;
    let legacy = match (&magic, version) {
        (value, CODEC_VERSION) if value == MAGIC => false,
        (value, LEGACY_CODEC_VERSION) if value == LEGACY_MAGIC => true,
        _ => return Err(CodecErrorV1::Invalid),
    };
    let domain = decode_limits(&mut reader, limits.domain)?;
    let plan = decode_plan(&mut reader, domain)?;
    let report_count = reader.count(domain.max_members)?;
    let mut reports = Vec::new();
    reports
        .try_reserve(report_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..report_count {
        reports.push(decode_report(&mut reader, domain)?);
    }
    let resolution = decode_resolution(&mut reader, domain.max_candidates)?;
    let (human_gate_request, human_gate_response) = if legacy {
        (None, None)
    } else {
        (
            decode_human_gate_request(&mut reader)?,
            decode_human_gate_response(&mut reader)?,
        )
    };
    let remaining_fuel = to_u32(reader.number()?)?;
    reader.finish()?;
    let checkpoint = DeliberationCheckpointV1 {
        version: CODEC_VERSION,
        plan,
        limits: domain,
        reports,
        resolution,
        human_gate_request,
        human_gate_response,
        remaining_fuel,
    };
    DeliberationV1::recover(checkpoint.clone()).map_err(|_| CodecErrorV1::Invalid)?;
    if !legacy && encode(&checkpoint, limits)? != bytes {
        return Err(CodecErrorV1::NonCanonical);
    }
    Ok(checkpoint)
}

fn encode_limits(writer: &mut Writer, limits: DeliberationLimitsV1) -> Result<(), CodecErrorV1> {
    for value in [
        limits.max_claims,
        limits.max_criteria,
        limits.max_candidates,
        limits.max_members,
        limits.max_evidence_per_report,
        limits.max_dissent_per_report,
    ] {
        writer.count(value)?;
    }
    writer.number(limits.max_fuel.into())
}

fn decode_limits(
    reader: &mut Reader<'_>,
    host: DeliberationLimitsV1,
) -> Result<DeliberationLimitsV1, CodecErrorV1> {
    let limits = DeliberationLimitsV1 {
        max_claims: reader.count(host.max_claims)?,
        max_criteria: reader.count(host.max_criteria)?,
        max_candidates: reader.count(host.max_candidates)?,
        max_members: reader.count(host.max_members)?,
        max_evidence_per_report: reader.count(host.max_evidence_per_report)?,
        max_dissent_per_report: reader.count(host.max_dissent_per_report)?,
        max_fuel: to_u32(reader.number()?)?,
    };
    validate_domain_limits(limits).map_err(|_| CodecErrorV1::InvalidLimits)?;
    if limits.max_fuel > host.max_fuel {
        return Err(CodecErrorV1::Limit);
    }
    Ok(limits)
}

fn encode_plan(writer: &mut Writer, plan: &DeliberationPlanV1) -> Result<(), CodecErrorV1> {
    writer.id(plan.id.0)?;
    writer.digest(plan.question_digest)?;
    writer.digest(plan.method.machine_digest)?;
    writer.digest(plan.method.state_digest)?;
    writer.count(plan.claims.len())?;
    for claim in &plan.claims {
        writer.byte(claim_tag(*claim))?;
    }
    writer.count(plan.criteria.len())?;
    for criterion in &plan.criteria {
        writer.id(criterion.id.0)?;
        writer.byte(direction_tag(criterion.direction))?;
    }
    writer.count(plan.candidates.len())?;
    for candidate in &plan.candidates {
        writer.id(candidate.id.0)?;
        writer.count(candidate.scores.len())?;
        for score in &candidate.scores {
            writer.signed(*score)?;
        }
    }
    writer.count(plan.members.len())?;
    writer.digest(plan.admission.affected_population_digest)?;
    writer.digest(plan.admission.stakeholder_map_digest)?;
    writer.digest(plan.admission.coverage_manifest_digest)?;
    writer.digest(plan.admission.equity_assessment_digest)?;
    writer.id(plan.admission.assessor_principal)?;
    writer.byte(fitness_tag(plan.admission.fitness))?;
    for member in &plan.members {
        writer.id(member.id.0)?;
        writer.id(member.stakeholder.principal)?;
        writer.digest(member.stakeholder.constituency_digest)?;
        writer.byte(representation_tag(member.stakeholder.representation))?;
        write_optional_id(writer, member.stakeholder.accountable_principal)?;
        write_optional_digest(writer, member.stakeholder.mandate_digest)?;
        write_optional_digest(writer, member.stakeholder.conflict_digest)?;
        writer.byte(lens_tag(member.lens))?;
        writer.digest(member.concern_root_digest)?;
        writer.digest(member.grounding.authority_digest)?;
        writer.digest(member.grounding.surface_digest)?;
        writer.byte(depth_tag(member.grounding.depth))?;
    }
    writer.digest(plan.orthogonality.query_digest)?;
    writer.digest(plan.orthogonality.result_digest)?;
    writer.byte(u8::from(plan.orthogonality.all_orthogonal))?;
    writer.number(plan.fuel.into())
}

fn decode_plan(
    reader: &mut Reader<'_>,
    limits: DeliberationLimitsV1,
) -> Result<DeliberationPlanV1, CodecErrorV1> {
    let id = DeliberationIdV1(reader.id()?);
    let question_digest = reader.digest()?;
    let method = MethodRefV1 {
        machine_digest: reader.digest()?,
        state_digest: reader.digest()?,
    };
    let claim_count = reader.count(limits.max_claims)?;
    let mut claims = Vec::new();
    claims
        .try_reserve(claim_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..claim_count {
        claims.push(read_claim(reader.byte()?)?);
    }
    let criterion_count = reader.count(limits.max_criteria)?;
    let mut criteria = Vec::new();
    criteria
        .try_reserve(criterion_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..criterion_count {
        criteria.push(CriterionV1 {
            id: CriterionIdV1(reader.id()?),
            direction: read_direction(reader.byte()?)?,
        });
    }
    let candidate_count = reader.count(limits.max_candidates)?;
    let mut candidates = Vec::new();
    candidates
        .try_reserve(candidate_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..candidate_count {
        let candidate_id = CandidateIdV1(reader.id()?);
        let score_count = reader.count(limits.max_criteria)?;
        let mut scores = Vec::new();
        scores
            .try_reserve(score_count)
            .map_err(|_| CodecErrorV1::Capacity)?;
        for _ in 0..score_count {
            scores.push(reader.signed()?);
        }
        candidates.push(CandidateV1 {
            id: candidate_id,
            scores,
        });
    }
    let member_count = reader.count(limits.max_members)?;
    let admission = EnsembleAdmissionV1 {
        affected_population_digest: reader.digest()?,
        stakeholder_map_digest: reader.digest()?,
        coverage_manifest_digest: reader.digest()?,
        equity_assessment_digest: reader.digest()?,
        assessor_principal: reader.id()?,
        fitness: read_fitness(reader.byte()?)?,
    };
    let mut members = Vec::new();
    members
        .try_reserve(member_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..member_count {
        members.push(EnsembleMemberV1 {
            id: EnsembleMemberIdV1(reader.id()?),
            stakeholder: StakeholderRefV1 {
                principal: reader.id()?,
                constituency_digest: reader.digest()?,
                representation: read_representation(reader.byte()?)?,
                accountable_principal: read_optional_id(reader)?,
                mandate_digest: read_optional_digest(reader)?,
                conflict_digest: read_optional_digest(reader)?,
            },
            lens: read_lens(reader.byte()?)?,
            concern_root_digest: reader.digest()?,
            grounding: GroundingRefV1 {
                authority_digest: reader.digest()?,
                surface_digest: reader.digest()?,
                depth: read_depth(reader.byte()?)?,
            },
        });
    }
    let orthogonality = OrthogonalityEvidenceV1 {
        query_digest: reader.digest()?,
        result_digest: reader.digest()?,
        all_orthogonal: read_bool(reader.byte()?)?,
    };
    Ok(DeliberationPlanV1 {
        id,
        question_digest,
        method,
        claims,
        criteria,
        candidates,
        admission,
        members,
        orthogonality,
        fuel: to_u32(reader.number()?)?,
    })
}

fn write_optional_id(writer: &mut Writer, value: Option<[u8; 16]>) -> Result<(), CodecErrorV1> {
    writer.byte(u8::from(value.is_some()))?;
    if let Some(value) = value {
        writer.id(value)?;
    }
    Ok(())
}

fn read_optional_id(reader: &mut Reader<'_>) -> Result<Option<[u8; 16]>, CodecErrorV1> {
    match reader.byte()? {
        0 => Ok(None),
        1 => Ok(Some(reader.id()?)),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn write_optional_digest(writer: &mut Writer, value: Option<[u8; 32]>) -> Result<(), CodecErrorV1> {
    writer.byte(u8::from(value.is_some()))?;
    if let Some(value) = value {
        writer.digest(value)?;
    }
    Ok(())
}

fn read_optional_digest(reader: &mut Reader<'_>) -> Result<Option<[u8; 32]>, CodecErrorV1> {
    match reader.byte()? {
        0 => Ok(None),
        1 => Ok(Some(reader.digest()?)),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn representation_tag(value: StakeholderRepresentationV1) -> u8 {
    match value {
        StakeholderRepresentationV1::ActualStakeholder => 0,
        StakeholderRepresentationV1::AuthorizedRepresentative => 1,
        StakeholderRepresentationV1::AgentModeledPerspective => 2,
        StakeholderRepresentationV1::TechnicalSme => 3,
    }
}

fn read_representation(value: u8) -> Result<StakeholderRepresentationV1, CodecErrorV1> {
    match value {
        0 => Ok(StakeholderRepresentationV1::ActualStakeholder),
        1 => Ok(StakeholderRepresentationV1::AuthorizedRepresentative),
        2 => Ok(StakeholderRepresentationV1::AgentModeledPerspective),
        3 => Ok(StakeholderRepresentationV1::TechnicalSme),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn fitness_tag(value: EnsembleFitnessV1) -> u8 {
    match value {
        EnsembleFitnessV1::Yes => 0,
        EnsembleFitnessV1::No => 1,
        EnsembleFitnessV1::Contested => 2,
    }
}

fn read_fitness(value: u8) -> Result<EnsembleFitnessV1, CodecErrorV1> {
    match value {
        0 => Ok(EnsembleFitnessV1::Yes),
        1 => Ok(EnsembleFitnessV1::No),
        2 => Ok(EnsembleFitnessV1::Contested),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn encode_report(writer: &mut Writer, report: &MemberReportV1) -> Result<(), CodecErrorV1> {
    writer.id(report.id.0)?;
    writer.id(report.member.0)?;
    writer.digest(report.output_digest)?;
    writer.count(report.evidence_digests.len())?;
    for digest in &report.evidence_digests {
        writer.digest(*digest)?;
    }
    writer.count(report.dissent_digests.len())?;
    for digest in &report.dissent_digests {
        writer.digest(*digest)?;
    }
    Ok(())
}

fn decode_report(
    reader: &mut Reader<'_>,
    limits: DeliberationLimitsV1,
) -> Result<MemberReportV1, CodecErrorV1> {
    let id = ReportIdV1(reader.id()?);
    let member = EnsembleMemberIdV1(reader.id()?);
    let output_digest = reader.digest()?;
    let evidence_count = reader.count(limits.max_evidence_per_report)?;
    let mut evidence_digests = Vec::new();
    evidence_digests
        .try_reserve(evidence_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..evidence_count {
        evidence_digests.push(reader.digest()?);
    }
    let dissent_count = reader.count(limits.max_dissent_per_report)?;
    let mut dissent_digests = Vec::new();
    dissent_digests
        .try_reserve(dissent_count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..dissent_count {
        dissent_digests.push(reader.digest()?);
    }
    Ok(MemberReportV1 {
        id,
        member,
        output_digest,
        evidence_digests,
        dissent_digests,
    })
}

fn encode_resolution(
    writer: &mut Writer,
    resolution: Option<&ResolutionV1>,
) -> Result<(), CodecErrorV1> {
    let Some(resolution) = resolution else {
        return writer.byte(0);
    };
    match resolution {
        ResolutionV1::ProvenBool {
            value,
            validator,
            evidence_digest,
        } => {
            writer.byte(1)?;
            writer.byte(u8::from(*value))?;
            writer.id(*validator)?;
            writer.digest(*evidence_digest)
        }
        ResolutionV1::AuthorizedChoice {
            frontier,
            selected,
            authority,
        } => {
            writer.byte(2)?;
            writer.count(frontier.len())?;
            for candidate in frontier {
                writer.id(candidate.0)?;
            }
            writer.id(selected.0)?;
            writer.id(authority.decision.0)?;
            writer.id(authority.subject.0)?;
            writer.number(authority.epoch)
        }
        ResolutionV1::Maybe(value) => {
            writer.byte(3)?;
            writer.byte(maybe_tag(*value))
        }
        ResolutionV1::Mu(reason) => {
            writer.byte(4)?;
            writer.byte(mu_tag(*reason))
        }
    }
}

fn decode_resolution(
    reader: &mut Reader<'_>,
    max_candidates: usize,
) -> Result<Option<ResolutionV1>, CodecErrorV1> {
    Ok(match reader.byte()? {
        0 => None,
        1 => Some(ResolutionV1::ProvenBool {
            value: read_bool(reader.byte()?)?,
            validator: reader.id()?,
            evidence_digest: reader.digest()?,
        }),
        2 => {
            let count = reader.count(max_candidates)?;
            let mut frontier = Vec::new();
            frontier
                .try_reserve(count)
                .map_err(|_| CodecErrorV1::Capacity)?;
            for _ in 0..count {
                frontier.push(CandidateIdV1(reader.id()?));
            }
            Some(ResolutionV1::AuthorizedChoice {
                frontier,
                selected: CandidateIdV1(reader.id()?),
                authority: AuthorityRefV1 {
                    decision: AuthorityDecisionIdV1(reader.id()?),
                    subject: AuthoritySubjectIdV1(reader.id()?),
                    epoch: reader.number()?,
                },
            })
        }
        3 => Some(ResolutionV1::Maybe(read_maybe(reader.byte()?)?)),
        4 => Some(ResolutionV1::Mu(read_mu(reader.byte()?)?)),
        _ => return Err(CodecErrorV1::Invalid),
    })
}

fn encode_human_gate_request(
    writer: &mut Writer,
    request: Option<&HumanGateRequestV1>,
) -> Result<(), CodecErrorV1> {
    let Some(request) = request else {
        return writer.byte(0);
    };
    writer.byte(1)?;
    writer.id(request.id.0)?;
    writer.id(request.reviewer)?;
    writer.digest(request.finding_digest)?;
    writer.digest(request.decision_digest)?;
    writer.digest(request.material_revision_digest)?;
    writer.digest(request.policy_digest)?;
    encode_human_gate_material_ref(writer, request.decision_packet)?;
    writer.number(request.requested_at)
}

fn decode_human_gate_request(
    reader: &mut Reader<'_>,
) -> Result<Option<HumanGateRequestV1>, CodecErrorV1> {
    match reader.byte()? {
        0 => Ok(None),
        1 => Ok(Some(HumanGateRequestV1 {
            id: HumanGateRequestIdV1(reader.id()?),
            reviewer: reader.id()?,
            finding_digest: reader.digest()?,
            decision_digest: reader.digest()?,
            material_revision_digest: reader.digest()?,
            policy_digest: reader.digest()?,
            decision_packet: decode_human_gate_material_ref(reader)?,
            requested_at: reader.number()?,
        })),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn encode_human_gate_response(
    writer: &mut Writer,
    response: Option<&HumanGateResponseV1>,
) -> Result<(), CodecErrorV1> {
    let Some(response) = response else {
        return writer.byte(0);
    };
    writer.byte(1)?;
    writer.id(response.id.0)?;
    writer.id(response.request.0)?;
    writer.byte(human_gate_disposition_tag(response.disposition))?;
    writer.digest(response.rationale_digest)?;
    encode_human_gate_material_ref(writer, response.rationale)?;
    writer.number(response.responded_at)?;
    writer.id(response.authority.decision)?;
    writer.id(response.authority.subject)?;
    writer.number(response.authority.epoch)
}

fn decode_human_gate_response(
    reader: &mut Reader<'_>,
) -> Result<Option<HumanGateResponseV1>, CodecErrorV1> {
    match reader.byte()? {
        0 => Ok(None),
        1 => Ok(Some(HumanGateResponseV1 {
            id: HumanGateResponseIdV1(reader.id()?),
            request: HumanGateRequestIdV1(reader.id()?),
            disposition: read_human_gate_disposition(reader.byte()?)?,
            rationale_digest: reader.digest()?,
            rationale: decode_human_gate_material_ref(reader)?,
            responded_at: reader.number()?,
            authority: HumanGateAuthorityRefV1 {
                decision: reader.id()?,
                subject: reader.id()?,
                epoch: reader.number()?,
            },
        })),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn encode_human_gate_material_ref(
    writer: &mut Writer,
    value: HumanGateMaterialRefV1,
) -> Result<(), CodecErrorV1> {
    writer.id(value.owner_route)?;
    writer.id(value.artifact_route)?;
    writer.number(value.expected_revision)?;
    writer.digest(value.expected_sha256)
}

fn decode_human_gate_material_ref(
    reader: &mut Reader<'_>,
) -> Result<HumanGateMaterialRefV1, CodecErrorV1> {
    Ok(HumanGateMaterialRefV1 {
        owner_route: reader.id()?,
        artifact_route: reader.id()?,
        expected_revision: reader.number()?,
        expected_sha256: reader.digest()?,
    })
}

fn human_gate_disposition_tag(value: HumanGateDispositionV1) -> u8 {
    match value {
        HumanGateDispositionV1::Approved => 0,
        HumanGateDispositionV1::Rejected => 1,
        HumanGateDispositionV1::Deferred => 2,
        HumanGateDispositionV1::ChangesRequested => 3,
    }
}

fn read_human_gate_disposition(value: u8) -> Result<HumanGateDispositionV1, CodecErrorV1> {
    match value {
        0 => Ok(HumanGateDispositionV1::Approved),
        1 => Ok(HumanGateDispositionV1::Rejected),
        2 => Ok(HumanGateDispositionV1::Deferred),
        3 => Ok(HumanGateDispositionV1::ChangesRequested),
        _ => Err(CodecErrorV1::Invalid),
    }
}

fn validate_codec_limits(limits: CodecLimitsV1) -> Result<(), CodecErrorV1> {
    if limits.max_bytes == 0 || limits.max_bytes > isize::MAX as usize {
        return Err(CodecErrorV1::InvalidLimits);
    }
    validate_domain_limits(limits.domain).map_err(|_| CodecErrorV1::InvalidLimits)
}

fn validate_domain_limits(limits: DeliberationLimitsV1) -> Result<(), ()> {
    if limits.max_claims == 0
        || limits.max_criteria == 0
        || limits.max_candidates == 0
        || limits.max_members == 0
        || limits.max_evidence_per_report == 0
        || limits.max_dissent_per_report == 0
        || limits.max_fuel == 0
    {
        Err(())
    } else {
        Ok(())
    }
}

fn validate_host_bounds(
    checkpoint: &DeliberationCheckpointV1,
    host: CodecLimitsV1,
) -> Result<(), CodecErrorV1> {
    let value = checkpoint.limits;
    let bound = host.domain;
    if value.max_claims > bound.max_claims
        || value.max_criteria > bound.max_criteria
        || value.max_candidates > bound.max_candidates
        || value.max_members > bound.max_members
        || value.max_evidence_per_report > bound.max_evidence_per_report
        || value.max_dissent_per_report > bound.max_dissent_per_report
        || value.max_fuel > bound.max_fuel
    {
        Err(CodecErrorV1::Limit)
    } else {
        Ok(())
    }
}

fn to_u32(value: u64) -> Result<u32, CodecErrorV1> {
    value.try_into().map_err(|_| CodecErrorV1::Limit)
}

fn claim_tag(value: ClaimKindV1) -> u8 {
    match value {
        ClaimKindV1::Formal => 0,
        ClaimKindV1::Empirical => 1,
        ClaimKindV1::Normative => 2,
        ClaimKindV1::Historical => 3,
        ClaimKindV1::Creative => 4,
    }
}
fn read_claim(value: u8) -> Result<ClaimKindV1, CodecErrorV1> {
    Ok(match value {
        0 => ClaimKindV1::Formal,
        1 => ClaimKindV1::Empirical,
        2 => ClaimKindV1::Normative,
        3 => ClaimKindV1::Historical,
        4 => ClaimKindV1::Creative,
        _ => return Err(CodecErrorV1::Invalid),
    })
}
fn direction_tag(value: CriterionDirectionV1) -> u8 {
    match value {
        CriterionDirectionV1::Maximize => 0,
        CriterionDirectionV1::Minimize => 1,
    }
}
fn read_direction(value: u8) -> Result<CriterionDirectionV1, CodecErrorV1> {
    Ok(match value {
        0 => CriterionDirectionV1::Maximize,
        1 => CriterionDirectionV1::Minimize,
        _ => return Err(CodecErrorV1::Invalid),
    })
}
fn depth_tag(value: SurfaceDepthV1) -> u8 {
    match value {
        SurfaceDepthV1::Ambient => 0,
        SurfaceDepthV1::Activated => 1,
        SurfaceDepthV1::Reference => 2,
    }
}
fn read_depth(value: u8) -> Result<SurfaceDepthV1, CodecErrorV1> {
    Ok(match value {
        0 => SurfaceDepthV1::Ambient,
        1 => SurfaceDepthV1::Activated,
        2 => SurfaceDepthV1::Reference,
        _ => return Err(CodecErrorV1::Invalid),
    })
}
fn lens_tag(value: PersonaLensV1) -> u8 {
    match value {
        PersonaLensV1::Historian => 0,
        PersonaLensV1::IdeaStormer => 1,
        PersonaLensV1::Theorist => 2,
        PersonaLensV1::Researcher => 3,
        PersonaLensV1::RigorReviewer => 4,
        PersonaLensV1::ConvergenceFacilitator => 5,
    }
}
fn read_lens(value: u8) -> Result<PersonaLensV1, CodecErrorV1> {
    Ok(match value {
        0 => PersonaLensV1::Historian,
        1 => PersonaLensV1::IdeaStormer,
        2 => PersonaLensV1::Theorist,
        3 => PersonaLensV1::Researcher,
        4 => PersonaLensV1::RigorReviewer,
        5 => PersonaLensV1::ConvergenceFacilitator,
        _ => return Err(CodecErrorV1::Invalid),
    })
}
fn maybe_tag(value: MaybeValueV1) -> u8 {
    match value {
        MaybeValueV1::TrueOnly => 0,
        MaybeValueV1::FalseOnly => 1,
        MaybeValueV1::Both => 2,
        MaybeValueV1::Neither => 3,
    }
}
fn read_maybe(value: u8) -> Result<MaybeValueV1, CodecErrorV1> {
    Ok(match value {
        0 => MaybeValueV1::TrueOnly,
        1 => MaybeValueV1::FalseOnly,
        2 => MaybeValueV1::Both,
        3 => MaybeValueV1::Neither,
        _ => return Err(CodecErrorV1::Invalid),
    })
}
fn mu_tag(value: MuReasonV1) -> u8 {
    match value {
        MuReasonV1::FuelExhausted => 0,
        MuReasonV1::MissingEvidence => 1,
        MuReasonV1::ContradictoryEvidence => 2,
        MuReasonV1::WrongQuestion => 3,
        MuReasonV1::WrongEnsemble => 4,
        MuReasonV1::NoAuthorizedSelector => 5,
    }
}
fn read_mu(value: u8) -> Result<MuReasonV1, CodecErrorV1> {
    Ok(match value {
        0 => MuReasonV1::FuelExhausted,
        1 => MuReasonV1::MissingEvidence,
        2 => MuReasonV1::ContradictoryEvidence,
        3 => MuReasonV1::WrongQuestion,
        4 => MuReasonV1::WrongEnsemble,
        5 => MuReasonV1::NoAuthorizedSelector,
        _ => return Err(CodecErrorV1::Invalid),
    })
}
fn read_bool(value: u8) -> Result<bool, CodecErrorV1> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CodecErrorV1::Invalid),
    }
}
