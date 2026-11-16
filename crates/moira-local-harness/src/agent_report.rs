//! Admission gate from one terminal agent participation into a methodical
//! deliberation report. Provider completion is necessary but never sufficient:
//! the retained Rune method, Dendrite grounding, persona, member, provider
//! evidence, and cursor-complete durable output bytes must all match before the
//! registry is touched.

use moira_core::deliberation::{
    self as core, MemberReportV1, ReportIdV1,
    registry::{RegistryCommandV1, RegistryReceiptV1},
};
use moira_daimon::ensemble::{
    MemberResponseLimitsV1, MemberResponseRefusalV1, MemberResponseV1, parse_member_response,
};
use moira_host::{AgentOutputStoreV1, DeliberationApplyErrorV1, DeliberationStore};
use moira_protocol::{agent_participation as participation, deliberation as public};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Debug, Eq, PartialEq)]
pub enum AgentReportAdmissionErrorV1<StoreError, OutputError> {
    NeedsReopen,
    MissingDeliberation,
    InvalidParticipation,
    BindingMismatch,
    InvalidReport,
    MissingOutput,
    Output(OutputError),
    Response(MemberResponseRefusalV1),
    Store(DeliberationApplyErrorV1<StoreError>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmittedAgentReportV1 {
    pub receipt: RegistryReceiptV1,
    pub report: MemberReportV1,
    pub response: MemberResponseV1,
}

/// Parse and admit only the exact cursor-complete bytes retained for a terminal
/// participation. Report identity and baseline evidence are derived here; the
/// provider supplies neither.
pub fn admit_terminal_output<S: DeliberationStore, O: AgentOutputStoreV1>(
    store: &mut S,
    outputs: &mut O,
    participation: &participation::ParticipationViewV1,
    limits: MemberResponseLimitsV1,
) -> Result<AdmittedAgentReportV1, AgentReportAdmissionErrorV1<S::Error, O::Error>> {
    let terminal = participation
        .terminal
        .ok_or(AgentReportAdmissionErrorV1::InvalidParticipation)?;
    let (_, output) = retained_output(outputs, participation, terminal)?;
    let deliberation_id = core::DeliberationIdV1(participation.binding.deliberation.0);
    let retained = store
        .deliberation(deliberation_id)
        .ok_or(AgentReportAdmissionErrorV1::MissingDeliberation)?;
    let response = parse_member_response(&retained.plan, &output, limits)
        .map_err(AgentReportAdmissionErrorV1::Response)?;
    let mut evidence_digests = vec![
        retained.plan.question_digest,
        retained.plan.method.machine_digest,
        retained.plan.method.state_digest,
        participation.binding.grounding.authority_digest,
        participation.binding.grounding.surface_digest,
        terminal.provider_evidence_digest,
    ];
    let mut dissent_digests = Vec::new();
    for claim in &response.claims {
        evidence_digests.extend_from_slice(&claim.evidence_digests);
        dissent_digests.extend_from_slice(&claim.dissent_digests);
    }
    if evidence_digests.len() > limits.max_evidence_digests
        || dissent_digests.len() > limits.max_dissent_digests
        || !valid_digests(&evidence_digests)
        || !valid_digests(&dissent_digests)
    {
        return Err(AgentReportAdmissionErrorV1::InvalidReport);
    }
    let report = MemberReportV1 {
        id: derived_report_id(participation, terminal)?,
        member: core::EnsembleMemberIdV1(participation.binding.member.0),
        output_digest: terminal.output_digest,
        evidence_digests,
        dissent_digests,
    };
    let receipt = admit_terminal_report(store, outputs, participation, report.clone())?;
    Ok(AdmittedAgentReportV1 {
        receipt,
        report,
        response,
    })
}

pub fn admit_terminal_report<S: DeliberationStore, O: AgentOutputStoreV1>(
    store: &mut S,
    outputs: &mut O,
    participation: &participation::ParticipationViewV1,
    report: MemberReportV1,
) -> Result<RegistryReceiptV1, AgentReportAdmissionErrorV1<S::Error, O::Error>> {
    if store.needs_reopen() {
        return Err(AgentReportAdmissionErrorV1::NeedsReopen);
    }
    let terminal = participation
        .terminal
        .ok_or(AgentReportAdmissionErrorV1::InvalidParticipation)?;
    if participation.phase != participation::PhaseV1::Terminal
        || participation.pending.is_some()
        || participation.active_turn.is_some()
        || terminal.disposition != participation::DispositionV1::Completed
        || terminal.stop_reason != participation::StopReasonV1::EndTurn
        || terminal.output_digest == [0; 32]
        || terminal.provider_evidence_digest == [0; 32]
    {
        return Err(AgentReportAdmissionErrorV1::InvalidParticipation);
    }
    retained_output(outputs, participation, terminal)?;
    let deliberation_id = core::DeliberationIdV1(participation.binding.deliberation.0);
    let retained = store
        .deliberation(deliberation_id)
        .ok_or(AgentReportAdmissionErrorV1::MissingDeliberation)?;
    let member_id = core::EnsembleMemberIdV1(participation.binding.member.0);
    let member = retained
        .plan
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or(AgentReportAdmissionErrorV1::BindingMismatch)?;
    if retained.plan.id != deliberation_id
        || retained.plan.method != method(participation.binding.method)
        || member.lens != persona(participation.binding.persona)
        || member.grounding != grounding(participation.binding.grounding)
        || report.member != member_id
    {
        return Err(AgentReportAdmissionErrorV1::BindingMismatch);
    }
    if report.id.0 == [0; 16]
        || report.output_digest != terminal.output_digest
        || !valid_digests(&report.evidence_digests)
        || !valid_digests(&report.dissent_digests)
        || !report
            .evidence_digests
            .contains(&terminal.provider_evidence_digest)
        || !report
            .evidence_digests
            .contains(&participation.binding.grounding.authority_digest)
        || !report
            .evidence_digests
            .contains(&participation.binding.grounding.surface_digest)
    {
        return Err(AgentReportAdmissionErrorV1::InvalidReport);
    }
    store
        .apply(
            store.revision(),
            RegistryCommandV1::SubmitReport {
                deliberation: deliberation_id,
                report,
            },
        )
        .map_err(AgentReportAdmissionErrorV1::Store)
}

fn retained_output<O: AgentOutputStoreV1, StoreError>(
    outputs: &mut O,
    participation: &participation::ParticipationViewV1,
    terminal: participation::TerminalEvidenceV1,
) -> Result<
    (moira_host::AgentOutputRecordV1, Vec<u8>),
    AgentReportAdmissionErrorV1<StoreError, O::Error>,
> {
    let (record, output) = outputs
        .get(participation.binding.participation.0, terminal.turn.0)
        .map_err(AgentReportAdmissionErrorV1::Output)?
        .ok_or(AgentReportAdmissionErrorV1::MissingOutput)?;
    if record.participation != participation.binding.participation.0
        || record.turn != terminal.turn.0
        || Some(record.last_cursor) != participation.last_event_cursor
        || record.output_digest != terminal.output_digest
        || record.output_bytes != output.len()
        || <[u8; 32]>::from(Sha256::digest(&output)) != record.output_digest
    {
        return Err(AgentReportAdmissionErrorV1::InvalidReport);
    }
    Ok((record, output))
}

fn derived_report_id<StoreError, OutputError>(
    participation: &participation::ParticipationViewV1,
    terminal: participation::TerminalEvidenceV1,
) -> Result<ReportIdV1, AgentReportAdmissionErrorV1<StoreError, OutputError>> {
    let mut hash = Sha256::new();
    hash.update(b"MOIRAARPT1");
    hash.update(participation.binding.participation.0);
    hash.update(participation.binding.workflow.0);
    hash.update(participation.binding.deliberation.0);
    hash.update(participation.binding.member.0);
    hash.update(participation.binding.session.0);
    hash.update(participation.binding.profile.0);
    hash.update(participation.binding.attempt.task_key);
    hash.update(participation.binding.attempt.incarnation.to_be_bytes());
    hash.update(participation.binding.attempt.attempt.to_be_bytes());
    hash.update(participation.binding.method.machine_digest);
    hash.update(participation.binding.method.state_digest);
    hash.update(participation.binding.grounding.authority_digest);
    hash.update(participation.binding.grounding.surface_digest);
    hash.update([depth_tag(participation.binding.grounding.depth)]);
    hash.update([persona_tag(participation.binding.persona)]);
    hash.update(terminal.turn.0);
    hash.update(terminal.output_digest);
    hash.update(terminal.provider_evidence_digest);
    let digest: [u8; 32] = hash.finalize().into();
    let id: [u8; 16] = digest[..16]
        .try_into()
        .map_err(|_| AgentReportAdmissionErrorV1::InvalidReport)?;
    if id == [0; 16] {
        Err(AgentReportAdmissionErrorV1::InvalidReport)
    } else {
        Ok(ReportIdV1(id))
    }
}

const fn depth_tag(value: public::SurfaceDepthV1) -> u8 {
    match value {
        public::SurfaceDepthV1::Ambient => 0,
        public::SurfaceDepthV1::Activated => 1,
        public::SurfaceDepthV1::Reference => 2,
    }
}

const fn persona_tag(value: public::PersonaLensV1) -> u8 {
    match value {
        public::PersonaLensV1::Historian => 0,
        public::PersonaLensV1::IdeaStormer => 1,
        public::PersonaLensV1::Theorist => 2,
        public::PersonaLensV1::Researcher => 3,
        public::PersonaLensV1::RigorReviewer => 4,
        public::PersonaLensV1::ConvergenceFacilitator => 5,
    }
}

fn valid_digests(values: &[[u8; 32]]) -> bool {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .all(|value| *value != [0; 32] && seen.insert(*value))
}

fn method(value: public::MethodRefV1) -> core::MethodRefV1 {
    core::MethodRefV1 {
        machine_digest: value.machine_digest,
        state_digest: value.state_digest,
    }
}

fn grounding(value: public::GroundingRefV1) -> core::GroundingRefV1 {
    core::GroundingRefV1 {
        authority_digest: value.authority_digest,
        surface_digest: value.surface_digest,
        depth: match value.depth {
            public::SurfaceDepthV1::Ambient => core::SurfaceDepthV1::Ambient,
            public::SurfaceDepthV1::Activated => core::SurfaceDepthV1::Activated,
            public::SurfaceDepthV1::Reference => core::SurfaceDepthV1::Reference,
        },
    }
}

fn persona(value: public::PersonaLensV1) -> core::PersonaLensV1 {
    match value {
        public::PersonaLensV1::Historian => core::PersonaLensV1::Historian,
        public::PersonaLensV1::IdeaStormer => core::PersonaLensV1::IdeaStormer,
        public::PersonaLensV1::Theorist => core::PersonaLensV1::Theorist,
        public::PersonaLensV1::Researcher => core::PersonaLensV1::Researcher,
        public::PersonaLensV1::RigorReviewer => core::PersonaLensV1::RigorReviewer,
        public::PersonaLensV1::ConvergenceFacilitator => {
            core::PersonaLensV1::ConvergenceFacilitator
        }
    }
}
