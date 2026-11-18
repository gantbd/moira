use std::collections::VecDeque;

use moira_client::{
    AgentParticipationOpenClient, AgentParticipationServicePort, PrepareAgentOpenDispositionV1,
};
use moira_protocol::{WorkflowIdV1, agent_participation as p, deliberation as d};

struct Port {
    replies: VecDeque<p::ReplyV1>,
    commands: Vec<p::CommandV1>,
}

impl AgentParticipationServicePort for Port {
    type Error = &'static str;

    fn execute_participation(&mut self, command: p::CommandV1) -> Result<p::ReplyV1, Self::Error> {
        self.commands.push(command);
        self.replies.pop_front().ok_or("missing reply")
    }
}

fn admitted() -> p::ParticipationViewV1 {
    p::ParticipationViewV1 {
        binding: p::BindingV1 {
            participation: p::ParticipationIdV1([1; 16]),
            workflow: WorkflowIdV1([2; 16]),
            deliberation: d::DeliberationIdV1([3; 16]),
            member: d::EnsembleMemberIdV1([4; 16]),
            session: p::AgentSessionIdV1([5; 16]),
            profile: p::ProfileIdV1([6; 16]),
            attempt: p::AttemptV1 {
                task_key: [7; 16],
                incarnation: 8,
                attempt: 9,
            },
            method: d::MethodRefV1 {
                machine_digest: [10; 32],
                state_digest: [11; 32],
            },
            grounding: d::GroundingRefV1 {
                authority_digest: [12; 32],
                surface_digest: [13; 32],
                depth: d::SurfaceDepthV1::Reference,
            },
            persona: d::PersonaLensV1::Historian,
        },
        revision: 0,
        phase: p::PhaseV1::Admitted,
        session_revision: None,
        last_event_cursor: None,
        profile_document_digest: None,
        capabilities: p::CapabilitiesV1::default(),
        active_turn: None,
        permission: None,
        terminal: None,
        pending: None,
    }
}

fn accepted(outcome: p::OutcomeV1) -> p::ReplyV1 {
    p::ReplyV1::Accepted(Box::new(outcome))
}

#[test]
fn prepares_only_the_owner_derived_open_operation() {
    let view = admitted();
    let operation = p::prepared_open_operation(14, &view).unwrap();
    let mut applied = view.clone();
    applied.revision = 1;
    applied.pending = Some(operation);
    let port = Port {
        replies: VecDeque::from([accepted(p::OutcomeV1::Applied {
            storage_revision: 15,
            participation: applied.clone(),
            replayed: false,
        })]),
        commands: Vec::new(),
    };
    let mut client = AgentParticipationOpenClient::new(port);
    assert_eq!(
        client.prepare(view.binding.participation).unwrap(),
        PrepareAgentOpenDispositionV1::Prepared {
            storage_revision: 15,
            participation: applied,
            replayed: false,
        }
    );
    let port = client.into_port();
    assert_eq!(
        port.commands,
        vec![p::CommandV1::PrepareOpen {
            participation: view.binding.participation,
        }]
    );
}

#[test]
fn retained_pending_open_is_replay_evidence_without_another_apply() {
    let mut view = admitted();
    view.pending = p::prepared_open_operation(14, &view);
    view.revision = 1;
    let port = Port {
        replies: VecDeque::from([accepted(p::OutcomeV1::Applied {
            storage_revision: 15,
            participation: view.clone(),
            replayed: true,
        })]),
        commands: Vec::new(),
    };
    let mut client = AgentParticipationOpenClient::new(port);
    assert_eq!(
        client.prepare(view.binding.participation).unwrap(),
        PrepareAgentOpenDispositionV1::AlreadyPrepared {
            storage_revision: 15,
            participation: view,
        }
    );
    assert_eq!(
        client.into_port().commands,
        vec![p::CommandV1::PrepareOpen {
            participation: p::ParticipationIdV1([1; 16]),
        }]
    );
}
