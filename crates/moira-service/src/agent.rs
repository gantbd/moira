//! Authorization-first service boundary for interactive agent sessions.

use crate::AccessV1;
use moira_host::AgentGateway;
use moira_protocol::agent;

pub struct AgentService<G> {
    gateway: G,
}

impl<G: AgentGateway> AgentService<G> {
    pub const fn new(gateway: G) -> Self {
        Self { gateway }
    }

    pub fn execute(&mut self, command: agent::CommandV1) -> Result<agent::ReplyV1, G::Error> {
        self.execute_authorized(
            AccessV1 {
                read: true,
                mutate: true,
            },
            command,
        )
    }

    pub fn execute_authorized(
        &mut self,
        access: AccessV1,
        command: agent::CommandV1,
    ) -> Result<agent::ReplyV1, G::Error> {
        let allowed = match command {
            agent::CommandV1::Events { .. } | agent::CommandV1::Inspect { .. } => access.read,
            agent::CommandV1::Open { .. }
            | agent::CommandV1::Prompt { .. }
            | agent::CommandV1::DecidePermission { .. }
            | agent::CommandV1::Advance { .. }
            | agent::CommandV1::Cancel { .. }
            | agent::CommandV1::Close { .. } => access.mutate,
        };
        if !allowed {
            return Ok(agent::ReplyV1::Refused(agent::RefusalV1::NotAuthorized));
        }
        self.gateway.execute_agent(command)
    }

    pub fn into_gateway(self) -> G {
        self.gateway
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Gateway {
        calls: usize,
    }

    impl AgentGateway for Gateway {
        type Error = ();

        fn execute_agent(
            &mut self,
            command: agent::CommandV1,
        ) -> Result<agent::ReplyV1, Self::Error> {
            self.calls += 1;
            let session = match command {
                agent::CommandV1::Inspect { session } | agent::CommandV1::Close { session, .. } => {
                    session
                }
                _ => panic!("unexpected command"),
            };
            Ok(agent::ReplyV1::Accepted(agent::OutcomeV1::OutcomeUnknown {
                session,
                idempotency_key: agent::IdempotencyKeyV1([3; 16]),
            }))
        }
    }

    #[test]
    fn refuses_before_gateway_dispatch() {
        let mut service = AgentService::new(Gateway { calls: 0 });
        let reply = service
            .execute_authorized(
                AccessV1 {
                    read: false,
                    mutate: false,
                },
                agent::CommandV1::Close {
                    session: agent::SessionIdV1([1; 16]),
                    expected_revision: 2,
                    idempotency_key: agent::IdempotencyKeyV1([3; 16]),
                    now: 4,
                },
            )
            .unwrap();
        assert_eq!(
            reply,
            agent::ReplyV1::Refused(agent::RefusalV1::NotAuthorized)
        );
        assert_eq!(service.into_gateway().calls, 0);
    }

    #[test]
    fn read_access_delegates_inspection() {
        let mut service = AgentService::new(Gateway { calls: 0 });
        let reply = service
            .execute_authorized(
                AccessV1 {
                    read: true,
                    mutate: false,
                },
                agent::CommandV1::Inspect {
                    session: agent::SessionIdV1([1; 16]),
                },
            )
            .unwrap();
        assert!(matches!(reply, agent::ReplyV1::Accepted(_)));
        assert_eq!(service.into_gateway().calls, 1);
    }
}
