# Moira agent local

`moira-agent-local` advances one governed interactive-agent participation by at
most one durable step. It uses Moira's bounded participation socket and
Daimon's dedicated private agent socket; provider profiles, credentials,
authority, current-attempt validation, and prompt storage remain in their
owning services.

`--check` parses and validates the complete JSON configuration without opening
a socket or SQLite database:

```text
cargo run -p moira-agent-local -- --check agent.json
```

At runtime the process loads one exact participation. Idle state returns
without contacting Daimon or opening output storage. A retained uncertainty
fence can only use provider inspection and events. Prepared work is checked,
durably fenced, and dispatched once. For a prompt operation, the worker obtains
bytes only from the participation daemon using the exact current participation,
revision, and digest; it closes that connection and reconnects before the
currency check and dispatch fence. It never opens the daemon's prompt database.

Output is a content-free JSON summary containing only the step outcome and
participation revision. Run the process again only under deployment-owned
bounded scheduling; a fenced outcome is not permission to redispatch.

The real-process qualification launches the actual binary against Moira's
participation daemon and Daimon's agent daemon. Its open-session case proves
policy identity derivation and one observed transition. Its prompt case proves
worker-only materialization, exact prompt projection, cursor-contiguous durable
assistant output, and terminal-evidence correlation.

The mixed-provider qualification runs two actual worker processes against one
provider-neutral Daimon service: an ACP historian and a Claude-stream rigor
reviewer. Both traverse durable open and prompt operations, retain distinct
provider evidence, and admit their closed terminal outputs into one affirmative
orthogonal Moira ensemble. The providers are hermetic process fixtures; this is
composition and governance evidence, not a claim about live model behavior.

Run the complete offline qualification with `make qualify-agent-local` from the
Moira workspace root. The real-process cases require permission to create Unix
sockets in the host temporary directory.
