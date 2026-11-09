# Initial specification

The intended semantic shape is:

```text
(WorkflowState, CommandV1, ExplicitEvidence) ->
    (WorkflowState, Result<TransitionV1, RefusalV1>)
```

Refusal is an identity state transition. Every accepted transition increments one
workflow revision and emits bounded typed intents/events after the domain result.
Matching replay returns the original semantic transition without incrementing the
revision; it is observation of a prior acceptance, not a new accepted transition.
Reuse of the same source/effect identity with different canonical content is a
typed conflict.

An exact retained Chronos admission receipt remains replayable after later
workflow phases, including an uncertainty fence or completion. Replay neither
changes the current phase nor schedules the original effect again; a conflicting
receipt remains a conflict. Connectors must use retained current state, not treat
an observed old transition as a fresh effect-dispatch instruction.

## First workflow contract

1. Admit one bounded Agora message reference and intended participant recipient.
2. Prepare exactly one Chronos task-submission intent with a stable idempotency key.
3. Accept only a receipt attributable to that intent.
4. Record one Daimon execution result associated with the leased Chronos attempt.
5. Prepare either a terminal failure response or one Agora reply-publication intent.
6. Accept one attributable publication receipt and complete the workflow.

Recovery resumes from retained decisions. It does not infer completion from an
empty queue, missing process, elapsed timeout, or connector restart.

The implemented reference slice currently covers steps 1–6 in volatile memory.
Its engine owns a bounded workflow map, one optimistic revision, stable opaque
workflow/effect identities, an admitted-message replay index, exact Chronos
admission and attempt attribution, and an exact Daimon result receipt. Matching
replay after either engine boundary returns the retained transition without
advancing revision. A Daimon `Uncertain` disposition enters `FencedUncertain`;
it is not translated into failure or completion. A known harness result prepares
one retained Agora publication effect, and an exact Agora receipt completes the
workflow. The Agora qualification replays a lost publication reply against
Agora's real keyed publication profile without creating a second message.
`CheckpointV1` captures the complete semantic state and `Engine::recover`
validates message-index, workflow-phase, bound, and revision invariants before
reconstruction. Behavior evidence reconstructs the engine at the Chronos,
Daimon, Agora-pending, and terminal boundaries and observes the same retained
transition identities. A canonical byte codec, store protocol, and proof of
atomic durable persistence remain open.

Recovery also verifies the current V1 allocation census: the message index is a
bijection; workflow/effect identities have the allocator's exact form and complete
non-reused prefixes; effect order within a workflow is monotone; and the global
revision equals the accepted-transition census with distinct current snapshot
revisions. Retained task/attempt/result/publication receipts must agree with their
original local intents. Uncertain harness results cannot acquire a publication
intent through recovery. These checks reject inconsistent semantic snapshots;
they do not authenticate a checkpoint, supply a persistent codec, establish
cross-owner identity scopes, or prove a filesystem durability claim.

## Adopted persistence direction

SQLite is selected for the first durable local slice, rather than revisioned
checkpoint files or a custom append-only journal. Persistence belongs to an
adapter outside the deterministic core. Related workflow decisions, pending
intents and connector receipt mappings must commit together where they share
the local transaction boundary. SQLite does not make external engine effects
part of that transaction or eliminate uncertain outcomes. The first slice uses
one active coordinator owner per persisted store, with dedicated Chronos/Agora
connector scopes. Logical owner identity and retained correlation mappings survive
restart; recovered identity does not revive leases or authorization. Dedicated
scopes do not require separate processes. Shared-peer multi-owner identity and
active-active workflow ownership are not part of this first slice. The host must
enforce exclusive active ownership; database write serialization alone does not
prevent a stale host from dispatching an external effect. Schema, durability
configuration, ownership enforcement and crash qualification remain to be completed;
this selection is not an implementation or durability claim.

## Optional instrumentation

`apply_observed` delegates to the same `apply` transition and records its result
afterward, without callbacks, clock reads or connector calls. An unchanged
revision on success is matching replay, not new acceptance. Aggregate counters
remain active when capture is disabled or full; bounded FIFO capture drops newest
events on overload and exposes loss separately. Harness receipt populations count
only new acceptance, not replay. `usage` reads current bounded state independently
of event delivery. Observation counters are process-local and are not checkpointed;
service time is explicitly unknown. Diagnostics are neither authority nor an audit
log, and do not establish external-effect truth or durable commitment.

