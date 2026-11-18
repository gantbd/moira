use moira_protocol::{WorkflowIdV1, agent_participation as p, deliberation as d};

fn view() -> p::ParticipationViewV1 {
    let mut option_ids = [p::PermissionOptionIdV1([0; 16]); p::MAX_PERMISSION_OPTIONS_V1];
    option_ids[0] = p::PermissionOptionIdV1([21; 16]);
    option_ids[1] = p::PermissionOptionIdV1([22; 16]);
    let turn = p::AgentTurnIdV1([18; 16]);
    p::ParticipationViewV1 {
        binding: p::BindingV1 {
            participation: p::ParticipationIdV1([1; 16]),
            workflow: WorkflowIdV1([2; 16]),
            deliberation: d::DeliberationIdV1([3; 16]),
            member: d::EnsembleMemberIdV1([4; 16]),
            session: p::AgentSessionIdV1([5; 16]),
            profile: p::ProfileIdV1([6; 16]),
            attempt: p::AttemptV1 {
                task_key: [7; 16],
                incarnation: 8,
                attempt: 9,
            },
            method: d::MethodRefV1 {
                machine_digest: [10; 32],
                state_digest: [11; 32],
            },
            grounding: d::GroundingRefV1 {
                authority_digest: [12; 32],
                surface_digest: [13; 32],
                depth: d::SurfaceDepthV1::Reference,
            },
            persona: d::PersonaLensV1::Historian,
        },
        revision: 14,
        phase: p::PhaseV1::AwaitingPermission,
        session_revision: Some(15),
        last_event_cursor: Some(16),
        profile_document_digest: Some([17; 32]),
        capabilities: p::CapabilitiesV1 {
            permissions: true,
            ..p::CapabilitiesV1::default()
        },
        active_turn: Some(turn),
        permission: Some(p::PermissionEvidenceV1 {
            request: p::PermissionRequestIdV1([19; 16]),
            turn,
            capability_digest: [20; 32],
            options_digest: [23; 32],
            option_count: 2,
            option_ids,
        }),
        terminal: None,
        pending: None,
    }
}

#[test]
fn confirmation_binds_durable_context_and_derives_replay_stable_identities() {
    let value = view();
    let confirmation = p::permission_confirmation(24, &value).unwrap();
    assert_ne!(confirmation.confirmation_digest, [0; 32]);
    let selected = p::ConfirmedPermissionSelectionV1 {
        participation: value.binding.participation,
        confirmation_digest: confirmation.confirmation_digest,
        option: p::PermissionOptionIdV1([21; 16]),
    };
    let first =
        p::confirmed_permission_selection(confirmation, selected, value.binding.attempt).unwrap();
    let replay =
        p::confirmed_permission_selection(confirmation, selected, value.binding.attempt).unwrap();
    assert_eq!(first, replay);
    assert_ne!(first.key.0, [0; 16]);
    assert_ne!(first.execution.0, [0; 16]);

    let alternate = p::confirmed_permission_selection(
        confirmation,
        p::ConfirmedPermissionSelectionV1 {
            option: p::PermissionOptionIdV1([22; 16]),
            ..selected
        },
        value.binding.attempt,
    )
    .unwrap();
    assert_ne!(first.key, alternate.key);
    assert_ne!(first.execution, alternate.execution);
}

#[test]
fn stale_context_and_nonmember_options_fail_closed() {
    let value = view();
    let first = p::permission_confirmation(24, &value).unwrap();
    let mut changed = value.clone();
    changed.binding.method.state_digest[0] ^= 1;
    assert_ne!(
        first.confirmation_digest,
        p::permission_confirmation(24, &changed)
            .unwrap()
            .confirmation_digest
    );
    assert_ne!(
        first.confirmation_digest,
        p::permission_confirmation(25, &value)
            .unwrap()
            .confirmation_digest
    );
    assert!(
        p::confirmed_permission_selection(
            first,
            p::ConfirmedPermissionSelectionV1 {
                participation: value.binding.participation,
                confirmation_digest: first.confirmation_digest,
                option: p::PermissionOptionIdV1([99; 16]),
            },
            value.binding.attempt,
        )
        .is_none()
    );

    let mut malformed = first;
    malformed.option_count = u8::MAX;
    assert!(
        p::confirmed_permission_selection(
            malformed,
            p::ConfirmedPermissionSelectionV1 {
                participation: value.binding.participation,
                confirmation_digest: malformed.confirmation_digest,
                option: p::PermissionOptionIdV1([21; 16]),
            },
            value.binding.attempt,
        )
        .is_none()
    );

    changed = value;
    changed.permission.as_mut().unwrap().option_count = 0;
    assert!(p::permission_confirmation(24, &changed).is_none());

    changed = view();
    changed.pending = Some(p::OperationV1::Cancel {
        key: p::IdempotencyKeyV1([31; 16]),
        turn: changed.active_turn.unwrap(),
    });
    assert!(p::permission_confirmation(24, &changed).is_none());
}

#[test]
fn cancellation_confirmation_binds_active_turn_and_derives_only_the_key() {
    let mut value = view();
    value.phase = p::PhaseV1::TurnActive;
    value.permission = None;
    let confirmation = p::cancel_confirmation(24, &value).unwrap();
    let selected = p::ConfirmedCancelV1 {
        participation: value.binding.participation,
        confirmation_digest: confirmation.confirmation_digest,
    };
    let first = p::confirmed_cancel_operation(confirmation, selected).unwrap();
    let replay = p::confirmed_cancel_operation(confirmation, selected).unwrap();
    assert_eq!(first, replay);
    assert!(matches!(
        first,
        p::OperationV1::Cancel { key, turn }
            if key.0 != [0; 16] && turn == value.active_turn.unwrap()
    ));

    value.revision += 1;
    assert_ne!(
        confirmation.confirmation_digest,
        p::cancel_confirmation(24, &value)
            .unwrap()
            .confirmation_digest
    );
    value.pending = Some(first);
    assert!(p::cancel_confirmation(24, &value).is_none());
}

#[test]
fn prompt_preparation_binds_bytes_and_full_ready_context() {
    let mut value = view();
    value.phase = p::PhaseV1::Ready;
    value.active_turn = None;
    value.permission = None;
    let digest = p::agent_prompt_digest(b"use the retained evidence").unwrap();
    let first = p::prepared_prompt_operation(24, &value, digest).unwrap();
    let replay = p::prepared_prompt_operation(24, &value, digest).unwrap();
    assert_eq!(first, replay);
    assert!(matches!(
        first,
        p::OperationV1::Prompt {
            key,
            turn,
            execution,
            prompt_digest,
            attempt,
        } if key.0 != [0; 16]
            && turn.0 != [0; 16]
            && execution.0 != [0; 16]
            && prompt_digest == digest
            && attempt == value.binding.attempt
    ));

    let other_digest = p::agent_prompt_digest(b"use different evidence").unwrap();
    assert_ne!(
        first,
        p::prepared_prompt_operation(24, &value, other_digest).unwrap()
    );
    value.binding.grounding.surface_digest[0] ^= 1;
    assert_ne!(
        first,
        p::prepared_prompt_operation(24, &value, digest).unwrap()
    );
    value.pending = Some(first);
    assert!(p::prepared_prompt_operation(24, &value, digest).is_none());
    assert!(p::agent_prompt_digest(b"").is_none());
}

#[test]
fn open_preparation_binds_the_complete_admitted_participation() {
    let mut value = view();
    value.revision = 0;
    value.phase = p::PhaseV1::Admitted;
    value.session_revision = None;
    value.last_event_cursor = None;
    value.profile_document_digest = None;
    value.capabilities = p::CapabilitiesV1::default();
    value.active_turn = None;
    value.permission = None;
    let first = p::prepared_open_operation(24, &value).unwrap();
    assert_eq!(first, p::prepared_open_operation(24, &value).unwrap());
    assert!(matches!(
        first,
        p::OperationV1::Open { key, execution }
            if key.0 != [0; 16] && execution.0 != [0; 16]
    ));

    let mut changed = value.clone();
    changed.binding.attempt.incarnation += 1;
    assert_ne!(first, p::prepared_open_operation(24, &changed).unwrap());
    assert_ne!(first, p::prepared_open_operation(25, &value).unwrap());
    value.pending = Some(first);
    assert!(p::prepared_open_operation(24, &value).is_none());
    value.pending = None;
    value.phase = p::PhaseV1::Ready;
    assert!(p::prepared_open_operation(24, &value).is_none());
}

#[test]
fn reattach_confirmation_binds_detached_methodical_context_without_exposing_attempt() {
    let mut value = view();
    value.phase = p::PhaseV1::ReattachRequired;
    value.active_turn = None;
    value.permission = None;
    value.capabilities.resume_session = true;
    let confirmation = p::reattach_confirmation(24, &value).unwrap();
    let selected = p::ConfirmedReattachV1 {
        participation: value.binding.participation,
        confirmation_digest: confirmation.confirmation_digest,
    };
    let first =
        p::confirmed_reattach_operation(confirmation, selected, value.binding.attempt).unwrap();
    assert_eq!(
        first,
        p::confirmed_reattach_operation(confirmation, selected, value.binding.attempt).unwrap()
    );
    assert!(matches!(
        first,
        p::OperationV1::Reattach {
            key,
            execution,
            attempt,
        } if key.0 != [0; 16]
            && execution.0 != [0; 16]
            && attempt == value.binding.attempt
    ));

    value.binding.method.state_digest[0] ^= 1;
    assert_ne!(
        confirmation.confirmation_digest,
        p::reattach_confirmation(24, &value)
            .unwrap()
            .confirmation_digest
    );
    value.pending = Some(first);
    assert!(p::reattach_confirmation(24, &value).is_none());
}
