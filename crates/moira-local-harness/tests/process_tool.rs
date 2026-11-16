#![cfg(unix)]

use moira_core::{
    EffectId, WorkflowId,
    tool_continuation::{
        AttemptEvidenceV1, AuthorityDecisionIdV1, AuthorityEvidenceV1, AuthoritySubjectIdV1,
        ToolBudgetV1, ToolCallV1, ToolCapabilityIdV1, ToolExecutionIdV1, ToolGateV1,
        ToolRequestIdV1,
    },
};
use moira_host::{ToolExecutionOutcomeV1, ToolExecutorV1, ToolInvocationV1};
use moira_local_harness::process_tool::{
    ProcessToolConfigErrorV1, ProcessToolErrorV1, ProcessToolExecutorV1, ToolProcessProfileV1,
};
use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "moira-process-tool-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn script(&self) -> PathBuf {
        let path = self.0.join("tool");
        fs::write(&path, b"#!/bin/sh\n/bin/cat\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(&self.0);
    }
}

fn profile(
    capability: u8,
    executable: PathBuf,
    arguments: Vec<OsString>,
    directory: &Fixture,
) -> ToolProcessProfileV1 {
    ToolProcessProfileV1 {
        capability: ToolCapabilityIdV1([capability; 16]),
        executable,
        arguments,
        working_directory: directory.0.clone(),
        max_elapsed: Duration::from_millis(500),
        max_input_bytes: 1_024,
        max_output_bytes: 1_024,
        max_stderr_bytes: 1_024,
        max_executable_bytes: 2 * 1_024 * 1_024,
    }
}

fn invocation(capability: u8, arguments: &[u8], max_output_bytes: usize) -> ToolInvocationV1 {
    ToolInvocationV1 {
        workflow: WorkflowId::new([1; 16]),
        effect: EffectId::new([2; 16]),
        execution: ToolExecutionIdV1([3; 16]),
        gate: ToolGateV1 {
            authority: AuthorityEvidenceV1 {
                decision: AuthorityDecisionIdV1([4; 16]),
                subject: AuthoritySubjectIdV1([5; 16]),
                capability: ToolCapabilityIdV1([capability; 16]),
                epoch: 6,
            },
            attempt: AttemptEvidenceV1 {
                task_key: [7; 16],
                incarnation: 8,
                attempt: 1,
            },
        },
        call: ToolCallV1 {
            request: ToolRequestIdV1([9; 16]),
            capability: ToolCapabilityIdV1([capability; 16]),
            provider_correlation: b"call-9".to_vec(),
            arguments: arguments.to_vec(),
            budget: ToolBudgetV1 {
                max_elapsed_millis: 250,
                max_output_bytes,
            },
        },
    }
}

#[test]
fn fixed_profile_executes_without_a_shell_and_returns_exact_stdout() {
    let fixture = Fixture::new();
    let mut executor = ProcessToolExecutorV1::try_new(
        vec![profile(10, PathBuf::from("/bin/cat"), Vec::new(), &fixture)],
        Duration::from_millis(1),
    )
    .unwrap();
    let output = executor
        .execute(&invocation(10, br#"{"path":"README.md"}"#, 1_024))
        .unwrap();
    let ToolExecutionOutcomeV1::Succeeded {
        evidence_digest,
        output,
    } = output
    else {
        panic!("success expected")
    };
    assert_ne!(evidence_digest, [0; 32]);
    assert_eq!(output, br#"{"path":"README.md"}"#);
    assert!(fs::read_dir(&fixture.0).unwrap().next().is_none());
}

#[test]
fn timeout_nonzero_exit_and_output_limit_are_attributed_failures() {
    let fixture = Fixture::new();
    let mut timeout = profile(
        11,
        PathBuf::from("/bin/sleep"),
        vec![OsString::from("1")],
        &fixture,
    );
    timeout.max_elapsed = Duration::from_millis(20);
    timeout.max_input_bytes = 512 * 1_024;
    let limited = ToolProcessProfileV1 {
        max_output_bytes: 4,
        ..profile(12, PathBuf::from("/bin/cat"), Vec::new(), &fixture)
    };
    let mut executor = ProcessToolExecutorV1::try_new(
        vec![
            timeout,
            profile(13, PathBuf::from("/usr/bin/false"), Vec::new(), &fixture),
            limited,
        ],
        Duration::from_millis(1),
    )
    .unwrap();
    let started = Instant::now();
    assert!(matches!(
        executor
            .execute(&invocation(11, &vec![b'x'; 256 * 1_024], 1_024))
            .unwrap(),
        ToolExecutionOutcomeV1::Failed { .. }
    ));
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(matches!(
        executor.execute(&invocation(13, b"{}", 1_024)).unwrap(),
        ToolExecutionOutcomeV1::Failed { .. }
    ));
    assert!(matches!(
        executor.execute(&invocation(12, b"12345", 4)).unwrap(),
        ToolExecutionOutcomeV1::Failed { .. }
    ));
}

#[test]
fn executable_drift_and_unknown_capability_never_dispatch_as_success() {
    let fixture = Fixture::new();
    let script = fixture.script();
    let mut executor = ProcessToolExecutorV1::try_new(
        vec![profile(14, script.clone(), Vec::new(), &fixture)],
        Duration::from_millis(1),
    )
    .unwrap();
    let mut mismatched_gate = invocation(14, b"{}", 1_024);
    mismatched_gate.gate.authority.capability = ToolCapabilityIdV1([15; 16]);
    assert!(matches!(
        executor.execute(&mismatched_gate).unwrap(),
        ToolExecutionOutcomeV1::Failed { .. }
    ));
    fs::write(&script, b"#!/bin/sh\nexit 0\n").unwrap();
    assert_eq!(
        executor.execute(&invocation(14, b"{}", 1_024)),
        Err(ProcessToolErrorV1::ProfileDrift)
    );
    assert_eq!(
        executor.execute(&invocation(99, b"{}", 1_024)),
        Err(ProcessToolErrorV1::UnknownCapability)
    );
}

#[test]
fn configuration_refuses_relative_paths_and_duplicate_capabilities() {
    let fixture = Fixture::new();
    assert_eq!(
        ProcessToolExecutorV1::try_new(
            vec![profile(10, PathBuf::from("cat"), Vec::new(), &fixture)],
            Duration::from_millis(1),
        )
        .err(),
        Some(ProcessToolConfigErrorV1::InvalidPath)
    );
    assert_eq!(
        ProcessToolExecutorV1::try_new(
            vec![
                profile(10, PathBuf::from("/bin/cat"), Vec::new(), &fixture),
                profile(10, PathBuf::from("/bin/cat"), Vec::new(), &fixture),
            ],
            Duration::from_millis(1),
        )
        .err(),
        Some(ProcessToolConfigErrorV1::DuplicateCapability)
    );
}
