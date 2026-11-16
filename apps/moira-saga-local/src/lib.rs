#![forbid(unsafe_code)]

use moira_core::saga::{SagaLimitsV1, persistence::SagaCodecLimitsV1};
use moira_local_daemon::{
    ConnectionErrorV1,
    unix::{SagaDaemon, SagaDaemonErrorV1, SagaDispatchErrorV1},
};
use moira_protocol::OperatorOperationV1;
use moira_service::{
    AccessV1,
    saga::{OperatorAccessV1, OperatorAuthorizer, SagaService},
};
use moira_sqlite::saga::{SagaSqliteStore, SagaStoreLimitsV1};
use moira_unix_peer::{MappingV1, UnixPeerAuthenticatorV1, UnixSagaAuthenticatorV1};
use serde::Deserialize;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::{
    convert::Infallible,
    error::Error,
    fs,
    io::Read,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_MAPPINGS: usize = 256;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigV1 {
    pub database: PathBuf,
    pub socket: PathBuf,
    pub owner: String,
    pub mappings: Vec<PeerMappingV1>,
    pub io_timeout_ms: u64,
    pub max_requests_per_connection: usize,
    pub max_frame_bytes: usize,
    pub max_checkpoint_bytes: usize,
    pub max_database_pages: u32,
    pub max_sagas: usize,
    pub max_retries_per_saga: u8,
    pub max_inspection_page: usize,
    pub max_operator_actions: usize,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerMappingV1 {
    pub effective_uid: u32,
    pub effective_gid: u32,
    pub read: bool,
    pub mutate: bool,
}

pub struct ValidatedConfigV1 {
    raw: ConfigV1,
    owner: [u8; 16],
}

pub fn load(path: impl AsRef<Path>) -> Result<ValidatedConfigV1, String> {
    let metadata = fs::metadata(path.as_ref())
        .map_err(|error| format!("configuration unavailable: {error}"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_CONFIG_BYTES {
        return Err("configuration must be a nonempty regular file no larger than 64 KiB".into());
    }
    let bytes =
        fs::read(path.as_ref()).map_err(|error| format!("configuration unavailable: {error}"))?;
    let raw: ConfigV1 = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid configuration: {error}"))?;
    validate(raw)
}

pub fn validate(raw: ConfigV1) -> Result<ValidatedConfigV1, String> {
    if !raw.database.is_absolute() || !raw.socket.is_absolute() || raw.database == raw.socket {
        return Err("database and socket must be distinct absolute paths".into());
    }
    if raw.mappings.is_empty() || raw.mappings.len() > MAX_MAPPINGS {
        return Err("peer mappings must contain between 1 and 256 entries".into());
    }
    if raw
        .mappings
        .iter()
        .any(|mapping| !mapping.read && !mapping.mutate)
    {
        return Err("every peer mapping must grant read or mutate access".into());
    }
    if raw.io_timeout_ms == 0
        || raw.max_requests_per_connection == 0
        || raw.max_frame_bytes == 0
        || raw.max_checkpoint_bytes == 0
        || raw.max_database_pages == 0
        || raw.max_sagas == 0
        || raw.max_inspection_page == 0
        || raw.max_operator_actions == 0
    {
        return Err("all capacity and timeout limits must be nonzero".into());
    }
    if raw.max_inspection_page > raw.max_sagas {
        return Err("inspection page limit cannot exceed saga capacity".into());
    }
    let owner = parse_owner(&raw.owner)?;
    let mappings = raw.mappings.iter().map(mapping).collect();
    UnixPeerAuthenticatorV1::new(MAX_MAPPINGS, mappings)
        .map_err(|error| format!("invalid peer mapping registry: {error:?}"))?;
    let limits = store_limits(&raw);
    // Store construction performs further bounded profile validation, but
    // --check must remain side-effect free. These relationships are the ones
    // required before any SQLite owner is opened.
    if limits.codec.max_sagas != limits.saga.max_sagas
        || limits.codec.max_operator_actions != limits.saga.max_operator_actions
    {
        return Err("checkpoint and saga capacities must agree".into());
    }
    Ok(ValidatedConfigV1 { raw, owner })
}

pub fn run(config: ValidatedConfigV1) -> Result<(), Box<dyn Error>> {
    let stop = Arc::new(AtomicBool::new(false));
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let handle = signals.handle();
    let observer = Arc::clone(&stop);
    let socket = config.raw.socket.clone();
    let signal_thread = thread::Builder::new()
        .name("moira-saga-signal-owner".into())
        .spawn(move || {
            if signals.forever().next().is_some() {
                observer.store(true, Ordering::Release);
                let _ = UnixStream::connect(socket);
            }
        })?;
    let result = run_with_stop(config, stop);
    handle.close();
    let _ = signal_thread.join();
    result
}

/// Runs under the Workbench owner channel. Only the exact byte `x` requests a
/// clean drain; EOF and all other input leave the owner running.
pub fn run_supervised<R>(config: ValidatedConfigV1, mut shutdown: R) -> Result<(), Box<dyn Error>>
where
    R: Read + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let signal_handle = signals.handle();
    let signal_observer = Arc::clone(&stop);
    let signal_socket = config.raw.socket.clone();
    let signal_thread = thread::Builder::new()
        .name("moira-saga-signal-owner".into())
        .spawn(move || {
            if signals.forever().next().is_some() {
                signal_observer.store(true, Ordering::Release);
                let _ = UnixStream::connect(signal_socket);
            }
        })
        .map_err(|error| format!("signal owner unavailable: {error}"))?;
    let observer = Arc::clone(&stop);
    let socket = config.raw.socket.clone();
    thread::Builder::new()
        .name("moira-saga-supervisor".into())
        .spawn(move || {
            let mut request = [0_u8; 1];
            if shutdown.read_exact(&mut request).is_ok() && request == *b"x" {
                observer.store(true, Ordering::Release);
                let _ = UnixStream::connect(socket);
            }
        })
        .map_err(|error| format!("supervisor channel unavailable: {error}"))?;
    let result = run_with_stop(config, stop);
    signal_handle.close();
    let _ = signal_thread.join();
    result
}

fn run_with_stop(config: ValidatedConfigV1, stop: Arc<AtomicBool>) -> Result<(), Box<dyn Error>> {
    let mappings = config.raw.mappings.iter().map(mapping).collect();
    let peer = UnixPeerAuthenticatorV1::new(MAX_MAPPINGS, mappings)
        .map_err(|error| format!("peer mapping startup refused: {error:?}"))?;
    let authentication = UnixSagaAuthenticatorV1::new(peer, DenyOperators);
    let store = SagaSqliteStore::open(
        &config.raw.database,
        config.owner,
        store_limits(&config.raw),
    )?;
    let mut daemon = SagaDaemon::bind_new(
        &config.raw.socket,
        SagaService::new(store),
        authentication,
        moira_wire::LimitsV1 {
            max_frame_bytes: config.raw.max_frame_bytes,
        },
        Duration::from_millis(config.raw.io_timeout_ms),
        config.raw.max_requests_per_connection,
    )
    .map_err(|error| format!("private saga socket unavailable: {error}"))?;
    while !stop.load(Ordering::Acquire) {
        let served = daemon.serve_next();
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        match served {
            Ok(())
            | Err(SagaDaemonErrorV1::Authentication(_))
            | Err(SagaDaemonErrorV1::Connection(ConnectionErrorV1::BeforeDispatch(_)))
            | Err(SagaDaemonErrorV1::Connection(ConnectionErrorV1::Wire(_)))
            | Err(SagaDaemonErrorV1::Connection(ConnectionErrorV1::OutcomeUnknown(_)))
            | Err(SagaDaemonErrorV1::Connection(ConnectionErrorV1::Service(
                SagaDispatchErrorV1::Authorization(_),
            ))) => {}
            Err(SagaDaemonErrorV1::Connection(ConnectionErrorV1::Service(
                SagaDispatchErrorV1::Service(error),
            ))) => return Err(error.into()),
            Err(SagaDaemonErrorV1::Io(error)) => return Err(error.into()),
        }
    }
    Ok(())
}

fn mapping(value: &PeerMappingV1) -> MappingV1 {
    MappingV1 {
        effective_uid: value.effective_uid,
        effective_gid: value.effective_gid,
        access: AccessV1 {
            read: value.read,
            mutate: value.mutate,
        },
    }
}

fn store_limits(config: &ConfigV1) -> SagaStoreLimitsV1 {
    SagaStoreLimitsV1 {
        codec: SagaCodecLimitsV1 {
            max_bytes: config.max_checkpoint_bytes,
            max_sagas: config.max_sagas,
            max_operator_actions: config.max_operator_actions,
        },
        saga: SagaLimitsV1 {
            max_sagas: config.max_sagas,
            max_retries_per_saga: config.max_retries_per_saga,
            max_inspection_page: config.max_inspection_page,
            max_operator_actions: config.max_operator_actions,
        },
        max_database_pages: config.max_database_pages,
    }
}

fn parse_owner(value: &str) -> Result<[u8; 16], String> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("owner must be exactly 32 lowercase hexadecimal digits".into());
    }
    let mut owner = [0_u8; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        owner[index] = (hex(pair[0]) << 4) | hex(pair[1]);
    }
    Ok(owner)
}

fn hex(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => unreachable!("validated lowercase hexadecimal"),
    }
}

struct DenyOperators;

impl OperatorAuthorizer for DenyOperators {
    type Error = Infallible;

    fn authorize_operator(
        &mut self,
        _: OperatorOperationV1,
    ) -> Result<Option<OperatorAccessV1>, Self::Error> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> ConfigV1 {
        ConfigV1 {
            database: PathBuf::from("/private/tmp/moira-saga.db"),
            socket: PathBuf::from("/private/tmp/moira-saga.sock"),
            owner: "01010101010101010101010101010101".into(),
            mappings: vec![PeerMappingV1 {
                effective_uid: 1,
                effective_gid: 2,
                read: true,
                mutate: false,
            }],
            io_timeout_ms: 5_000,
            max_requests_per_connection: 32,
            max_frame_bytes: 65_536,
            max_checkpoint_bytes: 65_536,
            max_database_pages: 1_024,
            max_sagas: 256,
            max_retries_per_saga: 8,
            max_inspection_page: 128,
            max_operator_actions: 1_024,
        }
    }

    #[test]
    fn validation_refuses_ambient_or_useless_access() {
        let mut config = valid();
        config.mappings[0].read = false;
        assert!(validate(config).is_err());
    }

    #[test]
    fn validation_refuses_duplicate_kernel_identity() {
        let mut config = valid();
        config.mappings.push(config.mappings[0]);
        assert!(validate(config).is_err());
    }
}
