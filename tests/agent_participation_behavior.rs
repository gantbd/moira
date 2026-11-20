use moira_core::{WorkflowId, agent_participation as agent, deliberation};

#[test]
fn exact_binding_permission_authority_terminal_and_close_are_durable() {
    let mut engine = agent::Engine::try_new(agent::LimitsV1::default()).unwrap();
    let binding = binding();
    assert_eq!(engine.admit(binding).unwrap().revision, 0);
    assert!(engine.admit(binding).unwrap().replayed);

    let open = agent::OperationV1::Open {
        key: agent::IdempotencyKeyV1([10; 16]),
        execution: agent::ExecutionIdV1([11; 16]),
    };
    assert_eq!(
        engine
            .prepare(binding.participation, 0, open)
            .unwrap()
            .revision,
        1
    );
    assert!(
        engine
            .prepare(binding.participation, 0, open)
            .unwrap()
            .replayed
    );
    let ready = observation(agent::PhaseV1::Ready, 1, None, None, None);
    assert_eq!(
        engine
            .observe(binding.participation, 1, open.key(), ready)
            .unwrap()
            .revision,
        2
    );
    assert!(
        engine
            .observe(binding.participation, 1, open.key(), ready)
            .unwrap()
            .replayed
    );

    let turn = agent::AgentTurnIdV1([12; 16]);
    let prompt = agent::OperationV1::Prompt {
        key: agent::IdempotencyKeyV1([13; 16]),
        turn,
        execution: agent::ExecutionIdV1([14; 16]),
        prompt_digest: [15; 32],
        attempt: binding.attempt,
    };
    engine.prepare(binding.participation, 2, prompt).unwrap();
    let permission = agent::PermissionEvidenceV1 {
        request: agent::PermissionRequestIdV1([16; 16]),
        turn,
        capability_digest: [17; 32],
        options_digest: [18; 32],
        option_count: 1,
        option_ids: [
            agent::PermissionOptionIdV1([20; 16]),
            agent::PermissionOptionIdV1([0; 16]),
            agent::PermissionOptionIdV1([0; 16]),
            agent::PermissionOptionIdV1([0; 16]),
            agent::PermissionOptionIdV1([0; 16]),
            agent::PermissionOptionIdV1([0; 16]),
            agent::PermissionOptionIdV1([0; 16]),
            agent::PermissionOptionIdV1([0; 16]),
        ],
    };
    engine
        .observe(
            binding.participation,
            3,
            prompt.key(),
            observation(
                agent::PhaseV1::AwaitingPermission,
                2,
                Some(turn),
                Some(permission),
                None,
            ),
        )
        .unwrap();

    let wrong = agent::OperationV1::DecidePermission {
        key: agent::IdempotencyKeyV1([19; 16]),
        execution: agent::ExecutionIdV1([24; 16]),
        request: permission.request,
        option: agent::PermissionOptionIdV1([20; 16]),
        authority: agent::PermissionAuthorityV1 {
            decision: agent::AuthorityDecisionIdV1([21; 16]),
            subject: agent::AuthoritySubjectIdV1([22; 16]),
            capability_digest: [23; 32],
            epoch: 1,
        },
        attempt: binding.attempt,
    };
    assert_eq!(
        engine.prepare(binding.participation, 4, wrong),
        Err(agent::RefusalV1::InvalidOperation)
    );
    let decide = agent::OperationV1::DecidePermission {
        key: agent::IdempotencyKeyV1([19; 16]),
        execution: agent::ExecutionIdV1([24; 16]),
        request: permission.request,
        option: agent::PermissionOptionIdV1([20; 16]),
        authority: agent::PermissionAuthorityV1 {
            decision: agent::AuthorityDecisionIdV1([21; 16]),
            subject: agent::AuthoritySubjectIdV1([22; 16]),
            capability_digest: permission.capability_digest,
            epoch: 1,
        },
        attempt: binding.attempt,
    };
    engine.prepare(binding.participation, 4, decide).unwrap();
    engine
        .observe(
            binding.participation,
            5,
            decide.key(),
            observation(agent::PhaseV1::TurnActive, 3, Some(turn), None, None),
        )
        .unwrap();

    let advance = agent::OperationV1::Advance {
        key: agent::IdempotencyKeyV1([24; 16]),
        turn,
    };
    engine.prepare(binding.participation, 6, advance).unwrap();
    let terminal = agent::TerminalEvidenceV1 {
        turn,
        disposition: agent::DispositionV1::Completed,
        stop_reason: agent::StopReasonV1::EndTurn,
        output_digest: [25; 32],
        provider_evidence_digest: [26; 32],
    };
    engine
        .observe(
            binding.participation,
            7,
            advance.key(),
            observation(agent::PhaseV1::Terminal, 4, None, None, Some(terminal)),
        )
        .unwrap();

    let close = agent::OperationV1::Close {
        key: agent::IdempotencyKeyV1([27; 16]),
    };
    engine.prepare(binding.participation, 8, close).unwrap();
    engine
        .observe(
            binding.participation,
            9,
            close.key(),
            observation(agent::PhaseV1::Closed, 5, None, None, Some(terminal)),
        )
        .unwrap();
    let snapshot = engine.participation(binding.participation).unwrap();
    assert_eq!(snapshot.phase, agent::PhaseV1::Closed);
    assert_eq!(snapshot.terminal, Some(terminal));
    assert_eq!(snapshot.binding.method, binding.method);
    assert_eq!(snapshot.binding.grounding, binding.grounding);
    assert_eq!(snapshot.binding.persona, binding.persona);

    let recovered = agent::Engine::recover(engine.checkpoint()).unwrap();
    assert_eq!(
        recovered.participation(binding.participation),
        Some(snapshot)
    );
}

#[test]
fn unknown_outcome_fences_without_redispatch_and_can_be_reconciled() {
    let mut engine = agent::Engine::try_new(agent::LimitsV1::default()).unwrap();
    let binding = binding();
    engine.admit(binding).unwrap();
    let open = agent::OperationV1::Open {
        key: agent::IdempotencyKeyV1([30; 16]),
        execution: agent::ExecutionIdV1([31; 16]),
    };
    engine.prepare(binding.participation, 0, open).unwrap();
    assert_eq!(
        engine
            .fence_unknown(binding.participation, 1, open.key())
            .unwrap()
            .phase,
        agent::PhaseV1::FencedUnknown
    );
    assert!(
        engine
            .fence_unknown(binding.participation, 1, open.key())
            .unwrap()
            .replayed
    );
    assert!(
        engine
            .prepare(binding.participation, 0, open)
            .unwrap()
            .replayed
    );
    let recovered_checkpoint = engine.checkpoint();
    let mut recovered = agent::Engine::recover(recovered_checkpoint).unwrap();
    recovered
        .observe(
            binding.participation,
            2,
            open.key(),
            observation(agent::PhaseV1::Ready, 1, None, None, None),
        )
        .unwrap();
    assert_eq!(
        recovered
            .participation(binding.participation)
            .unwrap()
            .phase,
        agent::PhaseV1::Ready
    );
}

#[test]
fn detached_idle_binding_requires_a_derived_reattach_and_survives_recovery() {
    let mut engine = agent::Engine::try_new(agent::LimitsV1::default()).unwrap();
    let binding = binding();
    engine.admit(binding).unwrap();
    let open = agent::OperationV1::Open {
        key: agent::IdempotencyKeyV1([30; 16]),
        execution: agent::ExecutionIdV1([31; 16]),
    };
    engine.prepare(binding.participation, 0, open).unwrap();
    engine
        .observe(
            binding.participation,
            1,
            open.key(),
            observation(agent::PhaseV1::Ready, 1, None, None, None),
        )
        .unwrap();
    let marked = engine
        .mark_reattach_required(binding.participation, 2, 1)
        .unwrap();
    assert_eq!(marked.phase, agent::PhaseV1::ReattachRequired);
    assert!(
        engine
            .mark_reattach_required(binding.participation, 2, 1)
            .unwrap()
            .replayed
    );
    let operation = agent::OperationV1::Reattach {
        key: agent::IdempotencyKeyV1([32; 16]),
        execution: agent::ExecutionIdV1([33; 16]),
        attempt: binding.attempt,
    };
    engine.prepare(binding.participation, 3, operation).unwrap();
    engine
        .fence_unknown(binding.participation, 4, operation.key())
        .unwrap();
    let mut recovered = agent::Engine::recover(engine.checkpoint()).unwrap();
    recovered
        .observe(
            binding.participation,
            5,
            operation.key(),
            observation(agent::PhaseV1::Ready, 2, None, None, None),
        )
        .unwrap();
    assert_eq!(
        recovered
            .participation(binding.participation)
            .unwrap()
            .phase,
        agent::PhaseV1::Ready
    );
}

fn binding() -> agent::BindingV1 {
    agent::BindingV1 {
        participation: agent::ParticipationIdV1([1; 16]),
        workflow: WorkflowId::new([2; 16]),
        deliberation: deliberation::DeliberationIdV1([3; 16]),
        member: deliberation::EnsembleMemberIdV1([4; 16]),
        session: agent::AgentSessionIdV1([5; 16]),
        profile: agent::ProfileIdV1([6; 16]),
        attempt: agent::AttemptV1 {
            task_key: [7; 16],
            incarnation: 1,
            attempt: 1,
        },
        method: deliberation::MethodRefV1 {
            machine_digest: [8; 32],
            state_digest: [9; 32],
        },
        grounding: deliberation::GroundingRefV1 {
            authority_digest: [10; 32],
            surface_digest: [11; 32],
            depth: deliberation::SurfaceDepthV1::Activated,
        },
        persona: deliberation::PersonaLensV1::RigorReviewer,
    }
}

#[test]
fn chronos_first_incarnation_is_a_valid_exact_attempt_identity() {
    let mut binding = binding();
    binding.attempt.incarnation = 0;
    let mut engine = agent::Engine::try_new(agent::LimitsV1::default()).unwrap();

    assert_eq!(engine.admit(binding).unwrap().revision, 0);

    binding.attempt.task_key = [0; 16];
    assert_eq!(engine.admit(binding), Err(agent::RefusalV1::InvalidBinding));
}

fn observation(
    phase: agent::PhaseV1,
    session_revision: u64,
    active_turn: Option<agent::AgentTurnIdV1>,
    permission: Option<agent::PermissionEvidenceV1>,
    terminal: Option<agent::TerminalEvidenceV1>,
) -> agent::ObservationV1 {
    agent::ObservationV1 {
        phase,
        session_revision,
        active_turn,
        last_event_cursor: Some(session_revision),
        profile_document_digest: [40; 32],
        capabilities: agent::CapabilitiesV1 {
            resume_session: true,
            permissions: true,
            terminal: true,
            ..agent::CapabilitiesV1::default()
        },
        permission,
        terminal,
    }
}
