//! Projects retained Moira review facts into addressed Agora messages.
//!
//! This adapter does not infer review state from chat and does not turn room
//! participation into disposition authority. Callers publish only facts already
//! retained by Moira's review exchange.

use moira_core::deliberation::{
    DeliberationIdV1, FindingDispositionIdV1, FindingIdV1, HumanGateResponseIdV1, RebuttalIdV1,
    ReportIdV1,
    review::{FindingDispositionKindV1, FindingDispositionV1, RebuttalV1, ReviewFindingV1},
};
use moira_protocol::EffectIdV1;

use crate::{
    PublicationIntentV1, PublicationReplyV1,
    durable::{DurablePublishErrorV1, DurableResponsePublisher, PublisherStateStoreV1},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewPublicationRouteV1 {
    pub deliberation: DeliberationIdV1,
    pub effect: EffectIdV1,
    pub source: [u8; 16],
    pub room_sequence: u64,
    pub room_id: String,
    pub expected_agora_revision: u64,
    pub logical_time: u64,
    pub addressed_to: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewPublicationRefV1<'a> {
    Finding(&'a ReviewFindingV1),
    Rebuttal(&'a RebuttalV1),
    Disposition(&'a FindingDispositionV1),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewProjectionRefusalV1 {
    InvalidDeliberation,
    Unaddressed,
    DuplicateAddress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewPublicationItemV1 {
    Finding(FindingIdV1),
    Rebuttal {
        rebuttal: RebuttalIdV1,
        finding: FindingIdV1,
    },
    Disposition {
        disposition: FindingDispositionIdV1,
        finding: FindingIdV1,
    },
}

/// Exact owner correlation carried beside the Agora transport intent.
///
/// Agora does not interpret this value. A caller reconstructs it from Moira's
/// retained review after restart and presents it with the durable publication
/// outcome, without parsing the projected chat body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewPublicationCorrelationV1 {
    pub deliberation: DeliberationIdV1,
    pub item: ReviewPublicationItemV1,
    pub effect: EffectIdV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewPublicationIntentV1 {
    correlation: ReviewPublicationCorrelationV1,
    publication: PublicationIntentV1,
}

impl ReviewPublicationIntentV1 {
    pub const fn correlation(&self) -> ReviewPublicationCorrelationV1 {
        self.correlation
    }

    pub fn body(&self) -> &str {
        &self.publication.body
    }

    pub fn addressed_to(&self) -> &[String] {
        &self.publication.addressed_to
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewPublicationOutcomeV1 {
    pub correlation: ReviewPublicationCorrelationV1,
    pub reply: PublicationReplyV1,
    pub replayed: bool,
    pub storage_revision: u64,
}

pub fn project_review_publication(
    route: ReviewPublicationRouteV1,
    publication: ReviewPublicationRefV1<'_>,
) -> Result<ReviewPublicationIntentV1, ReviewProjectionRefusalV1> {
    if route.deliberation.0 == [0; 16] {
        return Err(ReviewProjectionRefusalV1::InvalidDeliberation);
    }
    let correlation = ReviewPublicationCorrelationV1 {
        deliberation: route.deliberation,
        item: item(publication),
        effect: route.effect,
    };
    let mut addressed_to = route.addressed_to;
    if addressed_to.is_empty() {
        return Err(ReviewProjectionRefusalV1::Unaddressed);
    }
    addressed_to.sort();
    if addressed_to.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ReviewProjectionRefusalV1::DuplicateAddress);
    }
    Ok(ReviewPublicationIntentV1 {
        correlation,
        publication: PublicationIntentV1 {
            effect: route.effect,
            source: route.source,
            room_sequence: route.room_sequence,
            room_id: route.room_id,
            expected_agora_revision: route.expected_agora_revision,
            logical_time: route.logical_time,
            body: body(publication),
            addressed_to,
        },
    })
}

/// Publish one exact retained review item and return a typed correlation to the
/// durable transport outcome. Retrying the same projected intent after restart
/// reuses the generic publisher's exact replay contract.
pub fn publish_review<S, P>(
    publisher: &mut DurableResponsePublisher<S, P>,
    intent: &ReviewPublicationIntentV1,
) -> Result<ReviewPublicationOutcomeV1, DurablePublishErrorV1<S::Error, P::Error>>
where
    S: PublisherStateStoreV1,
    P: agora_client::v2::ServicePortV2,
{
    let outcome = publisher.publish(intent.publication.clone())?;
    Ok(ReviewPublicationOutcomeV1 {
        correlation: intent.correlation,
        reply: outcome.reply,
        replayed: outcome.replayed,
        storage_revision: outcome.storage_revision,
    })
}

fn item(publication: ReviewPublicationRefV1<'_>) -> ReviewPublicationItemV1 {
    match publication {
        ReviewPublicationRefV1::Finding(value) => ReviewPublicationItemV1::Finding(value.id),
        ReviewPublicationRefV1::Rebuttal(value) => ReviewPublicationItemV1::Rebuttal {
            rebuttal: value.id,
            finding: value.finding,
        },
        ReviewPublicationRefV1::Disposition(value) => ReviewPublicationItemV1::Disposition {
            disposition: value.id,
            finding: value.finding,
        },
    }
}

fn body(publication: ReviewPublicationRefV1<'_>) -> String {
    match publication {
        ReviewPublicationRefV1::Finding(value) => format!(
            "moira.review.v1 kind=finding id={} source_report={} digest={} evidence_count={} human_gate={}",
            finding_id(value.id),
            report_id(value.source_report),
            hex(&value.finding_digest),
            value.evidence_digests.len(),
            if value.requires_human_gate {
                "required"
            } else {
                "not_required"
            },
        ),
        ReviewPublicationRefV1::Rebuttal(value) => format!(
            "moira.review.v1 kind=rebuttal id={} finding={} member={} digest={} evidence_count={}",
            rebuttal_id(value.id),
            finding_id(value.finding),
            hex(&value.member.0),
            hex(&value.rebuttal_digest),
            value.evidence_digests.len(),
        ),
        ReviewPublicationRefV1::Disposition(value) => format!(
            "moira.review.v1 kind=disposition id={} finding={} owner={} disposition={} rationale={} evidence_count={} human_gate_response={}",
            disposition_id(value.id),
            finding_id(value.finding),
            hex(&value.owner),
            disposition_kind(value.kind),
            hex(&value.rationale_digest),
            value.evidence_digests.len(),
            value
                .human_gate_response
                .map(human_gate_response_id)
                .unwrap_or_else(|| "none".to_owned()),
        ),
    }
}

fn disposition_kind(value: FindingDispositionKindV1) -> &'static str {
    match value {
        FindingDispositionKindV1::Accepted => "accepted",
        FindingDispositionKindV1::Mitigated => "mitigated",
        FindingDispositionKindV1::Tested => "tested",
        FindingDispositionKindV1::Deferred => "deferred",
        FindingDispositionKindV1::RiskAccepted => "risk_accepted",
        FindingDispositionKindV1::Disputed => "disputed",
        FindingDispositionKindV1::OutOfScope => "out_of_scope",
    }
}

fn finding_id(value: FindingIdV1) -> String {
    hex(&value.0)
}

fn report_id(value: ReportIdV1) -> String {
    hex(&value.0)
}

fn rebuttal_id(value: RebuttalIdV1) -> String {
    hex(&value.0)
}

fn disposition_id(value: FindingDispositionIdV1) -> String {
    hex(&value.0)
}

fn human_gate_response_id(value: HumanGateResponseIdV1) -> String {
    hex(&value.0)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use moira_core::deliberation::{EnsembleMemberIdV1, FindingDispositionIdV1};

    fn route() -> ReviewPublicationRouteV1 {
        ReviewPublicationRouteV1 {
            deliberation: DeliberationIdV1([20; 16]),
            effect: EffectIdV1([1; 16]),
            source: [2; 16],
            room_sequence: 3,
            room_id: "review-room".to_owned(),
            expected_agora_revision: 4,
            logical_time: 5,
            addressed_to: vec!["work-owner".to_owned(), "human-advocate".to_owned()],
        }
    }

    #[test]
    fn projections_are_addressed_and_preserve_exact_retained_references() {
        let finding = ReviewFindingV1 {
            id: FindingIdV1([6; 16]),
            source_report: ReportIdV1([7; 16]),
            owner: [8; 16],
            finding_digest: [9; 32],
            evidence_digests: vec![[10; 32]],
            requires_human_gate: true,
        };
        let rebuttal = RebuttalV1 {
            id: RebuttalIdV1([11; 16]),
            finding: finding.id,
            member: EnsembleMemberIdV1([12; 16]),
            rebuttal_digest: [13; 32],
            evidence_digests: vec![[14; 32]],
        };
        let disposition = FindingDispositionV1 {
            id: FindingDispositionIdV1([15; 16]),
            finding: finding.id,
            owner: finding.owner,
            kind: FindingDispositionKindV1::RiskAccepted,
            rationale_digest: [16; 32],
            evidence_digests: vec![[17; 32]],
            human_gate_response: Some(HumanGateResponseIdV1([18; 16])),
            decided_at: 19,
        };

        let finding_message =
            project_review_publication(route(), ReviewPublicationRefV1::Finding(&finding)).unwrap();
        assert_eq!(
            finding_message.correlation(),
            ReviewPublicationCorrelationV1 {
                deliberation: DeliberationIdV1([20; 16]),
                item: ReviewPublicationItemV1::Finding(finding.id),
                effect: EffectIdV1([1; 16]),
            }
        );
        assert!(finding_message.body().contains("kind=finding"));
        assert!(finding_message.body().contains("human_gate=required"));
        assert_eq!(finding_message.addressed_to().len(), 2);
        assert_eq!(
            finding_message.addressed_to(),
            vec!["human-advocate", "work-owner"]
        );

        let rebuttal_message =
            project_review_publication(route(), ReviewPublicationRefV1::Rebuttal(&rebuttal))
                .unwrap();
        assert!(rebuttal_message.body().contains("kind=rebuttal"));
        assert!(
            rebuttal_message
                .body()
                .contains(&hex(&rebuttal.rebuttal_digest))
        );

        let disposition_message =
            project_review_publication(route(), ReviewPublicationRefV1::Disposition(&disposition))
                .unwrap();
        assert!(
            disposition_message
                .body()
                .contains("disposition=risk_accepted")
        );
        assert!(
            disposition_message
                .body()
                .contains(&hex(&HumanGateResponseIdV1([18; 16]).0))
        );
    }

    #[test]
    fn unaddressed_review_projection_is_refused() {
        let finding = ReviewFindingV1 {
            id: FindingIdV1([1; 16]),
            source_report: ReportIdV1([2; 16]),
            owner: [3; 16],
            finding_digest: [4; 32],
            evidence_digests: vec![],
            requires_human_gate: false,
        };
        let mut route = route();
        route.addressed_to.clear();
        assert_eq!(
            project_review_publication(route, ReviewPublicationRefV1::Finding(&finding)),
            Err(ReviewProjectionRefusalV1::Unaddressed)
        );
    }

    #[test]
    fn duplicate_review_address_is_refused_before_agora_dispatch() {
        let finding = ReviewFindingV1 {
            id: FindingIdV1([1; 16]),
            source_report: ReportIdV1([2; 16]),
            owner: [3; 16],
            finding_digest: [4; 32],
            evidence_digests: vec![],
            requires_human_gate: false,
        };
        let mut route = route();
        route.addressed_to = vec!["work-owner".to_owned(), "work-owner".to_owned()];
        assert_eq!(
            project_review_publication(route, ReviewPublicationRefV1::Finding(&finding)),
            Err(ReviewProjectionRefusalV1::DuplicateAddress)
        );
    }

    #[test]
    fn zero_deliberation_cannot_create_a_correlation() {
        let finding = ReviewFindingV1 {
            id: FindingIdV1([1; 16]),
            source_report: ReportIdV1([2; 16]),
            owner: [3; 16],
            finding_digest: [4; 32],
            evidence_digests: vec![],
            requires_human_gate: false,
        };
        let mut route = route();
        route.deliberation = DeliberationIdV1([0; 16]);
        assert_eq!(
            project_review_publication(route, ReviewPublicationRefV1::Finding(&finding)),
            Err(ReviewProjectionRefusalV1::InvalidDeliberation)
        );
    }
}
