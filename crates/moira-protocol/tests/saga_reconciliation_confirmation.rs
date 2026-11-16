use moira_protocol::*;

fn view() -> SagaReconciliationViewV1 {
    SagaReconciliationViewV1 {
        saga: SagaViewV1 {
            saga: SagaIdV1(7),
            phase: SagaPhaseV1::FencedUncertain,
            deadline: 50,
            timer: TimerIdV1(3),
            current_effect: Some(SagaEffectIdV1(9)),
            current_kind: Some(SagaEffectKindV1::Forward),
            retries_used: 1,
            retry_budget: 2,
        },
        last_confirmed_revision: 4,
        retained_evidence: 1,
        fence: None,
        automatic_progress: AutomaticProgressV1::BlockedUncertain,
    }
}

#[test]
fn confirmation_binds_revision_and_disposition_and_derives_replay_identity() {
    let proposal = ReconciliationProposalV1 {
        saga: SagaIdV1(7),
        disposition: OperatorDispositionV1::KeepUncertain,
    };
    let confirmation = reconciliation_confirmation(5, view(), proposal).unwrap();
    let selected = ConfirmedReconciliationV1 {
        storage_revision: 5,
        saga: proposal.saga,
        disposition: proposal.disposition,
        action: confirmation.action,
        confirmation_digest: confirmation.confirmation_digest,
    };
    let first = confirmed_reconciliation_operation(confirmation, selected).unwrap();
    let replay = confirmed_reconciliation_operation(confirmation, selected).unwrap();
    assert_eq!(first, replay);
    assert_eq!(first.action, confirmation.action);
    assert_ne!(first.action.0, [0; 16]);
    assert!(
        confirmed_reconciliation_operation(
            confirmation,
            ConfirmedReconciliationV1 {
                disposition: OperatorDispositionV1::FailTerminal,
                ..selected
            }
        )
        .is_none()
    );
    assert_ne!(
        confirmation.confirmation_digest,
        reconciliation_confirmation(6, view(), proposal)
            .unwrap()
            .confirmation_digest
    );
}
