#![forbid(unsafe_code)]

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use agora_actor_publication_port::{
        ActorPublicationAcceptedV1, ActorPublicationObservationRefusalV1,
        ActorPublicationObservationV1, ActorPublicationRefusalV1, ActorPublicationRequestV1,
        ActorPublicationWireClientV1,
    };
    use agora_local_client::{LocalCallErrorV1, ScheduledLocalClientV1};
    use agora_wire::WireLimits;
    use moira_agora::{PublicationLimitsV1, actor::ActorResponsePublisher};
    use moira_client::{ModelSagaMutationClient, ModelSagaServicePort};
    use moira_daimon::{ModelProposalV1, ModelResultV1};
    use moira_local_harness::{
        peer_router::{ActorFederatedPeerRouterV1, AgoraRouteV1, ExternalConversationRefV1},
        peer_turn::{
            PeerEffectOutcomeV1, PeerEffectPortV1, PeerReconcileOutcomeV1, PeerTurnOutcomeV1,
            execute_peer_turn, resume_peer_turn,
        },
    };
    use moira_protocol::{
        DispositionV1, EffectIdV1, ResultReceiptV1, TaskRefV1, WorkflowIdV1, model_saga as saga,
    };
    use moira_service::model_saga::ModelSagaService;
    use moira_sqlite::model_saga::{
        ModelSagaSqliteStore, ModelSagaStoreErrorV1, ModelSagaStoreLimitsV1,
    };
    use moira_vellum::{DocumentRouteV1, EditAdapter, ExternalDocumentRefV1};
    use std::{
        collections::BTreeMap,
        fs,
        io::{BufRead, BufReader, Read, Write},
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        process::{Child, ChildStdout, Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use vellum_adapter_local_authentication::{
        LocalCredentialClaimProviderV1, LocalCredentialStoreV1,
    };
    use vellum_client::Client as VellumClient;
    use vellum_daemon::AuthenticatedUnixConnector;
    use vellum_protocol::{OutcomeV1 as VellumOutcomeV1, ReplyV1 as VellumReplyV1};
    use vellum_transport::{FrameExchange, RemoteTransport, TransportError, TransportLimits};

    const WORKFLOW: WorkflowIdV1 = WorkflowIdV1([0x40; 16]);
    const AGORA_ROOM: [u8; 16] = [0x41; 16];
    const AGORA_REPLY_TO: [u8; 16] = [0x42; 16];
    const VELLUM_WORKSPACE: [u8; 16] = [0x43; 16];
    const VELLUM_DOCUMENT: [u8; 16] = [0x44; 16];

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct SagaPort(ModelSagaService<ModelSagaSqliteStore>);

    impl ModelSagaServicePort for SagaPort {
        type Error = ModelSagaStoreErrorV1;

        fn execute(&mut self, command: saga::CommandV1) -> Result<saga::ReplyV1, Self::Error> {
            self.0.execute(command)
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum AgoraFaultV1 {
        Transport(LocalCallErrorV1),
        CommittedReplyDiscarded,
    }

    struct DiscardAgoraReplyV1 {
        inner: ScheduledLocalClientV1,
        discard: bool,
    }

    impl ActorPublicationWireClientV1 for DiscardAgoraReplyV1 {
        type Error = AgoraFaultV1;

        fn publish_actor_wire(
            &mut self,
            request: ActorPublicationRequestV1,
        ) -> Result<Result<ActorPublicationAcceptedV1, ActorPublicationRefusalV1>, Self::Error>
        {
            let reply = self
                .inner
                .publish_actor_wire(request)
                .map_err(AgoraFaultV1::Transport)?;
            if self.discard && reply.is_ok() {
                self.discard = false;
                Err(AgoraFaultV1::CommittedReplyDiscarded)
            } else {
                Ok(reply)
            }
        }

        fn observe_actor_wire(
            &mut self,
            request: ActorPublicationRequestV1,
        ) -> Result<
            Result<ActorPublicationObservationV1, ActorPublicationObservationRefusalV1>,
            Self::Error,
        > {
            self.inner
                .observe_actor_wire(request)
                .map_err(AgoraFaultV1::Transport)
        }
    }

    fn agora_outcome_unknown(error: &AgoraFaultV1) -> bool {
        matches!(
            error,
            AgoraFaultV1::CommittedReplyDiscarded
                | AgoraFaultV1::Transport(LocalCallErrorV1::OutcomeUnknown)
        )
    }

    struct DiscardVellumReplyV1<X> {
        inner: X,
        discard: bool,
    }

    impl<X: FrameExchange> FrameExchange for DiscardVellumReplyV1<X> {
        fn exchange(
            &mut self,
            request: &[u8],
            max_reply_bytes: usize,
        ) -> Result<Vec<u8>, TransportError> {
            let discard = self.discard
                && matches!(
                    vellum_wire::decode_request_v1(request, vellum_wire::WireLimits::reference(),),
                    Ok(vellum_wire::WireRequestV1::DurableExecute(_))
                );
            let reply = self.inner.exchange(request, max_reply_bytes)?;
            if discard {
                self.discard = false;
                Err(TransportError::OutcomeUnknown)
            } else {
                Ok(reply)
            }
        }
    }

    struct PauseReconcileV1<P> {
        inner: P,
        pause_agora: bool,
        pause_vellum: bool,
    }

    impl<P: PeerEffectPortV1> PeerEffectPortV1 for PauseReconcileV1<P> {
        type Error = P::Error;

        fn execute(
            &mut self,
            effect: &saga::ProposalEffectV1,
        ) -> Result<PeerEffectOutcomeV1, Self::Error> {
            self.inner.execute(effect)
        }

        fn reconcile(
            &mut self,
            effect: &saga::ProposalEffectV1,
        ) -> Result<PeerReconcileOutcomeV1, Self::Error> {
            match &effect.proposal {
                saga::ProposalV1::AgoraMessage { .. } if self.pause_agora => {
                    self.pause_agora = false;
                    Ok(PeerReconcileOutcomeV1::Uncertain)
                }
                saga::ProposalV1::VellumEdit { .. } if self.pause_vellum => {
                    self.pause_vellum = false;
                    Ok(PeerReconcileOutcomeV1::Uncertain)
                }
                _ => self.inner.reconcile(effect),
            }
        }
    }

    struct Fixture {
        directory: PathBuf,
        source: PathBuf,
        aegis_socket: PathBuf,
        agora_socket: PathBuf,
        vellum_socket: PathBuf,
        vellum_control: PathBuf,
        vellum_credential: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "mpep-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
            let source = directory.join("source.txt");
            fs::write(&source, "alpha").unwrap();
            Self {
                source,
                aegis_socket: directory.join("aegis.sock"),
                agora_socket: directory.join("agora.sock"),
                vellum_socket: directory.join("vellum.sock"),
                vellum_control: directory.join("vellum-control.sock"),
                vellum_credential: directory.join("vellum-credential"),
                directory,
            }
        }

        fn open_saga(&self) -> SagaPort {
            let limits = moira_core::model_saga::LimitsV1 {
                max_sagas: 2,
                max_proposals_per_saga: 2,
                max_proposal_bytes: 4_096,
            };
            SagaPort(ModelSagaService::new(
                ModelSagaSqliteStore::open(
                    self.directory.join("saga.db"),
                    [0x51; 16],
                    ModelSagaStoreLimitsV1 {
                        codec: moira_core::model_saga::persistence::CodecLimitsV1 {
                            max_bytes: 64 * 1_024,
                            max_sagas: limits.max_sagas,
                            max_proposals_per_saga: limits.max_proposals_per_saga,
                            max_proposal_bytes: limits.max_proposal_bytes,
                        },
                        saga: limits,
                        max_database_pages: 64,
                    },
                )
                .unwrap(),
            ))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn required_program(name: &str) -> PathBuf {
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")))
    }

    fn effective_id(flag: &str) -> String {
        let output = Command::new("id").arg(flag).output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
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
                panic!("owner exited {status}: {stderr}");
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("owner did not create {}", socket.display());
    }

    fn start_aegis(fixture: &Fixture) -> Child {
        let mut child = Command::new(required_program("MOIRA_AEGIS_DAEMON_PROGRAM"))
            .arg("create")
            .arg(fixture.directory.join("authority"))
            .arg(&fixture.aegis_socket)
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
        wait_for_socket(&mut child, &fixture.aegis_socket);
        thread::sleep(Duration::from_millis(50));
        child
    }

    fn start_agora(fixture: &Fixture) -> Child {
        let now = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        let config = fixture.directory.join("agora.conf");
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
                fixture.directory.join("agora-store").display(),
                fixture.agora_socket.display(),
                fixture.aegis_socket.display(),
                now,
                now + 600_000,
            ),
        )
        .unwrap();
        let mut child = Command::new(required_program("MOIRA_AGORA_DAEMON_PROGRAM"))
            .arg(config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_for_socket(&mut child, &fixture.agora_socket);
        child
    }

    fn start_vellum(fixture: &Fixture) -> (Child, BufReader<ChildStdout>) {
        let program = required_program("MOIRA_VELLUM_DAEMON_PROGRAM");
        let provisioned = Command::new(&program)
            .arg("--provision-credential")
            .arg(&fixture.vellum_credential)
            .output()
            .unwrap();
        assert!(provisioned.status.success());
        let mut child = Command::new(program)
            .arg("--socket")
            .arg(&fixture.vellum_socket)
            .arg("--control")
            .arg(&fixture.vellum_control)
            .arg("--credential")
            .arg(&fixture.vellum_credential)
            .arg("--allow-edit")
            .arg("--allow-save")
            .arg("--")
            .arg(&fixture.source)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut ready = String::new();
        stdout.read_line(&mut ready).unwrap();
        if ready != "vellum-daemon ready\n" {
            let status = child.wait().unwrap();
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("Vellum owner exited {status}; stdout={ready:?}; stderr={stderr}");
        }
        (child, stdout)
    }

    fn vellum_connector(fixture: &Fixture) -> AuthenticatedUnixConnector {
        let record = LocalCredentialStoreV1::default()
            .load(&fixture.vellum_credential)
            .unwrap();
        AuthenticatedUnixConnector::with_provider_and_timeout(
            fixture.vellum_socket.clone(),
            LocalCredentialClaimProviderV1::new(record),
            Duration::from_secs(2),
        )
        .unwrap()
    }

    fn baseline(fixture: &Fixture) -> (u64, u64, u64) {
        let transport =
            RemoteTransport::new(vellum_connector(fixture), TransportLimits::reference()).unwrap();
        let mut client = VellumClient::connect(transport).unwrap();
        let inventory = match client.workspace_inventory().unwrap() {
            VellumReplyV1::Accepted(VellumOutcomeV1::WorkspaceInventory(value)) => value,
            other => panic!("workspace inventory expected: {other:?}"),
        };
        let snapshot = match client.snapshot().unwrap() {
            VellumReplyV1::Accepted(VellumOutcomeV1::Snapshot(value)) => value,
            other => panic!("initial snapshot expected: {other:?}"),
        };
        let replaced = match client
            .replace(snapshot.witness, 0, 5, "ready".into())
            .unwrap()
        {
            VellumReplyV1::Accepted(VellumOutcomeV1::Replaced(value)) => value,
            other => panic!("baseline replacement expected: {other:?}"),
        };
        assert!(matches!(
            client.save(replaced.witness).unwrap(),
            VellumReplyV1::Accepted(VellumOutcomeV1::Saved(_))
        ));
        let baseline = match client.snapshot().unwrap() {
            VellumReplyV1::Accepted(VellumOutcomeV1::Snapshot(value)) => value,
            other => panic!("saved baseline expected: {other:?}"),
        };
        client.disconnect().map_err(|(_, error)| error).unwrap();
        (
            inventory.workspace.get(),
            inventory.documents[0].get(),
            baseline.revision,
        )
    }

    fn result(vellum_revision: u64) -> ModelResultV1 {
        ModelResultV1 {
            receipt: ResultReceiptV1 {
                effect: EffectIdV1([0x61; 16]),
                execution_id: [0x62; 16],
                task: TaskRefV1 {
                    external_task_key: [0x63; 16],
                    incarnation: 1,
                },
                attempt_number: 1,
                disposition: DispositionV1::Completed,
            },
            evidence_digest: [0x64; 32],
            input_tokens: 1,
            output_tokens: 1,
            output: b"answer".to_vec(),
            proposals: vec![
                ModelProposalV1::AgoraMessage {
                    room: AGORA_ROOM,
                    reply_to: AGORA_REPLY_TO,
                    body: b"answer".to_vec(),
                },
                ModelProposalV1::VellumEdit {
                    workspace: VELLUM_WORKSPACE,
                    document: VELLUM_DOCUMENT,
                    expected_revision: vellum_revision,
                    replacement: b"omega".to_vec(),
                },
            ],
        }
    }

    fn proposals(vellum_revision: u64) -> Vec<saga::ProposalV1> {
        vec![
            saga::ProposalV1::AgoraMessage {
                room: AGORA_ROOM,
                reply_to: AGORA_REPLY_TO,
                body: b"answer".to_vec(),
            },
            saga::ProposalV1::VellumEdit {
                workspace: VELLUM_WORKSPACE,
                document: VELLUM_DOCUMENT,
                expected_revision: vellum_revision,
                expected_extent: 5,
                replacement: b"omega".to_vec(),
            },
        ]
    }

    fn router(
        fixture: &Fixture,
        vellum_workspace: u64,
        vellum_document: u64,
        discard_agora: bool,
        discard_vellum: bool,
        pause_agora_reconcile: bool,
        pause_vellum_reconcile: bool,
    ) -> impl PeerEffectPortV1<Error: std::fmt::Debug> {
        let publisher = ActorResponsePublisher::new(
            DiscardAgoraReplyV1 {
                inner: ScheduledLocalClientV1::new(
                    &fixture.agora_socket,
                    "bot",
                    WireLimits {
                        max_frame_bytes: 8_192,
                        max_text_bytes: 128,
                        max_fields: 512,
                        max_list_items: 32,
                    },
                    Duration::from_secs(2),
                ),
                discard: discard_agora,
            },
            PublicationLimitsV1::default(),
        )
        .unwrap();
        let vellum_transport = RemoteTransport::new(
            DiscardVellumReplyV1 {
                inner: vellum_connector(fixture),
                discard: discard_vellum,
            },
            TransportLimits::reference(),
        )
        .unwrap();
        let target = ExternalDocumentRefV1 {
            workspace: VELLUM_WORKSPACE,
            document: VELLUM_DOCUMENT,
        };
        let vellum = EditAdapter::connect(
            vellum_transport,
            BTreeMap::from([(
                target,
                DocumentRouteV1 {
                    workspace: vellum_protocol::WorkspaceIdV1::new(vellum_workspace).unwrap(),
                    document: vellum_protocol::WorkspaceDocumentIdV1::new(
                        vellum_protocol::WorkspaceIdV1::new(vellum_workspace).unwrap(),
                        vellum_document,
                    )
                    .unwrap(),
                },
            )]),
            1_024,
        )
        .unwrap();
        PauseReconcileV1 {
            inner: ActorFederatedPeerRouterV1::new(
                publisher,
                BTreeMap::from([(
                    ExternalConversationRefV1 {
                        room: AGORA_ROOM,
                        reply_to: AGORA_REPLY_TO,
                    },
                    AgoraRouteV1 {
                        source: [0x53; 16],
                        room_sequence: 1,
                        room_id: "room".into(),
                        expected_agora_revision: 2,
                        logical_time: 10,
                        addressed_to: vec!["bot".into()],
                    },
                )]),
                vellum,
                agora_outcome_unknown,
            ),
            pause_agora: pause_agora_reconcile,
            pause_vellum: pause_vellum_reconcile,
        }
    }

    struct NoPeerCalls;

    impl PeerEffectPortV1 for NoPeerCalls {
        type Error = ();

        fn execute(
            &mut self,
            _: &saga::ProposalEffectV1,
        ) -> Result<moira_local_harness::peer_turn::PeerEffectOutcomeV1, Self::Error> {
            panic!("completed replay must not execute a peer effect")
        }

        fn reconcile(
            &mut self,
            _: &saga::ProposalEffectV1,
        ) -> Result<PeerReconcileOutcomeV1, Self::Error> {
            panic!("completed replay must not reconcile a peer effect")
        }
    }

    #[test]
    fn both_peer_reply_losses_reconcile_in_one_exact_owner_workflow() {
        let fixture = Fixture::new();
        let mut aegis = start_aegis(&fixture);
        let mut agora = start_agora(&fixture);
        let (mut vellum, mut vellum_stdout) = start_vellum(&fixture);
        let (workspace, document, revision) = baseline(&fixture);

        let mut first_router = router(&fixture, workspace, document, true, false, true, false);
        let mut client = ModelSagaMutationClient::new(fixture.open_saga());
        let first = execute_peer_turn(
            &mut client,
            &mut first_router,
            None,
            WORKFLOW,
            [0x65; 32],
            &result(revision),
            proposals(revision),
        )
        .unwrap();
        assert!(matches!(first, PeerTurnOutcomeV1::Uncertain { .. }));
        drop(client);
        drop(first_router);

        let mut second_router = router(&fixture, workspace, document, false, true, false, true);
        let second =
            resume_peer_turn(&mut fixture.open_saga(), &mut second_router, WORKFLOW).unwrap();
        assert!(matches!(second, PeerTurnOutcomeV1::Uncertain { .. }));
        drop(second_router);

        let mut third_router = router(&fixture, workspace, document, false, false, false, false);
        let third =
            resume_peer_turn(&mut fixture.open_saga(), &mut third_router, WORKFLOW).unwrap();
        let PeerTurnOutcomeV1::Completed { saga, .. } = third else {
            panic!("both historically reconciled effects must complete the saga")
        };
        assert_eq!(saga.effects.len(), 2);
        assert!(matches!(
            saga.effects[0].state,
            saga::EffectStateV1::Succeeded(saga::PeerReceiptV1::AgoraMessage {
                agora_revision: 3,
                ..
            })
        ));
        assert!(matches!(
            saga.effects[1].state,
            saga::EffectStateV1::Succeeded(saga::PeerReceiptV1::VellumEdit {
                source_revision,
                durable: true,
                ..
            }) if source_revision == revision + 1
        ));
        drop(third_router);
        assert_eq!(fs::read_to_string(&fixture.source).unwrap(), "omega");

        let replay =
            resume_peer_turn(&mut fixture.open_saga(), &mut NoPeerCalls, WORKFLOW).unwrap();
        assert!(matches!(replay, PeerTurnOutcomeV1::Completed { .. }));

        agora.stdin.take().unwrap().write_all(&[1]).unwrap();
        assert!(agora.wait().unwrap().success());
        let mut agora_receipt = String::new();
        agora
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut agora_receipt)
            .unwrap();
        assert!(agora_receipt.contains("\"cleanup\":\"clean\""));

        let shutdown = Command::new(required_program("MOIRA_VELLUM_DAEMON_PROGRAM"))
            .arg("--shutdown")
            .arg(&fixture.vellum_control)
            .output()
            .unwrap();
        assert!(shutdown.status.success());
        assert!(vellum.wait().unwrap().success());
        let mut vellum_remainder = String::new();
        vellum_stdout.read_to_string(&mut vellum_remainder).unwrap();
        assert!(vellum_remainder.contains("vellum-daemon stopped"));

        assert!(
            Command::new("kill")
                .arg("-TERM")
                .arg(aegis.id().to_string())
                .status()
                .unwrap()
                .success()
        );
        assert!(aegis.wait().unwrap().success());
        assert!(!fixture.agora_socket.exists());
        assert!(!fixture.vellum_socket.exists());
        assert!(!fixture.vellum_control.exists());
        assert!(!fixture.aegis_socket.exists());
    }
}
