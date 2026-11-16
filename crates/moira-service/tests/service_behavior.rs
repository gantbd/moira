use moira_core::{
    CommandV1 as CoreCommandV1, Engine, Limits, WorkflowId, WorkflowRevision, WorkflowSnapshotV1,
};
use moira_host::{ApplyErrorV1, ApplyReceiptV1, CoordinatorStore};
use moira_protocol::{
    CommandV1, MessageRefV1, MutationV1, OutcomeV1, ParticipantRefV1 as ProtocolParticipant,
    RefusalV1 as ProtocolRefusal, ReplyV1, TransitionV1 as ProtocolTransition,
};
use moira_service::{AccessV1, Service};

struct Store {
    engine: Engine,
    fenced: bool,
}

impl Store {
    fn new() -> Self {
        Self {
            engine: Engine::try_new(Limits::default()).unwrap(),
            fenced: false,
        }
    }
}

impl CoordinatorStore for Store {
    type Error = ();

    fn revision(&self) -> WorkflowRevision {
        self.engine.revision()
    }

    fn needs_reopen(&self) -> bool {
        self.fenced
    }

    fn workflow(&self, workflow: WorkflowId) -> Option<WorkflowSnapshotV1> {
        self.engine.workflow(&workflow).copied()
    }

    fn apply(
        &mut self,
        expected: WorkflowRevision,
        command: CoreCommandV1,
    ) -> Result<ApplyReceiptV1, ApplyErrorV1<Self::Error>> {
        let before = self.engine.revision();
        let transition = self
            .engine
            .apply(expected, command)
            .map_err(ApplyErrorV1::Domain)?;
        Ok(ApplyReceiptV1 {
            storage_revision: self.engine.revision().0.saturating_sub(1),
            transition,
            replayed: before == self.engine.revision(),
        })
    }
}

fn admission() -> MutationV1 {
    MutationV1::AdmitMessage {
        message: MessageRefV1 {
            source: [1; 16],
            room_sequence: 2,
            message_sequence: 3,
        },
        recipient: ProtocolParticipant([4; 16]),
    }
}

#[test]
fn service_preserves_mutation_replay_refusal_and_read_models() {
    let mut service = Service::new(Store::new());
    let ReplyV1::Accepted(OutcomeV1::Applied {
        transition,
        replayed: false,
        ..
    }) = service
        .execute(CommandV1::Apply {
            expected_revision: 0,
            mutation: admission(),
        })
        .unwrap()
    else {
        panic!("new admission expected");
    };
    let ProtocolTransition::ChronosSubmissionPrepared { workflow, .. } = transition else {
        panic!("submission expected");
    };
    assert!(matches!(
        service.execute(CommandV1::Status).unwrap(),
        ReplyV1::Accepted(OutcomeV1::Status(status))
            if status.revision == 1 && !status.needs_reopen
    ));
    assert!(matches!(
        service.execute(CommandV1::Workflow { workflow }).unwrap(),
        ReplyV1::Accepted(OutcomeV1::Workflow(Some(snapshot)))
            if snapshot.workflow == workflow
    ));
    assert!(matches!(
        service
            .execute(CommandV1::Apply {
                expected_revision: 1,
                mutation: admission(),
            })
            .unwrap(),
        ReplyV1::Accepted(OutcomeV1::Applied { replayed: true, .. })
    ));
    assert_eq!(
        service
            .execute(CommandV1::Apply {
                expected_revision: 0,
                mutation: admission(),
            })
            .unwrap(),
        ReplyV1::Refused(ProtocolRefusal::RevisionConflict {
            expected: 0,
            actual: 1,
        })
    );
}

#[test]
fn authenticated_access_refuses_before_store_dispatch() {
    let mut service = Service::new(Store::new());
    assert_eq!(
        service
            .execute_authorized(
                AccessV1 {
                    read: true,
                    mutate: false,
                },
                CommandV1::Apply {
                    expected_revision: 0,
                    mutation: admission(),
                },
            )
            .unwrap(),
        ReplyV1::Refused(ProtocolRefusal::NotAuthorized)
    );
    assert!(matches!(
        service
            .execute_authorized(
                AccessV1 {
                    read: true,
                    mutate: false,
                },
                CommandV1::Status,
            )
            .unwrap(),
        ReplyV1::Accepted(OutcomeV1::Status(status)) if status.revision == 0
    ));
}
