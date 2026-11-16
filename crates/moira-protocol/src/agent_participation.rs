//! Public durable record for one governed interactive-agent participation.

use crate::{WorkflowIdV1, deliberation};
use sha2::{Digest, Sha256};

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub [u8; 16]);
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "(<opaque>)"))
            }
        }
    };
}
id!(ParticipationIdV1);
id!(AgentSessionIdV1);
id!(AgentTurnIdV1);
id!(ExecutionIdV1);
id!(ProfileIdV1);
id!(IdempotencyKeyV1);
id!(PermissionRequestIdV1);
id!(PermissionOptionIdV1);
id!(AuthorityDecisionIdV1);
id!(AuthoritySubjectIdV1);

pub const MAX_PERMISSION_OPTIONS_V1: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptV1 {
    pub task_key: [u8; 16],
    pub incarnation: u64,
    pub attempt: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BindingV1 {
    pub participation: ParticipationIdV1,
    pub workflow: WorkflowIdV1,
    pub deliberation: deliberation::DeliberationIdV1,
    pub member: deliberation::EnsembleMemberIdV1,
    pub session: AgentSessionIdV1,
    pub profile: ProfileIdV1,
    pub attempt: AttemptV1,
    pub method: deliberation::MethodRefV1,
    pub grounding: deliberation::GroundingRefV1,
    pub persona: deliberation::PersonaLensV1,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CapabilitiesV1 {
    pub load_session: bool,
    pub resume_session: bool,
    pub permissions: bool,
    pub filesystem_read: bool,
    pub filesystem_write: bool,
    pub terminal: bool,
    pub session_modes: bool,
    pub session_models: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionEvidenceV1 {
    pub request: PermissionRequestIdV1,
    pub turn: AgentTurnIdV1,
    pub capability_digest: [u8; 32],
    pub options_digest: [u8; 32],
    pub option_count: u8,
    pub option_ids: [PermissionOptionIdV1; MAX_PERMISSION_OPTIONS_V1],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionAuthorityV1 {
    pub decision: AuthorityDecisionIdV1,
    pub subject: AuthoritySubjectIdV1,
    pub capability_digest: [u8; 32],
    pub epoch: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispositionV1 {
    Completed,
    Failed,
    Cancelled,
    TimedOut,
    Uncertain,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReasonV1 {
    EndTurn,
    Cancelled,
    TimedOut,
    Failed,
    Disconnected,
    OutcomeUnknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalEvidenceV1 {
    pub turn: AgentTurnIdV1,
    pub disposition: DispositionV1,
    pub stop_reason: StopReasonV1,
    pub output_digest: [u8; 32],
    pub provider_evidence_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationV1 {
    Open {
        key: IdempotencyKeyV1,
        execution: ExecutionIdV1,
    },
    Prompt {
        key: IdempotencyKeyV1,
        turn: AgentTurnIdV1,
        execution: ExecutionIdV1,
        prompt_digest: [u8; 32],
        attempt: AttemptV1,
    },
    Advance {
        key: IdempotencyKeyV1,
        turn: AgentTurnIdV1,
    },
    DecidePermission {
        key: IdempotencyKeyV1,
        execution: ExecutionIdV1,
        request: PermissionRequestIdV1,
        option: PermissionOptionIdV1,
        authority: PermissionAuthorityV1,
        attempt: AttemptV1,
    },
    Cancel {
        key: IdempotencyKeyV1,
        turn: AgentTurnIdV1,
    },
    Close {
        key: IdempotencyKeyV1,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseV1 {
    Admitted,
    Ready,
    TurnActive,
    AwaitingPermission,
    Terminal,
    FencedUnknown,
    Closed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationV1 {
    pub phase: PhaseV1,
    pub session_revision: u64,
    pub active_turn: Option<AgentTurnIdV1>,
    pub last_event_cursor: Option<u64>,
    pub profile_document_digest: [u8; 32],
    pub capabilities: CapabilitiesV1,
    pub permission: Option<PermissionEvidenceV1>,
    pub terminal: Option<TerminalEvidenceV1>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParticipationViewV1 {
    pub binding: BindingV1,
    pub revision: u64,
    pub phase: PhaseV1,
    pub session_revision: Option<u64>,
    pub last_event_cursor: Option<u64>,
    pub profile_document_digest: Option<[u8; 32]>,
    pub capabilities: CapabilitiesV1,
    pub active_turn: Option<AgentTurnIdV1>,
    pub permission: Option<PermissionEvidenceV1>,
    pub terminal: Option<TerminalEvidenceV1>,
    pub pending: Option<OperationV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationV1 {
    Admit(BindingV1),
    Prepare {
        participation: ParticipationIdV1,
        expected_revision: u64,
        operation: OperationV1,
    },
    Observe {
        participation: ParticipationIdV1,
        expected_revision: u64,
        key: IdempotencyKeyV1,
        observation: ObservationV1,
    },
    FenceUnknown {
        participation: ParticipationIdV1,
        expected_revision: u64,
        key: IdempotencyKeyV1,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandV1 {
    Apply {
        expected_storage_revision: Option<u64>,
        mutation: Box<MutationV1>,
    },
    Status,
    Participation {
        participation: ParticipationIdV1,
    },
    SelectPermission(PermissionSelectionV1),
    PermissionConfirmation {
        participation: ParticipationIdV1,
    },
    ConfirmPermission(ConfirmedPermissionSelectionV1),
    CancelConfirmation {
        participation: ParticipationIdV1,
    },
    ConfirmCancel(ConfirmedCancelV1),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionSelectionV1 {
    pub expected_storage_revision: Option<u64>,
    pub participation: ParticipationIdV1,
    pub expected_revision: u64,
    pub key: IdempotencyKeyV1,
    pub execution: ExecutionIdV1,
    pub request: PermissionRequestIdV1,
    pub option: PermissionOptionIdV1,
    /// Untrusted claim used to scope command authorization. The durable
    /// participation engine checks it against the retained provider request.
    pub capability_digest: [u8; 32],
    pub attempt: AttemptV1,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfirmedPermissionSelectionV1 {
    pub participation: ParticipationIdV1,
    pub confirmation_digest: [u8; 32],
    pub option: PermissionOptionIdV1,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionConfirmationV1 {
    pub storage_revision: u64,
    pub participation: ParticipationIdV1,
    pub participation_revision: u64,
    pub session_revision: u64,
    pub request: PermissionRequestIdV1,
    pub turn: AgentTurnIdV1,
    pub capability_digest: [u8; 32],
    pub options_digest: [u8; 32],
    pub option_count: u8,
    pub option_ids: [PermissionOptionIdV1; MAX_PERMISSION_OPTIONS_V1],
    pub confirmation_digest: [u8; 32],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfirmedCancelV1 {
    pub participation: ParticipationIdV1,
    pub confirmation_digest: [u8; 32],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CancelConfirmationV1 {
    pub storage_revision: u64,
    pub participation: ParticipationIdV1,
    pub participation_revision: u64,
    pub session_revision: u64,
    pub turn: AgentTurnIdV1,
    pub confirmation_digest: [u8; 32],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusV1 {
    pub storage_revision: Option<u64>,
    pub needs_reopen: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutcomeV1 {
    Applied {
        storage_revision: u64,
        participation: ParticipationViewV1,
        replayed: bool,
    },
    Status(StatusV1),
    Participation(Option<ParticipationViewV1>),
    PermissionConfirmation(Option<PermissionConfirmationV1>),
    CancelConfirmation(Option<CancelConfirmationV1>),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
    NotAuthorized,
    StorageRevisionConflict {
        expected: Option<u64>,
        actual: Option<u64>,
    },
    InvalidLimits,
    Limit,
    InvalidBinding,
    DuplicateParticipation,
    UnknownParticipation,
    RevisionConflict,
    RevisionExhausted,
    InvalidOperation,
    OperationConflict,
    NoPendingOperation,
    InvalidObservation,
    ObservationConflict,
    InvalidCheckpoint,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplyV1 {
    Accepted(Box<OutcomeV1>),
    Refused(RefusalV1),
}

/// Builds the exact presentation challenge for one current retained permission.
/// The digest is evidence for stale-choice prevention, not execution authority.
pub fn permission_confirmation(
    storage_revision: u64,
    view: &ParticipationViewV1,
) -> Option<PermissionConfirmationV1> {
    let session_revision = view.session_revision?;
    let permission = view.permission?;
    let count = usize::from(permission.option_count);
    if view.phase != PhaseV1::AwaitingPermission
        || view.active_turn != Some(permission.turn)
        || count == 0
        || count > MAX_PERMISSION_OPTIONS_V1
        || permission.option_ids[..count]
            .iter()
            .any(|option| option.0 == [0; 16])
        || permission.option_ids[..count]
            .iter()
            .enumerate()
            .any(|(index, option)| permission.option_ids[..index].contains(option))
        || permission.option_ids[count..]
            .iter()
            .any(|option| option.0 != [0; 16])
    {
        return None;
    }
    let mut hash = Sha256::new();
    hash.update(b"MOIRAPCF1");
    hash.update(storage_revision.to_be_bytes());
    hash.update(view.binding.participation.0);
    hash.update(view.binding.workflow.0);
    hash.update(view.binding.deliberation.0);
    hash.update(view.binding.member.0);
    hash.update(view.binding.session.0);
    hash.update(view.binding.profile.0);
    hash.update(view.binding.attempt.task_key);
    hash.update(view.binding.attempt.incarnation.to_be_bytes());
    hash.update(view.binding.attempt.attempt.to_be_bytes());
    hash.update(view.binding.method.machine_digest);
    hash.update(view.binding.method.state_digest);
    hash.update(view.binding.grounding.authority_digest);
    hash.update(view.binding.grounding.surface_digest);
    hash.update([depth_tag(view.binding.grounding.depth)]);
    hash.update([persona_tag(view.binding.persona)]);
    hash.update(view.revision.to_be_bytes());
    hash.update(session_revision.to_be_bytes());
    hash.update(permission.request.0);
    hash.update(permission.turn.0);
    hash.update(permission.capability_digest);
    hash.update(permission.options_digest);
    hash.update([permission.option_count]);
    for option in &permission.option_ids[..count] {
        hash.update(option.0);
    }
    Some(PermissionConfirmationV1 {
        storage_revision,
        participation: view.binding.participation,
        participation_revision: view.revision,
        session_revision,
        request: permission.request,
        turn: permission.turn,
        capability_digest: permission.capability_digest,
        options_digest: permission.options_digest,
        option_count: permission.option_count,
        option_ids: permission.option_ids,
        confirmation_digest: hash.finalize().into(),
    })
}

/// Resolves a verified current challenge into opaque operation identities.
/// These identities are deterministic for replay and confer no authority.
pub fn confirmed_permission_selection(
    confirmation: PermissionConfirmationV1,
    selected: ConfirmedPermissionSelectionV1,
    attempt: AttemptV1,
) -> Option<PermissionSelectionV1> {
    let count = usize::from(confirmation.option_count);
    if count == 0
        || count > MAX_PERMISSION_OPTIONS_V1
        || confirmation.option_ids[..count]
            .iter()
            .any(|option| option.0 == [0; 16])
        || confirmation.option_ids[..count]
            .iter()
            .enumerate()
            .any(|(index, option)| confirmation.option_ids[..index].contains(option))
        || confirmation.option_ids[count..]
            .iter()
            .any(|option| option.0 != [0; 16])
        || selected.participation != confirmation.participation
        || selected.confirmation_digest != confirmation.confirmation_digest
        || !confirmation.option_ids[..count].contains(&selected.option)
    {
        return None;
    }
    let mut key = Sha256::new();
    key.update(b"MOIRAPKEY1");
    key.update(confirmation.confirmation_digest);
    key.update(selected.option.0);
    let key: [u8; 32] = key.finalize().into();
    let mut execution = Sha256::new();
    execution.update(b"MOIRAPEXE1");
    execution.update(confirmation.confirmation_digest);
    execution.update(selected.option.0);
    let execution: [u8; 32] = execution.finalize().into();
    if key[..16] == [0; 16] || execution[..16] == [0; 16] {
        return None;
    }
    Some(PermissionSelectionV1 {
        expected_storage_revision: Some(confirmation.storage_revision),
        participation: confirmation.participation,
        expected_revision: confirmation.participation_revision,
        key: IdempotencyKeyV1(key[..16].try_into().ok()?),
        execution: ExecutionIdV1(execution[..16].try_into().ok()?),
        request: confirmation.request,
        option: selected.option,
        capability_digest: confirmation.capability_digest,
        attempt,
    })
}

/// Builds a current cancellation challenge for one active provider turn.
pub fn cancel_confirmation(
    storage_revision: u64,
    view: &ParticipationViewV1,
) -> Option<CancelConfirmationV1> {
    let session_revision = view.session_revision?;
    let turn = view.active_turn?;
    if !matches!(
        view.phase,
        PhaseV1::TurnActive | PhaseV1::AwaitingPermission
    ) || view.pending.is_some()
    {
        return None;
    }
    let mut hash = Sha256::new();
    hash.update(b"MOIRACCF1");
    hash.update(storage_revision.to_be_bytes());
    hash.update(view.binding.participation.0);
    hash.update(view.binding.workflow.0);
    hash.update(view.binding.deliberation.0);
    hash.update(view.binding.member.0);
    hash.update(view.binding.session.0);
    hash.update(view.binding.profile.0);
    hash.update(view.binding.attempt.task_key);
    hash.update(view.binding.attempt.incarnation.to_be_bytes());
    hash.update(view.binding.attempt.attempt.to_be_bytes());
    hash.update(view.binding.method.machine_digest);
    hash.update(view.binding.method.state_digest);
    hash.update(view.binding.grounding.authority_digest);
    hash.update(view.binding.grounding.surface_digest);
    hash.update([depth_tag(view.binding.grounding.depth)]);
    hash.update([persona_tag(view.binding.persona)]);
    hash.update(view.revision.to_be_bytes());
    hash.update(session_revision.to_be_bytes());
    hash.update(turn.0);
    Some(CancelConfirmationV1 {
        storage_revision,
        participation: view.binding.participation,
        participation_revision: view.revision,
        session_revision,
        turn,
        confirmation_digest: hash.finalize().into(),
    })
}

pub fn confirmed_cancel_operation(
    confirmation: CancelConfirmationV1,
    selected: ConfirmedCancelV1,
) -> Option<OperationV1> {
    if selected.participation != confirmation.participation
        || selected.confirmation_digest != confirmation.confirmation_digest
    {
        return None;
    }
    let mut key = Sha256::new();
    key.update(b"MOIRACKEY1");
    key.update(confirmation.confirmation_digest);
    key.update(confirmation.turn.0);
    let key: [u8; 32] = key.finalize().into();
    if key[..16] == [0; 16] {
        return None;
    }
    Some(OperationV1::Cancel {
        key: IdempotencyKeyV1(key[..16].try_into().ok()?),
        turn: confirmation.turn,
    })
}

const fn depth_tag(value: deliberation::SurfaceDepthV1) -> u8 {
    match value {
        deliberation::SurfaceDepthV1::Ambient => 0,
        deliberation::SurfaceDepthV1::Activated => 1,
        deliberation::SurfaceDepthV1::Reference => 2,
    }
}

const fn persona_tag(value: deliberation::PersonaLensV1) -> u8 {
    match value {
        deliberation::PersonaLensV1::Historian => 0,
        deliberation::PersonaLensV1::IdeaStormer => 1,
        deliberation::PersonaLensV1::Theorist => 2,
        deliberation::PersonaLensV1::Researcher => 3,
        deliberation::PersonaLensV1::RigorReviewer => 4,
        deliberation::PersonaLensV1::ConvergenceFacilitator => 5,
    }
}
