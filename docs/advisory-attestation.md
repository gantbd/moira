# RustSec advisory attestation

Status: clean root-lockfile audit against a pinned current RustSec database;
not an SBOM, source review or legal opinion.

audited-at-utc: 2026-11-17T00:16:53Z
cargo-audit-version: 0.23.0
advisory-database-commit: 02e2517f457a359fc6d3440ea4560c8ea188fee8
advisory-database-commit-date: 2026-11-15T10:59:21+01:00
advisory-count: 1509
lockfile-sha256: f151ef25cb2bb58f26f4e472a8b4223f8c9608881fd978a8f94d1be5b9a71f87
dependency-count: 128
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
