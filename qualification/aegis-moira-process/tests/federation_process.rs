#![cfg(unix)]

// Executed only from the isolated cross-repository qualification workspace.

use aegis_authority::{EngineId, Limits as AegisLimits, PrincipalId, SessionId};
use aegis_client::{AuthorityClient, ServicePort as AegisServicePort};
use aegis_local_daemon::unix::{Authenticator as AegisAuthenticator, Daemon as AegisDaemon};
use aegis_protocol::{
    ActionV1, CommandV1 as AegisCommandV1, EngineNamespaceV1, GrantHandleV1,
    OutcomeV1 as AegisOutcomeV1, ReplyV1 as AegisReplyV1, ScopeV1, SessionHandleV1,
};
use aegis_service::{AuthenticatedSessionV1, Service as AegisService, ServiceLimitsV1};
use aegis_wire::{
    LimitsV1 as AegisWireLimitsV1, RequestFrameV1, decode_reply as decode_aegis_reply,
    decode_session_opened, encode_request as encode_aegis_request,
};
use moira_client::{
    SagaMutationClient, SagaOperatorClient, SagaReadOnlyClient, SagaServicePort as MoiraServicePort,
};
use moira_core::saga::{SagaLimitsV1, persistence::SagaCodecLimitsV1};
use moira_local_daemon::unix::{SagaAuthenticator, SagaDaemon};
use moira_protocol::{
    OperatorActionIdV1, OperatorDispositionV1, OperatorOperationV1, SagaCommandV1,
    SagaEffectOutcomeV1, SagaMutationV1, SagaOutcomeV1, SagaReplyV1, SagaSourceV1,
    SagaTransitionV1,
};
use moira_service::{
    AccessV1,
    saga::{OperatorAccessV1, SagaService},
};
use moira_sqlite::saga::{SagaSqliteStore, SagaStoreLimitsV1};
use moira_wire::{
    LimitsV1 as MoiraWireLimitsV1, decode_ready, decode_saga_reply, encode_saga_request,
};
use std::{
    convert::Infallible,
    fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

const SUBJECT: [u8; 16] = [0x21; 16];

fn read_frame(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

struct AegisSocketPort(UnixStream);

impl AegisServicePort for AegisSocketPort {
    type Error = std::io::Error;

    fn execute(
        &mut self,
        session: SessionHandleV1,
        command: AegisCommandV1,
    ) -> Result<AegisReplyV1, Self::Error> {
        let bytes = encode_aegis_request(
            &RequestFrameV1 { session, command },
            AegisWireLimitsV1::default(),
        )
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Aegis request"))?;
        self.0.write_all(&(bytes.len() as u32).to_be_bytes())?;
        self.0.write_all(&bytes)?;
        decode_aegis_reply(&read_frame(&mut self.0)?, AegisWireLimitsV1::default())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Aegis reply"))
    }
}

struct MoiraSocketPort(UnixStream);

impl MoiraServicePort for MoiraSocketPort {
    type Error = std::io::Error;

    fn execute(&mut self, command: SagaCommandV1) -> Result<SagaReplyV1, Self::Error> {
        let bytes = encode_saga_request(&command, MoiraWireLimitsV1::default())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Moira request"))?;
        self.0.write_all(&(bytes.len() as u32).to_be_bytes())?;
        self.0.write_all(&bytes)?;
        decode_saga_reply(&read_frame(&mut self.0)?, MoiraWireLimitsV1::default())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Moira reply"))
    }
}

struct AegisAuthentication;

impl AegisAuthenticator for AegisAuthentication {
    type Error = Infallible;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AuthenticatedSessionV1, Self::Error> {
        Ok(AuthenticatedSessionV1 {
            principal: PrincipalId::new(SUBJECT),
            session: SessionId::new([0x22; 16]),
            administrator: true,
        })
    }
}

struct LiveMoiraAuthentication {
    authority: AuthorityClient<AegisSocketPort>,
    grant: GrantHandleV1,
}

impl SagaAuthenticator for LiveMoiraAuthentication {
    type Error = std::io::Error;

    fn authenticate(&mut self, _: &UnixStream) -> Result<AccessV1, Self::Error> {
        Ok(AccessV1 {
            read: true,
            mutate: true,
        })
    }

    fn authorize_operator(
        &mut self,
        operation: OperatorOperationV1,
    ) -> Result<Option<OperatorAccessV1>, Self::Error> {
        let reply = self.authority.authorize(
            self.grant,
            10,
            EngineNamespaceV1::Moira,
            moira_aegis::saga_resource(operation.saga.0),
            ActionV1::Administer,
        )?;
        Ok(match reply {
            AegisReplyV1::Accepted(AegisOutcomeV1::Authorized(decision))
                if decision.engine == EngineNamespaceV1::Moira
                    && decision.action == ActionV1::Administer
                    && decision.expires_at >= 10 =>
            {
                Some(OperatorAccessV1 {
                    subject: SUBJECT,
                    authority_decision: decision.handle.0,
                    authority_epoch: decision.epoch,
                })
            }
            _ => None,
        })
    }
}

fn saga_limits() -> SagaStoreLimitsV1 {
    SagaStoreLimitsV1 {
        codec: SagaCodecLimitsV1 {
            max_bytes: 16_384,
            max_sagas: 4,
            max_operator_actions: 8,
        },
        saga: SagaLimitsV1 {
            max_sagas: 4,
            max_retries_per_saga: 2,
            max_inspection_page: 4,
            max_operator_actions: 8,
        },
        max_database_pages: 64,
    }
}

fn wait_for_path(path: &Path) {
    for _ in 0..400 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("process endpoint was not published: {}", path.display());
}

fn spawn_helper(mode: &str, root: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("federation_process_helper")
        .arg("--exact")
        .arg("--nocapture")
        .env("MOIRA_FEDERATION_HELPER", "1")
        .env("MOIRA_FEDERATION_MODE", mode)
        .env("MOIRA_FEDERATION_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn assert_child(child: Child, label: &str) {
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{label} process failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn federation_process_helper() {
    if std::env::var_os("MOIRA_FEDERATION_HELPER").is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os("MOIRA_FEDERATION_ROOT").unwrap());
    match std::env::var("MOIRA_FEDERATION_MODE").unwrap().as_str() {
        "aegis" => {
            let authority_id = EngineId::new([0x11; 16]);
            let authority = aegis_local::create_new(
                root.join("aegis-state"),
                authority_id,
                AegisLimits::default(),
            )
            .unwrap();
            let service =
                AegisService::new(authority, authority_id, ServiceLimitsV1::default()).unwrap();
            let mut daemon = AegisDaemon::bind_new(
                root.join("aegis.sock"),
                service,
                AegisAuthentication,
                AegisWireLimitsV1::default(),
                Duration::from_secs(3),
                16,
            )
            .unwrap();
            daemon.serve_next().unwrap();
        }
        "moira" => {
            let mut aegis_stream = UnixStream::connect(root.join("aegis.sock")).unwrap();
            let session = decode_session_opened(
                &read_frame(&mut aegis_stream).unwrap(),
                AegisWireLimitsV1::default(),
            )
            .unwrap();
            let mut authority = AuthorityClient::new(session, AegisSocketPort(aegis_stream));
            let AegisReplyV1::Accepted(AegisOutcomeV1::GrantIssued(grant)) = authority
                .issue(
                    0,
                    1,
                    100,
                    vec![ScopeV1 {
                        engine: EngineNamespaceV1::Moira,
                        resource: moira_aegis::saga_resource(0),
                        actions: 1 << ActionV1::Administer as u8,
                    }],
                )
                .unwrap()
            else {
                panic!("Aegis grant issuance failed")
            };
            let store =
                SagaSqliteStore::open(root.join("moira.db"), [0x31; 16], saga_limits()).unwrap();
            let mut daemon = SagaDaemon::bind_new(
                root.join("moira.sock"),
                SagaService::new(store),
                LiveMoiraAuthentication {
                    authority,
                    grant: grant.handle,
                },
                MoiraWireLimitsV1::default(),
                Duration::from_secs(3),
                8,
            )
            .unwrap();
            daemon.serve_next().unwrap();
        }
        mode => panic!("unknown federation helper mode {mode}"),
    }
}

fn run_live_aegis_moira_cycle() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "moira-federation-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();

    let aegis = spawn_helper("aegis", &root);
    wait_for_path(&root.join("aegis.sock"));
    let moira = spawn_helper("moira", &root);
    wait_for_path(&root.join("moira.sock"));

    let mut stream = UnixStream::connect(root.join("moira.sock")).unwrap();
    decode_ready(
        &read_frame(&mut stream).unwrap(),
        MoiraWireLimitsV1::default(),
    )
    .unwrap();
    let mut mutation = SagaMutationClient::new(MoiraSocketPort(stream));
    let SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
        transition: SagaTransitionV1::Admitted { saga, effect, .. },
        ..
    }) = mutation
        .apply(
            0,
            SagaMutationV1::Admit {
                source: SagaSourceV1([0x41; 16]),
                now: 1,
                deadline: 50,
                retry_budget: 1,
            },
        )
        .unwrap()
    else {
        panic!("saga admission failed")
    };
    mutation
        .apply(
            1,
            SagaMutationV1::AcceptEffect {
                saga,
                effect,
                outcome: SagaEffectOutcomeV1::Uncertain,
                observed_at: 2,
            },
        )
        .unwrap();
    let action = OperatorActionIdV1([0x51; 16]);
    let mut operator = SagaOperatorClient::new(mutation.into_port());
    assert!(matches!(
        operator
            .reconcile(
                2,
                OperatorOperationV1 {
                    action,
                    saga,
                    disposition: OperatorDispositionV1::KeepUncertain,
                },
            )
            .unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            replayed: false,
            transition: SagaTransitionV1::Reconciled { .. },
            ..
        })
    ));
    let mut reader = SagaReadOnlyClient::new(operator.into_port());
    assert!(matches!(
        reader.operator_receipt(action).unwrap(),
        SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipt(Some(receipt)))
            if receipt.saga == saga
                && receipt.authority_epoch == 0
                && receipt.authority_decision == 0
    ));
    drop(reader);

    assert_child(moira, "Moira");
    assert_child(aegis, "Aegis");
    assert!(!root.join("moira.sock").exists());
    assert!(!root.join("aegis.sock").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_aegis_process_authorizes_moira_process_reconciliation() {
    run_live_aegis_moira_cycle();
}

#[test]
#[ignore = "opt-in durable 16-cycle Aegis-Moira federation soak"]
fn sixteen_federation_process_cycles_preserve_attributed_reconciliation() {
    for cycle in 0..16 {
        run_live_aegis_moira_cycle();
        eprintln!("completed_cycle={cycle}");
    }
}
