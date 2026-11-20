# Traceability ledger

The deterministic workflow slice and selected local SQLite coordinator/receiver
profiles are implemented. The host/protocol/service/client and canonical bounded
wire boundaries and the deterministic saga/timer profile are implemented; the
public Aegis operator adapter, public-client Hermes workflow-event, Agora
response-publication, Chronos task-admission and Daimon execution adapters, and private saga
wire/daemon profile are
implemented. The model-proposal coordinator now has typed effect separation,
canonical bounded persistence and a separately owned SQLite profile. Other
production connectors remain open. In-memory
reconstruction is not crash-recovery evidence, and local SQLite evidence is not
a remote, distributed-transaction or physical-power-loss claim. Open is not a
pass.

| Requirement | Planned implementation | Current evidence | Status |
| --- | --- | --- | --- |
| M1–M2 authority/correlation | non-cloneable workflow engine, optimistic revision, fixed-width opaque workflow/effect/source references | canonical coordinator checkpoint, durable peer mappings, and native service/SQLite reopen journey | Satisfied for selected local vertical slice; authenticated service identity binding open |
| M3 recovery | validated semantic checkpoint and SQLite command/receiver protocols | actual database restart, before/after-commit faults, subprocess exit, phase-by-phase reopen, four-store recovery, authorized operator replay and bounded atomically published online snapshots for both native profiles | Satisfied for selected local fake-provider and saga/operator profiles; remaining production peers open |
| M4 idempotency | admitted-message index and retained Chronos/Daimon/Agora effect-receipt mappings | matching replay/conflict plus lost admission, result and publication reply recovery without duplicate effects; the four-store path reads the exact admitted Daimon result from durable Moira and reuses private-lease Chronos settlement while preserving distinct submission and execution effects | Satisfied for selected local fake-provider slice |
| M5 effect knowledge | qualified Keystone lattice plus attributed external evidence | exhaustive lattice correspondence, durable retained peer evidence, uncertainty and persistent conflict fences | Satisfied for selected local fake-provider slice; real-provider evidence open |
| M6 saga semantics | explicit forward/terminal/compensation commands, evidence and precedence | closed saga kernel, focused laws, 36-program independent recovery/lost-reply model, exact SQLite reopen/replay, native service and private wire mapping, server-prepared current-revision reconciliation challenges with derived replay-stable action identities, raw-operation refusal at the public daemon, Aegis-authorized confirmed dispositions, content-safe receipt export, and a terminal arm/confirm control whose fault-injected real socket reconnect recovers a discarded reply only by the exact durable receipt | Satisfied through authorized private local saga service and qualified terminal peer reconciliation; broader production peer-effect kill matrix remains open |
| M7 bounds | explicit workflow, identity, payload, retry, timer, retained-entry and database-page limits | core capacity, bounded codecs, bounded ordered saga inspection/census, saga retry/identity tests, allocation-adversarial wire tests and SQLite/service limits | Satisfied through native private saga daemon; production capacity qualification open |
| M8 authority | Aegis validated decisions adapted into bounded authority evidence | six-process execution-to-event chain obtains live profile and distinct-tool `Daimon/Execute` plus exact-stream Hermes `Submit`/`Observe`; `moira-aegis` uses the public client for exact saga `Administer`, deny-without-mutation and reauthorized replay | Satisfied for selected live execution and operator profiles; remaining production peer actions open |
| M9 isolation | deterministic core with explicit commands/receipts; Keystone effect lattice is the sole mechanism dependency | source boundary, behavior tests, strict lint | Satisfied for implemented volatile slice |
| M10 passive conversation ingress | durable Agora pending-event projection plus explicit core `AdmitMessage` | passive-before-admit, restart-stable fact, human/agent symmetry, mapping conflict, acknowledgement removal, and lost-admission-reply replay scenarios | Satisfied for the local Agora receiver profile |
| M11 public peer isolation | federation adapters depend on peer client/protocol contracts, not peer engines; an explicitly trusted in-process composition consumes a typed host profile in its own package rather than leaking that authority into a public adapter | dependency-closure gates; Aegis/Hermes/Chronos/Daimon service journeys and process profiles; independent Agora public-contract conformance; reconnect-safe actor observation against the exact private-local owner; Vellum historical lookup against its exact standalone owner; separate `moira-chronos-worker` host/engine closure | Adapter boundary satisfied for the selected Aegis, Hermes, actor-owned Agora, Chronos, Daimon, and Vellum profiles; the trusted Chronos worker is isolated from `moira-chronos`; the legacy session-scoped Agora V2 publisher remains non-reconnectable by design and is not the selected exact-owner recovery path |
| M12 process-separated federation | independently owned local service processes connected only by public contracts | a SQLite-backed Moira daemon admits and prepares the workflow, exits, then reopens to accept the recovered Daimon result; Aegis authorizes profile/tools and exact Hermes stream actions; Chronos validates the running attempt; Daimon discards its result reply, reopens and returns one execution without redispatch; Hermes publishes and reads one canonical Moira event; a separate exact Aegis/Agora/Vellum gate loses both selected peer-effect replies and reopens the Moira saga between actor observation and Vellum receipt lookup | Satisfied for the integrated execution-to-event segment and the combined exact-owner Agora/Vellum peer-effect segment; one top-level fault gate spanning ingress through event delivery and peer effects remains open |
| M13 storage integrity | bounded SQLite structural validation before existing state is trusted, with no repair/reset fallback | all current SQLite owners call `quick_check(1)` pre-trust; deterministic legacy/saga non-header page corruption refuses and leaves exact corrupted bytes unchanged | Satisfied for the selected structural-page profile; arbitrary corruption, rollback, torn writes and device certification remain open |
| M14 composition boundaries | separate coordinator/saga, Agora and execution-projection packages with explicit closures | Agora and Chronos/Daimon sources and tests reside in distinct crates; `check-sdk-boundaries.sh` freezes every direct production dependency set | Satisfied for the selected local SQLite architecture; full remote composition remains open |
| M15 proposal/effect separation | typed model proposals admitted into a Moira-owned ordered saga; one Moira effect per Agora/Vellum action; peer receipts never inferred from model output | deterministic proposal allocation, uncertainty fencing and historical reconciliation; canonical bounded checkpoint tests; separate SQLite reopen; representation-independent service/client; one exact-owner saga now loses and reconciles both Agora and Vellum replies in provider order, proving one actor operation and one durable edit with quiet completed replay | Satisfied for the selected local public-client and private-daemon peer-effect federation; the wider ingress/execution/event fault matrix remains open |
| M16 epistemic honesty | proof, authorized choice, four-valued maybe and reasoned mu remain distinct; Pareto removes only dominated candidates | bounded plan/report workflow with exact question/method/grounding digests, retained affirmative orthogonality evidence and dissent, correlated-ensemble refusal, exact fold/surface/orthogonality cohort assembly with authority-digest matching, bounded loopback MCP/session client, ordered surface-digest recomputation, authored method-tag validation, stable method/fold/lattice identity checks, content-bound concern identities, authenticated real-corpus complete plan-grounding qualification, distinct-identity Daimon cohort execution with attributed partial outcomes and provider evidence, closed claim/evidence/dissent envelope admission, predispatch cohort validation, governed report commit, authenticated private model-socket execution, SQLite report/dissent reopen, strict offline-checked operator configuration, real-corpus four-member executable-to-SQLite journey, repeated native tool turns retaining one member/report identity while withholding intermediate reports, deterministic frontier, replay-safe reports, fuel-to-mu, exact checkpoint-bound validator-witness and choice-authority ports, live bounded Rune T6 prover resolution, attributed exact-scope Aegis choice authority with SQLite reopen, optimistic multi-deliberation ownership, accepted-transition census recovery, mutation-tested canonical `MOIRADL2`/`MOIRADR2` bytes with prototype-codec refusal, independent public DTOs, host store port, authorization/revision/replay service conformance and capability-shaped clients, plus a disjoint canonical wire and authenticated finite-request daemon whose read-only TUI/CLI projection double-reads registry revision and displays exact personas, grounding, evidence/dissent, candidates and retained proof/choice/maybe/mu without recomputation; Daimon's selected OpenAI-compatible adapter now has one actual Ollama `gemma3:27b` completion with provider-reported usage and exact profile attribution, the grounded chatbot carries that adapter through its private model wire and actual Agora/Vellum peers under digest-bound zero-temperature sampling, the actual provider supplies four distinct closed reports for the unresolved Dendrite-grounded architecture ensemble, the resolved workflow removes two dominated alternatives, preserves two incomparable frontier survivors and requires distinct actual Aegis decisions for candidate choice and report action before peer effects, and an actual tool-capable provider turn remains intermediate until its authorized durable continuation returns a terminal closed report | Core semantic, authenticated Rune/Dendrite grounding, independent multi-turn model-member execution, closed model-report admission, governed local/operator composition, registry, checkpoint-codec, transport-neutral service/client/wire, read-only private-daemon terminal projection, real theorem-validator adapter, Aegis choice authorization, local durable-store slices, live provider-adapter smoke, combined chatbot first execution, unresolved and authorized live-provider workflow journeys, and actual-provider agent continuation satisfied; agent live-provider peer action and combined fault qualification remain open |
| M17 native tool continuation | only structured calls become effects; every call has separate authority, attempt, budget and receipt; uncertainty fences the tail | deterministic kernel allocates distinct call effects, retains the originating task key/incarnation/attempt and bounded provider-native call correlation, requires capability-bound Aegis-decision and an exactly matching current Chronos attempt before dispatch, binds capability/execution success and failure receipts including bounded output length, rejects duplicate model request identities before allocation, fences subsequent calls on unknown outcome, semantically validates recovery, roundtrips mutation-tested canonical bounded `MOIRATC3` bytes while refusing earlier reinterpretation, preserves in-flight and completed evidence through separately owned SQLite reopen, and exposes independent DTOs through an authorization-first service plus disjoint clients; live Aegis and Chronos public-client adapters produce the two independent gate facts; Daimon parses only provider-native calls, binds them to requested capabilities, preserves the opaque native identifier and exact continuation in additive wire and `DAIMONM4` checkpoint formats, and the Moira adapter projects calls with policy-owned budgets while withholding incomplete ensemble reports; the local runtime commits dispatch before one invocation, durably stores content-addressed output before success, reconciles lost store replies read-only, fences ambiguity, resolves all completed results in provider order, and submits them through Daimon's private native resume command; Daimon proves an exact call/result bijection and the OpenAI-compatible adapter emits matching assistant/tool messages; an ensemble continuation binds member, report and source execution across repeated native turns, and governed composition commits only the terminal closed response; the selected production process executor freezes a capability to canonical executable digest/fixed argv/cwd, invokes without a shell or inherited environment, concurrently drains bounded streams, enforces elapsed/input/output limits, attributes known failures and fences ambiguous post-spawn state; its real-process output is proven through durable dispatch and content-addressed storage; the bounded turn driver replay-admits multiple calls, obtains independent authority/current-attempt evidence per effect, advances strictly in provider order with restart-stable derived execution identities, reconciles in-flight output without reinvocation, and releases exact resume inputs only after completed durable state; the strict operator validates exact capability/profile/task/store topology without peer contact and runs a lazy-authority bounded resume loop through terminal report, failure, uncertainty or fuel exhaustion; the named live journey grounds the real Dendrite cohort, crosses authenticated Aegis, Chronos local wire and Daimon private sockets, executes the selected process, commits both tool stores and admits the terminal fourth report; an actual `qwen3:32b` profile now originates the native call, retains Ollama's additive call index safely, crosses those same authority/attempt/process/store boundaries and consumes the durable result through provider-native resume before terminal report admission | Core, canonical codecs, durable continuation/output stores, public service/client, native provider parsing and resume, live authority/attempt evidence, cross-engine admission/authorization, local dispatch, selected production process execution, private wire/daemon delivery, repeated result-to-model continuation, terminal report promotion, methodical operator turn-loop composition, live local tool journey and actual-provider native tool continuation satisfied; multi-process kill qualification and dynamic/secret-bearing tool profiles remain open |
| M18 selected peer-effect turn | only a policy-selected terminal result may project Agora/Vellum actions; deployment facts remain trusted inputs; uncertainty never falls through to fresh execution | the harness binds the exact retained checkpoint/member/report/output/Daimon effect to either a nonzero one-member chatbot policy or, only after `AuthorizedChoice`, a separate exact action-authority decision; proof/maybe/mu cannot authorize a member report to act; the Aegis public-client selector authorizes `Administer` on that deliberation and echoes the full selection subject, distinct from Pareto candidate choice; model-saga admission requires the derived nonzero digest, replay conflicts on selection drift, and canonical `MOIRAMP2` retains it while refusing the prior unbound identity; the selected-turn driver filters tool calls into their separate saga, requires a trusted Vellum extent resolver, executes in provider order, accepts only peer receipts, fences unknown outcomes and routes retained uncertainty solely to reconciliation; post-admission recovery reads the storage revision and retained saga directly from the service after SQLite reopen, never accepts a caller-provided view or reauthorizes, and performs no peer call when the saga is absent; the trusted router maps opaque conversation references to deployment-owned Agora room/revision/time/source/audience and opaque document references through Vellum's route catalogue; the selected exact-owner profile binds the Moira effect to Agora's actor operation and uses actor observation after reconnect, while Vellum uses lookup-only reconciliation; SQLite journeys prove digest retention, no re-execution after uncertainty and side-effect-free completed replay; a one-member chatbot journey crosses policy selection, `MOIRAMP2`, both SQLite owners and lost Agora reply recovery; a multi-member journey crosses distinct choice/action authority and Vellum lookup recovery; the exact-owner combined gate loses both peer replies in provider order, reopens the saga between them, observes one Agora actor operation and one durable Vellum edit, and completes offline replay without peer access | Action-selection, trusted routing, durable coordination, exact-owner actor/Vellum fault reconciliation, and offline completed replay are satisfied; top-level ingress/execution/event fault composition remains open |
| M19 product capability profiles | one engine core admits chatbots, workflows and agents without conflating participation shape, epistemic result, action permission or tool authority | the strict operator's closed product profile requires exactly one grounded member for a chatbot, at least two for a workflow, and at least two plus an advertised tool for an agent; chatbot peer action accepts only a nonzero single-member policy digest with no Aegis selector, while workflow/agent peer action accepts only `AuthorizedChoice` plus a distinct Aegis action selector; singleton orthogonality is the vacuous pairwise theorem pinned to Dendrite's live lattice identity rather than a fabricated lattice-check verdict; focused configuration tests cover valid and mismatched combinations; `qualify-methodical-profiles` runs the real Rune/Dendrite singleton chatbot, four-member workflow and Aegis/Chronos-gated durable tool agent through the same operator; `qualify-methodical-chatbot-peers` continues the top-level chatbot through actual Daimon, Agora and Vellum daemons; `qualify-methodical-chatbot-live-provider` substitutes an actual digest-bound `gemma3:27b` provider completion for the controlled chatbot service; `qualify-methodical-workflow-live-provider` obtains four distinct actual-provider reports but deliberately stops unresolved; `qualify-methodical-workflow-peer-live-provider` obtains distinct actual Aegis choice/action decisions before the selected actual-provider report crosses Agora/Vellum; `qualify-methodical-agent-live-provider` adds an actual tool-capable provider call and authorized durable continuation before the four-report unresolved state; the authorized ensemble peer journey supplies complementary offline-replay evidence | Explicit operator profiles, boundary validation, live grounded profile matrix, top-level chatbot peer execution, combined chatbot live-provider first execution, unresolved and authorized live-provider workflow journeys, and actual-provider agent continuation satisfied; agent live-provider peer action and combined top-level fault/replay injection remain open |
| M20 governed interactive agents | provider sessions, prompts, permissions and terminal reports remain subordinate to exact workflow/member, attempt, Rune method, Dendrite grounding, authority, revision and uncertainty evidence | Daimon's provider-neutral session protocol has canonical bounded wire/checkpoint recovery, cursor-ordered typed events, cooperative advance and no-redispatch ambiguity fencing; its canonical family has a dedicated authenticated private Unix endpoint and Moira's standard local harness projects that real socket into `AgentGateway` without exposing provider policy or credentials; Moira's public client now has a bounded participation Unix port and its one-step worker dispatches prepared work once, reconciles fences read-only, leaves ordinary idle state untouched and read-only inspects only a Ready resume-capable participation for detachment; its ACP v1 adapter has hermetic stream/permission/cancel/context gates plus a genuine Codex no-tool completion and fresh-process idle-session reattach with zero history replay, and its default-off client terminal port now has a production local-agent owner that admits only a canonical configured command/workspace/environment scope after fresh Aegis/Chronos validation, executes through a process-group boundary, and reopens durable terminal evidence without redispatch; its distinct Claude stream-JSON adapter has hermetic exact-launch/progress/context/permission/cancellation/uncertainty gates plus a genuine installed-provider no-prompt initialize/close gate; Moira's independent durable participation binds the exact methodical context, commits a one-way dispatch fence before one provider mutation, reconciles only by inspection, retains exact permission option membership and admits terminal reports only from matching completed EndTurn evidence; the public service and Unix profile derive permission execution/idempotency identities from a server-prepared `MOIRAPCF1` challenge, reject stale fingerprints before Aegis, and keep authority credentials adapter-private; a separate `MOIRACCF1` challenge binds cancellation to the exact active turn and authenticated mutation access while deriving the key server-side; bounded prompt submission commits exact bytes to a separately owned content-addressed store before Moira derives the digest-only operation's turn, execution, key and attempt from `MOIRAPSC1` full-context evidence, and the standard runtime materializer resolves only digest-verified retained bytes before dispatch; the strict one-step worker is qualified as an actual executable across both private sockets, including exact prompt materialization and cursor-bound durable output evidence; a hermetic mixed-provider journey runs actual ACP and Claude-stream worker processes through one Daimon service, completes durable open/prompt operations, retains distinct provider evidence and admits historian plus rigor-reviewer terminal responses into one affirmative orthogonal ensemble; cursor-ordered assistant chunks are durably composed across permission and advance slices with exact last-page replay, and report admission requires the matching participation, turn, final cursor, byte count and digest-verified output; those exact bytes pass the existing closed member-response parser while Moira derives report identity and baseline question/method/grounding/provider evidence, refuses malformed or host-colliding evidence before registry mutation, and preserves exact report replay; the terminal peer carries no operation identity, real bounded-socket CLI gates cover prompt, permission and cancellation, the reusable read-only participation component is qualified against the real daemon with no authority call, and a distinct read-only agent component double-reads stable sessions around bounded cursor pages, rejects gaps, labels truncation, preserves byte boundaries and traverses only server-derived cursors over one real daemon connection; separate prompt, cancellation and permission TUIs require arm then confirm and fence unknown mutation outcomes until refresh; permission additionally double-reads exact server challenges and verifies the complete cursor-contiguous agent stream before admitting only retained options, while prompt double-reads governed state and displays exact bounded content identity; synthetic and real-daemon gates prove all three controls, each reusing one participation connection so the bounded request loop cannot be starved; all terminal Unix client families invalidate uncertain streams and reconnect only for a later explicit command, with socket gates proving no prompt redispatch, component refresh recovery from a durable pending prompt, and a global request bound across agent reconnections | Core, persistence, public contract, canonical Daimon agent socket and Moira gateway composition, executable one-step worker, hermetic mixed-provider orthogonal ensemble, Codex ACP prompt, Claude no-prompt transport, production-governed ACP terminal execution, governed terminal prompt submission/materialization/output composition and structured report admission, terminal permission confirmation, terminal cancellation, read-only interactive agent session and participation views, and interactive prompt/permission/cancellation state machines satisfied; live Claude prompt behavior, Gemini session admission, and a live mixed-provider ensemble remain open |

M20 recovery evidence additionally drops committed cancellation and permission
replies beneath the reusable components. Both controls remain fenced until an
explicit reconnecting refresh observes the durable pending operation and no
second confirmation is sent. The permission gate crosses command-scoped
authorization exactly once and reopens the retained decision, subject,
capability digest, option, request, attempt, and epoch. Public permission and
cancellation challenge constructors now return no challenge whenever an
operation is pending.

The read-only M20 projection also has a real daemon lifecycle gate. Its initial
stable observation exhausts the advertised three-request connection; the
daemon then returns the same gateway owner, tears down its listener, and
rebinds the socket. No request is issued during the outage. The next explicit
refresh reconnects, repeats the stable double-read, and renders the exact prior
session/event snapshot. The gateway panics on any command other than inspection
and event reads, excluding a hidden mutation or inferred recovery transition.
The same read-only projection now renders all eight public capability facts,
including load, resume, mode and model support, while exposing no corresponding
provider command or direct Daimon dependency.
An independent real-daemon headless gate proves the same eight facts reach the
versioned `agent.session.inspect.v1` JSON envelope without omission.
The disjoint read-only result command now preserves Daimon's exact content-free
session/turn-to-execution/task-attempt terminal binding through Moira protocol,
wire, client and adapter layers. Canonical mutation tests and the adapter
projection test cover every field; unrelated `Inspect` and `Events` paths
explicitly refuse a result-shaped reply as invalid provider evidence.
The governed participation component now renders that complete projection too,
while retaining refresh and close as its only active controls.
Recovery mutation is isolated in a separate M20 control: the worker first
correlates an exact read-only `ReattachRequired` session and durably marks the
participation; `MOIRARCF1` then binds the retained Rune method, Dendrite
grounding, persona, profile/session revisions and cursor before Moira derives
the execution, key and attempt. The headless and reusable terminal controls
carry only this server confirmation, fence unknown outcomes, and exact replay
returns the already prepared operation. Canonical participation wire, SQLite
reopen, runtime dispatch and a real bounded-daemon CLI gate cover the path.
The `qualify-agent-reattach-recovery` aggregate additionally kills and reopens
the real Daimon executable around a live resume-capable ACP fixture, proves no
history appears before a new prompt, and runs an integrated Moira worker through
that reopened process from read-only detachment discovery to confirmed,
fenced, observed Ready. Focused executable and terminal gates independently
freeze the same public contract. The genuine pinned Codex 2.6.0 provider
restart also passes after one explicitly authorized no-tool seed turn, with
fresh-process resume and zero history replay. Simultaneous arbitrary-point
multi-process crash injection remains open.

The strict operator now contributes direct M16 evidence rather than stopping at
`ready_to_resolve`: its closed optional profile preserves Rune-witnessed proof,
Aegis-authorized Pareto choice, four-valued maybe and reasoned mu as distinct
terminal families. Resolution waits for the complete report set. Matching
SQLite replay is offline; terminal-family, candidate, Aegis subject/epoch or
explicit-value drift refuses. A local Unix qualification crosses the real Aegis
client/daemon once and proves matching operator replay does not reconnect.

Local peer-transport composition now has an Agora V2 Unix client port with a
canonical negotiated handshake, server-incarnation evidence, server-issued
client identity, bounded frame budget and write-phase `NotSent` versus
`OutcomeUnknown` classification. It is a separate package from the semantic
Agora adapter, preserving the public-client dependency boundary. Socket-pair
wire qualification plus an isolated actual Aegis-authorized Agora-daemon journey
are complete. The isolated methodical-peer gate additionally composes actual
Aegis, Agora and Vellum daemons through the operator seam and proves offline
completed replay. The exact-owner peer-effect gate now adds combined fault
injection through the reconnect-safe actor router. Agora engine/host/service
dependencies stay in the qualification lockfile rather than Moira's root graph.

Release assurance additionally binds the exact 130-component root lockfile to a
clean RustSec audit at the database revision recorded in
`docs/advisory-attestation.md`. CI audits the then-current database with warnings
denied. Deliberately separate qualification lockfiles remain outside this result;
it is known-advisory evidence, not source or legal review.

`make soak` separately runs 5,000 fresh closed-saga service lifecycles through
failed evidence, explicit fresh-identity retry, terminal completion and exact
admission replay. It is repeated in-memory service-mapping evidence only; SQLite,
daemon and peer-process soak remain separate. `make performance` runs 64
optimized samples for admission, point read, effect acceptance and retry under
the provisional reference-host p95/p99 budgets in `docs/performance.md`; it is
not SQLite, provider, peer-process or product-SLO evidence.
`make soak-process` adds 16 fresh Aegis/Moira daemon pairs with a fresh SQLite
saga store, public-client exact-saga authorization, attributed reconciliation,
durable operator-receipt readback and verified process/socket/path cleanup. It is
operator-segment evidence, not a soak of the complete peer chain.
`make soak-live-execution` adds 16 fresh six-process sets around the selected
durable Moira→Aegis/Chronos-gated Daimon→Hermes segment. Each set proves exact live authority
and running-attempt validation, discarded reply, Daimon daemon reopen, retained
result reauthorization, one create-once provider marker, one completed Moira
receipt, Moira daemon/SQLite reopen and result acceptance, one canonical
Moira-owned Hermes event and clean endpoint teardown. It
is not Agora/full-workflow or latency evidence.

`make check-physical-integrity` proves both canonical Moira SQLite profiles
refuse synchronized structural damage to a non-header database page without
rewriting or reinitializing the file. Every current SQLite receiver runs the same
bounded `quick_check(1)` before trusting existing state. This is structural-page
evidence, not arbitrary corruption, rollback, torn-write or device certification.

## Hermes workflow-event adapter

`moira-hermes` owns the bounded canonical `MOIRAWF1` encoding for Moira's closed
legacy transition family. Callers supply a stable workflow-event identity, exact
Hermes predecessor and configured producer/schema; the adapter rejects a
workflow/correlation mismatch before dispatch and maps identity/correlation into
Hermes without treating either as authority. Its normal dependency closure is
only `moira-protocol`, `hermes-client` and `hermes-protocol`. A real receipt-gated
Hermes service test drops the first append reply, retries the same public-client
request and observes `Replayed` plus exactly one retained event.
`make qualify-hermes-process` strengthens this across actual Hermes process
exit/reopen: public wire readback proves exactly one canonical event at the
original sequence after the discarded reply.
CI runs this together with the isolated Aegis process and Chronos recovery
profiles through `make check-peer-profiles`.

## Agora response-publication adapter

`moira-agora` retains its Agora V2 `SpeakIdentified` compatibility adapter and now
also maps a bounded response intent to Agora's actor-owned publication profile.
The actor profile keys the exact room/revision/time/body/audience intent by
Moira's stable effect identity rather than a connection-issued client namespace.
Accepted Agora revision/message sequence becomes a Moira `PublicationReceiptV1`, while the
Agora message ID remains available as peer evidence. An independent public-contract
port drops the first reply, rejects effect rebinding, replays the exact request and
observes one identified message without importing Agora engine/service/wire types.
The actor adapter additionally exposes historical observation where absence never
falls through to mutation and retains no Agora session or request identity.
Explicit abandonment on the compatibility adapter returns both identities without
claiming non-publication.
The client and publisher now export and validate their exact request namespaces
for restart: only the immediately allocated pending request and its matching
Moira intent can be restored. The separately locked
`make qualify-agora-recovery` profile runs against Agora's real durable Service
V2, discards the first committed reply, recreates both owners, rejects effect
rebinding and resolves the exact request without another store commit or second
message. The isolated `make qualify-agora-local-port` profile now also starts a
real durable Agora daemon in an independent process, commits one addressed actor
publication, discards its successful reply at Moira's boundary, and observes the
exact retained result through a new authenticated public client. This closes the
Agora independent-process lost-reply federation gate. The same named gate now
builds and launches the exact `aegis-daemon` and `agora-private-local` binaries,
issues a current bounded scheduled-composition grant, repeats committed-reply
loss and fresh-client observation, drains Agora through its stdin lifecycle
contract and then terminates Aegis cleanly. The combined Agora→Vellum and every-
effect fault matrix now runs as `make qualify-peer-effect-process`: one retained
Moira saga observes the actor-owned Agora operation and looks up the Vellum
receipt after separate lost replies and SQLite reopens. The wider top-level
ingress/execution/event matrix remains open.

## Vellum durable-edit adapter

`moira-vellum` maps a separately allocated Moira effect and trusted document
route to Vellum's public durable replace-and-save operation. The intent binds the
exact source revision, retained extent and replacement; lookup is historical only
and absence never falls through to execution. `make qualify-vellum-process`
builds and starts Vellum's exact standalone authenticated daemon binary in an
independent process, establishes a normal saved baseline through the public
client, discards the accepted durable edit reply, and resolves the exact receipt
through a fresh public client. The source contains the replacement once and the
owner then drains through its public control socket. This closes the isolated
Vellum process boundary with the same entrypoint used by standalone composition;
composing it with Agora and injecting faults at every effect boundary remains
open.

## Chronos task-admission adapter

`moira-chronos` prepares one minimized task through Chronos' public V3 atomic
invocation contract. The server-issued invocation serial is exportable with the
Moira intent; the invocation key is canonically derived from the effect ID. An
execute transport failure is explicitly outcome-unknown and can close only by V3
lookup, followed by V4 original-allocation lookup on the same host-bound service.
The retained singleton allocation supplies the exact original task incarnation
and accepted Chronos revision for `AdmissionReceiptV1`; current-name snapshots
are deliberately not evidence. A receipt-gated AtomicService test drops the
execute reply, reopens service state, restores the prepared record and proves V3
lookup plus V4 allocation returns one task with zero mutation replay. A separate
case proves an applied legacy invocation without captured allocation returns
`HistoryLost` rather than guessing a receipt.
That real-service fixture is isolated behind `make qualify-chronos-recovery` so
Chronos's vendored storage graph is never resolved together with Aegis's shared
Keystone storage graph in Moira's root lockfile.
The same isolated profile now also drives Chronos through its owner-only public
Unix transport in two helper processes: it discards the execute reply, reopens
the durable service, retires the original task, reuses its descriptive name and
still reconciles the original incarnation/revision through V3/V4 read-only
lookups without mutation replay.

## Chronos trusted-worker adapter

`moira-chronos-worker` is a deliberately separate trusted in-process composition
over Chronos's typed host profile; it is not a public completion API and does not
weaken `moira-chronos`'s public-client dependency closure. It owns the one
non-cloneable worker handle, keeps native grants and leases private, issues only
an authority-free exact-attempt receipt, and binds settlement to the matching
Moira effect, execution, task incarnation and attempt. Unsupported dispositions
refuse before an owner call. Transport or withheld-result ambiguity fences the
adapter rather than guessing whether mutation occurred. The isolated
`make qualify-chronos-recovery` profile crosses the real standalone Chronos
owner and proves exact completion, evidence mismatch without settlement,
unsupported failure without mutation, uncertain fencing without reclaim, and
invalid configuration without dispatch. Heartbeat and lease renewal, remaining
result dispositions, adapter restart recovery and Pantheon runtime composition
remain open.

## Task-agent journey admission

`moira_core::task_agent_journey` owns the first J1 admission aggregate rather
than asking Pantheon to synthesize workflow identities from profile state. A
bounded immutable template binds work mode, Daimon profile, room policy,
budgets, grounding/authority policy and human-visible consequence. Preview
admission retains the exact Chronos task incarnation/revision evidence plus
objective, acceptance, initiating principal and accountable human. Instantiate
requires the exact owner revision, unexpired preview, trusted current time and
matching named-human consequence confirmation; Moira then allocates one
owner-scoped journey/workflow identity. Matching operations replay, changed
operations conflict and a second operation cannot instantiate the same preview.
The canonical `MOIRATJ1` codec revalidates the full checkpoint and refuses
foreign ownership, noncanonical ordering, truncation, tails, mutation and host
limit violations. Six focused domain/codec tests and strict Clippy pass. This
is now available through independent public DTOs and a disjoint bounded wire
family covering every command, outcome and refusal plus truncation, tail and
cross-family rejection. Client requests carry only a bounded requested TTL and
human confirmation content; owner-issued observation, expiry and confirmation
times appear only in retained views. Transactional storage, service/time port,
Unix client/daemon, trusted peer progression and Pantheon composition remain
open.

## Daimon execution adapter

`moira-daimon` maps a Moira-owned effect and exact Chronos task attempt into
Daimon's public execute request without importing its harness, host, service,
provider or storage types in production. Daimon retains current authority and
attempt validation. Exact completed dispositions map back to the original Moira
causation; an outcome-unknown fence maps to an attributable uncertain receipt.
A real in-process Daimon service journey proves matching replay returns the same
receipt and calls its selected provider once. `make qualify-daimon-process`
strengthens this over Daimon's public wire and private daemon: the first reply is
discarded, the daemon exits, a fresh process reopens the canonical local store and
returns the exact completed Moira receipt. An external create-once marker proves
the provider was not redispatched.
`make qualify-live-execution-chain` separately builds sovereign Aegis and Chronos
helpers, then runs the harness with independent Moira, Daimon, Aegis, Chronos and
Hermes processes. The Moira daemon durably admits the workflow and prepares the
execution, exits, then reopens the same SQLite owner to accept the result. Daimon uses
the live Aegis session for exact profile/tool `Execute` decisions and the public
Chronos snapshot for the exact running incarnation/attempt before its create-once
provider marker can be written. Its recovery case discards the reply, exits the
first Daimon daemon, reopens the store in a fresh daemon and reauthorizes exact
retained delivery through the same live Aegis authority. Chronos and the provider
are not called again. The same Aegis daemon grants exact-stream Hermes
`Submit`/`Observe`; Moira then publishes the completed transition through the
public Hermes client/wire and reads back exactly one canonical event. Only public
clients and wire values cross process boundaries.

## Native service separation

`moira-host::CoordinatorStore` isolates durable coordinator ownership from the
application service. `moira-sqlite` implements it without exporting its engine;
domain refusals remain typed while storage uncertainty remains a backend failure.
It also owns the closed saga stores and reusable local ownership, integrity,
timing and usage mechanisms, but no peer engine dependency. Agora's local durable
projection is owned by `moira-agora-sqlite`; settlement-coupled Chronos and Daimon
projection is owned by `moira-execution-sqlite`. Their source and behavioral tests
live with those packages, and CI freezes all three direct production closures.
`moira-protocol` owns mutation/status/workflow requests and replies, and the
client exposes disjoint mutation and read-only facades. Service tests prove exact
revision refusal and replay; a real SQLite reopen journey proves that recovered
read models and replay classification survive the new boundary. The protocol
DTOs are representation-independent and `moira-wire` covers every command,
transition and read-model shape with truncation, trailing-data, bound and
redaction tests. Deterministic single-bit flips, deletion at every byte and
`0xff` insertion at every boundary must either be refused or decode to a value
whose canonical re-encoding is byte-exact; the corpus covers both protocol
families and readiness.

## Private local daemon

`moira-local-daemon` authenticates each accepted connection into explicit
read/mutate facts and refuses unauthorized operations before storage dispatch.
The selected `moira-unix-peer` adapter obtains effective UID/GID from the kernel,
performs a bounded exact mapping to those facts and refuses unknown peers before
readiness. Real-socket tests cover both daemon families and unknown-peer refusal.
The separate saga profile additionally asks its authentication adapter for
command-scoped operator facts; denial is a typed reply without store dispatch.
The canonical ready frame negotiates finite frame/request ceilings; connection
tests distinguish pre-dispatch failures from reply loss after execution. A real
mode-`0600` socket journey performs durable admission and inspection, then reopens
both daemon and SQLite and observes exact replay without another workflow. The
saga socket journey admits and fences work, authorizes reconciliation, exports
its subject-free receipt, reopens daemon and SQLite, exactly replays the stale
request and proves denial leaves revision unchanged. Health and socket cleanup
are bounded and content-free. A separate two-process journey runs real Aegis and
Moira daemons: Moira's authenticator obtains an exact saga `Administer` grant over
the public Aegis client/wire, validates reconciliation live, and persists the
returned decision and epoch in the operator receipt observed through the public
Moira client/wire. The UID/GID registry never supplies operator facts; the saga
adapter delegates that decision through a separate `OperatorAuthorizer`. The
`moira-aegis` implementation uses explicit time, exact-saga `Administer` scope,
typed denial and current decision attribution against a real Aegis service.
The selected closed saga profile is now packaged as `moira-saga-local`; its
standalone and Workbench-supervised process journeys qualify public-client
mutation, clean socket drain and durable reopen. Its default operator profile
denies authority; the separately selected production Aegis profile owns the
public-client session, short-lived exact-saga grant and monotonic time. The
cross-shell process gate qualifies exact confirmation, selected-saga refusal,
attributed durable receipt recovery and hosted reopen without exporting those
authority inputs to Workbench. Remote identity and process packaging for the
remaining service profiles remain open.

## Saga and timer profile

`moira_core::saga` closes one bounded forward/terminal/compensation family with
caller time, stable timer intent, declared misfire behavior, fresh explicit retry
identities and typed uncertainty. Cancellation or deadline policy after known
forward success prepares attributable compensation rather than claiming rollback.
The independent 36-program model reconstructs after every transition and replays
every accepted command as a lost reply. A distinct saga frame family exhaustively
covers the closed protocol without accepting legacy workflow frames. Stable
bounded inspection pages
and a content-free phase census expose runnable, decision-blocked, fenced and
terminal populations without embedding scheduling, priority or clock policy.
Canonical bounded saga checkpoint bytes feed a separately identified SQLite
profile; exact evidence and census survive reopen, while profile drift and legacy
coordinator databases are refused rather than silently adopted.

The separate `SagaStore` host port, representation-independent saga DTO family,
exhaustive service mapping and capability-shaped clients cover mutations, typed
refusals, point reads, bounded pages and census. Authentication-derived access is
checked before durable dispatch, and the native service's exact replay/read model
survives a real SQLite reopen. The canonical saga wire/local-daemon profile uses
separate frame kinds, bounded page allocation and command-scoped operator
authorization; it is not implied by or interchangeable with legacy transport.

The saga SQLite profile also has a deterministic logical mutation corpus over
owner, bound limits, initialization correspondence, domain revision, checkpoint
payload, application ID, schema version and unexpected schema objects. Every
altered store refuses reopen instead of creating replacement state. This is not
a claim that stock SQLite detects every physical page or storage-medium bit
corruption; selecting such an integrity profile remains open.

Digest-pinned external checkpoint and logical SQL fixtures cover one live legacy
workflow and one live saga with failed evidence plus explicit retry. The SQLite
fixtures reconstruct the frozen V1 schemas independently of the active creation
path; current stores must reopen them and replay their retained histories exactly.
They intentionally do not freeze SQLite's physical page layout.

The selected local operational profile qualifies both stopped copy/restore and
bounded online snapshots for both SQLite owners. Online-backup tests retain the
owner connection, write a private partial database under finite page/step limits,
verify integrity and source identity, atomically publish, then prove the source
can progress while the snapshot reopens at its exact earlier revision and replay.
Post-rename parent-sync failure is a distinct publication-uncertain result that
requires destination inspection before retry.
Concurrent writer/load qualification, installation packaging and future-version
migration remain open.

Saga reconciliation reads expose a bounded snapshot, per-saga last-confirmed
revision, retained-evidence count and pages, explicit effect/deadline fence cause,
and whether automatic progress is runnable, awaiting a decision, uncertainty
blocked or terminal. A real SQLite reopen preserves the uncertain evidence and
all of those classifications. Inspection alone carries no reconciliation
authority.

Operator reconciliation receipts have content-safe point and action-ordered
bounded-page exports. They retain the original Aegis decision/epoch,
disposition, resulting phase and prepared effect, but omit authenticated subject
and grant material. Recovery tests cover every disposition and a real SQLite
reopen preserves the exported receipt without granting reconciliation authority.

The closed operator command family now covers evidence adjudication,
keep-uncertain, fresh-identity retry, compensation, cancellation and terminal
failure without a force-success path. Bounded receipts retain opaque action ID,
subject and Aegis decision/epoch attribution and replay before stale-revision
refusal after restart. Ordinary mutate access is refused before dispatch; the
separate service entry point injects verified authority facts. The public-client
Aegis adapter scopes `Administer` to the stable saga resource, keeps session-local
grant handles private, and injects only the authorized decision facts. A live
Aegis service journey proves allow, deny-without-mutation, SQLite reopen and
reauthorized exact replay.

