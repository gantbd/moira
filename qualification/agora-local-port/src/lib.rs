#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use aegis_client::ServicePort as AegisServicePort;
    use aegis_protocol::{
        CommandV1 as AegisCommandV1, DecisionHandleV1, DecisionSummaryV1, EngineNamespaceV1,
        OutcomeV1 as AegisOutcomeV1, ReplyV1 as AegisReplyV1, SessionHandleV1,
    };
    use agora_actor_publication_port::{
        ActorPublicationAcceptedV1, ActorPublicationObservationRefusalV1,
        ActorPublicationObservationV1, ActorPublicationRefusalV1, ActorPublicationRequestV1,
        ActorPublicationWireClientV1,
    };
    use agora_aegis_verifier::{AegisContextV1, AegisVerifierV1};
    use agora_client::v2::ClientV2;
    use agora_durable::{DurableHostConfigV2, DurableHostV2};
    use agora_engine::{
        AdmissionPolicy, CommandV2 as EngineCommandV2, Engine, Limits, Participant, ParticipantKind,
    };
    use agora_host::v2::HostV2;
    use agora_host::v2::persistence::{HostCheckpointScopeV1, HostRecoveryLimitsV2};
    use agora_host::{HostLimits, HostTimeV1, VerifiedSubjectV1};
    use agora_local_client::{LocalCallErrorV1, ScheduledLocalClientV1};
    use agora_local_daemon::{
        AuthenticatorV1, DaemonCleanupV1, DaemonLimitsV1, DaemonV1, HostClockV1,
        ServerIncarnationV1,
    };
    use agora_protocol::v2::{CommandV2, ReplyV2};
    use agora_service::v2::persistence::ServiceRecoveryLimitsV2;
    use agora_service::v2::{ServiceLimitsV2, ServiceV2};
    use agora_storage_directory::{DirectoryCheckpointStoreV1, DirectoryLimitsV1};
    use agora_wire::WireLimits;
    use moira_agora::{
        PublicationIntentV1, PublicationLimitsV1,
        actor::{ActorObservationReplyV1, ActorPublisherErrorV1, ActorResponsePublisher},
    };
    use moira_agora_local::{AgoraUnixLimitsV1, connect_agora_v2};
    use moira_protocol::EffectIdV1;
    use std::{
        convert::Infallible,
        fs,
        io::{Read, Write},
        os::unix::{fs::PermissionsExt, net::UnixStream},
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    #[derive(Clone)]
    struct AuthorityPort;

    impl AegisServicePort for AuthorityPort {
        type Error = Infallible;

        fn execute(
            &mut self,
            _: SessionHandleV1,
            command: AegisCommandV1,
        ) -> Result<AegisReplyV1, Self::Error> {
            let AegisCommandV1::Authorize { action, .. } = command else {
                unreachable!("Agora host only requests current authorization")
            };
            Ok(AegisReplyV1::Accepted(AegisOutcomeV1::Authorized(
                DecisionSummaryV1 {
                    handle: DecisionHandleV1(1),
                    epoch: 0,
                    expires_at: 10_000,
                    engine: EngineNamespaceV1::Agora,
                    action,
                },
            )))
        }
    }

    struct Authentication;

    impl AuthenticatorV1 for Authentication {
        type Error = Infallible;

        fn authenticate(&mut self, _: &UnixStream) -> Result<AegisContextV1, Self::Error> {
            Ok(AegisContextV1::new(
                "bot",
                VerifiedSubjectV1::from_verifier([2; 16]),
                SessionHandleV1(3),
                aegis_protocol::GrantHandleV1(4),
            ))
        }
    }

    struct Clock(u64);

    impl HostClockV1 for Clock {
        fn now(&mut self) -> HostTimeV1 {
            let value = self.0;
            self.0 += 1;
            HostTimeV1(value)
        }
    }

    fn wire_limits() -> WireLimits {
        WireLimits {
            max_frame_bytes: 8_192,
            max_text_bytes: 128,
            max_fields: 512,
            max_list_items: 32,
        }
    }

    fn path() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        PathBuf::from(format!(
            "/tmp/moira-agora-port-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn durable_config() -> DurableHostConfigV2 {
        const MAX_SNAPSHOT_BYTES: usize = 8 * 1_024 * 1_024;
        let host_limits = HostLimits {
            max_sessions: 4,
            max_retired_sessions: 8,
            wire: wire_limits(),
        };
        DurableHostConfigV2::new(
            "moira-agora-actor-qualification",
            HostCheckpointScopeV1::new([0x41; 16]),
            MAX_SNAPSHOT_BYTES,
            HostRecoveryLimitsV2 {
                max_snapshot_bytes: MAX_SNAPSHOT_BYTES,
                max_service_snapshot_bytes: MAX_SNAPSHOT_BYTES,
                maximum_host_limits: host_limits,
                service: ServiceRecoveryLimitsV2 {
                    max_snapshot_bytes: MAX_SNAPSHOT_BYTES,
                    max_engine_snapshot_bytes: MAX_SNAPSHOT_BYTES,
                    maximum_service_limits: ServiceLimitsV2::default(),
                    maximum_engine_limits: Limits::default(),
                },
            },
        )
    }

    fn actor_host() -> HostV2<AegisVerifierV1<AuthorityPort>> {
        let mut engine = Engine::new();
        for (id, name, kind) in [
            ("human", "Human", ParticipantKind::Human),
            ("bot", "Bot", ParticipantKind::Agent),
        ] {
            engine
                .apply(EngineCommandV2::RegisterParticipant(Participant::new(
                    id, name, kind,
                )))
                .unwrap();
        }
        engine
            .apply(EngineCommandV2::CreateRoom {
                room_id: "room".into(),
                name: "Room".into(),
                owner: "human".into(),
                admission: AdmissionPolicy::Open,
            })
            .unwrap();
        engine
            .apply(EngineCommandV2::Join {
                room_id: "room".into(),
                participant: "bot".into(),
            })
            .unwrap();
        HostV2::new(
            ServiceV2::new(engine, ServiceLimitsV2::default()),
            AegisVerifierV1::new(AuthorityPort, [5; 16]),
            HostLimits {
                max_sessions: 4,
                max_retired_sessions: 8,
                wire: wire_limits(),
            },
        )
    }

    #[test]
    fn agora_actor_process_helper() {
        if std::env::var_os("MOIRA_AGORA_ACTOR_HELPER").is_none() {
            return;
        }
        let socket = PathBuf::from(std::env::var_os("MOIRA_AGORA_SOCKET").unwrap());
        let store_path = PathBuf::from(std::env::var_os("MOIRA_AGORA_STORE").unwrap());
        let store =
            DirectoryCheckpointStoreV1::create_new(store_path, DirectoryLimitsV1::default())
                .unwrap();
        let durable = DurableHostV2::initialize(actor_host(), store, durable_config()).unwrap();
        let mut daemon = DaemonV1::bind_new_durable_composition(
            socket,
            durable,
            Authentication,
            Clock(1),
            ServerIncarnationV1::new([6; 16]),
            DaemonLimitsV1 {
                max_frame_bytes: 8_192,
                max_frames_after_connect: 12,
                io_timeout: Duration::from_secs(2),
            },
        )
        .unwrap();
        daemon.serve_next().unwrap();
        daemon.serve_next().unwrap();
        assert_eq!(daemon.stop().cleanup, DaemonCleanupV1::Clean);
    }

    fn wait_for_socket(child: &mut Child, socket: &Path) {
        for _ in 0..200 {
            if socket.exists() {
                return;
            }
            if let Some(status) = child.try_wait().unwrap() {
                let mut stderr = String::new();
                child
                    .stderr
                    .as_mut()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                panic!("Agora helper exited {status}: {stderr}");
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("Agora helper did not create its socket");
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum DiscardedReplyErrorV1 {
        CommittedReplyDiscarded,
        Transport(LocalCallErrorV1),
    }

    struct DiscardCommittedReplyV1 {
        client: ScheduledLocalClientV1,
        discard_once: bool,
    }

    impl ActorPublicationWireClientV1 for DiscardCommittedReplyV1 {
        type Error = DiscardedReplyErrorV1;

        fn publish_actor_wire(
            &mut self,
            request: ActorPublicationRequestV1,
        ) -> Result<Result<ActorPublicationAcceptedV1, ActorPublicationRefusalV1>, Self::Error>
        {
            let reply = self
                .client
                .publish_actor_wire(request)
                .map_err(DiscardedReplyErrorV1::Transport)?;
            if self.discard_once && reply.is_ok() {
                self.discard_once = false;
                return Err(DiscardedReplyErrorV1::CommittedReplyDiscarded);
            }
            Ok(reply)
        }

        fn observe_actor_wire(
            &mut self,
            request: ActorPublicationRequestV1,
        ) -> Result<
            Result<ActorPublicationObservationV1, ActorPublicationObservationRefusalV1>,
            Self::Error,
        > {
            self.client
                .observe_actor_wire(request)
                .map_err(DiscardedReplyErrorV1::Transport)
        }
    }

    fn actor_intent() -> PublicationIntentV1 {
        PublicationIntentV1 {
            effect: EffectIdV1([0x31; 16]),
            source: [0x32; 16],
            room_sequence: 5,
            room_id: "room".into(),
            expected_agora_revision: 4,
            logical_time: 23,
            body: "answer".into(),
            addressed_to: vec!["human".into()],
        }
    }

    fn required_program(name: &str) -> PathBuf {
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")))
    }

    fn exact_owner_intent() -> PublicationIntentV1 {
        PublicationIntentV1 {
            effect: EffectIdV1([0x51; 16]),
            source: [0x52; 16],
            room_sequence: 8,
            room_id: "room".into(),
            expected_agora_revision: 2,
            logical_time: 29,
            body: "standalone answer".into(),
            addressed_to: vec!["bot".into()],
        }
    }

    fn effective_id(flag: &str) -> String {
        let output = Command::new("id").arg(flag).output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn lost_response_reply_is_observed_through_exact_agora_and_aegis_standalone_owners() {
        let directory = path();
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let aegis_socket = directory.join("aegis.sock");
        let agora_socket = directory.join("agora.sock");
        let mut aegis = Command::new(required_program("MOIRA_AEGIS_DAEMON_PROGRAM"))
            .arg("create")
            .arg(directory.join("authority"))
            .arg(&aegis_socket)
            .arg("01010101010101010101010101010101")
            .arg(effective_id("-u"))
            .arg(effective_id("-g"))
            .arg("02020202020202020202020202020202")
            .arg("0303030303030303")
            .arg("administrator")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_for_socket(&mut aegis, &aegis_socket);
        // Socket publication precedes the daemon's first blocking accept by a
        // few scheduler instructions; do not make that scheduling race part of
        // Agora's authority-startup evidence.
        thread::sleep(Duration::from_millis(50));

        let config = directory.join("agora.conf");
        let grant_now = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        fs::write(
            &config,
            format!(
                concat!(
                    "schema=1\nmode=create\nstore={}\nsocket={}\naegis_socket={}\n",
                    "participant=bot\nsubject=02020202020202020202020202020202\n",
                    "resource=07070707070707070707070707070707\nauthority_epoch=0\n",
                    "grant_now={}\ngrant_expires_at={}\ngrant_profile=scheduled\n",
                    "host_scope=08080808080808080808080808080808\n",
                    "server_incarnation=09090909090909090909090909090909\n",
                    "display_name=Bot\nparticipant_kind=agent\nroom_id=room\n",
                    "room_name=Room\nadmission=open\n"
                ),
                directory.join("agora-store").display(),
                agora_socket.display(),
                aegis_socket.display(),
                grant_now,
                grant_now + 600_000,
            ),
        )
        .unwrap();
        let mut agora = Command::new(required_program("MOIRA_AGORA_DAEMON_PROGRAM"))
            .arg(&config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_for_socket(&mut agora, &agora_socket);

        let first = ScheduledLocalClientV1::new(
            &agora_socket,
            "bot",
            wire_limits(),
            Duration::from_secs(2),
        );
        let mut publisher = ActorResponsePublisher::new(
            DiscardCommittedReplyV1 {
                client: first,
                discard_once: true,
            },
            PublicationLimitsV1::default(),
        )
        .unwrap();
        let first_result = publisher.publish(&exact_owner_intent());
        if first_result
            != Err(ActorPublisherErrorV1::Transport(
                DiscardedReplyErrorV1::CommittedReplyDiscarded,
            ))
        {
            let status = agora.try_wait().unwrap();
            let detail = if status.is_some() {
                let mut stderr = String::new();
                agora
                    .stderr
                    .as_mut()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                stderr
            } else {
                String::new()
            };
            panic!("unexpected first publication {first_result:?}; owner={status:?}: {detail}");
        }
        drop(publisher);

        let second = ScheduledLocalClientV1::new(
            &agora_socket,
            "bot",
            wire_limits(),
            Duration::from_secs(2),
        );
        let mut publisher =
            ActorResponsePublisher::new(second, PublicationLimitsV1::default()).unwrap();
        let ActorObservationReplyV1::Published(observed) =
            publisher.observe(&exact_owner_intent()).unwrap()
        else {
            panic!("standalone Agora owner must retain the actor publication")
        };
        assert_eq!(observed.receipt.effect, exact_owner_intent().effect);
        assert_eq!(observed.receipt.agora_revision, 3);

        agora.stdin.take().unwrap().write_all(&[1]).unwrap();
        assert!(agora.wait().unwrap().success());
        let mut receipt = String::new();
        agora
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut receipt)
            .unwrap();
        assert!(receipt.contains("\"cleanup\":\"clean\""));
        assert!(!agora_socket.exists());

        assert!(
            Command::new("kill")
                .arg("-TERM")
                .arg(aegis.id().to_string())
                .status()
                .unwrap()
                .success()
        );
        assert!(aegis.wait().unwrap().success());
        assert!(!aegis_socket.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn lost_response_reply_is_observed_by_a_new_client_in_an_independent_agora_process() {
        let directory = path();
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.join("agora.sock");
        let store = directory.join("store");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("tests::agora_actor_process_helper")
            .arg("--exact")
            .arg("--nocapture")
            .env("MOIRA_AGORA_ACTOR_HELPER", "1")
            .env("MOIRA_AGORA_SOCKET", &socket)
            .env("MOIRA_AGORA_STORE", &store)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_for_socket(&mut child, &socket);

        let first =
            ScheduledLocalClientV1::new(&socket, "bot", wire_limits(), Duration::from_secs(2));
        let mut publisher = ActorResponsePublisher::new(
            DiscardCommittedReplyV1 {
                client: first,
                discard_once: true,
            },
            PublicationLimitsV1::default(),
        )
        .unwrap();
        assert_eq!(
            publisher.publish(&actor_intent()),
            Err(ActorPublisherErrorV1::Transport(
                DiscardedReplyErrorV1::CommittedReplyDiscarded
            ))
        );
        drop(publisher);

        let second =
            ScheduledLocalClientV1::new(&socket, "bot", wire_limits(), Duration::from_secs(2));
        let mut publisher =
            ActorResponsePublisher::new(second, PublicationLimitsV1::default()).unwrap();
        let ActorObservationReplyV1::Published(observed) =
            publisher.observe(&actor_intent()).unwrap()
        else {
            panic!("durably retained publication expected")
        };
        assert_eq!(observed.receipt.effect, actor_intent().effect);
        assert_eq!(observed.receipt.message.source, actor_intent().source);
        assert_eq!(observed.receipt.message.room_sequence, 5);
        assert_eq!(observed.receipt.message.message_sequence, 0);
        assert_eq!(observed.receipt.agora_revision, 5);

        let status = child.wait().unwrap();
        if !status.success() {
            let mut stderr = String::new();
            child
                .stderr
                .as_mut()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("Agora helper exited {status}: {stderr}");
        }
        assert!(!socket.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn public_port_crosses_the_real_agora_local_daemon() {
        let directory = path();
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.join("agora.sock");
        let mut engine = Engine::new();
        engine
            .apply(EngineCommandV2::RegisterParticipant(Participant::new(
                "bot",
                "Bot",
                ParticipantKind::Agent,
            )))
            .unwrap();
        let host = HostV2::new(
            ServiceV2::new(engine, ServiceLimitsV2::default()),
            AegisVerifierV1::new(AuthorityPort, [5; 16]),
            HostLimits {
                max_sessions: 2,
                max_retired_sessions: 4,
                wire: wire_limits(),
            },
        );
        let mut daemon = DaemonV1::bind_new(
            &socket,
            host,
            Authentication,
            Clock(1),
            ServerIncarnationV1::new([6; 16]),
            DaemonLimitsV1 {
                max_frame_bytes: 8_192,
                max_frames_after_connect: 4,
                io_timeout: Duration::from_secs(2),
            },
        )
        .unwrap();
        let server = thread::spawn(move || {
            let served = daemon.serve_next();
            (served, daemon.stop())
        });

        let (client, port) = connect_agora_v2(
            &socket,
            "bot",
            AgoraUnixLimitsV1 {
                wire: wire_limits(),
                timeout: Duration::from_secs(2),
            },
        )
        .unwrap();
        let mut client_port = ClientV2::new(client, port);
        assert!(matches!(
            client_port.execute(CommandV2::Capabilities),
            Ok(ReplyV2::Accepted(_))
        ));
        let (port, pending) = client_port.into_parts();
        assert!(pending.is_none());
        port.disconnect().unwrap();

        let (served, stopped) = server.join().unwrap();
        served.unwrap();
        assert_eq!(stopped.cleanup, DaemonCleanupV1::Clean);
        fs::remove_dir(directory).unwrap();
    }
}
