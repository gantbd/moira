//! Moira-owned exact server-challenge guard for saga reconciliation.
//!
//! Preparation is read-only. Confirmation submits exactly one server-prepared
//! document and never retries a transport failure, because the durable effect
//! may already have committed. Authority remains a daemon-side decision.
#![forbid(unsafe_code)]

use moira_client::{SagaConfirmedReconciliationClient, SagaReadOnlyClient, SagaServicePort};
use moira_protocol::{
    ConfirmedReconciliationV1, OperatorActionIdV1, OperatorReceiptSummaryV1,
    ReconciliationConfirmationV1, ReconciliationProposalV1, SagaIdV1, SagaOutcomeV1, SagaRefusalV1,
    SagaReplyV1,
};

pub const PREPARE_CAPABILITY: &str = "sagas.reconciliation.confirmation.v1";
pub const CONFIRM_CAPABILITY: &str = "sagas.reconciliation.confirm.v1";
pub const RECOVER_CAPABILITY: &str = "sagas.reconciliation.recovery.v1";

#[derive(Debug, Eq, PartialEq)]
pub enum PrepareError<E> {
    Transport(E),
    Refused(SagaRefusalV1),
    NotAvailable,
    InvalidReply,
}

pub fn prepare<P: SagaServicePort>(
    port: P,
    proposal: ReconciliationProposalV1,
) -> Result<ReconciliationConfirmationV1, PrepareError<P::Error>> {
    let mut client = SagaReadOnlyClient::new(port);
    match client.reconciliation_confirmation(proposal) {
        Ok(SagaReplyV1::Accepted(SagaOutcomeV1::ReconciliationConfirmation(Some(value))))
            if value.saga == proposal.saga && value.disposition == proposal.disposition =>
        {
            Ok(value)
        }
        Ok(SagaReplyV1::Accepted(SagaOutcomeV1::ReconciliationConfirmation(None))) => {
            Err(PrepareError::NotAvailable)
        }
        Ok(SagaReplyV1::Refused(refusal)) => Err(PrepareError::Refused(refusal)),
        Ok(_) => Err(PrepareError::InvalidReply),
        Err(error) => Err(PrepareError::Transport(error)),
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ConfirmError<E> {
    Transport(E),
    Refused(SagaRefusalV1),
    InvalidReply,
}

pub fn confirm<P: SagaServicePort>(
    port: P,
    confirmation: ConfirmedReconciliationV1,
) -> Result<SagaOutcomeV1, ConfirmError<P::Error>> {
    let mut client = SagaConfirmedReconciliationClient::new(port);
    match client.confirm(confirmation) {
        Ok(SagaReplyV1::Accepted(outcome @ SagaOutcomeV1::Applied { .. })) => Ok(outcome),
        Ok(SagaReplyV1::Refused(refusal)) => Err(ConfirmError::Refused(refusal)),
        Ok(_) => Err(ConfirmError::InvalidReply),
        Err(error) => Err(ConfirmError::Transport(error)),
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum RecoverError<E> {
    Transport(E),
    Refused(SagaRefusalV1),
    NotFound,
    Mismatch,
    InvalidReply,
}

/// Resolves an indeterminate confirmation only from the exact durable receipt.
pub fn recover<P: SagaServicePort>(
    port: P,
    saga: SagaIdV1,
    action: OperatorActionIdV1,
) -> Result<OperatorReceiptSummaryV1, RecoverError<P::Error>> {
    let mut client = SagaReadOnlyClient::new(port);
    match client.operator_receipt(action) {
        Ok(SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipt(Some(receipt))))
            if receipt.action == action && receipt.saga == saga =>
        {
            Ok(receipt)
        }
        Ok(SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipt(Some(_)))) => {
            Err(RecoverError::Mismatch)
        }
        Ok(SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipt(None))) => {
            Err(RecoverError::NotFound)
        }
        Ok(SagaReplyV1::Refused(refusal)) => Err(RecoverError::Refused(refusal)),
        Ok(_) => Err(RecoverError::InvalidReply),
        Err(error) => Err(RecoverError::Transport(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moira_protocol::{OperatorActionIdV1, OperatorDispositionV1, SagaCommandV1, SagaIdV1};
    use std::{cell::RefCell, rc::Rc};

    struct Port {
        commands: Rc<RefCell<Vec<SagaCommandV1>>>,
        reply: Option<Result<SagaReplyV1, &'static str>>,
    }

    impl SagaServicePort for Port {
        type Error = &'static str;

        fn execute(&mut self, command: SagaCommandV1) -> Result<SagaReplyV1, Self::Error> {
            self.commands.borrow_mut().push(command);
            self.reply.take().expect("one command")
        }
    }

    fn proposal() -> ReconciliationProposalV1 {
        ReconciliationProposalV1 {
            saga: SagaIdV1(1),
            disposition: OperatorDispositionV1::KeepUncertain,
        }
    }

    fn challenge() -> ReconciliationConfirmationV1 {
        ReconciliationConfirmationV1 {
            storage_revision: 2,
            saga: proposal().saga,
            disposition: proposal().disposition,
            action: OperatorActionIdV1([3; 16]),
            confirmation_digest: [4; 32],
        }
    }

    #[test]
    fn preparation_accepts_only_the_exact_server_challenge() {
        let commands = Rc::new(RefCell::new(Vec::new()));
        assert_eq!(
            prepare(
                Port {
                    commands: Rc::clone(&commands),
                    reply: Some(Ok(SagaReplyV1::Accepted(
                        SagaOutcomeV1::ReconciliationConfirmation(Some(challenge())),
                    ))),
                },
                proposal(),
            ),
            Ok(challenge())
        );
        assert_eq!(
            commands.borrow().as_slice(),
            &[SagaCommandV1::ReconciliationConfirmation {
                proposal: proposal()
            }]
        );
    }

    #[test]
    fn confirmation_submits_once_and_preserves_unknown_transport() {
        let commands = Rc::new(RefCell::new(Vec::new()));
        let confirmation = ConfirmedReconciliationV1 {
            storage_revision: challenge().storage_revision,
            saga: challenge().saga,
            disposition: challenge().disposition,
            action: challenge().action,
            confirmation_digest: challenge().confirmation_digest,
        };
        assert_eq!(
            confirm(
                Port {
                    commands: Rc::clone(&commands),
                    reply: Some(Err("lost reply")),
                },
                confirmation,
            ),
            Err(ConfirmError::Transport("lost reply"))
        );
        assert_eq!(commands.borrow().len(), 1);
        assert!(matches!(
            commands.borrow()[0],
            SagaCommandV1::ConfirmReconciliation(_)
        ));
    }

    #[test]
    fn recovery_accepts_only_the_exact_durable_receipt() {
        let commands = Rc::new(RefCell::new(Vec::new()));
        let receipt = OperatorReceiptSummaryV1 {
            action: challenge().action,
            saga: challenge().saga,
            authority_decision: 7,
            authority_epoch: 3,
            disposition: challenge().disposition,
            phase: moira_protocol::SagaPhaseV1::FencedUncertain,
            prepared: None,
        };
        assert_eq!(
            recover(
                Port {
                    commands: Rc::clone(&commands),
                    reply: Some(Ok(SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipt(
                        Some(receipt),
                    )))),
                },
                receipt.saga,
                receipt.action,
            ),
            Ok(receipt)
        );
        assert_eq!(
            commands.borrow().as_slice(),
            &[SagaCommandV1::OperatorReceipt {
                action: receipt.action
            }]
        );

        assert_eq!(
            recover(
                Port {
                    commands: Rc::new(RefCell::new(Vec::new())),
                    reply: Some(Ok(SagaReplyV1::Accepted(SagaOutcomeV1::OperatorReceipt(
                        Some(receipt),
                    )))),
                },
                SagaIdV1(receipt.saga.0 + 1),
                receipt.action,
            ),
            Err(RecoverError::Mismatch)
        );
    }
}
