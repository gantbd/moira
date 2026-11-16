//! Read-only correlation profile with exact Chronos scope and attempt.
use super::{DispositionV1, WorkflowIdV1};

pub const MAX_SCOPE_BYTES: usize = 256;
pub const MAX_TASK_BYTES: usize = 256;

pub struct AttemptLookupRequestV1 {
    pub service_scope: String,
    pub task: String,
    pub incarnation: u64,
    pub attempt: u32,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ExecutionEvidence {
    ReceiptPending,
    Recorded {
        execution_id: [u8; 16],
        disposition: DispositionV1,
    },
}

impl std::fmt::Debug for ExecutionEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReceiptPending => f.write_str("ReceiptPending"),
            Self::Recorded { disposition, .. } => f
                .debug_struct("Recorded")
                .field("execution_id", &"<opaque>")
                .field("disposition", disposition)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptLookup {
    Denied,
    Unavailable,
    Unresolved,
    Found {
        workflow: WorkflowIdV1,
        execution: ExecutionEvidence,
    },
}
