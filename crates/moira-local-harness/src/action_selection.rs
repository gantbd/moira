//! Selects one exact terminal member result before peer-effect admission.
//!
//! Pareto candidates and member reports are different identities. This module
//! binds the retained report/result pair directly and asks a separate authority
//! when the one-member chatbot policy does not apply.

use moira_core::EffectId;
use moira_core::deliberation::{
    AuthorityRefV1, DeliberationCheckpointV1,
    persistence::{CodecLimitsV1, encode},
};
use moira_daimon::{ModelResultV1, ensemble::MemberOutcomeV1};
use moira_host::{ActionSelectionSubjectV1, ActionSelectorV1};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionSelectionEvidenceV1 {
    SingleMemberPolicy { policy_digest: [u8; 32] },
    Authorized { authority: AuthorityRefV1 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectedPeerActionV1<'a> {
    pub(crate) subject: ActionSelectionSubjectV1,
    pub(crate) evidence: ActionSelectionEvidenceV1,
    pub(crate) selection_digest: [u8; 32],
    pub(crate) result: &'a ModelResultV1,
}

impl SelectedPeerActionV1<'_> {
    pub const fn subject(&self) -> ActionSelectionSubjectV1 {
        self.subject
    }

    pub const fn evidence(&self) -> ActionSelectionEvidenceV1 {
        self.evidence
    }

    pub const fn selection_digest(&self) -> [u8; 32] {
        self.selection_digest
    }

    pub const fn result(&self) -> &ModelResultV1 {
        self.result
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ActionSelectionErrorV1<AuthorityError> {
    NotReport,
    ReportNotRetained,
    OutputMismatch,
    NotSingleMember,
    InvalidPolicy,
    Unresolved,
    IncompatibleResolution,
    CheckpointEncoding,
    Authority(AuthorityError),
    InvalidDecision,
}

pub fn select_single_member<'a, E, AuthorityError>(
    checkpoint: &DeliberationCheckpointV1,
    outcome: &'a MemberOutcomeV1<E>,
    policy_digest: [u8; 32],
) -> Result<SelectedPeerActionV1<'a>, ActionSelectionErrorV1<AuthorityError>> {
    if policy_digest == [0; 32] {
        return Err(ActionSelectionErrorV1::InvalidPolicy);
    }
    if checkpoint.plan.members.len() != 1 || checkpoint.reports.len() != 1 {
        return Err(ActionSelectionErrorV1::NotSingleMember);
    }
    let (subject, result) = subject(checkpoint, outcome)?;
    let evidence = ActionSelectionEvidenceV1::SingleMemberPolicy { policy_digest };
    Ok(SelectedPeerActionV1 {
        subject,
        evidence,
        selection_digest: selection_digest(subject, evidence),
        result,
    })
}

pub fn select_authorized<'a, E, A>(
    checkpoint: &DeliberationCheckpointV1,
    outcome: &'a MemberOutcomeV1<E>,
    selector: &mut A,
) -> Result<SelectedPeerActionV1<'a>, ActionSelectionErrorV1<A::Error>>
where
    A: ActionSelectorV1,
{
    match checkpoint.resolution {
        None => return Err(ActionSelectionErrorV1::Unresolved),
        Some(moira_core::deliberation::ResolutionV1::AuthorizedChoice { .. }) => {}
        Some(_) => return Err(ActionSelectionErrorV1::IncompatibleResolution),
    }
    let (subject, result) = subject(checkpoint, outcome)?;
    let decision = selector
        .select(subject)
        .map_err(ActionSelectionErrorV1::Authority)?;
    if decision.subject != subject {
        return Err(ActionSelectionErrorV1::InvalidDecision);
    }
    let evidence = ActionSelectionEvidenceV1::Authorized {
        authority: decision.authority,
    };
    Ok(SelectedPeerActionV1 {
        subject,
        evidence,
        selection_digest: selection_digest(subject, evidence),
        result,
    })
}

fn subject<'a, E, AuthorityError>(
    checkpoint: &DeliberationCheckpointV1,
    outcome: &'a MemberOutcomeV1<E>,
) -> Result<(ActionSelectionSubjectV1, &'a ModelResultV1), ActionSelectionErrorV1<AuthorityError>> {
    let MemberOutcomeV1::Report {
        member,
        report,
        result,
        ..
    } = outcome
    else {
        return Err(ActionSelectionErrorV1::NotReport);
    };
    if !checkpoint.reports.iter().any(|retained| retained == report) {
        return Err(ActionSelectionErrorV1::ReportNotRetained);
    }
    let output_digest: [u8; 32] = Sha256::digest(&result.output).into();
    if output_digest != report.output_digest || report.member != *member {
        return Err(ActionSelectionErrorV1::OutputMismatch);
    }
    let bytes = encode(
        checkpoint,
        CodecLimitsV1 {
            max_bytes: 8 * 1_024 * 1_024,
            domain: checkpoint.limits,
        },
    )
    .map_err(|_| ActionSelectionErrorV1::CheckpointEncoding)?;
    let mut hash = Sha256::new();
    hash.update(b"MOIRAACTIONSUBJECT1");
    hash.update(bytes);
    let checkpoint_digest = hash.finalize().into();
    Ok((
        ActionSelectionSubjectV1 {
            deliberation: checkpoint.plan.id,
            checkpoint_digest,
            member: *member,
            report: report.id,
            output_digest,
            daimon_effect: EffectId::new(result.receipt.effect.0),
        },
        result,
    ))
}

fn selection_digest(
    subject: ActionSelectionSubjectV1,
    evidence: ActionSelectionEvidenceV1,
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRAACTIONSELECTION1");
    hash.update(subject.deliberation.0);
    hash.update(subject.checkpoint_digest);
    hash.update(subject.member.0);
    hash.update(subject.report.0);
    hash.update(subject.output_digest);
    hash.update(subject.daimon_effect.into_bytes());
    match evidence {
        ActionSelectionEvidenceV1::SingleMemberPolicy { policy_digest } => {
            hash.update([1]);
            hash.update(policy_digest);
        }
        ActionSelectionEvidenceV1::Authorized { authority } => {
            hash.update([2]);
            hash.update(authority.decision.0);
            hash.update(authority.subject.0);
            hash.update(authority.epoch.to_be_bytes());
        }
    }
    hash.finalize().into()
}
