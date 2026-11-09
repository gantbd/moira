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
  checkpoint/reconstruction replay is covered; encoded durable restart remains
  Phase 2 work.
- Build an independent reference model and bounded state explorer.

## Phase 2 — recovery

- Define a canonical checkpoint codec, journal records, and optimistic store
  protocol over the validated `CheckpointV1` semantic state.
- Inject failure before, during, and after every effect and commit boundary.
- Prove restart never creates duplicate task or reply intents.

## Phase 3 — real adapters

- [Qualified against volatile cores] Hermes append, Chronos task admission,
  Aegis-authorized Daimon results, and Agora publication.
- Implement production adapters only after the Phase 2 store protocol closes
  crash/reply ambiguity.
- Add Vellum only after the message-to-reply path is recoverable.

