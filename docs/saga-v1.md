# Closed saga and timer algebra

Status: implemented in the deterministic `moira_core::saga` kernel with bounded
model evidence, a separate canonical SQLite profile and native
host/protocol/service/client boundaries. Wire transport and operator
reconciliation for this kernel remain open.

V1 coordinates exactly three effect kinds: forward, terminal publication and
compensation. It is not a workflow language or DAG scheduler. Cancellation or
deadline policy after known forward success prepares a new compensation effect.
Compensation is attributable work with its own identity and evidence; it never
erases the forward fact or claims a distributed rollback.

Every saga has a caller-supplied logical deadline, one stable timer identity and
a finite caller-selected retry budget bounded by engine limits. The engine reads
no clock and schedules no timer. A host submits `FireDeadline` with observed time
and an explicit keep-uncertain, fail or compensate action.

Retry names the failed effect, is accepted only from a typed failed phase before
the deadline, and always allocates a fresh effect identity. No timeout, connector
error, restart or uncertainty automatically consumes a retry.

## Deadline and precedence rules

- Firing before the deadline refuses as `DeadlineNotReached`.
- Forward/terminal preparation at or after the deadline refuses; compensation
  remains permitted because it disposes already known forward work.
- Exact repeated timer evidence replays; changed time/action conflicts.
- Forward timeout may keep uncertain or fail, but cannot compensate without a
  known successful forward effect.
- Terminal timeout may keep uncertain or prepare compensation; it cannot simply
  fail while leaving known forward work undisposed.
- Compensation timeout may keep uncertain or fail. Another attempt requires an
  explicit retry and remaining budget.

Serialized optimistic-revision order defines precedence. Evidence accepted first
determines the phase; cancellation or deadline policy accepted first makes new
old-effect evidence late. Previously accepted evidence remains exactly replayable.
New evidence cannot clear `FencedUncertain`; that belongs to operator reconciliation.

Cancellation before known forward success needs no compensation. Cancellation
after forward success or during terminal pending/failure prepares compensation.

## Fairness observation

Admission is bounded by `max_sagas`. `inspect_page` exposes immutable snapshots
in stable saga-identity order, with a caller limit capped by
`max_inspection_page`; invalid page sizes receive a distinct typed refusal. The
content-free census counts runnable effects, workflows awaiting an explicit
decision, uncertainty fences, terminal workflows and retries consumed, and
reports counter saturation. Inspection does not change the engine revision.

These surfaces let a host detect starvation or decision backlog without making
fairness policy authoritative in the core. Moira does not select the next saga,
assign priority, acquire leases, schedule timers or read an ambient clock. A host
must make those choices explicitly and submit ordinary revisioned commands.

## Recovery and model evidence

The semantic checkpoint retains sources, phases, effect/timer identities, retry
ancestry, accepted evidence and timer decisions. Recovery validates source
bijection, limits and identity bounds.

`tests/saga_reference_model.rs` independently models 36 combinations spanning
forward success/failure/uncertainty, terminal success/retry/cancel/deadline
compensation, and compensation success/failure/uncertainty. Every accepted
transition is checkpoint-recovered and replayed to simulate restart plus lost
reply. Focused tests cover misfire, retry exhaustion, late results and attribution.
They also prove ordered bounded multi-saga inspection, exact census values and
the absence of inspection-side revision changes.

## Durable profile

`saga::persistence` defines canonical bounded V1 checkpoint bytes and validates
semantic recovery before encoding and after decoding. Truncation, trailing data,
non-canonical identity order, host bounds and invalid recovery state are refused.

`moira_sqlite::saga::SagaSqliteStore` is a separately named single-owner profile
with its own application identifier, schema and database file. It persists both
storage and domain revisions, binds the selected saga limits at database
creation, and releases a newly accepted transition only after commit. Reopen
retains exact evidence replay and fairness observations. A legacy coordinator
database is refused as foreign rather than implicitly migrated. This is durable
local recovery evidence, not a remote coordination claim.

## Native service profile

`moira_host::SagaStore` isolates durable ownership from application behavior.
The representation-independent `moira_protocol` saga family covers every
mutation, transition, refusal, point read, bounded inspection page and census.
`moira_service::saga::SagaService` maps exhaustively between protocol and core,
and refuses missing read/mutate access before store dispatch. The client exposes
disjoint mutation and read-only facades. A real service-to-SQLite journey proves
status, read models and exact mutation replay after reopen. These APIs do not
grant authority from saga/effect identifiers.

`moira_wire` assigns the saga family distinct request/reply frame kinds, so a
legacy workflow frame cannot be reinterpreted as a saga command. Its canonical
bounded codec covers every command, mutation, operator disposition, transition,
outcome and refusal. Page counts are checked against remaining frame bytes before
allocation. The private Unix saga daemon adds authentication, finite request and
frame ceilings, deadlines, content-free health and command-scoped operator
authorization. Its real socket journey proves reconciliation and subject-free
receipt export survive daemon plus SQLite reopen, exact stale-request replay, and
deny-without-mutation behavior.

## Reconciliation inspection

Each saga retains the global revision at which its state was last confirmed.
The reconciliation view combines that revision with the current snapshot,
retained-evidence count, automatic-progress classification and an explicit fence
cause. Uncertainty fences distinguish an uncertain effect (identity, kind and
observation time) from a deadline decision (timer and observation time).

Retained effect evidence is exposed only through identity-ordered pages bounded
by `max_inspection_page`. Reads do not mutate revision, release work or confer
authority. The native service/SQLite journey proves the last-confirmed revision,
uncertain-effect fence and exact evidence page survive reopen. Disposition
authority is kept separate from these read models.

## Operator dispositions

The core has a closed bounded operator vocabulary: admit definitive evidence for
the named current effect, keep uncertain, retry a known failed effect with a
fresh identity, prepare compensation, cancel, or fail terminal. There is no
generic force-success command. Successful forward evidence advances only to
`ForwardSucceeded`; compensation remains new attributable work.

Every operation carries an opaque idempotency action ID plus verified subject,
Aegis decision and authority epoch facts. The bounded checkpoint retains those
facts and the exact result. Matching action replay succeeds even with the stale
revision from a lost reply. Reauthorization after restart may supply a new
session-local decision handle, so exact replay keys on the original subject,
action, saga and disposition; changing any of those conflicts. The originally
retained decision and epoch remain the causation. The SQLite store commits the
causation and resulting saga state before the service releases the reply.

Read-authorized clients can fetch one operator receipt or an action-ordered page
bounded by `max_inspection_page`. The content-safe summary includes action, saga,
original Aegis decision and epoch, requested disposition, resulting phase and any
fresh prepared effect. It deliberately omits the authenticated subject and never
returns a grant handle. Point and page reads do not mutate revision or confer
authority. Core recovery tests cover every disposition, while the native service
journey proves the export survives SQLite reopen.

Ordinary read/mutate access cannot invoke these operations. A separate service
entry point accepts only facts produced by an authentication/Aegis adapter and
injects them into the core; caller DTOs contain no authority facts.

`moira-aegis` maps each saga to the stable `MOIRASG1` plus big-endian identifier
resource and requests `Administer` through the public session-bound Aegis client.
Grant handles never leave the adapter. Reauthorization after restart may return
a new session-local decision handle; matching action ID, subject, saga and
disposition replay the originally retained causation, while a changed subject
conflicts. A real public Aegis service→adapter→Moira SQLite journey proves an
administer grant succeeds, an observe-only grant refuses without mutation, and
the authorized action replays after reopen.
