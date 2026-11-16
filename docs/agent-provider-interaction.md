# Agent-provider interaction

Status: architecture and delivery plan. Daimon's provider-neutral interactive
agent protocol, host ports, capability-shaped clients, and volatile
authority/attempt-gated open/prompt/permission reference service are
implemented. Daimon's bounded ACP v1 stdio adapter and Moira's independent
agent protocol/service/client/wire projection are implemented. The terminal
peer has read-only session, event, and durable-participation commands. Moira's
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
The exact terminal report-admission gate is implemented. Governed prompt
submission, provider-output parsing/composition, and the interactive TUI remain
open.

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
  Daimon agent-session profile
            |
       ACP client adapter
            |
 Codex · Claude · Gemini · other ACP agents
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

Current provider paths include:

- Codex through `@agentclientprotocol/codex-acp`;
- Claude through `@agentclientprotocol/claude-agent-acp`;
- Gemini CLI through its native `--acp` mode;
- additional agents through explicit, locally configured ACP profiles.

The official ACP Rust SDK and compatibility test kit should be preferred over a
new handwritten protocol implementation. Axon's implementation is behavioral
prior art, not a code dependency and not an alternate authority.

Direct vendor HTTP APIs remain separate Daimon adapters when ACP is unavailable
or the task is a terminal model completion rather than an interactive agent
session. Provider selection is by frozen wire profile, never by a vendor-shaped
branch in Moira.

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
5. persona, evidence, dissent, Pareto-frontier, and resolution views;
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
   encoding, and client facade are implemented. Provider-specific live
   qualification and production sandbox composition remain open. Genuine
   Codex gates pass initialize, session creation, one bounded no-tool prompt,
   completed terminal evidence, and close; Gemini initializes ACP but is
   account-blocked before session creation on the selected host; no Claude ACP
   bridge is installed there.
4. **Implemented:** Daimon agent sessions retain exact preparation attribution,
   operation fences, events and results across a generic durable store with
   no-redispatch reopen semantics, an independent canonical checkpoint, and
   exclusive local private-directory composition.
5. **Permission slice implemented:** the public selection contains no authority
   credential. A command-scoped Aegis adapter authorizes `Execute` for a
   domain-separated `(participation, execution, request, option, capability)` resource
   and returns attribution only. The participation engine independently checks
   the claimed digest and retained exact attempt. Broader launch-profile
   composition remains open.
6. **Public projection implemented:** independent Moira DTOs, disjoint clients,
   authorization-first service, Daimon public-client adapter, canonical bounded
   wire, Unix daemon profile, and read-only CLI session/event observation.
7. **Durable participation implemented:** canonical checkpoint, exclusive
   SQLite owner, public service/client/wire, private Unix daemon, exact
   permission authorization, pre-dispatch fence, no-redispatch recovery, and
   read-only `participation-inspect` CLI projection plus a stable, byte-preserving
   permission-confirmation view and explicit server-confirmed selection command.
   An exact terminal
   report-admission gate now requires a completed EndTurn observation plus
   matching member, Rune method, persona, Dendrite grounding, output, provider
   evidence, and grounding evidence before report storage. Governed
   prompt commands, provider-output parsing/composition, and interactive
   participation views remain open.
8. Qualify Codex, Claude, and Gemini independently, then qualify one mixed
   orthogonal ensemble. Each gate records exact advertised capabilities rather
   than claiming cross-provider feature parity.
9. Run the ACP compatibility test kit and add lost-reply, provider-exit,
   cancellation-race, stale-permission, credential-redaction, and replay tests.

## Explicit non-claims

- ACP compatibility does not imply identical provider behavior or tools.
- Registry presence does not authorize installation or execution.
- Authentication does not grant filesystem, terminal, network, or workflow
  authority.
- Session resume support is capability-negotiated, never assumed.
- A provider-generated plan is evidence, not Moira's governing Rune method.
- The TUI is not a provider sandbox, credential broker, or workflow authority.
