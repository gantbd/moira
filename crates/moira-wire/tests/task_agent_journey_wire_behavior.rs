use moira_protocol::{TaskRefV1, agent_participation::ProfileIdV1, task_agent_journey as p};
use moira_wire::{
    LimitsV1, decode_task_agent_journey_reply, decode_task_agent_journey_request,
    encode_task_agent_journey_reply, encode_task_agent_journey_request,
};

#[test]
fn every_command_outcome_and_refusal_round_trips_in_disjoint_family() {
    let preview = preview();
    let instantiate = instantiate(preview);
    let commands = [
        p::CommandV1::Catalogue,
        p::CommandV1::Preview {
            expected_storage_revision: Some(4),
            request: preview.request,
        },
        p::CommandV1::Instantiate {
            expected_storage_revision: Some(5),
            request: instantiate,
        },
        p::CommandV1::PreviewView {
            preview: preview.id,
        },
        p::CommandV1::Journey {
            journey: p::JourneyIdV1 {
                owner: preview.id.owner,
                sequence: 0,
            },
        },
        p::CommandV1::Participation {
            journey: p::JourneyIdV1 {
                owner: preview.id.owner,
                sequence: 0,
            },
        },
        p::CommandV1::TaskAttemptNamespace {
            journey: p::JourneyIdV1 {
                owner: preview.id.owner,
                sequence: 0,
            },
        },
        p::CommandV1::Status,
    ];
    for value in commands {
        let bytes = encode_task_agent_journey_request(&value, LimitsV1::default()).unwrap();
        assert_eq!(
            decode_task_agent_journey_request(&bytes, LimitsV1::default()),
            Ok(value)
        );
        assert!(moira_wire::decode_agent_request(&bytes, LimitsV1::default()).is_err());
        for end in 0..bytes.len() {
            assert!(decode_task_agent_journey_request(&bytes[..end], LimitsV1::default()).is_err());
        }
    }

    let journey = p::JourneyViewV1 {
        id: p::JourneyIdV1 {
            owner: preview.id.owner,
            sequence: 0,
        },
        workflow: p::WorkflowRefV1 {
            owner: preview.id.owner,
            sequence: 0,
        },
        revision: 0,
        phase: p::PhaseV1::Admitted,
        admission: instantiate,
        confirmed_at: 1_002,
        preview,
        task_attempt: None,
        room: None,
    };
    let ready = p::JourneyViewV1 {
        revision: 1,
        phase: p::PhaseV1::TaskAttemptReady,
        task_attempt: Some(p::TaskAttemptBindingV1 {
            journey: journey.id,
            expected_journey_revision: 0,
            task: preview.request.task.task,
            attempt_number: 2,
            chronos_revision: 12,
        }),
        ..journey
    };
    let room_ready = p::JourneyViewV1 {
        revision: 2,
        phase: p::PhaseV1::RoomReady,
        room: Some(p::RoomBindingV1 {
            journey: journey.id,
            expected_journey_revision: 1,
            source: journey.workflow.owner.0,
            room_sequence: journey.workflow.sequence,
            agora_revision: 8,
        }),
        ..ready
    };
    let participation_ready = p::JourneyViewV1 {
        revision: 3,
        phase: p::PhaseV1::ParticipationAdmitted,
        ..room_ready
    };
    let participation = p::ParticipationAdmissionViewV1 {
        journey: journey.id,
        expected_journey_revision: 2,
        participation: [21; 16],
        owner_revision: 0,
        owner_phase: moira_protocol::agent_participation::PhaseV1::Admitted,
        authority_decision: [22; 16],
        authority_principal: [23; 16],
        authority_epoch: 4,
        authority_expires_at: 2_000,
    };
    let namespace = p::TaskAttemptNamespaceViewV1 {
        journey: journey.id,
        expected_journey_revision: 0,
        task: preview.request.task.task,
        attempt_number: 2,
        chronos_revision: 12,
        chronos_task_id: "exact-task".into(),
    };
    let replies = vec![
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Catalogue {
            owner: preview.id.owner,
            owner_revision: 1,
            storage_revision: 2,
            templates: vec![template()],
        })),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Previewed {
            storage_revision: 3,
            receipt: p::PreviewReceiptV1 {
                preview,
                replayed: true,
            },
        })),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Admitted {
            storage_revision: 3,
            receipt: p::JourneyReceiptV1 {
                journey,
                replayed: false,
            },
        })),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Preview(Some(preview)))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Preview(None))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Journey(Some(journey)))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Journey(Some(ready)))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Journey(Some(room_ready)))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Journey(Some(participation_ready)))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Journey(None))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Participation(Some(participation)))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Participation(None))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::TaskAttemptNamespace(Some(
            namespace,
        )))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::TaskAttemptNamespace(None))),
        p::ReplyV1::Accepted(Box::new(p::OutcomeV1::Status(p::StatusV1 {
            owner: preview.id.owner,
            owner_revision: 1,
            storage_revision: 3,
            template_count: 1,
            preview_count: 1,
            journey_count: 1,
            needs_reopen: false,
        }))),
    ];
    for value in replies {
        let bytes = encode_task_agent_journey_reply(&value, LimitsV1::default()).unwrap();
        assert_eq!(
            decode_task_agent_journey_reply(&bytes, LimitsV1::default()),
            Ok(value)
        );
        for end in 0..bytes.len() {
            assert!(decode_task_agent_journey_reply(&bytes[..end], LimitsV1::default()).is_err());
        }
        let mut tailed = bytes;
        tailed.push(0);
        assert!(decode_task_agent_journey_reply(&tailed, LimitsV1::default()).is_err());
    }

    let refusals = [
        p::RefusalV1::NotAuthorized,
        p::RefusalV1::StorageRevisionConflict,
        p::RefusalV1::InvalidLimits,
        p::RefusalV1::InvalidOwner,
        p::RefusalV1::InvalidTemplate,
        p::RefusalV1::DuplicateTemplate,
        p::RefusalV1::TemplateLimit,
        p::RefusalV1::InvalidPreview,
        p::RefusalV1::PreviewLimit,
        p::RefusalV1::PreviewOperationConflict,
        p::RefusalV1::TemplateNotFound,
        p::RefusalV1::TemplateMismatch,
        p::RefusalV1::TemplateRetired,
        p::RefusalV1::PreviewNotFound,
        p::RefusalV1::PreviewExpired,
        p::RefusalV1::OwnerRevisionConflict,
        p::RefusalV1::InvalidConfirmation,
        p::RefusalV1::JourneyLimit,
        p::RefusalV1::InstantiateOperationConflict,
        p::RefusalV1::PreviewAlreadyInstantiated,
        p::RefusalV1::IdentityExhausted,
        p::RefusalV1::InvalidCheckpoint,
        p::RefusalV1::Storage,
    ];
    for refusal in refusals {
        let reply = p::ReplyV1::Refused(refusal);
        let bytes = encode_task_agent_journey_reply(&reply, LimitsV1::default()).unwrap();
        assert_eq!(
            decode_task_agent_journey_reply(&bytes, LimitsV1::default()),
            Ok(reply)
        );
    }
}

fn template() -> p::TemplateV1 {
    p::TemplateV1 {
        id: p::TemplateIdV1([2; 16]),
        version: 3,
        digest: [3; 32],
        work_mode: p::WorkModeV1::ProductManager,
        profile: ProfileIdV1([4; 16]),
        room_policy: p::RoomPolicyV1::SharedContext,
        budget: p::BudgetV1 {
            max_context_bytes: 32_768,
            max_output_tokens: 4_096,
            max_elapsed_millis: 120_000,
        },
        grounding_policy_digest: [5; 32],
        authority_policy_digest: [6; 32],
        consequence_digest: [7; 32],
        retired: false,
    }
}

fn preview() -> p::PreviewViewV1 {
    let template = template();
    p::PreviewViewV1 {
        id: p::PreviewIdV1 {
            owner: p::OwnerIdV1([1; 16]),
            sequence: 0,
        },
        owner_revision: 1,
        request: p::PreviewRequestV1 {
            operation: p::PreviewOperationIdV1([8; 16]),
            template: p::TemplateRefV1 {
                id: template.id,
                version: template.version,
                digest: template.digest,
            },
            task: p::TaskAdmissionV1 {
                task: TaskRefV1 {
                    external_task_key: [9; 16],
                    incarnation: 10,
                },
                chronos_revision: 11,
                evidence_digest: [12; 32],
            },
            profile: template.profile,
            room_policy: template.room_policy,
            initiating_principal: p::PrincipalIdV1([13; 16]),
            accountable_human: p::PrincipalIdV1([14; 16]),
            objective_digest: [15; 32],
            acceptance_digest: [16; 32],
            current_evidence_digest: [17; 32],
            requested_ttl: 300,
        },
        observed_at: 1_000,
        expires_at: 1_300,
        work_mode: template.work_mode,
        budget: template.budget,
        grounding_policy_digest: template.grounding_policy_digest,
        authority_policy_digest: template.authority_policy_digest,
        consequence_digest: template.consequence_digest,
    }
}

fn instantiate(preview: p::PreviewViewV1) -> p::InstantiateRequestV1 {
    p::InstantiateRequestV1 {
        operation: p::InstantiateOperationIdV1([18; 16]),
        preview: preview.id,
        expected_owner_revision: preview.owner_revision,
        confirmation: p::HumanConfirmationV1 {
            preview: preview.id,
            accountable_human: preview.request.accountable_human,
            consequence_digest: preview.consequence_digest,
        },
    }
}
