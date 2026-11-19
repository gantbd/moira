//! Revisioned ownership for multiple methodical deliberations.
//!
//! The registry is deterministic and performs no I/O. Matching replay observes
//! retained state without advancing the revision; conflicting identity reuse and
//! stale optimistic revisions refuse before mutation.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

pub mod persistence;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct RegistryRevisionV1(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistryLimitsV1 {
    pub max_deliberations: usize,
    pub deliberation: DeliberationLimitsV1,
}

impl Default for RegistryLimitsV1 {
    fn default() -> Self {
        Self {
            max_deliberations: 1_024,
            deliberation: DeliberationLimitsV1::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryCommandV1 {
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
    RespondHumanGate {
        deliberation: DeliberationIdV1,
        response: HumanGateResponseV1,
    },
    BeginReview {
        deliberation: DeliberationIdV1,
    },
    SubmitFinding {
        deliberation: DeliberationIdV1,
        finding: review::ReviewFindingV1,
    },
    SubmitRebuttal {
        deliberation: DeliberationIdV1,
        rebuttal: review::RebuttalV1,
    },
    DisposeFinding {
        deliberation: DeliberationIdV1,
        disposition: review::FindingDispositionV1,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryReceiptV1 {
    pub revision: RegistryRevisionV1,
    pub deliberation: DeliberationCheckpointV1,
    pub review: Option<review::ReviewExchangeCheckpointV1>,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryCheckpointV1 {
    pub version: u8,
    pub limits: RegistryLimitsV1,
    pub revision: RegistryRevisionV1,
    pub deliberations: Vec<DeliberationCheckpointV1>,
    pub reviews: Vec<review::ReviewExchangeCheckpointV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryRefusalV1 {
    InvalidLimits,
    UnsupportedVersion,
    RevisionConflict {
        expected: RegistryRevisionV1,
        actual: RegistryRevisionV1,
    },
    RevisionExhausted,
    Capacity,
    UnknownDeliberation,
    InputConflict,
    DuplicateDeliberation,
    CheckpointMismatch,
    Deliberation(DeliberationRefusalV1),
    Review(review::ReviewRefusalV1),
    DuplicateReview,
    ReviewNotBegun,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliberationRegistryV1 {
    limits: RegistryLimitsV1,
    revision: RegistryRevisionV1,
    deliberations: BTreeMap<DeliberationIdV1, DeliberationV1>,
    reviews: BTreeMap<DeliberationIdV1, review::ReviewExchangeV1>,
}

impl DeliberationRegistryV1 {
    pub fn try_new(limits: RegistryLimitsV1) -> Result<Self, RegistryRefusalV1> {
        validate_limits(limits)?;
        Ok(Self {
            limits,
            revision: RegistryRevisionV1(0),
            deliberations: BTreeMap::new(),
            reviews: BTreeMap::new(),
        })
    }

    pub const fn revision(&self) -> RegistryRevisionV1 {
        self.revision
    }

    pub fn deliberation(&self, id: DeliberationIdV1) -> Option<DeliberationCheckpointV1> {
        self.deliberations.get(&id).map(DeliberationV1::checkpoint)
    }

    pub fn review(&self, id: DeliberationIdV1) -> Option<review::ReviewExchangeCheckpointV1> {
        self.reviews
            .get(&id)
            .map(review::ReviewExchangeV1::checkpoint)
    }

    pub fn checkpoint(&self) -> RegistryCheckpointV1 {
        RegistryCheckpointV1 {
            version: 4,
            limits: self.limits,
            revision: self.revision,
            deliberations: self
                .deliberations
                .values()
                .map(DeliberationV1::checkpoint)
                .collect(),
            reviews: self
                .reviews
                .values()
                .map(review::ReviewExchangeV1::checkpoint)
                .collect(),
        }
    }

    pub fn apply(
        &mut self,
        expected_revision: RegistryRevisionV1,
        command: RegistryCommandV1,
    ) -> Result<RegistryReceiptV1, RegistryRefusalV1> {
        if expected_revision != self.revision {
            return Err(RegistryRefusalV1::RevisionConflict {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        match command {
            RegistryCommandV1::Admit(plan) => self.admit(*plan),
            RegistryCommandV1::SubmitReport {
                deliberation,
                report,
            } => self.update(deliberation, |state| state.submit_report(report)),
            RegistryCommandV1::Resolve {
                deliberation,
                resolution,
            } => self.update(deliberation, |state| state.resolve(resolution)),
            RegistryCommandV1::RequestHumanGate {
                deliberation,
                request,
            } => self.update(deliberation, |state| state.request_human_gate(request)),
            RegistryCommandV1::RespondHumanGate {
                deliberation,
                response,
            } => self.respond_human_gate(deliberation, response),
            RegistryCommandV1::BeginReview { deliberation } => self.begin_review(deliberation),
            RegistryCommandV1::SubmitFinding {
                deliberation,
                finding,
            } => self.update_review(deliberation, |state| state.submit_finding(finding)),
            RegistryCommandV1::SubmitRebuttal {
                deliberation,
                rebuttal,
            } => self.update_review(deliberation, |state| state.submit_rebuttal(rebuttal)),
            RegistryCommandV1::DisposeFinding {
                deliberation,
                disposition,
            } => self.update_review(deliberation, |state| state.dispose_finding(disposition)),
        }
    }

    pub fn recover(checkpoint: RegistryCheckpointV1) -> Result<Self, RegistryRefusalV1> {
        if checkpoint.version != 4 {
            return Err(RegistryRefusalV1::UnsupportedVersion);
        }
        let expected = checkpoint.clone();
        validate_limits(checkpoint.limits)?;
        if checkpoint.deliberations.len() > checkpoint.limits.max_deliberations {
            return Err(RegistryRefusalV1::Capacity);
        }
        let mut deliberations = BTreeMap::new();
        let mut accepted = 0u64;
        let mut seen = BTreeSet::new();
        for retained in checkpoint.deliberations {
            if !seen.insert(retained.plan.id) {
                return Err(RegistryRefusalV1::DuplicateDeliberation);
            }
            accepted = accepted
                .checked_add(1)
                .and_then(|value| value.checked_add(u64::try_from(retained.reports.len()).ok()?))
                .and_then(|value| value.checked_add(u64::from(retained.resolution.is_some())))
                .and_then(|value| {
                    value.checked_add(u64::from(retained.human_gate_request.is_some()))
                })
                .and_then(|value| {
                    value.checked_add(u64::from(retained.human_gate_response.is_some()))
                })
                .ok_or(RegistryRefusalV1::RevisionExhausted)?;
            let id = retained.plan.id;
            let deliberation =
                DeliberationV1::recover(retained).map_err(RegistryRefusalV1::Deliberation)?;
            deliberations.insert(id, deliberation);
        }
        let mut reviews = BTreeMap::new();
        for retained in checkpoint.reviews {
            if reviews.contains_key(&retained.deliberation) {
                return Err(RegistryRefusalV1::DuplicateReview);
            }
            let deliberation = deliberations
                .get(&retained.deliberation)
                .ok_or(RegistryRefusalV1::UnknownDeliberation)?;
            if retained.reports != deliberation.reports()
                || retained.admitted_human_gate_response
                    != deliberation.human_gate_response().copied()
            {
                return Err(RegistryRefusalV1::CheckpointMismatch);
            }
            accepted = accepted
                .checked_add(1)
                .and_then(|value| value.checked_add(u64::try_from(retained.findings.len()).ok()?))
                .and_then(|value| value.checked_add(u64::try_from(retained.rebuttals.len()).ok()?))
                .and_then(|value| {
                    value.checked_add(u64::try_from(retained.dispositions.len()).ok()?)
                })
                .ok_or(RegistryRefusalV1::RevisionExhausted)?;
            let id = retained.deliberation;
            let review =
                review::ReviewExchangeV1::recover(retained).map_err(RegistryRefusalV1::Review)?;
            reviews.insert(id, review);
        }
        if checkpoint.revision != RegistryRevisionV1(accepted) {
            return Err(RegistryRefusalV1::CheckpointMismatch);
        }
        let recovered = Self {
            limits: checkpoint.limits,
            revision: checkpoint.revision,
            deliberations,
            reviews,
        };
        if recovered.checkpoint() != expected {
            return Err(RegistryRefusalV1::CheckpointMismatch);
        }
        Ok(recovered)
    }

    fn admit(&mut self, plan: DeliberationPlanV1) -> Result<RegistryReceiptV1, RegistryRefusalV1> {
        if let Some(existing) = self.deliberations.get(&plan.id) {
            let checkpoint = existing.checkpoint();
            return if checkpoint.plan == plan {
                Ok(RegistryReceiptV1 {
                    revision: self.revision,
                    deliberation: checkpoint,
                    review: self.review(plan.id),
                    replayed: true,
                })
            } else {
                Err(RegistryRefusalV1::InputConflict)
            };
        }
        if self.deliberations.len() == self.limits.max_deliberations {
            return Err(RegistryRefusalV1::Capacity);
        }
        let next_revision = self.next_revision()?;
        let deliberation = DeliberationV1::try_new(plan, self.limits.deliberation)
            .map_err(RegistryRefusalV1::Deliberation)?;
        let id = deliberation.checkpoint().plan.id;
        let checkpoint = deliberation.checkpoint();
        self.deliberations.insert(id, deliberation);
        self.revision = next_revision;
        Ok(RegistryReceiptV1 {
            revision: self.revision,
            deliberation: checkpoint,
            review: None,
            replayed: false,
        })
    }

    fn update(
        &mut self,
        id: DeliberationIdV1,
        mutation: impl FnOnce(&mut DeliberationV1) -> Result<(), DeliberationRefusalV1>,
    ) -> Result<RegistryReceiptV1, RegistryRefusalV1> {
        let current = self
            .deliberations
            .get(&id)
            .ok_or(RegistryRefusalV1::UnknownDeliberation)?;
        let before = current.checkpoint();
        let mut candidate = current.clone();
        mutation(&mut candidate).map_err(RegistryRefusalV1::Deliberation)?;
        let after = candidate.checkpoint();
        if after == before {
            return Ok(RegistryReceiptV1 {
                revision: self.revision,
                deliberation: before,
                review: self.review(id),
                replayed: true,
            });
        }
        let next_revision = self.next_revision()?;
        self.deliberations.insert(id, candidate);
        self.revision = next_revision;
        Ok(RegistryReceiptV1 {
            revision: self.revision,
            deliberation: after,
            review: self.review(id),
            replayed: false,
        })
    }

    fn respond_human_gate(
        &mut self,
        id: DeliberationIdV1,
        response: HumanGateResponseV1,
    ) -> Result<RegistryReceiptV1, RegistryRefusalV1> {
        let current = self
            .deliberations
            .get(&id)
            .ok_or(RegistryRefusalV1::UnknownDeliberation)?;
        let before = current.checkpoint();
        let mut candidate = current.clone();
        candidate
            .respond_human_gate(response)
            .map_err(RegistryRefusalV1::Deliberation)?;
        let after = candidate.checkpoint();
        let mut review = self.reviews.get(&id).cloned();
        if let Some(value) = review.as_mut() {
            value
                .refresh_human_gate(&candidate)
                .map_err(RegistryRefusalV1::Review)?;
        }
        if after == before {
            return Ok(RegistryReceiptV1 {
                revision: self.revision,
                deliberation: before,
                review: review.map(|value| value.checkpoint()),
                replayed: true,
            });
        }
        let next_revision = self.next_revision()?;
        self.deliberations.insert(id, candidate);
        if let Some(value) = review {
            self.reviews.insert(id, value);
        }
        self.revision = next_revision;
        Ok(RegistryReceiptV1 {
            revision: self.revision,
            deliberation: after,
            review: self.review(id),
            replayed: false,
        })
    }

    fn begin_review(
        &mut self,
        id: DeliberationIdV1,
    ) -> Result<RegistryReceiptV1, RegistryRefusalV1> {
        let deliberation = self
            .deliberations
            .get(&id)
            .ok_or(RegistryRefusalV1::UnknownDeliberation)?;
        if let Some(existing) = self.reviews.get(&id) {
            return Ok(RegistryReceiptV1 {
                revision: self.revision,
                deliberation: deliberation.checkpoint(),
                review: Some(existing.checkpoint()),
                replayed: true,
            });
        }
        let review =
            review::ReviewExchangeV1::try_new(deliberation, review::ReviewLimitsV1::default())
                .map_err(RegistryRefusalV1::Review)?;
        let next_revision = self.next_revision()?;
        let checkpoint = review.checkpoint();
        self.reviews.insert(id, review);
        self.revision = next_revision;
        Ok(RegistryReceiptV1 {
            revision: self.revision,
            deliberation: deliberation.checkpoint(),
            review: Some(checkpoint),
            replayed: false,
        })
    }

    fn update_review(
        &mut self,
        id: DeliberationIdV1,
        mutation: impl FnOnce(&mut review::ReviewExchangeV1) -> Result<(), review::ReviewRefusalV1>,
    ) -> Result<RegistryReceiptV1, RegistryRefusalV1> {
        let deliberation = self
            .deliberations
            .get(&id)
            .ok_or(RegistryRefusalV1::UnknownDeliberation)?;
        let current = self
            .reviews
            .get(&id)
            .ok_or(RegistryRefusalV1::ReviewNotBegun)?;
        let before = current.checkpoint();
        let mut candidate = current.clone();
        mutation(&mut candidate).map_err(RegistryRefusalV1::Review)?;
        let after = candidate.checkpoint();
        if after == before {
            return Ok(RegistryReceiptV1 {
                revision: self.revision,
                deliberation: deliberation.checkpoint(),
                review: Some(before),
                replayed: true,
            });
        }
        let next_revision = self.next_revision()?;
        self.reviews.insert(id, candidate);
        self.revision = next_revision;
        Ok(RegistryReceiptV1 {
            revision: self.revision,
            deliberation: deliberation.checkpoint(),
            review: Some(after),
            replayed: false,
        })
    }

    fn next_revision(&self) -> Result<RegistryRevisionV1, RegistryRefusalV1> {
        self.revision
            .0
            .checked_add(1)
            .map(RegistryRevisionV1)
            .ok_or(RegistryRefusalV1::RevisionExhausted)
    }
}

fn validate_limits(limits: RegistryLimitsV1) -> Result<(), RegistryRefusalV1> {
    let domain = limits.deliberation;
    if limits.max_deliberations == 0
        || domain.max_claims == 0
        || domain.max_criteria == 0
        || domain.max_candidates == 0
        || domain.max_members == 0
        || domain.max_evidence_per_report == 0
        || domain.max_dissent_per_report == 0
        || domain.max_fuel == 0
    {
        Err(RegistryRefusalV1::InvalidLimits)
    } else {
        Ok(())
    }
}
