# Moira production roadmap

Status: pre-release; selected local OS-account authentication implemented,
production peer slice open.

Moira is the federation's durable workflow coordinator. Production maturity means
explicit saga, timing, reconciliation and peer-client contracts around recoverable
workflow authority. Moira must not absorb scheduling, execution, conversation,
document, ledger or authorization semantics from peer engines.

## Current baseline

- Deterministic workflow core from explicit Agora admission through Chronos task
  and attempt, fake Daimon result and replay-safe Agora publication.
- Stable effect identities, exact replay/conflict and uncertainty fencing.
- Canonical bounded coordinator checkpoint and single-owner SQLite command store.
- Durable Chronos, fake-Daimon and Agora receivers with lost-reply/process-restart
  evidence and selected schema upgrades.
- Live Aegis/Chronos-gated Daimon execution recovery through one exact Hermes
  result event, plus four-store local restart journeys.
- Content-safe bounded observations, embedded SDK facade, native saga service,
  private wire/daemon and Aegis-authorized operator reconciliation.

The legacy integration slice still uses sibling internals and a deterministic
fake provider. The closed saga family now has native service and private daemon
boundaries, but Moira is not yet a production connector set, supervised service,
real-provider federation slice or supported release.

## Release gate M0 — contract and assurance closure

- [x] Reconcile requirements, specification, model and traceability with the
  implemented SQLite and peer-receiver profiles.
- [x] Maintain a concrete STRIDE/LINDDUN threat model spanning every store,
  authenticated subject, peer receipt and operator action.
- [x] Pin the Rust toolchain and run `make check` in CI from a fresh coordinated
  checkout with all declared sibling requirements explicit.
- [x] Add changelog, release policy, package compatibility baseline, locked supply
  inventory and
  release-readiness ledger with adversarial drift tests.
- [x] Add digest-pinned external live fixtures for both legacy-workflow and saga
  canonical checkpoint profiles.
- [x] Add digest-pinned logical SQLite V1 fixtures for both profiles and prove
  current reopen plus exact replay without binding to SQLite page layout.
- [x] Add repository MIT and Apache-2.0 license texts matching package metadata.
- [x] Pin every locked package's declared license expression and reject missing
  declarations or graph/expression drift.
- [x] Add a current RustSec advisory attestation bound to the exact root lockfile
  and a live CI audit that refuses vulnerabilities or warnings.
- [ ] Complete source-content and legal license-compatibility review.
- [x] Add deterministic single-bit, byte-deletion and byte-insertion mutation
  assurance across every legacy-workflow, saga and readiness wire shape; every
  accepted mutation must re-encode byte-exactly.
- [x] Add deterministic logical database mutation assurance across owner/profile,
  initialization, retained revision/checkpoint and SQLite profile metadata; each
  altered store must refuse reopen rather than initialize replacement state.
- [x] Add an opt-in 5,000-cycle closed-saga service soak covering explicit retry,
  fresh effect identity, terminal completion and exact admission replay.
- [x] Select bounded SQLite `quick_check(1)` before trusting existing state and
  prove deterministic structural-page corruption refuses without rewriting the
  canonical legacy or saga database.
- [x] Add optimized phase-specific in-memory saga-service performance
  populations with declared p95/p99 engineering budgets and exact non-SLO
  limitations.
- [x] Add a 16-cycle SQLite-backed Aegis→Moira two-process campaign covering
  live exact-saga authorization and attributed durable reconciliation.
- [x] Add a 16-cycle durable Moira→Aegis→Chronos→Daimon→Hermes process campaign proving exact
  running-attempt validation, current profile/tool authority, discarded result
  reply, Daimon process reopen, reauthorized exact recovery, one provider call
  and one completed Moira workflow transition followed by one canonical event
  per fresh process set, with the Moira daemon reopened between preparation and
  result acceptance.
- [x] Extend process soak through Hermes with 16 fresh discarded-reply,
  process-reopen and exact one-event replay cycles.
- [ ] Extend the single-cycle Agora process qualification into a 16-cycle soak.
  Agora's actor-owned publication profile now retains a stable operation identity
  independently of authenticated connections, and the isolated qualification
  proves committed-reply loss followed by historical observation from a fresh
  public client. Do not turn latency into semantic evidence.

Exit: current claims and limitations are internally consistent and reproducible.

## Release gate M1 — complete workflow algebra

- [x] Close forward, terminal and compensation commands and outcomes for the
  selected V1 saga family. Compensation is a new attributable effect, not undo.
- [x] Define explicit retry commands, budgets and identity rules; no automatic
  retry may hide behind timeout or uncertainty handling.
- [x] Add caller-supplied time, deadlines, timer intents, timeout evidence and
  declared misfire behavior without an implicit clock in the core.
- [x] Close cancellation versus late-result, timeout, compensation and conflict
  precedence.
- [x] Define bounded multi-workflow admission and fairness observations.
- [x] Build an independently implemented reference model and bounded state
  explorer across phase, restart, loss, timeout, cancellation and compensation.
- [x] Adopt the closed saga checkpoint in SQLite, native service DTOs and
  reconciliation surfaces without implicitly migrating the existing V1 slice:
  - [x] Canonical bounded checkpoint bytes and a separately named SQLite saga
    store with exact reopen/replay and explicit foreign-profile refusal.
  - [x] Native saga host/protocol/service/client DTOs and ports, with durable
    service reopen/replay and capability-shaped client facades.
  - [x] Authenticated operator reconciliation commands and durable causation:
    - [x] Closed bounded disposition vocabulary, revision guards, durable
      subject/Aegis-decision causation and exact action-ID replay.
    - [x] Concrete authorization adapter against the public Aegis client.

Exit: M3–M7 are satisfied for one closed workflow family with model evidence.

## Release gate M2 — architectural separation

- [x] Add the native architectural layers around the sovereign core:
  - [x] `moira-host`, `moira-protocol`, `moira-service` and `moira-client` with
    a real SQLite reopen journey.
  - [x] `moira-wire` with distinct canonical bounded legacy-workflow and closed
    saga byte contracts.
- [x] Narrow `moira-sqlite` to coordinator/saga storage and reusable local SQLite
  mechanics; move Agora into `moira-agora-sqlite` and the coupled Chronos/Daimon
  execution projection into `moira-execution-sqlite`, with dependency-closure
  gates preventing it from becoming the universal composition root.
- [x] Define peer ports and production adapters against public Aegis, Hermes,
  Chronos, Agora and Daimon clients rather than their internal engine types:
  - [x] Aegis operator adapter using only `aegis-client`/`aegis-protocol` in its
    production dependency closure, including the service-owned
    `OperatorAuthorizer` port with explicit time and typed denial.
  - [x] Hermes workflow-event adapter using only `hermes-client`,
    `hermes-protocol` and Moira-owned protocol values in production.
  - [x] Agora response-publication adapter using only `agora-client`,
    `agora-protocol` and Moira-owned protocol values in production.
  - [x] Chronos task-admission adapter using only `chronos-client`,
    `chronos-protocol` and Moira-owned protocol values in production.
  - [x] Daimon public-client execution adapter with Moira-owned causation,
    attributable uncertainty and a production closure limited to
    `daimon-client`/`daimon-protocol` plus Moira protocol values. Durable Daimon
    process qualification now covers discarded-reply exit/reopen without provider
    redispatch; live Aegis/Chronos composition remains an M4 gate.
- [x] Add private local transport and daemon for both service profiles with
  bounded frames, deadlines, lifetime ownership, health/readiness, command-scoped
  operator authorization and truthful transmission uncertainty.
- [x] Bind both local daemon profiles to bounded exact kernel UID/GID mappings
  for ordinary read/mutate access, while keeping saga operator authority in a
  separately injected command-scoped authorizer.
- [x] Preserve embedded/local-daemon semantic parity and durable exact replay.

Exit: the complete fake-provider workflow runs through native Moira and peer
client contracts with no sibling internal dependency in production adapters.

## Release gate M3 — operator reconciliation

- [x] Add bounded read models for phase, last confirmed revision, pending effects,
  retained evidence, fences and blocked automatic progress.
- [x] Add authenticated, Aegis-authorized, revision-guarded commands to admit
  evidence, keep uncertain, retry with fresh identity, compensate, cancel or fail
  terminal. No generic "force success" operation exists.
- [x] Persist operator causation and exact replay receipts before releasing results.
- [x] Add explanation and content-safe export surfaces plus restart tests around
  every reconciliation choice.

Exit: every fenced workflow has an explicit inspectable disposition path or a
documented terminal reason why none is safe.

## Release gate M4 — production local federation slice

- [x] Replace fake execution with one real Daimon adapter under live Aegis checks.
  - [x] Aegis private daemon exports a reusable public-client Unix port with
    negotiated session binding and truthful outcome uncertainty.
  - [x] Daimon validates the exact running Chronos incarnation/attempt through a
    bounded public-client adapter and isolated real Chronos service qualification.
  - [x] Compose those two ports in the process-separated Moira execution journey.
- [ ] Use public peer services in separate processes:
  - [x] Aegis-authorized Moira operator reconciliation with both services in
    independent processes and only public client/wire traffic between them.
  - [x] Hermes workflow-event delivery with reply loss, Hermes process restart,
    exact public-client replay and one retained Moira-owned event.
  - [x] Chronos task admission with discarded execute reply, process exit/reopen,
    lookup-only reconciliation and one retained task receipt.
  - [x] Agora durable-service response publication with restored public-client
    and Moira intent state, discarded committed reply, both owners recreated and
    exact replay proving no second commit or message.
  - [x] Daimon execution with discarded reply, process exit/reopen, exact Moira
    receipt recovery and an external marker proving one provider dispatch.
  - [x] Moira execution through simultaneous real Aegis, Chronos and Daimon
    processes, with Aegis profile/tool authorization and exact running-attempt
    validation before the provider marker is created.
  - [x] Derive that execution intent from a SQLite-backed Moira daemon, stop the
    owner after durable preparation, reopen it after execution recovery, and
    accept the exact result through public Moira client/wire traffic.
  - [x] Discard that live execution reply, reopen Daimon's durable store in a
    fresh process, reauthorize retained delivery through the still-live Aegis
    authority and recover the exact Moira receipt without another Chronos lookup
    or provider dispatch.
  - [x] Publish the recovered completed disposition through an independent
    Hermes daemon after the same live Aegis authority grants exact-stream
    `Submit`/`Observe`, then read back one exact Moira-owned workflow event in
    the same process-separated chain.
  - [x] Enforce isolated Aegis, Agora, Chronos, Daimon and Hermes peer-profile
    qualifications in CI without merging their sovereign storage graphs into
    Moira's root lock. Agora's current evidence includes direct durable-service
    recovery and a clean durable-daemon state restart.
  - [x] Agora actor publication in an independent durable daemon process, with a
    committed reply discarded at Moira's boundary and the exact result observed
    by a new authenticated public client without republishing.
- [x] Add Vellum only through its public revisioned publication contract. The
  independent-process qualification launches Vellum's exact standalone daemon,
  establishes a saved baseline, discards the accepted durable edit reply, and
  reconciles the exact stable operation through a fresh authenticated client
  using historical lookup only before clean control-socket drain.
- [ ] Restart or lose replies at every process/effect boundary and prove no
  duplicate task, execution, event, edit or response:
  - [x] Live Daimon execution reply loss and process reopen under simultaneous
    Aegis/Chronos composition.
  - [ ] Compose the now-qualified Agora response and Vellum edit profiles with
    message admission and event delivery in one process-separated workflow.
- [x] Qualify stopped single-owner backup/restore for both native SQLite profiles,
  including released ownership, exclusion of persistent lock sentinels,
  byte-exact copy, retained effect identity and exact replay, with an operator
  recovery runbook.
- [x] Qualify bounded online backup for both native SQLite profiles while the
  single owner remains open, with private partial output, atomic publication,
  integrity verification, explicit post-rename publication uncertainty, source
  continuation and exact snapshot reopen/replay.
- [ ] Qualify installation packaging and future-version upgrade/migration for
  the selected local deployment profile.

Exit: one authenticated multi-process local workflow completes and reconciles
under failure using only public contracts.

## Deliberate non-goals

- Distributed transactions, implicit retries or inferred peer success.
- Owning Chronos schedules, Daimon execution policy, Agora rooms, Vellum edits,
  Hermes streams or Aegis authorization decisions.
- Treating correlation IDs, workflow IDs or stored peer receipts as authority.
- Remote or highly available deployment before the local service profile closes.

