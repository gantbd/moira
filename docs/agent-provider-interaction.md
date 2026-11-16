# Agent-provider interaction

Status: architecture and delivery plan. Daimon's provider-neutral interactive
agent protocol, host ports, capability-shaped clients, and volatile
authority/attempt-gated open/prompt/permission reference service are
implemented. Daimon's bounded ACP v1 stdio adapter, distinct Claude Agent SDK
stream-JSON adapter, and Moira's independent agent protocol/service/client/wire
projection are implemented. The terminal peer has read-only session, event,
and durable-participation commands plus a reusable interactive
agent-session/event component. Moira's
governed participation record, Aegis-backed permission-selection boundary,
one-way dispatch fence, and inspect-only recovery path are implemented.
The peer also has a read-only permission-confirmation view that correlates a
stable participation record with complete cursor-contiguous provider events and
emits a domain-separated server challenge. Its explicit confirmation command
submits only the challenge and retained option; Moira revalidates durable state,
derives replay-stable execution identities, and obtains command-scoped Aegis
authority without exposing credentials to the terminal.
Cancellation uses a separate `MOIRACCF1` challenge bound to the same methodical
context plus the exact active turn. Moira derives its idempotency key and
requires authenticated mutation access; the terminal supplies neither turn nor
key.
Governed prompt submission is also implemented. The terminal submits only a
participation and bounded bytes. Moira durably commits the exact content before
preparing a digest-only operation and derives its turn, execution, idempotency
key, and retained attempt from the full current method/grounding/persona
context. The participation daemon exclusively owns prompt storage. Its
default-deny worker command materializes only the exact current
participation/revision/digest; the runtime verifies the digest again immediately
before provider command construction. Missing, corrupt, stale, or unavailable
content fails before dispatch. The exact terminal report-admission gate is implemented.
Cursor-contiguous assistant chunks are now accumulated in a separately owned
turn-output store across permission and advance slices. Exact last-page replay
is idempotent, gaps and forks refuse, and the terminal digest is resolved from
those retained bytes when the provider omits it. Report admission requires the
same participation, turn, final cursor, byte count, and digest before touching
the deliberation registry. Those exact retained bytes are parsed with Daimon's
closed `moira.member-response.v1` contract. Moira derives the report identity
and baseline question, Rune method, Dendrite grounding, and provider evidence;
provider evidence colliding with those host-derived facts is refused.
The reusable read-only participation TUI is qualified against the real bounded
daemon; it renders the complete eight-fact capability projection and exposes
refresh and close only. A separate cancellation component
double-reads the server challenge, requires arm then confirm, submits no turn or
key, and fences an unknown mutation result until read-only refresh. Interactive
permission similarly reuses the complete cursor-correlated event verifier,
selects only retained options, requires arm then confirm, and fences an unknown
result. Interactive prompt renders a stable governed-state and exact bounded
content identity before the same arm/confirm and uncertainty fence. The
real-daemon gates prove each control over a single reused participation
connection, with permission additionally using one agent connection.
The distinct read-only agent component double-reads the provider-neutral
session around one bounded event page, refuses cursor gaps or a changing
snapshot, labels truncation, traverses only server-derived cursor pages, escapes
byte previews, and displays the complete public capability projection including
load, resume, modes and models without direct provider access. It passes a
one-connection real agent-daemon gate. A restart
gate consumes that connection's exact three-request allowance, transfers the
same gateway owner through daemon shutdown and socket rebind, and requires an
explicit component refresh before reproducing the identical session/event
snapshot. The gateway accepts only inspection and event reads, so the journey
cannot manufacture a transition while the daemon is absent.
The headless `agent-inspect` path separately crosses the real daemon and emits
all eight capability facts unchanged in its versioned JSON observation.
A fault-injected terminal gate commits one prompt and its content, loses the
Unix reply, reports a non-retry-safe unknown outcome, and proves that the exact
subsequent submission is recognized as replay rather than a second logical
operation.
The sibling terminal client's saga, agent, participation, and deliberation
ports now share the same bounded connection law: transport or codec uncertainty
invalidates the stream, the uncertain command is never retried, and only a
later explicit command reconnects. A socket gate loses a prompt-mutation reply
and observes governed participation on the fresh connection; another exhausts
two one-request agent connections without resetting the client's global request
budget. The reusable prompt component now crosses that same fault: it fences
arm/confirm after the lost reply and only an explicit refresh reconnects to the
durably pending operation, with no second prompt request.
Cancellation and permission components now cross the same fault boundary. Each
uses one exact server challenge, commits one derived pending operation, loses
the reply, fences another confirmation, and reconnects only for explicit
read-only refresh. The permission journey performs exactly one command-scoped
authorization and verifies its decision, subject, capability digest, and epoch
in durable state. Public permission and cancellation challenge constructors
return no challenge while any operation is pending, so recovery cannot present
retained provider evidence as a fresh choice.
The terminal peer also reaches Moira's methodical deliberation through a
separate canonical frame family and read-only private daemon. Its headless and
reusable interactive views double-read the registry revision and preserve exact
persona, grounding, report evidence/dissent, candidates and retained resolution.
They do not recompute Pareto membership or turn a displayed authority reference
into permission.

## Decision

Moira coordinates third-party coding agents through Daimon. Human presentations
never launch or speak directly to Codex, Claude, Gemini, or another provider.

```text
moira-tui / moira-tui-cli
            |
       moira-client
            |
      moira-protocol
            |
       Moira service
            |
       daimon-client
            |
 dedicated private agent socket
            |
  Daimon agent-session service
            |
        provider adapters
        /               \
 ACP client adapter   Claude Agent stream-JSON
        |               |
 Codex · Gemini ·      Claude Code
 other ACP agents
```

This preserves the shared component architecture:

- Moira owns workflow, deliberation, participant selection, durable effect
  identity, and report admission.
- Daimon owns already-authorized provider execution, provider-session evidence,
  budgets, cancellation, uncertainty, and replay fencing.
- Aegis owns permission to execute a profile or approve a requested capability.
- Chronos owns the exact current task attempt.
- Dendrite supplies provider-neutral, digest-verified ambient expertise.
- The terminal peer owns only one identified client's bounded presentation and
  pending confirmation state.

Provider text, agreement, an ACP permission request, and a UI selection are not
authority. Each must cross its owning typed boundary.

## Why ACP first

ACP is the common local agent boundary already exercised by Axon. Its prior art
covers persistent and ephemeral sessions, bidirectional JSON-RPC over stdio,
permission mediation, cancellation, filesystem and terminal capability
attenuation, session-bound configuration, and content-safe event separation.

Selected provider paths include:

- Codex through `@agentclientprotocol/codex-acp`;
- Gemini CLI through its native `--acp` mode;
- additional agents through explicit, locally configured ACP profiles.
- Claude Code through its official bidirectional Agent SDK stream-JSON/control
  transport when no qualified ACP bridge is installed.

The official ACP Rust SDK and compatibility test kit should be preferred over a
new handwritten protocol implementation. Axon's implementation is behavioral
prior art, not a code dependency and not an alternate authority.

Vendor-specific transports remain separate Daimon adapters when ACP is
unavailable or the task is a terminal model completion rather than an
interactive agent session. Provider selection is by frozen wire profile, never
by a vendor-shaped branch in Moira. Claude's stream-JSON discriminator therefore
cannot be decoded or persisted as ACP.

## Additive Daimon contract

ACP sessions do not fit the existing terminal model `Execute`/`Resume` family
without losing important facts. Daimon should gain a distinct, additive
agent-session protocol family with closed commands such as:

- open a session from a frozen profile and exact workspace/context binding;
- submit one prompt turn under an exact Chronos attempt and budget;
- inspect bounded ordered updates after an opaque cursor;
- answer one exact permission or elicitation request;
- cancel one active turn;
- inspect, resume when advertised, and close a session.

The result algebra must keep completed, failed, cancelled, timed out,
disconnected, and outcome-unknown distinct. Thought, plan, tool, terminal,
permission, file-change, usage, and assistant-message events remain separate
typed updates; only a terminal assistant result may become a Moira member-report
candidate.

The frozen non-secret profile binds:

- ACP protocol and negotiated capability version;
- exact argv and executable identity or an administrator-provisioned profile
  reference;
- workspace identity and cwd policy;
- sandbox mode and inherited-environment allowlist;
- independent filesystem-read, filesystem-write, and terminal scopes;
- permission policy and timeout;
- model/mode preferences only when the agent advertises them;
- event, elapsed-time, context, output, and tool-call budgets.

Credentials and login state remain host-owned and never enter checkpoints,
diagnostics, profile digests, or Moira protocol values.

## Moira and terminal-peer projection

Moira retains an agent participation record that binds one workflow or
deliberation member to the exact Daimon session/profile, task attempt, method,
grounding, persona lens, terminal result, and provider evidence. Prepared
operations include their exact execution and idempotency identities. The local
driver durably enters `FencedUnknown` before its one provider mutation; only an
exact session plus complete cursor-contiguous event observation clears that
fence. Recovery has only `Inspect` and `Events` and cannot redispatch. A
provider session may contribute a report or proposal; it cannot resolve a
frontier or authorize its own action.

The existing `moira-tui` peer remains reusable and read-only by default. Extend
it only as Moira exposes public typed projections:

1. census, evidence, operator-receipt, deliberation, and provider-session views;
2. bounded prompt submission with an exact workflow/member/profile binding;
3. cursor-based update observation and explicit cancellation;
4. confirmation cards for exact Aegis-backed permission requests;
5. **Implemented:** persona, evidence, dissent, retained Pareto-frontier, and
   resolution views through the independent deliberation wire/daemon/client;
6. authorized reconciliation and report-action commands with durable receipts.

Headless CLI commands emit versioned JSON observations and decisions without
constructing TUI state. Interactive and headless modes use the same
`moira-client` semantic contract. The component never initializes the terminal,
parses process arguments, exits the host process, stores credentials, or gains
ambient provider authority.

## Delivery slices

1. **Implemented:** define provider-neutral agent-session requirements,
   protocol values, limits, uncertainty laws, and an independent canonical
   bounded command/reply/event codec in Daimon.
2. **Implemented reference slice:** in-memory fake ACP provider plus an
   authority/attempt-gated volatile service for session open, prompt, exact
   permission decisions, typed events, cursor inspection, exact active-turn
   cancellation, idle close, and replay without redispatch.
3. **Reference adapter implemented:** ACP v1 stdio now has explicit process
   ownership, exact correlated permissions, cancellation, bounded progress,
   verified-context projection, capability-aware close, and no implicit retry. The
   adapter must use cooperative stream advancement (or an equivalent durable
   background projection): a blocking prompt call cannot prevent the same
   session from accepting `session/cancel` or permission decisions. Daimon's
   provider-neutral `Advance` command, durable fence, canonical wire/checkpoint
   encoding, and client facade are implemented. The ACP client filesystem-read
   and terminal callbacks are default-off host ports. Daimon's production local
   agent now enables them only for canonical workspace scopes and explicit
   granted capabilities; terminal creation additionally crosses a canonical
   absolute command registry, approved argv prefix, cwd/environment policy,
   fresh Aegis/Chronos checks, and the process-group boundary. Genuine
   Codex gates pass initialize, session creation, one bounded no-tool prompt,
   completed terminal evidence, and close; Gemini initializes ACP but is
   account-blocked before session creation on the selected host; no Claude ACP
   bridge is installed there. The separate `daimon-claude` adapter now passes
   exact launch/environment/workspace, initialize, bounded progress, verified
   context, one-shot permission, cancellation (including permission-waiting),
   uncertainty and graceful-close fixture gates. Installed Claude Code `2.1.311`
   also passes a genuine initialize/close gate with no user message or model turn.
   Daimon's bounded profile router now runs ACP and Claude fixture processes as
   concurrent sessions without transport fallthrough. Its `daimon-local-agent`
   application derives launch identity, issues an exact session-local Aegis
   profile/tool grant, validates Chronos attempts, owns the durable checkpoint
   and authenticated agent socket, and passes an executable open/prompt/close
   plus process-restart recovery journeys for both governed reads and governed
   terminal evidence. Provider-specific live qualification, Linux isolation and
   deployment policy review remain open.
4. **Implemented:** Daimon agent sessions retain exact preparation attribution,
   operation fences, events and results across a generic durable store with
   no-redispatch reopen semantics, an independent canonical checkpoint, and
   exclusive local private-directory composition.
5. **Permission slice implemented:** the public selection contains no authority
   credential. A command-scoped Aegis adapter authorizes `Execute` for a
   domain-separated `(participation, execution, request, option, capability)` resource
   and returns attribution only. The participation engine independently checks
   the claimed digest and retained exact attempt. Daimon's launch-profile
   composition is implemented; hardened deployment packaging remains open.
6. **Public projection implemented:** independent Moira DTOs, disjoint clients,
   authorization-first service, Daimon public-client adapter, canonical bounded
   wire, Unix daemon profile, and read-only CLI session/event observation.
   Daimon's underlying canonical agent family now also has its own authenticated
   private Unix endpoint and matching client port. `moira-local-harness` exposes
   the standard connection that projects that socket into Moira's `AgentGateway`;
   a real-socket gate proves the independent command/refusal mapping without
   exposing provider policy or credentials. `moira-agent-local` now supplies a
   strict offline-checkable, one-step lifecycle composition. `moira-client` additionally owns a
   bounded optional Unix participation port for non-presentation workers, and
   the local harness provides one `drive_current` step that dispatches prepared
   work once, reconciles fenced work read-only, and performs one read-only
   Daimon inspection only for a Ready resume-capable participation. Ordinary
   idle state remains untouched. Its fixed production template derives all mutable
   identities from that retained record; deployment supplies only bounded
   principal/workspace/context/tool/budget policy.
   Prompt bytes remain behind the participation daemon's exclusive store owner:
   a default-deny worker command requires the exact current participation,
   revision and digest, and the worker reconnects before the dispatch currency
   check and fence. The process never opens prompt SQLite directly.
7. **Durable participation implemented:** canonical checkpoint, exclusive
   SQLite owner, public service/client/wire, private Unix daemon, exact
   permission authorization, pre-dispatch fence, no-redispatch recovery, and
   read-only `participation-inspect` CLI projection plus a stable, byte-preserving
   permission-confirmation view and explicit server-confirmed selection command.
   An exact terminal
   report-admission gate now requires a completed EndTurn observation plus
   matching member, Rune method, persona, Dendrite grounding, output, provider
   evidence, and grounding evidence before report storage. A separately owned,
   content-addressed prompt store plus the `SubmitPrompt` command enforce
   durable bytes before a server-derived digest-only pending operation; the
   terminal carries no turn, execution, key, attempt, or provider authority.
   The daemon-backed worker materialization command and the retained runtime
   materializer independently verify the digest before giving bytes to the
   deployment-owned budget/context/tool template. Ordinary terminal
   authentication is denied this command by default.
   A real executable-level gate launches `moira-agent-local`, crosses both
   private sockets, fences exactly once, and commits the observed transition. A
   second prompt journey proves daemon-only materialization, exact provider
   prompt bytes, cursor-contiguous assistant-output storage, and matching
   terminal evidence.
   A mixed-provider executable journey now runs distinct ACP historian and
   Claude-stream rigor-reviewer workers through one provider-neutral Daimon
   service. Each completes durable open and prompt operations, retains distinct
   provider evidence, and admits its closed response into the same affirmative
   orthogonal Moira ensemble. The provider processes in this gate are hermetic;
   no live-provider behavior is inferred from it.
   A separate cursor-ordered output accumulator durably composes assistant
   chunks across provider slices; report admission requires its exact final
   cursor and digest-verified bytes rather than trusting a report DTO.
   The gate parses those bytes with the existing closed member-response schema,
   derives report identity and baseline evidence server-side, and replay-admits
   only the same report without advancing registry revision.
   A reusable read-only participation component is qualified against the real
   daemon without any authority call. The separate cancellation TUI has a
   closed challenge/arm/confirm state machine and synthetic public-port gate;
   permission has the same closed state machine over complete correlated events,
   and prompt submission over exact bounded bytes. Real-daemon gates prove all
   three controls without a second participation connection starving mutation.
8. Qualify Codex, Claude, and Gemini independently, then qualify one mixed
   orthogonal ensemble. Codex ACP 2.6.0 passes upstream `acp-tck` 0.3.0 (all 32
   mandatory tests and all 19 exercised capabilities), Daimon's lifecycle and
   no-tool prompt gates, and reject-only permission gates. The older 1.8.0
   evidence remains historical and is not upstream-TCK conformant because it
   fails delete and resume requirements. Claude has genuine
   no-prompt transport qualification on its distinct wire and a compiled
   explicit-spend prompt gate. Gemini 0.68.0 remains externally blocked before
   session creation on the current host. Executing the Claude prompt gate and
   a live, productively orthogonal Moira ensemble remain open. Hermetic
   qualification now covers both mixed ACP/Claude transport composition and
   the complete two-worker Moira participation-to-report ensemble path. Each
   gate records exact advertised capabilities rather than claiming
   cross-provider feature parity.
9. **Partially implemented:** both ACP and Claude process gates classify
   post-dispatch provider exit as outcome-unknown, never relaunch on `Advance`,
   and refuse stale permission request/option identities without consuming the
   retained valid challenge. Both adapters also clear the child environment
   before copying an exact name allowlist and redact launch arguments and those
   names from diagnostics. Their completion-before-cancel race gates preserve
   the true completed disposition or outcome-unknown instead of manufacturing
   cancellation evidence. A shared digest-bound macOS Seatbelt launch path now
   carries both adapters through negotiation under a deny-default fixture
   policy, and a separate enforcement gate proves file mutation is denied. The
   isolation kind and policy digest are part of Daimon's frozen launch identity.
   A separate pipe-mode boundary now owns terminal process groups, bounded
   backpressured output, owner-loss cleanup and direct-child reaping; the
   production governor admits only canonical configured commands after fresh
   authority/attempt validation. Linux provider isolation and deployment-specific
   policy review remain open. Daimon's canonical checkpoint now retains a
   bounded provider resume locator only for an idle resume-capable session,
   restores it as `ReattachRequired`, and requires an explicit revisioned
   mutation with fresh exact authority/attempt admission. The ACP adapter uses
   `session/resume` in a fresh child without `session/load`, history replay, or
   an implicit prompt; an active-turn loss remains outcome-unknown. Moira maps
   that command and phase through an independent DTO, client, authorization
   boundary, canonical wire, and Daimon adapter. A read-only worker inspection
   durably marks the exact detached participation. Moira then derives a
   domain-separated reattach challenge from its retained Rune method, Dendrite
   grounding, persona, profile evidence, attempt and revisions; the terminal's
   headless and arm/confirm controls return only that challenge and never carry
   operation identities, authority, attempt or provider-locator material.
   Exact confirmation replay and uncertainty fencing are qualified through the
   bounded participation daemon. `make qualify-agent-reattach-recovery` joins
   independently owned gates plus one integrated journey: a real Daimon
   executable and live ACP child restart with zero history replay, an actual
   Moira worker driving that reopened process through governed confirmation to
   Ready, the focused Moira executable mark/confirm/fence/observe path, and the
   bounded terminal confirmation CLI.
   A genuine upstream-provider restart and simultaneous multi-process crash
   injection remain open.

## Explicit non-claims

- ACP compatibility does not imply identical provider behavior or tools.
- Provider-native tool updates and permission choices do not imply access to
  Daimon's default-off ACP client filesystem-read or terminal callbacks; those
  require separate configured scopes and fresh host authority.
- Registry presence does not authorize installation or execution.
- Authentication does not grant filesystem, terminal, network, or workflow
  authority.
- Session resume support is capability-negotiated, never assumed.
- A provider-generated plan is evidence, not Moira's governing Rune method.
- The TUI is not a provider sandbox, credential broker, or workflow authority.
