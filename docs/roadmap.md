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
- Build an independent reference model and bounded state explorer.

## Phase 2 — recovery

- [Implemented for local coordinator state] Bounded canonical V1 checkpoint codec
  over Keystone's decoder and a separate SQLite command adapter with one active
  owner, commit-before-result, replay distinction and uncertain-commit fencing.
  Actual database tests cover every phase/disposition, before/after-commit faults,
  abrupt process exit and an abandoned uncommitted write. No power-loss claim.
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

- [Qualified against volatile cores] Hermes append, Chronos task admission,
  Aegis-authorized Daimon results, and Agora publication.
- [Qualified for local durable ingress] Participant-neutral addressed Agora
  events project as passive facts; explicit Moira admission and host-owned
  participant mapping survive restart/lost reply without duplicate workflow or
  task-submission identity.
- Implement production adapters only after the Phase 2 store protocol closes
  crash/reply ambiguity.
- Add Vellum only after the message-to-reply path is recoverable.

