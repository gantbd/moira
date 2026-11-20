# Moira

Moira is the planned durable workflow coordinator for composing Agora, Chronos,
Daimon, Vellum, Hermes, and Aegis without absorbing their domain semantics.

The repository now contains a deterministic in-memory workflow slice from an
explicitly admitted Agora message reference through Chronos admission, one exact
Chronos attempt, an attributable Daimon result, and replay-safe Agora response
publication. Each
boundary retains one stable effect identity; matching input/receipt replay is an
identity result and conflicting replay is refused. An uncertain harness result
fences automatic progress. A validated in-memory checkpoint transfer preserves
every effect identity across engine reconstruction. A bounded versioned byte codec
and the separate `moira-sqlite` adapter now persist coordinator commands in a real
local SQLite database. Dedicated `moira-execution-sqlite` and
`moira-agora-sqlite` receiver packages now close
the first local durable message-to-reply slice. The four-store path survives lost
result/publication replies without a duplicate task, fake dispatch or room message;
uncertainty remains attributable and fenced. Production providers, authenticated
transports and Vellum publication remain outside this slice.

## Native service boundary

`moira-host` defines the durable coordinator port, `moira-sqlite` implements that
port plus the closed saga stores, and `moira-protocol`, `moira-service` and
`moira-client` expose the first
transport-neutral mutation and read-only contract. The service preserves exact
optimistic revision refusal, durable replay classification, storage-revision
evidence, workflow snapshots and the store's reopen fence. A real SQLite journey
proves admission, service teardown, reopen, read-model recovery and exact replay
through this boundary. `moira-wire` encodes the representation-independent legacy
and saga DTO families with distinct canonical bounded V1 frames and rejects every
truncation/trailing byte or cross-family interpretation.
`moira-local-daemon` now supplies the private local transport: authenticated
read/mutate facts, negotiated finite frame/request ceilings, I/O deadlines,
truthful pre-dispatch versus outcome-unknown errors, content-free health and
inode-safe socket cleanup. Its real-socket journeys prove exact replay after both
daemon and SQLite reopen; the saga profile additionally proves command-scoped
operator authorization, receipt export and denial without mutation. See [the
local daemon profile](docs/local-daemon.md).
`moira-unix-peer` supplies the selected local OS-account binding: kernel-derived
effective UID/GID maps exactly to bounded read/mutate facts and unknown peers are
refused before readiness. Saga reconciliation still requires a separately
injected command-scoped operator authorizer; the account mapping grants no Aegis
or workflow authority. See [the credential profile](docs/unix-peer-credentials.md).
`moira-aegis::AegisOperatorAuthorizerV1` supplies the selected implementation of
that service-owned port using explicit time and exact-saga `Administer` checks.
Remote identity, same-UID hostile-process isolation and remaining peer-client
ports remain open.

`moira-saga-local` packages that closed saga profile as a sovereign production
owner. A strict bounded JSON document selects the durable database, private
socket, exact kernel UID/GID read/mutate mappings and all capacity/deadline
limits. Standalone signal drain and Workbench's exact one-byte supervised drain
both remove only the owned socket; durable mutation and reopen are qualified
through `moira-client::unix::SagaUnixPort`. This selected application explicitly
denies operator reconciliation rather than inferring Aegis authority from local
identity or supervision.

`moira-daimon` maps one exact Moira effect/task-attempt execution intent through
Daimon's public execution client. It carries no Aegis decision, Chronos lease,
provider receipt or Daimon engine value. A completed public result becomes an
exact Moira result receipt; Daimon's sticky outcome-unknown reply becomes an
attributable `Uncertain` receipt without an automatic retry. A real Daimon
service journey proves matching replay calls the provider once. Durable Daimon
daemon/process recovery is qualified separately: the first reply is discarded,
a fresh daemon process reopens the canonical store, and the public adapter returns
the exact Moira receipt without a second provider call.

The additive model path retains the same Moira effect, execution and Chronos
attempt attribution while returning Daimon's evidence digest, observed usage,
bounded output and typed Agora/Vellum proposals. These are not peer receipts.
`moira-vellum` maps a separately allocated Moira effect to Vellum's public durable
`ReplaceDocumentAndSave` operation, with deployment-owned document routing and
source extent. Exact historical lookup reconciles uncertainty without falling
through to another edit. `make qualify-vellum-process` proves that behavior
against an independent authenticated Vellum daemon: the accepted reply is
discarded and a fresh public client observes the exact durable receipt and one
replacement. `moira_core::model_saga` assigns a distinct Moira effect
to every proposal, advances them in order and fences the queued tail on an
uncertain peer result. Its canonical bounded checkpoint and separate
`ModelSagaSqliteStore` preserve that exact frontier across reopen; public
protocol, service and capability-shaped clients expose the saga without exposing
core representation. Admission now also requires and retains a nonzero action-
selection digest. The harness derives it from the exact checkpoint, retained
member/report, output digest, Daimon effect and either a one-member chatbot
policy or an exact Aegis-approved action selection. The `MOIRAMP2` checkpoint
binds that digest and refuses the earlier unbound identity instead of upgrading
it implicitly. A composed public-client journey proves one Chronos
validation and one provider call under model replay, one Agora message after a
lost reply, and one Vellum mutation followed by lookup-only reconciliation after
SQLite reopen. `moira-local-harness::peer_turn` now provides the reusable
admission/dispatch loop: trusted composition supplies Vellum's retained source
extent, tool calls stay in their independent continuation saga, effects advance
in provider order, and a fenced effect can enter reconciliation but cannot be
blindly executed again. Completed replay is peer-side-effect free. Selecting
which ensemble result may create effects remains an explicit policy/resolution
step; retaining an expert report does not publish or edit. Private wire/daemon
transport and operator route composition are implemented for the selected local
profile. The exact-owner peer-effect matrix is closed; the wider top-level
ingress/execution/event kill matrix remains open. A selected ensemble journey now begins unresolved, obtains one real Aegis
decision for Pareto-frontier candidate choice and a second distinct decision for
the exact retained report allowed to act, crosses trusted Vellum routing and
`MOIRAMP2`, recovers a lost edit reply by historical lookup only, and replays
the retained selection without another peer operation. A newly authorized
selection has a new identity and correctly conflicts with that replay.
After proposal admission, the recovery entry point obtains both the current
storage revision and retained saga directly from the service before touching a
peer. SQLite reopen therefore needs neither the ephemeral selected-result value
nor another Aegis decision, while a missing saga cannot trigger a peer effect.

The Agora adapter now also exposes a store-independent durable publisher
runtime. It commits the exact client request/intent before peer dispatch and the
exact publication outcome before returning it. Matching outcome replay is served
without another Agora call, including when the outcome-store commit succeeded
but its reply was lost. The separately owned
`moira-agora-publisher-sqlite` profile pins owner, client namespace and bounded
checkpoint profile, refuses drift, and proves prepared-request and completed-
outcome recovery across independent SQLite reopens without another message.
`moira-agora-local` now supplies the reusable private Unix `ServicePortV2`
needed by local operator composition. It negotiates Agora's canonical handshake,
retains server-incarnation and server-issued client identity, enforces negotiated
frame/request ceilings and distinguishes pre-send failure from an ambiguous
post-send outcome. This transport remains separate from the semantic adapter so
the latter still depends only on Agora's public client/protocol contracts.
The isolated `qualify-agora-local-port` profile crosses the actual
Aegis-authorized Agora daemon and cleanly disconnects. It also starts a durable
Agora daemon in an independent process, commits one addressed actor publication,
discards that successful reply at Moira's boundary and observes the exact result
through a fresh authenticated public client without republishing. The production
adapter continues to depend only on Agora's public actor-publication port and
local client; owner internals remain confined to the qualification workspace.
The production-shaped case launches the exact standalone Aegis and Agora
applications, provisions the scheduled-composition grant at current trusted
time, and verifies ordered clean shutdown after reconciliation.

The local harness now supplies the trusted peer router above those adapters.
Opaque `(room, reply_to)` model references resolve only through deployment-owned
Agora routes containing the real room, revision, logical time, source and
audience. Opaque Vellum references resolve only through `moira-vellum`'s trusted
workspace/document catalogue, while the retained source extent remains a host
fact. The legacy conversation router replays its prepared V2 request only within
the retained client namespace. The production-shaped actor router instead binds
the Moira effect directly to Agora's stable actor operation and reconciles only
through historical observation after reconnect; Vellum uncertainty likewise
uses historical lookup and cannot fall through to another edit. The isolated
`qualify-peer-effect-process` gate admits both effects into one SQLite-backed
saga, discards each accepted peer reply in provider order, reopens the saga
between boundaries, and completes through fresh public clients against the exact
Aegis, Agora, and Vellum standalone owners. Completed replay calls neither peer.
A selected one-member
chatbot qualification crosses policy selection, `MOIRAMP2`, both SQLite owners
and lost Agora reply recovery, then proves completed replay makes no peer call
and retains one message. The complementary ensemble qualification crosses
`AuthorizedChoice`, Aegis action selection and Vellum lookup-only recovery.

## Methodical local-model harness

The methodical profile adds Rune-defined method/state identities, exact
Dendrite fold/surface/lattice grounding, productively orthogonal persona lenses,
closed claim/evidence/dissent output, deterministic Pareto analysis and distinct
proof/authorized-choice/maybe/mu terminal values. `moira-local-harness` composes
the durable registry with Daimon's authenticated private model socket; it
validates the complete cohort before admission, commits only valid attributed
reports and never turns model agreement into authority. See the
[architecture and delivery record](docs/methodical-agent-harness.md) and the
[local composition guide](crates/moira-local-harness/README.md). The strict
one-shot executable and checked example are documented in the
[operator profile](docs/methodical-local-operator.md).

That same retained deliberation now has its own canonical bounded wire family,
authenticated finite-request private Unix daemon, and capability-shaped Unix
client. A read-only terminal attachment can observe exact personas, grounding,
reports, dissent, candidates and retained proof/authorized-choice/maybe/mu
resolution after SQLite reopen, while mutation remains refused by the service's
existing access boundary. The sibling `moira-tui` qualifies both versioned
headless JSON and a reusable stable-revision component over this path.

The strict operator exposes that core through closed `chatbot`, `workflow` and
`agent` product profiles. Chatbots admit one grounded member and an explicit
single-member action-policy digest; workflows require an orthogonal ensemble;
agents additionally require an advertised tool. Ensemble peer actions require
`AuthorizedChoice` and a separate Aegis action decision, while every tool call
retains its own Aegis and Chronos gates. These product constraints never alter
or substitute for proof, choice, maybe or mu.
`make qualify-methodical-profiles` runs all three through the same strict
operator against the real Rune/Dendrite corpus and private Daimon transport. A
singleton chatbot uses vacuous pairwise-orthogonality evidence pinned to the
served lattice identity; multi-member profiles still require the real lattice
verdict. The agent additionally crosses live Aegis, Chronos, a fixed local
process and both durable tool stores before terminal report promotion.
`make qualify-methodical-chatbot-peers` then carries the grounded chatbot result
through the complete operator, actual Daimon private wire, policy selection,
actual Agora publication and one authenticated durable Vellum edit/save.
Daimon's separate `qualify-openai-live` gate has also completed one bounded
OpenAI-compatible request against the installed Ollama `gemma3:27b` model with
nonempty output, provider-reported usage and exact frozen-profile attribution.
`qualify-methodical-chatbot-live-provider` now combines that actual provider
adapter with the grounded chatbot operator and actual Agora/Vellum daemons under
an explicit zero-temperature, digest-bound profile.
`qualify-methodical-workflow-live-provider` additionally collects four distinct
closed reports from the actual provider across Dendrite's orthogonal architecture
cohort—including bounded idea-stormer and historian lenses carried by distinct
persona contexts—and stops unresolved.
`qualify-methodical-workflow-peer-live-provider` continues it with authority:
two dominated candidates are removed, two incomparable candidates survive, and
distinct live Aegis decisions authorize one frontier choice and report action
before the selected actual-provider proposals cross actual Agora and Vellum
daemons.
`qualify-methodical-agent-live-provider` now additionally proves an actual
`qwen3:32b` native call through Aegis, Chronos, fixed-process execution, durable
tool state and provider-native resume before terminal report promotion.
Agent peer effects and the combined process-loss matrix remain open.

Interactive coding-agent sessions now cross a separate Moira public contract:
independent agent DTOs, capability-shaped mutation/read clients, authorization
before gateway dispatch, a `moira-daimon` public-client projection, distinct
canonical wire frames, and bounded local daemon profiles. A separate durable
`moira-local-harness::unix::connect_daimon_agent` composition binds that gateway
projection to Daimon's authenticated private agent socket while provider launch
policy and credentials remain in Daimon; a real-socket gate qualifies the path.
The read-only client now also observes Daimon's content-free terminal result
binding for one exact session/turn. The additive command and outcome preserve
execution, task attempt, principal/profile, operation and authority decision,
prompt/context digests, terminal event interval, disposition and output digest
through Moira's independent DTO and canonical wire. This is provenance only;
it grants no publication, task mutation, acceptance, retry or rollout authority.

Task-to-agent journey admission now has a separate owner core. An immutable,
bounded template catalogue issues expiring previews over an exact Chronos task
admission reference, requested profile/room policy, accountable human,
objective/acceptance evidence, budgets, grounding policy, authority policy and
consequence. Moira evaluates human confirmation against trusted current time,
allocates owner-scoped monotonic journey/workflow identities, and admits at
most one journey per preview with exact replay and changed-replay conflict.
Canonical `MOIRATJ1` checkpoints reject foreign-owner identities, template
drift, truncation, tails, mutation and host-limit overflow. This first slice
stops at `Admitted`. A transport-neutral public catalogue/preview/instantiate/
observe/status contract and disjoint canonical wire family now carry requested
TTL and confirmation content while only owner views publish observation,
expiry and confirmation times. Service, transactional store, client/daemon,
fresh Chronos claim/lease, room and participation effects, Daimon dispatch and
settlement remain open and cannot be inferred from admission.

Subsequent owner progression retains a trusted Chronos claim before room and
participation admission. Checkpoint V6 adds a separate exact task-namespace
binding beside the unchanged V1 attempt binding: journey, external task key,
incarnation, attempt, Chronos revision and the Chronos-owned task ID are
committed atomically by the journey store. The trusted worker supplies the task
ID obtained while claiming and fences if exact durable replay is unavailable;
changed identity conflicts. This owner fact is not ambient workspace scope, a
task-conversation relationship, a lease, current task phase, settlement
authority or human acceptance. Checkpoint versions 1 through 5 remain readable
and upgrade canonically without inventing missing namespace evidence.

`moira-client` now also owns a bounded optional Unix port for the governed
participation service, so runtime workers do not depend on terminal presentation
code. The standard `drive_current` step loads the exact store revision and then
dispatches one prepared operation, reconciles a retained uncertainty fence only
by inspection, or leaves an ordinary idle participation untouched. A Ready
resume-capable participation receives one read-only inspection so a detached
provider binding can be durably marked without automatic reattachment.
`moira-agent-local` is the strict one-step process composition for that path.
Its `--check` mode performs no socket or store access. At runtime it obtains
pending prompt bytes only through a worker-authorized, revision-and-digest-bound
daemon command, closes that connection, and reconnects before the existing
pre-dispatch currency check and fence. It never opens the prompt database.
A separate durable
participation record binds workflow/member, exact Chronos attempt, Rune method,
Dendrite grounding, persona, Daimon session/profile, execution identities,
permission evidence, and terminal evidence. Its exclusive SQLite owner and
canonical checkpoint survive reopen. Permission selection enters only through
a command-scoped Aegis `Execute` decision for the exact participation,
execution, request, option, and capability; the generic mutation route refuses forged
authority. The local driver commits a one-way dispatch fence before one provider
mutation, validates every materialized identity, and reconciles a successful
lost reply through provider inspection and cursor-contiguous events without a
redispatch path. The sibling `moira-tui` CLI can inspect a session, observe typed
events, and inspect this governed record without importing Daimon or contacting
ACP directly. Its read-only permission view joins stable participation and
cursor-contiguous event observations, verifies retained digests and option
identities, and emits a server-prepared confirmation fingerprint. Its explicit
confirmation command carries only that fingerprint and one retained option;
Moira revalidates current state, derives the operation identities, and crosses
the exact Aegis boundary without exposing authority material to the terminal.
The sibling cancellation challenge binds the active turn and current durable
context; Moira derives the cancel key and requires authenticated mutation
access, so the terminal cannot invent either turn or idempotency identity.
Permission and cancellation challenges cease to exist as soon as their exact
derived operation is durably pending. A retained provider request therefore
cannot be presented as a fresh selectable challenge during recovery.
For prompts, a separately owned content-addressed SQLite store acknowledges the
exact bounded bytes before the participation service prepares a digest-only
operation. Moira derives the turn, execution, key, and retained attempt from
the complete current binding and revisions; the terminal supplies none of
them. Exact pending submissions replay without creating another turn. The
participation daemon is the prompt store's exclusive owner. The terminal
profile is denied materialization by default; an independently authenticated
worker can receive only the exact current pending prompt. The worker and runtime
both verify its digest before the deployment template can construct a provider
command.
Assistant-message chunks flow in the other direction through a separately
owned, cursor-ordered output store. It composes bytes across permission and
advance slices, replays only the exact last page, and refuses cursor gaps,
forks, altered chunks, and bounds violations. Terminal report admission then
requires those exact bytes at the participation's final cursor and digest;
matching metadata alone cannot create a deliberation report.
The report-admission gate parses those retained bytes through the same closed
`moira.member-response.v1` contract used by ensemble execution. It derives the
report identity and baseline question, Rune method, Dendrite grounding, and
provider evidence server-side, then accepts only an exact completed EndTurn
observation whose governed context matches. Other mutating terminal controls
are not exposed; provider text and UI selection still confer no authority.

Terminal resolution is a separate authority boundary. The harness hashes the
exact unresolved checkpoint before asking an external validator or selector.
`moira-rune-validator` can bind an exact question to Rune's bounded OCaml T6
prover and will not promote `hold_quantum` or `mu` to a Boolean.
`moira-aegis::AegisChoiceAuthorityV1` authorizes selection only on the retained
Pareto frontier and preserves the Aegis decision, subject and epoch in the
durable resolution. Neither path grants authority to model output.
The strict operator now exposes this as an optional closed terminal-resolution
profile alongside explicit four-valued maybe and reasoned mu. Its proven-Boolean
profile invokes the bounded Rune validator with the exact grounded question;
its choice profile crosses Aegis over the owner-only Unix transport. Matching
resolved replay is offline, while terminal-family, candidate, subject or epoch
drift refuses.

The first structured-tool continuation kernel is also present in
`moira_core::tool_continuation`. It gives every native call a distinct effect,
requires retained Aegis and Chronos gate evidence before dispatch, and fences
the tail on an unknown outcome. Its semantic checkpoint and canonical bounded
`MOIRATC3` codec retain the exact source attempt, provider-native call
correlation, gate, execution and receipt
across a recovery boundary. A separately owned SQLite store plus representation-independent
protocol, service and capability-shaped clients preserve in-flight and completed
calls across reopen. Daimon now parses provider-native calls, preserves them in
its additive wire/checkpoint formats, and the public Moira adapter projects them
with a Moira-owned budget into the continuation client. Ensemble runs awaiting a
tool are not admitted as expert reports. Live Aegis and Chronos adapters produce
the independent authority and current-attempt gate facts. The local runtime now commits
the in-flight marker before invoking a tool once, persists bounded successful
bytes in a separately owned content-addressed SQLite store before accepting the
receipt, reconciles a lost output-store reply by execution identity, and fences
provider/output uncertainty without automatic reinvocation. Recovery closes an
in-flight effect from matching durable output or fences it when output is absent;
successful receipts resolve in provider order to exact bounded bytes. The
Moira–Daimon adapter submits those results through Daimon's private resume wire;
Daimon proves they are an exact bijection with the retained native calls and the
OpenAI-compatible adapter reconstructs the assistant tool-call plus matching
tool-result messages. Ensemble continuations retain the original member, report
and source-execution identity across repeated tool turns; intermediate calls
remain incomplete and only a terminal closed response can be committed as a
report. Multi-turn process-loss qualification remains open.
The selected production local-tool profile is shell-free and capability-fixed:
it freezes the canonical executable digest, argv and working directory, clears
the environment, drains stdout/stderr concurrently under explicit bounds, and
maps ambiguous post-spawn state to a fenced unknown outcome. Its successful
output is covered by the same durable-before-receipt path. Dynamic commands,
secret injection and multi-turn process-loss qualification remain open.
The strict `moira-methodical-local` operator now optionally composes this path:
its closed configuration requires exact advertised capability/process and
Chronos-task mappings, separate continuation/output stores and explicit turn
fuel. Pending calls obtain lazy Aegis authority and current Chronos evidence,
settle in provider order, and resume the originating Daimon member under stable
derived identities. Completed replay does not reconnect to Aegis or inspect the
executable. `make qualify-methodical-tools` proves the complete grounded local
journey through authenticated Aegis, Chronos local wire, Daimon private model
transport, the selected process, both tool stores and terminal report promotion.

An optional `peer_action` profile carries one selected terminal report into the
durable peer-effect saga. An ensemble requires a separate Aegis action-selection
decision; a one-member chatbot instead requires its explicit nonzero policy
digest. Both require distinct model-saga and Agora-publisher SQLite owners, deployment-owned Agora
conversation and Vellum document/source-extent routes, the bounded Agora V2 Unix
port, and Vellum's authenticated Unix transport. Completed peer replay returns
from the saga store without reconnecting to Aegis or either peer; retained
pending/uncertain work resumes without selecting a different action. See the
[operator profile](docs/methodical-local-operator.md) for the closed ownership
contract. `make qualify-methodical-peers` proves the simultaneous path with actual
local Aegis, Agora and authenticated Vellum daemons, one durable edit and an
offline completed replay. Combined multi-process kill recovery remains open.

## Closed saga kernel

`moira_core::saga` implements the selected bounded forward, terminal and
compensation family with caller-supplied deadlines, stable timer intents, explicit
retry budgets/fresh identities, cancellation and late-evidence precedence, and
uncertainty fencing. An independent model reconstructs and replays every accepted
transition across 36 policy programs. Bounded identity-ordered inspection and a
content-free phase census expose fairness evidence without putting scheduling or
priority policy in the core. Canonical bounded checkpoint bytes and a separately
identified SQLite saga store preserve exact evidence across reopen without
migrating the legacy coordinator database. A separate host port,
representation-independent protocol, service mapping, capability-shaped clients
and private saga wire/daemon profile preserve typed behavior through SQLite and
daemon reopen. See [the saga contract](docs/saga-v1.md).

Bounded reconciliation reads expose the last confirmed saga revision, pending
effect, retained evidence, explicit fence provenance and the reason automatic
progress is blocked. These observations survive SQLite/service reopen and remain
non-authoritative on their own.

The bounded operator command family now supports evidence adjudication,
keep-uncertain, fresh retry, compensation, cancellation and terminal failure.
The public service prepares an exact current-revision confirmation for a
proposed disposition, derives its opaque replay-stable action identity, and
revalidates both before the daemon requests fresh command-scoped authority.
The public daemon refuses the legacy raw reconciliation frame; that form exists
only as the service-internal command after confirmation and authorization.
After a lost reply, the exported action identity addresses the durable operator
receipt; the terminal does not infer success or submit a fresh action.
Action receipts durably retain redacted subject and Aegis decision/epoch
attribution and replay exactly after SQLite reopen. Ordinary mutation access
cannot invoke reconciliation. The `moira-aegis` adapter uses Aegis's public
session-bound client to require `Administer` on the exact saga resource; a live
service journey proves allow, deny-without-mutation and replay after reopen.

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
Every current SQLite owner also requires bounded `quick_check(1)` structural
validation before trusting an existing schema or state; detected damage refuses
without repair or replacement initialization.

Both native stores expose a bounded online snapshot operation while that single
owner remains open. It writes a mode-`0600` `.partial` database under explicit
page/step/retry ceilings, verifies source identity and snapshot integrity, fsyncs
the file, then atomically publishes a previously absent destination and syncs its
parent. Tests prove the source can continue to a later revision while the
snapshot reopens with the original owner/limits at its exact earlier revision and
replays retained work. This is synchronous local backup, not concurrent-writer,
continuous-archive, remote-filesystem, encryption, rollback or migration evidence.
If parent-directory synchronization fails after rename, publication is reported
as uncertain and the visible destination must be inspected before any retry.

The adapter requests rollback-journal `DELETE`, `synchronous=EXTRA` and macOS
`fullfsync`. Payload/workflow and main-database page ceilings are explicit; they
are not bounds on journal size, SQLite cache, RSS or filesystem allocations.
Real database and subprocess tests cover acknowledged commit followed by abrupt
exit, uncommitted-write abandonment, phase-by-phase reopen, uncertainty fences and
lost commit replies. They do not certify physical power-loss survival, malicious
hosts, arbitrary-bit detection, valid-looking rollback, network filesystems or
multi-owner operation. This layer dispatches no external effects and does not
make their receipts transactional.

### Chronos admission receiver

`moira_execution_sqlite::chronos::ChronosReceiver` owns a separate dedicated database, using
the same local ownership and SQLite durability profile. For one minimized task per
workflow effect, it commits Chronos's own checkpoint and the original canonical
intent/admission receipt in one transaction before returning success. Replay reads
the retained receipt without resubmitting, even with the original optimistic guard;
changed task attributes conflict. Receipt capacity is bounded without eviction.
Missing or inconsistent receipt records refuse reopen rather than creating a new
inbox. The receiver exposes no mutable engine, unrelated task admission, retirement,
or lease tokens. On uncertain storage failure, only last-confirmed revision/task
counts remain observable and further commands require reopen.

The receiver also exposes a read-only, redacted owner observation for the exact
external task key, Chronos task ID, incarnation and admission revision retained
by one receipt. It validates the task reference against the recovered Chronos
engine and survives reopen without changing engine revision or command usage.
This is namespace provenance inside one receiver, not ambient workspace scope,
task-conversation qualification, a lease, settlement authority or evidence that
the task reached any later phase.

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

`moira_execution_sqlite::daimon::FakeDaimonReceiver` uses Daimon's actual deterministic
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

`moira_agora_sqlite::AgoraReceiver` commits Agora's native checkpoint and a bounded
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
owns room authorization and participant-neutral addressing. Addressing is not an
execution request; Moira admits workflow only through its own explicit command.
No additional Aegis policy is inferred.

The receiver projects one of its own pending `MessageAccepted` outbox events as
a passive `ConversationMessageFactV1`: opaque source/room/message reference,
author, and canonical addressed participants, but no body or participant-kind
policy. Projection is read-only and replay-stable across receiver restart. A host
must explicitly map one addressed Agora participant to a `ParticipantRefV1` and
submit `AdmitMessage`; changed mapping conflicts. Acknowledging the Agora outbox
event removes it from this pending-fact surface. This is ingestion evidence, not
workflow admission, task creation, authority, or proof of participant execution.

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
lost append reply. `moira-hermes` now replaces the direct Hermes-engine shape for
production integration: it accepts representation-independent Moira transitions,
owns a bounded canonical `MOIRAWF1` event schema and publishes through only the
public Hermes append client. A real Hermes service journey loses the first reply,
then proves exact replay and one retained event. `moira-agora` publishes response
intents through Agora's actor-owned public profile, preserving the stable Moira
effect and exact room/revision/time/body/audience intent independently of
connection-issued identities. Its compatibility V2 path retains the existing
outcome-unknown request fence. An independent public-contract conformance journey
loses the first reply, replays the exact request, and retains one identified
message without linking Agora internals. `make qualify-agora-recovery` separately recreates both
the restored publisher namespace and Agora's real durable Service V2 after a
discarded committed reply, then proves exact replay performs no second commit or
message. `make qualify-agora-local-port` adds the independent-process proof: after
one committed reply is discarded, a new authenticated client observes the exact
durable actor result without another publication. `moira-chronos` uses Chronos V3 and V4 on
the same host-bound service. Enable allocation capture with
`--atomic-state --allocation-receipts` before admission. Prepared admission
retains the server-issued serial and derives the invocation key from the Moira
effect. After V3 reports Applied, V4 retrieves the original allocation for the
invocation. The original incarnation and admission revision become Moira
evidence. Missing or expired allocations return `HistoryLost`.
Daimon's public-client execution segment
now has discarded-reply process-restart evidence both in isolation and under live
Aegis/Chronos composition. A SQLite-backed Moira daemon now prepares the workflow,
exits and reopens to accept the recovered result. The completed transition continues
through an independent Hermes daemon and reads
back one exact Moira-owned event under an exact-stream grant from the same live
Aegis authority; the complete message-to-reply chain remains open.

The Aegis→Moira daemon journey runs from the isolated
`qualification/aegis-moira-process` workspace via `make qualify-aegis-process`.
The governed interactive-agent worker and its two private-socket process
journeys run via `make qualify-agent-local`.
Agora durable-service restart assurance runs from the isolated
`qualification/agora-service-recovery` workspace via `make qualify-agora-recovery`.
Chronos service restart/lookup assurance runs from the isolated
`qualification/chronos-service-recovery` workspace via
`make qualify-chronos-recovery`. Separate lockfiles preserve each peer's storage
ownership while the root Moira workspace keeps only public production edges.
That profile now includes actual Chronos process exit/reopen over the owner-only
Unix transport after discarding the execute reply. The test retires the original
task and reuses its name before read-only reconciliation. Moira still returns
the original incarnation and revision.
`make qualify-hermes-process` additionally discards a real Hermes daemon reply,
restarts the daemon over its durable directory, and proves the same Moira event
replays once through only Hermes's public client/wire contract.
`make qualify-daimon-process` discards a real Daimon execution reply, restarts the
daemon over its durable directory, and proves the exact Moira result receipt is
recovered through the public client/wire without provider redispatch.
`make qualify-live-execution-chain` adds simultaneous real Aegis, Chronos and
Daimon processes: the same Moira execution is admitted only after live profile/tool
authority and the exact running Chronos attempt are observed through public ports.
Its recovery case discards the result reply, reopens Daimon, reauthorizes retained
delivery and proves the provider marker remains singular.

## Boundary

Moira owns cross-engine correlation, workflow phases, effect intents, attributed
receipts, and terminal workflow outcomes. Chronos owns task scheduling and leases;
Agora owns conversations; Daimon owns harness execution; Vellum owns document
publication; Hermes owns event delivery; Aegis owns authority evidence.

The first target is one recoverable vertical slice:

```text
addressed Agora fact -> explicit Moira admission -> Chronos task
    -> fake Daimon result -> Agora reply
```

Restart at every boundary must produce neither a duplicate task nor a duplicate
reply. See [requirements](requirements.md), [specification](specification.md), and
the [focused roadmap](docs/roadmap.md). The staged production program is tracked
in [ROADMAP.production.md](ROADMAP.production.md); [SECURITY.md](SECURITY.md) and
the [threat model](docs/threat-model.md) define current security claims and
residual risks.

## Development

```sh
make check
make check-peer-profiles
make soak-live-execution # opt-in 16-cycle live execution-to-event campaign
```

The second gate keeps peer-owned assurance graphs isolated while enforcing the
Aegis process, Chronos recovery, Daimon process, live execution chain and Hermes
process profiles in CI.

## SDK facade

`moira-sdk-embedded` reexports the workflow engine without selecting peer
engines, persistence, effect interpreters, or runtime policy.

