# Methodical peer live qualification

This isolated workspace proves the `moira-methodical-local` peer-action
composition against actual local daemon implementations without adding Agora or
Vellum server internals to Moira's production dependency graph.

The journey starts from a resolved two-member checkpoint, obtains a fresh Aegis
action-selection decision, publishes through Agora's V2 Unix daemon, performs a
durable authenticated Vellum replace-and-save, and completes the `MOIRAMP2`
saga. It then removes all three daemons and repeats with an empty outcome vector;
matching completed replay must return solely from the model-saga SQLite owner.

With a live Dendrite corpus, the same isolated workspace also qualifies the
complete chatbot operator path. It resolves the authored Rune method and one
ambient tester surface, runs one private-wire Daimon completion containing
Agora and Vellum proposals, selects the sole report through a nonzero deployment
policy digest, and crosses the actual Agora and authenticated Vellum daemons.
Vellum observes exactly one durable effect. This first-execution journey and the
offline peer-saga replay above are distinct evidence; combined process-loss
injection across the full top-level journey remains open.

Run it through the repository gate:

```sh
make qualify-methodical-peers
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 make qualify-methodical-chatbot-peers
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 MOIRA_METHODICAL_LIVE_MODEL=gemma3:27b make qualify-methodical-chatbot-live-provider
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 MOIRA_METHODICAL_LIVE_MODEL=gemma3:27b make qualify-methodical-workflow-live-provider
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 MOIRA_METHODICAL_LIVE_MODEL=gemma3:27b make qualify-methodical-workflow-peer-live-provider
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 MOIRA_METHODICAL_LIVE_MODEL=qwen3:32b make qualify-methodical-agent-live-provider
```

The live-provider gate replaces the controlled chatbot service with Daimon's
actual OpenAI-compatible adapter, hosted behind the same private model wire. Its
credential-free profile binds explicit zero-temperature sampling. It requires
an already-running loopback provider and proves combined first execution, not
the full `daimon-local-model` Aegis/Chronos composition or process-loss recovery.

The workflow live-provider gate resolves the real four-member Rune/Dendrite
architecture cohort with distinct theorist, rigor-reviewer, idea-stormer, and
historian persona contexts projected into the provider requests. It obtains four
exact and distinct model reports through the private Daimon wire and retains
them in the durable Moira registry. It stops at
`ready_to_resolve`: it does not infer proof, choose a Pareto candidate, or emit
Agora/Vellum effects without separate authority.

The workflow-peer gate supplies that authority without collapsing its layers.
Its two-axis plan removes two dominated candidates and retains two incomparable
frontier survivors; one Aegis daemon authorizes one surviving choice, a second
authorizes the exact retained report to act, and the selected actual-provider
proposals then cross real Agora and Vellum daemons. Exactly one durable Vellum
effect is observed.

The agent live-provider gate requires a model with native tool support. It
crosses a real provider-originated function call, exact Aegis capability
authority, a current Chronos attempt, fixed shell-free process execution, both
durable tool stores and provider-native resume. The intermediate call is never a
member report; only the terminal resumed envelope is admitted. The resulting
four-report ensemble remains unresolved and emits no peer effect.
