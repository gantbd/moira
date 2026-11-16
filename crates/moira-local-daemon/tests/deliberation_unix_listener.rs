#![cfg(unix)]

use moira_client::{
    DeliberationMutationClient, DeliberationReadOnlyClient,
    unix::{DeliberationClientLimitsV1, DeliberationUnixPort},
};
use moira_core::deliberation::{
    DeliberationLimitsV1,
    registry::{RegistryLimitsV1, persistence::CodecLimitsV1},
};
use moira_local_daemon::unix::{Authenticator, DeliberationDaemon};
use moira_protocol::deliberation::*;
use moira_service::{AccessV1, deliberation::DeliberationService};
use moira_sqlite::deliberation::{DeliberationSqliteStore, DeliberationStoreLimitsV1};
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

struct ReadOnly;

impl Authenticator for ReadOnly {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AccessV1, Self::Error> {
        Ok(AccessV1 {
            read: true,
            mutate: false,
        })
    }
}

struct Fixture {
    root: PathBuf,
    database: PathBuf,
    socket: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "moira-deliberation-daemon-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self {
            database: root.join("deliberation.db"),
            socket: root.join("deliberation.sock"),
            root,
        }
    }

    fn open(&self) -> DeliberationSqliteStore {
        DeliberationSqliteStore::open(
            &self.database,
            [31; 16],
            DeliberationStoreLimitsV1 {
                codec: CodecLimitsV1 {
                    max_bytes: 65_536,
                    max_deliberation_bytes: 32_768,
                    registry: RegistryLimitsV1 {
                        max_deliberations: 8,
                        deliberation: DeliberationLimitsV1 {
                            max_claims: 8,
                            max_criteria: 8,
                            max_candidates: 8,
                            max_members: 4,
                            max_evidence_per_report: 8,
                            max_dissent_per_report: 8,
                            max_fuel: 32,
                        },
                    },
                },
                max_database_pages: 64,
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
        for name in [
            "deliberation.db",
            "deliberation.db-journal",
            "deliberation.db.owner.lock",
        ] {
            let _ = fs::remove_file(self.root.join(name));
        }
        let _ = fs::remove_dir(&self.root);
    }
}

fn plan() -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([1; 16]),
        question_digest: [20; 32],
        method: MethodRefV1 {
            machine_digest: [2; 32],
            state_digest: [3; 32],
        },
        claims: vec![ClaimKindV1::Historical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([4; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([5; 16]),
            scores: vec![1],
        }],
        members: vec![EnsembleMemberV1 {
            id: EnsembleMemberIdV1([6; 16]),
            lens: PersonaLensV1::Historian,
            concern_root_digest: [7; 32],
            grounding: GroundingRefV1 {
                authority_digest: [8; 32],
                surface_digest: [9; 32],
                depth: SurfaceDepthV1::Activated,
            },
        }],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [10; 32],
            result_digest: [11; 32],
            all_orthogonal: true,
        },
        fuel: 2,
    }
}

#[test]
fn read_only_private_port_reopens_exact_deliberation_and_refuses_mutation() {
    let fixture = Fixture::new();
    {
        let mut service = DeliberationService::new(fixture.open());
        assert!(matches!(
            service
                .execute(CommandV1::Apply {
                    expected_revision: 0,
                    mutation: MutationV1::Admit(Box::new(plan())),
                })
                .unwrap(),
            ReplyV1::Accepted(OutcomeV1::Applied {
                revision: 1,
                replayed: false,
                ..
            })
        ));
    }

    let mut daemon = DeliberationDaemon::bind_new(
        &fixture.socket,
        DeliberationService::new(fixture.open()),
        ReadOnly,
        LimitsV1::default(),
        Duration::from_secs(3),
        8,
    )
    .unwrap();
    let worker = thread::spawn(move || daemon.serve_next().unwrap());
    let port = DeliberationUnixPort::connect(
        &fixture.socket,
        DeliberationClientLimitsV1 {
            max_frame_bytes: 64 * 1024,
            max_requests: 8,
            io_timeout: Duration::from_secs(3),
        },
    )
    .unwrap();
    let mut reader = DeliberationReadOnlyClient::new(port);
    assert!(matches!(
        reader.status().unwrap(),
        ReplyV1::Accepted(OutcomeV1::Status(StatusV1 {
            revision: 1,
            needs_reopen: false,
        }))
    ));
    let ReplyV1::Accepted(OutcomeV1::Deliberation(Some(view))) =
        reader.deliberation(DeliberationIdV1([1; 16])).unwrap()
    else {
        panic!("durable deliberation expected")
    };
    assert_eq!(view.plan.members[0].lens, PersonaLensV1::Historian);
    assert_eq!(view.plan.members[0].grounding.surface_digest, [9; 32]);
    assert_eq!(view.phase, DeliberationPhaseV1::Collecting);

    let port = reader.into_port();
    let mut mutation = DeliberationMutationClient::new(port);
    assert_eq!(
        mutation
            .apply(1, MutationV1::Admit(Box::new(plan())))
            .unwrap(),
        ReplyV1::Refused(RefusalV1::NotAuthorized)
    );
    let mut reader = DeliberationReadOnlyClient::new(mutation.into_port());
    assert!(matches!(
        reader.status().unwrap(),
        ReplyV1::Accepted(OutcomeV1::Status(StatusV1 { revision: 1, .. }))
    ));
    drop(reader);
    worker.join().unwrap();
}
