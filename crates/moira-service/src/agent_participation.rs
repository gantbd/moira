//! Authorization-first public service for durable agent participation.

use crate::AccessV1;
use moira_core::{self as core, agent_participation as domain, deliberation as dd};
use moira_host::{
    AgentParticipationApplyErrorV1, AgentParticipationMutationV1, AgentParticipationStore,
};
use moira_protocol::{agent_participation as public, deliberation as pd};

pub struct AgentParticipationService<S> {
    store: S,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct PermissionAccessV1 {
    pub decision: [u8; 16],
    pub subject: [u8; 16],
    pub capability_digest: [u8; 32],
    pub epoch: u64,
}

impl std::fmt::Debug for PermissionAccessV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PermissionAccessV1")
            .field("decision", &"<opaque>")
            .field("subject", &"<redacted>")
            .field("capability_digest", &"<opaque>")
            .field("epoch", &self.epoch)
            .finish()
    }
}

/// Command-scoped authority for one exact permission selection. Implementors
/// retain session credentials and grant handles; only verified attribution
/// facts cross into the durable participation service.
pub trait PermissionAuthorizer {
    type Error;

    fn authorize_permission(
        &mut self,
        selection: public::PermissionSelectionV1,
    ) -> Result<Option<PermissionAccessV1>, Self::Error>;
}

impl<S: AgentParticipationStore> AgentParticipationService<S> {
    pub const fn new(store: S) -> Self {
        Self { store }
    }
    pub fn execute(&mut self, command: public::CommandV1) -> Result<public::ReplyV1, S::Error> {
        self.execute_authorized(
            AccessV1 {
                read: true,
                mutate: true,
            },
            command,
        )
    }
    pub fn execute_authorized(
        &mut self,
        access: AccessV1,
        command: public::CommandV1,
    ) -> Result<public::ReplyV1, S::Error> {
        let allowed = match command {
            public::CommandV1::Apply { ref mutation, .. } => {
                access.mutate && !untrusted_permission_mutation(mutation)
            }
            public::CommandV1::Status
            | public::CommandV1::Participation { .. }
            | public::CommandV1::PermissionConfirmation { .. }
            | public::CommandV1::CancelConfirmation { .. } => access.read,
            public::CommandV1::ConfirmCancel(_) => access.mutate,
            public::CommandV1::SelectPermission(_) | public::CommandV1::ConfirmPermission(_) => {
                false
            }
        };
        if !allowed {
            return Ok(public::ReplyV1::Refused(public::RefusalV1::NotAuthorized));
        }
        match command {
            public::CommandV1::Apply {
                expected_storage_revision,
                mutation,
            } => match self
                .store
                .apply(expected_storage_revision, mutation_domain(*mutation))
            {
                Ok(value) => Ok(public::ReplyV1::Accepted(Box::new(
                    public::OutcomeV1::Applied {
                        storage_revision: value.storage_revision,
                        participation: view(value.participation),
                        replayed: value.replayed,
                    },
                ))),
                Err(AgentParticipationApplyErrorV1::Domain(value)) => {
                    Ok(public::ReplyV1::Refused(refusal(value)))
                }
                Err(AgentParticipationApplyErrorV1::Conflict { expected, actual }) => {
                    Ok(public::ReplyV1::Refused(
                        public::RefusalV1::StorageRevisionConflict { expected, actual },
                    ))
                }
                Err(AgentParticipationApplyErrorV1::Backend(error)) => Err(error),
            },
            public::CommandV1::Status => Ok(public::ReplyV1::Accepted(Box::new(
                public::OutcomeV1::Status(public::StatusV1 {
                    storage_revision: self.store.storage_revision(),
                    needs_reopen: self.store.needs_reopen(),
                }),
            ))),
            public::CommandV1::Participation { participation } => Ok(public::ReplyV1::Accepted(
                Box::new(public::OutcomeV1::Participation(
                    self.store
                        .participation(domain::ParticipationIdV1(participation.0))
                        .map(view),
                )),
            )),
            public::CommandV1::PermissionConfirmation { participation } => {
                let confirmation = self.store.storage_revision().and_then(|storage_revision| {
                    self.store
                        .participation(domain::ParticipationIdV1(participation.0))
                        .map(view)
                        .and_then(|value| public::permission_confirmation(storage_revision, &value))
                });
                Ok(public::ReplyV1::Accepted(Box::new(
                    public::OutcomeV1::PermissionConfirmation(confirmation),
                )))
            }
            public::CommandV1::CancelConfirmation { participation } => {
                let confirmation = self.store.storage_revision().and_then(|storage_revision| {
                    self.store
                        .participation(domain::ParticipationIdV1(participation.0))
                        .map(view)
                        .and_then(|value| public::cancel_confirmation(storage_revision, &value))
                });
                Ok(public::ReplyV1::Accepted(Box::new(
                    public::OutcomeV1::CancelConfirmation(confirmation),
                )))
            }
            public::CommandV1::ConfirmCancel(selected) => self.execute_confirmed_cancel(selected),
            public::CommandV1::SelectPermission(_) | public::CommandV1::ConfirmPermission(_) => {
                Ok(public::ReplyV1::Refused(public::RefusalV1::NotAuthorized))
            }
        }
    }

    fn execute_confirmed_cancel(
        &mut self,
        selected: public::ConfirmedCancelV1,
    ) -> Result<public::ReplyV1, S::Error> {
        if self.store.needs_reopen() {
            return Ok(public::ReplyV1::Refused(
                public::RefusalV1::InvalidCheckpoint,
            ));
        }
        let Some(storage_revision) = self.store.storage_revision() else {
            return Ok(public::ReplyV1::Refused(
                public::RefusalV1::UnknownParticipation,
            ));
        };
        let Some(value) = self
            .store
            .participation(domain::ParticipationIdV1(selected.participation.0))
            .map(view)
        else {
            return Ok(public::ReplyV1::Refused(
                public::RefusalV1::UnknownParticipation,
            ));
        };
        let Some(confirmation) = public::cancel_confirmation(storage_revision, &value) else {
            return Ok(public::ReplyV1::Refused(
                public::RefusalV1::NoPendingOperation,
            ));
        };
        if confirmation.confirmation_digest != selected.confirmation_digest {
            return Ok(public::ReplyV1::Refused(
                public::RefusalV1::RevisionConflict,
            ));
        }
        let Some(operation) = public::confirmed_cancel_operation(confirmation, selected) else {
            return Ok(public::ReplyV1::Refused(
                public::RefusalV1::InvalidOperation,
            ));
        };
        match self.store.apply(
            Some(storage_revision),
            AgentParticipationMutationV1::Prepare {
                participation: domain::ParticipationIdV1(selected.participation.0),
                expected_revision: value.revision,
                operation: operation_domain(operation),
            },
        ) {
            Ok(value) => Ok(public::ReplyV1::Accepted(Box::new(
                public::OutcomeV1::Applied {
                    storage_revision: value.storage_revision,
                    participation: view(value.participation),
                    replayed: value.replayed,
                },
            ))),
            Err(AgentParticipationApplyErrorV1::Domain(value)) => {
                Ok(public::ReplyV1::Refused(refusal(value)))
            }
            Err(AgentParticipationApplyErrorV1::Conflict { expected, actual }) => {
                Ok(public::ReplyV1::Refused(
                    public::RefusalV1::StorageRevisionConflict { expected, actual },
                ))
            }
            Err(AgentParticipationApplyErrorV1::Backend(error)) => Err(error),
        }
    }

    /// Revalidates a terminal confirmation against the current durable record
    /// and derives the exact selection that may be presented to Aegis.
    pub fn prepare_confirmed_permission(
        &self,
        selected: public::ConfirmedPermissionSelectionV1,
    ) -> Result<public::PermissionSelectionV1, public::RefusalV1> {
        if self.store.needs_reopen() {
            return Err(public::RefusalV1::InvalidCheckpoint);
        }
        let storage_revision = self
            .store
            .storage_revision()
            .ok_or(public::RefusalV1::UnknownParticipation)?;
        let value = self
            .store
            .participation(domain::ParticipationIdV1(selected.participation.0))
            .map(view)
            .ok_or(public::RefusalV1::UnknownParticipation)?;
        let confirmation = public::permission_confirmation(storage_revision, &value)
            .ok_or(public::RefusalV1::NoPendingOperation)?;
        if confirmation.confirmation_digest != selected.confirmation_digest {
            return Err(public::RefusalV1::RevisionConflict);
        }
        public::confirmed_permission_selection(confirmation, selected, value.binding.attempt)
            .ok_or(public::RefusalV1::InvalidOperation)
    }

    pub fn execute_permission_authorized(
        &mut self,
        access: PermissionAccessV1,
        command: public::CommandV1,
    ) -> Result<public::ReplyV1, S::Error> {
        let public::CommandV1::SelectPermission(selection) = command else {
            return Ok(public::ReplyV1::Refused(public::RefusalV1::NotAuthorized));
        };
        if access.capability_digest != selection.capability_digest {
            return Ok(public::ReplyV1::Refused(public::RefusalV1::NotAuthorized));
        }
        let mutation = AgentParticipationMutationV1::Prepare {
            participation: domain::ParticipationIdV1(selection.participation.0),
            expected_revision: selection.expected_revision,
            operation: domain::OperationV1::DecidePermission {
                key: domain::IdempotencyKeyV1(selection.key.0),
                execution: domain::ExecutionIdV1(selection.execution.0),
                request: domain::PermissionRequestIdV1(selection.request.0),
                option: domain::PermissionOptionIdV1(selection.option.0),
                authority: domain::PermissionAuthorityV1 {
                    decision: domain::AuthorityDecisionIdV1(access.decision),
                    subject: domain::AuthoritySubjectIdV1(access.subject),
                    capability_digest: selection.capability_digest,
                    epoch: access.epoch,
                },
                attempt: attempt_domain(selection.attempt),
            },
        };
        match self
            .store
            .apply(selection.expected_storage_revision, mutation)
        {
            Ok(value) => Ok(public::ReplyV1::Accepted(Box::new(
                public::OutcomeV1::Applied {
                    storage_revision: value.storage_revision,
                    participation: view(value.participation),
                    replayed: value.replayed,
                },
            ))),
            Err(AgentParticipationApplyErrorV1::Domain(value)) => {
                Ok(public::ReplyV1::Refused(refusal(value)))
            }
            Err(AgentParticipationApplyErrorV1::Conflict { expected, actual }) => {
                Ok(public::ReplyV1::Refused(
                    public::RefusalV1::StorageRevisionConflict { expected, actual },
                ))
            }
            Err(AgentParticipationApplyErrorV1::Backend(error)) => Err(error),
        }
    }
    pub fn into_store(self) -> S {
        self.store
    }
}

fn untrusted_permission_mutation(value: &public::MutationV1) -> bool {
    matches!(
        value,
        public::MutationV1::Prepare {
            operation: public::OperationV1::DecidePermission { .. },
            ..
        }
    )
}

fn mutation_domain(value: public::MutationV1) -> AgentParticipationMutationV1 {
    match value {
        public::MutationV1::Admit(value) => {
            AgentParticipationMutationV1::Admit(binding_domain(value))
        }
        public::MutationV1::Prepare {
            participation,
            expected_revision,
            operation,
        } => AgentParticipationMutationV1::Prepare {
            participation: domain::ParticipationIdV1(participation.0),
            expected_revision,
            operation: operation_domain(operation),
        },
        public::MutationV1::Observe {
            participation,
            expected_revision,
            key,
            observation,
        } => AgentParticipationMutationV1::Observe {
            participation: domain::ParticipationIdV1(participation.0),
            expected_revision,
            key: domain::IdempotencyKeyV1(key.0),
            observation: observation_domain(observation),
        },
        public::MutationV1::FenceUnknown {
            participation,
            expected_revision,
            key,
        } => AgentParticipationMutationV1::FenceUnknown {
            participation: domain::ParticipationIdV1(participation.0),
            expected_revision,
            key: domain::IdempotencyKeyV1(key.0),
        },
    }
}
fn binding_domain(v: public::BindingV1) -> domain::BindingV1 {
    domain::BindingV1 {
        participation: domain::ParticipationIdV1(v.participation.0),
        workflow: core::WorkflowId::new(v.workflow.0),
        deliberation: dd::DeliberationIdV1(v.deliberation.0),
        member: dd::EnsembleMemberIdV1(v.member.0),
        session: domain::AgentSessionIdV1(v.session.0),
        profile: domain::ProfileIdV1(v.profile.0),
        attempt: attempt_domain(v.attempt),
        method: dd::MethodRefV1 {
            machine_digest: v.method.machine_digest,
            state_digest: v.method.state_digest,
        },
        grounding: dd::GroundingRefV1 {
            authority_digest: v.grounding.authority_digest,
            surface_digest: v.grounding.surface_digest,
            depth: depth_domain(v.grounding.depth),
        },
        persona: persona_domain(v.persona),
    }
}
fn attempt_domain(v: public::AttemptV1) -> domain::AttemptV1 {
    domain::AttemptV1 {
        task_key: v.task_key,
        incarnation: v.incarnation,
        attempt: v.attempt,
    }
}
fn operation_domain(v: public::OperationV1) -> domain::OperationV1 {
    match v {
        public::OperationV1::Open { key, execution } => domain::OperationV1::Open {
            key: domain::IdempotencyKeyV1(key.0),
            execution: domain::ExecutionIdV1(execution.0),
        },
        public::OperationV1::Prompt {
            key,
            turn,
            execution,
            prompt_digest,
            attempt,
        } => domain::OperationV1::Prompt {
            key: domain::IdempotencyKeyV1(key.0),
            turn: domain::AgentTurnIdV1(turn.0),
            execution: domain::ExecutionIdV1(execution.0),
            prompt_digest,
            attempt: attempt_domain(attempt),
        },
        public::OperationV1::Advance { key, turn } => domain::OperationV1::Advance {
            key: domain::IdempotencyKeyV1(key.0),
            turn: domain::AgentTurnIdV1(turn.0),
        },
        public::OperationV1::DecidePermission {
            key,
            execution,
            request,
            option,
            authority,
            attempt,
        } => domain::OperationV1::DecidePermission {
            key: domain::IdempotencyKeyV1(key.0),
            execution: domain::ExecutionIdV1(execution.0),
            request: domain::PermissionRequestIdV1(request.0),
            option: domain::PermissionOptionIdV1(option.0),
            authority: domain::PermissionAuthorityV1 {
                decision: domain::AuthorityDecisionIdV1(authority.decision.0),
                subject: domain::AuthoritySubjectIdV1(authority.subject.0),
                capability_digest: authority.capability_digest,
                epoch: authority.epoch,
            },
            attempt: attempt_domain(attempt),
        },
        public::OperationV1::Cancel { key, turn } => domain::OperationV1::Cancel {
            key: domain::IdempotencyKeyV1(key.0),
            turn: domain::AgentTurnIdV1(turn.0),
        },
        public::OperationV1::Close { key } => domain::OperationV1::Close {
            key: domain::IdempotencyKeyV1(key.0),
        },
    }
}
fn observation_domain(v: public::ObservationV1) -> domain::ObservationV1 {
    domain::ObservationV1 {
        phase: phase_domain(v.phase),
        session_revision: v.session_revision,
        active_turn: v.active_turn.map(|v| domain::AgentTurnIdV1(v.0)),
        last_event_cursor: v.last_event_cursor,
        profile_document_digest: v.profile_document_digest,
        capabilities: capabilities_domain(v.capabilities),
        permission: v.permission.map(permission_domain),
        terminal: v.terminal.map(terminal_domain),
    }
}
fn capabilities_domain(v: public::CapabilitiesV1) -> domain::CapabilitiesV1 {
    domain::CapabilitiesV1 {
        load_session: v.load_session,
        resume_session: v.resume_session,
        permissions: v.permissions,
        filesystem_read: v.filesystem_read,
        filesystem_write: v.filesystem_write,
        terminal: v.terminal,
        session_modes: v.session_modes,
        session_models: v.session_models,
    }
}
fn permission_domain(v: public::PermissionEvidenceV1) -> domain::PermissionEvidenceV1 {
    domain::PermissionEvidenceV1 {
        request: domain::PermissionRequestIdV1(v.request.0),
        turn: domain::AgentTurnIdV1(v.turn.0),
        capability_digest: v.capability_digest,
        options_digest: v.options_digest,
        option_count: v.option_count,
        option_ids: v
            .option_ids
            .map(|value| domain::PermissionOptionIdV1(value.0)),
    }
}
fn terminal_domain(v: public::TerminalEvidenceV1) -> domain::TerminalEvidenceV1 {
    domain::TerminalEvidenceV1 {
        turn: domain::AgentTurnIdV1(v.turn.0),
        disposition: disposition_domain(v.disposition),
        stop_reason: stop_domain(v.stop_reason),
        output_digest: v.output_digest,
        provider_evidence_digest: v.provider_evidence_digest,
    }
}

fn view(v: domain::SnapshotV1) -> public::ParticipationViewV1 {
    public::ParticipationViewV1 {
        binding: binding_view(v.binding),
        revision: v.revision,
        phase: phase_view(v.phase),
        session_revision: v.session_revision,
        last_event_cursor: v.last_event_cursor,
        profile_document_digest: v.profile_document_digest,
        capabilities: capabilities_view(v.capabilities),
        active_turn: v.active_turn.map(|v| public::AgentTurnIdV1(v.0)),
        permission: v.permission.map(permission_view),
        terminal: v.terminal.map(terminal_view),
        pending: v.pending.map(operation_view),
    }
}
fn binding_view(v: domain::BindingV1) -> public::BindingV1 {
    public::BindingV1 {
        participation: public::ParticipationIdV1(v.participation.0),
        workflow: moira_protocol::WorkflowIdV1(v.workflow.into_bytes()),
        deliberation: pd::DeliberationIdV1(v.deliberation.0),
        member: pd::EnsembleMemberIdV1(v.member.0),
        session: public::AgentSessionIdV1(v.session.0),
        profile: public::ProfileIdV1(v.profile.0),
        attempt: attempt_view(v.attempt),
        method: pd::MethodRefV1 {
            machine_digest: v.method.machine_digest,
            state_digest: v.method.state_digest,
        },
        grounding: pd::GroundingRefV1 {
            authority_digest: v.grounding.authority_digest,
            surface_digest: v.grounding.surface_digest,
            depth: depth_view(v.grounding.depth),
        },
        persona: persona_view(v.persona),
    }
}
fn attempt_view(v: domain::AttemptV1) -> public::AttemptV1 {
    public::AttemptV1 {
        task_key: v.task_key,
        incarnation: v.incarnation,
        attempt: v.attempt,
    }
}
fn operation_view(v: domain::OperationV1) -> public::OperationV1 {
    match v {
        domain::OperationV1::Open { key, execution } => public::OperationV1::Open {
            key: public::IdempotencyKeyV1(key.0),
            execution: public::ExecutionIdV1(execution.0),
        },
        domain::OperationV1::Prompt {
            key,
            turn,
            execution,
            prompt_digest,
            attempt,
        } => public::OperationV1::Prompt {
            key: public::IdempotencyKeyV1(key.0),
            turn: public::AgentTurnIdV1(turn.0),
            execution: public::ExecutionIdV1(execution.0),
            prompt_digest,
            attempt: attempt_view(attempt),
        },
        domain::OperationV1::Advance { key, turn } => public::OperationV1::Advance {
            key: public::IdempotencyKeyV1(key.0),
            turn: public::AgentTurnIdV1(turn.0),
        },
        domain::OperationV1::DecidePermission {
            key,
            execution,
            request,
            option,
            authority,
            attempt,
        } => public::OperationV1::DecidePermission {
            key: public::IdempotencyKeyV1(key.0),
            execution: public::ExecutionIdV1(execution.0),
            request: public::PermissionRequestIdV1(request.0),
            option: public::PermissionOptionIdV1(option.0),
            authority: public::PermissionAuthorityV1 {
                decision: public::AuthorityDecisionIdV1(authority.decision.0),
                subject: public::AuthoritySubjectIdV1(authority.subject.0),
                capability_digest: authority.capability_digest,
                epoch: authority.epoch,
            },
            attempt: attempt_view(attempt),
        },
        domain::OperationV1::Cancel { key, turn } => public::OperationV1::Cancel {
            key: public::IdempotencyKeyV1(key.0),
            turn: public::AgentTurnIdV1(turn.0),
        },
        domain::OperationV1::Close { key } => public::OperationV1::Close {
            key: public::IdempotencyKeyV1(key.0),
        },
    }
}
fn capabilities_view(v: domain::CapabilitiesV1) -> public::CapabilitiesV1 {
    public::CapabilitiesV1 {
        load_session: v.load_session,
        resume_session: v.resume_session,
        permissions: v.permissions,
        filesystem_read: v.filesystem_read,
        filesystem_write: v.filesystem_write,
        terminal: v.terminal,
        session_modes: v.session_modes,
        session_models: v.session_models,
    }
}
fn permission_view(v: domain::PermissionEvidenceV1) -> public::PermissionEvidenceV1 {
    public::PermissionEvidenceV1 {
        request: public::PermissionRequestIdV1(v.request.0),
        turn: public::AgentTurnIdV1(v.turn.0),
        capability_digest: v.capability_digest,
        options_digest: v.options_digest,
        option_count: v.option_count,
        option_ids: v
            .option_ids
            .map(|value| public::PermissionOptionIdV1(value.0)),
    }
}
fn terminal_view(v: domain::TerminalEvidenceV1) -> public::TerminalEvidenceV1 {
    public::TerminalEvidenceV1 {
        turn: public::AgentTurnIdV1(v.turn.0),
        disposition: disposition_view(v.disposition),
        stop_reason: stop_view(v.stop_reason),
        output_digest: v.output_digest,
        provider_evidence_digest: v.provider_evidence_digest,
    }
}

macro_rules! map_enum{($a:ident,$b:ident,$from:ty,$to:ty,{$($left:path=>$right:path),+$(,)?})=>{fn $a(v:$from)->$to{match v{$($left=>$right),+}}fn $b(v:$to)->$from{match v{$($right=>$left),+}}};}
map_enum!(phase_domain,phase_view,public::PhaseV1,domain::PhaseV1,{public::PhaseV1::Admitted=>domain::PhaseV1::Admitted,public::PhaseV1::Ready=>domain::PhaseV1::Ready,public::PhaseV1::TurnActive=>domain::PhaseV1::TurnActive,public::PhaseV1::AwaitingPermission=>domain::PhaseV1::AwaitingPermission,public::PhaseV1::Terminal=>domain::PhaseV1::Terminal,public::PhaseV1::FencedUnknown=>domain::PhaseV1::FencedUnknown,public::PhaseV1::Closed=>domain::PhaseV1::Closed});
map_enum!(disposition_domain,disposition_view,public::DispositionV1,domain::DispositionV1,{public::DispositionV1::Completed=>domain::DispositionV1::Completed,public::DispositionV1::Failed=>domain::DispositionV1::Failed,public::DispositionV1::Cancelled=>domain::DispositionV1::Cancelled,public::DispositionV1::TimedOut=>domain::DispositionV1::TimedOut,public::DispositionV1::Uncertain=>domain::DispositionV1::Uncertain});
map_enum!(stop_domain,stop_view,public::StopReasonV1,domain::StopReasonV1,{public::StopReasonV1::EndTurn=>domain::StopReasonV1::EndTurn,public::StopReasonV1::Cancelled=>domain::StopReasonV1::Cancelled,public::StopReasonV1::TimedOut=>domain::StopReasonV1::TimedOut,public::StopReasonV1::Failed=>domain::StopReasonV1::Failed,public::StopReasonV1::Disconnected=>domain::StopReasonV1::Disconnected,public::StopReasonV1::OutcomeUnknown=>domain::StopReasonV1::OutcomeUnknown});
map_enum!(depth_domain,depth_view,pd::SurfaceDepthV1,dd::SurfaceDepthV1,{pd::SurfaceDepthV1::Ambient=>dd::SurfaceDepthV1::Ambient,pd::SurfaceDepthV1::Activated=>dd::SurfaceDepthV1::Activated,pd::SurfaceDepthV1::Reference=>dd::SurfaceDepthV1::Reference});
map_enum!(persona_domain,persona_view,pd::PersonaLensV1,dd::PersonaLensV1,{pd::PersonaLensV1::Historian=>dd::PersonaLensV1::Historian,pd::PersonaLensV1::IdeaStormer=>dd::PersonaLensV1::IdeaStormer,pd::PersonaLensV1::Theorist=>dd::PersonaLensV1::Theorist,pd::PersonaLensV1::Researcher=>dd::PersonaLensV1::Researcher,pd::PersonaLensV1::RigorReviewer=>dd::PersonaLensV1::RigorReviewer,pd::PersonaLensV1::ConvergenceFacilitator=>dd::PersonaLensV1::ConvergenceFacilitator});

fn refusal(v: domain::RefusalV1) -> public::RefusalV1 {
    match v {
        domain::RefusalV1::InvalidLimits => public::RefusalV1::InvalidLimits,
        domain::RefusalV1::Limit => public::RefusalV1::Limit,
        domain::RefusalV1::InvalidBinding => public::RefusalV1::InvalidBinding,
        domain::RefusalV1::DuplicateParticipation => public::RefusalV1::DuplicateParticipation,
        domain::RefusalV1::UnknownParticipation => public::RefusalV1::UnknownParticipation,
        domain::RefusalV1::RevisionConflict => public::RefusalV1::RevisionConflict,
        domain::RefusalV1::RevisionExhausted => public::RefusalV1::RevisionExhausted,
        domain::RefusalV1::InvalidOperation => public::RefusalV1::InvalidOperation,
        domain::RefusalV1::OperationConflict => public::RefusalV1::OperationConflict,
        domain::RefusalV1::NoPendingOperation => public::RefusalV1::NoPendingOperation,
        domain::RefusalV1::InvalidObservation => public::RefusalV1::InvalidObservation,
        domain::RefusalV1::ObservationConflict => public::RefusalV1::ObservationConflict,
        domain::RefusalV1::InvalidCheckpoint => public::RefusalV1::InvalidCheckpoint,
    }
}
