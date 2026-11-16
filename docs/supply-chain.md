# Supply-chain profile

Status: exact locked coordinated-source inventory, license-declaration baseline
and current root-lockfile RustSec attestation; not an SBOM or legal/source-content review.

`make check-supply-chain` compares the complete `Cargo.lock` package set with
`supply-chain-baseline.txt`, requires an exact allowlisted identity for every Git
source, requires a checksum for every registry package and resolves the locked
graph offline. The sole current Git identity is Vellum wire's immutable Keystone
codec revision; drift or any additional Git source fails closed. Local peer
packages are part of the inventory because Moira is currently qualified only
from the coordinated sibling checkout.

The same gate feeds complete locked offline Cargo metadata to Keystone's
dev-only verifier and compares every exact package license expression with
`license-baseline.txt`. A missing declaration or expression drift fails closed;
the accepted baseline remains Moira-owned policy.

The excluded `qualification/aegis-moira-process` assurance workspace has a
separate lockfile and is exercised by `make qualify-aegis-process`. Its graph is
not silently merged into Moira's 112-component production/test baseline: doing so
would conflate Aegis's shared Keystone storage with Chronos's sovereign vendored
storage package merely because both currently use the same Cargo name and version.
The excluded `qualification/chronos-service-recovery` workspace likewise owns a
separate lockfile and is exercised by `make qualify-chronos-recovery`.
The excluded `qualification/agora-service-recovery` workspace owns its own
11-package locked assurance graph and is exercised by
`make qualify-agora-recovery`; Agora engine/service/durable internals remain
outside the production adapter closure and Moira's root lock.
The excluded `qualification/hermes-process-delivery` workspace owns its own
locked assurance graph and is exercised by `make qualify-hermes-process`.
The excluded `qualification/live-execution-chain` workspace owns a separate
81-package lock for its SQLite-backed Moira/Aegis/Chronos/Daimon/Hermes process harness and is
exercised by `make qualify-live-execution-chain`; those peer implementation
graphs remain outside Moira's production lock and adapter closures.

The Aegis adapter's production closure is separately checked to contain only
`aegis-client` and `aegis-protocol`, never Aegis authority, host or service
internals. The latter appear only in integration-test dependencies.
The Hermes adapter's production closure is separately checked to contain only
`hermes-client`, `hermes-protocol` and `moira-protocol`, never Hermes ledger,
host, service, wire or local-adapter internals. Those appear only in qualification
test dependencies.
The Unix peer adapter's production closure is separately checked to contain only
Moira local-daemon/protocol/service contracts and pinned `nix`; SQLite, Aegis and
peer engine internals remain test-only or independently injected.
The Agora adapter's production closure is separately checked to contain only
`agora-client`, `agora-protocol` and `moira-protocol`, never Agora engine, host,
service or wire internals. The conformance tests retain the same closure;
the separately locked durable-service recovery qualification contains the
test-only Agora internals needed to prove exact replay after both owners restart.
The Chronos adapter's production closure is separately checked to contain only
`chronos-client`, `chronos-protocol` and `moira-protocol`, never Chronos native
adapter, engine, host, service or wire internals. Those appear only in its
receipt-gated qualification-test dependencies.

The lockfile-bound [advisory attestation](advisory-attestation.md) records a clean
audit against an exact RustSec database revision, and CI audits the current
database without ignored warnings. This detects unreviewed dependency drift,
incomplete registry provenance and known advisories at audit time. It does not
pin sibling repository revisions, establish source/license compatibility,
artifact contents, signatures, reproducibility or
provenance from a clean Cargo home. Those remain release blockers.
