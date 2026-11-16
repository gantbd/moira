# Requirements

Status: selected initial contract. Requirement-level implementation status is
tracked in `docs/traceability.md`; the deterministic core and selected local
SQLite vertical slice, closed saga service and Aegis-authorized operator path are
implemented, while remaining production-peer profiles remain open. This document
alone is not an implementation or deployment claim.

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
- **M11 — Public peer isolation:** production connectors use peer-owned public
  client/protocol contracts, retain Moira's causation and uncertainty, and never
  link peer engines, stores, services or transport mechanisms as authority.
- **M12 — Process-separated federation:** the selected local deployment proves
  authenticated peer decisions and workflow mutations across independently
  restartable service processes using only public client/wire contracts.
- **M13 — Fail-closed storage integrity:** every selected SQLite owner validates
  bounded structural integrity before trusting existing state; detected damage
  refuses without repair, reset or replacement initialization.
- **M14 — Composition boundaries:** the coordinator/saga store, Agora receiver and
  coupled Chronos/Daimon execution projection have separate package ownership and
  mechanically checked production dependency closures; no SQLite package is the
  universal composition root.
- **M15 — Proposal/effect separation:** Daimon model output enters Moira only as
  bounded typed proposals. Each accepted Agora publication or Vellum edit receives
  its own stable Moira effect identity and peer-native durable/replay identity;
  proposal receipt is never treated as peer-effect completion.
- **M16 — Epistemic honesty:** deliberation preserves deterministic proof,
  accountable choice, four-valued maybe and bounded non-convergence as distinct
  outcomes. Pareto membership is neither proof nor selection, model agreement is
  not authority, and fuel exhaustion cannot be translated into success.

For the selected local Unix deployment, M8 and M12 use a bounded exact
kernel-established UID/GID mapping only for ordinary read/mutate access. Saga
operator reconciliation remains a separate command-scoped authorization step;
an OS account is never workflow authority.

The deterministic saga kernel closes retry, timer, cancellation, compensation
and operator-disposition semantics for one bounded family. Canonical SQLite,
native service/client, bounded reconciliation reads, durable causation and the
public-client Aegis operator adapter, content-safe receipt export and private saga
wire/daemon profile are implemented. The process-separated Aegis→Moira,
Moira→Chronos and Moira→Hermes segments are qualified. Open: the complete peer
workflow chain, durable Daimon and Agora process profiles, workflow migration,
production connectors, and operational SLOs.

