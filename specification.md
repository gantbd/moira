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

1. Observe one bounded Agora message fact without changing workflow state.
2. Explicitly admit its message reference and one host-mapped participant recipient.
3. Prepare exactly one Chronos task-submission intent with a stable idempotency key.
4. Accept only a receipt attributable to that intent.
5. Record one Daimon execution result associated with the leased Chronos attempt.
6. Prepare either a terminal failure response or one Agora reply-publication intent.
7. Accept one attributable publication receipt and complete the workflow.

Recovery resumes from retained decisions. It does not infer completion from an
empty queue, missing process, elapsed timeout, or connector restart.

The qualified local composition covers steps 1–7. The deterministic engine covers
steps 2–7 in volatile memory; step 1 belongs to the durable Agora receiver. The
engine owns a bounded workflow map, one optimistic revision, stable opaque
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
transition identities. The bounded V1 canonical byte codec and the separate
SQLite command adapter now cover local coordinator-state persistence, with actual
reopen and process-loss tests. The local fake-provider peer mappings and restart
path are now implemented; broader production-provider crash qualification remains
separate. The Chronos admission receiver
now covers the first peer boundary: one task checkpoint and its canonical
intent/original receipt commit together, and matching lost-reply replay reads that
receipt without issuing another Chronos admission command. Targeted claims now
commit the native Chronos checkpoint and the exact attempt receipt together, keeping
lease tokens private. Host-asserted workers receive finite volatile Chronos grants;
recovery does not revive them. Recovered running attempts become uncertain and that
fence commits before the receiver exposes observations. Claim replay returns only
the original attribution/current phase, not a fresh execution instruction. The
admission-only schema upgrades atomically with its receipts intact. Together with
the result/publication receivers below, this covers the local fake-provider path through separate durable
receivers. Production provider and remote-service conformance remain separate.

The durable Agora receiver's pending-message projection is outside the Moira
engine transition. It accepts an `EventSequence`, resolves only a retained event
from that receiver's own storage scope, and returns an opaque
`AgoraMessageRefV1` plus author and canonical addressed participant IDs. It does
not expose the body, inspect participant kind, acknowledge the event, or construct
`AdmitMessage`. The host owns participant-reference mapping and the explicit
admission decision. Matching admission after a lost SQLite reply replays the
original workflow/submission identity; a changed recipient mapping conflicts.

Chronos result settlement now consumes only receiver-sealed fake-Daimon facts,
explicit host action and source-owner correspondence. Completion, terminal failure,
cancellation and uncertainty use Chronos's native lease, exact-reference or recovered
reconciliation paths; no lease is revived and no retry is selected. Timeout policy
is an explicit input. The native checkpoint, settlement evidence and count census
commit together; replay observes the retained original settlement revision. V1/V2
stores upgrade atomically to V3. Missing settlement records refuse recovery. Bounded
contradictory facts persist a receiver-wide conflict fence rather than changing a
previously settled task to a convenient outcome. Conflict clearance and late-evidence
promotion are not automatic in this profile. Counters, optional transaction timing
and conflict status distinguish these operations from claim/admission activity.

The separate fake-Daimon receiver now commits a canonical bounded request/simulated
reply and dispatch marker before invoking Daimon's actual fake dispatch. It commits
the attributed result before returning it. Restart with only a marker persists
`RecoveryUnknown`; this does not assert a provider receipt or completed execution,
and cannot trigger automatic redispatch. Completed results replay exactly, while
changed canonical inputs and execution/key aliasing conflict. The adopted live
Aegis gate requires Execute on the profile and every tool for both execution and
retained delivery. New execution additionally verifies the exact live Chronos
attempt; old grants/leases are not persisted. These are fake-provider semantics,
not production provider, sandbox or durable Aegis recovery guarantees.

The Agora receiver now atomically persists its native checkpoint and stable
source/room correspondence. Native replay receipts, messages and the event outbox
are one committed semantic image; matching lost-reply replay does not create a new
message. Outbox acknowledgements are persisted without inferring delivery truth.
The four-store scenario reaches `Completed` after restart/lost result and reply
acknowledgements, with one task, one fake execution and one response. An uncertain
harness result instead retains `FencedUncertain` and publishes no response. The
durability boundary remains the documented single-owner local SQLite profile.

Recovery also verifies the current V1 allocation census: the message index is a
bijection; workflow/effect identities have the allocator's exact form and complete
non-reused prefixes; effect order within a workflow is monotone; and the global
revision equals the accepted-transition census with distinct current snapshot
revisions. Retained task/attempt/result/publication receipts must agree with their
original local intents. Uncertain harness results cannot acquire a publication
intent through recovery. These checks reject inconsistent semantic snapshots;
they do not authenticate a checkpoint, establish
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
prevent a stale host from dispatching an external effect. `moira-sqlite` now uses
schema/application version checks, persistent logical owner identity, a cooperative
exclusive sidecar lock, single-link regular-file refusal rules and resource-identity
checks. It commits the staged semantic checkpoint before releasing a newly accepted
transition, marks replay explicitly, and fences uncertain commit outcomes until
reopen. Rollback-journal `DELETE`, `synchronous=EXTRA` and `fullfsync` are requested.
The [README](README.md#local-persistence-profile) states the bounded local profile
and tested fault boundaries. The coordinator store dispatches no external effects;
the separate Chronos receiver implements its admission mapping only. Physical
power-loss survival is not certified.

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

