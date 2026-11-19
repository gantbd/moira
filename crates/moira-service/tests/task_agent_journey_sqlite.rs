#![cfg(unix)]

use moira_core::{
    agent_participation::ProfileIdV1 as DomainProfileIdV1, task_agent_journey as domain,
};
use moira_protocol::{TaskRefV1, agent_participation::ProfileIdV1, task_agent_journey as public};
use moira_service::{
    AccessV1,
    task_agent_journey::{TaskAgentJourneyService, TrustedTimeV1},
};
use moira_sqlite::task_agent_journey::{
    TaskAgentJourneySqliteStore, TaskAgentJourneyStoreLimitsV1,
};
use std::{
    convert::Infallible,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        Self {
            path: std::env::temp_dir().join(format!(
                "moira-task-agent-journey-{}-{}.db",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
        }
    }

    fn store(&self) -> TaskAgentJourneySqliteStore {
        TaskAgentJourneySqliteStore::open(
            &self.path,
            domain::OwnerIdV1([1; 16]),
            limits(),
            [template()],
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-journal", ".owner.lock"] {
            let _ = fs::remove_file(format!("{}{}", self.path.display(), suffix));
        }
    }
}

struct Time(u64);

impl TrustedTimeV1 for Time {
    type Error = Infallible;

    fn now(&mut self) -> Result<u64, Self::Error> {
        let value = self.0;
        self.0 += 1;
        Ok(value)
    }
}

fn limits() -> TaskAgentJourneyStoreLimitsV1 {
    let domain = domain::LimitsV1 {
        max_templates: 2,
        max_previews: 4,
        max_journeys: 4,
        max_preview_ttl: 60,
    };
    TaskAgentJourneyStoreLimitsV1 {
        codec: domain::persistence::CodecLimitsV1 {
            max_bytes: 65_536,
            domain,
        },
        domain,
        max_database_pages: 64,
    }
}

fn template() -> domain::TemplateV1 {
    domain::TemplateV1 {
        id: domain::TemplateIdV1([2; 16]),
        version: 1,
        digest: [3; 32],
        work_mode: domain::WorkModeV1::Collaborator,
        profile: DomainProfileIdV1([4; 16]),
        room_policy: domain::RoomPolicyV1::SharedContext,
        budget: domain::BudgetV1 {
            max_context_bytes: 4_096,
            max_output_tokens: 512,
            max_elapsed_millis: 10_000,
        },
        grounding_policy_digest: [5; 32],
        authority_policy_digest: [6; 32],
        consequence_digest: [7; 32],
        retired: false,
    }
}

fn preview_request() -> public::PreviewRequestV1 {
    public::PreviewRequestV1 {
        operation: public::PreviewOperationIdV1([8; 16]),
        template: public::TemplateRefV1 {
            id: public::TemplateIdV1([2; 16]),
            version: 1,
            digest: [3; 32],
        },
        task: public::TaskAdmissionV1 {
            task: TaskRefV1 {
                external_task_key: [9; 16],
                incarnation: 1,
            },
            chronos_revision: 11,
            evidence_digest: [10; 32],
        },
        profile: ProfileIdV1([4; 16]),
        room_policy: public::RoomPolicyV1::SharedContext,
        initiating_principal: public::PrincipalIdV1([12; 16]),
        accountable_human: public::PrincipalIdV1([13; 16]),
        objective_digest: [14; 32],
        acceptance_digest: [15; 32],
        current_evidence_digest: [16; 32],
        requested_ttl: 30,
    }
}

#[test]
fn public_service_persists_preview_admission_and_exact_replay_across_reopen() {
    let fixture = Fixture::new();
    let preview_command = public::CommandV1::Preview {
        expected_storage_revision: None,
        request: preview_request(),
    };
    let mut service = TaskAgentJourneyService::new(fixture.store(), Time(100));

    assert_eq!(
        service
            .execute_authorized(
                AccessV1 {
                    read: true,
                    mutate: false,
                },
                preview_command.clone(),
            )
            .unwrap(),
        public::ReplyV1::Refused(public::RefusalV1::NotAuthorized)
    );
    let public::ReplyV1::Accepted(outcome) = service.execute(preview_command.clone()).unwrap()
    else {
        panic!("preview expected")
    };
    let public::OutcomeV1::Previewed {
        storage_revision,
        receipt,
    } = *outcome
    else {
        panic!("preview outcome expected")
    };
    assert_eq!(storage_revision, 1);
    assert!(!receipt.replayed);
    let preview = receipt.preview;

    let replay = public::CommandV1::Preview {
        expected_storage_revision: Some(1),
        request: preview_request(),
    };
    let public::ReplyV1::Accepted(outcome) = service.execute(replay.clone()).unwrap() else {
        panic!("preview replay expected")
    };
    let public::OutcomeV1::Previewed {
        storage_revision,
        receipt,
    } = *outcome
    else {
        panic!("preview replay outcome expected")
    };
    assert_eq!(storage_revision, 1);
    assert!(receipt.replayed);
    drop(service);

    let instantiate_request = public::InstantiateRequestV1 {
        operation: public::InstantiateOperationIdV1([17; 16]),
        preview: preview.id,
        expected_owner_revision: preview.owner_revision,
        confirmation: public::HumanConfirmationV1 {
            preview: preview.id,
            accountable_human: preview.request.accountable_human,
            consequence_digest: preview.consequence_digest,
        },
    };
    let instantiate = public::CommandV1::Instantiate {
        expected_storage_revision: Some(1),
        request: instantiate_request,
    };
    let mut service = TaskAgentJourneyService::new(fixture.store(), Time(101));
    assert_eq!(
        service.execute(replay).unwrap(),
        public::ReplyV1::Accepted(Box::new(public::OutcomeV1::Previewed {
            storage_revision: 1,
            receipt: public::PreviewReceiptV1 {
                preview,
                replayed: true,
            },
        }))
    );
    let public::ReplyV1::Accepted(outcome) = service.execute(instantiate.clone()).unwrap() else {
        panic!("journey admission expected")
    };
    let public::OutcomeV1::Admitted {
        storage_revision,
        receipt,
    } = *outcome
    else {
        panic!("journey outcome expected")
    };
    assert_eq!(storage_revision, 2);
    assert!(!receipt.replayed);
    let journey = receipt.journey;

    let replay = public::CommandV1::Instantiate {
        expected_storage_revision: Some(2),
        request: instantiate_request,
    };
    let public::ReplyV1::Accepted(outcome) = service.execute(replay).unwrap() else {
        panic!("journey replay expected")
    };
    let public::OutcomeV1::Admitted {
        storage_revision,
        receipt,
    } = *outcome
    else {
        panic!("journey replay outcome expected")
    };
    assert_eq!(storage_revision, 2);
    assert!(receipt.replayed);
    assert_eq!(receipt.journey, journey);
    drop(service);

    let mut service = TaskAgentJourneyService::new(fixture.store(), Time(200));
    assert_eq!(
        service.execute(public::CommandV1::Status).unwrap(),
        public::ReplyV1::Accepted(Box::new(public::OutcomeV1::Status(public::StatusV1 {
            owner: public::OwnerIdV1([1; 16]),
            owner_revision: 1,
            storage_revision: 2,
            template_count: 1,
            preview_count: 1,
            journey_count: 1,
            needs_reopen: false,
        })))
    );
    assert_eq!(
        service
            .execute(public::CommandV1::Journey {
                journey: journey.id,
            })
            .unwrap(),
        public::ReplyV1::Accepted(Box::new(public::OutcomeV1::Journey(Some(journey))))
    );
    assert_eq!(
        service.execute(preview_command).unwrap(),
        public::ReplyV1::Refused(public::RefusalV1::StorageRevisionConflict)
    );
}
