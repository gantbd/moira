# Moira–Vellum process qualification

This isolated workspace qualifies `moira-vellum` against Vellum's real
authenticated Unix daemon without adding Vellum owner internals to Moira's root
dependency graph.

The test creates a normal saved source baseline, maps one stable Moira effect to
Vellum's revision- and extent-bound durable replace-and-save operation, discards
the accepted reply after the owner has processed it, and reconnects through a
fresh public client. Recovery uses `lookup_durable_receipt` only and verifies the
replacement appears exactly once. Absence is never evidence to execute again.

Run it with:

```sh
make qualify-vellum-process
```
