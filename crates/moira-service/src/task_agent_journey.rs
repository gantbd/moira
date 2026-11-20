//! Authorization-first service for task-to-agent journey admission.

use crate::AccessV1;
use moira_core::{self as core, task_agent_journey as domain};
use moira_host::{
    TaskAgentJourneyApplyErrorV1, TaskAgentJourneyApplyV1, TaskAgentJourneyMutationV1,
    TaskAgentJourneyStore,
};
use moira_protocol::{TaskRefV1, task_agent_journey as public};

/// Trusted service time. Client-supplied DTOs intentionally contain no owner
/// observation, expiry or confirmation timestamp.
pub trait TrustedTimeV1 {
    type Error;
    fn now(&mut self) -> Result<u64, Self::Error>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum ErrorV1<S, T> {
    Store(S),
    Time(T),
}

pub struct TaskAgentJourneyService<S, T> {
    store: S,
    time: T,
}

impl<S: TaskAgentJourneyStore, T: TrustedTimeV1> TaskAgentJourneyService<S, T> {
    pub const fn new(store: S, time: T) -> Self {
        Self { store, time }
    }

    pub fn execute(
        &mut self,
        command: public::CommandV1,
    ) -> Result<public::ReplyV1, ErrorV1<S::Error, T::Error>> {
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
    ) -> Result<public::ReplyV1, ErrorV1<S::Error, T::Error>> {
        let allowed = match command {
            public::CommandV1::Preview { .. } | public::CommandV1::Instantiate { .. } => {
                access.mutate
            }
            public::CommandV1::Catalogue
            | public::CommandV1::PreviewView { .. }
            | public::CommandV1::Journey { .. }
            | public::CommandV1::Participation { .. }
            | public::CommandV1::TaskAttemptNamespace { .. }
            | public::CommandV1::ActorParticipationRelationship { .. }
            | public::CommandV1::Status => access.read,
        };
        if !allowed {
            return Ok(public::ReplyV1::Refused(public::RefusalV1::NotAuthorized));
        }
        if self.store.needs_reopen() && !matches!(command, public::CommandV1::Status) {
            return Ok(public::ReplyV1::Refused(
                public::RefusalV1::InvalidCheckpoint,
            ));
        }
        match command {
            public::CommandV1::Catalogue => Ok(public::ReplyV1::Accepted(Box::new(
                public::OutcomeV1::Catalogue {
                    owner: owner_view(self.store.owner()),
                    owner_revision: self.store.owner_revision(),
                    storage_revision: self.store.storage_revision(),
                    templates: self
                        .store
                        .catalogue()
                        .into_iter()
                        .map(template_view)
                        .collect(),
                },
            ))),
            public::CommandV1::Preview {
                expected_storage_revision,
                request,
            } => {
                let observed_at = self.time.now().map_err(ErrorV1::Time)?;
                self.apply(
                    expected_storage_revision,
                    TaskAgentJourneyMutationV1::Preview {
                        request: preview_request(request),
                        observed_at,
                    },
                    true,
                )
            }
            public::CommandV1::Instantiate {
                expected_storage_revision,
                request,
            } => {
                let observed_at = self.time.now().map_err(ErrorV1::Time)?;
                self.apply(
                    expected_storage_revision,
                    TaskAgentJourneyMutationV1::Instantiate {
                        request: instantiate_request(request),
                        observed_at,
                    },
                    false,
                )
            }
            public::CommandV1::PreviewView { preview } => Ok(public::ReplyV1::Accepted(Box::new(
                public::OutcomeV1::Preview(
                    self.store.preview(preview_id(preview)).map(preview_view),
                ),
            ))),
            public::CommandV1::Journey { journey } => Ok(public::ReplyV1::Accepted(Box::new(
                public::OutcomeV1::Journey(
                    self.store.journey(journey_id(journey)).map(journey_view),
                ),
            ))),
            public::CommandV1::Participation { journey } => Ok(public::ReplyV1::Accepted(
                Box::new(public::OutcomeV1::Participation(
                    self.store
                        .journey(journey_id(journey))
                        .and_then(|snapshot| snapshot.participation)
                        .map(participation_view),
                )),
            )),
            public::CommandV1::TaskAttemptNamespace { journey } => Ok(public::ReplyV1::Accepted(
                Box::new(public::OutcomeV1::TaskAttemptNamespace(
                    self.store
                        .task_attempt_namespace(journey_id(journey))
                        .map(task_attempt_namespace_view),
                )),
            )),
            public::CommandV1::ActorParticipationRelationship { journey } => {
                Ok(public::ReplyV1::Accepted(Box::new(
                    public::OutcomeV1::ActorParticipationRelationship(
                        self.store
                            .actor_participation_relationship(journey_id(journey))
                            .map(actor_participation_relationship_view),
                    ),
                )))
            }
            public::CommandV1::Status => {
                let census = self.store.census();
                Ok(public::ReplyV1::Accepted(Box::new(
                    public::OutcomeV1::Status(public::StatusV1 {
                        owner: owner_view(self.store.owner()),
                        owner_revision: self.store.owner_revision(),
                        storage_revision: self.store.storage_revision(),
                        template_count: census.templates,
                        preview_count: census.previews,
                        journey_count: census.journeys,
                        needs_reopen: self.store.needs_reopen(),
                    }),
                )))
            }
        }
    }

    fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        mutation: TaskAgentJourneyMutationV1,
        preview_expected: bool,
    ) -> Result<public::ReplyV1, ErrorV1<S::Error, T::Error>> {
        match self.store.apply(expected_storage_revision, mutation) {
            Ok(value) => match (preview_expected, value.outcome) {
                (true, TaskAgentJourneyApplyV1::Preview(receipt)) => Ok(public::ReplyV1::Accepted(
                    Box::new(public::OutcomeV1::Previewed {
                        storage_revision: value.storage_revision,
                        receipt: public::PreviewReceiptV1 {
                            preview: preview_view(receipt.preview),
                            replayed: receipt.replayed,
                        },
                    }),
                )),
                (false, TaskAgentJourneyApplyV1::Instantiate(receipt)) => Ok(
                    public::ReplyV1::Accepted(Box::new(public::OutcomeV1::Admitted {
                        storage_revision: value.storage_revision,
                        receipt: public::JourneyReceiptV1 {
                            journey: journey_view(receipt.journey),
                            replayed: receipt.replayed,
                        },
                    })),
                ),
                (_, TaskAgentJourneyApplyV1::TaskAttempt(_)) => Ok(public::ReplyV1::Refused(
                    public::RefusalV1::InvalidCheckpoint,
                )),
                (_, TaskAgentJourneyApplyV1::Room(_)) => Ok(public::ReplyV1::Refused(
                    public::RefusalV1::InvalidCheckpoint,
                )),
                _ => Ok(public::ReplyV1::Refused(
                    public::RefusalV1::InvalidCheckpoint,
                )),
            },
            Err(TaskAgentJourneyApplyErrorV1::Domain(value)) => {
                Ok(public::ReplyV1::Refused(refusal(value)))
            }
            Err(TaskAgentJourneyApplyErrorV1::Conflict { .. }) => Ok(public::ReplyV1::Refused(
                public::RefusalV1::StorageRevisionConflict,
            )),
            Err(TaskAgentJourneyApplyErrorV1::Backend(error)) => Err(ErrorV1::Store(error)),
        }
    }

    pub fn into_parts(self) -> (S, T) {
        (self.store, self.time)
    }
}

fn owner_view(value: domain::OwnerIdV1) -> public::OwnerIdV1 {
    public::OwnerIdV1(value.0)
}

fn owner(value: public::OwnerIdV1) -> domain::OwnerIdV1 {
    domain::OwnerIdV1(value.0)
}

fn preview_id(value: public::PreviewIdV1) -> domain::PreviewIdV1 {
    domain::PreviewIdV1 {
        owner: owner(value.owner),
        sequence: value.sequence,
    }
}

fn preview_id_view(value: domain::PreviewIdV1) -> public::PreviewIdV1 {
    public::PreviewIdV1 {
        owner: owner_view(value.owner),
        sequence: value.sequence,
    }
}

fn journey_id(value: public::JourneyIdV1) -> domain::JourneyIdV1 {
    domain::JourneyIdV1 {
        owner: owner(value.owner),
        sequence: value.sequence,
    }
}

fn journey_id_view(value: domain::JourneyIdV1) -> public::JourneyIdV1 {
    public::JourneyIdV1 {
        owner: owner_view(value.owner),
        sequence: value.sequence,
    }
}

fn work_mode_view(value: domain::WorkModeV1) -> public::WorkModeV1 {
    match value {
        domain::WorkModeV1::Collaborator => public::WorkModeV1::Collaborator,
        domain::WorkModeV1::ProductManager => public::WorkModeV1::ProductManager,
        domain::WorkModeV1::ProjectManager => public::WorkModeV1::ProjectManager,
        domain::WorkModeV1::SiteReliability => public::WorkModeV1::SiteReliability,
    }
}

fn room_policy(value: public::RoomPolicyV1) -> domain::RoomPolicyV1 {
    match value {
        public::RoomPolicyV1::Direct => domain::RoomPolicyV1::Direct,
        public::RoomPolicyV1::SharedContext => domain::RoomPolicyV1::SharedContext,
    }
}

fn room_policy_view(value: domain::RoomPolicyV1) -> public::RoomPolicyV1 {
    match value {
        domain::RoomPolicyV1::Direct => public::RoomPolicyV1::Direct,
        domain::RoomPolicyV1::SharedContext => public::RoomPolicyV1::SharedContext,
    }
}

fn budget_view(value: domain::BudgetV1) -> public::BudgetV1 {
    public::BudgetV1 {
        max_context_bytes: value.max_context_bytes,
        max_output_tokens: value.max_output_tokens,
        max_elapsed_millis: value.max_elapsed_millis,
    }
}

fn template_view(value: domain::TemplateV1) -> public::TemplateV1 {
    public::TemplateV1 {
        id: public::TemplateIdV1(value.id.0),
        version: value.version,
        digest: value.digest,
        work_mode: work_mode_view(value.work_mode),
        profile: moira_protocol::agent_participation::ProfileIdV1(value.profile.0),
        room_policy: room_policy_view(value.room_policy),
        budget: budget_view(value.budget),
        grounding_policy_digest: value.grounding_policy_digest,
        authority_policy_digest: value.authority_policy_digest,
        consequence_digest: value.consequence_digest,
        retired: value.retired,
    }
}

fn template_ref(value: public::TemplateRefV1) -> domain::TemplateRefV1 {
    domain::TemplateRefV1 {
        id: domain::TemplateIdV1(value.id.0),
        version: value.version,
        digest: value.digest,
    }
}

fn task(value: TaskRefV1) -> core::ChronosTaskRefV1 {
    core::ChronosTaskRefV1::new(value.external_task_key, value.incarnation)
}

fn task_view(value: core::ChronosTaskRefV1) -> TaskRefV1 {
    TaskRefV1 {
        external_task_key: value.external_task_key(),
        incarnation: value.incarnation(),
    }
}

fn preview_request(value: public::PreviewRequestV1) -> domain::PreviewRequestV1 {
    domain::PreviewRequestV1 {
        operation: domain::PreviewOperationIdV1(value.operation.0),
        template: template_ref(value.template),
        task: domain::TaskAdmissionV1 {
            task: task(value.task.task),
            chronos_revision: value.task.chronos_revision,
            evidence_digest: value.task.evidence_digest,
        },
        profile: core::agent_participation::ProfileIdV1(value.profile.0),
        room_policy: room_policy(value.room_policy),
        initiating_principal: domain::PrincipalIdV1(value.initiating_principal.0),
        accountable_human: domain::PrincipalIdV1(value.accountable_human.0),
        objective_digest: value.objective_digest,
        acceptance_digest: value.acceptance_digest,
        current_evidence_digest: value.current_evidence_digest,
        requested_ttl: value.requested_ttl,
    }
}

fn preview_request_view(value: domain::PreviewRequestV1) -> public::PreviewRequestV1 {
    public::PreviewRequestV1 {
        operation: public::PreviewOperationIdV1(value.operation.0),
        template: public::TemplateRefV1 {
            id: public::TemplateIdV1(value.template.id.0),
            version: value.template.version,
            digest: value.template.digest,
        },
        task: public::TaskAdmissionV1 {
            task: task_view(value.task.task),
            chronos_revision: value.task.chronos_revision,
            evidence_digest: value.task.evidence_digest,
        },
        profile: moira_protocol::agent_participation::ProfileIdV1(value.profile.0),
        room_policy: room_policy_view(value.room_policy),
        initiating_principal: public::PrincipalIdV1(value.initiating_principal.0),
        accountable_human: public::PrincipalIdV1(value.accountable_human.0),
        objective_digest: value.objective_digest,
        acceptance_digest: value.acceptance_digest,
        current_evidence_digest: value.current_evidence_digest,
        requested_ttl: value.requested_ttl,
    }
}

fn preview_view(value: domain::PreviewSnapshotV1) -> public::PreviewViewV1 {
    public::PreviewViewV1 {
        id: preview_id_view(value.id),
        owner_revision: value.owner_revision,
        request: preview_request_view(value.request),
        observed_at: value.observed_at,
        expires_at: value.expires_at,
        work_mode: work_mode_view(value.work_mode),
        budget: budget_view(value.budget),
        grounding_policy_digest: value.grounding_policy_digest,
        authority_policy_digest: value.authority_policy_digest,
        consequence_digest: value.consequence_digest,
    }
}

fn instantiate_request(value: public::InstantiateRequestV1) -> domain::InstantiateRequestV1 {
    domain::InstantiateRequestV1 {
        operation: domain::InstantiateOperationIdV1(value.operation.0),
        preview: preview_id(value.preview),
        expected_owner_revision: value.expected_owner_revision,
        confirmation: domain::HumanConfirmationV1 {
            preview: preview_id(value.confirmation.preview),
            accountable_human: domain::PrincipalIdV1(value.confirmation.accountable_human.0),
            consequence_digest: value.confirmation.consequence_digest,
        },
    }
}

fn instantiate_request_view(value: domain::InstantiateRequestV1) -> public::InstantiateRequestV1 {
    public::InstantiateRequestV1 {
        operation: public::InstantiateOperationIdV1(value.operation.0),
        preview: preview_id_view(value.preview),
        expected_owner_revision: value.expected_owner_revision,
        confirmation: public::HumanConfirmationV1 {
            preview: preview_id_view(value.confirmation.preview),
            accountable_human: public::PrincipalIdV1(value.confirmation.accountable_human.0),
            consequence_digest: value.confirmation.consequence_digest,
        },
    }
}

fn journey_view(value: domain::JourneySnapshotV1) -> public::JourneyViewV1 {
    public::JourneyViewV1 {
        id: journey_id_view(value.id),
        workflow: public::WorkflowRefV1 {
            owner: owner_view(value.workflow.owner),
            sequence: value.workflow.sequence,
        },
        revision: value.revision,
        phase: match value.phase {
            domain::PhaseV1::Admitted => public::PhaseV1::Admitted,
            domain::PhaseV1::TaskAttemptReady => public::PhaseV1::TaskAttemptReady,
            domain::PhaseV1::RoomReady => public::PhaseV1::RoomReady,
            domain::PhaseV1::ParticipationAdmitted => public::PhaseV1::ParticipationAdmitted,
        },
        admission: instantiate_request_view(value.admission),
        confirmed_at: value.confirmed_at,
        preview: preview_view(value.preview),
        task_attempt: value
            .task_attempt
            .map(|binding| public::TaskAttemptBindingV1 {
                journey: journey_id_view(binding.journey),
                expected_journey_revision: binding.expected_journey_revision,
                task: task_view(binding.task),
                attempt_number: binding.attempt_number,
                chronos_revision: binding.chronos_revision,
            }),
        room: value.room.map(|binding| public::RoomBindingV1 {
            journey: journey_id_view(binding.journey),
            expected_journey_revision: binding.expected_journey_revision,
            source: binding.source,
            room_sequence: binding.room_sequence,
            agora_revision: binding.agora_revision,
        }),
    }
}

fn participation_view(
    binding: domain::ParticipationAdmissionBindingV1,
) -> public::ParticipationAdmissionViewV1 {
    public::ParticipationAdmissionViewV1 {
        journey: journey_id_view(binding.journey),
        expected_journey_revision: binding.expected_journey_revision,
        participation: binding.participation.0,
        owner_revision: binding.owner_revision,
        owner_phase: match binding.owner_phase {
            core::agent_participation::PhaseV1::Admitted => {
                moira_protocol::agent_participation::PhaseV1::Admitted
            }
            core::agent_participation::PhaseV1::Ready => {
                moira_protocol::agent_participation::PhaseV1::Ready
            }
            core::agent_participation::PhaseV1::TurnActive => {
                moira_protocol::agent_participation::PhaseV1::TurnActive
            }
            core::agent_participation::PhaseV1::AwaitingPermission => {
                moira_protocol::agent_participation::PhaseV1::AwaitingPermission
            }
            core::agent_participation::PhaseV1::Terminal => {
                moira_protocol::agent_participation::PhaseV1::Terminal
            }
            core::agent_participation::PhaseV1::FencedUnknown => {
                moira_protocol::agent_participation::PhaseV1::FencedUnknown
            }
            core::agent_participation::PhaseV1::Closed => {
                moira_protocol::agent_participation::PhaseV1::Closed
            }
            core::agent_participation::PhaseV1::ReattachRequired => {
                moira_protocol::agent_participation::PhaseV1::ReattachRequired
            }
        },
        authority_decision: binding.authority.decision,
        authority_principal: binding.authority.principal,
        authority_epoch: binding.authority.epoch,
        authority_expires_at: binding.authority.expires_at,
    }
}

fn task_attempt_namespace_view(
    binding: domain::TaskAttemptNamespaceBindingV1,
) -> public::TaskAttemptNamespaceViewV1 {
    public::TaskAttemptNamespaceViewV1 {
        journey: journey_id_view(binding.journey),
        expected_journey_revision: binding.expected_journey_revision,
        task: task_view(binding.task),
        attempt_number: binding.attempt_number,
        chronos_revision: binding.chronos_revision,
        chronos_task_id: binding.chronos_task_id,
    }
}

fn actor_participation_relationship_view(
    value: domain::ActorParticipationRelationshipV1,
) -> public::ActorParticipationRelationshipViewV1 {
    public::ActorParticipationRelationshipViewV1 {
        journey: journey_id_view(value.journey),
        journey_revision: value.journey_revision,
        participation: participation_binding_view(value.participation),
        participation_owner_revision: value.participation_owner_revision,
        authority_decision: value.authority.decision,
        authority_principal: value.authority.principal,
        authority_epoch: value.authority.epoch,
        authority_expires_at: value.authority.expires_at,
        publication: public::ActorPublicationEvidenceViewV1 {
            effect: value.publication.effect,
            source: value.publication.source,
            room_sequence: value.publication.room_sequence,
            room_id: value.publication.room_id,
            message_sequence: value.publication.message_sequence,
            message_id: value.publication.message_id,
            agora_revision: value.publication.agora_revision,
            actor: value.publication.actor,
            subject: value.publication.subject,
        },
    }
}

fn participation_binding_view(
    value: core::agent_participation::BindingV1,
) -> moira_protocol::agent_participation::BindingV1 {
    use moira_protocol::{agent_participation as agent, deliberation};
    agent::BindingV1 {
        participation: agent::ParticipationIdV1(value.participation.0),
        workflow: moira_protocol::WorkflowIdV1(value.workflow.into_bytes()),
        deliberation: deliberation::DeliberationIdV1(value.deliberation.0),
        member: deliberation::EnsembleMemberIdV1(value.member.0),
        session: agent::AgentSessionIdV1(value.session.0),
        profile: agent::ProfileIdV1(value.profile.0),
        attempt: agent::AttemptV1 {
            task_key: value.attempt.task_key,
            incarnation: value.attempt.incarnation,
            attempt: value.attempt.attempt,
        },
        method: deliberation::MethodRefV1 {
            machine_digest: value.method.machine_digest,
            state_digest: value.method.state_digest,
        },
        grounding: deliberation::GroundingRefV1 {
            authority_digest: value.grounding.authority_digest,
            surface_digest: value.grounding.surface_digest,
            depth: match value.grounding.depth {
                core::deliberation::SurfaceDepthV1::Ambient => {
                    deliberation::SurfaceDepthV1::Ambient
                }
                core::deliberation::SurfaceDepthV1::Activated => {
                    deliberation::SurfaceDepthV1::Activated
                }
                core::deliberation::SurfaceDepthV1::Reference => {
                    deliberation::SurfaceDepthV1::Reference
                }
            },
        },
        persona: match value.persona {
            core::deliberation::PersonaLensV1::Historian => deliberation::PersonaLensV1::Historian,
            core::deliberation::PersonaLensV1::IdeaStormer => {
                deliberation::PersonaLensV1::IdeaStormer
            }
            core::deliberation::PersonaLensV1::Theorist => deliberation::PersonaLensV1::Theorist,
            core::deliberation::PersonaLensV1::Researcher => {
                deliberation::PersonaLensV1::Researcher
            }
            core::deliberation::PersonaLensV1::RigorReviewer => {
                deliberation::PersonaLensV1::RigorReviewer
            }
            core::deliberation::PersonaLensV1::ConvergenceFacilitator => {
                deliberation::PersonaLensV1::ConvergenceFacilitator
            }
        },
    }
}

fn refusal(value: domain::RefusalV1) -> public::RefusalV1 {
    match value {
        domain::RefusalV1::InvalidLimits => public::RefusalV1::InvalidLimits,
        domain::RefusalV1::InvalidOwner => public::RefusalV1::InvalidOwner,
        domain::RefusalV1::InvalidTemplate => public::RefusalV1::InvalidTemplate,
        domain::RefusalV1::DuplicateTemplate => public::RefusalV1::DuplicateTemplate,
        domain::RefusalV1::TemplateLimit => public::RefusalV1::TemplateLimit,
        domain::RefusalV1::InvalidPreview => public::RefusalV1::InvalidPreview,
        domain::RefusalV1::PreviewLimit => public::RefusalV1::PreviewLimit,
        domain::RefusalV1::PreviewOperationConflict => public::RefusalV1::PreviewOperationConflict,
        domain::RefusalV1::TemplateNotFound => public::RefusalV1::TemplateNotFound,
        domain::RefusalV1::TemplateMismatch => public::RefusalV1::TemplateMismatch,
        domain::RefusalV1::TemplateRetired => public::RefusalV1::TemplateRetired,
        domain::RefusalV1::PreviewNotFound => public::RefusalV1::PreviewNotFound,
        domain::RefusalV1::PreviewExpired => public::RefusalV1::PreviewExpired,
        domain::RefusalV1::OwnerRevisionConflict => public::RefusalV1::OwnerRevisionConflict,
        domain::RefusalV1::InvalidConfirmation => public::RefusalV1::InvalidConfirmation,
        domain::RefusalV1::JourneyLimit => public::RefusalV1::JourneyLimit,
        domain::RefusalV1::InstantiateOperationConflict => {
            public::RefusalV1::InstantiateOperationConflict
        }
        domain::RefusalV1::PreviewAlreadyInstantiated => {
            public::RefusalV1::PreviewAlreadyInstantiated
        }
        domain::RefusalV1::JourneyNotFound
        | domain::RefusalV1::InvalidTaskAttempt
        | domain::RefusalV1::TaskAttemptMismatch
        | domain::RefusalV1::TaskAttemptConflict
        | domain::RefusalV1::InvalidTaskNamespace
        | domain::RefusalV1::TaskNamespaceConflict
        | domain::RefusalV1::StaleChronosRevision
        | domain::RefusalV1::InvalidRoom
        | domain::RefusalV1::RoomMismatch
        | domain::RefusalV1::RoomConflict
        | domain::RefusalV1::InvalidParticipation
        | domain::RefusalV1::ParticipationConflict
        | domain::RefusalV1::InvalidActorParticipationRelationship
        | domain::RefusalV1::ActorParticipationRelationshipConflict => {
            public::RefusalV1::InvalidCheckpoint
        }
        domain::RefusalV1::IdentityExhausted => public::RefusalV1::IdentityExhausted,
        domain::RefusalV1::InvalidCheckpoint => public::RefusalV1::InvalidCheckpoint,
    }
}
