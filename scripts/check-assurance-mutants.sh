#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
task_dir=$(mktemp -d "${TMPDIR:-/tmp}/moira-assurance-mutants.XXXXXX")
trap 'rm -f "$task_dir"/*; rmdir "$task_dir"' EXIT

awk '!/^package\|Cargo.toml\|/' docs/compatibility-baseline.txt > "$task_dir/compatibility.txt"
if MOIRA_COMPATIBILITY_BASELINE="$task_dir/compatibility.txt" bash scripts/check-compatibility-baseline.sh >"$task_dir/output" 2>&1; then
  echo 'Moira compatibility gate accepted a missing-package mutant' >&2; exit 1
fi
sed 's/c53d8c85/d53d8c85/' docs/compatibility-baseline.txt > "$task_dir/compatibility.txt"
if MOIRA_COMPATIBILITY_BASELINE="$task_dir/compatibility.txt" bash scripts/check-compatibility-baseline.sh >"$task_dir/output" 2>&1; then
  echo 'Moira compatibility gate accepted a fixture-digest mutant' >&2; exit 1
fi
sed 's/component|moira-core|0.0.1/component|moira-core|0.0.2/' docs/supply-chain-baseline.txt > "$task_dir/supply.txt"
if MOIRA_SUPPLY_BASELINE="$task_dir/supply.txt" bash scripts/check-supply-chain.sh >"$task_dir/output" 2>&1; then
  echo 'Moira supply gate accepted a version-drift mutant' >&2; exit 1
fi
sed 's/license|moira-core|0.0.1|MIT OR Apache-2.0/license|moira-core|0.0.1|MIT/' docs/license-baseline.txt > "$task_dir/licenses.txt"
if MOIRA_LICENSE_BASELINE="$task_dir/licenses.txt" bash scripts/check-supply-chain.sh >"$task_dir/output" 2>&1; then
  echo 'Moira supply gate accepted a license-expression mutant' >&2; exit 1
fi
sed 's/98127c27/08127c27/' docs/advisory-attestation.md > "$task_dir/advisory.md"
if MOIRA_ADVISORY_ATTESTATION="$task_dir/advisory.md" bash scripts/check-advisory-attestation.sh >"$task_dir/output" 2>&1; then
  echo 'Moira advisory gate accepted a lockfile-digest mutant' >&2; exit 1
fi
echo 'Moira assurance gates reject compatibility, supply, license and advisory mutants'
