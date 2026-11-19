//! Retained, authority-honest review exchange after independent first passes.
//!
//! Agora can project these facts into a room, but conversation is not the
//! authoritative record. Moira retains exact findings, cross-member rebuttals,
//! owner dispositions and any named-human response required by policy.

use std::collections::BTreeSet;

use super::{
    DeliberationIdV1, DeliberationPhaseV1, DeliberationV1, EnsembleMemberIdV1,
    FindingDispositionIdV1, FindingIdV1, HumanGateDispositionV1, HumanGateResponseIdV1,
    HumanGateResponseV1, MemberReportV1, RebuttalIdV1, ReportIdV1,
};

pub mod persistence;

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

impl CorrelationDimensionV1 {
    pub const ALL: [Self; 9] = [
        Self::ExecutionPrincipal,
        Self::ExecutionProfile,
        Self::ContextManifest,
        Self::Model,
        Self::Provider,
        Self::Prompt,
        Self::Grounding,
        Self::EvidenceScope,
        Self::Organization,
    ];
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

/// Content-free cohort disclosure retained at the cross-review boundary.
///
/// Zero observations disclose missing evidence. Distinct runtime identities are
/// necessary for some forms of separation, but never establish cognitive
/// independence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnsembleCorrelationEvidenceV1 {
    pub members: usize,
    pub dimensions: Vec<CorrelationDimensionEvidenceV1>,
    pub first_pass: FirstPassExposureV1,
}

impl EnsembleCorrelationEvidenceV1 {
    pub fn undisclosed(members: usize) -> Self {
        Self {
            members,
            dimensions: CorrelationDimensionV1::ALL
                .into_iter()
                .map(|dimension| CorrelationDimensionEvidenceV1 {
                    dimension,
                    observed_members: 0,
                    distinct_values: 0,
                })
                .collect(),
            first_pass: FirstPassExposureV1 {
                committed_before_cross_review: 0,
                exposed_before_commit: 0,
                unknown: members,
            },
        }
    }

    pub fn independence(&self) -> IndependenceAssessmentV1 {
        if self.dimensions.iter().any(|evidence| {
            evidence.observed_members > 1 && evidence.distinct_values < evidence.observed_members
        }) {
            IndependenceAssessmentV1::Correlated
        } else {
            IndependenceAssessmentV1::NotEstablished
        }
    }

    pub fn dimension(
        &self,
        dimension: CorrelationDimensionV1,
    ) -> Option<CorrelationDimensionEvidenceV1> {
        self.dimensions
            .iter()
            .copied()
            .find(|evidence| evidence.dimension == dimension)
    }
}

impl Default for ReviewLimitsV1 {
    fn default() -> Self {
        Self {
            max_findings: 128,
            max_rebuttals: 256,
            max_evidence_per_item: 64,
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
pub struct ReviewExchangeCheckpointV1 {
    pub version: u8,
    pub deliberation: DeliberationIdV1,
    pub limits: ReviewLimitsV1,
    pub reports: Vec<MemberReportV1>,
    pub correlation: EnsembleCorrelationEvidenceV1,
    pub admitted_human_gate_response: Option<HumanGateResponseV1>,
    pub findings: Vec<ReviewFindingV1>,
    pub rebuttals: Vec<RebuttalV1>,
    pub dispositions: Vec<FindingDispositionV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewExchangeV1 {
    deliberation: DeliberationIdV1,
    limits: ReviewLimitsV1,
    reports: Vec<MemberReportV1>,
    correlation: EnsembleCorrelationEvidenceV1,
    admitted_human_gate_response: Option<HumanGateResponseV1>,
    findings: Vec<ReviewFindingV1>,
    rebuttals: Vec<RebuttalV1>,
    dispositions: Vec<FindingDispositionV1>,
}

impl ReviewExchangeV1 {
    pub fn try_new(
        deliberation: &DeliberationV1,
        limits: ReviewLimitsV1,
        correlation: EnsembleCorrelationEvidenceV1,
    ) -> Result<Self, ReviewRefusalV1> {
        validate_limits(limits)?;
        if deliberation.phase() == DeliberationPhaseV1::Collecting {
            return Err(ReviewRefusalV1::FirstPassIncomplete);
        }
        validate_correlation(&correlation, deliberation.reports().len())?;
        Ok(Self {
            deliberation: deliberation.plan().id,
            limits,
            reports: deliberation.reports().to_vec(),
            correlation,
            admitted_human_gate_response: deliberation.human_gate_response().copied(),
            findings: Vec::new(),
            rebuttals: Vec::new(),
            dispositions: Vec::new(),
        })
    }

    pub fn phase(&self) -> ReviewPhaseV1 {
        if !self.findings.is_empty() && self.dispositions.len() == self.findings.len() {
            ReviewPhaseV1::Disposed
        } else if !self.rebuttals.is_empty() {
            ReviewPhaseV1::Challenged
        } else {
            ReviewPhaseV1::FirstPassComplete
        }
    }

    pub fn findings(&self) -> &[ReviewFindingV1] {
        &self.findings
    }

    pub fn correlation(&self) -> &EnsembleCorrelationEvidenceV1 {
        &self.correlation
    }

    pub fn rebuttals(&self) -> &[RebuttalV1] {
        &self.rebuttals
    }

    pub fn dispositions(&self) -> &[FindingDispositionV1] {
        &self.dispositions
    }

    pub fn refresh_human_gate(
        &mut self,
        deliberation: &DeliberationV1,
    ) -> Result<(), ReviewRefusalV1> {
        if deliberation.plan().id != self.deliberation || deliberation.reports() != self.reports {
            return Err(ReviewRefusalV1::DeliberationMismatch);
        }
        let response = deliberation.human_gate_response().copied();
        if self.admitted_human_gate_response.is_some()
            && self.admitted_human_gate_response != response
        {
            return Err(ReviewRefusalV1::HumanGateMismatch);
        }
        self.admitted_human_gate_response = response;
        Ok(())
    }

    pub fn submit_finding(&mut self, finding: ReviewFindingV1) -> Result<(), ReviewRefusalV1> {
        if finding.id.0 == [0; 16]
            || finding.owner == [0; 16]
            || finding.finding_digest == [0; 32]
            || finding.evidence_digests.len() > self.limits.max_evidence_per_item
        {
            return Err(
                if finding.evidence_digests.len() > self.limits.max_evidence_per_item {
                    ReviewRefusalV1::EvidenceLimit
                } else {
                    ReviewRefusalV1::InvalidFinding
                },
            );
        }
        if !self
            .reports
            .iter()
            .any(|report| report.id == finding.source_report)
        {
            return Err(ReviewRefusalV1::UnknownReport);
        }
        if let Some(existing) = self.findings.iter().find(|value| value.id == finding.id) {
            return if existing == &finding {
                Ok(())
            } else {
                Err(ReviewRefusalV1::FindingConflict)
            };
        }
        if self.findings.len() == self.limits.max_findings {
            return Err(ReviewRefusalV1::FindingLimit);
        }
        self.findings.push(finding);
        Ok(())
    }

    pub fn submit_rebuttal(&mut self, rebuttal: RebuttalV1) -> Result<(), ReviewRefusalV1> {
        let finding = self
            .findings
            .iter()
            .find(|value| value.id == rebuttal.finding)
            .ok_or(ReviewRefusalV1::UnknownFinding)?;
        if self
            .dispositions
            .iter()
            .any(|value| value.finding == rebuttal.finding)
        {
            return Err(ReviewRefusalV1::AlreadyDisposed);
        }
        if rebuttal.id.0 == [0; 16]
            || rebuttal.rebuttal_digest == [0; 32]
            || rebuttal.evidence_digests.len() > self.limits.max_evidence_per_item
        {
            return Err(
                if rebuttal.evidence_digests.len() > self.limits.max_evidence_per_item {
                    ReviewRefusalV1::EvidenceLimit
                } else {
                    ReviewRefusalV1::InvalidRebuttal
                },
            );
        }
        if !self
            .reports
            .iter()
            .any(|report| report.member == rebuttal.member)
        {
            return Err(ReviewRefusalV1::UnknownMember);
        }
        let source_member = self
            .reports
            .iter()
            .find(|report| report.id == finding.source_report)
            .map(|report| report.member)
            .ok_or(ReviewRefusalV1::UnknownReport)?;
        if source_member == rebuttal.member {
            return Err(ReviewRefusalV1::SelfRebuttal);
        }
        if let Some(existing) = self.rebuttals.iter().find(|value| value.id == rebuttal.id) {
            return if existing == &rebuttal {
                Ok(())
            } else {
                Err(ReviewRefusalV1::RebuttalConflict)
            };
        }
        if self.rebuttals.len() == self.limits.max_rebuttals {
            return Err(ReviewRefusalV1::RebuttalLimit);
        }
        self.rebuttals.push(rebuttal);
        Ok(())
    }

    pub fn dispose_finding(
        &mut self,
        disposition: FindingDispositionV1,
    ) -> Result<(), ReviewRefusalV1> {
        let finding = self
            .findings
            .iter()
            .find(|value| value.id == disposition.finding)
            .ok_or(ReviewRefusalV1::UnknownFinding)?;
        if let Some(existing) = self
            .dispositions
            .iter()
            .find(|value| value.finding == disposition.finding || value.id == disposition.id)
        {
            return if existing == &disposition {
                Ok(())
            } else {
                Err(ReviewRefusalV1::DispositionConflict)
            };
        }
        if finding.owner != disposition.owner {
            return Err(ReviewRefusalV1::OwnerMismatch);
        }
        if !self
            .rebuttals
            .iter()
            .any(|value| value.finding == finding.id)
        {
            return Err(ReviewRefusalV1::RebuttalRequired);
        }
        if disposition.id.0 == [0; 16]
            || disposition.rationale_digest == [0; 32]
            || disposition.evidence_digests.len() > self.limits.max_evidence_per_item
        {
            return Err(
                if disposition.evidence_digests.len() > self.limits.max_evidence_per_item {
                    ReviewRefusalV1::EvidenceLimit
                } else {
                    ReviewRefusalV1::InvalidDisposition
                },
            );
        }
        if finding.requires_human_gate {
            let admitted = self
                .admitted_human_gate_response
                .ok_or(ReviewRefusalV1::HumanGateRequired)?;
            if admitted.disposition != HumanGateDispositionV1::Approved {
                return Err(ReviewRefusalV1::HumanGateNotApproved);
            }
            match disposition.human_gate_response {
                Some(response) if response == admitted.id => {}
                Some(_) => return Err(ReviewRefusalV1::HumanGateMismatch),
                None => return Err(ReviewRefusalV1::HumanGateRequired),
            }
        } else if disposition.human_gate_response.is_some()
            && disposition.human_gate_response
                != self
                    .admitted_human_gate_response
                    .map(|response| response.id)
        {
            return Err(ReviewRefusalV1::HumanGateMismatch);
        }
        self.dispositions.push(disposition);
        Ok(())
    }

    pub fn checkpoint(&self) -> ReviewExchangeCheckpointV1 {
        ReviewExchangeCheckpointV1 {
            version: 2,
            deliberation: self.deliberation,
            limits: self.limits,
            reports: self.reports.clone(),
            correlation: self.correlation.clone(),
            admitted_human_gate_response: self.admitted_human_gate_response,
            findings: self.findings.clone(),
            rebuttals: self.rebuttals.clone(),
            dispositions: self.dispositions.clone(),
        }
    }

    pub fn recover(checkpoint: ReviewExchangeCheckpointV1) -> Result<Self, ReviewRefusalV1> {
        if checkpoint.version != 2 {
            return Err(ReviewRefusalV1::UnsupportedVersion);
        }
        validate_limits(checkpoint.limits)?;
        validate_reports(&checkpoint.reports)?;
        validate_correlation(&checkpoint.correlation, checkpoint.reports.len())?;
        let expected = checkpoint.clone();
        let mut recovered = Self {
            deliberation: checkpoint.deliberation,
            limits: checkpoint.limits,
            reports: checkpoint.reports,
            correlation: checkpoint.correlation,
            admitted_human_gate_response: checkpoint.admitted_human_gate_response,
            findings: Vec::new(),
            rebuttals: Vec::new(),
            dispositions: Vec::new(),
        };
        for finding in checkpoint.findings {
            recovered.submit_finding(finding)?;
        }
        for rebuttal in checkpoint.rebuttals {
            recovered.submit_rebuttal(rebuttal)?;
        }
        for disposition in checkpoint.dispositions {
            recovered.dispose_finding(disposition)?;
        }
        if recovered.checkpoint() != expected {
            return Err(ReviewRefusalV1::CheckpointMismatch);
        }
        Ok(recovered)
    }
}

fn validate_correlation(
    evidence: &EnsembleCorrelationEvidenceV1,
    report_count: usize,
) -> Result<(), ReviewRefusalV1> {
    if evidence.members == 0
        || evidence.members != report_count
        || evidence.dimensions.len() != CorrelationDimensionV1::ALL.len()
        || evidence
            .first_pass
            .committed_before_cross_review
            .checked_add(evidence.first_pass.exposed_before_commit)
            .and_then(|count| count.checked_add(evidence.first_pass.unknown))
            != Some(evidence.members)
    {
        return Err(ReviewRefusalV1::InvalidCorrelationEvidence);
    }
    let mut dimensions = BTreeSet::new();
    for dimension in &evidence.dimensions {
        if !dimensions.insert(dimension.dimension)
            || dimension.observed_members > evidence.members
            || dimension.distinct_values > dimension.observed_members
            || (dimension.observed_members == 0) != (dimension.distinct_values == 0)
        {
            return Err(ReviewRefusalV1::InvalidCorrelationEvidence);
        }
    }
    if dimensions != CorrelationDimensionV1::ALL.into_iter().collect() {
        return Err(ReviewRefusalV1::InvalidCorrelationEvidence);
    }
    Ok(())
}

fn validate_limits(limits: ReviewLimitsV1) -> Result<(), ReviewRefusalV1> {
    if limits.max_findings == 0 || limits.max_rebuttals == 0 || limits.max_evidence_per_item == 0 {
        return Err(ReviewRefusalV1::InvalidLimits);
    }
    Ok(())
}

fn validate_reports(reports: &[MemberReportV1]) -> Result<(), ReviewRefusalV1> {
    if reports.is_empty() {
        return Err(ReviewRefusalV1::FirstPassIncomplete);
    }
    for (index, report) in reports.iter().enumerate() {
        if report.id.0 == [0; 16]
            || report.member.0 == [0; 16]
            || reports[..index]
                .iter()
                .any(|prior| prior.id == report.id || prior.member == report.member)
        {
            return Err(ReviewRefusalV1::InvalidReports);
        }
    }
    Ok(())
}
