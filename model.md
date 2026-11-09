# Domain model

## Aggregate

A workflow is identified by `WorkflowId` and guarded by `WorkflowRevision`. It
contains a closed phase, the initiating cause, a bounded set of step records, and
at most the explicitly configured retained effect evidence.

## Initial phase vocabulary

```text
Received
  -> TaskSubmissionPending
  -> TaskSubmitted
  -> HarnessPending
  -> HarnessFinished
  -> ResponsePublicationPending
  -> Completed

Any effect boundary may enter FencedUncertain.
Policy may terminate the workflow as Failed or Cancelled.
```

This sequence is a first scenario, not a universal workflow language. General
DAG execution belongs to Chronos; arbitrary user-authored code is outside Moira.

## Effects

An effect intent is an immutable bounded value. A connector returns attributed
evidence tied to that exact intent and attempt. Timeout supplies no completion
knowledge. Contradictory evidence fences automatic progress.

