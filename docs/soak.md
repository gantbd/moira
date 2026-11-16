# Deterministic soak profile

`make soak` runs 5,000 fresh closed-saga service lifecycles. Every cycle admits
one saga, records a failed forward effect, explicitly retries under a fresh
effect identity, accepts success, prepares and accepts the terminal effect, then
exactly replays the original admission without changing the retained identity.

The fixed campaign is opt-in so ordinary correctness tests remain fast and
focused. It detects phase, effect-allocation, retry-budget and service-mapping
state accumulation in the selected profile. It is not a latency benchmark,
SQLite/power-loss test, daemon/process soak, peer federation campaign,
concurrency proof or product SLO.

`make soak-process` runs 16 fresh Aegis→Moira federation cycles. Each cycle
starts an Aegis authority daemon and a separate Moira saga daemon, creates a
fresh SQLite saga store, obtains exact-saga `Administer` authority through the
public Aegis client/wire contract, admits an uncertain saga, performs attributed
operator reconciliation through the public Moira client/wire contract, reads
the durable operator receipt, waits for both processes, verifies socket cleanup,
and removes the fresh directory.

The campaign passed on 2026-11-10 in 2.10 seconds on Darwin arm64. It is selected
operator-profile repeatability evidence, not the complete Moira workflow chain.
It does not soak Chronos admission, Hermes publication, Agora response, sudden
power loss, concurrent operators, resource leakage or a product availability
objective.

`make soak-hermes-process` runs 16 fresh Moira→Hermes delivery cycles. Every
cycle starts a new Hermes daemon and physical ledger, publishes one Moira-owned
workflow event through the public client/wire contract, discards the committed
append reply, waits for process exit, reopens the ledger in a fresh process and
replays the exact producer key. It then observes exactly one event with the
expected Moira correlation and payload and verifies clean socket teardown before
removing the fresh directory.

This is repeatability evidence for one delivery segment. It does not establish a
complete workflow chain, concurrent producers, abrupt power-loss behavior,
remote transport security, latency or an availability objective. The campaign
passed on 2026-11-11 in 1.08 seconds on Darwin arm64; that duration is an
observation, not a threshold.

`make soak-live-execution` runs 16 fresh six-process execution-to-event cycles.
Every cycle first prepares a workflow through a SQLite-backed Moira daemon, then
starts independent Aegis and Chronos helpers and a Daimon daemon whose
public adapters require current `Daimon/Execute` authority for both the profile
and tool and validate the exact running Chronos incarnation and attempt. Moira
submits through Daimon's public client and discards the reply. After that daemon
exits, a fresh Daimon process reopens the durable store, obtains fresh authority
from the still-live Aegis process and returns the exact completed attributable
receipt. A fresh Moira daemon reopens the same SQLite owner and accepts that exact
result. The completed transition is then published through an independent
Hermes daemon after the same Aegis authority grants exact-stream
`Submit`/`Observe`, and read back as one exact Moira-owned workflow event. The cycle
verifies a create-once provider marker, no second Chronos lookup, one event and
clean process/socket shutdown before removing its fresh directory.

This campaign exercises repeatability of the selected live execution-to-event
segment; it is not an end-to-end message-to-reply soak. Hermes delivery also has
separate discarded-reply process-soak evidence; Agora response publication, Vellum
editing, provider realism, concurrency, power loss, latency and availability
objectives remain outside it.
The expanded durable six-process campaign passed on 2026-11-11 in 77.00 test
seconds on Darwin arm64; that duration is an observation, not a
performance threshold.
