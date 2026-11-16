# Moira saga local owner

`moira-saga-local` is the sovereign production application for Moira's closed
durable saga service. It owns the SQLite store, private Unix listener, bounded
wire profile, and exact kernel UID/GID access registry. Ordinary read and
mutation access is explicit per mapping.

Operator reconciliation is deliberately denied by this selected application.
It is not inferred from an OS account, Workbench supervision, or possession of
the socket. A future authority-bearing profile must inject Moira's independent
command-scoped `OperatorAuthorizer` and retain its own Aegis configuration.

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
