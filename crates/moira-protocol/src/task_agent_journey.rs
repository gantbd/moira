//! Transport-neutral task-to-agent journey admission contract.
//!
//! Human confirmation binds a preview and consequence but grants no authority.
//! Peer effects remain unavailable from this contract until Moira publishes
//! their owner-evidence transitions separately.

use crate::{TaskRefV1, agent_participation::ProfileIdV1};

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub [u8; 16]);
        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(<opaque>)"))
            }
        }
    };
}

id!(OwnerIdV1);
id!(TemplateIdV1);
id!(PrincipalIdV1);
id!(PreviewOperationIdV1);
id!(InstantiateOperationIdV1);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PreviewIdV1 {
    pub owner: OwnerIdV1,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct JourneyIdV1 {
    pub owner: OwnerIdV1,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    pub task: TaskRefV1,
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
pub struct PreviewViewV1 {
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
pub struct JourneyViewV1 {
    pub id: JourneyIdV1,
    pub workflow: WorkflowRefV1,
    pub revision: u64,
    pub phase: PhaseV1,
    pub admission: InstantiateRequestV1,
    pub confirmed_at: u64,
    pub preview: PreviewViewV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewReceiptV1 {
    pub preview: PreviewViewV1,
    pub replayed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JourneyReceiptV1 {
    pub journey: JourneyViewV1,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandV1 {
    Catalogue,
    Preview {
        expected_storage_revision: Option<u64>,
        request: PreviewRequestV1,
    },
    Instantiate {
        expected_storage_revision: Option<u64>,
        request: InstantiateRequestV1,
    },
    PreviewView {
        preview: PreviewIdV1,
    },
    Journey {
        journey: JourneyIdV1,
    },
    Status,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusV1 {
    pub owner: OwnerIdV1,
    pub owner_revision: u64,
    pub storage_revision: u64,
    pub template_count: u64,
    pub preview_count: u64,
    pub journey_count: u64,
    pub needs_reopen: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutcomeV1 {
    Catalogue {
        owner: OwnerIdV1,
        owner_revision: u64,
        storage_revision: u64,
        templates: Vec<TemplateV1>,
    },
    Previewed {
        storage_revision: u64,
        receipt: PreviewReceiptV1,
    },
    Admitted {
        storage_revision: u64,
        receipt: JourneyReceiptV1,
    },
    Preview(Option<PreviewViewV1>),
    Journey(Option<JourneyViewV1>),
    Status(StatusV1),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
    NotAuthorized,
    StorageRevisionConflict,
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
    Storage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplyV1 {
    Accepted(Box<OutcomeV1>),
    Refused(RefusalV1),
}
