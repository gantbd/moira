//! Aegis authorization through its public session-bound client contract.
//!
//! Grant handles never leave this adapter. Decision handles are retained by
//! Moira only as attribution and confer no authority after the call returns.

#![forbid(unsafe_code)]

use aegis_client::{AuthorityClient, ServicePort};
use aegis_protocol::{ActionV1, EngineNamespaceV1, GrantHandleV1, OutcomeV1, RefusalV1, ReplyV1};
use moira_core::deliberation::{
    AuthorityDecisionIdV1, AuthorityRefV1, AuthoritySubjectIdV1, CandidateIdV1, DeliberationIdV1,
};
use moira_host::{
    ActionSelectionDecisionV1, ActionSelectionSubjectV1, ActionSelectorV1, ChoiceAuthorityV1,
    ChoiceDecisionV1, ChoiceRequestV1, SagaStore,
};
use moira_protocol::tool_continuation::{
    AuthorityDecisionIdV1 as ToolAuthorityDecisionIdV1,
    AuthorityEvidenceV1 as ToolAuthorityEvidenceV1,
    AuthoritySubjectIdV1 as ToolAuthoritySubjectIdV1, ToolCapabilityIdV1,
};
use moira_protocol::{
    OperatorOperationV1, SagaCommandV1, SagaReplyV1, agent_participation as participation,
    deliberation as deliberation_protocol,
};
use moira_service::{
    agent_participation::{PermissionAccessV1, PermissionAuthorizer},
    deliberation::{HumanGateAccessV1, HumanGateAuthorizer},
    saga::{OperatorAccessV1, OperatorAuthorizer, SagaService},
};
use sha2::{Digest, Sha256};

#[derive(Debug, Eq, PartialEq)]
pub enum ReconcileError<AuthorityError, MoiraError> {
    InvalidCommand,
    AuthorityRefused(RefusalV1),
    UnexpectedAuthorityReply,
    AuthorityTransport(AuthorityError),
    MoiraBackend(MoiraError),
}

#[derive(Debug, Eq, PartialEq)]
pub enum OperatorAuthorizationError<AuthorityError> {
    UnexpectedAuthorityReply,
    AuthorityTransport(AuthorityError),
}

#[derive(Debug, Eq, PartialEq)]
pub enum ChoiceAuthorizationError<AuthorityError> {
    AuthorityRefused(RefusalV1),
    UnexpectedAuthorityReply,
    AuthorityTransport(AuthorityError),
}

#[derive(Debug, Eq, PartialEq)]
pub enum ToolAuthorizationError<AuthorityError> {
    AuthorityRefused(RefusalV1),
    UnexpectedAuthorityReply,
    AuthorityTransport(AuthorityError),
}

#[derive(Debug, Eq, PartialEq)]
pub enum PermissionAuthorizationError<AuthorityError> {
    UnexpectedAuthorityReply,
    AuthorityTransport(AuthorityError),
}

#[derive(Debug, Eq, PartialEq)]
pub enum HumanGateAuthorizationError<AuthorityError> {
    UnexpectedAuthorityReply,
    AuthorityTransport(AuthorityError),
}

/// Explicit time input for current Aegis decisions. The adapter owns no clock
/// and does not infer time from a connection or operation identity.
pub trait TimeSourceV1 {
    fn now(&mut self) -> u64;
}

/// Current Aegis authority for one exact provider-advertised capability.
/// The grant remains session-local; returned evidence is attribution for the
/// Moira continuation gate and is not reusable bearer authority.
pub struct AegisToolAuthorityV1<P, T> {
    authority: AuthorityClient<P>,
    grant: GrantHandleV1,
    subject: [u8; 16],
    time: T,
}

impl<P, T> AegisToolAuthorityV1<P, T> {
    pub const fn new(
        authority: AuthorityClient<P>,
        grant: GrantHandleV1,
        subject: [u8; 16],
        time: T,
    ) -> Self {
        Self {
            authority,
            grant,
            subject,
            time,
        }
    }
}

impl<P: ServicePort, T> AegisToolAuthorityV1<P, T> {
    pub fn into_parts(self) -> (P, T) {
        (self.authority.into_port(), self.time)
    }
}

impl<P: ServicePort, T: TimeSourceV1> AegisToolAuthorityV1<P, T> {
    pub fn authorize(
        &mut self,
        capability: ToolCapabilityIdV1,
    ) -> Result<ToolAuthorityEvidenceV1, ToolAuthorizationError<P::Error>> {
        let now = self.time.now();
        let reply = self
            .authority
            .authorize(
                self.grant,
                now,
                EngineNamespaceV1::Daimon,
                capability.0,
                ActionV1::Execute,
            )
            .map_err(ToolAuthorizationError::AuthorityTransport)?;
        let ReplyV1::Accepted(OutcomeV1::Authorized(decision)) = reply else {
            return match reply {
                ReplyV1::Refused(refusal) => Err(ToolAuthorizationError::AuthorityRefused(refusal)),
                ReplyV1::Accepted(_) => Err(ToolAuthorizationError::UnexpectedAuthorityReply),
            };
        };
        if decision.engine != EngineNamespaceV1::Daimon
            || decision.action != ActionV1::Execute
            || decision.expires_at < now
        {
            return Err(ToolAuthorizationError::UnexpectedAuthorityReply);
        }
        Ok(ToolAuthorityEvidenceV1 {
            decision: ToolAuthorityDecisionIdV1(aegis_decision_id(decision.handle.0)),
            subject: ToolAuthoritySubjectIdV1(self.subject),
            capability,
            epoch: decision.epoch,
        })
    }
}

/// One explicit candidate selection whose subject must hold a current Aegis
/// `Administer` decision for the exact Moira deliberation resource.
pub struct AegisChoiceAuthorityV1<P, T> {
    authority: AuthorityClient<P>,
    grant: GrantHandleV1,
    subject: [u8; 16],
    selected: CandidateIdV1,
    time: T,
}

/// Current Aegis approval for one exact retained report/result selection.
/// Candidate choice remains a separate decision and is not inferred here.
pub struct AegisActionSelectorV1<P, T> {
    authority: AuthorityClient<P>,
    grant: GrantHandleV1,
    subject: [u8; 16],
    time: T,
}

impl<P, T> AegisActionSelectorV1<P, T> {
    pub const fn new(
        authority: AuthorityClient<P>,
        grant: GrantHandleV1,
        subject: [u8; 16],
        time: T,
    ) -> Self {
        Self {
            authority,
            grant,
            subject,
            time,
        }
    }
}

impl<P: ServicePort, T> AegisActionSelectorV1<P, T> {
    pub fn into_parts(self) -> (P, T) {
        (self.authority.into_port(), self.time)
    }
}

impl<P: ServicePort, T: TimeSourceV1> ActionSelectorV1 for AegisActionSelectorV1<P, T> {
    type Error = ChoiceAuthorizationError<P::Error>;

    fn select(
        &mut self,
        subject: ActionSelectionSubjectV1,
    ) -> Result<ActionSelectionDecisionV1, Self::Error> {
        let now = self.time.now();
        let reply = self
            .authority
            .authorize(
                self.grant,
                now,
                EngineNamespaceV1::Moira,
                choice_resource(subject.deliberation),
                ActionV1::Administer,
            )
            .map_err(ChoiceAuthorizationError::AuthorityTransport)?;
        let ReplyV1::Accepted(OutcomeV1::Authorized(decision)) = reply else {
            return match reply {
                ReplyV1::Refused(refusal) => {
                    Err(ChoiceAuthorizationError::AuthorityRefused(refusal))
                }
                ReplyV1::Accepted(_) => Err(ChoiceAuthorizationError::UnexpectedAuthorityReply),
            };
        };
        if decision.engine != EngineNamespaceV1::Moira
            || decision.action != ActionV1::Administer
            || decision.expires_at < now
        {
            return Err(ChoiceAuthorizationError::UnexpectedAuthorityReply);
        }
        Ok(ActionSelectionDecisionV1 {
            subject,
            authority: AuthorityRefV1 {
                decision: AuthorityDecisionIdV1(aegis_decision_id(decision.handle.0)),
                subject: AuthoritySubjectIdV1(self.subject),
                epoch: decision.epoch,
            },
        })
    }
}

impl<P, T> AegisChoiceAuthorityV1<P, T> {
    pub const fn new(
        authority: AuthorityClient<P>,
        grant: GrantHandleV1,
        subject: [u8; 16],
        selected: CandidateIdV1,
        time: T,
    ) -> Self {
        Self {
            authority,
            grant,
            subject,
            selected,
            time,
        }
    }
}

impl<P: ServicePort, T> AegisChoiceAuthorityV1<P, T> {
    pub fn into_parts(self) -> (P, T) {
        (self.authority.into_port(), self.time)
    }
}

impl<P: ServicePort, T: TimeSourceV1> ChoiceAuthorityV1 for AegisChoiceAuthorityV1<P, T> {
    type Error = ChoiceAuthorizationError<P::Error>;

    fn choose(&mut self, request: ChoiceRequestV1) -> Result<ChoiceDecisionV1, Self::Error> {
        let now = self.time.now();
        let reply = self
            .authority
            .authorize(
                self.grant,
                now,
                EngineNamespaceV1::Moira,
                choice_resource(request.subject.deliberation),
                ActionV1::Administer,
            )
            .map_err(ChoiceAuthorizationError::AuthorityTransport)?;
        let ReplyV1::Accepted(OutcomeV1::Authorized(decision)) = reply else {
            return match reply {
                ReplyV1::Refused(refusal) => {
                    Err(ChoiceAuthorizationError::AuthorityRefused(refusal))
                }
                ReplyV1::Accepted(_) => Err(ChoiceAuthorizationError::UnexpectedAuthorityReply),
            };
        };
        if decision.engine != EngineNamespaceV1::Moira
            || decision.action != ActionV1::Administer
            || decision.expires_at < now
        {
            return Err(ChoiceAuthorizationError::UnexpectedAuthorityReply);
        }
        Ok(ChoiceDecisionV1 {
            subject: request.subject,
            authority: AuthorityRefV1 {
                decision: AuthorityDecisionIdV1(aegis_decision_id(decision.handle.0)),
                subject: AuthoritySubjectIdV1(self.subject),
                epoch: decision.epoch,
            },
            selected: self.selected,
        })
    }
}

/// Command-scoped Aegis authorization for the Moira saga service port.
/// Connection authentication is deliberately absent from this value.
pub struct AegisOperatorAuthorizerV1<P, T> {
    authority: AuthorityClient<P>,
    grant: GrantHandleV1,
    subject: [u8; 16],
    time: T,
}

impl<P, T> AegisOperatorAuthorizerV1<P, T> {
    pub const fn new(
        authority: AuthorityClient<P>,
        grant: GrantHandleV1,
        subject: [u8; 16],
        time: T,
    ) -> Self {
        Self {
            authority,
            grant,
            subject,
            time,
        }
    }
}

impl<P: ServicePort, T> AegisOperatorAuthorizerV1<P, T> {
    pub fn into_parts(self) -> (P, T) {
        (self.authority.into_port(), self.time)
    }
}

impl<P: ServicePort, T: TimeSourceV1> OperatorAuthorizer for AegisOperatorAuthorizerV1<P, T> {
    type Error = OperatorAuthorizationError<P::Error>;

    fn authorize_operator(
        &mut self,
        operation: OperatorOperationV1,
    ) -> Result<Option<OperatorAccessV1>, Self::Error> {
        let now = self.time.now();
        let reply = self
            .authority
            .authorize(
                self.grant,
                now,
                EngineNamespaceV1::Moira,
                saga_resource(operation.saga.0),
                ActionV1::Administer,
            )
            .map_err(OperatorAuthorizationError::AuthorityTransport)?;
        let ReplyV1::Accepted(OutcomeV1::Authorized(decision)) = reply else {
            return match reply {
                ReplyV1::Refused(_) => Ok(None),
                ReplyV1::Accepted(_) => Err(OperatorAuthorizationError::UnexpectedAuthorityReply),
            };
        };
        if decision.engine != EngineNamespaceV1::Moira
            || decision.action != ActionV1::Administer
            || decision.expires_at < now
        {
            return Err(OperatorAuthorizationError::UnexpectedAuthorityReply);
        }
        Ok(Some(OperatorAccessV1 {
            subject: self.subject,
            authority_decision: decision.handle.0,
            authority_epoch: decision.epoch,
        }))
    }
}

/// Command-scoped Aegis authorization for one exact provider permission
/// selection. The public capability digest remains an untrusted claim until
/// the participation engine compares it with retained provider evidence.
pub struct AegisPermissionAuthorizerV1<P, T> {
    authority: AuthorityClient<P>,
    grant: GrantHandleV1,
    subject: [u8; 16],
    time: T,
}

impl<P, T> AegisPermissionAuthorizerV1<P, T> {
    pub const fn new(
        authority: AuthorityClient<P>,
        grant: GrantHandleV1,
        subject: [u8; 16],
        time: T,
    ) -> Self {
        Self {
            authority,
            grant,
            subject,
            time,
        }
    }
}

impl<P: ServicePort, T> AegisPermissionAuthorizerV1<P, T> {
    pub fn into_parts(self) -> (P, T) {
        (self.authority.into_port(), self.time)
    }
}

impl<P: ServicePort, T: TimeSourceV1> PermissionAuthorizer for AegisPermissionAuthorizerV1<P, T> {
    type Error = PermissionAuthorizationError<P::Error>;

    fn authorize_permission(
        &mut self,
        selection: participation::PermissionSelectionV1,
    ) -> Result<Option<PermissionAccessV1>, Self::Error> {
        let now = self.time.now();
        let reply = self
            .authority
            .authorize(
                self.grant,
                now,
                EngineNamespaceV1::Moira,
                permission_resource(
                    selection.participation,
                    selection.execution,
                    selection.request,
                    selection.option,
                    selection.capability_digest,
                ),
                ActionV1::Execute,
            )
            .map_err(PermissionAuthorizationError::AuthorityTransport)?;
        let ReplyV1::Accepted(OutcomeV1::Authorized(decision)) = reply else {
            return match reply {
                ReplyV1::Refused(_) => Ok(None),
                ReplyV1::Accepted(_) => Err(PermissionAuthorizationError::UnexpectedAuthorityReply),
            };
        };
        if decision.engine != EngineNamespaceV1::Moira
            || decision.action != ActionV1::Execute
            || decision.expires_at < now
        {
            return Err(PermissionAuthorizationError::UnexpectedAuthorityReply);
        }
        Ok(Some(PermissionAccessV1 {
            decision: aegis_decision_id(decision.handle.0),
            subject: self.subject,
            capability_digest: selection.capability_digest,
            epoch: decision.epoch,
        }))
    }
}

/// Command-scoped Aegis authorization for one exact retained human-gate
/// request. The response disposition remains the human's choice; the grant
/// only proves that the named subject may administer this exact gate.
pub struct AegisHumanGateAuthorizerV1<P, T> {
    authority: AuthorityClient<P>,
    grant: GrantHandleV1,
    subject: [u8; 16],
    time: T,
}

impl<P, T> AegisHumanGateAuthorizerV1<P, T> {
    pub const fn new(
        authority: AuthorityClient<P>,
        grant: GrantHandleV1,
        subject: [u8; 16],
        time: T,
    ) -> Self {
        Self {
            authority,
            grant,
            subject,
            time,
        }
    }
}

impl<P: ServicePort, T> AegisHumanGateAuthorizerV1<P, T> {
    pub fn into_parts(self) -> (P, T) {
        (self.authority.into_port(), self.time)
    }
}

impl<P: ServicePort, T: TimeSourceV1> HumanGateAuthorizer for AegisHumanGateAuthorizerV1<P, T> {
    type Error = HumanGateAuthorizationError<P::Error>;

    fn authorize_human_gate(
        &mut self,
        operation: deliberation_protocol::HumanGateOperationV1,
    ) -> Result<Option<HumanGateAccessV1>, Self::Error> {
        let now = self.time.now();
        let reply = self
            .authority
            .authorize(
                self.grant,
                now,
                EngineNamespaceV1::Moira,
                human_gate_resource(operation.deliberation, operation.response.request),
                ActionV1::Administer,
            )
            .map_err(HumanGateAuthorizationError::AuthorityTransport)?;
        let ReplyV1::Accepted(OutcomeV1::Authorized(decision)) = reply else {
            return match reply {
                ReplyV1::Refused(_) => Ok(None),
                ReplyV1::Accepted(_) => Err(HumanGateAuthorizationError::UnexpectedAuthorityReply),
            };
        };
        if decision.engine != EngineNamespaceV1::Moira
            || decision.action != ActionV1::Administer
            || decision.expires_at < now
        {
            return Err(HumanGateAuthorizationError::UnexpectedAuthorityReply);
        }
        Ok(Some(HumanGateAccessV1 {
            decision: aegis_decision_id(decision.handle.0),
            subject: self.subject,
            epoch: decision.epoch,
        }))
    }
}

/// One authenticated Aegis session and one session-owned grant. Constructing
/// this value grants no authority; Aegis validates every operation.
pub struct OperatorAdapter<P> {
    authority: AuthorityClient<P>,
    grant: GrantHandleV1,
    subject: [u8; 16],
}

impl<P: ServicePort> OperatorAdapter<P> {
    pub const fn new(
        authority: AuthorityClient<P>,
        grant: GrantHandleV1,
        subject: [u8; 16],
    ) -> Self {
        Self {
            authority,
            grant,
            subject,
        }
    }

    pub fn reconcile<S: SagaStore>(
        &mut self,
        service: &mut SagaService<S>,
        now: u64,
        command: SagaCommandV1,
    ) -> Result<SagaReplyV1, ReconcileError<P::Error, S::Error>> {
        let SagaCommandV1::Reconcile { operation, .. } = command else {
            return Err(ReconcileError::InvalidCommand);
        };
        let reply = self
            .authority
            .authorize(
                self.grant,
                now,
                EngineNamespaceV1::Moira,
                saga_resource(operation.saga.0),
                ActionV1::Administer,
            )
            .map_err(ReconcileError::AuthorityTransport)?;
        let ReplyV1::Accepted(OutcomeV1::Authorized(decision)) = reply else {
            return match reply {
                ReplyV1::Refused(refusal) => Err(ReconcileError::AuthorityRefused(refusal)),
                ReplyV1::Accepted(_) => Err(ReconcileError::UnexpectedAuthorityReply),
            };
        };
        if decision.engine != EngineNamespaceV1::Moira
            || decision.action != ActionV1::Administer
            || decision.expires_at < now
        {
            return Err(ReconcileError::UnexpectedAuthorityReply);
        }
        service
            .execute_operator_authorized(
                OperatorAccessV1 {
                    subject: self.subject,
                    authority_decision: decision.handle.0,
                    authority_epoch: decision.epoch,
                },
                command,
            )
            .map_err(ReconcileError::MoiraBackend)
    }

    pub fn into_port(self) -> P {
        self.authority.into_port()
    }
}

/// Stable V1 resource mapping: eight-byte domain tag followed by the saga ID in
/// network order. It is correlation and scope input, never authority itself.
pub const fn saga_resource(saga: u64) -> [u8; 16] {
    let id = saga.to_be_bytes();
    [
        b'M', b'O', b'I', b'R', b'A', b'S', b'G', b'1', id[0], id[1], id[2], id[3], id[4], id[5],
        id[6], id[7],
    ]
}

/// Stable domain-separated resource for one methodical deliberation. Candidate
/// and checkpoint identities remain request evidence, not authority scope.
pub fn choice_resource(deliberation: DeliberationIdV1) -> [u8; 16] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRACH1");
    hash.update(deliberation.0);
    let digest = hash.finalize();
    let mut resource = [0_u8; 16];
    resource.copy_from_slice(&digest[..16]);
    resource
}

/// Stable domain-separated scope for one exact human-gate request. The
/// deliberation owner independently verifies that the request is current and
/// that the authorized subject is its named reviewer.
pub fn human_gate_resource(
    deliberation: deliberation_protocol::DeliberationIdV1,
    request: deliberation_protocol::HumanGateRequestIdV1,
) -> [u8; 16] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRAHG1");
    hash.update(deliberation.0);
    hash.update(request.0);
    let digest = hash.finalize();
    let mut resource = [0_u8; 16];
    resource.copy_from_slice(&digest[..16]);
    resource
}

/// Stable domain-separated authority resource for a single permission request
/// and capability within one durable participation.
pub fn permission_resource(
    participation: participation::ParticipationIdV1,
    execution: participation::ExecutionIdV1,
    request: participation::PermissionRequestIdV1,
    option: participation::PermissionOptionIdV1,
    capability_digest: [u8; 32],
) -> [u8; 16] {
    let mut hash = Sha256::new();
    hash.update(b"MOIRAAP1");
    hash.update(participation.0);
    hash.update(execution.0);
    hash.update(request.0);
    hash.update(option.0);
    hash.update(capability_digest);
    let digest = hash.finalize();
    let mut resource = [0_u8; 16];
    resource.copy_from_slice(&digest[..16]);
    resource
}

const fn aegis_decision_id(handle: u64) -> [u8; 16] {
    let value = handle.to_be_bytes();
    [
        b'A', b'E', b'G', b'I', b'S', b'D', b'0', b'1', value[0], value[1], value[2], value[3],
        value[4], value[5], value[6], value[7],
    ]
}
