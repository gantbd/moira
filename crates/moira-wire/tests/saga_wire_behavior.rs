mod support;

use moira_protocol::*;
use moira_wire::{
    CodecErrorV1, LimitsV1, decode_saga_reply, decode_saga_request, encode_saga_reply,
    encode_saga_request,
};
use support::assert_canonical_mutations;

fn round_request(value: SagaCommandV1) {
    let bytes = encode_saga_request(&value, LimitsV1::default()).unwrap();
    assert_eq!(decode_saga_request(&bytes, LimitsV1::default()), Ok(value));
    assert_canonical_mutations(
        &bytes,
        |candidate| decode_saga_request(candidate, LimitsV1::default()),
        |decoded| encode_saga_request(decoded, LimitsV1::default()),
    );
    for end in 0..bytes.len() {
        assert!(decode_saga_request(&bytes[..end], LimitsV1::default()).is_err());
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert_eq!(
        decode_saga_request(&trailing, LimitsV1::default()),
        Err(CodecErrorV1::Malformed)
    );
}

fn round_reply(value: SagaReplyV1) {
    let bytes = encode_saga_reply(&value, LimitsV1::default()).unwrap();
    assert_eq!(decode_saga_reply(&bytes, LimitsV1::default()), Ok(value));
    assert_canonical_mutations(
        &bytes,
        |candidate| decode_saga_reply(candidate, LimitsV1::default()),
        |decoded| encode_saga_reply(decoded, LimitsV1::default()),
    );
}

fn view() -> SagaViewV1 {
    SagaViewV1 {
        saga: SagaIdV1(1),
        phase: SagaPhaseV1::FencedUncertain,
        deadline: 12,
        timer: TimerIdV1(2),
        current_effect: Some(SagaEffectIdV1(3)),
        current_kind: Some(SagaEffectKindV1::Forward),
        retries_used: 1,
        retry_budget: 2,
    }
}

fn operator_receipt() -> OperatorReceiptSummaryV1 {
    OperatorReceiptSummaryV1 {
        action: OperatorActionIdV1([9; 16]),
        saga: SagaIdV1(1),
        authority_decision: 8,
        authority_epoch: 7,
        disposition: OperatorDispositionV1::Retry {
            failed_effect: SagaEffectIdV1(3),
            now: 13,
        },
        phase: SagaPhaseV1::ForwardPending,
        prepared: Some(SagaEffectIdV1(4)),
    }
}

#[test]
fn every_saga_command_and_closed_mutation_roundtrips() {
    let saga = SagaIdV1(1);
    let effect = SagaEffectIdV1(2);
    for mutation in [
        SagaMutationV1::Admit {
            source: SagaSourceV1([1; 16]),
            now: 10,
            deadline: 20,
            retry_budget: 2,
        },
        SagaMutationV1::AcceptEffect {
            saga,
            effect,
            outcome: SagaEffectOutcomeV1::Uncertain,
            observed_at: 11,
        },
        SagaMutationV1::PrepareTerminal { saga, now: 12 },
        SagaMutationV1::Retry {
            saga,
            failed_effect: effect,
            now: 13,
        },
        SagaMutationV1::Cancel { saga, now: 14 },
        SagaMutationV1::FireDeadline {
            saga,
            timer: TimerIdV1(3),
            observed_at: 20,
            action: DeadlineActionV1::Compensate,
        },
    ] {
        round_request(SagaCommandV1::Apply {
            expected_revision: 4,
            mutation,
        });
    }
    for disposition in [
        OperatorDispositionV1::AdmitEvidence {
            effect,
            outcome: SagaEffectOutcomeV1::Failed,
            observed_at: 21,
        },
        OperatorDispositionV1::KeepUncertain,
        OperatorDispositionV1::Retry {
            failed_effect: effect,
            now: 22,
        },
        OperatorDispositionV1::Compensate { now: 23 },
        OperatorDispositionV1::Cancel { now: 24 },
        OperatorDispositionV1::FailTerminal,
    ] {
        round_request(SagaCommandV1::Reconcile {
            expected_revision: 5,
            operation: OperatorOperationV1 {
                action: OperatorActionIdV1([5; 16]),
                saga,
                disposition,
            },
        });
        round_request(SagaCommandV1::ReconciliationConfirmation {
            proposal: ReconciliationProposalV1 { saga, disposition },
        });
        round_request(SagaCommandV1::ConfirmReconciliation(
            ConfirmedReconciliationV1 {
                storage_revision: 5,
                saga,
                disposition,
                action: OperatorActionIdV1([7; 16]),
                confirmation_digest: [8; 32],
            },
        ));
    }
    for command in [
        SagaCommandV1::Status,
        SagaCommandV1::Saga { saga },
        SagaCommandV1::Inspect {
            after: Some(saga),
            limit: 2,
        },
        SagaCommandV1::Census,
        SagaCommandV1::Reconciliation { saga },
        SagaCommandV1::Evidence {
            saga,
            after: Some(effect),
            limit: 2,
        },
        SagaCommandV1::OperatorReceipt {
            action: OperatorActionIdV1([6; 16]),
        },
        SagaCommandV1::OperatorReceipts {
            after: Some(OperatorActionIdV1([7; 16])),
            limit: 2,
        },
    ] {
        round_request(command);
    }
}

#[test]
fn every_saga_outcome_transition_and_refusal_roundtrips() {
    let saga = SagaIdV1(1);
    let effect = SagaEffectIdV1(2);
    let timer = TimerIdV1(3);
    for transition in [
        SagaTransitionV1::Admitted {
            saga,
            effect,
            timer,
        },
        SagaTransitionV1::EffectPrepared {
            saga,
            effect,
            kind: SagaEffectKindV1::Compensation,
            retry: 1,
        },
        SagaTransitionV1::EffectAccepted {
            saga,
            effect,
            outcome: SagaEffectOutcomeV1::Succeeded,
            phase: SagaPhaseV1::ForwardSucceeded,
        },
        SagaTransitionV1::DeadlineApplied {
            saga,
            timer,
            action: DeadlineActionV1::KeepUncertain,
            phase: SagaPhaseV1::FencedUncertain,
            compensation: Some(effect),
        },
        SagaTransitionV1::Cancelled {
            saga,
            compensation: None,
        },
        SagaTransitionV1::Reconciled {
            saga,
            action: OperatorActionIdV1([4; 16]),
            phase: SagaPhaseV1::Failed,
            prepared: None,
        },
    ] {
        round_reply(SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            storage_revision: 6,
            transition,
            replayed: false,
        }));
    }
    for outcome in [
        SagaOutcomeV1::Status(SagaStatusV1 {
            revision: 7,
            needs_reopen: true,
        }),
        SagaOutcomeV1::Saga(Some(view())),
        SagaOutcomeV1::Page(SagaPageV1 {
            entries: vec![view()],
            next_after: Some(saga),
        }),
        SagaOutcomeV1::Census(SagaCensusV1 {
            total: 1,
            runnable_effects: 0,
            awaiting_decision: 0,
            fenced: 1,
            terminal: 0,
            retries_consumed: 1,
            saturated: false,
        }),
        SagaOutcomeV1::Reconciliation(Some(SagaReconciliationViewV1 {
            saga: view(),
            last_confirmed_revision: 7,
            retained_evidence: 1,
            fence: Some(FenceCauseV1::EffectUncertain {
                effect,
                kind: SagaEffectKindV1::Forward,
                observed_at: 11,
            }),
            automatic_progress: AutomaticProgressV1::BlockedUncertain,
        })),
        SagaOutcomeV1::ReconciliationConfirmation(Some(ReconciliationConfirmationV1 {
            storage_revision: 7,
            saga,
            disposition: OperatorDispositionV1::KeepUncertain,
            action: OperatorActionIdV1([8; 16]),
            confirmation_digest: [9; 32],
        })),
        SagaOutcomeV1::Evidence(EffectEvidencePageV1 {
            entries: vec![EffectEvidenceV1 {
                effect,
                outcome: SagaEffectOutcomeV1::Uncertain,
                observed_at: 11,
            }],
            next_after: Some(effect),
        }),
        SagaOutcomeV1::OperatorReceipt(Some(operator_receipt())),
        SagaOutcomeV1::OperatorReceipts(OperatorReceiptPageV1 {
            entries: vec![operator_receipt()],
            next_after: Some(OperatorActionIdV1([9; 16])),
        }),
    ] {
        round_reply(SagaReplyV1::Accepted(outcome));
    }

    for refusal in [
        SagaRefusalV1::NotAuthorized,
        SagaRefusalV1::InvalidLimits,
        SagaRefusalV1::InvalidPageLimit,
        SagaRefusalV1::RevisionConflict {
            expected: 1,
            actual: 2,
        },
        SagaRefusalV1::RevisionExhausted,
        SagaRefusalV1::IdentityExhausted,
        SagaRefusalV1::SagaLimit,
        SagaRefusalV1::InvalidDeadline,
        SagaRefusalV1::RetryBudget,
        SagaRefusalV1::InputConflict,
        SagaRefusalV1::UnknownSaga,
        SagaRefusalV1::InvalidPhase,
        SagaRefusalV1::EffectMismatch,
        SagaRefusalV1::EvidenceConflict,
        SagaRefusalV1::DeadlineNotReached,
        SagaRefusalV1::DeadlineReached,
        SagaRefusalV1::TimerMismatch,
        SagaRefusalV1::TimerConflict,
        SagaRefusalV1::LateEvidence,
        SagaRefusalV1::InvalidDeadlineAction,
        SagaRefusalV1::InvalidCheckpoint,
        SagaRefusalV1::OperatorActionLimit,
        SagaRefusalV1::OperatorActionConflict,
    ] {
        round_reply(SagaReplyV1::Refused(refusal));
    }
}

#[test]
fn saga_and_legacy_frame_families_are_not_interchangeable() {
    let saga = encode_saga_request(&SagaCommandV1::Status, LimitsV1::default()).unwrap();
    assert!(moira_wire::decode_request(&saga, LimitsV1::default()).is_err());
    let legacy = moira_wire::encode_request(&CommandV1::Status, LimitsV1::default()).unwrap();
    assert!(decode_saga_request(&legacy, LimitsV1::default()).is_err());
}

#[test]
fn declared_page_count_cannot_force_unbounded_allocation() {
    let reply = SagaReplyV1::Accepted(SagaOutcomeV1::Page(SagaPageV1 {
        entries: Vec::new(),
        next_after: None,
    }));
    let mut bytes = encode_saga_reply(&reply, LimitsV1::default()).unwrap();
    // magic + kind + accepted + page; overwrite the entry count.
    bytes[11..15].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        decode_saga_reply(&bytes, LimitsV1::default()),
        Err(CodecErrorV1::Limit)
    );
}
