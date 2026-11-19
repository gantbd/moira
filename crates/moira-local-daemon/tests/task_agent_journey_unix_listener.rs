#![cfg(unix)]

use moira_client::{
    TaskAgentJourneyMutationClient, TaskAgentJourneyReadOnlyClient,
    unix::{TaskAgentJourneyClientLimitsV1, TaskAgentJourneyUnixPort},
};
use moira_core::{
    agent_participation::ProfileIdV1 as DomainProfileIdV1, task_agent_journey as domain,
};
use moira_local_daemon::unix::{Authenticator, TaskAgentJourneyDaemon};
use moira_protocol::{TaskRefV1, agent_participation::ProfileIdV1, task_agent_journey as public};
use moira_service::{
    AccessV1,
    task_agent_journey::{TaskAgentJourneyService, TrustedTimeV1},
};
use moira_sqlite::task_agent_journey::{
    TaskAgentJourneySqliteStore, TaskAgentJourneyStoreLimitsV1,
};
use moira_wire::LimitsV1;
use std::{
    convert::Infallible,
    fs,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    database: PathBuf,
    socket: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "moira-task-agent-daemon-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self {
            database: root.join("task-agent.db"),
            socket: root.join("task-agent.sock"),
            root,
        }
    }

    fn store(&self) -> TaskAgentJourneySqliteStore {
        TaskAgentJourneySqliteStore::open(
            &self.database,
            domain::OwnerIdV1([1; 16]),
            store_limits(),
            [template()],
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
        for name in [
            "task-agent.db",
            "task-agent.db-journal",
            "task-agent.db.owner.lock",
        ] {
            let _ = fs::remove_file(self.root.join(name));
        }
        let _ = fs::remove_dir(&self.root);
    }
}

struct Authentication;

impl Authenticator for Authentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AccessV1, Self::Error> {
        Ok(AccessV1 {
            read: true,
            mutate: true,
        })
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

fn store_limits() -> TaskAgentJourneyStoreLimitsV1 {
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

fn client_limits() -> TaskAgentJourneyClientLimitsV1 {
    TaskAgentJourneyClientLimitsV1 {
        max_frame_bytes: 64 * 1024,
        max_requests: 8,
        io_timeout: Duration::from_secs(3),
    }
}

fn daemon(
    fixture: &Fixture,
    time: u64,
) -> TaskAgentJourneyDaemon<TaskAgentJourneySqliteStore, Time, Authentication> {
    TaskAgentJourneyDaemon::bind_new(
        &fixture.socket,
        TaskAgentJourneyService::new(fixture.store(), Time(time)),
        Authentication,
        LimitsV1::default(),
        Duration::from_secs(3),
        8,
    )
    .unwrap()
}

#[test]
fn private_port_admits_and_reopens_a_human_bound_task_agent_journey() {
    let fixture = Fixture::new();
    let mut owner = daemon(&fixture, 100);
    let worker = thread::spawn(move || owner.serve_next().unwrap());

    let port = TaskAgentJourneyUnixPort::connect(&fixture.socket, client_limits()).unwrap();
    let mut reader = TaskAgentJourneyReadOnlyClient::new(port);
    assert!(matches!(
        reader.catalogue().unwrap(),
        public::ReplyV1::Accepted(outcome)
            if matches!(*outcome, public::OutcomeV1::Catalogue {
                owner,
                storage_revision: 0,
                ref templates,
                ..
            } if owner == public::OwnerIdV1([1; 16]) && templates.len() == 1)
    ));

    let mut mutation = TaskAgentJourneyMutationClient::new(reader.into_port());
    let public::ReplyV1::Accepted(outcome) = mutation.preview(None, preview_request()).unwrap()
    else {
        panic!("preview expected")
    };
    let public::OutcomeV1::Previewed {
        storage_revision: 1,
        receipt,
    } = *outcome
    else {
        panic!("preview receipt expected")
    };
    assert!(!receipt.replayed);
    let preview = receipt.preview;
    let instantiate = public::InstantiateRequestV1 {
        operation: public::InstantiateOperationIdV1([17; 16]),
        preview: preview.id,
        expected_owner_revision: preview.owner_revision,
        confirmation: public::HumanConfirmationV1 {
            preview: preview.id,
            accountable_human: preview.request.accountable_human,
            consequence_digest: preview.consequence_digest,
        },
    };
    let public::ReplyV1::Accepted(outcome) = mutation.instantiate(Some(1), instantiate).unwrap()
    else {
        panic!("journey admission expected")
    };
    let public::OutcomeV1::Admitted {
        storage_revision: 2,
        receipt,
    } = *outcome
    else {
        panic!("journey receipt expected")
    };
    assert!(!receipt.replayed);
    let journey = receipt.journey;

    let mut reader = TaskAgentJourneyReadOnlyClient::new(mutation.into_port());
    assert!(matches!(
        reader.status().unwrap(),
        public::ReplyV1::Accepted(outcome)
            if matches!(*outcome, public::OutcomeV1::Status(public::StatusV1 {
                storage_revision: 2,
                preview_count: 1,
                journey_count: 1,
                needs_reopen: false,
                ..
            }))
    ));
    drop(reader);
    worker.join().unwrap();
    assert!(!fixture.socket.exists());

    let mut reopened = daemon(&fixture, 200);
    let worker = thread::spawn(move || reopened.serve_next().unwrap());
    let port = TaskAgentJourneyUnixPort::connect(&fixture.socket, client_limits()).unwrap();
    let mut mutation = TaskAgentJourneyMutationClient::new(port);
    let public::ReplyV1::Accepted(outcome) = mutation.instantiate(Some(2), instantiate).unwrap()
    else {
        panic!("journey replay expected")
    };
    assert!(matches!(
        *outcome,
        public::OutcomeV1::Admitted {
            storage_revision: 2,
            receipt
        } if receipt.replayed && receipt.journey == journey
    ));
    let mut reader = TaskAgentJourneyReadOnlyClient::new(mutation.into_port());
    assert_eq!(
        reader.journey(journey.id).unwrap(),
        public::ReplyV1::Accepted(Box::new(public::OutcomeV1::Journey(Some(journey))))
    );
    drop(reader);
    worker.join().unwrap();
}
