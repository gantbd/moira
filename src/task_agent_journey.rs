//! Owner-issued task-to-agent journey admission.
//!
//! This module owns immutable templates, expiring previews and human-confirmed
//! journey admission. It deliberately stops before Chronos claim/lease, Agora
//! room creation, participation admission, Daimon session creation or any peer
//! effect. Those transitions require fresh evidence from their sovereign owners.

use crate::{ChronosTaskRefV1, agent_participation::ProfileIdV1};
use std::collections::BTreeMap;

pub mod persistence;

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub [u8; 16]);
    };
}

id!(OwnerIdV1);
id!(TemplateIdV1);
id!(PrincipalIdV1);
id!(PreviewOperationIdV1);
id!(InstantiateOperationIdV1);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PreviewIdV1 {
    pub owner: OwnerIdV1,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct JourneyIdV1 {
    pub owner: OwnerIdV1,
    pub sequence: u64,
}

/// Moira-owned workflow identity reserved with the journey. It is not a peer
/// effect identity and grants no authority.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkflowRefV1 {
    pub owner: OwnerIdV1,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkModeV1 {
    Collaborator,
    ProductManager,
    ProjectManager,
    SiteReliability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoomPolicyV1 {
    Direct,
    SharedContext,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetV1 {
    pub max_context_bytes: u64,
    pub max_output_tokens: u64,
    pub max_elapsed_millis: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TemplateV1 {
    pub id: TemplateIdV1,
    pub version: u64,
    pub digest: [u8; 32],
    pub work_mode: WorkModeV1,
    pub profile: ProfileIdV1,
    pub room_policy: RoomPolicyV1,
    pub budget: BudgetV1,
    pub grounding_policy_digest: [u8; 32],
    pub authority_policy_digest: [u8; 32],
    pub consequence_digest: [u8; 32],
    pub retired: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TemplateRefV1 {
    pub id: TemplateIdV1,
    pub version: u64,
    pub digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskAdmissionV1 {
    pub task: ChronosTaskRefV1,
    pub chronos_revision: u64,
    pub evidence_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewRequestV1 {
    pub operation: PreviewOperationIdV1,
    pub template: TemplateRefV1,
    pub task: TaskAdmissionV1,
    pub profile: ProfileIdV1,
    pub room_policy: RoomPolicyV1,
    pub initiating_principal: PrincipalIdV1,
    pub accountable_human: PrincipalIdV1,
    pub objective_digest: [u8; 32],
    pub acceptance_digest: [u8; 32],
    pub current_evidence_digest: [u8; 32],
    pub requested_ttl: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewSnapshotV1 {
    pub id: PreviewIdV1,
    pub owner_revision: u64,
    pub request: PreviewRequestV1,
    pub observed_at: u64,
    pub expires_at: u64,
    pub work_mode: WorkModeV1,
    pub budget: BudgetV1,
    pub grounding_policy_digest: [u8; 32],
    pub authority_policy_digest: [u8; 32],
    pub consequence_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HumanConfirmationV1 {
    pub preview: PreviewIdV1,
    pub accountable_human: PrincipalIdV1,
    pub consequence_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstantiateRequestV1 {
    pub operation: InstantiateOperationIdV1,
    pub preview: PreviewIdV1,
    pub expected_owner_revision: u64,
    pub confirmation: HumanConfirmationV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseV1 {
    Admitted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JourneySnapshotV1 {
    pub id: JourneyIdV1,
    pub workflow: WorkflowRefV1,
    pub revision: u64,
    pub phase: PhaseV1,
    pub admission: InstantiateRequestV1,
    pub confirmed_at: u64,
    pub preview: PreviewSnapshotV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewReceiptV1 {
    pub preview: PreviewSnapshotV1,
    pub replayed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JourneyReceiptV1 {
    pub journey: JourneySnapshotV1,
    pub replayed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitsV1 {
    pub max_templates: usize,
    pub max_previews: usize,
    pub max_journeys: usize,
    pub max_preview_ttl: u64,
}

impl Default for LimitsV1 {
    fn default() -> Self {
        Self {
            max_templates: 64,
            max_previews: 1_024,
            max_journeys: 1_024,
            max_preview_ttl: 15 * 60,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
    InvalidLimits,
    InvalidOwner,
    InvalidTemplate,
    DuplicateTemplate,
    TemplateLimit,
    InvalidPreview,
    PreviewLimit,
    PreviewOperationConflict,
    TemplateNotFound,
    TemplateMismatch,
    TemplateRetired,
    PreviewNotFound,
    PreviewExpired,
    OwnerRevisionConflict,
    InvalidConfirmation,
    JourneyLimit,
    InstantiateOperationConflict,
    PreviewAlreadyInstantiated,
    IdentityExhausted,
    InvalidCheckpoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointV1 {
    pub owner: OwnerIdV1,
    pub limits: LimitsV1,
    pub owner_revision: u64,
    pub next_preview: u64,
    pub next_journey: u64,
    pub templates: Vec<TemplateV1>,
    pub previews: Vec<PreviewSnapshotV1>,
    pub journeys: Vec<JourneySnapshotV1>,
}

pub struct Engine {
    owner: OwnerIdV1,
    limits: LimitsV1,
    owner_revision: u64,
    next_preview: u64,
    next_journey: u64,
    templates: BTreeMap<TemplateIdV1, TemplateV1>,
    previews: BTreeMap<PreviewIdV1, PreviewSnapshotV1>,
    preview_operations: BTreeMap<PreviewOperationIdV1, PreviewIdV1>,
    journeys: BTreeMap<JourneyIdV1, JourneySnapshotV1>,
    instantiate_operations: BTreeMap<InstantiateOperationIdV1, JourneyIdV1>,
    preview_journeys: BTreeMap<PreviewIdV1, JourneyIdV1>,
}

impl Engine {
    pub fn try_new(
        owner: OwnerIdV1,
        limits: LimitsV1,
        templates: impl IntoIterator<Item = TemplateV1>,
    ) -> Result<Self, RefusalV1> {
        validate_limits(limits)?;
        if owner.0 == [0; 16] {
            return Err(RefusalV1::InvalidOwner);
        }
        let mut catalogue = BTreeMap::new();
        for template in templates {
            if !valid_template(template) {
                return Err(RefusalV1::InvalidTemplate);
            }
            if catalogue.len() >= limits.max_templates {
                return Err(RefusalV1::TemplateLimit);
            }
            if catalogue.insert(template.id, template).is_some() {
                return Err(RefusalV1::DuplicateTemplate);
            }
        }
        Ok(Self {
            owner,
            limits,
            owner_revision: 1,
            next_preview: 0,
            next_journey: 0,
            templates: catalogue,
            previews: BTreeMap::new(),
            preview_operations: BTreeMap::new(),
            journeys: BTreeMap::new(),
            instantiate_operations: BTreeMap::new(),
            preview_journeys: BTreeMap::new(),
        })
    }

    pub const fn owner_revision(&self) -> u64 {
        self.owner_revision
    }

    pub fn template(&self, id: TemplateIdV1) -> Option<&TemplateV1> {
        self.templates.get(&id)
    }

    pub fn preview_snapshot(&self, id: PreviewIdV1) -> Option<&PreviewSnapshotV1> {
        self.previews.get(&id)
    }

    pub fn journey(&self, id: JourneyIdV1) -> Option<&JourneySnapshotV1> {
        self.journeys.get(&id)
    }

    pub fn preview(
        &mut self,
        request: PreviewRequestV1,
        observed_at: u64,
    ) -> Result<PreviewReceiptV1, RefusalV1> {
        if let Some(id) = self.preview_operations.get(&request.operation) {
            let existing = self.previews.get(id).ok_or(RefusalV1::InvalidCheckpoint)?;
            return if existing.request == request {
                Ok(PreviewReceiptV1 {
                    preview: *existing,
                    replayed: true,
                })
            } else {
                Err(RefusalV1::PreviewOperationConflict)
            };
        }
        if self.previews.len() >= self.limits.max_previews {
            return Err(RefusalV1::PreviewLimit);
        }
        if !valid_preview_request(request, self.limits.max_preview_ttl) || observed_at == 0 {
            return Err(RefusalV1::InvalidPreview);
        }
        let expires_at = observed_at
            .checked_add(request.requested_ttl)
            .ok_or(RefusalV1::InvalidPreview)?;
        let template = *self
            .templates
            .get(&request.template.id)
            .ok_or(RefusalV1::TemplateNotFound)?;
        if template.retired {
            return Err(RefusalV1::TemplateRetired);
        }
        if request.template.version != template.version
            || request.template.digest != template.digest
            || request.profile != template.profile
            || request.room_policy != template.room_policy
        {
            return Err(RefusalV1::TemplateMismatch);
        }
        let id = PreviewIdV1 {
            owner: self.owner,
            sequence: self.next_preview,
        };
        let next = self
            .next_preview
            .checked_add(1)
            .ok_or(RefusalV1::IdentityExhausted)?;
        let preview = PreviewSnapshotV1 {
            id,
            owner_revision: self.owner_revision,
            request,
            observed_at,
            expires_at,
            work_mode: template.work_mode,
            budget: template.budget,
            grounding_policy_digest: template.grounding_policy_digest,
            authority_policy_digest: template.authority_policy_digest,
            consequence_digest: template.consequence_digest,
        };
        self.previews.insert(id, preview);
        self.preview_operations.insert(request.operation, id);
        self.next_preview = next;
        Ok(PreviewReceiptV1 {
            preview,
            replayed: false,
        })
    }

    pub fn instantiate(
        &mut self,
        request: InstantiateRequestV1,
        observed_at: u64,
    ) -> Result<JourneyReceiptV1, RefusalV1> {
        if let Some(id) = self.instantiate_operations.get(&request.operation) {
            let existing = self.journeys.get(id).ok_or(RefusalV1::InvalidCheckpoint)?;
            return if existing.admission == request {
                Ok(JourneyReceiptV1 {
                    journey: *existing,
                    replayed: true,
                })
            } else {
                Err(RefusalV1::InstantiateOperationConflict)
            };
        }
        if self.journeys.len() >= self.limits.max_journeys {
            return Err(RefusalV1::JourneyLimit);
        }
        if request.expected_owner_revision != self.owner_revision {
            return Err(RefusalV1::OwnerRevisionConflict);
        }
        let preview = *self
            .previews
            .get(&request.preview)
            .ok_or(RefusalV1::PreviewNotFound)?;
        if self.preview_journeys.contains_key(&preview.id) {
            return Err(RefusalV1::PreviewAlreadyInstantiated);
        }
        if observed_at == 0 || observed_at >= preview.expires_at {
            return Err(RefusalV1::PreviewExpired);
        }
        if request.confirmation.preview != preview.id
            || request.confirmation.accountable_human != preview.request.accountable_human
            || request.confirmation.consequence_digest != preview.consequence_digest
        {
            return Err(RefusalV1::InvalidConfirmation);
        }
        let id = JourneyIdV1 {
            owner: self.owner,
            sequence: self.next_journey,
        };
        let next = self
            .next_journey
            .checked_add(1)
            .ok_or(RefusalV1::IdentityExhausted)?;
        let journey = JourneySnapshotV1 {
            id,
            workflow: WorkflowRefV1 {
                owner: self.owner,
                sequence: id.sequence,
            },
            revision: 0,
            phase: PhaseV1::Admitted,
            admission: request,
            confirmed_at: observed_at,
            preview,
        };
        self.journeys.insert(id, journey);
        self.instantiate_operations.insert(request.operation, id);
        self.preview_journeys.insert(preview.id, id);
        self.next_journey = next;
        Ok(JourneyReceiptV1 {
            journey,
            replayed: false,
        })
    }

    pub fn checkpoint(&self) -> CheckpointV1 {
        CheckpointV1 {
            owner: self.owner,
            limits: self.limits,
            owner_revision: self.owner_revision,
            next_preview: self.next_preview,
            next_journey: self.next_journey,
            templates: self.templates.values().copied().collect(),
            previews: self.previews.values().copied().collect(),
            journeys: self.journeys.values().copied().collect(),
        }
    }

    pub fn recover(checkpoint: CheckpointV1) -> Result<Self, RefusalV1> {
        let mut engine = Self::try_new(checkpoint.owner, checkpoint.limits, checkpoint.templates)?;
        if checkpoint.owner_revision != 1
            || checkpoint.next_preview
                != u64::try_from(checkpoint.previews.len())
                    .map_err(|_| RefusalV1::InvalidCheckpoint)?
            || checkpoint.next_journey
                != u64::try_from(checkpoint.journeys.len())
                    .map_err(|_| RefusalV1::InvalidCheckpoint)?
        {
            return Err(RefusalV1::InvalidCheckpoint);
        }
        for preview in checkpoint.previews {
            if preview.id.owner != checkpoint.owner
                || preview.id.sequence >= checkpoint.next_preview
                || preview.owner_revision != checkpoint.owner_revision
                || engine
                    .preview(preview.request, preview.observed_at)?
                    .preview
                    != preview
            {
                return Err(RefusalV1::InvalidCheckpoint);
            }
        }
        for journey in checkpoint.journeys {
            if journey.id.owner != checkpoint.owner
                || journey.workflow.owner != checkpoint.owner
                || journey.id.sequence != journey.workflow.sequence
                || journey.id.sequence >= checkpoint.next_journey
                || engine
                    .instantiate(journey.admission, journey.confirmed_at)?
                    .journey
                    != journey
            {
                return Err(RefusalV1::InvalidCheckpoint);
            }
        }
        if engine.next_preview != checkpoint.next_preview
            || engine.next_journey != checkpoint.next_journey
        {
            return Err(RefusalV1::InvalidCheckpoint);
        }
        Ok(engine)
    }
}

fn validate_limits(limits: LimitsV1) -> Result<(), RefusalV1> {
    if limits.max_templates == 0
        || limits.max_previews == 0
        || limits.max_journeys == 0
        || limits.max_preview_ttl == 0
    {
        Err(RefusalV1::InvalidLimits)
    } else {
        Ok(())
    }
}

fn valid_template(value: TemplateV1) -> bool {
    value.id.0 != [0; 16]
        && value.version > 0
        && value.digest != [0; 32]
        && value.profile.0 != [0; 16]
        && valid_budget(value.budget)
        && value.grounding_policy_digest != [0; 32]
        && value.authority_policy_digest != [0; 32]
        && value.consequence_digest != [0; 32]
}

fn valid_budget(value: BudgetV1) -> bool {
    value.max_context_bytes > 0 && value.max_output_tokens > 0 && value.max_elapsed_millis > 0
}

fn valid_preview_request(value: PreviewRequestV1, max_ttl: u64) -> bool {
    value.operation.0 != [0; 16]
        && value.task.task.external_task_key() != [0; 16]
        && value.task.task.incarnation() > 0
        && value.task.chronos_revision > 0
        && value.task.evidence_digest != [0; 32]
        && value.initiating_principal.0 != [0; 16]
        && value.accountable_human.0 != [0; 16]
        && value.objective_digest != [0; 32]
        && value.acceptance_digest != [0; 32]
        && value.current_evidence_digest != [0; 32]
        && value.requested_ttl > 0
        && value.requested_ttl <= max_ttl
}
