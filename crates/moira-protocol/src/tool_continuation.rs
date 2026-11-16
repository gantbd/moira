//! Representation-independent contract for structured tool continuations.

use crate::{EffectIdV1, WorkflowIdV1};

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

id!(ToolRequestIdV1);
id!(ToolCapabilityIdV1);
id!(ToolExecutionIdV1);
id!(AuthorityDecisionIdV1);
id!(AuthoritySubjectIdV1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolBudgetV1 {
    pub max_elapsed_millis: u64,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCallV1 {
    pub request: ToolRequestIdV1,
    pub capability: ToolCapabilityIdV1,
    pub provider_correlation: Vec<u8>,
    pub arguments: Vec<u8>,
    pub budget: ToolBudgetV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorityEvidenceV1 {
    pub decision: AuthorityDecisionIdV1,
    pub subject: AuthoritySubjectIdV1,
    pub capability: ToolCapabilityIdV1,
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptEvidenceV1 {
    pub task_key: [u8; 16],
    pub incarnation: u64,
    pub attempt: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolGateV1 {
    pub authority: AuthorityEvidenceV1,
    pub attempt: AttemptEvidenceV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolReceiptV1 {
    pub effect: EffectIdV1,
    pub capability: ToolCapabilityIdV1,
    pub execution: ToolExecutionIdV1,
    pub evidence_digest: [u8; 32],
    pub output_digest: [u8; 32],
    pub output_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolFailureV1 {
    pub effect: EffectIdV1,
    pub capability: ToolCapabilityIdV1,
    pub execution: ToolExecutionIdV1,
    pub evidence_digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationV1 {
    Admit {
        workflow: WorkflowIdV1,
        source_execution: EffectIdV1,
        source_attempt: AttemptEvidenceV1,
        calls: Vec<ToolCallV1>,
    },
    Authorize {
        workflow: WorkflowIdV1,
        effect: EffectIdV1,
        gate: ToolGateV1,
    },
    Dispatch {
        workflow: WorkflowIdV1,
        effect: EffectIdV1,
        execution: ToolExecutionIdV1,
    },
    Accept {
        workflow: WorkflowIdV1,
        receipt: ToolReceiptV1,
    },
    Fail {
        workflow: WorkflowIdV1,
        failure: ToolFailureV1,
    },
    MarkUncertain {
        workflow: WorkflowIdV1,
        effect: EffectIdV1,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandV1 {
    Apply {
        expected_storage_revision: Option<u64>,
        mutation: MutationV1,
    },
    Status,
    Saga {
        workflow: WorkflowIdV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolEffectStateV1 {
    Queued,
    PendingAuthorization,
    Ready(ToolGateV1),
    InFlight {
        gate: ToolGateV1,
        execution: ToolExecutionIdV1,
    },
    Succeeded {
        gate: ToolGateV1,
        receipt: ToolReceiptV1,
    },
    Failed {
        gate: ToolGateV1,
        failure: ToolFailureV1,
    },
    Uncertain {
        gate: ToolGateV1,
        execution: ToolExecutionIdV1,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolEffectV1 {
    pub effect: EffectIdV1,
    pub call: ToolCallV1,
    pub state: ToolEffectStateV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseV1 {
    Pending,
    FencedUncertain { effect: EffectIdV1 },
    Completed,
    Failed { effect: EffectIdV1 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SagaViewV1 {
    pub workflow: WorkflowIdV1,
    pub source_execution: EffectIdV1,
    pub source_attempt: AttemptEvidenceV1,
    pub phase: PhaseV1,
    pub effects: Vec<ToolEffectV1>,
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
        saga: SagaViewV1,
        replayed: bool,
    },
    Status(StatusV1),
    Saga(Option<SagaViewV1>),
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
    Conflict,
    DuplicateRequest,
    UnknownSaga,
    UnknownEffect,
    NotPendingAuthorization,
    NotReady,
    NotInFlight,
    InvalidEvidence,
    ReceiptMismatch,
    IdentityExhausted,
    InvalidCheckpoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplyV1 {
    Accepted(OutcomeV1),
    Refused(RefusalV1),
}
