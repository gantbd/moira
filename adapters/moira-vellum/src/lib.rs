//! Submits one Moira-owned edit effect through Vellum's durable public client.
//!
//! The model proposes opaque deployment references and replacement bytes.
//! Trusted Moira composition resolves those references and supplies the exact
//! source extent; the model never selects a Vellum identity or byte range.

#![forbid(unsafe_code)]

use moira_protocol::{EffectIdV1, deliberation::HumanGateMaterialRefV1};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use vellum_client::Client;
use vellum_protocol::{
    DurableEffectIntentV1, DurableEffectReceiptV1, DurableExecuteOutcomeV1, DurableOperationRefV1,
    DurableReceiptLookupOutcomeV1, DurableReplaceAndSaveReceiptV1, OutcomeV1, RefusalV1,
    ServiceAccessErrorV1, ServiceControlErrorV1, ServicePort, WorkspaceDocumentIdV1, WorkspaceIdV1,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExternalDocumentRefV1 {
    pub workspace: [u8; 16],
    pub document: [u8; 16],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DocumentRouteV1 {
    pub workspace: WorkspaceIdV1,
    pub document: WorkspaceDocumentIdV1,
}

/// One externally stable decision-packet identity bound to the exact Vellum
/// revision and UTF-8 bytes a human must review. The route remains private to
/// trusted composition; Moira need not import Vellum workspace identities.
pub type DecisionPacketRefV1 = HumanGateMaterialRefV1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionPacketV1 {
    pub reference: DecisionPacketRefV1,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionPacketErrorV1 {
    InvalidInput,
    UnknownTarget,
    InvalidRoute,
    Access(ServiceAccessErrorV1),
    Refused(RefusalV1),
    UnexpectedReply,
    RevisionMismatch { expected: u64, actual: u64 },
    DigestMismatch,
    PacketLimit,
}

/// Read-only materialization of one exact human decision packet. Selection is
/// scoped to this adapter's Vellum client view and grants no edit or decision
/// authority.
pub struct DecisionPacketAdapter<P> {
    client: Client<P>,
    routes: BTreeMap<ExternalDocumentRefV1, DocumentRouteV1>,
    max_packet_bytes: usize,
}

impl<P: ServicePort> DecisionPacketAdapter<P> {
    pub fn connect(
        port: P,
        routes: BTreeMap<ExternalDocumentRefV1, DocumentRouteV1>,
        max_packet_bytes: usize,
    ) -> Result<Self, ServiceControlErrorV1> {
        Ok(Self {
            client: Client::connect(port)?,
            routes,
            max_packet_bytes,
        })
    }

    pub fn read(
        &mut self,
        reference: DecisionPacketRefV1,
    ) -> Result<DecisionPacketV1, DecisionPacketErrorV1> {
        if reference.expected_revision == 0 || self.max_packet_bytes == 0 {
            return Err(DecisionPacketErrorV1::InvalidInput);
        }
        let target = ExternalDocumentRefV1 {
            workspace: reference.owner_route,
            document: reference.artifact_route,
        };
        let route = self
            .routes
            .get(&target)
            .copied()
            .ok_or(DecisionPacketErrorV1::UnknownTarget)?;
        if route.document.workspace() != route.workspace {
            return Err(DecisionPacketErrorV1::InvalidRoute);
        }
        accepted(self.client.open_view())?;
        match accepted(self.client.select_document(route.document))? {
            OutcomeV1::DocumentSelected(document) if document == route.document => {}
            _ => return Err(DecisionPacketErrorV1::UnexpectedReply),
        }
        let OutcomeV1::Snapshot(snapshot) = accepted(self.client.snapshot())? else {
            return Err(DecisionPacketErrorV1::UnexpectedReply);
        };
        if snapshot.revision != reference.expected_revision {
            return Err(DecisionPacketErrorV1::RevisionMismatch {
                expected: reference.expected_revision,
                actual: snapshot.revision,
            });
        }
        if snapshot.text.len() > self.max_packet_bytes {
            return Err(DecisionPacketErrorV1::PacketLimit);
        }
        if Sha256::digest(snapshot.text.as_bytes()).as_slice() != reference.expected_sha256 {
            return Err(DecisionPacketErrorV1::DigestMismatch);
        }
        Ok(DecisionPacketV1 {
            reference,
            text: snapshot.text.to_string(),
        })
    }
}

fn accepted(
    reply: Result<vellum_protocol::ReplyV1, ServiceAccessErrorV1>,
) -> Result<OutcomeV1, DecisionPacketErrorV1> {
    match reply.map_err(DecisionPacketErrorV1::Access)? {
        vellum_protocol::ReplyV1::Accepted(outcome) => Ok(outcome),
        vellum_protocol::ReplyV1::Refused(refusal) => Err(DecisionPacketErrorV1::Refused(refusal)),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditIntentV1 {
    pub effect: EffectIdV1,
    pub target: ExternalDocumentRefV1,
    pub expected_revision: u64,
    /// Exact byte extent retained with the source context supplied to Daimon.
    pub expected_extent: u64,
    pub replacement: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditReplyV1 {
    Completed {
        effect: EffectIdV1,
        receipt: DurableReplaceAndSaveReceiptV1,
    },
    Refused(RefusalV1),
    Retired,
    OutcomeUnknown,
    AbsentOrForeign,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdapterErrorV1 {
    InvalidInput,
    UnknownTarget,
    InvalidRoute,
    Access(ServiceAccessErrorV1),
    UnexpectedReceipt,
    RequestConflict,
    Unavailable,
}

pub struct EditAdapter<P> {
    client: Client<P>,
    routes: BTreeMap<ExternalDocumentRefV1, DocumentRouteV1>,
    max_replacement_bytes: usize,
}

impl<P: ServicePort> EditAdapter<P> {
    pub fn connect(
        port: P,
        routes: BTreeMap<ExternalDocumentRefV1, DocumentRouteV1>,
        max_replacement_bytes: usize,
    ) -> Result<Self, ServiceControlErrorV1> {
        Ok(Self {
            client: Client::connect(port)?,
            routes,
            max_replacement_bytes,
        })
    }

    pub fn execute(&mut self, intent: &EditIntentV1) -> Result<EditReplyV1, AdapterErrorV1> {
        let (operation, vellum_intent, document) = self.prepare(intent)?;
        let outcome = match self.client.execute_durable(operation, vellum_intent) {
            Ok(outcome) => outcome,
            Err(ServiceAccessErrorV1::OutcomeUnknown) => return Ok(EditReplyV1::OutcomeUnknown),
            Err(error) => return Err(AdapterErrorV1::Access(error)),
        };
        map_outcome(intent.effect, document, outcome)
    }

    /// Historical reconciliation only. Absence cannot fall through to execute.
    pub fn lookup(&mut self, intent: &EditIntentV1) -> Result<EditReplyV1, AdapterErrorV1> {
        let (operation, vellum_intent, document) = self.prepare(intent)?;
        match self
            .client
            .lookup_durable_receipt(operation, vellum_intent)
            .map_err(AdapterErrorV1::Access)?
        {
            DurableReceiptLookupOutcomeV1::Complete(outcome) => {
                map_outcome(intent.effect, document, outcome)
            }
            DurableReceiptLookupOutcomeV1::RequestConflict => Err(AdapterErrorV1::RequestConflict),
            DurableReceiptLookupOutcomeV1::OutcomeUnknown => Ok(EditReplyV1::OutcomeUnknown),
            DurableReceiptLookupOutcomeV1::AbsentOrForeign => Ok(EditReplyV1::AbsentOrForeign),
            DurableReceiptLookupOutcomeV1::Unavailable => Err(AdapterErrorV1::Unavailable),
        }
    }

    fn prepare(
        &self,
        intent: &EditIntentV1,
    ) -> Result<
        (
            DurableOperationRefV1,
            DurableEffectIntentV1,
            WorkspaceDocumentIdV1,
        ),
        AdapterErrorV1,
    > {
        if intent.expected_revision == 0
            || intent.replacement.is_empty()
            || intent.replacement.len() > self.max_replacement_bytes
        {
            return Err(AdapterErrorV1::InvalidInput);
        }
        let operation =
            DurableOperationRefV1::new(intent.effect.0).ok_or(AdapterErrorV1::InvalidInput)?;
        let route = self
            .routes
            .get(&intent.target)
            .copied()
            .ok_or(AdapterErrorV1::UnknownTarget)?;
        if route.document.workspace() != route.workspace {
            return Err(AdapterErrorV1::InvalidRoute);
        }
        let replacement = String::from_utf8(intent.replacement.clone())
            .map_err(|_| AdapterErrorV1::InvalidInput)?;
        Ok((
            operation,
            DurableEffectIntentV1::ReplaceDocumentAndSave {
                document: route.document,
                expected_revision: intent.expected_revision,
                start: 0,
                end: intent.expected_extent,
                inserted: replacement,
            },
            route.document,
        ))
    }
}

fn map_outcome(
    effect: EffectIdV1,
    document: WorkspaceDocumentIdV1,
    outcome: DurableExecuteOutcomeV1,
) -> Result<EditReplyV1, AdapterErrorV1> {
    match outcome {
        DurableExecuteOutcomeV1::Completed(DurableEffectReceiptV1::DocumentReplacedAndSaved(
            receipt,
        )) if receipt.document == document => Ok(EditReplyV1::Completed { effect, receipt }),
        DurableExecuteOutcomeV1::Completed(_) => Err(AdapterErrorV1::UnexpectedReceipt),
        DurableExecuteOutcomeV1::Refused(refusal) => Ok(EditReplyV1::Refused(refusal)),
        DurableExecuteOutcomeV1::Retired => Ok(EditReplyV1::Retired),
    }
}
