//! Moira's independent interactive-agent projection over Daimon's public port.

use daimon_client::AgentServicePort;
use daimon_protocol::{self as daimon, agent as upstream};
use moira_host::AgentGateway;
use moira_protocol::agent as public;

pub struct AgentAdapter<P> {
    service_session: daimon::SessionHandleV1,
    port: P,
}

impl<P> AgentAdapter<P> {
    pub const fn new(service_session: daimon::SessionHandleV1, port: P) -> Self {
        Self {
            service_session,
            port,
        }
    }

    pub fn into_port(self) -> P {
        self.port
    }
}

impl<P: AgentServicePort> AgentGateway for AgentAdapter<P> {
    type Error = P::Error;

    fn execute_agent(
        &mut self,
        command: public::CommandV1,
    ) -> Result<public::ReplyV1, Self::Error> {
        self.port
            .execute_agent(self.service_session, command_upstream(command))
            .map(reply_public)
    }
}

fn command_upstream(value: public::CommandV1) -> upstream::AgentCommandV1 {
    match value {
        public::CommandV1::Open { request, now } => upstream::AgentCommandV1::Open {
            request: upstream::AgentOpenRequestV1 {
                session: session(request.session),
                execution: execution(request.execution),
                workspace: request.workspace,
                budget: upstream::AgentSessionBudgetV1 {
                    max_elapsed_millis: request.budget.max_elapsed_millis,
                    max_turns: request.budget.max_turns,
                    max_events: request.budget.max_events,
                    max_event_bytes: request.budget.max_event_bytes,
                    max_prompt_bytes: request.budget.max_prompt_bytes,
                    max_permission_requests: request.budget.max_permission_requests,
                },
            },
            now,
        },
        public::CommandV1::Prompt { request, now } => upstream::AgentCommandV1::Prompt {
            request: upstream::AgentTurnRequestV1 {
                session: session(request.session),
                turn: turn(request.turn),
                expected_revision: request.expected_revision,
                execution: execution(request.execution),
                prompt_digest: daimon::ContentDigestV1::new(request.prompt_digest.0),
                prompt: request.prompt,
            },
            now,
        },
        public::CommandV1::Events {
            session: id,
            after,
            limit,
        } => upstream::AgentCommandV1::Events {
            session: session(id),
            after,
            limit,
        },
        public::CommandV1::Inspect { session: id } => upstream::AgentCommandV1::Inspect {
            session: session(id),
        },
        public::CommandV1::DecidePermission {
            session: id,
            expected_revision,
            execution: request,
            decision,
            now,
        } => upstream::AgentCommandV1::DecidePermission {
            session: session(id),
            expected_revision,
            execution: execution(request),
            decision: upstream::AgentPermissionDecisionV1 {
                request: daimon::AgentPermissionRequestIdV1::new(decision.request.0),
                option: daimon::AgentPermissionOptionIdV1::new(decision.option.0),
            },
            now,
        },
        public::CommandV1::Reattach {
            session: id,
            expected_revision,
            execution: request,
            now,
        } => upstream::AgentCommandV1::Reattach {
            session: session(id),
            expected_revision,
            execution: execution(request),
            now,
        },
        public::CommandV1::Advance {
            session: id,
            turn: turn_id,
            expected_revision,
            idempotency_key,
            now,
        } => upstream::AgentCommandV1::Advance {
            session: session(id),
            turn: turn(turn_id),
            expected_revision,
            idempotency_key: key(idempotency_key),
            now,
        },
        public::CommandV1::Cancel {
            session: id,
            turn: turn_id,
            expected_revision,
            idempotency_key,
            now,
        } => upstream::AgentCommandV1::Cancel {
            session: session(id),
            turn: turn(turn_id),
            expected_revision,
            idempotency_key: key(idempotency_key),
            now,
        },
        public::CommandV1::Close {
            session: id,
            expected_revision,
            idempotency_key,
            now,
        } => upstream::AgentCommandV1::Close {
            session: session(id),
            expected_revision,
            idempotency_key: key(idempotency_key),
            now,
        },
    }
}

fn execution(value: public::ExecutionV1) -> daimon::ExecuteRequestV1 {
    daimon::ExecuteRequestV1 {
        execution_id: daimon::ExecutionIdV1::new(value.id.0),
        task_attempt: daimon::TaskAttemptRefV1 {
            task_key: value.attempt.task_key,
            task_incarnation: value.attempt.incarnation,
            attempt_number: value.attempt.attempt,
        },
        principal: daimon::PrincipalIdV1::new(value.principal.0),
        profile: daimon::HarnessProfileIdV1::new(value.profile.0),
        idempotency_key: key(value.idempotency_key),
        context: value
            .context
            .into_iter()
            .map(|id| daimon::ContextReferenceIdV1::new(id.0))
            .collect(),
        tools: value
            .tools
            .into_iter()
            .map(|id| daimon::ToolCapabilityIdV1::new(id.0))
            .collect(),
        budget: daimon::ExecutionBudgetV1 {
            max_elapsed_millis: value.budget.max_elapsed_millis,
            max_input_tokens: value.budget.max_input_tokens,
            max_output_tokens: value.budget.max_output_tokens,
            max_tool_calls: value.budget.max_tool_calls,
            max_context_bytes: value.budget.max_context_bytes,
            max_output_bytes: value.budget.max_output_bytes,
        },
    }
}

fn reply_public(value: upstream::AgentReplyV1) -> public::ReplyV1 {
    match value {
        upstream::AgentReplyV1::Accepted(value) => public::ReplyV1::Accepted(outcome_public(value)),
        upstream::AgentReplyV1::Refused(value) => public::ReplyV1::Refused(refusal_public(value)),
    }
}

fn outcome_public(value: upstream::AgentOutcomeV1) -> public::OutcomeV1 {
    match value {
        upstream::AgentOutcomeV1::Session(value) => {
            public::OutcomeV1::Session(public::SessionViewV1 {
                session: public::SessionIdV1(value.session.into_bytes()),
                revision: value.revision,
                phase: phase_public(value.phase),
                profile_document_digest: public::DigestV1(
                    value.profile_document_digest.into_bytes(),
                ),
                active_turn: value
                    .active_turn
                    .map(|id| public::TurnIdV1(id.into_bytes())),
                last_event_cursor: value.last_event_cursor,
                last_context: value.last_context.map(|receipt| public::ContextReceiptV1 {
                    execution: public::ExecutionIdV1(receipt.execution.into_bytes()),
                    manifest_digest: public::DigestV1(receipt.manifest_digest.into_bytes()),
                    entries: receipt
                        .entries
                        .into_iter()
                        .map(|entry| public::ContextManifestEntryV1 {
                            reference: public::ContextIdV1(entry.reference.into_bytes()),
                            kind: match entry.kind {
                                daimon::ContextKindV1::AgoraMessage => {
                                    public::ContextKindV1::AgoraMessage
                                }
                                daimon::ContextKindV1::ChronosWork => {
                                    public::ContextKindV1::ChronosWork
                                }
                                daimon::ContextKindV1::VellumDocument => {
                                    public::ContextKindV1::VellumDocument
                                }
                                daimon::ContextKindV1::DendriteGrounding => {
                                    public::ContextKindV1::DendriteGrounding
                                }
                            },
                            source_digest: public::DigestV1(entry.source_digest.into_bytes()),
                            content_bytes: entry.content_bytes,
                            grounding: entry.grounding.map(|grounding| {
                                public::GroundingEvidenceV1 {
                                    depth: match grounding.depth {
                                        daimon::GroundingDepthV1::Ambient => {
                                            public::GroundingDepthV1::Ambient
                                        }
                                        daimon::GroundingDepthV1::Activated => {
                                            public::GroundingDepthV1::Activated
                                        }
                                        daimon::GroundingDepthV1::Reference => {
                                            public::GroundingDepthV1::Reference
                                        }
                                    },
                                    grounding_digest: public::DigestV1(
                                        grounding.grounding_digest.into_bytes(),
                                    ),
                                    surface_digest: public::DigestV1(
                                        grounding.surface_digest.into_bytes(),
                                    ),
                                }
                            }),
                        })
                        .collect(),
                }),
                capabilities: capabilities_public(value.capabilities),
            })
        }
        upstream::AgentOutcomeV1::Events(value) => public::OutcomeV1::Events(public::EventPageV1 {
            session: public::SessionIdV1(value.session.into_bytes()),
            entries: value.entries.into_iter().map(event_record_public).collect(),
            next_after: value.next_after,
        }),
        upstream::AgentOutcomeV1::OutcomeUnknown {
            session,
            idempotency_key,
        } => public::OutcomeV1::OutcomeUnknown {
            session: public::SessionIdV1(session.into_bytes()),
            idempotency_key: public::IdempotencyKeyV1(idempotency_key.into_bytes()),
        },
    }
}

fn event_record_public(value: upstream::AgentEventRecordV1) -> public::EventRecordV1 {
    public::EventRecordV1 {
        cursor: value.cursor,
        event: event_public(value.event),
    }
}

fn event_public(value: upstream::AgentEventV1) -> public::EventV1 {
    match value {
        upstream::AgentEventV1::AssistantMessage { bytes } => {
            public::EventV1::AssistantMessage { bytes }
        }
        upstream::AgentEventV1::Thought { bytes } => public::EventV1::Thought { bytes },
        upstream::AgentEventV1::Plan { bytes } => public::EventV1::Plan { bytes },
        upstream::AgentEventV1::ToolCall { title, detail } => {
            public::EventV1::ToolCall { title, detail }
        }
        upstream::AgentEventV1::Terminal { stream, bytes } => {
            public::EventV1::Terminal { stream, bytes }
        }
        upstream::AgentEventV1::Permission(value) => {
            public::EventV1::Permission(permission_public(value))
        }
        upstream::AgentEventV1::FileChange {
            path,
            content_digest,
        } => public::EventV1::FileChange {
            path,
            content_digest: public::DigestV1(content_digest.into_bytes()),
        },
        upstream::AgentEventV1::FileRead {
            path,
            content_digest,
            content_bytes,
        } => public::EventV1::FileRead {
            path,
            content_digest: public::DigestV1(content_digest.into_bytes()),
            content_bytes,
        },
        upstream::AgentEventV1::Usage {
            input_tokens,
            output_tokens,
        } => public::EventV1::Usage {
            input_tokens,
            output_tokens,
        },
        upstream::AgentEventV1::TurnFinished {
            turn,
            disposition,
            stop_reason,
            output_digest,
        } => public::EventV1::TurnFinished {
            turn: public::TurnIdV1(turn.into_bytes()),
            disposition: disposition_public(disposition),
            stop_reason: stop_reason_public(stop_reason),
            output_digest: output_digest.map(|value| public::DigestV1(value.into_bytes())),
        },
    }
}

fn permission_public(value: upstream::AgentPermissionRequestV1) -> public::PermissionRequestV1 {
    public::PermissionRequestV1 {
        request: public::PermissionRequestIdV1(value.request.into_bytes()),
        turn: public::TurnIdV1(value.turn.into_bytes()),
        capability: value.capability,
        description: value.description,
        options: value
            .options
            .into_iter()
            .map(|option| public::PermissionOptionV1 {
                id: public::PermissionOptionIdV1(option.id.into_bytes()),
                kind: match option.kind {
                    upstream::AgentPermissionOptionKindV1::AllowOnce => {
                        public::PermissionOptionKindV1::AllowOnce
                    }
                    upstream::AgentPermissionOptionKindV1::AllowAlways => {
                        public::PermissionOptionKindV1::AllowAlways
                    }
                    upstream::AgentPermissionOptionKindV1::RejectOnce => {
                        public::PermissionOptionKindV1::RejectOnce
                    }
                    upstream::AgentPermissionOptionKindV1::RejectAlways => {
                        public::PermissionOptionKindV1::RejectAlways
                    }
                },
                label: option.label,
            })
            .collect(),
    }
}

fn capabilities_public(value: upstream::AgentCapabilitiesV1) -> public::CapabilitiesV1 {
    public::CapabilitiesV1 {
        load_session: value.load_session,
        resume_session: value.resume_session,
        permissions: value.permissions,
        filesystem_read: value.filesystem_read,
        filesystem_write: value.filesystem_write,
        terminal: value.terminal,
        session_modes: value.session_modes,
        session_models: value.session_models,
    }
}

fn phase_public(value: upstream::AgentSessionPhaseV1) -> public::PhaseV1 {
    match value {
        upstream::AgentSessionPhaseV1::Opening => public::PhaseV1::Opening,
        upstream::AgentSessionPhaseV1::Ready => public::PhaseV1::Ready,
        upstream::AgentSessionPhaseV1::TurnActive => public::PhaseV1::TurnActive,
        upstream::AgentSessionPhaseV1::AwaitingPermission => public::PhaseV1::AwaitingPermission,
        upstream::AgentSessionPhaseV1::OutcomeUnknown => public::PhaseV1::OutcomeUnknown,
        upstream::AgentSessionPhaseV1::Closed => public::PhaseV1::Closed,
        upstream::AgentSessionPhaseV1::ReattachRequired => public::PhaseV1::ReattachRequired,
    }
}

fn disposition_public(value: daimon::ExecutionDispositionV1) -> public::DispositionV1 {
    match value {
        daimon::ExecutionDispositionV1::Completed => public::DispositionV1::Completed,
        daimon::ExecutionDispositionV1::Failed => public::DispositionV1::Failed,
        daimon::ExecutionDispositionV1::Cancelled => public::DispositionV1::Cancelled,
        daimon::ExecutionDispositionV1::TimedOut => public::DispositionV1::TimedOut,
        daimon::ExecutionDispositionV1::Uncertain => public::DispositionV1::Uncertain,
    }
}

fn stop_reason_public(value: upstream::AgentTurnStopReasonV1) -> public::StopReasonV1 {
    match value {
        upstream::AgentTurnStopReasonV1::EndTurn => public::StopReasonV1::EndTurn,
        upstream::AgentTurnStopReasonV1::Cancelled => public::StopReasonV1::Cancelled,
        upstream::AgentTurnStopReasonV1::TimedOut => public::StopReasonV1::TimedOut,
        upstream::AgentTurnStopReasonV1::Failed => public::StopReasonV1::Failed,
        upstream::AgentTurnStopReasonV1::Disconnected => public::StopReasonV1::Disconnected,
        upstream::AgentTurnStopReasonV1::OutcomeUnknown => public::StopReasonV1::OutcomeUnknown,
    }
}

fn refusal_public(value: upstream::AgentRefusalV1) -> public::RefusalV1 {
    match value {
        upstream::AgentRefusalV1::UnknownServiceSession => public::RefusalV1::UnknownServiceSession,
        upstream::AgentRefusalV1::PrincipalMismatch => public::RefusalV1::PrincipalMismatch,
        upstream::AgentRefusalV1::UnknownAgentSession => public::RefusalV1::UnknownAgentSession,
        upstream::AgentRefusalV1::UnknownTurn => public::RefusalV1::UnknownTurn,
        upstream::AgentRefusalV1::UnknownPermissionRequest => {
            public::RefusalV1::UnknownPermissionRequest
        }
        upstream::AgentRefusalV1::UnknownPermissionOption => {
            public::RefusalV1::UnknownPermissionOption
        }
        upstream::AgentRefusalV1::RevisionConflict => public::RefusalV1::RevisionConflict,
        upstream::AgentRefusalV1::AuthorityDenied => public::RefusalV1::AuthorityDenied,
        upstream::AgentRefusalV1::AuthorityExpired => public::RefusalV1::AuthorityExpired,
        upstream::AgentRefusalV1::AttemptDenied => public::RefusalV1::AttemptDenied,
        upstream::AgentRefusalV1::IdempotencyConflict => public::RefusalV1::IdempotencyConflict,
        upstream::AgentRefusalV1::SessionConflict => public::RefusalV1::SessionConflict,
        upstream::AgentRefusalV1::TurnConflict => public::RefusalV1::TurnConflict,
        upstream::AgentRefusalV1::ProviderReceiptMismatch => {
            public::RefusalV1::ProviderReceiptMismatch
        }
        upstream::AgentRefusalV1::ProviderReceiptConflict => {
            public::RefusalV1::ProviderReceiptConflict
        }
        upstream::AgentRefusalV1::Busy => public::RefusalV1::Busy,
        upstream::AgentRefusalV1::Capacity => public::RefusalV1::Capacity,
        upstream::AgentRefusalV1::Limit => public::RefusalV1::Limit,
        upstream::AgentRefusalV1::Unsupported => public::RefusalV1::Unsupported,
        upstream::AgentRefusalV1::InvalidInput => public::RefusalV1::InvalidInput,
        upstream::AgentRefusalV1::Closed => public::RefusalV1::Closed,
    }
}

fn session(value: public::SessionIdV1) -> daimon::AgentSessionIdV1 {
    daimon::AgentSessionIdV1::new(value.0)
}

fn turn(value: public::TurnIdV1) -> daimon::AgentTurnIdV1 {
    daimon::AgentTurnIdV1::new(value.0)
}

fn key(value: public::IdempotencyKeyV1) -> daimon::IdempotencyKeyV1 {
    daimon::IdempotencyKeyV1::new(value.0)
}
