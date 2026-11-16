# Release readiness

Status: source-verifiable local fake-provider integration profile; not ready for
a standalone service, registry publication or production deployment.

Established evidence is recorded in `docs/traceability.md`. Repository license
texts, exact license-declaration baselines, digest-pinned checkpoint/logical
database fixtures, bounded SQLite structural integrity checks and stopped
backup/restore qualification and a lockfile-bound clean RustSec attestation are
present. Current blockers are source-content/legal review,
remote identity and same-UID hostile-process isolation beyond the selected local
OS-account authenticator, deployment operator grant/time provisioning,
installation/upgrade qualification,
durable peer process profiles, a real provider and completion of multi-process
qualification beyond the established Aegis→Moira operator and Moira→Chronos
admission segments plus the integrated Aegis/Chronos-gated
Moira→Daimon→Hermes execution-to-event chain. Agora and
the complete end-to-end workflow remain open. Agora's real durable service now
qualifies restored lost-reply replay without duplicate commit/message, and its
local daemon now qualifies clean durable-state restart. The daemon still closes
and reclaims the authenticated service-client receipt namespace on disconnect,
so it does not yet close the independent-process lost-reply boundary.
The public Daimon execution adapter is present and qualified against Daimon's
real service contract. Daimon's canonical exclusive local persistence, bounded
wire/private daemon and discarded-reply process restart are established. It does
not yet replace the local durable fake receiver in the release slice because a
production provider profile and complete workflow integration remain open. The chain is now
composed in an assurance profile: Aegis exposes a reusable daemon `ServicePort`,
Daimon validates a real running Chronos attempt, and Moira recovers the completed
result after discarding its reply and reopening Daimon. Retained delivery is
reauthorized through the still-live authority without a second Chronos lookup or
provider call. A selected production provider profile and incorporation into the
complete message-to-reply chain remain open.
The optimized in-memory phase budgets and opt-in 5,000-cycle closed-saga service
soak are green. Separate 16-cycle campaigns cover SQLite-backed Aegis→Moira
operator federation, discarded-reply Moira→Hermes event delivery, and the live
durable Moira→Aegis/Chronos-gated Daimon→Hermes execution-to-event segment. None establishes power-loss,
full-chain, production-provider, concurrency or product-SLO qualification.

Passing `make check` plus `make check-peer-profiles` establishes only the current
coordinated source and isolated peer-assurance profiles; it explicitly does not
authorize publication.

The local SQLite composition is no longer rooted in one peer-aware package:
`moira-sqlite` owns coordinator/saga stores and shared mechanisms,
`moira-agora-sqlite` owns Agora projection, and `moira-execution-sqlite` owns the
settlement-coupled Chronos/Daimon projection. CI checks each production dependency
closure. This is architectural separation, not a production peer-client claim for
the local receiver crates.

