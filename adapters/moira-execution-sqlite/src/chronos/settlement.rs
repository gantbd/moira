//! Host settlement intent plus receiver-sealed result facts. No retry defaults;
//! this stays coupled to the local Chronos/Daimon execution projection.
use super::*;
use crate::daimon::SettlementEvidence;
use chronos_engine::{EffectKnowledge, FailureDisposition, Task};
use moira_core::DaimonDispositionV1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettlementAction {
    Complete,
    FailTerminal,
    Cancel,
    KeepUncertain,
}
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SettlementContext {
    /// Host-established correspondence to the dedicated result receiver owner.
    pub source_owner: [u8; 16],
    pub now: SystemTime,
}
impl std::fmt::Debug for SettlementContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SettlementContext(<redacted>)")
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settlement {
    pub revision: StateRevision,
    pub phase: TaskPhase,
    pub replayed: bool,
}

fn tag(action: SettlementAction) -> u8 {
    match action {
        SettlementAction::Complete => 0,
        SettlementAction::FailTerminal => 1,
        SettlementAction::Cancel => 2,
        SettlementAction::KeepUncertain => 3,
    }
}
fn phase(action: SettlementAction) -> TaskPhase {
    match action {
        SettlementAction::Complete => TaskPhase::Succeeded,
        SettlementAction::FailTerminal => TaskPhase::Failed,
        SettlementAction::Cancel => TaskPhase::Cancelled,
        SettlementAction::KeepUncertain => TaskPhase::Uncertain,
    }
}
fn legal(disposition: u8, action: SettlementAction) -> bool {
    matches!(
        (disposition, action),
        (0, SettlementAction::Complete)
            | (1, SettlementAction::FailTerminal)
            | (2, SettlementAction::Cancel)
            | (
                3,
                SettlementAction::FailTerminal | SettlementAction::KeepUncertain
            )
            | (4, SettlementAction::KeepUncertain)
    )
}
fn encode(e: &SettlementEvidence, action: SettlementAction) -> Result<[u8; 88], ReceiverError> {
    let disposition = match e.disposition {
        DaimonDispositionV1::Completed => 0,
        DaimonDispositionV1::Failed => 1,
        DaimonDispositionV1::Cancelled => 2,
        DaimonDispositionV1::TimedOut => 3,
        DaimonDispositionV1::Uncertain => 4,
    };
    if !legal(disposition, action)
        || (e.recovery_unknown && disposition != 4)
        || e.attempt.attempt_number() == 0
    {
        return Err(ReceiverError::SettlementRefused);
    }
    let mut out = [0; 88];
    out[..8].copy_from_slice(b"MCHRSET\0");
    out[8] = 1;
    out[9..25].copy_from_slice(&e.source_owner);
    out[25..41].copy_from_slice(&e.effect.into_bytes());
    out[41..57].copy_from_slice(&e.execution);
    out[57..73].copy_from_slice(&e.attempt.task_key());
    out[73..81].copy_from_slice(&e.attempt.task_incarnation().to_be_bytes());
    out[81..85].copy_from_slice(&e.attempt.attempt_number().to_be_bytes());
    out[85] = disposition;
    out[86] = u8::from(e.recovery_unknown);
    out[87] = tag(action);
    Ok(out)
}
struct Decoded {
    bytes: [u8; 88],
    action: SettlementAction,
}
fn decode(bytes: &[u8]) -> Result<Decoded, ReceiverError> {
    let bytes: [u8; 88] = bytes.try_into().map_err(|_| ReceiverError::InvalidState)?;
    if &bytes[..8] != b"MCHRSET\0"
        || bytes[8] != 1
        || bytes[86] > 1
        || (bytes[86] == 1 && bytes[85] != 4)
        || bytes[81..85] == [0; 4]
    {
        return Err(ReceiverError::InvalidState);
    }
    let action = match bytes[87] {
        0 => SettlementAction::Complete,
        1 => SettlementAction::FailTerminal,
        2 => SettlementAction::Cancel,
        3 => SettlementAction::KeepUncertain,
        _ => return Err(ReceiverError::InvalidState),
    };
    if !legal(bytes[85], action) {
        return Err(ReceiverError::InvalidState);
    }
    Ok(Decoded { bytes, action })
}
fn contradicts(first: &Decoded, next: &Decoded) -> bool {
    first.bytes[..85] != next.bytes[..85]
        || (first.bytes[85] != next.bytes[85]
            && first.action != SettlementAction::KeepUncertain
            && next.action != SettlementAction::KeepUncertain)
}

impl ChronosReceiver {
    pub fn settlement_usage(&self) -> Usage {
        self.settlement_usage
    }
    pub fn has_evidence_conflict(&self) -> bool {
        self.evidence_conflict
    }
    /// Privileged host operation. Evidence is a fact, not authority: the receiver
    /// must own the live lease or the exact recovered reconciliation path.
    /// Matching replay observes the retained outcome before checking the guard.
    pub fn settle(
        &mut self,
        submission: EffectId,
        expected: StateRevision,
        evidence: &SettlementEvidence,
        action: SettlementAction,
        context: SettlementContext,
    ) -> Result<Settlement, ReceiverError> {
        let result = self.settle_inner(submission, expected, evidence, action, context);
        let u = &mut self.settlement_usage;
        count(&mut u.command_offers, &mut u.overflowed);
        match &result {
            Ok(s) if s.replayed => count(&mut u.replayed, &mut u.overflowed),
            Ok(_) => count(&mut u.committed, &mut u.overflowed),
            Err(ReceiverError::Storage(StoreError::Indeterminate)) => {
                count(&mut u.indeterminate, &mut u.overflowed)
            }
            Err(e) => {
                count(&mut u.refused, &mut u.overflowed);
                if matches!(
                    e,
                    ReceiverError::SettlementConflict | ReceiverError::EvidenceConflict
                ) {
                    count(&mut u.conflicts, &mut u.overflowed)
                }
            }
        }
        result
    }
    fn settle_inner(
        &mut self,
        submission: EffectId,
        expected: StateRevision,
        e: &SettlementEvidence,
        action: SettlementAction,
        context: SettlementContext,
    ) -> Result<Settlement, ReceiverError> {
        self.guard()?;
        if e.source_owner != context.source_owner || e.attempt.task_key() != submission.into_bytes()
        {
            return Err(ReceiverError::SettlementRefused);
        }
        let canonical = encode(e, action)?;
        let binding:Option<(String,Vec<u8>,u32)>=self.connection.query_row("SELECT r.task_id,r.incarnation,a.number FROM receiver_receipts r JOIN receiver_attempts a ON r.effect=a.effect WHERE r.effect=?1",[submission.into_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().inspect_err(|_|self.healthy=false)?;
        let (id, incarnation, attempt) = binding.ok_or(ReceiverError::SettlementRefused)?;
        if number(&incarnation)? != e.attempt.task_incarnation()
            || attempt != e.attempt.attempt_number()
        {
            return Err(ReceiverError::SettlementRefused);
        }
        let retained: Option<(Vec<u8>, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT evidence,revision FROM receiver_settlements WHERE effect=?1",
                [submission.into_bytes().as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .inspect_err(|_| self.healthy = false)?;
        if let Some((old, revision)) = retained {
            let old = decode(&old).inspect_err(|_| self.healthy = false)?;
            if old.bytes == canonical {
                return Ok(Settlement {
                    revision: StateRevision::finite(number(&revision)?),
                    phase: phase(old.action),
                    replayed: true,
                });
            }
            if contradicts(&old, &decode(&canonical)?) {
                self.retain_conflict(submission, &canonical)?;
                return Err(ReceiverError::SettlementConflict);
            }
            // V1 has no implicit policy replacement or late-evidence promotion.
            // Unknown stays fenced; a weaker report cannot undo a known result.
            return Err(ReceiverError::ReconciliationRequired);
        }
        if self.engine.task_state_revision() != expected {
            return Err(ReceiverError::SettlementRefused);
        }
        let id = chronos_engine::TaskId::new(id);
        let task = self.engine.task(&id).ok_or(ReceiverError::InvalidState)?;
        if task.reference().map(|r| r.incarnation().get()) != Some(e.attempt.task_incarnation())
            || task.attempts != attempt
        {
            return Err(ReceiverError::SettlementRefused);
        }
        let reference = task.reference().ok_or(ReceiverError::InvalidState)?;
        let state = task.state.clone();
        let time = self
            .engine
            .advance_time(context.now)
            .map_err(|_| ReceiverError::SettlementRefused)?;
        match (state, action) {
            (TaskState::Running(claim), SettlementAction::Complete) => self
                .engine
                .complete_at_time(&id, &claim.lease, &time)
                .map_err(|_| ReceiverError::SettlementRefused)?,
            (TaskState::Running(claim), SettlementAction::FailTerminal) => self
                .engine
                .fail_at_time(
                    &id,
                    &claim.lease,
                    "execution",
                    FailureDisposition::Terminal,
                    &time,
                )
                .map_err(|_| ReceiverError::SettlementRefused)?,
            (TaskState::Running(claim), SettlementAction::KeepUncertain) => self
                .engine
                .fence_uncertain_at_time(&id, &claim.lease, &time)
                .map_err(|_| ReceiverError::SettlementRefused)?,
            (TaskState::Running(_) | TaskState::Uncertain { .. }, SettlementAction::Cancel) => self
                .engine
                .cancel_ref_at_revision(expected, &reference)
                .map_err(|_| ReceiverError::SettlementRefused)?,
            (TaskState::Uncertain { attempt_number, .. }, _) if attempt_number == attempt => {
                match action {
                    SettlementAction::Complete => self
                        .engine
                        .resolve_uncertain_ref_at_revision(
                            expected,
                            &reference,
                            EffectKnowledge::Completed,
                            "",
                            FailureDisposition::Terminal,
                        )
                        .map_err(|_| ReceiverError::SettlementRefused)?,
                    SettlementAction::FailTerminal => self
                        .engine
                        .resolve_uncertain_ref_at_revision(
                            expected,
                            &reference,
                            EffectKnowledge::Failed,
                            "execution",
                            FailureDisposition::Terminal,
                        )
                        .map_err(|_| ReceiverError::SettlementRefused)?,
                    SettlementAction::KeepUncertain => {}
                    SettlementAction::Cancel => unreachable!("handled above"),
                }
            }
            _ => return Err(ReceiverError::SettlementRefused),
        }
        self.healthy = false;
        let StateRevision::Finite(revision) = self.engine.task_state_revision() else {
            return Err(ReceiverError::InvalidState);
        };
        let cp = self
            .engine
            .prepare_checkpoint(self.limits.max_snapshot_bytes)
            .map_err(|_| ReceiverError::Checkpoint)?;
        self.commit(
            &cp,
            PendingWrite::Settlement {
                effect: submission,
                evidence: &canonical,
                revision,
                fenced: action == SettlementAction::KeepUncertain,
            },
        )?;
        Ok(Settlement {
            revision: StateRevision::finite(revision),
            phase: phase(action),
            replayed: false,
        })
    }
    fn retain_conflict(
        &mut self,
        submission: EffectId,
        canonical: &[u8; 88],
    ) -> Result<(), ReceiverError> {
        self.healthy = false;
        let started = match self.limits.timing {
            Timing::Off => None,
            Timing::Process => std::panic::catch_unwind(Instant::now).ok(),
        };
        let result = (|| -> Result<(), ReceiverError> {
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let actual: Vec<u8> =
                tx.query_row("SELECT revision FROM receiver_meta WHERE slot=1", [], |r| {
                    r.get(0)
                })?;
            if number(&actual)? != self.engine.snapshot_revision().get() {
                return Err(ReceiverError::InvalidState);
            }
            let updated=tx.execute("UPDATE receiver_settlements SET conflicting=?1 WHERE effect=?2 AND conflicting IS NULL",params![canonical.as_slice(),submission.into_bytes().as_slice()])?;
            if updated != 1 {
                return Err(ReceiverError::InvalidState);
            }
            tx.commit().map_err(|_| StoreError::Indeterminate)?;
            self.resource
                .check()
                .map_err(|_| StoreError::Indeterminate)?;
            Ok(())
        })();
        record_timing(&mut self.settlement_usage, started);
        result?;
        self.healthy = true;
        self.evidence_conflict = true;
        Ok(())
    }
}

struct StoredSettlement {
    canonical: Vec<u8>,
    revision: Vec<u8>,
    conflicting: Option<Vec<u8>>,
}
pub(super) fn validate_retained(
    c: &Connection,
    submission: &[u8],
    task: &Task,
    attempt: u32,
    claim_revision: u64,
    current: StateRevision,
) -> Result<bool, ReceiverError> {
    let stored: Option<StoredSettlement> = c
        .query_row(
            "SELECT evidence,revision,conflicting FROM receiver_settlements WHERE effect=?1",
            [submission],
            |r| {
                Ok(StoredSettlement {
                    canonical: r.get(0)?,
                    revision: r.get(1)?,
                    conflicting: r.get(2)?,
                })
            },
        )
        .optional()?;
    let Some(StoredSettlement {
        canonical,
        revision,
        conflicting,
    }) = stored
    else {
        return Ok(false);
    };
    let d = decode(&canonical)?;
    let revision = number(&revision)?;
    if d.bytes[57..73] != *submission
        || d.bytes[73..81]
            != task
                .reference()
                .ok_or(ReceiverError::InvalidState)?
                .incarnation()
                .get()
                .to_be_bytes()
        || d.bytes[81..85] != attempt.to_be_bytes()
        || revision < claim_revision
        || StateRevision::finite(revision) > current
    {
        return Err(ReceiverError::InvalidState);
    }
    let consistent = matches!(
        (&task.state, phase(d.action)),
        (TaskState::Succeeded, TaskPhase::Succeeded)
            | (TaskState::Failed { .. }, TaskPhase::Failed)
            | (TaskState::Cancelled, TaskPhase::Cancelled)
            | (TaskState::Uncertain { .. }, TaskPhase::Uncertain)
    );
    if !consistent {
        return Err(ReceiverError::InvalidState);
    }
    if let Some(other) = conflicting {
        let next = decode(&other)?;
        if next.bytes[57..85] != d.bytes[57..85] || !contradicts(&d, &next) {
            return Err(ReceiverError::InvalidState);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::super::tests::Fixture;
    use super::*;
    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(10)
    }
    fn context() -> SettlementContext {
        SettlementContext {
            source_owner: [7; 16],
            now: now(),
        }
    }
    fn setup() -> (Fixture, ChronosReceiver, SettlementEvidence) {
        let f = Fixture::new();
        let mut c = f.open();
        let submission = EffectId::new([1; 16]);
        c.admit(submission, StateRevision::ZERO, TaskSpecV2::new("work"))
            .unwrap();
        let grant = c
            .authorize_worker(Agent::new("worker"), now(), Duration::from_secs(60), 1)
            .unwrap();
        c.claim(
            submission,
            c.revision(),
            &grant,
            now(),
            Duration::from_secs(30),
        )
        .unwrap();
        // Negative/fault fixture; real receiver sealing is covered by the
        // cross-receiver integration scenarios, not this test-only constructor.
        let e = SettlementEvidence {
            source_owner: [7; 16],
            effect: EffectId::new([2; 16]),
            execution: [2; 16],
            attempt: daimon_harness::TaskAttemptRefV1::new([1; 16], 0, 1),
            disposition: DaimonDispositionV1::Completed,
            recovery_unknown: false,
        };
        (f, c, e)
    }
    #[test]
    fn settlement_faults_commit_both_state_and_receipt_or_neither() {
        for fault in [crate::Fault::BeforeCommit, crate::Fault::AfterCommit] {
            let (f, mut c, e) = setup();
            let before = c.revision();
            c.fault = Some(fault);
            assert!(
                c.settle(
                    EffectId::new([1; 16]),
                    before,
                    &e,
                    SettlementAction::Complete,
                    context()
                )
                .is_err()
            );
            assert!(c.needs_reopen());
            assert_eq!(c.revision(), before);
            assert_eq!(c.settlement_usage().committed, 0);
            drop(c);
            let mut c = f.open();
            let result = c
                .settle(
                    EffectId::new([1; 16]),
                    c.revision(),
                    &e,
                    SettlementAction::Complete,
                    context(),
                )
                .unwrap();
            assert_eq!(result.phase, TaskPhase::Succeeded);
            assert_eq!(result.replayed, fault == crate::Fault::AfterCommit);
            assert_eq!(
                c.settlement_usage().commit_timed_samples,
                if result.replayed { 0 } else { 1 }
            );
        }
    }
    #[test]
    fn contradictions_persist_a_receiver_fence_across_reopen() {
        let (f, mut c, mut e) = setup();
        let submission = EffectId::new([1; 16]);
        c.settle(
            submission,
            c.revision(),
            &e,
            SettlementAction::Complete,
            context(),
        )
        .unwrap();
        e.disposition = DaimonDispositionV1::Failed;
        assert!(matches!(
            c.settle(
                submission,
                c.revision(),
                &e,
                SettlementAction::FailTerminal,
                context()
            ),
            Err(ReceiverError::SettlementConflict)
        ));
        assert!(c.has_evidence_conflict());
        drop(c);
        let mut c = f.open();
        assert!(c.has_evidence_conflict());
        assert!(matches!(
            c.admit(
                EffectId::new([3; 16]),
                c.revision(),
                TaskSpecV2::new("other")
            ),
            Err(ReceiverError::EvidenceConflict)
        ));
    }
    #[test]
    fn source_guard_and_action_refusals_do_not_mutate_or_claim_again() {
        let (f, mut c, e) = setup();
        let submission = EffectId::new([1; 16]);
        let before = c.revision();
        assert!(matches!(
            c.settle(
                submission,
                before,
                &e,
                SettlementAction::Complete,
                SettlementContext {
                    source_owner: [9; 16],
                    now: now()
                }
            ),
            Err(ReceiverError::SettlementRefused)
        ));
        assert!(matches!(
            c.settle(
                submission,
                StateRevision::ZERO,
                &e,
                SettlementAction::Complete,
                context()
            ),
            Err(ReceiverError::SettlementRefused)
        ));
        assert!(matches!(
            c.settle(
                submission,
                before,
                &e,
                SettlementAction::FailTerminal,
                context()
            ),
            Err(ReceiverError::SettlementRefused)
        ));
        assert_eq!(c.revision(), before);
        assert!(!c.needs_reopen());
        drop(c);
        let mut c = f.open();
        assert!(
            !c.settle(
                submission,
                c.revision(),
                &e,
                SettlementAction::Complete,
                context()
            )
            .unwrap()
            .replayed
        );
    }
    #[test]
    fn a_missing_uncertainty_receipt_is_not_an_unsettled_attempt() {
        let (f, mut c, mut e) = setup();
        e.disposition = DaimonDispositionV1::Uncertain;
        c.settle(
            EffectId::new([1; 16]),
            c.revision(),
            &e,
            SettlementAction::KeepUncertain,
            context(),
        )
        .unwrap();
        c.connection
            .execute("DELETE FROM receiver_settlements", [])
            .unwrap();
        drop(c);
        assert!(matches!(
            ChronosReceiver::open(f.path(), [7; 16], super::super::tests::limits()),
            Err(ReceiverError::InvalidState)
        ));
    }
    #[test]
    fn v2_running_attempt_migrates_without_reviving_lease_or_losing_attribution() {
        let (f, c, e) = setup();
        c.connection.execute_batch("DROP TABLE receiver_settlements; ALTER TABLE receiver_meta DROP COLUMN settlements; PRAGMA user_version=2;").unwrap();
        drop(c);
        let mut c = f.open();
        assert_eq!(c.recovery_usage().newly_fenced_attempts, 1);
        assert_eq!(
            c.connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        let result = c
            .settle(
                EffectId::new([1; 16]),
                c.revision(),
                &e,
                SettlementAction::Complete,
                context(),
            )
            .unwrap();
        assert!(!result.replayed);
        assert_eq!(result.phase, TaskPhase::Succeeded);
    }
    #[test]
    fn expired_lease_cannot_settle_as_live_but_recovery_uses_retained_evidence() {
        let (f, mut c, e) = setup();
        let before = c.revision();
        let late = SettlementContext {
            source_owner: [7; 16],
            now: SystemTime::UNIX_EPOCH + Duration::from_secs(40),
        };
        assert!(matches!(
            c.settle(
                EffectId::new([1; 16]),
                before,
                &e,
                SettlementAction::Complete,
                late
            ),
            Err(ReceiverError::SettlementRefused)
        ));
        assert_eq!(c.revision(), before);
        drop(c);
        let mut c = f.open();
        assert_eq!(
            c.settle(
                EffectId::new([1; 16]),
                c.revision(),
                &e,
                SettlementAction::Complete,
                late
            )
            .unwrap()
            .phase,
            TaskPhase::Succeeded
        );
        assert_eq!(format!("{late:?}"), "SettlementContext(<redacted>)");
        assert_eq!(format!("{e:?}"), "SettlementEvidence(<redacted>)");
    }
}
