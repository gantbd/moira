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

The closed saga kernel refines this with explicit forward, terminal and
compensation effects; typed failed/pending/uncertain phases; caller-supplied
deadline/timer evidence; and finite explicit retry. Compensation is new work with
new causation, never state erasure. See `docs/saga-v1.md`.

An uncertainty fence has an inspectable effect- or deadline-origin. Operator
disposition is a separate Aegis-authorized transition family with bounded durable
causation and exact action replay. It may adjudicate named evidence, retain the
fence, allocate fresh retry/compensation work, cancel or fail; it cannot erase
evidence or assert generic success.

## Effects

An effect intent is an immutable bounded value. A connector returns attributed
evidence tied to that exact intent and attempt. Timeout supplies no completion
knowledge. Contradictory evidence fences automatic progress.

