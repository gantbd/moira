#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# Read direct normal dependencies without resolving the complete graph. The
# isolated process qualification combines Chronos's vendored and Keystone's
# shared, same-name storage packages only in separate lockfiles.
metadata=$(cargo metadata --no-deps --format-version 1 --locked --offline)

direct_dependencies() {
  local package=$1
  printf '%s\n' "$metadata" | node -e '
    let input = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", chunk => input += chunk);
    process.stdin.on("end", () => {
      const packageName = process.argv[1];
      const pkg = JSON.parse(input).packages.find(candidate => candidate.name === packageName);
      if (!pkg) {
        console.error(`package not found: ${packageName}`);
        process.exit(1);
      }
      process.stdout.write(pkg.dependencies
        .filter(dependency => dependency.kind === null)
        .map(dependency => dependency.name)
        .sort()
        .join(","));
    });
  ' "$package"
}

expect_dependencies() {
  local package=$1
  local expected=$2
  local actual
  actual=$(direct_dependencies "$package")
  if [[ "$actual" != "$expected" ]]; then
    echo "$package direct production dependencies: $actual" >&2
    echo "$package expected production dependencies: $expected" >&2
    exit 1
  fi
}

expect_dependencies moira-sdk-embedded 'moira-core'
echo 'Moira embedded SDK dependency closure verified'

expect_dependencies moira-aegis 'aegis-client,aegis-protocol,moira-core,moira-host,moira-protocol,moira-service,sha2'
echo 'Moira Aegis adapter public-client dependency closure verified'

expect_dependencies moira-dendrite 'moira-core,serde_json,sha2'
echo 'Moira Dendrite grounding adapter dependency closure verified'

expect_dependencies moira-rune-validator 'moira-host,serde,serde_json,sha2'
echo 'Moira Rune validator adapter dependency closure verified'

expect_dependencies moira-local-harness 'agora-actor-publication-port,agora-client,daimon-client,daimon-wire,moira-agora,moira-client,moira-core,moira-daimon,moira-host,moira-protocol,moira-vellum,sha2,vellum-protocol'
echo 'Moira governed local harness dependency closure verified'

expect_dependencies moira-methodical-local 'aegis-client,aegis-local-daemon,aegis-protocol,aegis-wire,chronos-local,daimon-protocol,daimon-wire,moira-aegis,moira-agora,moira-agora-local,moira-agora-publisher-sqlite,moira-chronos,moira-client,moira-core,moira-daimon,moira-dendrite,moira-host,moira-local-harness,moira-protocol,moira-rune-validator,moira-service,moira-sqlite,moira-vellum,serde,serde_json,sha2,vellum-protocol,vellum-transport'
echo 'Moira methodical operator composition dependency closure verified'

expect_dependencies moira-agora 'agora-actor-publication-port,agora-client,agora-protocol,moira-protocol'
echo 'Moira Agora adapter public-client dependency closure verified'

expect_dependencies moira-agora-local 'agora-client,agora-protocol,agora-wire'
echo 'Moira Agora local transport boundary verified'

expect_dependencies moira-chronos 'chronos-client,chronos-protocol,moira-protocol'
echo 'Moira Chronos adapter public-client dependency closure verified'

expect_dependencies moira-daimon 'daimon-client,daimon-protocol,moira-client,moira-core,moira-host,moira-protocol,serde,serde_json,sha2'
echo 'Moira Daimon adapter public-client dependency closure verified'

expect_dependencies moira-hermes 'hermes-client,hermes-protocol,moira-protocol'
echo 'Moira Hermes adapter public-client dependency closure verified'

expect_dependencies moira-vellum 'moira-protocol,vellum-client,vellum-protocol'
echo 'Moira Vellum adapter public-client dependency closure verified'

expect_dependencies moira-local-daemon 'keystone-framing,keystone-unix-listener,moira-host,moira-protocol,moira-service,moira-wire'
echo 'Moira local daemon production peer isolation verified'

expect_dependencies moira-unix-peer 'moira-local-daemon,moira-protocol,moira-service,nix'
echo 'Moira Unix peer authentication dependency closure verified'

expect_dependencies moira-sqlite 'keystone-codec,moira-core,moira-host,rusqlite,sha2'
echo 'Moira coordinator/saga SQLite dependency closure verified'

expect_dependencies moira-agora-sqlite 'agora-engine,moira-core,moira-sqlite,rusqlite'
echo 'Moira local Agora receiver package boundary verified'

expect_dependencies moira-agora-publisher-sqlite 'agora-client,agora-protocol,moira-agora,moira-protocol,moira-sqlite,rusqlite,serde,serde_json,sha2'
echo 'Moira Agora publisher checkpoint boundary verified'

expect_dependencies moira-execution-sqlite 'aegis-authority,chronos-engine,daimon-harness,keystone-codec,moira-core,moira-sqlite,rusqlite'
echo 'Moira local execution receiver package boundary verified'
