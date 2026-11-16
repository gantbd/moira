# RustSec advisory attestation

Status: clean root-lockfile audit against a pinned current RustSec database;
not an SBOM, source review or legal opinion.

audited-at-utc: 2026-11-16T00:48:49Z
cargo-audit-version: 0.23.0
advisory-database-commit: 28d73fc95e5972af3403225a217def57dcaba3c4
advisory-database-commit-date: 2026-11-10T14:45:03+01:00
advisory-count: 1446
lockfile-sha256: 98127c27af5baeffa92c973f8950bdba7dcc87f04ced49718c14b524a4d1b0aa
dependency-count: 112
vulnerability-count: 0
warning-count: 0

The result was produced with `cargo audit 0.23.0 --no-fetch --json` after
checking out the exact advisory database commit above. No advisory is ignored.
The audit used no target filter and included the default informational warning
classes (`unmaintained`, `unsound`, and `notice`).

`scripts/check-advisory-attestation.sh` binds this evidence to the exact root
`Cargo.lock`; lockfile drift requires a new audit and attestation. CI additionally
runs `cargo audit --deny warnings` against the then-current database so a clean
pinned result cannot silently age into a release claim.

This covers Moira's root coordinated-source lockfile only. The deliberately
separate qualification workspaces have independent lockfiles and are not covered
by this result. It does not inspect unlocked source contents, prove absence of
undisclosed vulnerabilities, review license compatibility or attest artifacts.
