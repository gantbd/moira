#![cfg(unix)]

use moira_core::persistence::CodecLimits;
use moira_protocol::{
    CommandV1, MessageRefV1, MutationV1, OutcomeV1, ParticipantRefV1, ReplyV1, TransitionV1,
};
use moira_service::Service;
use moira_sqlite::{SqliteStore, StoreLimits, Timing};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "moira-service-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn limits() -> StoreLimits {
    StoreLimits {
        codec: CodecLimits {
            max_bytes: 8_192,
            max_workflows: 8,
        },
        max_database_pages: 64,
        timing: Timing::Off,
    }
}

#[test]
fn native_service_retains_exact_transition_and_read_model_across_sqlite_reopen() {
    let path = path();
    let owner = [7; 16];
    let mutation = MutationV1::AdmitMessage {
        message: MessageRefV1 {
            source: [1; 16],
            room_sequence: 2,
            message_sequence: 3,
        },
        recipient: ParticipantRefV1([4; 16]),
    };
    let mut service = Service::new(SqliteStore::open(&path, owner, limits()).unwrap());
    let ReplyV1::Accepted(OutcomeV1::Applied {
        transition,
        replayed: false,
        ..
    }) = service
        .execute(CommandV1::Apply {
            expected_revision: 0,
            mutation,
        })
        .unwrap()
    else {
        panic!("durable admission expected");
    };
    let TransitionV1::ChronosSubmissionPrepared { workflow, .. } = transition else {
        panic!("submission expected");
    };
    drop(service);

    let mut reopened = Service::new(SqliteStore::open(&path, owner, limits()).unwrap());
    assert!(matches!(
        reopened.execute(CommandV1::Status).unwrap(),
        ReplyV1::Accepted(OutcomeV1::Status(status))
            if status.revision == 1 && !status.needs_reopen
    ));
    assert!(matches!(
        reopened
            .execute(CommandV1::Workflow { workflow })
            .unwrap(),
        ReplyV1::Accepted(OutcomeV1::Workflow(Some(snapshot)))
            if snapshot.workflow == workflow
    ));
    assert!(matches!(
        reopened
            .execute(CommandV1::Apply {
                expected_revision: 1,
                mutation,
            })
            .unwrap(),
        ReplyV1::Accepted(OutcomeV1::Applied { replayed: true, .. })
    ));
    drop(reopened);
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

#[test]
fn attempt_lookup_requires_current_scope_and_receipt_evidence_after_reopen() {
    use moira_protocol::{
        AdmissionReceiptV1, AttemptReceiptV1, DispositionV1, ResultReceiptV1, TaskRefV1,
    };
    use moira_service::{
        AccessV1,
        attempt_lookup::{AttemptLookup, AttemptLookupPolicy, ExactAttempt, ExecutionEvidence},
    };
    struct Policy(bool);
    impl AttemptLookupPolicy for Policy {
        fn allows_lookup(&mut self, _: &ExactAttempt<'_>) -> bool {
            true
        }
        fn bind_task(
            &mut self,
            attempt: &ExactAttempt<'_>,
        ) -> Option<moira_core::ChronosTaskRefV1> {
            (attempt.service_scope == "atlas" && attempt.task == "review")
                .then(|| moira_core::ChronosTaskRefV1::new([3; 16], attempt.incarnation))
        }
        fn allows_result(&mut self, _: &ExactAttempt<'_>, _: moira_protocol::WorkflowIdV1) -> bool {
            self.0
        }
    }
    fn apply(
        service: &mut Service<SqliteStore>,
        revision: u64,
        mutation: MutationV1,
    ) -> TransitionV1 {
        match service
            .execute(CommandV1::Apply {
                expected_revision: revision,
                mutation,
            })
            .unwrap()
        {
            ReplyV1::Accepted(OutcomeV1::Applied { transition, .. }) => transition,
            other => panic!("unexpected transition {other:?}"),
        }
    }
    let path = path();
    let owner = [7; 16];
    let mut service = Service::new(SqliteStore::open(&path, owner, limits()).unwrap());
    let TransitionV1::ChronosSubmissionPrepared { workflow, effect } = apply(
        &mut service,
        0,
        MutationV1::AdmitMessage {
            message: MessageRefV1 {
                source: [1; 16],
                room_sequence: 2,
                message_sequence: 3,
            },
            recipient: ParticipantRefV1([4; 16]),
        },
    ) else {
        panic!("submission expected")
    };
    let task = TaskRefV1 {
        external_task_key: [3; 16],
        incarnation: 4,
    };
    apply(
        &mut service,
        1,
        MutationV1::AcceptChronosAdmission {
            workflow,
            receipt: AdmissionReceiptV1 {
                effect,
                task,
                chronos_revision: 1,
            },
        },
    );
    let TransitionV1::DaimonExecutionPrepared { effect, .. } = apply(
        &mut service,
        2,
        MutationV1::AcceptChronosAttempt {
            workflow,
            receipt: AttemptReceiptV1 {
                task,
                attempt_number: 1,
                chronos_revision: 2,
            },
        },
    ) else {
        panic!("execution expected")
    };
    let read = AccessV1 {
        read: true,
        mutate: false,
    };
    let exact = ExactAttempt {
        service_scope: "atlas",
        task: "review",
        incarnation: 4,
        attempt: 1,
    };
    assert_eq!(
        service.lookup_attempt(read, &exact, &mut Policy(true)),
        AttemptLookup::Found {
            workflow,
            execution: ExecutionEvidence::ReceiptPending
        }
    );
    apply(
        &mut service,
        3,
        MutationV1::AcceptDaimonResult {
            workflow,
            receipt: ResultReceiptV1 {
                effect,
                execution_id: [9; 16],
                task,
                attempt_number: 1,
                disposition: DispositionV1::Uncertain,
            },
        },
    );
    drop(service);
    let reopened = Service::new(SqliteStore::open(&path, owner, limits()).unwrap());
    assert_eq!(
        reopened.lookup_attempt(read, &exact, &mut Policy(true)),
        AttemptLookup::Found {
            workflow,
            execution: ExecutionEvidence::Recorded {
                execution_id: [9; 16],
                disposition: DispositionV1::Uncertain,
            },
        }
    );
    assert_eq!(
        reopened.lookup_attempt(read, &exact, &mut Policy(false)),
        AttemptLookup::Denied
    );
    assert_eq!(
        reopened.lookup_attempt(
            AccessV1 {
                read: false,
                mutate: true
            },
            &exact,
            &mut Policy(true)
        ),
        AttemptLookup::Denied
    );
    for mismatch in [
        ExactAttempt {
            service_scope: "boreal",
            ..exact
        },
        ExactAttempt {
            task: "another-task",
            ..exact
        },
        ExactAttempt {
            incarnation: 5,
            ..exact
        },
        ExactAttempt {
            attempt: 2,
            ..exact
        },
    ] {
        assert_eq!(
            reopened.lookup_attempt(read, &mismatch, &mut Policy(true)),
            AttemptLookup::Unresolved
        );
    }
    drop(reopened);
    for suffix in ["", "-journal", ".owner.lock"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}
