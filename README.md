# Moira

Moira is the planned durable workflow coordinator for composing Agora, Chronos,
Daimon, Vellum, Hermes, and Aegis without absorbing their domain semantics.

The repository now contains a deterministic in-memory workflow slice from an
Agora message reference through Chronos admission, one exact Chronos attempt, an
attributable Daimon result, and replay-safe Agora response publication. Each
boundary retains one stable effect identity; matching input/receipt replay is an
identity result and conflicting replay is refused. An uncertain harness result
fences automatic progress. A validated in-memory checkpoint transfer preserves
every effect identity across engine reconstruction. A bounded versioned byte codec
and the separate `moira-sqlite` adapter now persist coordinator commands in a real
local SQLite database. Dedicated Chronos, fake-Daimon and Agora receivers now close
the first local durable message-to-reply slice. The four-store path survives lost
result/publication replies without a duplicate task, fake dispatch or room message;
uncertainty remains attributable and fenced. Production providers, authenticated
transports and Vellum publication remain outside this slice.

Semantic recovery now rejects rewound/skipped allocation frontiers, missing
replay-index entries, duplicate/foreign allocations, inconsistent revisions and
misattributed retained receipts. Valid phase/disposition interleavings retain
their identities. This is a prerequisite for storage, not durable recovery itself.

`Engine::apply_observed` optionally records closed command/result classifications
through Keystone's preallocated bounded capture buffer. Counters distinguish new
acceptance, matching replay, refusal and newly accepted harness receipts; current
state gauges report workflow phases and pending effects. Disabled, available and
overloaded capture preserve the same domain results. Capture loss is explicit and
counters saturate without wrapping. Observations contain no content or identity
labels, read no clock and establish neither latency nor durable/audit evidence.
The SQLite adapter separately counts command offers, new commits, retained replay,
refusals, conflicts, indeterminate commits and loads. Optional process timing
measures the SQL transaction attempt only; untimed samples remain explicit, and
replay adds no commit sample. Counters saturate with a sticky overflow flag and
carry no content/identity labels. These observations are not durable audit evidence,
connector latency, end-to-end responsiveness or an SLO qualification.

## Local persistence profile

`moira_core::persistence` uses Keystone's bounded decoder with a closed V1 byte
vocabulary, canonical ordering and semantic recovery validation. The core still
performs no storage I/O. `moira-sqlite::SqliteStore` accepts domain commands rather
than arbitrary replacement checkpoints, preventing callers from rewinding the
allocator by substituting a separately valid older state. Newly accepted results
are released only after the SQLite transaction acknowledges commit. Matching replay
is explicitly marked as observation, not fresh dispatch. An uncertain commit fences
that instance until reopen; recovery can then observe the retained outcome.

The first target is a trusted local macOS directory, a regular single-link database
and one cooperating active owner. A persistent `.owner.lock` sidecar is kept separate
from SQLite's database locks. Do not delete or replace either file while in use;
hard-linked databases are refused and symlink paths resolve to their canonical
destination. Logical owner identity survives restart but is not authorization.
Foreign databases are refused, and missing previously initialized state is not
treated as a fresh allocator. Database and lock replacement fence the live owner.

The adapter requests rollback-journal `DELETE`, `synchronous=EXTRA` and macOS
`fullfsync`. Payload/workflow and main-database page ceilings are explicit; they
are not bounds on journal size, SQLite cache, RSS or filesystem allocations.
Real database and subprocess tests cover acknowledged commit followed by abrupt
exit, uncommitted-write abandonment, phase-by-phase reopen, uncertainty fences and
lost commit replies. They do not certify physical power-loss survival, malicious
hosts, network filesystems or multi-owner operation. This layer dispatches no
external effects and does not make their receipts transactional.

### Chronos admission receiver

`moira_sqlite::chronos::ChronosReceiver` owns a separate dedicated database, using
the same local ownership and SQLite durability profile. For one minimized task per
workflow effect, it commits Chronos's own checkpoint and the original canonical
intent/admission receipt in one transaction before returning success. Replay reads
the retained receipt without resubmitting, even with the original optimistic guard;
changed task attributes conflict. Receipt capacity is bounded without eviction.
Missing or inconsistent receipt records refuse reopen rather than creating a new
inbox. The receiver exposes no mutable engine, unrelated task admission, retirement,
or lease tokens. On uncertain storage failure, only last-confirmed revision/task
counts remain observable and further commands require reopen.

Actual tests restart both coordinator and receiver around a lost admission reply,
and separately exit a receiver subprocess after commit but before origin
acknowledgement. Both retain one exact task/receipt. Transaction faults cover
before/after commit; counters and optional transaction timing share the content-free
observation contract above. This adapter does not authenticate callers or revive
grants/leases. Hosts must supply current authorization.

Targeted claims now commit Chronos's checkpoint and an exact attempt record before
returning an authority-free attempt receipt. `authorize_worker` accepts an explicit
host-authenticated agent assertion, validity interval and finite claim budget;
Chronos issues the volatile grant and owns its validation. Leases remain private.
Claim replay observes the original attempt rather than dispatching again. Expired
or foreign grants cannot claim new work; expired live claims do not return a
`Running` replay. This first profile has one retained attempt per admitted task
and performs no automatic retry. Exact sealed result facts can now settle the
owned attempt or reconcile its recovered uncertainty under explicit host intent.

On reopen, Chronos revokes recovered leases and marks running attempts uncertain.
The receiver commits that recovery fence before exposing observations; subsequent
reopens retain it without another claim or repeated recovery transition. Schema V1
admission-only and V2 attempt stores upgrade atomically to V3 without replacing
their receipts or reviving leases.
Claim offers/commits/replays/refusals and optional SQL transaction timing are separate
from admission counters. Worker grant offers/issuance/refusals and startup fence
count/timing are exposed without identity/content labels. Real subprocess loss,
before/after-commit faults and a durable Moira harness-intent handoff are tested.
A recovered uncertain claim is not itself evidence that Daimon executed.

Settlement accepts only evidence sealed by the durable fake-Daimon receiver, plus
an explicit action and host-established source-owner correspondence. `Complete`,
`FailTerminal`, `Cancel` and `KeepUncertain` remain distinct. A timeout requires an
explicit terminal-failure or uncertainty choice; there is no hidden retry/default.
Chronos owns the live lease, exact cancellation or recovered reconciliation path.
Expired live completion/failure is refused; reopen can fence the old attempt and
then reconcile its retained known result without reviving the lease.

The native checkpoint, exact settlement record and settlement census commit
together before success. Matching replay returns the original settlement revision,
including after a lost commit reply. Missing records refuse recovery. Contradictory
evidence is retained as a bounded second fact and commits a receiver-wide conflict
fence; that refusal is not an identity storage operation. Reopen does not clear the
fence. The first profile has no automatic conflict clearance, policy replacement or
late-evidence promotion. Settlement counters and optional SQL timings are separate
from claim/admission observations; conflict status is explicit and persistent.

### Durable fake-Daimon receiver

`moira_sqlite::daimon::FakeDaimonReceiver` uses Daimon's actual deterministic
prepare/dispatch contracts, with no model, tool, process or network invocation.
A canonical bounded request/fake-reply codec reuses Keystone's decoder. The
receiver commits a dispatch marker before the fake dispatch and commits the exact
result before returning it. A surviving marker without a result becomes explicit
`RecoveryUnknown`, not a fabricated provider receipt or completion. It is never
automatically dispatched again. Retained results replay without calling dispatch;
changed input, execution-ID aliases and idempotency-key aliases conflict. Entry,
record and database-page limits are explicit and retained entries are not evicted.

The adopted Aegis gate checks current `Execute` permission on the requested profile
and every requested tool, including on retained-result delivery. The host binds the
authenticated subject/session and supplies the live Aegis owner/grant. New dispatch
also checks the exact currently leased Chronos task incarnation/attempt; no lease
or authority is serialized. Aegis remains a volatile authority core: its durable
revocation/recovery is not established by this receiver or by reusing an engine ID.
Hosts own current authority continuity and explicit time inputs.
Host-issued profile/tool IDs must not alias distinct resources within that Aegis
namespace; the adapter does not invent a global identity registry.

Execution counters separate calls, new results, replay, recovery unknown, authority
and attempt refusals, conflicts, fake dispatch calls and indeterminate operations.
Actual SQL transaction counts and optional timing are separate populations;
startup exposes the number of newly fenced markers. These are content/identity-free,
process-local diagnostics, not audit evidence or latency/SLO certification. Tests
cover real process exit after a marker, commit-reply loss, no redispatch, revocation,
empty-tool profile authorization and all three durable stores reopening before Moira
accepts the lost result, settles the exact Chronos attempt and prepares its Agora
publication intent.

### Durable Agora receiver

`moira_sqlite::agora::AgoraReceiver` commits Agora's native checkpoint and a bounded
bijective room-ordinal catalogue in one SQLite transaction. The native checkpoint
includes publication receipts, retained messages and the ordered event outbox, so
reply acceptance and its exact replay receipt survive together. A supplied source
scope is persisted and checked on reopen; ordinals are assigned on accepted room
creation and never remapped in this no-room-deletion profile. Missing/inconsistent
catalogue state refuses recovery. Public observations and mutation results are
withheld while storage outcome is uncertain; only last-confirmed revisions remain.

Core commands, keyed publications and outbox acknowledgements retain separate
offer/result/timing populations. Read counters and startup byte/room/time observations
are explicit. Timed samples measure actual SQL attempts (startup separately), not
end-to-end/UI responsiveness or delivery proof. Refusal diagnostics classify causes
without participant/message identifiers. Acknowledgements preserve exact prefixes
but remain host assertions of external delivery. Hosts authenticate actors; Agora
owns room authorization and activation. No additional Aegis policy is inferred.

Real database tests cover commit rollback/reply loss, receipt replay after message
eviction, exact outbox recovery and receiver process exit without cleanup. The full
four-store scenario begins with a persisted human message, settles the exact Chronos
attempt, replays the lost fake result, and completes Moira with the original Agora
reply receipt after reopening all stores. Its uncertain variant persists a workflow
fence and leaves only the input message. This is a library/integration slice using a
deterministic fake provider, not a production server, distributed transaction or
physical power-loss certification.

Qualification tests execute the actual neighboring Chronos, Daimon, Agora,
Hermes, and Aegis volatile contracts. Aegis decision evidence authorizes the
Daimon request, and Hermes producer-key replay retains one workflow event after a
lost append reply. These are compatibility proofs, not production connectors.

## Boundary

Moira owns cross-engine correlation, workflow phases, effect intents, attributed
receipts, and terminal workflow outcomes. Chronos owns task scheduling and leases;
Agora owns conversations; Daimon owns harness execution; Vellum owns document
publication; Hermes owns event delivery; Aegis owns authority evidence.

The first target is one recoverable vertical slice:

```text
addressed Agora message -> Chronos task -> fake Daimon result -> Agora reply
```

Restart at every boundary must produce neither a duplicate task nor a duplicate
reply. See [requirements](requirements.md), [specification](specification.md), and
the [roadmap](docs/roadmap.md).

## Development

```sh
make check
```

