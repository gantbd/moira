# Requirements

Status: proposed. Nothing in this document is an implementation claim.

- **M1 — Single authority:** one workflow owner serializes each workflow's
  decisions under an optimistic revision.
- **M2 — Correlation:** every workflow, step, effect, task, execution, artifact,
  and response retains explicit bounded causation and correlation.
- **M3 — Durable decisions:** accepted decisions and pending effect intents can be
  recovered without reinterpreting external state.
- **M4 — Idempotent boundaries:** replayed inputs and effect receipts cannot create
  duplicate tasks, executions, edits, or responses.
- **M5 — Honest effects:** prepared, dispatched, known-completed, known-failed, and
  uncertain effects remain distinct.
- **M6 — Saga semantics:** cross-engine composition uses explicit forward steps and
  compensation decisions; it never claims a distributed transaction.
- **M7 — Bounded state:** workflows, steps, identifiers, evidence, retries, timers,
  and retained outcomes have declared limits.
- **M8 — External authority:** Moira requires current Aegis evidence for protected
  actions but does not authenticate credentials itself.
- **M9 — Effect isolation:** engine transitions perform no I/O, read no clock, and
  invoke no connector.
- **M10 — Passive conversation ingress:** Agora addressing is observed as bounded
  conversation data only. It creates no workflow, task, authority, or execution
  inference. Workflow admission and participant-reference mapping require a
  separate explicit Moira command whose replay is exact and whose conflicting
  mapping is refused.

Open: additional production peer mappings, timers, compensation vocabulary, workflow migration,
multi-workflow fairness, production connectors, and operational SLOs.

