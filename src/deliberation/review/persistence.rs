//! Canonical bounded bytes for one retained review exchange.

use super::*;
use crate::deliberation::{
    HumanGateAuthorityRefV1, HumanGateDispositionV1, HumanGateMaterialRefV1, HumanGateRequestIdV1,
};

const MAGIC: &[u8; 8] = b"MOIRARV1";
const VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewCodecLimitsV1 {
    pub max_bytes: usize,
    pub review: ReviewLimitsV1,
    pub max_reports: usize,
    pub max_evidence_per_report: usize,
    pub max_dissent_per_report: usize,
}

impl Default for ReviewCodecLimitsV1 {
    fn default() -> Self {
        Self {
            max_bytes: 8 * 1024 * 1024,
            review: ReviewLimitsV1::default(),
            max_reports: 16,
            max_evidence_per_report: 64,
            max_dissent_per_report: 32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewCodecErrorV1 {
    InvalidLimits,
    Limit,
    Capacity,
    Truncated,
    Invalid,
    NonCanonical,
    Review(ReviewRefusalV1),
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
    fn raw(&mut self, value: &[u8]) -> Result<(), ReviewCodecErrorV1> {
        if value.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(ReviewCodecErrorV1::Limit);
        }
        self.bytes
            .try_reserve(value.len())
            .map_err(|_| ReviewCodecErrorV1::Capacity)?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }
    fn byte(&mut self, value: u8) -> Result<(), ReviewCodecErrorV1> {
        self.raw(&[value])
    }
    fn number(&mut self, value: u64) -> Result<(), ReviewCodecErrorV1> {
        self.raw(&value.to_be_bytes())
    }
    fn id(&mut self, value: [u8; 16]) -> Result<(), ReviewCodecErrorV1> {
        self.raw(&value)
    }
    fn digest(&mut self, value: [u8; 32]) -> Result<(), ReviewCodecErrorV1> {
        self.raw(&value)
    }
    fn count(&mut self, value: usize) -> Result<(), ReviewCodecErrorV1> {
        self.number(u64::try_from(value).map_err(|_| ReviewCodecErrorV1::Limit)?)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], maximum: usize) -> Result<Self, ReviewCodecErrorV1> {
        if bytes.len() > maximum {
            return Err(ReviewCodecErrorV1::Limit);
        }
        Ok(Self { bytes, position: 0 })
    }
    fn raw<const N: usize>(&mut self) -> Result<[u8; N], ReviewCodecErrorV1> {
        let end = self
            .position
            .checked_add(N)
            .ok_or(ReviewCodecErrorV1::Invalid)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(ReviewCodecErrorV1::Truncated)?;
        self.position = end;
        value.try_into().map_err(|_| ReviewCodecErrorV1::Invalid)
    }
    fn byte(&mut self) -> Result<u8, ReviewCodecErrorV1> {
        Ok(self.raw::<1>()?[0])
    }
    fn number(&mut self) -> Result<u64, ReviewCodecErrorV1> {
        Ok(u64::from_be_bytes(self.raw()?))
    }
    fn id(&mut self) -> Result<[u8; 16], ReviewCodecErrorV1> {
        self.raw()
    }
    fn digest(&mut self) -> Result<[u8; 32], ReviewCodecErrorV1> {
        self.raw()
    }
    fn count(&mut self, maximum: usize) -> Result<usize, ReviewCodecErrorV1> {
        let value = usize::try_from(self.number()?).map_err(|_| ReviewCodecErrorV1::Limit)?;
        if value > maximum {
            return Err(ReviewCodecErrorV1::Limit);
        }
        Ok(value)
    }
    fn finish(self) -> Result<(), ReviewCodecErrorV1> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(ReviewCodecErrorV1::Invalid)
        }
    }
}

pub fn encode_review_checkpoint(
    checkpoint: &ReviewExchangeCheckpointV1,
    limits: ReviewCodecLimitsV1,
) -> Result<Vec<u8>, ReviewCodecErrorV1> {
    validate_limits(limits)?;
    validate_checkpoint_bounds(checkpoint, limits)?;
    ReviewExchangeV1::recover(checkpoint.clone()).map_err(ReviewCodecErrorV1::Review)?;
    let mut writer = Writer::new(limits.max_bytes);
    writer.raw(MAGIC)?;
    writer.byte(VERSION)?;
    encode_review_limits(&mut writer, checkpoint.limits)?;
    writer.id(checkpoint.deliberation.0)?;
    writer.count(checkpoint.reports.len())?;
    for report in &checkpoint.reports {
        encode_report(&mut writer, report)?;
    }
    encode_human_response(
        &mut writer,
        checkpoint.admitted_human_gate_response.as_ref(),
    )?;
    writer.count(checkpoint.findings.len())?;
    for finding in &checkpoint.findings {
        encode_finding(&mut writer, finding)?;
    }
    writer.count(checkpoint.rebuttals.len())?;
    for rebuttal in &checkpoint.rebuttals {
        encode_rebuttal(&mut writer, rebuttal)?;
    }
    writer.count(checkpoint.dispositions.len())?;
    for disposition in &checkpoint.dispositions {
        encode_disposition(&mut writer, disposition)?;
    }
    Ok(writer.bytes)
}

pub fn decode_review_checkpoint(
    bytes: &[u8],
    limits: ReviewCodecLimitsV1,
) -> Result<ReviewExchangeCheckpointV1, ReviewCodecErrorV1> {
    validate_limits(limits)?;
    let mut reader = Reader::new(bytes, limits.max_bytes)?;
    if &reader.raw::<8>()? != MAGIC || reader.byte()? != VERSION {
        return Err(ReviewCodecErrorV1::Invalid);
    }
    let retained_limits = decode_review_limits(&mut reader, limits.review)?;
    let deliberation = DeliberationIdV1(reader.id()?);
    let report_count = reader.count(limits.max_reports)?;
    let mut reports = Vec::new();
    reports
        .try_reserve(report_count)
        .map_err(|_| ReviewCodecErrorV1::Capacity)?;
    for _ in 0..report_count {
        reports.push(decode_report(&mut reader, limits)?);
    }
    let admitted_human_gate_response = decode_human_response(&mut reader)?;
    let finding_count = reader.count(retained_limits.max_findings)?;
    let mut findings = Vec::new();
    findings
        .try_reserve(finding_count)
        .map_err(|_| ReviewCodecErrorV1::Capacity)?;
    for _ in 0..finding_count {
        findings.push(decode_finding(&mut reader, retained_limits)?);
    }
    let rebuttal_count = reader.count(retained_limits.max_rebuttals)?;
    let mut rebuttals = Vec::new();
    rebuttals
        .try_reserve(rebuttal_count)
        .map_err(|_| ReviewCodecErrorV1::Capacity)?;
    for _ in 0..rebuttal_count {
        rebuttals.push(decode_rebuttal(&mut reader, retained_limits)?);
    }
    let disposition_count = reader.count(retained_limits.max_findings)?;
    let mut dispositions = Vec::new();
    dispositions
        .try_reserve(disposition_count)
        .map_err(|_| ReviewCodecErrorV1::Capacity)?;
    for _ in 0..disposition_count {
        dispositions.push(decode_disposition(&mut reader, retained_limits)?);
    }
    reader.finish()?;
    let checkpoint = ReviewExchangeCheckpointV1 {
        version: VERSION,
        deliberation,
        limits: retained_limits,
        reports,
        admitted_human_gate_response,
        findings,
        rebuttals,
        dispositions,
    };
    ReviewExchangeV1::recover(checkpoint.clone()).map_err(ReviewCodecErrorV1::Review)?;
    if encode_review_checkpoint(&checkpoint, limits)? != bytes {
        return Err(ReviewCodecErrorV1::NonCanonical);
    }
    Ok(checkpoint)
}

fn validate_limits(limits: ReviewCodecLimitsV1) -> Result<(), ReviewCodecErrorV1> {
    if limits.max_bytes == 0
        || limits.max_bytes > isize::MAX as usize
        || limits.max_reports == 0
        || limits.max_evidence_per_report == 0
        || limits.max_dissent_per_report == 0
        || limits.review.max_findings == 0
        || limits.review.max_rebuttals == 0
        || limits.review.max_evidence_per_item == 0
    {
        return Err(ReviewCodecErrorV1::InvalidLimits);
    }
    Ok(())
}

fn validate_checkpoint_bounds(
    checkpoint: &ReviewExchangeCheckpointV1,
    host: ReviewCodecLimitsV1,
) -> Result<(), ReviewCodecErrorV1> {
    if checkpoint.reports.len() > host.max_reports
        || checkpoint.limits.max_findings > host.review.max_findings
        || checkpoint.limits.max_rebuttals > host.review.max_rebuttals
        || checkpoint.limits.max_evidence_per_item > host.review.max_evidence_per_item
        || checkpoint.reports.iter().any(|report| {
            report.evidence_digests.len() > host.max_evidence_per_report
                || report.dissent_digests.len() > host.max_dissent_per_report
        })
    {
        return Err(ReviewCodecErrorV1::Limit);
    }
    Ok(())
}

fn encode_review_limits(
    writer: &mut Writer,
    limits: ReviewLimitsV1,
) -> Result<(), ReviewCodecErrorV1> {
    writer.count(limits.max_findings)?;
    writer.count(limits.max_rebuttals)?;
    writer.count(limits.max_evidence_per_item)
}
fn decode_review_limits(
    reader: &mut Reader<'_>,
    host: ReviewLimitsV1,
) -> Result<ReviewLimitsV1, ReviewCodecErrorV1> {
    let value = ReviewLimitsV1 {
        max_findings: reader.count(host.max_findings)?,
        max_rebuttals: reader.count(host.max_rebuttals)?,
        max_evidence_per_item: reader.count(host.max_evidence_per_item)?,
    };
    if value.max_findings == 0 || value.max_rebuttals == 0 || value.max_evidence_per_item == 0 {
        return Err(ReviewCodecErrorV1::Invalid);
    }
    Ok(value)
}

fn encode_digests(writer: &mut Writer, values: &[[u8; 32]]) -> Result<(), ReviewCodecErrorV1> {
    writer.count(values.len())?;
    for value in values {
        writer.digest(*value)?;
    }
    Ok(())
}
fn decode_digests(
    reader: &mut Reader<'_>,
    maximum: usize,
) -> Result<Vec<[u8; 32]>, ReviewCodecErrorV1> {
    let count = reader.count(maximum)?;
    let mut values = Vec::new();
    values
        .try_reserve(count)
        .map_err(|_| ReviewCodecErrorV1::Capacity)?;
    for _ in 0..count {
        values.push(reader.digest()?);
    }
    Ok(values)
}

fn encode_report(writer: &mut Writer, value: &MemberReportV1) -> Result<(), ReviewCodecErrorV1> {
    writer.id(value.id.0)?;
    writer.id(value.member.0)?;
    writer.digest(value.output_digest)?;
    encode_digests(writer, &value.evidence_digests)?;
    encode_digests(writer, &value.dissent_digests)
}
fn decode_report(
    reader: &mut Reader<'_>,
    limits: ReviewCodecLimitsV1,
) -> Result<MemberReportV1, ReviewCodecErrorV1> {
    Ok(MemberReportV1 {
        id: ReportIdV1(reader.id()?),
        member: EnsembleMemberIdV1(reader.id()?),
        output_digest: reader.digest()?,
        evidence_digests: decode_digests(reader, limits.max_evidence_per_report)?,
        dissent_digests: decode_digests(reader, limits.max_dissent_per_report)?,
    })
}

fn encode_finding(writer: &mut Writer, value: &ReviewFindingV1) -> Result<(), ReviewCodecErrorV1> {
    writer.id(value.id.0)?;
    writer.id(value.source_report.0)?;
    writer.id(value.owner)?;
    writer.digest(value.finding_digest)?;
    encode_digests(writer, &value.evidence_digests)?;
    writer.byte(u8::from(value.requires_human_gate))
}
fn decode_finding(
    reader: &mut Reader<'_>,
    limits: ReviewLimitsV1,
) -> Result<ReviewFindingV1, ReviewCodecErrorV1> {
    let id = FindingIdV1(reader.id()?);
    let source_report = ReportIdV1(reader.id()?);
    let owner = reader.id()?;
    let finding_digest = reader.digest()?;
    let evidence_digests = decode_digests(reader, limits.max_evidence_per_item)?;
    let requires_human_gate = match reader.byte()? {
        0 => false,
        1 => true,
        _ => return Err(ReviewCodecErrorV1::Invalid),
    };
    Ok(ReviewFindingV1 {
        id,
        source_report,
        owner,
        finding_digest,
        evidence_digests,
        requires_human_gate,
    })
}

fn encode_rebuttal(writer: &mut Writer, value: &RebuttalV1) -> Result<(), ReviewCodecErrorV1> {
    writer.id(value.id.0)?;
    writer.id(value.finding.0)?;
    writer.id(value.member.0)?;
    writer.digest(value.rebuttal_digest)?;
    encode_digests(writer, &value.evidence_digests)
}
fn decode_rebuttal(
    reader: &mut Reader<'_>,
    limits: ReviewLimitsV1,
) -> Result<RebuttalV1, ReviewCodecErrorV1> {
    Ok(RebuttalV1 {
        id: RebuttalIdV1(reader.id()?),
        finding: FindingIdV1(reader.id()?),
        member: EnsembleMemberIdV1(reader.id()?),
        rebuttal_digest: reader.digest()?,
        evidence_digests: decode_digests(reader, limits.max_evidence_per_item)?,
    })
}

fn encode_disposition(
    writer: &mut Writer,
    value: &FindingDispositionV1,
) -> Result<(), ReviewCodecErrorV1> {
    writer.id(value.id.0)?;
    writer.id(value.finding.0)?;
    writer.id(value.owner)?;
    writer.byte(disposition_tag(value.kind))?;
    writer.digest(value.rationale_digest)?;
    encode_digests(writer, &value.evidence_digests)?;
    match value.human_gate_response {
        Some(id) => {
            writer.byte(1)?;
            writer.id(id.0)?;
        }
        None => writer.byte(0)?,
    }
    writer.number(value.decided_at)
}
fn decode_disposition(
    reader: &mut Reader<'_>,
    limits: ReviewLimitsV1,
) -> Result<FindingDispositionV1, ReviewCodecErrorV1> {
    let id = FindingDispositionIdV1(reader.id()?);
    let finding = FindingIdV1(reader.id()?);
    let owner = reader.id()?;
    let kind = read_disposition(reader.byte()?)?;
    let rationale_digest = reader.digest()?;
    let evidence_digests = decode_digests(reader, limits.max_evidence_per_item)?;
    let human_gate_response = match reader.byte()? {
        0 => None,
        1 => Some(HumanGateResponseIdV1(reader.id()?)),
        _ => return Err(ReviewCodecErrorV1::Invalid),
    };
    Ok(FindingDispositionV1 {
        id,
        finding,
        owner,
        kind,
        rationale_digest,
        evidence_digests,
        human_gate_response,
        decided_at: reader.number()?,
    })
}

fn disposition_tag(value: FindingDispositionKindV1) -> u8 {
    match value {
        FindingDispositionKindV1::Accepted => 0,
        FindingDispositionKindV1::Mitigated => 1,
        FindingDispositionKindV1::Tested => 2,
        FindingDispositionKindV1::Deferred => 3,
        FindingDispositionKindV1::RiskAccepted => 4,
        FindingDispositionKindV1::Disputed => 5,
        FindingDispositionKindV1::OutOfScope => 6,
    }
}
fn read_disposition(value: u8) -> Result<FindingDispositionKindV1, ReviewCodecErrorV1> {
    match value {
        0 => Ok(FindingDispositionKindV1::Accepted),
        1 => Ok(FindingDispositionKindV1::Mitigated),
        2 => Ok(FindingDispositionKindV1::Tested),
        3 => Ok(FindingDispositionKindV1::Deferred),
        4 => Ok(FindingDispositionKindV1::RiskAccepted),
        5 => Ok(FindingDispositionKindV1::Disputed),
        6 => Ok(FindingDispositionKindV1::OutOfScope),
        _ => Err(ReviewCodecErrorV1::Invalid),
    }
}

fn encode_material(
    writer: &mut Writer,
    value: HumanGateMaterialRefV1,
) -> Result<(), ReviewCodecErrorV1> {
    writer.id(value.owner_route)?;
    writer.id(value.artifact_route)?;
    writer.number(value.expected_revision)?;
    writer.digest(value.expected_sha256)
}
fn decode_material(reader: &mut Reader<'_>) -> Result<HumanGateMaterialRefV1, ReviewCodecErrorV1> {
    Ok(HumanGateMaterialRefV1 {
        owner_route: reader.id()?,
        artifact_route: reader.id()?,
        expected_revision: reader.number()?,
        expected_sha256: reader.digest()?,
    })
}
fn encode_human_response(
    writer: &mut Writer,
    value: Option<&HumanGateResponseV1>,
) -> Result<(), ReviewCodecErrorV1> {
    let Some(value) = value else {
        return writer.byte(0);
    };
    writer.byte(1)?;
    writer.id(value.id.0)?;
    writer.id(value.request.0)?;
    writer.byte(human_disposition_tag(value.disposition))?;
    writer.digest(value.rationale_digest)?;
    encode_material(writer, value.rationale)?;
    writer.number(value.responded_at)?;
    writer.id(value.authority.decision)?;
    writer.id(value.authority.subject)?;
    writer.number(value.authority.epoch)
}
fn decode_human_response(
    reader: &mut Reader<'_>,
) -> Result<Option<HumanGateResponseV1>, ReviewCodecErrorV1> {
    match reader.byte()? {
        0 => Ok(None),
        1 => Ok(Some(HumanGateResponseV1 {
            id: HumanGateResponseIdV1(reader.id()?),
            request: HumanGateRequestIdV1(reader.id()?),
            disposition: read_human_disposition(reader.byte()?)?,
            rationale_digest: reader.digest()?,
            rationale: decode_material(reader)?,
            responded_at: reader.number()?,
            authority: HumanGateAuthorityRefV1 {
                decision: reader.id()?,
                subject: reader.id()?,
                epoch: reader.number()?,
            },
        })),
        _ => Err(ReviewCodecErrorV1::Invalid),
    }
}
fn human_disposition_tag(value: HumanGateDispositionV1) -> u8 {
    match value {
        HumanGateDispositionV1::Approved => 0,
        HumanGateDispositionV1::Rejected => 1,
        HumanGateDispositionV1::Deferred => 2,
        HumanGateDispositionV1::ChangesRequested => 3,
    }
}
fn read_human_disposition(value: u8) -> Result<HumanGateDispositionV1, ReviewCodecErrorV1> {
    match value {
        0 => Ok(HumanGateDispositionV1::Approved),
        1 => Ok(HumanGateDispositionV1::Rejected),
        2 => Ok(HumanGateDispositionV1::Deferred),
        3 => Ok(HumanGateDispositionV1::ChangesRequested),
        _ => Err(ReviewCodecErrorV1::Invalid),
    }
}
