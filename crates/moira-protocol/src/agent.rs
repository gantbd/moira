//! Provider-neutral interactive-agent projection owned by Moira.
//!
//! Values are independent of Daimon's representation. Identifiers correlate
//! one governed participation; they confer no provider or workflow authority.

macro_rules! opaque_id {
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

opaque_id!(SessionIdV1);
opaque_id!(TurnIdV1);
opaque_id!(PermissionRequestIdV1);
opaque_id!(PermissionOptionIdV1);
opaque_id!(ExecutionIdV1);
opaque_id!(PrincipalIdV1);
opaque_id!(ProfileIdV1);
opaque_id!(IdempotencyKeyV1);
opaque_id!(ContextIdV1);
opaque_id!(ToolIdV1);

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct DigestV1(pub [u8; 32]);

impl std::fmt::Debug for DigestV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DigestV1(<opaque>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptV1 {
    pub task_key: [u8; 16],
    pub incarnation: u64,
    pub attempt: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionBudgetV1 {
    pub max_elapsed_millis: u64,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_context_bytes: usize,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionV1 {
    pub id: ExecutionIdV1,
    pub attempt: AttemptV1,
    pub principal: PrincipalIdV1,
    pub profile: ProfileIdV1,
    pub idempotency_key: IdempotencyKeyV1,
    pub context: Vec<ContextIdV1>,
    pub tools: Vec<ToolIdV1>,
    pub budget: ExecutionBudgetV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionBudgetV1 {
    pub max_elapsed_millis: u64,
    pub max_turns: u32,
    pub max_events: u64,
    pub max_event_bytes: usize,
    pub max_prompt_bytes: usize,
    pub max_permission_requests: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenRequestV1 {
    pub session: SessionIdV1,
    pub execution: ExecutionV1,
    pub workspace: [u8; 16],
    pub budget: SessionBudgetV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptRequestV1 {
    pub session: SessionIdV1,
    pub turn: TurnIdV1,
    pub expected_revision: u64,
    pub execution: ExecutionV1,
    pub prompt_digest: DigestV1,
    pub prompt: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionOptionKindV1 {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionOptionV1 {
    pub id: PermissionOptionIdV1,
    pub kind: PermissionOptionKindV1,
    pub label: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionRequestV1 {
    pub request: PermissionRequestIdV1,
    pub turn: TurnIdV1,
    pub capability: Vec<u8>,
    pub description: Vec<u8>,
    pub options: Vec<PermissionOptionV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionDecisionV1 {
    pub request: PermissionRequestIdV1,
    pub option: PermissionOptionIdV1,
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
pub enum PhaseV1 {
    Opening,
    Ready,
    TurnActive,
    AwaitingPermission,
    OutcomeUnknown,
    Closed,
    ReattachRequired,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventV1 {
    AssistantMessage {
        bytes: Vec<u8>,
    },
    Thought {
        bytes: Vec<u8>,
    },
    Plan {
        bytes: Vec<u8>,
    },
    ToolCall {
        title: Vec<u8>,
        detail: Vec<u8>,
    },
    Terminal {
        stream: Vec<u8>,
        bytes: Vec<u8>,
    },
    Permission(PermissionRequestV1),
    FileChange {
        path: Vec<u8>,
        content_digest: DigestV1,
    },
    FileRead {
        path: Vec<u8>,
        content_digest: DigestV1,
        content_bytes: usize,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    TurnFinished {
        turn: TurnIdV1,
        disposition: DispositionV1,
        stop_reason: StopReasonV1,
        output_digest: Option<DigestV1>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventRecordV1 {
    pub cursor: u64,
    pub event: EventV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionViewV1 {
    pub session: SessionIdV1,
    pub revision: u64,
    pub phase: PhaseV1,
    pub profile_document_digest: DigestV1,
    pub active_turn: Option<TurnIdV1>,
    pub last_event_cursor: Option<u64>,
    pub capabilities: CapabilitiesV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventPageV1 {
    pub session: SessionIdV1,
    pub entries: Vec<EventRecordV1>,
    pub next_after: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandV1 {
    Open {
        request: OpenRequestV1,
        now: u64,
    },
    Prompt {
        request: PromptRequestV1,
        now: u64,
    },
    Events {
        session: SessionIdV1,
        after: Option<u64>,
        limit: u64,
    },
    Inspect {
        session: SessionIdV1,
    },
    DecidePermission {
        session: SessionIdV1,
        expected_revision: u64,
        execution: ExecutionV1,
        decision: PermissionDecisionV1,
        now: u64,
    },
    Reattach {
        session: SessionIdV1,
        expected_revision: u64,
        execution: ExecutionV1,
        now: u64,
    },
    Advance {
        session: SessionIdV1,
        turn: TurnIdV1,
        expected_revision: u64,
        idempotency_key: IdempotencyKeyV1,
        now: u64,
    },
    Cancel {
        session: SessionIdV1,
        turn: TurnIdV1,
        expected_revision: u64,
        idempotency_key: IdempotencyKeyV1,
        now: u64,
    },
    Close {
        session: SessionIdV1,
        expected_revision: u64,
        idempotency_key: IdempotencyKeyV1,
        now: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutcomeV1 {
    Session(SessionViewV1),
    Events(EventPageV1),
    OutcomeUnknown {
        session: SessionIdV1,
        idempotency_key: IdempotencyKeyV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
    NotAuthorized,
    UnknownServiceSession,
    PrincipalMismatch,
    UnknownAgentSession,
    UnknownTurn,
    UnknownPermissionRequest,
    UnknownPermissionOption,
    RevisionConflict,
    AuthorityDenied,
    AuthorityExpired,
    AttemptDenied,
    IdempotencyConflict,
    SessionConflict,
    TurnConflict,
    ProviderReceiptMismatch,
    ProviderReceiptConflict,
    Busy,
    Capacity,
    Limit,
    Unsupported,
    InvalidInput,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplyV1 {
    Accepted(OutcomeV1),
    Refused(RefusalV1),
}
