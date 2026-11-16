# Moira saga local owner

`moira-saga-local` is the sovereign production application for Moira's closed
durable saga service. It owns the SQLite store, private Unix listener, bounded
wire profile, and exact kernel UID/GID access registry. Ordinary read and
mutation access is explicit per mapping.

Operator reconciliation defaults to denial and is never inferred from an OS
account, Workbench supervision, or possession of the socket. The explicit
`operator.kind = "aegis"` profile connects through Aegis's public local client,
issues one short-lived grant on that exact session for a bounded list of saga
IDs, and injects Moira's command-scoped authorizer. A trusted Unix clock is
validated at startup and then advanced monotonically for the grant lifetime.
The Aegis socket, expected epoch, TTL, receipt subject and selected sagas remain
inside this opaque owner configuration; Workbench receives none of them.

The configuration is strict JSON, limited to 64 KiB, and uses distinct absolute
database and socket paths. Copy `config.example.json`, replace its example
paths, owner and effective UID/GID, then validate it without opening either
resource:

```sh
cargo run -p moira-saga-local --locked --offline -- --check config.json
```

Run it directly for signal-aware standalone ownership, or select
`--supervised` to expose the exact one-byte `x` drain channel used by Workbench.
EOF and every other byte do not authorize shutdown.

The real-process qualification in
`../../../workbench-tui/tests/live/moira_reconciliation.sh` proves exact
server-prepared confirmation in both shell directions, current selected-saga
Aegis authority, refusal without authority and no revision change, durable
receipt recovery after reopen, and clean Workbench-hosted drain.
