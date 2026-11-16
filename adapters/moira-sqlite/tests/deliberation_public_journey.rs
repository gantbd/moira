use moira_client::{
    DeliberationMutationClient, DeliberationReadOnlyClient, DeliberationServicePort,
};
use moira_core::deliberation::{
    DeliberationLimitsV1,
    registry::{RegistryLimitsV1, persistence::CodecLimitsV1},
};
use moira_protocol::deliberation::*;
use moira_service::deliberation::DeliberationService;
use moira_sqlite::deliberation::{
    DeliberationSqliteStore, DeliberationStoreErrorV1, DeliberationStoreLimitsV1,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Port(DeliberationService<DeliberationSqliteStore>);

impl DeliberationServicePort for Port {
    type Error = DeliberationStoreErrorV1;

    fn execute(&mut self, command: CommandV1) -> Result<ReplyV1, Self::Error> {
        self.0.execute(command)
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "moira-deliberation-public-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self(directory)
    }

    fn path(&self) -> PathBuf {
        self.0.join("deliberation.db")
    }

    fn open(&self) -> DeliberationSqliteStore {
        DeliberationSqliteStore::open(
            self.path(),
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
        for name in [
            "deliberation.db",
            "deliberation.db-journal",
            "deliberation.db.owner.lock",
        ] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
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
fn public_client_service_store_journey_survives_reopen() {
    let fixture = Fixture::new();
    {
        let mut client =
            DeliberationMutationClient::new(Port(DeliberationService::new(fixture.open())));
        assert!(matches!(
            client
                .apply(0, MutationV1::Admit(Box::new(plan())))
                .unwrap(),
            ReplyV1::Accepted(OutcomeV1::Applied {
                revision: 1,
                replayed: false,
                ..
            })
        ));
    }

    let mut reader =
        DeliberationReadOnlyClient::new(Port(DeliberationService::new(fixture.open())));
    assert!(matches!(
        reader.status().unwrap(),
        ReplyV1::Accepted(OutcomeV1::Status(StatusV1 {
            revision: 1,
            needs_reopen: false
        }))
    ));
    let ReplyV1::Accepted(OutcomeV1::Deliberation(Some(view))) =
        reader.deliberation(DeliberationIdV1([1; 16])).unwrap()
    else {
        panic!("durable public read expected");
    };
    assert_eq!(view.plan.method.machine_digest, [2; 32]);
    assert_eq!(view.plan.members[0].grounding.surface_digest, [9; 32]);
    assert_eq!(view.phase, DeliberationPhaseV1::Collecting);
}
