#![cfg(unix)]

use moira_client::{
    AgentParticipationServicePort,
    unix::{AgentParticipationUnixPort, ParticipationClientLimitsV1},
};
use moira_core::agent_participation::{LimitsV1 as DomainLimits, persistence::CodecLimitsV1};
use moira_local_daemon::unix::{AgentParticipationAuthenticator, AgentParticipationDaemon};
use moira_protocol::{WorkflowIdV1, agent_participation as p, deliberation as d};
use moira_service::{
    AccessV1,
    agent_participation::{AgentParticipationService, PermissionAccessV1},
};
use moira_sqlite::agent_participation::{
    AgentParticipationSqliteStore, AgentParticipationStoreLimitsV1,
};
use moira_sqlite::agent_prompt::{AgentPromptSqliteStore, AgentPromptStoreLimitsV1};
use moira_wire::{
    LimitsV1, decode_agent_participation_reply, decode_ready, encode_agent_participation_request,
};
use std::{
    convert::Infallible,
    fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

struct Authentication {
    authorizations: Arc<AtomicUsize>,
}

impl AgentParticipationAuthenticator for Authentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AccessV1, Self::Error> {
        Ok(AccessV1 {
            read: true,
            mutate: true,
        })
    }

    fn authorize_permission(
        &mut self,
        selection: p::PermissionSelectionV1,
    ) -> Result<Option<PermissionAccessV1>, Self::Error> {
        self.authorizations.fetch_add(1, Ordering::Relaxed);
        Ok(Some(PermissionAccessV1 {
            decision: [41; 16],
            subject: [42; 16],
            capability_digest: selection.capability_digest,
            epoch: 43,
        }))
    }
}

struct WorkerAuthentication(Authentication);

impl AgentParticipationAuthenticator for WorkerAuthentication {
    type Error = Infallible;

    fn authenticate(&mut self, stream: &UnixStream) -> Result<AccessV1, Self::Error> {
        self.0.authenticate(stream)
    }

    fn authorize_permission(
        &mut self,
        selection: p::PermissionSelectionV1,
    ) -> Result<Option<PermissionAccessV1>, Self::Error> {
        self.0.authorize_permission(selection)
    }

    fn authorize_prompt_materialization(
        &mut self,
        _: p::PromptMaterializationRequestV1,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

fn path(kind: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "moira-agent-participation-{kind}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn limits() -> AgentParticipationStoreLimitsV1 {
    AgentParticipationStoreLimitsV1 {
        codec: CodecLimitsV1 {
            max_bytes: 65_536,
            domain: DomainLimits {
                max_participations: 4,
            },
        },
        max_database_pages: 64,
    }
}

fn frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut header = [0; 4];
    stream.read_exact(&mut header).unwrap();
    let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut bytes).unwrap();
    bytes
}

fn send(stream: &mut UnixStream, command: p::CommandV1) -> p::ReplyV1 {
    let bytes = encode_agent_participation_request(&command, LimitsV1::default()).unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
    decode_agent_participation_reply(&frame(stream), LimitsV1::default()).unwrap()
}

#[test]
fn ordinary_terminal_authentication_does_not_grant_prompt_materialization() {
    let mut authentication = Authentication {
        authorizations: Arc::new(AtomicUsize::new(0)),
    };
    assert!(
        !authentication
            .authorize_prompt_materialization(p::PromptMaterializationRequestV1 {
                participation: p::ParticipationIdV1([1; 16]),
                expected_revision: 2,
                prompt_digest: [3; 32],
            })
            .unwrap()
    );
}

#[test]
fn private_profile_authorizes_selection_but_rejects_forged_generic_permission() {
    let database = path("service.db");
    let socket = path("service.sock");
    let binding = binding();
    let permission = prime(&database, binding);
    let service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(&database, [1; 16], limits()).unwrap(),
    );
    let authorizations = Arc::new(AtomicUsize::new(0));
    let mut daemon = AgentParticipationDaemon::bind_new(
        &socket,
        service,
        Authentication {
            authorizations: Arc::clone(&authorizations),
        },
        LimitsV1::default(),
        Duration::from_secs(2),
        4,
    )
    .unwrap();
    let client_socket = socket.clone();
    let client = thread::spawn(move || {
        let mut stream = UnixStream::connect(client_socket).unwrap();
        assert_eq!(
            decode_ready(&frame(&mut stream), LimitsV1::default())
                .unwrap()
                .protocol_version,
            1
        );
        let confirmation = send(
            &mut stream,
            p::CommandV1::PermissionConfirmation {
                participation: binding.participation,
            },
        );
        let challenge = match confirmation {
            p::ReplyV1::Accepted(value) => match *value {
                p::OutcomeV1::PermissionConfirmation(Some(value)) => value,
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        };
        let mut stale_digest = challenge.confirmation_digest;
        stale_digest[0] ^= 1;
        let stale = send(
            &mut stream,
            p::CommandV1::ConfirmPermission(p::ConfirmedPermissionSelectionV1 {
                participation: binding.participation,
                confirmation_digest: stale_digest,
                option: p::PermissionOptionIdV1([32; 16]),
            }),
        );
        let selected = send(
            &mut stream,
            p::CommandV1::ConfirmPermission(p::ConfirmedPermissionSelectionV1 {
                participation: binding.participation,
                confirmation_digest: challenge.confirmation_digest,
                option: p::PermissionOptionIdV1([32; 16]),
            }),
        );
        let forged = send(
            &mut stream,
            p::CommandV1::Apply {
                expected_storage_revision: Some(5),
                mutation: Box::new(p::MutationV1::Prepare {
                    participation: binding.participation,
                    expected_revision: 5,
                    operation: p::OperationV1::DecidePermission {
                        key: p::IdempotencyKeyV1([33; 16]),
                        execution: p::ExecutionIdV1([36; 16]),
                        request: permission.request,
                        option: p::PermissionOptionIdV1([32; 16]),
                        authority: p::PermissionAuthorityV1 {
                            decision: p::AuthorityDecisionIdV1([34; 16]),
                            subject: p::AuthoritySubjectIdV1([35; 16]),
                            capability_digest: permission.capability_digest,
                            epoch: 1,
                        },
                        attempt: binding.attempt,
                    },
                }),
            },
        );
        (challenge, stale, selected, forged)
    });
    daemon.serve_next().unwrap();
    let (challenge, stale, selected, forged) = client.join().unwrap();
    assert_eq!(challenge.storage_revision, 4);
    assert_eq!(challenge.request, permission.request);
    assert_eq!(stale, p::ReplyV1::Refused(p::RefusalV1::RevisionConflict));
    assert!(matches!(
        selected,
        p::ReplyV1::Accepted(value)
            if matches!(*value, p::OutcomeV1::Applied { replayed: false, .. })
    ));
    assert_eq!(forged, p::ReplyV1::Refused(p::RefusalV1::NotAuthorized));
    assert_eq!(authorizations.load(Ordering::Relaxed), 1);
    let (mut service, _) = daemon.into_parts();
    let view = match service
        .execute(p::CommandV1::Participation {
            participation: binding.participation,
        })
        .unwrap()
    {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::Participation(Some(value)) => value,
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    };
    assert!(matches!(
        view.pending,
        Some(p::OperationV1::DecidePermission { authority, .. })
            if authority.decision == p::AuthorityDecisionIdV1([41; 16])
                && authority.subject == p::AuthoritySubjectIdV1([42; 16])
                && authority.epoch == 43
    ));
    drop(service);
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", database.display(), suffix));
    }
}

#[test]
fn governed_prompt_socket_commits_bytes_and_replays_server_derived_operation() {
    let database = path("prompt-service.db");
    let prompts = path("prompt-bytes.db");
    let socket = path("prompt-service.sock");
    let binding = binding();
    prime_ready(&database, binding);
    let service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(&database, [1; 16], limits()).unwrap(),
    );
    let prompt_store = AgentPromptSqliteStore::open(
        &prompts,
        [2; 16],
        AgentPromptStoreLimitsV1 {
            max_entries: 4,
            max_prompt_bytes: 128,
            max_database_pages: 64,
        },
    )
    .unwrap();
    let authorizations = Arc::new(AtomicUsize::new(0));
    let mut daemon = AgentParticipationDaemon::bind_new_with_prompt_store(
        &socket,
        service,
        WorkerAuthentication(Authentication {
            authorizations: Arc::clone(&authorizations),
        }),
        prompt_store,
        LimitsV1::default(),
        Duration::from_secs(2),
        4,
    )
    .unwrap();
    let client_socket = socket.clone();
    let client = thread::spawn(move || {
        let mut stream = UnixStream::connect(client_socket).unwrap();
        decode_ready(&frame(&mut stream), LimitsV1::default()).unwrap();
        let submission = p::PromptSubmissionV1 {
            participation: binding.participation,
            prompt: b"respond from this exact prompt".to_vec(),
        };
        let forged = send(
            &mut stream,
            p::CommandV1::Apply {
                expected_storage_revision: Some(2),
                mutation: Box::new(p::MutationV1::Prepare {
                    participation: binding.participation,
                    expected_revision: 2,
                    operation: p::OperationV1::Prompt {
                        key: p::IdempotencyKeyV1([81; 16]),
                        turn: p::AgentTurnIdV1([82; 16]),
                        execution: p::ExecutionIdV1([83; 16]),
                        prompt_digest: [84; 32],
                        attempt: binding.attempt,
                    },
                }),
            },
        );
        let first = send(&mut stream, p::CommandV1::SubmitPrompt(submission.clone()));
        let replay = send(&mut stream, p::CommandV1::SubmitPrompt(submission));
        let materialized = send(
            &mut stream,
            p::CommandV1::MaterializePrompt(p::PromptMaterializationRequestV1 {
                participation: binding.participation,
                expected_revision: 3,
                prompt_digest: p::agent_prompt_digest(b"respond from this exact prompt").unwrap(),
            }),
        );
        (forged, first, replay, materialized)
    });
    daemon.serve_next().unwrap();
    let (forged, first, replay, materialized) = client.join().unwrap();
    assert_eq!(forged, p::ReplyV1::Refused(p::RefusalV1::NotAuthorized));
    let operation = match first {
        p::ReplyV1::Accepted(value) => match *value {
            p::OutcomeV1::Applied {
                participation,
                replayed: false,
                ..
            } => participation.pending.unwrap(),
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    };
    assert!(matches!(
        replay,
        p::ReplyV1::Accepted(value)
            if matches!(&*value, p::OutcomeV1::Applied {
                participation,
                replayed: true,
                ..
            } if participation.pending == Some(operation))
    ));
    assert!(matches!(
        materialized,
        p::ReplyV1::Accepted(value)
            if matches!(&*value, p::OutcomeV1::PromptMaterialization(value)
                if value.participation == binding.participation
                    && value.participation_revision == 3
                    && value.prompt == b"respond from this exact prompt")
    ));
    assert_eq!(authorizations.load(Ordering::Relaxed), 0);
    let (_, _, mut prompt_store) = daemon.into_prompt_parts();
    let digest = p::agent_prompt_digest(b"respond from this exact prompt").unwrap();
    use moira_host::AgentPromptStoreV1;
    assert_eq!(
        prompt_store.get(digest).unwrap().unwrap(),
        b"respond from this exact prompt"
    );
    drop(prompt_store);
    for value in [&database, &prompts] {
        for suffix in ["", "-journal", ".owner.lock"] {
            let _ = fs::remove_file(format!("{}{}", value.display(), suffix));
        }
    }
}

#[test]
fn canonical_client_connects_to_real_private_participation_profile() {
    let database = path("client-service.db");
    let socket = path("client-service.sock");
    let service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(&database, [1; 16], limits()).unwrap(),
    );
    let mut daemon = AgentParticipationDaemon::bind_new(
        &socket,
        service,
        Authentication {
            authorizations: Arc::new(AtomicUsize::new(0)),
        },
        LimitsV1::default(),
        Duration::from_secs(2),
        1,
    )
    .unwrap();
    let client_socket = socket.clone();
    let client = thread::spawn(move || {
        let mut port = AgentParticipationUnixPort::connect(
            client_socket,
            ParticipationClientLimitsV1 {
                max_requests: 1,
                ..ParticipationClientLimitsV1::default()
            },
        )
        .unwrap();
        port.execute_participation(p::CommandV1::Status).unwrap()
    });
    daemon.serve_next().unwrap();
    assert!(matches!(
        client.join().unwrap(),
        p::ReplyV1::Accepted(value)
            if matches!(&*value, p::OutcomeV1::Status(p::StatusV1 {
                storage_revision: None,
                needs_reopen: false,
            }))
    ));
    drop(daemon);
    assert!(!socket.exists());
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", database.display(), suffix));
    }
}

fn prime_ready(database: &PathBuf, binding: p::BindingV1) {
    let mut service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(database, [1; 16], limits()).unwrap(),
    );
    apply(&mut service, None, p::MutationV1::Admit(binding));
    let open = p::OperationV1::Open {
        key: p::IdempotencyKeyV1([61; 16]),
        execution: p::ExecutionIdV1([62; 16]),
    };
    apply(
        &mut service,
        Some(0),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: 0,
            operation: open,
        },
    );
    apply(
        &mut service,
        Some(1),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 1,
            key: p::IdempotencyKeyV1([61; 16]),
            observation: observation(p::PhaseV1::Ready, 1, None, None),
        },
    );
}

fn prime(database: &PathBuf, binding: p::BindingV1) -> p::PermissionEvidenceV1 {
    let mut service = AgentParticipationService::new(
        AgentParticipationSqliteStore::open(database, [1; 16], limits()).unwrap(),
    );
    apply(&mut service, None, p::MutationV1::Admit(binding));
    let open = p::OperationV1::Open {
        key: p::IdempotencyKeyV1([20; 16]),
        execution: p::ExecutionIdV1([21; 16]),
    };
    apply(
        &mut service,
        Some(0),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: 0,
            operation: open,
        },
    );
    apply(
        &mut service,
        Some(1),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 1,
            key: p::IdempotencyKeyV1([20; 16]),
            observation: observation(p::PhaseV1::Ready, 1, None, None),
        },
    );
    let turn = p::AgentTurnIdV1([22; 16]);
    let prompt = p::OperationV1::Prompt {
        key: p::IdempotencyKeyV1([23; 16]),
        turn,
        execution: p::ExecutionIdV1([24; 16]),
        prompt_digest: [25; 32],
        attempt: binding.attempt,
    };
    apply(
        &mut service,
        Some(2),
        p::MutationV1::Prepare {
            participation: binding.participation,
            expected_revision: 2,
            operation: prompt,
        },
    );
    let permission = p::PermissionEvidenceV1 {
        request: p::PermissionRequestIdV1([26; 16]),
        turn,
        capability_digest: [27; 32],
        options_digest: [28; 32],
        option_count: 1,
        option_ids: [
            p::PermissionOptionIdV1([32; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
            p::PermissionOptionIdV1([0; 16]),
        ],
    };
    apply(
        &mut service,
        Some(3),
        p::MutationV1::Observe {
            participation: binding.participation,
            expected_revision: 3,
            key: p::IdempotencyKeyV1([23; 16]),
            observation: observation(
                p::PhaseV1::AwaitingPermission,
                2,
                Some(turn),
                Some(permission),
            ),
        },
    );
    drop(service);
    permission
}

fn apply(
    service: &mut AgentParticipationService<AgentParticipationSqliteStore>,
    revision: Option<u64>,
    mutation: p::MutationV1,
) {
    assert!(matches!(
        service
            .execute(p::CommandV1::Apply {
                expected_storage_revision: revision,
                mutation: Box::new(mutation),
            })
            .unwrap(),
        p::ReplyV1::Accepted(_)
    ));
}

fn binding() -> p::BindingV1 {
    p::BindingV1 {
        participation: p::ParticipationIdV1([2; 16]),
        workflow: WorkflowIdV1([3; 16]),
        deliberation: d::DeliberationIdV1([4; 16]),
        member: d::EnsembleMemberIdV1([5; 16]),
        session: p::AgentSessionIdV1([6; 16]),
        profile: p::ProfileIdV1([7; 16]),
        attempt: p::AttemptV1 {
            task_key: [8; 16],
            incarnation: 1,
            attempt: 1,
        },
        method: d::MethodRefV1 {
            machine_digest: [9; 32],
            state_digest: [10; 32],
        },
        grounding: d::GroundingRefV1 {
            authority_digest: [11; 32],
            surface_digest: [12; 32],
            depth: d::SurfaceDepthV1::Activated,
        },
        persona: d::PersonaLensV1::Researcher,
    }
}

fn observation(
    phase: p::PhaseV1,
    session_revision: u64,
    active_turn: Option<p::AgentTurnIdV1>,
    permission: Option<p::PermissionEvidenceV1>,
) -> p::ObservationV1 {
    p::ObservationV1 {
        phase,
        session_revision,
        active_turn,
        last_event_cursor: Some(session_revision),
        profile_document_digest: [30; 32],
        capabilities: p::CapabilitiesV1 {
            permissions: true,
            ..p::CapabilitiesV1::default()
        },
        permission,
        terminal: None,
    }
}
