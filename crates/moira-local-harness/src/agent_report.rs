//! Admission gate from one terminal agent participation into a methodical
//! deliberation report. Provider completion is necessary but never sufficient:
//! the retained Rune method, Dendrite grounding, persona, member, output and
//! provider evidence must all match before the durable registry is touched.

use moira_core::deliberation::{
    self as core, MemberReportV1,
    registry::{RegistryCommandV1, RegistryReceiptV1},
};
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};
use moira_protocol::{agent_participation as participation, deliberation as public};
use std::collections::BTreeSet;

#[derive(Debug, Eq, PartialEq)]
pub enum AgentReportAdmissionErrorV1<StoreError> {
    NeedsReopen,
    MissingDeliberation,
    InvalidParticipation,
    BindingMismatch,
    InvalidReport,
    Store(DeliberationApplyErrorV1<StoreError>),
}

pub fn admit_terminal_report<S: DeliberationStore>(
    store: &mut S,
    participation: &participation::ParticipationViewV1,
    report: MemberReportV1,
) -> Result<RegistryReceiptV1, AgentReportAdmissionErrorV1<S::Error>> {
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
