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
        io::{BufRead, BufReader, Read},
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        process::{Child, ChildStdout, Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };
    use vellum_adapter_local_authentication::{
        LocalCredentialClaimProviderV1, LocalCredentialStoreV1,
    };
    use vellum_client::Client as VellumClient;
    use vellum_daemon::AuthenticatedUnixConnector;
    use vellum_protocol::{OutcomeV1, ReplyV1};
    use vellum_transport::{FrameExchange, RemoteTransport, TransportError, TransportLimits};

    static NEXT: AtomicU64 = AtomicU64::new(0);

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

    struct Fixture {
        directory: PathBuf,
        source: PathBuf,
        socket: PathBuf,
        control: PathBuf,
        credential: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "moira-vellum-process-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
            let source = directory.join("source.txt");
            fs::write(&source, "alpha").unwrap();
            Self {
                source,
                socket: directory.join("vellum.sock"),
                control: directory.join("control.sock"),
                credential: directory.join("credential"),
                directory,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn program() -> PathBuf {
        PathBuf::from(
            std::env::var_os("MOIRA_VELLUM_DAEMON_PROGRAM")
                .expect("MOIRA_VELLUM_DAEMON_PROGRAM must name the standalone daemon"),
        )
    }

    fn connector(socket: &Path, credential: &Path) -> AuthenticatedUnixConnector {
        let record = LocalCredentialStoreV1::default().load(credential).unwrap();
        AuthenticatedUnixConnector::with_provider_and_timeout(
            socket.to_path_buf(),
            LocalCredentialClaimProviderV1::new(record),
            Duration::from_secs(2),
        )
        .unwrap()
    }

    fn start(fixture: &Fixture) -> (Child, BufReader<ChildStdout>) {
        let provisioned = Command::new(program())
            .arg("--provision-credential")
            .arg(&fixture.credential)
            .output()
            .unwrap();
        assert!(provisioned.status.success());
        let mut child = Command::new(program())
            .arg("--socket")
            .arg(&fixture.socket)
            .arg("--control")
            .arg(&fixture.control)
            .arg("--credential")
            .arg(&fixture.credential)
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
        assert_eq!(ready, "vellum-daemon ready\n");
        (child, stdout)
    }

    #[test]
    fn lost_edit_reply_is_reconciled_by_a_fresh_client_through_the_standalone_daemon() {
        let fixture = Fixture::new();
        let (mut child, mut stdout) = start(&fixture);
        let transport = RemoteTransport::new(
            connector(&fixture.socket, &fixture.credential),
            TransportLimits::reference(),
        )
        .unwrap();
        let mut client = VellumClient::connect(transport).unwrap();
        let inventory = match client.workspace_inventory().unwrap() {
            ReplyV1::Accepted(OutcomeV1::WorkspaceInventory(value)) => value,
            other => panic!("workspace inventory expected: {other:?}"),
        };
        let snapshot = match client.snapshot().unwrap() {
            ReplyV1::Accepted(OutcomeV1::Snapshot(value)) => value,
            other => panic!("initial snapshot expected: {other:?}"),
        };
        let replaced = match client
            .replace(snapshot.witness, 0, 5, "ready".into())
            .unwrap()
        {
            ReplyV1::Accepted(OutcomeV1::Replaced(value)) => value,
            other => panic!("baseline replacement expected: {other:?}"),
        };
        assert!(matches!(
            client.save(replaced.witness).unwrap(),
            ReplyV1::Accepted(OutcomeV1::Saved(_))
        ));
        let baseline = match client.snapshot().unwrap() {
            ReplyV1::Accepted(OutcomeV1::Snapshot(value)) => value,
            other => panic!("saved baseline expected: {other:?}"),
        };
        client.disconnect().map_err(|(_, error)| error).unwrap();

        let target = ExternalDocumentRefV1 {
            workspace: [3; 16],
            document: [4; 16],
        };
        let routes = BTreeMap::from([(
            target,
            DocumentRouteV1 {
                workspace: inventory.workspace,
                document: inventory.documents[0],
            },
        )]);
        let intent = EditIntentV1 {
            effect: EffectIdV1([5; 16]),
            target,
            expected_revision: baseline.revision,
            expected_extent: 5,
            replacement: b"bravo".to_vec(),
        };
        let first = RemoteTransport::new(
            LoseDurableReply {
                inner: connector(&fixture.socket, &fixture.credential),
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

        let second = RemoteTransport::new(
            connector(&fixture.socket, &fixture.credential),
            TransportLimits::reference(),
        )
        .unwrap();
        let mut adapter = EditAdapter::connect(second, routes, 1_024).unwrap();
        assert!(matches!(
            adapter.lookup(&intent).unwrap(),
            EditReplyV1::Completed { effect, .. } if effect == intent.effect
        ));
        assert_eq!(fs::read_to_string(&fixture.source).unwrap(), "bravo");

        let shutdown = Command::new(program())
            .arg("--shutdown")
            .arg(&fixture.control)
            .output()
            .unwrap();
        assert!(shutdown.status.success());
        assert!(child.wait().unwrap().success());
        let mut remainder = String::new();
        stdout.read_to_string(&mut remainder).unwrap();
        assert!(remainder.contains("vellum-daemon stopped"));
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(!stderr.contains("alpha"));
        assert!(!fixture.socket.exists());
        assert!(!fixture.control.exists());
    }
}
