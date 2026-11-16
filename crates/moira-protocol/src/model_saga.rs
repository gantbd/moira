//! Transport-neutral public contract for Moira-owned model proposal sagas.

use crate::{EffectIdV1, WorkflowIdV1};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalV1 {
    AgoraMessage {
        room: [u8; 16],
        reply_to: [u8; 16],
        body: Vec<u8>,
    },
    VellumEdit {
        workspace: [u8; 16],
        document: [u8; 16],
        expected_revision: u64,
        expected_extent: u64,
        replacement: Vec<u8>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerReceiptV1 {
    AgoraMessage {
        effect: EffectIdV1,
        room: [u8; 16],
        reply_to: [u8; 16],
        message: [u8; 16],
        agora_revision: u64,
    },
    VellumEdit {
        effect: EffectIdV1,
        workspace: [u8; 16],
        document: [u8; 16],
        source_revision: u64,
        durable: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationV1 {
    Admit {
        workflow: WorkflowIdV1,
        daimon_effect: EffectIdV1,
        selection_digest: [u8; 32],
        proposals: Vec<ProposalV1>,
    },
    Accept {
        workflow: WorkflowIdV1,
        receipt: PeerReceiptV1,
    },
    Fail {
        workflow: WorkflowIdV1,
        effect: EffectIdV1,
    },
    MarkUncertain {
        workflow: WorkflowIdV1,
        effect: EffectIdV1,
    },
    Reconcile {
        workflow: WorkflowIdV1,
        receipt: PeerReceiptV1,
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
pub enum EffectStateV1 {
    Queued,
    Pending,
    Succeeded(PeerReceiptV1),
    Failed,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseV1 {
    Pending,
    FencedUncertain { effect: EffectIdV1 },
    Completed,
    Failed { effect: EffectIdV1 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalEffectV1 {
    pub effect: EffectIdV1,
    pub proposal: ProposalV1,
    pub state: EffectStateV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SagaViewV1 {
    pub workflow: WorkflowIdV1,
    pub daimon_effect: EffectIdV1,
    pub selection_digest: [u8; 32],
    pub phase: PhaseV1,
    pub effects: Vec<ProposalEffectV1>,
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
    InvalidSelection,
    Limit,
    Conflict,
    UnknownSaga,
    UnknownEffect,
    NotPending,
    ReceiptMismatch,
    IdentityExhausted,
    InvalidCheckpoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplyV1 {
    Accepted(OutcomeV1),
    Refused(RefusalV1),
}
