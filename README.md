# Moira

Moira is the planned durable workflow coordinator for composing Agora, Chronos,
Daimon, Vellum, Hermes, and Aegis without absorbing their domain semantics.

The repository now contains a deterministic in-memory workflow slice from an
Agora message reference through Chronos admission, one exact Chronos attempt, an
attributable Daimon result, and replay-safe Agora response publication. Each
boundary retains one stable effect identity; matching input/receipt replay is an
identity result and conflicting replay is refused. An uncertain harness result
fences automatic progress. A validated in-memory checkpoint transfer preserves
every effect identity across engine reconstruction. It does not yet claim an
encoded durable store, atomic persistence, production connector correctness, or
Vellum publication.

Semantic recovery now rejects rewound/skipped allocation frontiers, missing
replay-index entries, duplicate/foreign allocations, inconsistent revisions and
misattributed retained receipts. Valid phase/disposition interleavings retain
their identities. This is a prerequisite for storage, not durable recovery itself.

`Engine::apply_observed` optionally records closed command/result classifications
through Keystone's preallocated bounded capture buffer. Counters distinguish new
acceptance, matching replay, refusal and newly accepted harness receipts; current
state gauges report workflow phases and pending effects. Disabled, available and
overloaded capture preserve the same domain results. Capture loss is explicit and
counters saturate without wrapping. Observations contain no content or identity
labels, read no clock and establish neither latency nor durable/audit evidence.
Hosts remain responsible for timing storage and connector operations.

Qualification tests execute the actual neighboring Chronos, Daimon, Agora,
Hermes, and Aegis volatile contracts. Aegis decision evidence authorizes the
Daimon request, and Hermes producer-key replay retains one workflow event after a
lost append reply. These are compatibility proofs, not production connectors.

## Boundary

Moira owns cross-engine correlation, workflow phases, effect intents, attributed
receipts, and terminal workflow outcomes. Chronos owns task scheduling and leases;
Agora owns conversations; Daimon owns harness execution; Vellum owns document
publication; Hermes owns event delivery; Aegis owns authority evidence.

The first target is one recoverable vertical slice:

```text
addressed Agora message -> Chronos task -> fake Daimon result -> Agora reply
```

Restart at every boundary must produce neither a duplicate task nor a duplicate
reply. See [requirements](requirements.md), [specification](specification.md), and
the [roadmap](docs/roadmap.md).

## Development

```sh
make check
```

