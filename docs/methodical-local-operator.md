# Methodical local operator profile

`moira-methodical-local` is a one-shot composition executable. It does not run a
second provider, scheduler, authority service or corpus server.

## Required owners

Before execution, the deployment supplies:

1. Dendrite serving the selected Rune method, SME fold/surfaces and concern
   lattice. Required-auth deployments put the short-lived bearer in the
   configuration's named environment variable.
2. A running `daimon-local-model` composition. Its private model socket binds a
   locally hosted OpenAI-compatible endpoint to current Aegis authority and a
   current Chronos attempt. For member deliberation, use a plain-text provider
   profile and a system instruction requiring only `moira.member-response.v1`;
   Moira validates the result after Daimon returns it.
3. A unique Moira SQLite path and stable 16-byte owner identity.
4. Stable member, report, effect, execution and idempotency identities. Every
   run must refer to a task attempt, principal, profile and context already
   understood by Daimon.

The checked-in [example configuration](../examples/methodical-local.example.json)
uses the real architecture fold/lattice names but placeholder deployment
identities. Replace every principal, profile, task and context identity with the
corresponding configured Daimon values before execution.

## Product profiles

The top-level `product` field is a closed capability profile. It shapes the
composition admitted by the operator; it does not change the meaning of a
terminal epistemic result:

- `chatbot` requires exactly one grounded member. A peer action uses
  `selection: { "kind": "single_member", "policy_digest": "..." }`, where the
  digest is a nonzero lowercase 64-hex deployment-policy identity, and omits
  `aegis`. The policy selects the sole answer for action; it does not prove that
  answer true.
- `workflow` is the default and requires at least two productively orthogonal
  grounded members. A peer action requires an `authorized_choice` resolution,
  `selection: { "kind": "authorized" }`, and a separate Aegis action selector.
- `agent` has the workflow requirements and additionally requires at least one
  advertised tool capability. Each tool call still needs its own exact Aegis
  decision and current Chronos attempt before execution. Its peer action uses
  the same separately authorized selection as a workflow.

All three profiles retain Rune method/state identity, Dendrite grounding,
evidence-bound reports and the same distinct proof, authorized-choice, maybe and
mu terminal families. Product shape, epistemic resolution, action selection and
tool execution authority are four different boundaries.

Orthogonality is pairwise. A chatbot singleton therefore carries deterministic
vacuous-pairwise evidence pinned to Dendrite's current lattice content identity;
it does not claim that Rune's two-or-more-role lattice checker ran. Workflow and
agent ensembles still require Dendrite's actual affirmative lattice verdict.

## Validate without peers

```bash
cargo run -p moira-methodical-local -- --check examples/methodical-local.example.json
```

This reads at most one MiB, rejects unknown fields and checks closed enums,
lowercase fixed-width identities, cohort equality, uniqueness, dimensions and
nonzero bounds. When `tooling` is present it additionally requires an exact
match between run-advertised capabilities and fixed process profiles, an exact
Chronos task binding for every tool-enabled run, three distinct stores/owners,
absolute executable and working-directory paths, bounded argv/process limits,
adequate output-store capacity and explicit tool-turn fuel. It does not read a
bearer secret, inspect executables, connect to a socket, mutate a database or
contact Dendrite, Aegis, Chronos, Daimon, Agora or Vellum.

When `peer_action` is present, validation additionally requires one exact
retained member, a stable peer workflow identity, distinct
deliberation/model-saga/publisher stores and owners, bounded Agora and Vellum
transports, and duplicate-free deployment-owned route catalogues. A chatbot
requires its single-member policy selection and no Aegis selector; a workflow
or agent requires an `authorized_choice` resolution and separate Aegis action
selection. The Vellum local-authentication claim is named by environment
variable and is not read during validation.

## Execute once

```bash
cargo run -p moira-methodical-local -- examples/methodical-local.json
```

The executable then:

1. obtains the optional Dendrite bearer from the named environment variable;
2. resolves and pins the Rune method, authored context tag, fold, surfaces and
   concern lattice into one exact orthogonal plan;
3. opens the separately owned deliberation SQLite profile;
4. connects to Daimon's authenticated private model socket;
5. validates the entire run cohort before admission, executes independently and
   commits only valid reports;
6. for a native tool continuation, replay-admits its calls into the dedicated
   continuation store, observes the exact running Chronos attempt, requests an
   exact capability-scoped session-local Aegis grant/decision, executes the
   fixed shell-free process profile, durably stores result bytes, and resumes
   the same Daimon member until terminal response or bounded turn exhaustion;
7. when an optional closed `resolution` profile is present and every report is
   retained, resolves through Rune proof, Aegis choice, explicit four-valued
   maybe or reasoned mu; without that profile it remains ready but unresolved;
8. when `peer_action` is present, selects its exact terminal report through the
   profile's single-member policy or a second Aegis `Administer` decision,
   projects only that result's Agora/Vellum proposals with trusted route and
   source-extent facts, and settles them through the durable `MOIRAMP2` saga;
9. prints a JSON summary containing counts, readiness, resolution and peer-turn
   disposition, but no claim content or secret.

## Optional terminal resolution

`resolution.kind` preserves the four terminal families instead of asking a
model to collapse them:

- `proven_bool` names an absolute Rune theorem program, bounded arguments,
  validator identity, timeout and output ceiling. The exact grounded question
  digest is supplied by the operator; `hold_quantum` or `mu` cannot be promoted
  to Boolean.
- `authorized_choice` names one configured candidate and a local Aegis profile.
  The resolution coordinator recomputes the Pareto frontier and accepts the
  candidate only with an exact `Moira/Administer` decision for this
  deliberation.
- `maybe` accepts only `true_only`, `false_only`, `both` or `neither`.
- `mu` accepts only the closed reasons `fuel_exhausted`, `missing_evidence`,
  `contradictory_evidence`, `wrong_question`, `wrong_ensemble` or
  `no_authorized_selector`.

Resolution is optional and occurs only after the full report set is durable.
Matching resolved replay is served from the deliberation store without running
Rune or reconnecting to Aegis. A changed terminal family, selected candidate,
Aegis subject/epoch or explicit value refuses as resolution drift.

An orthogonal cohort produces competing evidence, not one product action per
member. For workflow and agent profiles, `peer_action.member` identifies the
single terminal report eligible for action only after the ensemble has an
`authorized_choice` resolution. Candidate choice and action selection are
separate Aegis decisions. A chatbot's explicit policy digest selects its only
member without inventing a second member or an epistemic choice. Proof, maybe
and mu cannot authorize ensemble peer effects.

The peer profile owns a model-saga store, an Agora publisher checkpoint store,
an Agora V2 Unix participant and conversation route catalogue, and an
authenticated Vellum Unix client plus document/source-extent catalogue. Model
output carries only opaque external references and proposal bytes; it cannot
choose deployment identities, revisions, audiences or document extents. A
completed peer saga is returned from SQLite before reading the Vellum claim or
contacting Aegis, Agora or Vellum. Pending or uncertainty-fenced sagas resume
their retained proposal vector without obtaining a new action decision;
uncertain Vellum effects use historical lookup, and Agora uses its retained
exact request.

The optional `tooling` object owns:

- separate `continuation_store` and `output_store` paths and owner identities;
- an Aegis Unix socket, expected epoch, expiry and attribution subject;
- a Chronos owner-only local directory plus exact external-key/task-id bindings;
- one absolute, fixed process profile for every advertised capability;
- one policy-owned elapsed/output budget, poll interval and `max_turns` fuel.

The Aegis Unix peer must be configured as an administrator able to issue the
requested exact scopes; a configured handle cannot be reused because Aegis
grants are session-local. Authority connection, grant issuance and executable
canonicalization are lazy: a fully completed replay requires none of them and
depends only on the retained continuation/output stores. Chronos observation is
read-only and likewise occurs only for a pending effect. Dynamic commands,
inherited environments and secret injection are not accepted by this profile.

## Live qualification

With a selected Dendrite instance already serving the real workspace corpus:

```bash
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 make qualify-methodical-profiles
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 make qualify-methodical-chatbot-peers
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 MOIRA_METHODICAL_LIVE_MODEL=gemma3:27b make qualify-methodical-chatbot-live-provider
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 MOIRA_METHODICAL_LIVE_MODEL=gemma3:27b make qualify-methodical-workflow-live-provider
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 MOIRA_METHODICAL_LIVE_MODEL=gemma3:27b make qualify-methodical-workflow-peer-live-provider
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 MOIRA_METHODICAL_LIVE_MODEL=qwen3:32b make qualify-methodical-agent-live-provider
MOIRA_METHODICAL_LIVE_DENDRITE_PORT=4498 make qualify-methodical-tools
```

`qualify-methodical-profiles` runs the same strict executable composition for
all three capability profiles. The chatbot resolves the authored BDD
`formulation`/`test-oracle-design` route to one ambient tester surface and one
private Daimon completion. The workflow resolves the four-member architecture
cohort and retains four reports. The agent repeats that cohort with an
advertised tool and crosses live Aegis authority, a current Chronos attempt, the
fixed process, durable tool stores and Daimon resume before retaining its fourth
terminal report. The model services are controlled; this is profile-composition
evidence, not a live third-party provider claim.

`qualify-methodical-chatbot-peers` extends the chatbot profile through the full
operator. One live Rune/Dendrite-grounded report arrives over Daimon's actual
private model wire with Agora and Vellum proposals, is bound to the nonzero
single-member action policy, publishes through Agora's actual local daemon and
performs one authenticated Vellum replace-and-save. Vellum's durable-effect
counter remains exactly one. The controlled model service isolates composition;
it is distinct from the live-provider gate.

`qualify-methodical-chatbot-live-provider` runs the same grounded chatbot and
peer path with Daimon's actual OpenAI-compatible provider adapter inside the
qualification's private model daemon. The frozen provider profile uses explicit
zero-temperature sampling and binds that policy into its digest. Against an
already-running loopback Ollama `gemma3:27b`, two consecutive qualification runs
completed with one durable Vellum effect each. This proves combined first
execution; it does not substitute for the full `daimon-local-model`
Aegis/Chronos composition or the multi-process fault matrix.

`qualify-methodical-workflow-live-provider` runs the four-member architecture
cohort—architecture theorist, network rigor reviewer, identity idea stormer and
data historian—through the same actual adapter and private model wire. Each
request carries exactly one persona context, and the adapter projects that
context into an explicit bounded epistemic-lens instruction. Rune and
Dendrite supply the authored method, fold surfaces and affirmative lattice
evidence before dispatch. The provider must return four exact, distinct closed
report envelopes; Moira retains all four and stops ready-to-resolve. The gate
deliberately obtains no choice authority and emits no peer effect, so correlated
model completion cannot masquerade as proof or an authorized workflow decision.

`qualify-methodical-workflow-peer-live-provider` continues that same ensemble
through the authority boundary. Its two-axis, four-candidate plan removes two
dominated alternatives while preserving two incomparable frontier survivors.
One actual Aegis service authorizes one of those retained Pareto candidates; a
separate actual Aegis service authorizes the exact selected
member report to act. Only then are that report's typed proposals admitted and
sent through actual Agora and authenticated Vellum daemons. The gate observes
one publication and exactly one durable Vellum edit/save. This separation proves
that model agreement, candidate choice and permission to act remain three
different facts.

`qualify-methodical-agent-live-provider` uses a tool-capable frozen profile. The
actual `qwen3:32b` provider originates one native function call for the first
ensemble member. Moira then obtains independent exact-capability Aegis authority
and the current Chronos attempt, runs the fixed shell-free `/bin/cat` profile,
commits output and continuation state, and resumes the provider with its original
call correlation plus matching assistant/tool messages. Only the resumed closed
report is promoted; the complete four-member agent ensemble again stops
ready-to-resolve. This is live-provider tool continuation evidence, not action
selection, Agora/Vellum publication, or process-loss qualification.

The journey starts an authenticated real Aegis service/Unix daemon, a bounded
Chronos local-wire snapshot peer and Daimon's private model daemon. It grounds
the real four-member architecture cohort, obtains exact gate facts, runs the
fixed `/bin/cat` profile, commits its output through both SQLite stores, resumes
the originating member and admits the terminal fourth report at deliberation
revision 5. The model service is controlled; provider-native parsing and real
OpenAI-compatible continuation is qualified in Daimon. The chatbot live-provider
gate also runs an actual Ollama-backed completion through the complete operator
and peer path. The workflow live-provider gate covers unresolved ensemble report
collection, and the agent gate covers one authorized native tool continuation.
The live-provider workflow peer effect is covered; an agent peer effect remains
separate from its tool-continuation qualification.

`make qualify-methodical-peers` runs the extracted operator composition seam
against actual local Aegis, Agora and authenticated Vellum daemons. One resolved
two-member result publishes an Agora message and durably replaces and saves a
Vellum document; Vellum observes exactly one durable effect. The daemons are
then gone and matching `MOIRAMP2` replay still completes from SQLite with an
empty outcome vector, proving that it does not reselect, read the Vellum claim
or contact authority or either peer. Multi-process kill qualification during
the combined operator journey remains open.

Rerunning the same configuration is an exact replay only if the retained plan,
all identities and Daimon inputs still match. Changes require fresh identities.
An uncertain execution, invalid member envelope or report-commit failure is not
converted to completion. Inspect and reconcile the owning store/service before
choosing another action.

## Member output

The model output must be exactly the closed envelope documented in the
[local harness guide](../crates/moira-local-harness/README.md). Evidence and
dissent values are lowercase SHA-256 references supplied in the model's bounded
context. The harness computes claim-text identities itself. Model agreement is
not proof or selection authority.
