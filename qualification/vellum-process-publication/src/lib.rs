#![forbid(unsafe_code)]

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use moira_protocol::EffectIdV1;
    use moira_vellum::{
        DocumentRouteV1, EditAdapter, EditIntentV1, EditReplyV1, ExternalDocumentRefV1,
    };
    use std::{
        collections::BTreeMap,
        fs,
        io::Read,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::Duration,
    };
    use vellum_adapter_local_fs::{
        FilesystemLimits, LocalFilesystemFactory, LocalWorkspaceIdentityMetadataV1,
    };
    use vellum_client::Client as VellumClient;
    use vellum_daemon::{
        AuthenticatedUnixConnector, DaemonLimits, LocalAuthenticationClaimV1, LocalDaemon,
    };
    use vellum_editor::Workspace;
    use vellum_host::{
        authentication::{
            AuthenticationDecisionV1, AuthenticationEvidenceV1, AuthenticationPortV1,
            AuthenticationRequestV1,
        },
        authority::{AuthorityActionSetV1, AuthorityActionV1, AuthoritySubjectRefV1},
        workspace::{WorkspaceIdentityMetadataPortV1, WorkspaceIdentityRecoveryV1},
    };
    use vellum_protocol::{WorkspaceDocumentIdV1, WorkspaceIdV1};
    use vellum_transport::{FrameExchange, RemoteTransport, TransportError, TransportLimits};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    const CLAIM: [u8; 16] = [0x41; 16];
    const SUBJECT: [u8; 16] = [0x42; 16];

    struct Authentication;

    impl AuthenticationPortV1 for Authentication {
        type Context = ();

        fn authenticate(
            &mut self,
            request: AuthenticationRequestV1,
            (): &Self::Context,
        ) -> AuthenticationDecisionV1 {
            if request.claim != vellum_host::authentication::AuthenticationClaimRefV1::new(CLAIM) {
                return AuthenticationDecisionV1::Rejected;
            }
            AuthenticationDecisionV1::Authenticated(AuthenticationEvidenceV1::from_port(
                request,
                AuthoritySubjectRefV1::new(SUBJECT),
            ))
        }
    }

    fn limits() -> DaemonLimits {
        DaemonLimits {
            io_timeout: Duration::from_secs(2),
            request_latency: None,
            ..DaemonLimits::reference()
        }
    }

    fn workspace(path: &Path) -> Workspace {
        Workspace::open(
            &[path.to_path_buf()],
            vellum_editor::reference_workspace_profile(
                LocalFilesystemFactory::new(FilesystemLimits {
                    max_resources: 8,
                    max_bytes: 4_096,
                    max_stage_attempts: 8,
                    max_metadata_bytes: 4_096,
                })
                .erase(),
            ),
        )
        .unwrap()
    }

    #[test]
    fn vellum_process_helper() {
        if std::env::var_os("MOIRA_VELLUM_HELPER").is_none() {
            return;
        }
        let socket = PathBuf::from(std::env::var_os("MOIRA_VELLUM_SOCKET").unwrap());
        let source = PathBuf::from(std::env::var_os("MOIRA_VELLUM_SOURCE").unwrap());
        let identity = PathBuf::from(std::env::var_os("MOIRA_VELLUM_IDENTITY").unwrap());
        let actions = AuthorityActionSetV1::read_only()
            .join(AuthorityActionSetV1::singleton(
                AuthorityActionV1::EditSource,
            ))
            .join(AuthorityActionSetV1::singleton(
                AuthorityActionV1::PersistSource,
            ));
        let mut daemon = LocalDaemon::bind_with_persistent_identity_and_action_ceiling(
            &socket,
            workspace(&source),
            limits(),
            actions,
        )
        .unwrap()
        .require_authentication(Authentication, (), 32);
        let mut metadata = LocalWorkspaceIdentityMetadataV1::default();
        let WorkspaceIdentityRecoveryV1::Recovered(recovered) =
            metadata.load(socket.parent().unwrap(), limits().workspace_identity)
        else {
            panic!("workspace identity expected")
        };
        fs::write(
            identity,
            format!(
                "{} {}",
                recovered.workspace(),
                recovered.document_bindings()[0].ordinal()
            ),
        )
        .unwrap();
        loop {
            daemon.serve_next().unwrap();
        }
    }

    struct LoseDurableReply<X> {
        inner: X,
        lose: bool,
    }

    impl<X: FrameExchange> FrameExchange for LoseDurableReply<X> {
        fn exchange(
            &mut self,
            request: &[u8],
            max_reply_bytes: usize,
        ) -> Result<Vec<u8>, TransportError> {
            let discard = self.lose
                && matches!(
                    vellum_wire::decode_request_v1(request, vellum_wire::WireLimits::reference()),
                    Ok(vellum_wire::WireRequestV1::DurableExecute(_))
                );
            let reply = self.inner.exchange(request, max_reply_bytes)?;
            if discard {
                self.lose = false;
                Err(TransportError::OutcomeUnknown)
            } else {
                Ok(reply)
            }
        }
    }

    fn connector(socket: &Path) -> AuthenticatedUnixConnector {
        AuthenticatedUnixConnector::with_timeout(
            socket.to_path_buf(),
            LocalAuthenticationClaimV1::new(CLAIM),
            Duration::from_secs(2),
        )
        .unwrap()
    }

    fn wait_for_file(child: &mut Child, path: &Path) {
        for _ in 0..200 {
            if path.exists() {
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
                panic!("Vellum helper exited {status}: {stderr}");
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("Vellum helper did not publish identity");
    }

    #[test]
    fn lost_edit_reply_is_reconciled_by_a_fresh_client_in_an_independent_process() {
        let directory = std::env::temp_dir().join(format!(
            "moira-vellum-process-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let source = directory.join("source.txt");
        fs::write(&source, "alpha").unwrap();
        let socket = directory.join("vellum.sock");
        let identity = directory.join("identity.txt");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("tests::vellum_process_helper")
            .arg("--exact")
            .arg("--nocapture")
            .env("MOIRA_VELLUM_HELPER", "1")
            .env("MOIRA_VELLUM_SOCKET", &socket)
            .env("MOIRA_VELLUM_SOURCE", &source)
            .env("MOIRA_VELLUM_IDENTITY", &identity)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_for_file(&mut child, &identity);
        let ids = fs::read_to_string(&identity).unwrap();
        let mut ids = ids
            .split_whitespace()
            .map(|value| value.parse::<u64>().unwrap());
        let workspace = WorkspaceIdV1::new(ids.next().unwrap()).unwrap();
        let document = WorkspaceDocumentIdV1::new(workspace, ids.next().unwrap()).unwrap();
        let target = ExternalDocumentRefV1 {
            workspace: [3; 16],
            document: [4; 16],
        };
        let routes = BTreeMap::from([(
            target,
            DocumentRouteV1 {
                workspace,
                document,
            },
        )]);
        let transport =
            RemoteTransport::new(connector(&socket), TransportLimits::reference()).unwrap();
        let mut client = VellumClient::connect(transport).unwrap();
        let snapshot = match client.snapshot().unwrap() {
            vellum_protocol::ReplyV1::Accepted(vellum_protocol::OutcomeV1::Snapshot(value)) => {
                value
            }
            other => panic!("initial snapshot expected: {other:?}"),
        };
        let replaced = match client
            .replace(snapshot.witness, 0, 5, "ready".into())
            .unwrap()
        {
            vellum_protocol::ReplyV1::Accepted(vellum_protocol::OutcomeV1::Replaced(value)) => {
                value
            }
            other => panic!("baseline replacement expected: {other:?}"),
        };
        assert!(matches!(
            client.save(replaced.witness).unwrap(),
            vellum_protocol::ReplyV1::Accepted(vellum_protocol::OutcomeV1::Saved(_))
        ));
        let baseline = match client.snapshot().unwrap() {
            vellum_protocol::ReplyV1::Accepted(vellum_protocol::OutcomeV1::Snapshot(value)) => {
                value
            }
            other => panic!("saved baseline expected: {other:?}"),
        };
        drop(client);

        let intent = EditIntentV1 {
            effect: EffectIdV1([5; 16]),
            target,
            expected_revision: baseline.revision,
            expected_extent: 5,
            replacement: b"bravo".to_vec(),
        };

        let first = RemoteTransport::new(
            LoseDurableReply {
                inner: connector(&socket),
                lose: true,
            },
            TransportLimits::reference(),
        )
        .unwrap();
        let mut adapter = EditAdapter::connect(first, routes.clone(), 1_024).unwrap();
        assert_eq!(
            adapter.execute(&intent).unwrap(),
            EditReplyV1::OutcomeUnknown
        );
        drop(adapter);

        let second =
            RemoteTransport::new(connector(&socket), TransportLimits::reference()).unwrap();
        let mut adapter = EditAdapter::connect(second, routes, 1_024).unwrap();
        let recovered = match adapter.lookup(&intent) {
            Ok(value) => value,
            Err(error) => {
                let status = child.try_wait().unwrap();
                let mut stderr = String::new();
                child
                    .stderr
                    .as_mut()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                panic!("lookup failed {error:?}; helper={status:?}: {stderr}")
            }
        };
        assert!(
            matches!(
                &recovered,
                EditReplyV1::Completed { effect, .. } if *effect == intent.effect
            ),
            "unexpected recovery: {recovered:?}"
        );
        assert_eq!(fs::read_to_string(&source).unwrap(), "bravo");

        child.kill().unwrap();
        let status = child.wait().unwrap();
        assert!(
            !status.success(),
            "killed helper must not report clean exit"
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
