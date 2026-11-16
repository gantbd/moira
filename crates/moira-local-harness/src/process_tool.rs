//! Shell-free local process profile for already-authorized tool effects.

use moira_core::tool_continuation::ToolCapabilityIdV1;
use moira_host::{ToolExecutionOutcomeV1, ToolExecutorV1, ToolInvocationV1};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolProcessProfileV1 {
    pub capability: ToolCapabilityIdV1,
    pub executable: PathBuf,
    pub arguments: Vec<OsString>,
    pub working_directory: PathBuf,
    pub max_elapsed: Duration,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_stderr_bytes: usize,
    pub max_executable_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessToolConfigErrorV1 {
    Empty,
    DuplicateCapability,
    InvalidCapability,
    InvalidPath,
    InvalidLimit,
    ExecutableUnavailable,
}

#[derive(Debug)]
pub enum ProcessToolErrorV1 {
    UnknownCapability,
    ProfileDrift,
    Io(std::io::Error),
}

impl PartialEq for ProcessToolErrorV1 {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::UnknownCapability, Self::UnknownCapability)
            | (Self::ProfileDrift, Self::ProfileDrift) => true,
            (Self::Io(left), Self::Io(right)) => left.kind() == right.kind(),
            _ => false,
        }
    }
}

impl Eq for ProcessToolErrorV1 {}

struct FrozenProfileV1 {
    profile: ToolProcessProfileV1,
    executable_digest: [u8; 32],
    profile_digest: [u8; 32],
}

pub struct ProcessToolExecutorV1 {
    profiles: BTreeMap<ToolCapabilityIdV1, FrozenProfileV1>,
    poll_interval: Duration,
}

impl ProcessToolExecutorV1 {
    pub fn try_new(
        profiles: Vec<ToolProcessProfileV1>,
        poll_interval: Duration,
    ) -> Result<Self, ProcessToolConfigErrorV1> {
        if profiles.is_empty() || poll_interval.is_zero() {
            return Err(ProcessToolConfigErrorV1::Empty);
        }
        let mut frozen = BTreeMap::new();
        for mut profile in profiles {
            if profile.capability.0 == [0; 16] {
                return Err(ProcessToolConfigErrorV1::InvalidCapability);
            }
            if !profile.executable.is_absolute() || !profile.working_directory.is_absolute() {
                return Err(ProcessToolConfigErrorV1::InvalidPath);
            }
            if profile.max_elapsed.is_zero()
                || profile.max_input_bytes == 0
                || profile.max_output_bytes == 0
                || profile.max_stderr_bytes == 0
                || profile.max_executable_bytes == 0
                || profile.arguments.len() > 128
                || profile
                    .arguments
                    .iter()
                    .map(|value| value.as_encoded_bytes().len())
                    .sum::<usize>()
                    > 16 * 1_024
            {
                return Err(ProcessToolConfigErrorV1::InvalidLimit);
            }
            profile.executable = profile
                .executable
                .canonicalize()
                .map_err(|_| ProcessToolConfigErrorV1::ExecutableUnavailable)?;
            profile.working_directory = profile
                .working_directory
                .canonicalize()
                .map_err(|_| ProcessToolConfigErrorV1::InvalidPath)?;
            if !profile.executable.is_file() || !profile.working_directory.is_dir() {
                return Err(ProcessToolConfigErrorV1::InvalidPath);
            }
            let executable_digest = digest_file(&profile.executable, profile.max_executable_bytes)
                .map_err(|_| ProcessToolConfigErrorV1::ExecutableUnavailable)?;
            let profile_digest = digest_profile(&profile, executable_digest);
            let capability = profile.capability;
            if frozen
                .insert(
                    capability,
                    FrozenProfileV1 {
                        profile,
                        executable_digest,
                        profile_digest,
                    },
                )
                .is_some()
            {
                return Err(ProcessToolConfigErrorV1::DuplicateCapability);
            }
        }
        Ok(Self {
            profiles: frozen,
            poll_interval,
        })
    }
}

impl ToolExecutorV1 for ProcessToolExecutorV1 {
    type Error = ProcessToolErrorV1;

    fn execute(
        &mut self,
        invocation: &ToolInvocationV1,
    ) -> Result<ToolExecutionOutcomeV1, Self::Error> {
        let capability = invocation.call.capability;
        let frozen = self
            .profiles
            .get(&capability)
            .ok_or(ProcessToolErrorV1::UnknownCapability)?;
        if invocation.gate.authority.capability != capability {
            return Ok(ToolExecutionOutcomeV1::Failed {
                evidence_digest: refusal_digest(invocation, frozen, b"gate-capability-mismatch"),
            });
        }
        if invocation.call.arguments.len() > frozen.profile.max_input_bytes
            || invocation.call.budget.max_output_bytes > frozen.profile.max_output_bytes
        {
            return Ok(ToolExecutionOutcomeV1::Failed {
                evidence_digest: refusal_digest(invocation, frozen, b"profile-limit"),
            });
        }
        if digest_file(
            &frozen.profile.executable,
            frozen.profile.max_executable_bytes,
        )
        .map_err(ProcessToolErrorV1::Io)?
            != frozen.executable_digest
        {
            return Err(ProcessToolErrorV1::ProfileDrift);
        }
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(
                invocation.call.budget.max_elapsed_millis.min(
                    frozen
                        .profile
                        .max_elapsed
                        .as_millis()
                        .try_into()
                        .unwrap_or(u64::MAX),
                ),
            ))
            .ok_or(ProcessToolErrorV1::ProfileDrift)?;
        let mut child = Command::new(&frozen.profile.executable)
            .args(&frozen.profile.arguments)
            .current_dir(&frozen.profile.working_directory)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(ProcessToolErrorV1::Io)?;
        let Some(stdout) = child.stdout.take() else {
            terminate(&mut child);
            return Ok(ToolExecutionOutcomeV1::OutcomeUnknown);
        };
        let Some(stderr) = child.stderr.take() else {
            terminate(&mut child);
            return Ok(ToolExecutionOutcomeV1::OutcomeUnknown);
        };
        let stdout_limit = invocation
            .call
            .budget
            .max_output_bytes
            .min(frozen.profile.max_output_bytes);
        let stderr_limit = frozen.profile.max_stderr_bytes;
        let stdout_reader = match spawn_reader("stdout", stdout, stdout_limit) {
            Ok(handle) => handle,
            Err(_) => {
                terminate(&mut child);
                return Ok(ToolExecutionOutcomeV1::OutcomeUnknown);
            }
        };
        let stderr_reader = match spawn_reader("stderr", stderr, stderr_limit) {
            Ok(handle) => handle,
            Err(_) => {
                terminate(&mut child);
                let _ = stdout_reader.join();
                return Ok(ToolExecutionOutcomeV1::OutcomeUnknown);
            }
        };
        let Some(mut stdin) = child.stdin.take() else {
            terminate(&mut child);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Ok(ToolExecutionOutcomeV1::OutcomeUnknown);
        };
        let input = invocation.call.arguments.clone();
        let stdin_writer = match thread::Builder::new()
            .name("moira-tool-stdin".to_owned())
            .spawn(move || {
                stdin.write_all(&input)?;
                stdin.flush()
            }) {
            Ok(handle) => handle,
            Err(_) => {
                terminate(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Ok(ToolExecutionOutcomeV1::OutcomeUnknown);
            }
        };
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => thread::sleep(self.poll_interval),
                Ok(None) => {
                    if child.kill().is_err() || child.wait().is_err() {
                        let _ = stdin_writer.join();
                        let _ = stdout_reader.join();
                        let _ = stderr_reader.join();
                        return Ok(ToolExecutionOutcomeV1::OutcomeUnknown);
                    }
                    break None;
                }
                Err(_) => {
                    terminate(&mut child);
                    let _ = stdin_writer.join();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Ok(ToolExecutionOutcomeV1::OutcomeUnknown);
                }
            }
        };
        let stdin_result = stdin_writer.join();
        let (Some(stdout), Some(stderr)) =
            (collect_reader(stdout_reader), collect_reader(stderr_reader))
        else {
            return Ok(ToolExecutionOutcomeV1::OutcomeUnknown);
        };
        if status.is_some() && !matches!(stdin_result, Ok(Ok(()))) {
            return Ok(ToolExecutionOutcomeV1::OutcomeUnknown);
        }
        if stdout.exceeded || stderr.exceeded {
            return Ok(ToolExecutionOutcomeV1::Failed {
                evidence_digest: limited_output_evidence(invocation, frozen, &stdout, &stderr),
            });
        }
        let evidence_digest =
            process_evidence(invocation, frozen, status, &stdout.bytes, &stderr.bytes);
        match status {
            Some(status) if status.success() => Ok(ToolExecutionOutcomeV1::Succeeded {
                evidence_digest,
                output: stdout.bytes,
            }),
            Some(_) | None => Ok(ToolExecutionOutcomeV1::Failed { evidence_digest }),
        }
    }
}

fn digest_file(path: &Path, maximum: u64) -> Result<[u8; 32], std::io::Error> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > maximum {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "executable exceeds profile bound",
        ));
    }
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1_024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().into())
}

fn digest_profile(profile: &ToolProcessProfileV1, executable_digest: [u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"moira-local-process-profile-v1\0");
    hash.update(profile.capability.0);
    hash.update(executable_digest);
    hash_len(&mut hash, profile.executable.as_os_str().as_encoded_bytes());
    hash.update(
        u64::try_from(profile.arguments.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    for argument in &profile.arguments {
        hash_len(&mut hash, argument.as_encoded_bytes());
    }
    hash_len(
        &mut hash,
        profile.working_directory.as_os_str().as_encoded_bytes(),
    );
    hash.update(
        u64::try_from(profile.max_elapsed.as_millis())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(
        u64::try_from(profile.max_input_bytes)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(
        u64::try_from(profile.max_output_bytes)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(
        u64::try_from(profile.max_stderr_bytes)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(profile.max_executable_bytes.to_be_bytes());
    hash.finalize().into()
}

#[derive(Debug)]
struct BoundedOutputV1 {
    bytes: Vec<u8>,
    observed_bytes: u64,
    exceeded: bool,
}

type ReaderHandle = thread::JoinHandle<Result<BoundedOutputV1, std::io::Error>>;

fn spawn_reader(
    label: &str,
    reader: impl Read + Send + 'static,
    maximum: usize,
) -> Result<ReaderHandle, std::io::Error> {
    thread::Builder::new()
        .name(format!("moira-tool-{label}"))
        .spawn(move || read_bounded(reader, maximum))
}

fn collect_reader(handle: ReaderHandle) -> Option<BoundedOutputV1> {
    handle.join().ok()?.ok()
}

fn read_bounded(mut reader: impl Read, maximum: usize) -> Result<BoundedOutputV1, std::io::Error> {
    let mut bytes = Vec::new();
    bytes.try_reserve(maximum.min(64 * 1_024)).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::OutOfMemory, "output allocation refused")
    })?;
    let mut observed_bytes = 0_u64;
    let mut buffer = [0_u8; 16 * 1_024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        observed_bytes = observed_bytes.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        let remaining = maximum.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buffer[..count.min(remaining)]);
    }
    Ok(BoundedOutputV1 {
        exceeded: observed_bytes > u64::try_from(maximum).unwrap_or(u64::MAX),
        bytes,
        observed_bytes,
    })
}

fn terminate(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn refusal_digest(
    invocation: &ToolInvocationV1,
    profile: &FrozenProfileV1,
    reason: &[u8],
) -> [u8; 32] {
    evidence_hash(invocation, profile, reason, &[], &[])
}

fn process_evidence(
    invocation: &ToolInvocationV1,
    profile: &FrozenProfileV1,
    status: Option<ExitStatus>,
    stdout: &[u8],
    stderr: &[u8],
) -> [u8; 32] {
    let mut disposition = match status {
        None => b"deadline-killed\0".to_vec(),
        Some(_) => b"process-exited\0".to_vec(),
    };
    disposition.extend_from_slice(
        &status
            .and_then(|value| value.code())
            .unwrap_or(i32::MIN)
            .to_be_bytes(),
    );
    evidence_hash(invocation, profile, &disposition, stdout, stderr)
}

fn limited_output_evidence(
    invocation: &ToolInvocationV1,
    profile: &FrozenProfileV1,
    stdout: &BoundedOutputV1,
    stderr: &BoundedOutputV1,
) -> [u8; 32] {
    let mut disposition = b"process-output-limit\0".to_vec();
    disposition.extend_from_slice(&stdout.observed_bytes.to_be_bytes());
    disposition.extend_from_slice(&stderr.observed_bytes.to_be_bytes());
    evidence_hash(
        invocation,
        profile,
        &disposition,
        &stdout.bytes,
        &stderr.bytes,
    )
}

fn evidence_hash(
    invocation: &ToolInvocationV1,
    profile: &FrozenProfileV1,
    disposition: &[u8],
    stdout: &[u8],
    stderr: &[u8],
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"moira-local-process-tool-v1\0");
    hash.update(invocation.workflow.into_bytes());
    hash.update(invocation.effect.into_bytes());
    hash.update(invocation.execution.0);
    hash.update(invocation.call.request.0);
    hash.update(invocation.call.capability.0);
    hash_len(&mut hash, &invocation.call.provider_correlation);
    hash.update(invocation.call.budget.max_elapsed_millis.to_be_bytes());
    hash.update(
        u64::try_from(invocation.call.budget.max_output_bytes)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hash.update(invocation.gate.authority.decision.0);
    hash.update(invocation.gate.authority.subject.0);
    hash.update(invocation.gate.authority.capability.0);
    hash.update(invocation.gate.authority.epoch.to_be_bytes());
    hash.update(invocation.gate.attempt.task_key);
    hash.update(invocation.gate.attempt.incarnation.to_be_bytes());
    hash.update(invocation.gate.attempt.attempt.to_be_bytes());
    hash.update(profile.profile_digest);
    hash_len(&mut hash, &invocation.call.arguments);
    hash_len(&mut hash, disposition);
    hash_len(&mut hash, stdout);
    hash_len(&mut hash, stderr);
    hash.finalize().into()
}

fn hash_len(hash: &mut Sha256, bytes: &[u8]) {
    hash.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
    hash.update(bytes);
}
