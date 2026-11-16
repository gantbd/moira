#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use aegis_client::ServicePort as AegisServicePort;
    use aegis_protocol::{
        CommandV1 as AegisCommandV1, DecisionHandleV1, DecisionSummaryV1, EngineNamespaceV1,
        OutcomeV1 as AegisOutcomeV1, ReplyV1 as AegisReplyV1, SessionHandleV1,
    };
    use agora_aegis_verifier::{AegisContextV1, AegisVerifierV1};
    use agora_client::v2::ClientV2;
    use agora_engine::{CommandV2 as EngineCommandV2, Engine, Participant, ParticipantKind};
    use agora_host::v2::HostV2;
    use agora_host::{HostLimits, HostTimeV1, VerifiedSubjectV1};
    use agora_local_daemon::{
        AuthenticatorV1, DaemonCleanupV1, DaemonLimitsV1, DaemonV1, HostClockV1,
        ServerIncarnationV1,
    };
    use agora_protocol::v2::{CommandV2, ReplyV2};
    use agora_service::v2::{ServiceLimitsV2, ServiceV2};
    use agora_wire::WireLimits;
    use moira_agora_local::{AgoraUnixLimitsV1, connect_agora_v2};
    use std::{
        convert::Infallible,
        fs,
        os::unix::{fs::PermissionsExt, net::UnixStream},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::Duration,
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
