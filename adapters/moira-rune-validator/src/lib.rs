//! Bounded adapter for Rune's closed-world OCaml theorem provers.
//!
//! The configured question digest states exactly which theorem the selected
//! prover answers. A successful process is not enough: every NDJSON row is
//! parsed, the aggregate must reconcile with the instance rows, and any
//! `hold_quantum` or `mu` result refuses Boolean promotion.

#![forbid(unsafe_code)]

use moira_host::{BooleanValidatorV1, ProofRequestV1, ProofWitnessV1};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    io::{ErrorKind, Read},
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAX_ARGUMENTS: usize = 16;
const MAX_ARGUMENT_BYTES: usize = 4_096;
const MAX_OUTPUT_BYTES: usize = 1_048_576;
const MAX_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuneTheoremValidatorConfigV1 {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
    pub expected_question_digest: [u8; 32],
    pub validator: [u8; 16],
    pub max_output_bytes: usize,
    pub timeout: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuneTheoremValidatorErrorV1 {
    InvalidConfig,
    WrongQuestion,
    Spawn(ErrorKind),
    Wait(ErrorKind),
    Read(ErrorKind),
    Timeout,
    ProcessFailed,
    OutputLimit,
    InvalidOutput,
}

pub struct RuneTheoremValidatorV1 {
    config: RuneTheoremValidatorConfigV1,
}

impl RuneTheoremValidatorV1 {
    pub fn new(config: RuneTheoremValidatorConfigV1) -> Result<Self, RuneTheoremValidatorErrorV1> {
        validate_config(&config)?;
        Ok(Self { config })
    }
}

impl BooleanValidatorV1 for RuneTheoremValidatorV1 {
    type Error = RuneTheoremValidatorErrorV1;

    fn validate(&mut self, request: ProofRequestV1) -> Result<ProofWitnessV1, Self::Error> {
        if request.subject.question_digest != self.config.expected_question_digest {
            return Err(RuneTheoremValidatorErrorV1::WrongQuestion);
        }
        let output = run(&self.config)?;
        let value = parse_verdict(&output)?;
        let mut hash = Sha256::new();
        hash.update(b"MOIRATP1");
        hash.update(request.subject.deliberation.0);
        hash.update(request.subject.question_digest);
        hash.update(request.subject.checkpoint_digest);
        hash.update(&output);
        Ok(ProofWitnessV1 {
            subject: request.subject,
            validator: self.config.validator,
            value,
            evidence_digest: hash.finalize().into(),
        })
    }
}

fn validate_config(
    config: &RuneTheoremValidatorConfigV1,
) -> Result<(), RuneTheoremValidatorErrorV1> {
    let argument_bytes = config
        .arguments
        .iter()
        .try_fold(0_usize, |total, argument| {
            total.checked_add(argument.as_encoded_bytes().len())
        });
    if config.program.as_os_str().is_empty()
        || config.arguments.len() > MAX_ARGUMENTS
        || argument_bytes.is_none_or(|value| value > MAX_ARGUMENT_BYTES)
        || config.max_output_bytes == 0
        || config.max_output_bytes > MAX_OUTPUT_BYTES
        || config.timeout.is_zero()
        || config.timeout > MAX_TIMEOUT
        || config.validator == [0; 16]
    {
        return Err(RuneTheoremValidatorErrorV1::InvalidConfig);
    }
    Ok(())
}

fn run(config: &RuneTheoremValidatorConfigV1) -> Result<Vec<u8>, RuneTheoremValidatorErrorV1> {
    let mut child = Command::new(&config.program)
        .args(&config.arguments)
        .arg("--format=json")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| RuneTheoremValidatorErrorV1::Spawn(error.kind()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or(RuneTheoremValidatorErrorV1::InvalidOutput)?;
    let limit = u64::try_from(config.max_output_bytes)
        .map_err(|_| RuneTheoremValidatorErrorV1::InvalidConfig)?
        .saturating_add(1);
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(limit)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
            .map_err(|error| error.kind())
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| RuneTheoremValidatorErrorV1::Wait(error.kind()))?
        {
            break status;
        }
        if started.elapsed() >= config.timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(RuneTheoremValidatorErrorV1::Timeout);
        }
        thread::sleep(Duration::from_millis(2));
    };
    let output = reader
        .join()
        .map_err(|_| RuneTheoremValidatorErrorV1::InvalidOutput)?
        .map_err(RuneTheoremValidatorErrorV1::Read)?;
    if !status.success() {
        return Err(RuneTheoremValidatorErrorV1::ProcessFailed);
    }
    if output.len() > config.max_output_bytes {
        return Err(RuneTheoremValidatorErrorV1::OutputLimit);
    }
    Ok(output)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceRow {
    verdict: Verdict,
    lens_rune_id: String,
    tier: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    Grounded,
    Refused,
    HoldQuantum,
    Mu,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AggregateRow {
    aggregate: bool,
    grounded: usize,
    refused: usize,
    hold_quantum: usize,
    mu: usize,
    total: usize,
    tier: String,
    anchor: String,
    promotion_path: String,
}

fn parse_verdict(bytes: &[u8]) -> Result<bool, RuneTheoremValidatorErrorV1> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| RuneTheoremValidatorErrorV1::InvalidOutput)?;
    let mut identities = BTreeSet::new();
    let mut counts = [0_usize; 4];
    let mut aggregate = None;
    for line in text.lines() {
        if line.is_empty() || aggregate.is_some() {
            return Err(RuneTheoremValidatorErrorV1::InvalidOutput);
        }
        if let Ok(row) = serde_json::from_str::<InstanceRow>(line) {
            if row.tier != "pt-4"
                || row.lens_rune_id.is_empty()
                || row.lens_rune_id.len() > 256
                || !identities.insert(row.lens_rune_id)
            {
                return Err(RuneTheoremValidatorErrorV1::InvalidOutput);
            }
            counts[match row.verdict {
                Verdict::Grounded => 0,
                Verdict::Refused => 1,
                Verdict::HoldQuantum => 2,
                Verdict::Mu => 3,
            }] += 1;
        } else {
            aggregate = Some(
                serde_json::from_str::<AggregateRow>(line)
                    .map_err(|_| RuneTheoremValidatorErrorV1::InvalidOutput)?,
            );
        }
    }
    let row = aggregate.ok_or(RuneTheoremValidatorErrorV1::InvalidOutput)?;
    if !row.aggregate
        || row.tier != "pt-4"
        || row.anchor != "Mac Lane CWM §III.4"
        || row.promotion_path.is_empty()
        || row.total == 0
        || [row.grounded, row.refused, row.hold_quantum, row.mu] != counts
        || row.total != identities.len()
        || row.hold_quantum != 0
        || row.mu != 0
    {
        return Err(RuneTheoremValidatorErrorV1::InvalidOutput);
    }
    Ok(row.refused == 0)
}
