# Moira threat and privacy model

Status: baseline for the deterministic core, local SQLite profile, private local
daemon, qualified peer receivers, Aegis/Hermes/Agora/Chronos public-client
adapter profiles and the first Aegis→Moira process-separated segment.

## Assets and trust boundaries

Protected assets are workflow/effect identities, phase and revision, peer intents
and attributed receipts, uncertainty/conflict fences, operator decisions, SQLite
state and content-safe operational evidence. Trust boundaries exist at Agora
ingress, Chronos admission/claim/settlement, Daimon dispatch/result, Aegis
authorization, Hermes append, future Vellum publication, operator control, each
SQLite owner and the surrounding operating system.

The current vertical slice trusts its embedding host, kernel-established local
UID/GID and selected SQLite/filesystem behavior. The operator segment exercises
separate Aegis and Moira processes over private public-contract sockets, and the
Agora response segment exercises an independent durable daemon with two fresh
authenticated public clients. Other peer journeys remain separately qualified or
in-process. It uses a fake Daimon provider and does not qualify authenticated
remote peers, distributed transactions, physical power loss or production
remote credential provisioning. The selected Unix mapping is an OS-account
boundary, not a person, binary, process, container, root or hostile-host boundary.

## STRIDE controls and residual risks

| Threat | Current control | Residual risk / required gate |
| --- | --- | --- |
| Workflow or peer spoofing | Fixed typed identities, exact owner/source bindings, receipt attribution, bounded exact kernel UID/GID mapping before readiness and live Aegis→Moira exact-saga authorization in separate processes | Same-UID hostile processes, remote identity and remaining peer authentication bindings remain open |
| Effect duplication | Stable effect identities, retained replay, commit-before-result, recovery fences, Hermes append replay and Chronos lookup-only reconciliation across real process restart, Agora actor-operation observation, and Vellum historical durable-operation lookup after committed-reply loss from fresh authenticated clients | The combined Agora→Vellum and every-effect process fault matrix remains open |
| Receipt substitution | Exact intent/receipt correspondence and conflicting-evidence fences | Compromised peer/host can fabricate facts before authentication boundary |
| Unauthorized execution | Current Aegis checks for fake Daimon profile/tools, Chronos attempt validation, public-client `Administer` checks for exact saga operator resources, command-scoped saga-daemon denial before store dispatch, and a Unix peer adapter that cannot manufacture operator facts | Selected peer mappings still require protected launcher configuration; remote identity, same-UID isolation and remaining peer actions need one closed authorization matrix |
| Repudiation | Retained commands, receipts, revisions, bounded durable operator subject/Aegis-decision causation, and subject-free point/page receipt export | Production audit sink and retention policy remain open |
| Information disclosure | Content-free observations, redacted errors, bounded codecs and mode-`0600` partial online snapshots | Workflow, peer databases and published snapshots contain sensitive plaintext and metadata; encryption and retention remain deployment policy |
| Denial of service | Workflow/database/page bounds, single-owner serialization, bounded frames/connections and I/O deadlines | Multi-workflow fairness and principal admission policy are open |
| Recovery ambiguity | Indeterminate commits and recovered dispatch markers fence automatic progress; bounded SQLite structural checks refuse damaged pages before state is trusted; operator dispositions replay after reopen; bounded online snapshots publish atomically, report post-rename sync failure as publication uncertainty and reopen at an exact earlier revision | Valid-looking snapshot rollback, torn-write/device certification and real-provider recovery remain open |
| Cross-engine confused deputy | Domain ownership boundaries, explicit mappings, public-client dependency gates and process-separated Aegis→Moira authorization | Remaining peer processes and full workflow authorization matrix are open |

## LINDDUN privacy considerations

Cross-engine correlation is intrinsically linkable. Workflow IDs, causation,
participant mappings, task attempts, tool profiles, publication targets and
timing can reconstruct sensitive activity even without bodies. Logs and metrics
must avoid these labels; operator exports require explicit authorization,
minimization and bounded retention.

## Security release gates

- Conversation addressing never implies workflow admission or execution.
- Every protected effect is authorized against current Aegis state.
- Correlation, workflow and peer IDs never substitute for authentication.
- Unknown outcomes remain fenced until attributable evidence and explicit policy
  choose a disposition; no timeout silently becomes success or retry.
- Operators use authenticated, authorized, revision-guarded replay-safe commands.
- Production adapters consume public peer contracts with authenticated transport.
- A multi-process restart/lost-reply suite proves no duplicate external effects.

