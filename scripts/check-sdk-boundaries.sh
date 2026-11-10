#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

tree=$(cargo tree -p moira-sdk-embedded --edges normal --depth 1 --locked --offline)
if [[ $(printf '%s\n' "$tree" | wc -l | tr -d ' ') != 2 ]] \
  || ! printf '%s\n' "$tree" | rg -q 'moira-core' \
  || printf '%s\n' "$tree" | rg -q 'moira-sqlite'; then
  printf '%s\n' "$tree" >&2
  echo 'Moira embedded SDK escaped its core-only closure' >&2
  exit 1
fi

echo 'Moira embedded SDK dependency closure verified'
