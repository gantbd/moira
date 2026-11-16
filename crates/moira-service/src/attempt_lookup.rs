//! Receipt-backed correlation for hosts with an established Chronos binding.
use crate::{AccessV1, Service, disposition_view};
use moira_core::ChronosTaskRefV1;
use moira_host::AttemptReceiptStore;
pub use moira_protocol::attempt_lookup::{AttemptLookup, ExecutionEvidence};
use moira_protocol::{
    WorkflowIdV1,
    attempt_lookup::{MAX_SCOPE_BYTES, MAX_TASK_BYTES},
};

pub struct ExactAttempt<'a> {
    pub service_scope: &'a str,
    pub task: &'a str,
    pub incarnation: u64,
    pub attempt: u32,
}

/// The host resolves scope and name through its trusted deployment mapping.
/// The returned key must uniquely represent that scoped Chronos task in
/// Moira's receipt domain. A task name or key alone carries no authority.
pub trait AttemptLookupPolicy {
    fn allows_lookup(&mut self, attempt: &ExactAttempt<'_>) -> bool;
    fn bind_task(&mut self, attempt: &ExactAttempt<'_>) -> Option<ChronosTaskRefV1>;
    fn allows_result(&mut self, attempt: &ExactAttempt<'_>, workflow: WorkflowIdV1) -> bool;
}

impl<S: AttemptReceiptStore> Service<S> {
    pub fn lookup_attempt(
        &self,
        access: AccessV1,
        attempt: &ExactAttempt<'_>,
        policy: &mut impl AttemptLookupPolicy,
    ) -> AttemptLookup {
        if !access.read || !policy.allows_lookup(attempt) {
            return AttemptLookup::Denied;
        }
        if self.store.needs_reopen() {
            return AttemptLookup::Unavailable;
        }
        if attempt.attempt == 0
            || !(1..=MAX_TASK_BYTES).contains(&attempt.task.len())
            || !(1..=MAX_SCOPE_BYTES).contains(&attempt.service_scope.len())
        {
            return AttemptLookup::Unresolved;
        }
        let Some(task) = policy.bind_task(attempt) else {
            return AttemptLookup::Unresolved;
        };
        if task.incarnation() != attempt.incarnation {
            return AttemptLookup::Unresolved;
        }
        let Ok(Some(record)) = self.store.workflow_for_attempt(task, attempt.attempt) else {
            return AttemptLookup::Unresolved;
        };
        let workflow = WorkflowIdV1(record.workflow_id().into_bytes());
        if !policy.allows_result(attempt, workflow) {
            return AttemptLookup::Denied;
        }
        let execution = match record.daimon_result() {
            None => ExecutionEvidence::ReceiptPending,
            Some(receipt)
                if receipt.task() == task && receipt.attempt_number() == attempt.attempt =>
            {
                ExecutionEvidence::Recorded {
                    execution_id: receipt.execution_id(),
                    disposition: disposition_view(receipt.disposition()),
                }
            }
            Some(_) => return AttemptLookup::Unresolved,
        };
        AttemptLookup::Found {
            workflow,
            execution,
        }
    }
}
