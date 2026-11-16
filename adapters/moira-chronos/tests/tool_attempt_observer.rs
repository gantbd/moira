use chronos_client::WireClientV1;
use chronos_protocol as chronos;
use moira_chronos::{
    AttemptBindingV1, AttemptObservationErrorV1, AttemptObserverConfigurationErrorV1,
    CurrentAttemptObserverV1,
};

#[derive(Clone, Copy)]
enum Mode {
    Exact,
    Ready,
    WrongAttempt,
    WrongIncarnation,
    Malformed,
    Unavailable,
}

struct Port {
    mode: Mode,
    calls: usize,
}

impl WireClientV1 for Port {
    type Error = ();

    fn request(&mut self, request: chronos::Request) -> Result<chronos::Response, Self::Error> {
        self.calls += 1;
        if matches!(self.mode, Mode::Unavailable) {
            return Err(());
        }
        let chronos::Command::Snapshot { task } = request.command else {
            panic!("snapshot expected")
        };
        let (phase, attempt, incarnation) = match self.mode {
            Mode::Exact | Mode::Malformed => (chronos::Phase::Running, Some(3), "7"),
            Mode::Ready => (chronos::Phase::Ready, None, "7"),
            Mode::WrongAttempt => (chronos::Phase::Running, Some(4), "7"),
            Mode::WrongIncarnation => (chronos::Phase::Running, Some(3), "8"),
            Mode::Unavailable => unreachable!(),
        };
        Ok(chronos::Response {
            version: if matches!(self.mode, Mode::Malformed) {
                2
            } else {
                1
            },
            outcome: chronos::Outcome::Snapshot {
                value: Some(chronos::Entry {
                    reference: chronos::Reference {
                        task,
                        incarnation: incarnation.into(),
                    },
                    snapshot: chronos::Snapshot {
                        phase,
                        attempt_number: attempt,
                        dependency_count: 0,
                        required_capability_count: 1,
                        priority: 0,
                        max_attempts: 4,
                    },
                }),
                revision: Some(chronos::Revision::Finite("9".into())),
            },
        })
    }
}

fn limits() -> chronos::Limits {
    chronos::Limits {
        max_frame_bytes: 4_096,
        max_id_bytes: 64,
        max_graph_tasks: 4,
        max_dependencies_per_task: 4,
        max_capabilities_per_task: 4,
        max_capability_bytes: 64,
        max_page_entries: 4,
    }
}

fn observer(mode: Mode) -> CurrentAttemptObserverV1<Port> {
    CurrentAttemptObserverV1::new(
        Port { mode, calls: 0 },
        limits(),
        4,
        vec![AttemptBindingV1 {
            external_task_key: [2; 16],
            chronos_task_id: "moira-task".into(),
        }],
    )
    .unwrap()
}

#[test]
fn exact_running_attempt_becomes_tool_gate_evidence() {
    let mut observer = observer(Mode::Exact);
    let evidence = observer.observe([2; 16], 7, 3).unwrap();
    assert_eq!(evidence.task_key, [2; 16]);
    assert_eq!(evidence.incarnation, 7);
    assert_eq!(evidence.attempt, 3);
    assert_eq!(observer.into_port().calls, 1);
}

#[test]
fn stale_or_unverifiable_attempts_never_become_evidence() {
    for mode in [Mode::Ready, Mode::WrongAttempt, Mode::WrongIncarnation] {
        assert_eq!(
            observer(mode).observe([2; 16], 7, 3),
            Err(AttemptObservationErrorV1::NotCurrent)
        );
    }
    assert_eq!(
        observer(Mode::Malformed).observe([2; 16], 7, 3),
        Err(AttemptObservationErrorV1::InvalidResponse)
    );
    assert_eq!(
        observer(Mode::Unavailable).observe([2; 16], 7, 3),
        Err(AttemptObservationErrorV1::Unavailable(()))
    );
    let mut unknown = observer(Mode::Exact);
    assert_eq!(
        unknown.observe([99; 16], 7, 3),
        Err(AttemptObservationErrorV1::NotCurrent)
    );
    assert_eq!(unknown.into_port().calls, 0);
}

#[test]
fn registry_aliases_and_unbounded_configuration_refuse() {
    let binding = AttemptBindingV1 {
        external_task_key: [2; 16],
        chronos_task_id: "moira-task".into(),
    };
    let port = || Port {
        mode: Mode::Exact,
        calls: 0,
    };
    assert!(matches!(
        CurrentAttemptObserverV1::new(port(), limits(), 0, vec![binding.clone()]),
        Err(AttemptObserverConfigurationErrorV1::InvalidLimits)
    ));
    assert!(matches!(
        CurrentAttemptObserverV1::new(port(), limits(), 1, vec![]),
        Err(AttemptObserverConfigurationErrorV1::EmptyRegistry)
    ));
    assert!(matches!(
        CurrentAttemptObserverV1::new(port(), limits(), 1, vec![binding.clone(), binding]),
        Err(AttemptObserverConfigurationErrorV1::RegistryLimit)
    ));
}
