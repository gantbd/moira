# Moira local harness

This crate is the application-composition seam for one methodical local-model
deliberation. It does not own a second workflow engine.

The intended local path is:

```text
Rune method + SME/lattice corpus
              |
         Dendrite MCP
              |
       grounded Moira plan
              |
      durable Moira registry
              |
  authenticated Daimon model socket
              |
 independently attributed member reports
              |
 explicit proof / choice / maybe / mu
```

Use `moira_dendrite::DendriteClient::ground_plan` to create the exact plan,
`unix::connect_daimon` to connect to the selected `daimon-local-model` process,
and `run` with a `DeliberationStore` (normally `DeliberationSqliteStore`). The
caller supplies stable execution/report/effect/idempotency identities and an
already-authorized Chronos attempt mapping understood by Daimon.

`run` validates the complete cohort before plan admission, persists the plan,
executes every member, and commits only outputs that satisfy the closed
`moira.member-response.v1` contract. It returns malformed, refused, uncertain,
failed and non-completed outcomes without converting them into reports.
`resolve_explicit` accepts a terminal value supplied by the appropriate
validator or authority; it never derives one from model agreement.

The member model must return only this JSON shape as its output (typically via
a dedicated plain-text Daimon profile and a system instruction):

```json
{
  "schema": "moira.member-response.v1",
  "claims": [
    {
      "kind": "empirical",
      "statement": "claim text",
      "evidence_digests": ["lowercase SHA-256"],
      "dissent_digests": ["lowercase SHA-256"]
    }
  ]
}
```

The harness—not the model—computes the SHA-256 identity of each UTF-8 claim.
Unknown fields, unadmitted claim kinds, malformed evidence identities,
duplicates and bound violations refuse report admission. Provider configuration,
credentials, Aegis authorization and Chronos-attempt validation remain in
Daimon.

## Selected local process tools

`process_tool::ProcessToolExecutorV1` is the production local profile for an
already-authorized tool effect. Each capability is frozen to one canonical
absolute executable, fixed argument vector and canonical working directory.
It invokes no shell, clears the inherited environment, writes the model's
bounded structured arguments to stdin, and verifies the executable digest
again before every dispatch.

Stdout and stderr are drained concurrently into bounded in-memory prefixes, so
a tool can neither deadlock on a full pipe nor grow an unbounded temporary
file. Elapsed time, input, stdout, stderr and executable size all have explicit
profile limits. Success evidence binds the Moira workflow/effect/execution,
Aegis decision, Chronos attempt, capability, executable digest, arguments,
exit disposition and captured streams. Timeout, nonzero exit and output-limit
events are attributable failures; any ambiguous post-spawn I/O or process
state is `OutcomeUnknown` and therefore fenced rather than retried.

The executor composes with `tool_runtime::dispatch_ready`: dispatch is durable
before process admission, successful bytes are content-addressed and durable
before the continuation receipt, and restart reconciliation never blindly
reinvokes the process. Dynamic command construction, inherited credentials and
secret injection are deliberately outside this profile.

`tool_turn::execute_tool_turn` closes the next orchestration seam without
absorbing peer authority. It replay-admits one provider-native turn, asks the
caller for a fresh independent Chronos observation and Aegis decision for each
call, authorizes and dispatches calls in provider order, and returns exact
durable results for Daimon resume only after the saga is complete. Execution
identities are domain-separated from workflow/effect identities inside the
harness, making them stable across restart. A retained in-flight call is
reconciled from the output store; missing output fences the turn and is never
converted into another invocation.

## Selected peer effects

`peer_turn::project_peer_proposals` separates model output from deployment
facts. It preserves Agora/Vellum proposal order, excludes tool calls, and asks
trusted composition for the exact retained source extent of every Vellum edit.
`peer_turn::execute_peer_turn` then replay-admits that vector into the durable
model saga and advances one peer effect at a time. Only a matching peer receipt
is success. Unknown outcomes fence the tail and can enter only the port's
reconciliation method; a completed replay performs no peer call.
After admission, `peer_turn::resume_peer_turn` reads the retained saga and
storage revision itself through the trusted service port before any peer call.
It therefore resumes after SQLite reopen without reconstructing proposals or
asking Aegis for another decision, and it cannot dispatch a caller-forged saga
view. An absent or unreadable retained saga makes no peer call.

`action_selection` closes the admission gate before that projection. A
single-member chatbot must bind its exact retained report/result to a nonzero
deployment policy digest. A multi-member workflow or agent must already have a
terminal `AuthorizedChoice` resolution and obtain a separate action-authority decision echoing the
exact checkpoint, member, report, output and Daimon effect. The derived digest
is retained by `MOIRAMP2`; changing the selection conflicts on replay. This does
not reinterpret a Pareto candidate as a model report, and proof/maybe/mu do not
silently authorize one. The authority decision is itself replay identity: the
caller retains the selected-action artifact for retry instead of asking Aegis
for a fresh decision and pretending it is the original action.

`peer_router::FederatedPeerRouterV1` performs the deployment-owned projection.
Models provide only opaque conversation or document references and content.
Agora room IDs, expected revision, logical time, source, room sequence and
audience come from the route catalogue. Vellum workspace/document IDs come from
`EditAdapter` routes, and the exact source extent is supplied during trusted
proposal projection. On uncertainty, Agora may replay only its durable prepared
request; Vellum calls historical receipt lookup only. The chatbot journey test
composes this router with action selection, `MOIRAMP2`, the model-saga SQLite
store and the Agora publisher SQLite store, proving one message under lost reply
and no peer contact on completed replay. The ensemble journey uses the real
Aegis public client/service twice across distinct contracts: the resolution
coordinator first authorizes one candidate on the computed Pareto frontier, then
the action selector separately authorizes one exact retained report/result.
Their decision identities differ. Only then does the journey route its edit
through Vellum, lose the execute reply, and prove recovery performs historical
lookup only. Replaying the original selection performs neither operation again.

This driver deliberately does not choose an ensemble member. All expert reports
remain evidence. A workflow must first select the one terminal result authorized
to act, then provide product-owned Agora publication routing and Vellum document
routes. The strict operator has not yet composed those routes or durable Agora
publisher routing, so it does not execute peer proposals merely because a
report was retained.

`moira-agora::durable` supplies the store-independent publisher protocol needed
by that future composition: prepare the exact Agora request before dispatch,
then retain its exact outcome before exposing the receipt. It safely replays an
indeterminate local outcome commit without a second peer call. The concrete
bounded SQLite owner is `moira-agora-publisher-sqlite`; operator routing remains
intentionally outside this harness crate.
