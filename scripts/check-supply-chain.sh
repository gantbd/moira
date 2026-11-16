#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

baseline=${MOIRA_SUPPLY_BASELINE:-docs/supply-chain-baseline.txt}
licenses=${MOIRA_LICENSE_BASELINE:-docs/license-baseline.txt}
[[ -s "$baseline" ]] || { echo 'Moira supply-chain baseline is missing' >&2; exit 1; }
[[ -s "$licenses" ]] || { echo 'Moira license baseline is missing' >&2; exit 1; }
actual=$(mktemp "${TMPDIR:-/tmp}/moira-supply.XXXXXX")
expected=$(mktemp "${TMPDIR:-/tmp}/moira-supply-expected.XXXXXX")
trap 'rm -f "$actual" "$expected"' EXIT

awk -F '|' '$1 == "component" { print }' "$baseline" | LC_ALL=C sort > "$expected"
[[ -s "$expected" ]] || { echo 'Moira supply inventory is empty' >&2; exit 1; }
awk '
  /^\[\[package\]\]$/ { if (name != "") print "component|" name "|" version; name=""; version=""; next }
  /^name = / { value=$0; sub(/^name = "/,"",value); sub(/"$/,"",value); name=value }
  /^version = / { value=$0; sub(/^version = "/,"",value); sub(/"$/,"",value); version=value }
  END { if (name != "") print "component|" name "|" version }
' Cargo.lock | LC_ALL=C sort > "$actual"
diff -u "$expected" "$actual" || { echo 'Moira locked component inventory drifted' >&2; exit 1; }
expected_git=$(mktemp "${TMPDIR:-/tmp}/moira-git-expected.XXXXXX")
actual_git=$(mktemp "${TMPDIR:-/tmp}/moira-git.XXXXXX")
trap 'rm -f "$actual" "$expected" "$expected_git" "$actual_git"' EXIT
awk -F '|' '$1 == "git-source" { print $2 }' "$baseline" | LC_ALL=C sort -u > "$expected_git"
awk -F '"' '/^source = "git\+/ { print $2 }' Cargo.lock | LC_ALL=C sort -u > "$actual_git"
diff -u "$expected_git" "$actual_git" || {
  echo 'Moira locked Git source inventory drifted' >&2
  exit 1
}
awk '
  /^\[\[package\]\]$/ { if (registry && !checksum) exit 1; registry=0; checksum=0; next }
  /^source = "registry\+/ { registry=1 }
  /^checksum = "/ { checksum=1 }
  END { if (registry && !checksum) exit 1 }
' Cargo.lock || { echo 'Moira registry component lacks a checksum' >&2; exit 1; }
cargo metadata --locked --offline --format-version 1 | node ../keystone/scripts/check-cargo-license-baseline.mjs "$licenses"
echo "Moira supply profile closes over $(wc -l < "$expected" | tr -d ' ') locked components"
