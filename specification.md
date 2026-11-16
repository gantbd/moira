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

## Methodical deliberation profile

The methodical-agent profile is a separate Moira workflow family. Rune owns its
authored activity machine and SME/fold content; Dendrite supplies exact attributed
projections; Daimon executes bounded member work. Moira retains plans, independent
reports, dissent, frontier calculation, validator witnesses and the final
resolution. The closed resolution family is `ProvenBool`, `AuthorizedChoice`,
four-valued `Maybe`, or reasoned `Mu`. A Pareto fold only removes dominated
candidates. It cannot select a winner or turn preference into a proved boolean.

The initial deterministic slice is `moira_core::deliberation`: closed claim,
persona and resolution values, a strict Pareto-frontier function, bounded plans,
attributed ensemble reports with retained dissent, fuel accounting, resolution
validation and semantic checkpoint recovery. Admission refuses a negative
orthogonality verdict before correlated reports can masquerade as independent
agreement. The grounding assembler additionally requires fold, surface and
orthogonality evidence to cover the exact intended cohort and requires every
surface authority digest to match its fold grounding. It performs no I/O, grants no
authority and dispatches no effect. `MOIRADL2` provides a canonical bounded
checkpoint encoding with strict host limits, truncation/trailing-byte refusal and
semantic recovery validation. The codec revision binds the exact question and
the attributed authority subject/epoch; prototype `MOIRADL1` bytes are refused
rather than reinterpreted. `DeliberationRegistryV1` serializes multiple
deliberations under one optimistic revision; matching plan/report/resolution
replay is non-mutating, identity conflict and stale revision refuse before
mutation, and recovery checks the complete accepted-transition census and
canonical deliberation order. `MOIRADR2` encodes that registry using bounded
nested `MOIRADL2` documents and likewise refuses prototype `MOIRADR1` bytes.
`moira-protocol::deliberation` independently owns
the public command, refusal and evidence-bearing read DTOs;
`moira-host::DeliberationStore` defines the persistence seam; and
`moira-service::deliberation` maps that contract without bypassing authorization,
optimistic revision or matching-replay rules. Capability-shaped public clients
are available for mutation and read-only use. The separate deliberation SQLite
profile persists canonical `MOIRADR2` state under the same registry revision,
validates structural integrity, owner, configuration profile, canonical recovery
and revision agreement before trusting retained state, and has a public
client/service reopen journey. Wire transport remains open.

`moira-dendrite` is the loopback-only MCP adapter for grounding evidence. It
performs the MCP initialize/initialized exchange, retains the negotiated session,
supports bearer authentication, bounds request/response/member sizes, ignores
display text, and accepts only structured fold, surface and orthogonality data.
It recomputes Dendrite's ordered canonical surface digest before producing core
evidence. Live qualification covers both disabled-auth development mode and a
required-auth token journey against the real workspace corpus. Its complete
plan-grounding operation accepts only fold tags authored by the selected Rune
state, detects method/fold/lattice identity changes across the multi-call read,
and domain-separates each concern-root digest over the exact lattice identity
and canonical role name.

`moira-daimon::ensemble` executes one already-admitted cohort through Daimon's
public model client. It refuses incomplete cohorts and duplicate report,
execution, idempotency or effect identities before dispatch. A completed member
result becomes a report whose evidence chain includes the retained Rune
machine/state digests, Dendrite authority/surface digests and Daimon provider
evidence; output bytes receive their own digest. Completed output is not itself
admissible: it must parse as the closed `moira.member-response.v1` envelope,
use only plan-admitted claim kinds and stay within unique canonical
evidence/dissent bounds. The harness computes each claim text's SHA-256 identity;
it never trusts the model to self-attest one. Invalid structured
output, refused, uncertain, non-completed and adapter-failed members remain
distinct attributed outcomes. This adapter creates no proof, choice, maybe or
mu result. Registry integration evidence admits every report before a terminal
resolution is accepted.

`moira-local-harness` is the application composition over those seams. It
validates the complete member-run cohort before durable plan admission, invokes
Daimon, commits every valid report through the optimistic store contract and
returns all invalid or partial outcomes to the caller. A report-commit failure
returns the complete execution outcomes and last retained checkpoint for
reconciliation. Terminal resolution is a separate explicit operation; the
harness does not derive one from model agreement. The Unix profile connects to
Daimon's authenticated private model socket, so local provider configuration,
credentials, Aegis authorization and Chronos-attempt validation remain in the
owning Daimon composition. Qualification covers two-member socket execution and
SQLite report/dissent reopen.

`moira-methodical-local` is the strict one-shot operator composition. Its closed
bounded JSON configuration names the exact Dendrite method/fold/lattice,
deliberation store owner, Daimon socket and stable execution identities; bearer
material is referenced by environment-variable name rather than embedded. The
offline check path performs no peer I/O or mutation. Execution emits only
revision/outcome counts and readiness. The selected live journey grounds the
real architecture/network/identity/data cohort through Dendrite, crosses the
private model socket and commits four reports to SQLite at registry revision 5.

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

## Closed saga and operator profile

The separate closed saga profile coordinates forward, terminal and compensation
effects with explicit time, retry and uncertainty. Bounded reconciliation reads
expose the last confirmed revision, retained evidence and exact fence cause.
Operator mutations are revision-guarded and limited to attributable evidence,
keep-uncertain, fresh retry, compensation, cancellation or terminal failure;
there is no force-success command. Each accepted operation atomically retains an
opaque action ID, subject and Aegis decision/epoch causation before reply.

The public-client Aegis adapter requests `Administer` on the stable V1 saga
resource (`MOIRASG1` plus big-endian saga ID). Grant handles remain session-local.
After restart, a newly authorized decision for the same subject may replay the
original action receipt; changed subject, saga or disposition conflicts.

The selected local Unix deployment adapter maps a kernel-established effective
UID/GID pair through a bounded exact registry only to ordinary `read`/`mutate`
facts. Unknown peers are refused before readiness. In the saga profile, operator
authorization is a separately injected command-scoped decision; the account
mapping cannot produce an `OperatorAccessV1` or substitute for Aegis evidence.
The selected Aegis implementation consumes an explicit time source, requests
`Moira/Administer` on the exact saga resource through the public client and maps
only a current accepted decision to `OperatorAccessV1`; Aegis refusal maps to no
operator facts.

The public-client Hermes adapter encodes the closed Moira transition family under
the bounded Moira-owned `MOIRAWF1` schema and uses a caller-retained event identity
for exact append replay. The public-client Agora adapter maps a bounded response
intent to V2 `SpeakIdentified`. While Agora's result is unknown, the adapter
retains both Agora's exact request and Moira's effect/source/room correspondence;
a different intent refuses before dispatch. Accepted revision/message sequence
becomes an attributable Moira publication receipt. The public-client Chronos
adapter prepares one atomic invocation and makes its server-issued serial
exportable; uncertain execution closes through invocation lookup, never automatic
resubmission. After an exact V3 applied result, the same host-bound client's V4
original-allocation lookup supplies the retained singleton task incarnation and
admission revision; missing, expired or uncaptured history yields no receipt.
Current-name snapshots are not reconciliation evidence because retirement and
name reuse can change their referent. None of these adapters treats peer
correlation handles as credentials or imports peer engine/storage semantics.

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

## Closed saga kernel

`moira_core::saga` implements a separate closed V1 forward/terminal/compensation
algebra. It receives caller time, retains one timer intent and deadline, allocates
fresh identities for explicit bounded retries and compensation, and treats
uncertainty as a fence. Timer firing is an explicit command with declared policy;
early firing refuses and timeout never implies success or retry. Serialized command
order closes cancellation, late-evidence and timer precedence. See
`docs/saga-v1.md`. SQLite and protocol adoption require an explicit new profile.

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
checks. Every current owner also requires bounded SQLite `quick_check(1)` to
report exactly `ok` before trusting existing state or changing persistent
connection settings; detected structural damage refuses without repair or reset.
It commits the staged semantic checkpoint before releasing a newly accepted
transition, marks replay explicitly, and fences uncertain commit outcomes until
reopen. Rollback-journal `DELETE`, `synchronous=EXTRA` and `fullfsync` are requested.
The two native stores also expose a bounded SQLite online snapshot while the
single owner remains open. A private partial file is published to a new
destination only after finite backup steps, structural verification, source
identity recheck and file/parent synchronization. Its receipt binds the snapshot
to the retained storage revision; later source progress is intentionally absent.
Failure after the atomic rename is reported as publication uncertainty, never as
a definite refusal; an operator must inspect the destination before retrying.
The [README](README.md#local-persistence-profile) states the bounded local profile
and tested fault boundaries. The coordinator store dispatches no external effects;
the separate `moira-execution-sqlite` Chronos receiver implements its admission
mapping only. The Agora projection is owned by `moira-agora-sqlite`; the coupled
Chronos/Daimon execution projection is isolated in `moira-execution-sqlite` because
its settlement contract consumes receiver-sealed Daimon facts. Shared ownership,
integrity, timing and usage mechanics remain in `moira-sqlite`, whose production
closure contains no peer engine. Physical
power-loss survival, arbitrary-bit detection and valid-looking rollback are not
certified.

## Public Daimon execution mapping

`moira-daimon` constructs Daimon's native execution request from a Moira-owned
effect, exact task incarnation/attempt, principal, selected profile, bounded
context/tool references and explicit budget. The adapter depends only on
`daimon-client`, `daimon-protocol` and `moira-protocol`; current Aegis authority,
Chronos attempt validation, provider selection and execution state remain Daimon
service responsibilities.

The returned execution identity must equal the requested identity. A terminal
Daimon result maps to the same Moira effect/task/attempt and closed disposition.
Daimon's outcome-unknown fence maps to an attributable Moira `Uncertain` receipt,
never an inferred failure or automatic retry. A transport failure is also exposed
as outcome unknown to the host because dispatch may already have occurred.

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

