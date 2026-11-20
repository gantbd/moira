# Compatibility profile

Status: package/source topology plus canonical live legacy/saga checkpoint and
logical SQLite V1 fixtures frozen; future cross-version compatibility remains
open until a second version exists.

`make check-compatibility-baseline` refuses unreviewed package-name, version,
manifest-path or fixture-digest drift against `compatibility-baseline.txt`.
The frozen topology keeps coordinator/saga persistence in `moira-sqlite`, Agora
local projection in `moira-agora-sqlite`, and the settlement-coupled Chronos/Daimon
projection in `moira-execution-sqlite`.
External canonical fixtures retain one admitted legacy workflow and one saga
after failed forward evidence plus explicit retry; active encoders must reproduce
them byte-for-byte and current decoders must recover exact replay. Digest-pinned
logical SQL fixtures independently reconstruct both V1 SQLite profiles and must
reopen with the same exact replay. They avoid binding compatibility to a bundled
SQLite page-layout version. Until a future schema/checkpoint version is exercised
against these V1 fixtures, Moira makes no general upgrade or downgrade promise.

The non-published `moira-qualification-aegis-process` package is frozen in this
source inventory but excluded from the root workspace. It is an assurance harness,
not a production dependency or public compatibility surface, and retains its own
lockfile so the Aegis/Keystone and Chronos storage implementations remain sovereign.
The non-published `moira-qualification-agora-recovery` package isolates Agora's
durable service and engine graph from the production adapter and root lock. It
is assurance-only and adds no public compatibility surface.
The likewise non-published `moira-qualification-chronos-recovery` package keeps
the real Chronos process restart fixture out of Moira's root resolution while
Chronos owns its vendored storage graph; it is also assurance-only.
The non-published `moira-qualification-hermes-process` package is likewise an
assurance-only process harness with its own lockfile; it adds no public Moira or
Hermes compatibility surface.
The non-published `moira-qualification-daimon-process` package similarly isolates
Daimon's durable daemon graph and discarded-reply recovery evidence. It is an
assurance harness, not a production dependency or compatibility surface.
The non-published `moira-qualification-live-execution-chain` package composes
separately built Aegis and Chronos helpers with Daimon, Hermes and Moira over
public client/wire contracts. Its 81-package lockfile isolates process-assurance
implementation graphs and does not merge Chronos's vendored store into the
Aegis/Daimon/Hermes production closures.

The private wire contract now has disjoint legacy-workflow and saga request/reply
frame kinds under the V1 magic. Their tags and field order are canonical source
contracts, but no cross-version byte fixture is frozen yet; changing them before
that fixture gate requires an explicit compatibility review and coordinated
client/server update.

The `moira-aegis` package is part of this frozen source topology. Its stable V1
saga resource mapping is eight ASCII domain-tag bytes (`MOIRASG1`) followed by
the big-endian saga identifier. Changing that mapping requires an explicit
compatibility decision because existing Aegis grants scope those exact bytes.

The `moira-unix-peer` package freezes only an exact deployment mapping from an
effective UID/GID pair to Moira `read`/`mutate` facts. Those numeric accounts and
mapping entries are deployment configuration, not portable protocol identity or
authority. Saga operator decisions remain outside this mapping.

The `moira-hermes` package is also frozen in this topology. Its workflow-event
payload begins with `MOIRAWF1`, followed by one closed transition tag and
canonical fixed-width fields. Its configured Hermes schema identifier is the
deployment-level schema reference; changing either the byte vocabulary or its
meaning requires an explicit compatibility decision and fixture update.

The `moira-agora` mapping selects Agora V2 `SpeakIdentified`. Its accepted
revision and message sequence map to Moira's publication receipt while the
configured source and room ordinal remain Moira deployment data. Changing that
mapping or substituting a non-identified Agora command requires an explicit
compatibility decision.

The model-proposal checkpoint now uses `MOIRAMP2`. V2 adds the required action-
selection digest to every saga, so the former unbound `MOIRAMP1` identity is
refused rather than silently assigned synthetic authority during recovery.

The task-agent journey `MOIRATJ1` checkpoint encoder now emits version 7. V6
added a bounded exact Chronos task-namespace binding as a separate owner fact.
V7 adds the separately versioned Agora-actor-to-admitted-participation
relationship, including the complete retained participation binding, admission
authority attribution and content-free publication provenance. Decoders
continue to accept versions 1 through 6 and preserve their honest absence of
the later evidence. Re-encoding upgrades them to V7 without inventing a task ID
or actor relationship. V7 readers are required for checkpoints containing the
new relationship; no downgrade projection is claimed.

The `moira-chronos` mapping selects Chronos V3 atomic `SubmitGraph` with one task,
derives the invocation key as lowercase hexadecimal Moira effect bytes, and uses
V4 original-allocation evidence for the singleton task incarnation and accepted
revision on the same host-bound service. Allocation capture must be enabled
before admission; missing, expired or uncaptured history returns `HistoryLost`
without a V1 current-name snapshot fallback. Changing the key derivation, graph
shape, V3/V4 correspondence or receipt projection requires an explicit
compatibility decision because retained Chronos invocation and allocation
lookups depend on exact identity.

The `moira-daimon` mapping selects Daimon's native execute command and binds the
Moira effect to one exact execution ID, task incarnation and attempt number.
Daimon outcome uncertainty maps to Moira's `Uncertain` disposition. Changing
that causation or uncertainty mapping requires an explicit compatibility decision.
