use agora_client::v2::{PortFailureV2, ServicePortV2};
use agora_protocol::v2::{AdmissionPolicyV2, ClientIdV2, OutcomeV2, ReplyV2, RequestV2};
use moira_agora::room::{
    DurableRoomErrorV1, DurableRoomProvisioner, JourneyRoomErrorV1, JourneyRoomProgressV1,
    RoomCheckpointV1, RoomIntentV1, RoomLimitsV1, RoomRecordV1, RoomReplyV1, RoomStateStoreV1,
    provision_and_bind_journey_room,
};
use moira_core::{
    ChronosAttemptReceiptV1, ChronosTaskRefV1,
    agent_participation::ProfileIdV1,
    task_agent_journey::{self as journey, JourneyIdV1, OwnerIdV1},
};
use moira_host::{
    TaskAgentJourneyApplyErrorV1, TaskAgentJourneyApplyReceiptV1, TaskAgentJourneyApplyV1,
    TaskAgentJourneyCensusV1, TaskAgentJourneyMutationV1, TaskAgentJourneyStore,
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

#[derive(Clone)]
struct Store {
    record: Rc<RefCell<RoomRecordV1>>,
    writes: Rc<RefCell<Vec<RoomCheckpointV1>>>,
}

impl Store {
    fn new() -> Self {
        Self {
            record: Rc::new(RefCell::new(RoomRecordV1 {
                revision: 0,
                checkpoint: RoomCheckpointV1::initial(ClientIdV2(4)),
            })),
            writes: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl RoomStateStoreV1 for Store {
    type Error = &'static str;

    fn load(&mut self) -> Result<RoomRecordV1, Self::Error> {
        Ok(self.record.borrow().clone())
    }

    fn compare_exchange(
        &mut self,
        expected_revision: u64,
        checkpoint: &RoomCheckpointV1,
    ) -> Result<RoomRecordV1, Self::Error> {
        let mut current = self.record.borrow_mut();
        if current.revision != expected_revision {
            return Err("conflict");
        }
        current.revision += 1;
        current.checkpoint = checkpoint.clone();
        self.writes.borrow_mut().push(checkpoint.clone());
        Ok(current.clone())
    }
}

struct Port {
    requests: Rc<RefCell<Vec<RequestV2>>>,
    replies: VecDeque<Result<ReplyV2, PortFailureV2<&'static str>>>,
}

impl ServicePortV2 for Port {
    type Error = &'static str;

    fn execute(
        &mut self,
        _: ClientIdV2,
        request: RequestV2,
    ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
        self.requests.borrow_mut().push(request);
        self.replies.pop_front().unwrap_or(Ok(ReplyV2::Refused(
            agora_protocol::v2::RefusalV2::Unavailable,
        )))
    }
}

fn port(
    requests: Rc<RefCell<Vec<RequestV2>>>,
    replies: impl IntoIterator<Item = Result<ReplyV2, PortFailureV2<&'static str>>>,
) -> Port {
    Port {
        requests,
        replies: replies.into_iter().collect(),
    }
}

fn intent() -> RoomIntentV1 {
    RoomIntentV1 {
        journey: JourneyIdV1 {
            owner: OwnerIdV1([7; 16]),
            sequence: 4,
        },
        expected_journey_revision: 1,
        source: [7; 16],
        room_sequence: 4,
        request: agora_client::room_command::CreateRoomRequestV1 {
            room_id: "task-agent-4".into(),
            name: "Task agent 4".into(),
            admission: AdmissionPolicyV2::InviteOnly,
            expected_revision: 7,
        },
    }
}

#[test]
fn prepares_before_dispatch_and_replays_the_retained_owner_receipt() {
    let store = Store::new();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let mut provisioner = DurableRoomProvisioner::new(
        store.clone(),
        port(
            requests.clone(),
            [Ok(ReplyV2::Accepted(OutcomeV2::RoomCreated {
                revision: 8,
            }))],
        ),
        RoomLimitsV1::default(),
    )
    .unwrap();

    let completed = provisioner.provision(intent()).unwrap();
    assert_eq!(completed.storage_revision, 2);
    assert!(!completed.replayed);
    assert_eq!(
        completed.reply,
        RoomReplyV1::Created(agora_client::room_command::CreateRoomReceiptV1 {
            room_id: "task-agent-4".into(),
            revision: 8,
        })
    );
    let writes = store.writes.borrow();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].pending_intent.as_ref(), Some(&intent()));
    assert!(writes[0].client.pending.is_some());
    assert!(writes[1].pending_intent.is_none());
    assert!(writes[1].client.pending.is_none());
    drop(writes);

    let replay = provisioner.provision(intent()).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.storage_revision, 2);
    assert_eq!(requests.borrow().len(), 1);

    let mut changed = intent();
    changed.request.name = "Different room".into();
    assert_eq!(
        provisioner.provision(changed),
        Err(DurableRoomErrorV1::InvalidCheckpoint)
    );
    assert_eq!(requests.borrow().len(), 1);
}

#[test]
fn outcome_unknown_restarts_only_the_identical_prepared_request() {
    let store = Store::new();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let mut first = DurableRoomProvisioner::new(
        store.clone(),
        port(
            requests.clone(),
            [Err(PortFailureV2::OutcomeUnknown("lost"))],
        ),
        RoomLimitsV1::default(),
    )
    .unwrap();
    assert_eq!(
        first.provision(intent()),
        Err(DurableRoomErrorV1::Client(
            agora_client::v2::ClientErrorV2::OutcomeUnknown("lost")
        ))
    );
    assert_eq!(store.record.borrow().revision, 1);
    assert_eq!(store.writes.borrow().len(), 1);

    let mut changed = intent();
    changed.request.room_id = "other".into();
    assert_eq!(
        first.provision(changed),
        Err(DurableRoomErrorV1::InvalidCheckpoint)
    );
    assert_eq!(requests.borrow().len(), 1);

    let mut resumed = DurableRoomProvisioner::new(
        store,
        port(
            requests.clone(),
            [Ok(ReplyV2::Accepted(OutcomeV2::RoomCreated {
                revision: 8,
            }))],
        ),
        RoomLimitsV1::default(),
    )
    .unwrap();
    let completed = resumed.provision(intent()).unwrap();
    assert_eq!(completed.storage_revision, 2);
    assert_eq!(requests.borrow().len(), 2);
    assert_eq!(requests.borrow()[0], requests.borrow()[1]);
}

#[test]
fn refuses_a_room_not_owned_by_the_exact_journey_relationship() {
    let store = Store::new();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let mut provisioner =
        DurableRoomProvisioner::new(store, port(requests.clone(), []), RoomLimitsV1::default())
            .unwrap();
    let mut invalid = intent();
    invalid.room_sequence += 1;
    assert_eq!(
        provisioner.provision(invalid),
        Err(DurableRoomErrorV1::InvalidInput)
    );
    assert!(requests.borrow().is_empty());
}

struct JourneyStore {
    engine: journey::Engine,
    storage_revision: u64,
    fail_next_room_bind: bool,
}

impl TaskAgentJourneyStore for JourneyStore {
    type Error = &'static str;

    fn owner(&self) -> OwnerIdV1 {
        self.engine.checkpoint().owner
    }

    fn owner_revision(&self) -> u64 {
        self.engine.owner_revision()
    }

    fn storage_revision(&self) -> u64 {
        self.storage_revision
    }

    fn needs_reopen(&self) -> bool {
        false
    }

    fn catalogue(&self) -> Vec<journey::TemplateV1> {
        self.engine.checkpoint().templates
    }

    fn census(&self) -> TaskAgentJourneyCensusV1 {
        let checkpoint = self.engine.checkpoint();
        TaskAgentJourneyCensusV1 {
            templates: checkpoint.templates.len() as u64,
            previews: checkpoint.previews.len() as u64,
            journeys: checkpoint.journeys.len() as u64,
        }
    }

    fn preview(&self, id: journey::PreviewIdV1) -> Option<journey::PreviewSnapshotV1> {
        self.engine.preview_snapshot(id).copied()
    }

    fn journey(&self, id: JourneyIdV1) -> Option<journey::JourneySnapshotV1> {
        self.engine.journey(id).copied()
    }

    fn apply(
        &mut self,
        expected_storage_revision: Option<u64>,
        mutation: TaskAgentJourneyMutationV1,
    ) -> Result<TaskAgentJourneyApplyReceiptV1, TaskAgentJourneyApplyErrorV1<Self::Error>> {
        if expected_storage_revision != Some(self.storage_revision) {
            return Err(TaskAgentJourneyApplyErrorV1::Conflict {
                expected: expected_storage_revision,
                actual: self.storage_revision,
            });
        }
        if self.fail_next_room_bind
            && matches!(mutation, TaskAgentJourneyMutationV1::BindRoom { .. })
        {
            self.fail_next_room_bind = false;
            self.storage_revision += 1;
            return Err(TaskAgentJourneyApplyErrorV1::Conflict {
                expected: expected_storage_revision,
                actual: self.storage_revision,
            });
        }
        let before = self.engine.checkpoint();
        let outcome = match mutation {
            TaskAgentJourneyMutationV1::BindRoom { journey, receipt } => {
                TaskAgentJourneyApplyV1::Room(
                    self.engine
                        .bind_room(journey, receipt)
                        .map_err(TaskAgentJourneyApplyErrorV1::Domain)?,
                )
            }
            _ => return Err(TaskAgentJourneyApplyErrorV1::Backend("unsupported")),
        };
        if self.engine.checkpoint() != before {
            self.storage_revision += 1;
        }
        Ok(TaskAgentJourneyApplyReceiptV1 {
            storage_revision: self.storage_revision,
            outcome,
        })
    }
}

fn task_ready_journey_store(fail_next_room_bind: bool) -> (JourneyStore, JourneyIdV1) {
    let template = journey::TemplateV1 {
        id: journey::TemplateIdV1([2; 16]),
        version: 1,
        digest: [3; 32],
        work_mode: journey::WorkModeV1::Collaborator,
        profile: ProfileIdV1([4; 16]),
        room_policy: journey::RoomPolicyV1::SharedContext,
        budget: journey::BudgetV1 {
            max_context_bytes: 1_024,
            max_output_tokens: 256,
            max_elapsed_millis: 10_000,
        },
        grounding_policy_digest: [5; 32],
        authority_policy_digest: [6; 32],
        consequence_digest: [7; 32],
        retired: false,
    };
    let mut engine =
        journey::Engine::try_new(OwnerIdV1([1; 16]), journey::LimitsV1::default(), [template])
            .unwrap();
    let preview = engine
        .preview(
            journey::PreviewRequestV1 {
                operation: journey::PreviewOperationIdV1([8; 16]),
                template: journey::TemplateRefV1 {
                    id: template.id,
                    version: template.version,
                    digest: template.digest,
                },
                task: journey::TaskAdmissionV1 {
                    task: ChronosTaskRefV1::new([9; 16], 0),
                    chronos_revision: 11,
                    evidence_digest: [12; 32],
                },
                profile: template.profile,
                room_policy: template.room_policy,
                initiating_principal: journey::PrincipalIdV1([13; 16]),
                accountable_human: journey::PrincipalIdV1([14; 16]),
                objective_digest: [15; 32],
                acceptance_digest: [16; 32],
                current_evidence_digest: [17; 32],
                requested_ttl: 60,
            },
            1_000,
        )
        .unwrap()
        .preview;
    let admitted = engine
        .instantiate(
            journey::InstantiateRequestV1 {
                operation: journey::InstantiateOperationIdV1([18; 16]),
                preview: preview.id,
                expected_owner_revision: preview.owner_revision,
                confirmation: journey::HumanConfirmationV1 {
                    preview: preview.id,
                    accountable_human: preview.request.accountable_human,
                    consequence_digest: preview.consequence_digest,
                },
            },
            1_002,
        )
        .unwrap()
        .journey;
    engine
        .bind_task_attempt(
            admitted.id,
            ChronosAttemptReceiptV1::new(preview.request.task.task, 1, 12),
        )
        .unwrap();
    (
        JourneyStore {
            engine,
            storage_revision: 3,
            fail_next_room_bind,
        },
        admitted.id,
    )
}

fn journey_room_intent(journey: journey::JourneySnapshotV1) -> RoomIntentV1 {
    RoomIntentV1 {
        journey: journey.id,
        expected_journey_revision: journey.revision,
        source: journey.workflow.owner.0,
        room_sequence: journey.workflow.sequence,
        request: agora_client::room_command::CreateRoomRequestV1 {
            room_id: "task-agent-journey-0".into(),
            name: "Task agent journey 0".into(),
            admission: AdmissionPolicyV2::InviteOnly,
            expected_revision: 7,
        },
    }
}

#[test]
fn journey_bind_conflict_reuses_one_durable_agora_outcome() {
    let (mut journey_store, journey_id) = task_ready_journey_store(true);
    let intent = journey_room_intent(journey_store.journey(journey_id).unwrap());
    let room_store = Store::new();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let mut provisioner = DurableRoomProvisioner::new(
        room_store,
        port(
            requests.clone(),
            [Ok(ReplyV2::Accepted(OutcomeV2::RoomCreated {
                revision: 8,
            }))],
        ),
        RoomLimitsV1::default(),
    )
    .unwrap();

    let error =
        provision_and_bind_journey_room(&mut journey_store, &mut provisioner, 3, intent.clone())
            .unwrap_err();
    match error {
        JourneyRoomErrorV1::Bind {
            room,
            error:
                TaskAgentJourneyApplyErrorV1::Conflict {
                    expected: Some(3),
                    actual: 4,
                },
        } => {
            assert!(matches!(room.reply, RoomReplyV1::Created(_)));
            assert!(!room.replayed);
        }
        other => panic!("unexpected bind result: {other:?}"),
    }
    assert_eq!(requests.borrow().len(), 1);

    let recovered =
        provision_and_bind_journey_room(&mut journey_store, &mut provisioner, 4, intent.clone())
            .unwrap();
    let JourneyRoomProgressV1::Ready(recovered) = recovered else {
        panic!("ready room expected")
    };
    assert!(recovered.room.replayed);
    assert!(!recovered.journey_replayed);
    assert_eq!(recovered.journey_storage_revision, 5);
    assert_eq!(recovered.journey.phase, journey::PhaseV1::RoomReady);
    assert_eq!(requests.borrow().len(), 1);

    let replay =
        provision_and_bind_journey_room(&mut journey_store, &mut provisioner, 5, intent).unwrap();
    let JourneyRoomProgressV1::Ready(replay) = replay else {
        panic!("ready replay expected")
    };
    assert!(replay.room.replayed);
    assert!(replay.journey_replayed);
    assert_eq!(replay.journey_storage_revision, 5);
    assert_eq!(requests.borrow().len(), 1);
}
