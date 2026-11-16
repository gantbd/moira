# Roadmap

## Phase 0 — close the contract

- Review ownership against `../ENGINE_MAP.md`.
- Define bounded IDs, revisions, causes, effect intents, receipts, and refusals.
- Write Given/When/Then scenarios before transition implementation.
- Mine Axon and Engram for failure cases without importing their runtime design.

## Phase 1 — deterministic reference slice

- [Complete in volatile reference profile] Agora input through attributable
  Chronos admission, exact task attempt, fake Daimon result, and replay-safe
  Agora publication is implemented with matching replay, conflict, lost-reply,
  and uncertain-result fencing scenarios.
- [In progress] Add stale revision, saturation, and timeout scenarios. Semantic
  checkpoint/reconstruction replay is covered; local encoded durable restart is
  now covered in Phase 2, including the local fake-provider cross-engine path.
- [Complete for closed saga family] Build an independent reference model and
  bounded state explorer across restart/lost reply, retry, timeout, cancellation
  and compensation. Canonical SQLite, native service/client, reconciliation
  inspection, durable operator causation, content-safe export, public-client
  Aegis authorization and a private saga wire/daemon profile are implemented.

## Phase 2 — recovery

- [Implemented architectural split] `moira-sqlite` owns coordinator/saga storage
  and reusable local SQLite mechanisms only; Agora projection lives in
  `moira-agora-sqlite`, while the settlement-coupled Chronos/Daimon projection
  lives in `moira-execution-sqlite`. CI freezes their production dependency
  closures so the base adapter cannot regress into a universal composition root.
- [Implemented for local coordinator state] Bounded canonical V1 checkpoint codec
  over Keystone's decoder and a separate SQLite command adapter with one active
  owner, commit-before-result, replay distinction and uncertain-commit fencing.
  Actual database tests cover every phase/disposition, before/after-commit faults,
  abrupt process exit, an abandoned uncommitted write and bounded structural-page
  refusal before existing state is trusted. No power-loss or arbitrary-corruption
  claim.
- Add durable peer-side canonical intent/receipt mappings; the coordinator's
  checkpoint alone cannot prevent duplicate peer effects after a lost reply.
- [Implemented for Chronos admission] A separate receiver commits the actual
  Chronos checkpoint and canonical one-task intent/original receipt together.
  Matching replay after receiver process loss or both-engine restart creates no
  second task.
- [Implemented for one exact attempt] Targeted claims persist their native Chronos
  checkpoint and attribution before release; leases stay private. Recovery persists
  uncertainty without claiming again. Finite worker grants are volatile and foreign
  after reopen.
- [Implemented for the fake provider] Durable dispatch marker, exact retained
  result, recovery-unknown fencing without redispatch, current Aegis profile/tool
  Execute checks and exact live Chronos attempt validation. All three stores can
  reopen around a lost result reply and continue Moira to publication pending.
  Durable Agora publication is now covered below.
- [Implemented for exact result settlement] Receiver-sealed facts, explicit host
  actions and source-owner binding settle live or recovered attempts with no retry
  default. The native checkpoint, settlement receipt and census commit together;
  lost-reply replay, V1/V2 upgrades, missing records and persistent conflict fences
  are covered. The three-store path now settles Chronos before Agora publication.
- [Implemented for the local fake-provider vertical slice] Agora's native bounded
  checkpoint and the separate SQLite receiver retain replies, exact replay receipts,
  events/acknowledgements and stable room/source correspondence. The four-store test
  completes a persisted human-message workflow after lost result/final reply
  acknowledgement, without duplicate effects. Its uncertain variant stays fenced
  with no reply. Production providers, remote services and Vellum remain separate.
- Inject failure before, during, and after every effect and commit boundary.
- Prove restart never creates duplicate task or reply intents.

## Phase 3 — real adapters

- [Public-client adapter qualified] Hermes workflow-event append owns its schema
  in Moira and proves lost-reply replay against the real Hermes service without a
  production dependency on Hermes internals.
- [Public-client, durable-service and independent-process recovery qualified]
  Agora response publication has both its compatibility V2 request path and an
  actor-owned operation path. The actor path preserves Moira's stable effect,
  exact room/revision/time/body/audience intent and accepted result independently
  of an authenticated connection. An isolated real daemon process commits the
  publication, Moira discards the successful reply, and a fresh authenticated
  client observes the exact result without another mutation.
- [Public-client adapter qualified] Chronos task admission exports prepared
  invocation identity and closes a lost execute reply through V3 invocation plus
  V4 original-allocation lookup after service reopen, never automatic mutation
  replay or current-name snapshot fallback. Process qualification retires the
  original task, reuses its name and still returns the original receipt.
- [Public-client process profile qualified] Daimon execution discards the first
  reply, exits, reopens its durable daemon and returns the exact Moira receipt
  without provider redispatch. The live composition now repeats this under a
  still-running Aegis authority after exact Chronos validation, reauthorizing
  retained delivery without another Chronos lookup or provider call.
- [Qualified for local durable ingress] Participant-neutral addressed Agora
  events project as passive facts; explicit Moira admission and host-owned
  participant mapping survive restart/lost reply without duplicate workflow or
  task-submission identity.
- Implement production adapters only after the Phase 2 store protocol closes
  crash/reply ambiguity.
- [Public-client adapter implemented] Model execution retains the originating
  Moira effect and exact Chronos attempt while projecting bounded typed Agora and
  Vellum proposals. Vellum submission maps a separate Moira effect to Vellum's
  restart-stable durable operation and supports lookup-only reconciliation without
  falling through to execution. The ordered proposal set now has a deterministic
  saga, canonical bounded checkpoint and separately owned SQLite store with exact
  replay/reopen behavior. A representation-independent public service/client
  contract now exposes admission, effect evidence and typed revision conflicts.
  A selected local federation journey loses Agora and Vellum replies, reopens the
  proposal store, reconciles Vellum through historical lookup and proves one
  Chronos validation, provider call, message and document mutation. Private
  wire/daemon transport and the multi-process kill matrix remain open.

## Phase 4 — reusable operator surface and standalone CLI

- Define `moira-tui` as a composable presentation over Moira's public client.
  It owns bounded workflow/saga projection, local input, confirmations, and
  lifecycle requests, but not terminal setup, argument parsing, daemon startup,
  global shortcuts, or process exit.
- Provide a separate Moira CLI/application composition root. It selects embedded
  or remote service access and can run the TUI or bounded headless workflow and
  reconciliation commands without fabricating presentation state.
- Let Workbench reuse the same TUI component and optionally host Moira's service
  runtime under one executable and lifecycle. Moira keeps workflow authority,
  receipt interpretation, authorization, and uncertainty semantics.
- Qualify the four parity paths: standalone interactive, standalone headless,
  Workbench interactive, and Workbench headless. A close request removes a pane
  in Workbench and exits only when the standalone CLI chooses that policy.

