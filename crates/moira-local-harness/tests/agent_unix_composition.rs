#![cfg(unix)]

use daimon_host::AuthenticatedSessionV1;
use daimon_local_daemon::{
    AgentServicePort,
    unix::{AgentDaemon, AgentSessionPort, Authenticator},
};
use daimon_protocol::{SessionHandleV1, agent as upstream};
use daimon_wire::{LimitsV1, agent::AgentWireLimitsV1};
use moira_host::AgentGateway;
use moira_local_harness::unix::connect_daimon_agent;
use moira_protocol::agent as public;
use std::{
    convert::Infallible,
    fs,
    os::unix::net::UnixStream,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

struct Authentication;

impl Authenticator for Authentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AuthenticatedSessionV1, Self::Error> {
        Ok(AuthenticatedSessionV1 {
            principal: [4; 16],
            authority_context: [9; 16],
        })
    }
}

struct Port;

impl AgentServicePort for Port {
    type Error = Infallible;

    fn execute_agent(
        &mut self,
        session: SessionHandleV1,
        command: upstream::AgentCommandV1,
    ) -> Result<upstream::AgentReplyV1, Self::Error> {
        assert_eq!(session, SessionHandleV1(17));
        assert!(matches!(command, upstream::AgentCommandV1::Inspect { .. }));
        Ok(upstream::AgentReplyV1::Refused(
            upstream::AgentRefusalV1::AttemptDenied,
        ))
    }
}

impl AgentSessionPort for Port {
    fn open_agent_service_session(
        &mut self,
        _: AuthenticatedSessionV1,
    ) -> Result<SessionHandleV1, upstream::AgentRefusalV1> {
        Ok(SessionHandleV1(17))
    }

    fn close_agent_service_session(&mut self, session: SessionHandleV1) -> bool {
        assert_eq!(session, SessionHandleV1(17));
        true
    }
}

#[test]
fn moira_agent_gateway_uses_daimons_real_private_agent_socket() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "moira-daimon-agent-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let socket = directory.join("agent.sock");
    let daemon = AgentDaemon::bind_new(
        &socket,
        Port,
        Authentication,
        LimitsV1::default(),
        AgentWireLimitsV1::default(),
        Duration::from_secs(2),
        1,
    )
    .unwrap();
    let server = thread::spawn(move || {
        let mut daemon = daemon;
        daemon.serve_next().unwrap();
    });

    let mut gateway = connect_daimon_agent(
        &socket,
        LimitsV1::default(),
        AgentWireLimitsV1::default(),
        Duration::from_secs(2),
    )
    .unwrap();
    let reply = gateway
        .execute_agent(public::CommandV1::Inspect {
            session: public::SessionIdV1([8; 16]),
        })
        .unwrap();
    assert_eq!(
        reply,
        public::ReplyV1::Refused(public::RefusalV1::AttemptDenied)
    );
    drop(gateway);
    server.join().unwrap();
    assert!(!socket.exists());
    fs::remove_dir(directory).unwrap();
}
