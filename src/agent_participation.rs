//! Durable methodical participation for interactive agent sessions.
//!
//! This state machine never invokes a provider and never mints authority. It
//! binds every prepared operation and observed result to one retained workflow,
//! deliberation member, exact attempt, Rune method, and Dendrite grounding.

use crate::{WorkflowId, deliberation};
use std::collections::BTreeMap;

pub mod persistence;

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub [u8; 16]);
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
    pub workflow: WorkflowId,
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

impl OperationV1 {
    pub const fn key(self) -> IdempotencyKeyV1 {
        match self {
            Self::Open { key, .. }
            | Self::Prompt { key, .. }
            | Self::Advance { key, .. }
            | Self::DecidePermission { key, .. }
            | Self::Cancel { key, .. }
            | Self::Close { key } => key,
        }
    }
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
pub struct SnapshotV1 {
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
    pub last_operation: Option<OperationV1>,
    pub last_observation: Option<ObservationV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointV1 {
    pub version: u8,
    pub limits: LimitsV1,
    pub participations: Vec<SnapshotV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitsV1 {
    pub max_participations: usize,
}

impl Default for LimitsV1 {
    fn default() -> Self {
        Self {
            max_participations: 1_024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiptV1 {
    pub participation: ParticipationIdV1,
    pub revision: u64,
    pub phase: PhaseV1,
    pub replayed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
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

pub struct Engine {
    limits: LimitsV1,
    participations: BTreeMap<ParticipationIdV1, SnapshotV1>,
}

impl Engine {
    pub fn try_new(limits: LimitsV1) -> Result<Self, RefusalV1> {
        if limits.max_participations == 0 {
            return Err(RefusalV1::InvalidLimits);
        }
        Ok(Self {
            limits,
            participations: BTreeMap::new(),
        })
    }

    pub fn participation(&self, id: ParticipationIdV1) -> Option<&SnapshotV1> {
        self.participations.get(&id)
    }

    pub fn checkpoint(&self) -> CheckpointV1 {
        CheckpointV1 {
            version: 1,
            limits: self.limits,
            participations: self.participations.values().cloned().collect(),
        }
    }

    pub fn admit(&mut self, binding: BindingV1) -> Result<ReceiptV1, RefusalV1> {
        if !valid_binding(binding) {
            return Err(RefusalV1::InvalidBinding);
        }
        if let Some(existing) = self.participations.get(&binding.participation) {
            return if existing.binding == binding {
                Ok(receipt(existing, true))
            } else {
                Err(RefusalV1::DuplicateParticipation)
            };
        }
        if self.participations.len() >= self.limits.max_participations {
            return Err(RefusalV1::Limit);
        }
        let snapshot = SnapshotV1 {
            binding,
            revision: 0,
            phase: PhaseV1::Admitted,
            session_revision: None,
            last_event_cursor: None,
            profile_document_digest: None,
            capabilities: CapabilitiesV1::default(),
            active_turn: None,
            permission: None,
            terminal: None,
            pending: None,
            last_operation: None,
            last_observation: None,
        };
        let result = receipt(&snapshot, false);
        self.participations.insert(binding.participation, snapshot);
        Ok(result)
    }

    pub fn prepare(
        &mut self,
        id: ParticipationIdV1,
        expected_revision: u64,
        operation: OperationV1,
    ) -> Result<ReceiptV1, RefusalV1> {
        let snapshot = self
            .participations
            .get_mut(&id)
            .ok_or(RefusalV1::UnknownParticipation)?;
        if snapshot.pending == Some(operation) {
            return Ok(receipt(snapshot, true));
        }
        if snapshot.last_operation == Some(operation) {
            return Ok(receipt(snapshot, true));
        }
        if snapshot.revision != expected_revision {
            return Err(RefusalV1::RevisionConflict);
        }
        if snapshot.pending.is_some() {
            return Err(RefusalV1::OperationConflict);
        }
        if !valid_operation(snapshot, operation) {
            return Err(RefusalV1::InvalidOperation);
        }
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .ok_or(RefusalV1::RevisionExhausted)?;
        snapshot.pending = Some(operation);
        snapshot.last_observation = None;
        Ok(receipt(snapshot, false))
    }

    pub fn observe(
        &mut self,
        id: ParticipationIdV1,
        expected_revision: u64,
        key: IdempotencyKeyV1,
        observation: ObservationV1,
    ) -> Result<ReceiptV1, RefusalV1> {
        let snapshot = self
            .participations
            .get_mut(&id)
            .ok_or(RefusalV1::UnknownParticipation)?;
        if snapshot.last_operation.is_some_and(|op| op.key() == key)
            && snapshot.last_observation == Some(observation)
        {
            return Ok(receipt(snapshot, true));
        }
        if snapshot.revision != expected_revision {
            return Err(RefusalV1::RevisionConflict);
        }
        let operation = snapshot.pending.ok_or(RefusalV1::NoPendingOperation)?;
        if operation.key() != key || !valid_observation(snapshot, operation, observation) {
            return Err(RefusalV1::InvalidObservation);
        }
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .ok_or(RefusalV1::RevisionExhausted)?;
        snapshot.phase = observation.phase;
        snapshot.session_revision = Some(observation.session_revision);
        snapshot.last_event_cursor = observation.last_event_cursor;
        snapshot.profile_document_digest = Some(observation.profile_document_digest);
        snapshot.capabilities = observation.capabilities;
        snapshot.active_turn = observation.active_turn;
        snapshot.permission = observation.permission;
        snapshot.terminal = observation.terminal;
        snapshot.pending = None;
        snapshot.last_operation = Some(operation);
        snapshot.last_observation = Some(observation);
        Ok(receipt(snapshot, false))
    }

    pub fn fence_unknown(
        &mut self,
        id: ParticipationIdV1,
        expected_revision: u64,
        key: IdempotencyKeyV1,
    ) -> Result<ReceiptV1, RefusalV1> {
        let snapshot = self
            .participations
            .get_mut(&id)
            .ok_or(RefusalV1::UnknownParticipation)?;
        let operation = snapshot.pending.ok_or(RefusalV1::NoPendingOperation)?;
        if operation.key() != key {
            return Err(RefusalV1::OperationConflict);
        }
        if snapshot.phase == PhaseV1::FencedUnknown {
            return Ok(receipt(snapshot, true));
        }
        if snapshot.revision != expected_revision {
            return Err(RefusalV1::RevisionConflict);
        }
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .ok_or(RefusalV1::RevisionExhausted)?;
        snapshot.phase = PhaseV1::FencedUnknown;
        Ok(receipt(snapshot, false))
    }

    pub fn recover(checkpoint: CheckpointV1) -> Result<Self, RefusalV1> {
        if checkpoint.version != 1
            || checkpoint.participations.len() > checkpoint.limits.max_participations
        {
            return Err(RefusalV1::InvalidCheckpoint);
        }
        let mut engine =
            Self::try_new(checkpoint.limits).map_err(|_| RefusalV1::InvalidCheckpoint)?;
        let mut prior = None;
        for snapshot in checkpoint.participations {
            if prior.is_some_and(|value| value >= snapshot.binding.participation)
                || !valid_snapshot(&snapshot)
            {
                return Err(RefusalV1::InvalidCheckpoint);
            }
            prior = Some(snapshot.binding.participation);
            engine
                .participations
                .insert(snapshot.binding.participation, snapshot);
        }
        Ok(engine)
    }
}

fn receipt(snapshot: &SnapshotV1, replayed: bool) -> ReceiptV1 {
    ReceiptV1 {
        participation: snapshot.binding.participation,
        revision: snapshot.revision,
        phase: snapshot.phase,
        replayed,
    }
}

fn valid_binding(value: BindingV1) -> bool {
    value.participation.0 != [0; 16]
        && value.workflow.into_bytes() != [0; 16]
        && value.deliberation.0 != [0; 16]
        && value.member.0 != [0; 16]
        && value.session.0 != [0; 16]
        && value.profile.0 != [0; 16]
        && valid_attempt(value.attempt)
        && value.method.machine_digest != [0; 32]
        && value.method.state_digest != [0; 32]
        && value.grounding.authority_digest != [0; 32]
        && value.grounding.surface_digest != [0; 32]
}

fn valid_attempt(value: AttemptV1) -> bool {
    value.task_key != [0; 16] && value.incarnation != 0 && value.attempt != 0
}
fn nonzero_key(value: IdempotencyKeyV1) -> bool {
    value.0 != [0; 16]
}

fn valid_operation(snapshot: &SnapshotV1, value: OperationV1) -> bool {
    if !nonzero_key(value.key()) || snapshot.phase == PhaseV1::Closed {
        return false;
    }
    match value {
        OperationV1::Open { execution, .. } => {
            snapshot.phase == PhaseV1::Admitted && execution.0 != [0; 16]
        }
        OperationV1::Prompt {
            turn,
            execution,
            prompt_digest,
            attempt,
            ..
        } => {
            snapshot.phase == PhaseV1::Ready
                && turn.0 != [0; 16]
                && execution.0 != [0; 16]
                && prompt_digest != [0; 32]
                && attempt == snapshot.binding.attempt
        }
        OperationV1::Advance { turn, .. } => {
            matches!(snapshot.phase, PhaseV1::TurnActive) && snapshot.active_turn == Some(turn)
        }
        OperationV1::DecidePermission {
            execution,
            request,
            option,
            authority,
            attempt,
            ..
        } => {
            snapshot.phase == PhaseV1::AwaitingPermission
                && execution.0 != [0; 16]
                && option.0 != [0; 16]
                && attempt == snapshot.binding.attempt
                && snapshot.permission.is_some_and(|permission| {
                    permission.request == request
                        && permission.capability_digest == authority.capability_digest
                        && permission.option_ids[..usize::from(permission.option_count)]
                            .contains(&option)
                })
                && authority.decision.0 != [0; 16]
                && authority.subject.0 != [0; 16]
                && authority.epoch != 0
        }
        OperationV1::Cancel { turn, .. } => {
            matches!(
                snapshot.phase,
                PhaseV1::TurnActive | PhaseV1::AwaitingPermission
            ) && snapshot.active_turn == Some(turn)
        }
        OperationV1::Close { .. } => matches!(snapshot.phase, PhaseV1::Ready | PhaseV1::Terminal),
    }
}

fn valid_observation(snapshot: &SnapshotV1, operation: OperationV1, value: ObservationV1) -> bool {
    if value.profile_document_digest == [0; 32]
        || snapshot
            .session_revision
            .is_some_and(|prior| value.session_revision <= prior)
    {
        return false;
    }
    let shape = match value.phase {
        PhaseV1::Admitted | PhaseV1::FencedUnknown => false,
        PhaseV1::Ready => {
            value.active_turn.is_none() && value.permission.is_none() && value.terminal.is_none()
        }
        PhaseV1::TurnActive => {
            value.active_turn.is_some() && value.permission.is_none() && value.terminal.is_none()
        }
        PhaseV1::AwaitingPermission => {
            value.active_turn.is_some()
                && value
                    .permission
                    .is_some_and(|permission| Some(permission.turn) == value.active_turn)
                && value.terminal.is_none()
        }
        PhaseV1::Terminal => {
            value.active_turn.is_none() && value.permission.is_none() && value.terminal.is_some()
        }
        PhaseV1::Closed => value.active_turn.is_none() && value.permission.is_none(),
    };
    if !shape {
        return false;
    }
    match operation {
        OperationV1::Open { .. } => value.phase == PhaseV1::Ready,
        OperationV1::Prompt { turn, .. } => observation_for_turn(value, turn),
        OperationV1::Advance { turn, .. } => observation_for_turn(value, turn),
        OperationV1::DecidePermission { request, .. } => {
            value.permission.is_none()
                && matches!(value.phase, PhaseV1::TurnActive | PhaseV1::Terminal)
                && snapshot
                    .permission
                    .is_some_and(|permission| permission.request == request)
        }
        OperationV1::Cancel { turn, .. } => {
            value.phase == PhaseV1::Terminal
                && value.terminal.is_some_and(|terminal| {
                    terminal.turn == turn
                        && terminal.disposition == DispositionV1::Cancelled
                        && terminal.stop_reason == StopReasonV1::Cancelled
                })
        }
        OperationV1::Close { .. } => value.phase == PhaseV1::Closed,
    }
}

fn observation_for_turn(value: ObservationV1, turn: AgentTurnIdV1) -> bool {
    match value.phase {
        PhaseV1::TurnActive | PhaseV1::AwaitingPermission => value.active_turn == Some(turn),
        PhaseV1::Terminal => value.terminal.is_some_and(|terminal| terminal.turn == turn),
        _ => false,
    }
}

fn valid_snapshot(value: &SnapshotV1) -> bool {
    if !valid_binding(value.binding)
        || value.pending.is_some() && value.last_operation == value.pending
        || value
            .profile_document_digest
            .is_some_and(|digest| digest == [0; 32])
        || value.permission.is_some_and(|evidence| {
            evidence.request.0 == [0; 16]
                || evidence.turn.0 == [0; 16]
                || evidence.capability_digest == [0; 32]
                || evidence.options_digest == [0; 32]
                || !valid_permission_options(evidence)
        })
        || value.terminal.is_some_and(|evidence| {
            evidence.turn.0 == [0; 16]
                || evidence.output_digest == [0; 32]
                || evidence.provider_evidence_digest == [0; 32]
        })
        || value.pending.is_some_and(|operation| {
            if value.phase == PhaseV1::FencedUnknown {
                !valid_fenced_operation(value, operation)
            } else {
                !valid_operation(value, operation)
            }
        })
        || !matches!(
            (
                value.last_operation,
                value.last_observation,
                value.pending.is_some()
            ),
            (None, None, _) | (Some(_), Some(_), _) | (Some(_), None, true)
        )
        || value
            .last_observation
            .is_some_and(|observation| !observation_matches_snapshot(value, observation))
    {
        return false;
    }
    match value.phase {
        PhaseV1::Admitted => {
            value.session_revision.is_none()
                && value.active_turn.is_none()
                && value.permission.is_none()
                && value.terminal.is_none()
                && value
                    .pending
                    .is_none_or(|operation| matches!(operation, OperationV1::Open { .. }))
        }
        PhaseV1::Ready => {
            value.session_revision.is_some()
                && value.active_turn.is_none()
                && value.permission.is_none()
                && value.terminal.is_none()
        }
        PhaseV1::TurnActive => {
            value.session_revision.is_some()
                && value.active_turn.is_some()
                && value.permission.is_none()
                && value.terminal.is_none()
        }
        PhaseV1::AwaitingPermission => {
            value.session_revision.is_some()
                && value.active_turn.is_some()
                && value.permission.is_some()
        }
        PhaseV1::Terminal => {
            value.session_revision.is_some()
                && value.active_turn.is_none()
                && value.permission.is_none()
                && value.terminal.is_some()
        }
        PhaseV1::FencedUnknown => value.pending.is_some(),
        PhaseV1::Closed => {
            value.session_revision.is_some()
                && value.pending.is_none()
                && value.active_turn.is_none()
        }
    }
}

fn valid_fenced_operation(snapshot: &SnapshotV1, value: OperationV1) -> bool {
    if !nonzero_key(value.key()) {
        return false;
    }
    match value {
        OperationV1::Open { execution, .. } => {
            execution.0 != [0; 16] && snapshot.session_revision.is_none()
        }
        OperationV1::Prompt {
            turn,
            execution,
            prompt_digest,
            attempt,
            ..
        } => {
            turn.0 != [0; 16]
                && execution.0 != [0; 16]
                && prompt_digest != [0; 32]
                && attempt == snapshot.binding.attempt
                && snapshot.session_revision.is_some()
                && snapshot.active_turn.is_none()
                && snapshot.permission.is_none()
                && snapshot.terminal.is_none()
        }
        OperationV1::Advance { turn, .. } => snapshot.active_turn == Some(turn),
        OperationV1::DecidePermission {
            execution,
            request,
            option,
            authority,
            attempt,
            ..
        } => {
            execution.0 != [0; 16]
                && option.0 != [0; 16]
                && attempt == snapshot.binding.attempt
                && snapshot.permission.is_some_and(|permission| {
                    permission.request == request
                        && permission.capability_digest == authority.capability_digest
                        && permission.option_ids[..usize::from(permission.option_count)]
                            .contains(&option)
                })
                && authority.decision.0 != [0; 16]
                && authority.subject.0 != [0; 16]
                && authority.epoch != 0
        }
        OperationV1::Cancel { turn, .. } => snapshot.active_turn == Some(turn),
        OperationV1::Close { .. } => {
            snapshot.session_revision.is_some() && snapshot.active_turn.is_none()
        }
    }
}

fn valid_permission_options(value: PermissionEvidenceV1) -> bool {
    let count = usize::from(value.option_count);
    if count == 0 || count > MAX_PERMISSION_OPTIONS_V1 {
        return false;
    }
    for index in 0..MAX_PERMISSION_OPTIONS_V1 {
        let id = value.option_ids[index];
        if index < count {
            if id.0 == [0; 16] || value.option_ids[..index].contains(&id) {
                return false;
            }
        } else if id.0 != [0; 16] {
            return false;
        }
    }
    true
}

fn observation_matches_snapshot(snapshot: &SnapshotV1, value: ObservationV1) -> bool {
    snapshot.phase == value.phase
        && snapshot.session_revision == Some(value.session_revision)
        && snapshot.last_event_cursor == value.last_event_cursor
        && snapshot.profile_document_digest == Some(value.profile_document_digest)
        && snapshot.capabilities == value.capabilities
        && snapshot.active_turn == value.active_turn
        && snapshot.permission == value.permission
        && snapshot.terminal == value.terminal
}
