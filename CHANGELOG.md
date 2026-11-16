# Changelog

All notable Moira changes will be recorded here. Released versions will follow
Semantic Versioning once a public distribution contract is selected.

## [Unreleased]

### Added

- Added an actor-owned Agora response-publication adapter whose stable operation
  identity survives authenticated reconnects while preserving exact room,
  revision, logical time, body and addressed recipients.
- Extended the isolated Agora qualification with a durable daemon subprocess:
  Moira discards a committed publication reply and a new authenticated public
  client observes the exact retained result without a second mutation.
- Extended that proof through the exact Aegis and Agora standalone binaries,
  including a current bounded scheduled-composition grant and ordered clean
  owner shutdown.
- Added an isolated qualification against Vellum's exact standalone daemon that
  discards an accepted durable edit reply, reconciles the stable Moira operation
  through a fresh authenticated public client using historical lookup only, and
  cleanly drains the owner through its public control socket.
- Added a lockfile-bound RustSec advisory attestation and live CI advisory gate.
- Added `moira-unix-peer`, a bounded exact kernel UID/GID authenticator for both
  local daemon profiles. It maps only ordinary read/mutate facts, refuses unknown
  peers before readiness and composes saga operator authority through a separate
  injected authorizer.
- Added a service-owned command-scoped operator authorization port and a
  public-client Aegis implementation with explicit time, exact-saga scope and
  typed denial.
- Added bounded online SQLite snapshots for both native store profiles with
  private partial output, integrity verification, atomic publication and exact
  earlier-revision reopen/replay while the source owner continues.
- Added an optimized reference-host performance gate for saga admission, point
  read, effect acceptance and retry with declared p95/p99 engineering budgets.
- Added a 16-cycle SQLite-backed Aegis→Moira two-process soak for live exact-saga
  authorization, attributed reconciliation and durable operator receipts.
- Added a bounded pre-trust SQLite structural integrity profile and deterministic
  legacy/saga page-damage tests that prove refusal without database replacement.
- Added repository MIT and Apache-2.0 license texts matching package metadata and
  made them mandatory release-contract artifacts.
- Added an exact locked-graph license-declaration baseline using Keystone's
  dev-only verifier; missing declarations and expression drift now fail closed.
- Added an opt-in 5,000-cycle closed-saga service soak spanning failed evidence,
  fresh-identity retry, terminal completion and exact admission replay.
- Added digest-pinned external live checkpoint fixtures for both the legacy
  workflow and closed saga profiles.
- Added digest-pinned logical SQLite V1 fixtures for both storage profiles with
  current reopen and exact-replay qualification.
- Added stopped backup/restore qualification for both SQLite profiles and a
  fail-closed local operator recovery runbook.
- Added bounded online snapshots for both SQLite profiles, with private partial
  output, exact revision receipts and explicit post-rename publication uncertainty.
- Added deterministic single-bit, byte-deletion and byte-insertion mutation
  assurance across every legacy-workflow, saga and readiness wire shape, with
  byte-exact canonical re-encoding required for every accepted mutation.
- Added fail-closed logical saga-database mutation coverage for ownership,
  profile, initialization, revision, checkpoint and SQLite schema metadata.
- Extended the isolated Chronos qualification across an actual discarded execute
  reply, process exit/reopen, task retirement/name reuse and V3/V4 lookup-only
  recovery of the original task receipt; uncaptured history fails closed.
- Added validated Agora client/publisher namespace restoration and an isolated
  real durable-service qualification proving discarded-reply recovery performs
  no second commit or message after both owners are recreated.
- Added a process-separated Moira→Hermes qualification that discards the first
  append reply, restarts Hermes from durable state and proves exact one-event
  replay through public client/wire contracts.
- Isolated the live Chronos lost-reply/restart qualification behind
  `make qualify-chronos-recovery`, keeping peer-owned storage graphs out of the
  root Moira lock while retaining the real public-client service evidence.
- Added the deterministic workflow core, canonical checkpoint, SQLite command
  store, durable peer receivers and embedded SDK facade.
- Added the production roadmap, pinned toolchain, CI workflow, security policy
  and threat/privacy model.
- Added the first native `moira-host`, `moira-protocol`, `moira-service` and
  `moira-client` boundaries, including a real SQLite reopen/replay journey.
- Added representation-independent protocol DTOs and the canonical bounded
  `moira-wire` V1 request/reply codec.
- Added an authenticated private Unix daemon with negotiated bounds, deadlines,
  health, truthful delivery uncertainty and exact replay across daemon restart.
- Added the closed deterministic saga/timer kernel with explicit retry,
  cancellation, compensation and a 36-program independent recovery model.
- Added bounded identity-ordered saga inspection and a content-free phase census
  for host-owned fairness monitoring without implicit scheduling policy.
- Added canonical bounded saga checkpoint bytes and a separate SQLite saga
  profile with exact reopen/replay, persisted limit binding and foreign-profile
  refusal.
- Added separate saga host, protocol, service and capability-shaped client
  contracts, including authorization-before-dispatch and native service replay
  after SQLite reopen.
- Added bounded reconciliation views and evidence pages with durable
  last-confirmed revision, explicit effect/deadline fence provenance and
  automatic-progress classification.
- Added a closed operator disposition contract with revision guards, bounded
  durable subject/Aegis-decision causation, exact lost-reply replay and no
  generic force-success operation.
- Added `moira-aegis`, a public-client-only operator adapter with stable saga
  resource scoping and a live Aegis allow/deny plus SQLite-reopen journey.
- Added content-safe point and bounded-page operator receipt exports that omit
  authenticated subjects while retaining original Aegis attribution,
  disposition and resulting state across restart.
- Added a distinct canonical saga wire family plus private Unix saga daemon with
  bounded allocation, command-scoped operator authorization, typed denial and
  exact reconciliation replay across daemon/SQLite reopen.
- Added `moira-hermes`, a public-client-only workflow-event publisher with a
  bounded Moira-owned transition schema, workflow-correlation guard and real
  Hermes service lost-reply/replay qualification.
- Added `moira-agora`, a public-client-only response publisher that composes
  Agora's exact request uncertainty fence with Moira effect/source/room identity
  and proves one-message replay after a lost service reply.
- Added `moira-chronos`, a public-client-only task-admission adapter with
  exportable prepared invocation identity, lookup-only uncertainty closure and
  exact task-incarnation receipt recovery after atomic-service reopen.
- Added `moira-daimon`, a public-client-only execution adapter that preserves
  Moira effect/task-attempt causation, maps Daimon outcome uncertainty explicitly
  and proves matching replay invokes the real volatile Daimon service provider once.
- Added a two-process Aegis→Moira local federation journey: live exact-saga
  authorization over Aegis public wire and attributed reconciliation over Moira
  public wire, with peer runtimes confined to test dependencies.

### Changed

- Split peer-aware local SQLite receivers out of `moira-sqlite`: Agora now lives
  in `moira-agora-sqlite`, while settlement-coupled Chronos/Daimon execution lives
  in `moira-execution-sqlite`; dependency-closure gates keep the base package
  limited to coordinator/saga storage and reusable SQLite mechanics.
- Scoped formatting verification to Moira-owned packages and updated the Agora
  receiver for the current message-identity and moderation refusal vocabulary.
- Updated the Agora receiver for explicit unknown-message and repeated-tombstone
  refusal classification introduced by the current Agora contract.
- Classified Agora's archived-thread and invalid lifecycle-change refusals as
  invalid publication input while Agora's contract refactor proceeds.
- Classified Agora reaction capacity separately from invalid reaction input as
  its public refusal vocabulary evolves; Moira retains no reaction semantics.
- Classified Agora message-revision capacity separately from invalid edit input
  while its public contract evolves; Moira retains no message-edit semantics.
- Classified Agora topic capacity, identity exhaustion, duplication, unknown
  resource and invalid-name refusals while retaining no topic semantics.

### Security

- Unknown or contradictory peer evidence remains fenced; correlation and
  workflow identifiers confer no authority.

[Unreleased]: https://github.com/gantbd/moira/commits/main

