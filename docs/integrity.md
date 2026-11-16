# SQLite physical integrity profile

Every current Moira SQLite owner executes bounded `PRAGMA quick_check(1)`
immediately after opening/configuring the connection and before trusting an
existing schema, ownership row, checkpoint, receipt or peer state. The result
must be exactly `ok`. A reported structural fault or SQLite read error refuses
open; the adapter does not initialize replacement state, run repair, or change
persistent journal/synchronization settings first.

`make check-physical-integrity` creates canonical legacy-workflow and saga
databases, closes their owners, flips the first byte of the final 4 KiB SQLite
page, synchronizes the corrupted file, and proves reopen refuses. It then proves
the failed attempt left the corrupted database byte-for-byte unchanged. The
ordinary workspace test gate also runs this qualification. The separately owned
`moira-agora-sqlite` and `moira-execution-sqlite` receivers use the same pre-trust
check exported by `moira-sqlite`; the
deterministic mutation corpus is anchored on the two canonical Moira storage
profiles rather than freezing peer-specific page layouts during their refactors.

This is a selected structural-page/index fault profile, not an arbitrary-bit or
exhaustive SQLite proof. `quick_check(1)` reports at most one fault and omits the
cross-index completeness work of full `integrity_check`; it does not authenticate
valid-looking malicious databases, detect rollback to an older valid database,
certify the filesystem/device, or simulate torn writes and power loss. Logical
owner/profile/revision/checkpoint guards, Aegis continuity evidence, stopped
backup/restore procedures, bounded atomically published online snapshots and
external storage monitoring remain independent.

On integrity refusal, operators must stop the owner, preserve the database and
side evidence, and restore only a previously qualified stopped backup or online
snapshot. A damaged source is never snapshotted as a repair path. Moira does not
repair in place or silently create a new workflow history.
