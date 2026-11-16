# Moira local operator runbook

Status: qualified for stopped backup/restore and bounded online snapshots of the
selected legacy-workflow and saga SQLite profiles. Installation packaging and
schema upgrade remain release gates.

## Invariants

- One process owns each database and its adjacent owner lock.
- Owner identity, codec/domain limits and database profile are deployment
  configuration and must be restored unchanged.
- A database file, checkpoint, peer receipt or correlation identifier grants no
  authority.
- Never infer success from timeout or transport loss. Reopen and use retained
  replay/read evidence.
- Never replace a refused or corrupt database with a fresh one at the same path.

## Stopped backup

1. Stop the Moira daemon and wait for the owner process to exit. This profile has
   no online-backup claim.
2. Confirm the adjacent persistent `*.owner.lock` is no longer held, for example
   by a successful same-owner reopen/close. The file is an identity sentinel and
   normally remains after shutdown; do not copy it.
3. Copy the database as one regular file to a new backup path; preserve restrictive
   permissions and record a digest through deployment tooling.
4. Record the exact owner identity, selected profile, codec/domain limits and
   application release separately. These are required inputs, not discoverable
   authority from the database.
5. Retain the source until restore qualification completes.

`moira-sqlite` uses rollback-journal mode and `synchronous=EXTRA`; copying is
qualified only after the owning store has closed. The executable
`offline_backup_restore` test proves byte-exact copy, reopen, retained effect
identity and exact replay for both profiles.

## Restore and recovery

1. Restore into a new empty private directory; do not overwrite a live store.
2. Supply the original owner identity and exact limits. Owner/profile mismatch,
   missing state and invalid checkpoints must refuse without reinitialization.
3. Open read-first and inspect revision, saga/workflow phase, pending effect and
   fence state before enabling mutation traffic.
4. Replay the original stable command/effect identity to close an unknown reply.
   A replay receipt is evidence of the retained commit; a changed command is a
   conflict, not a retry.
5. For a fenced saga, use the bounded reconciliation view and retained evidence.
   Any disposition requires a fresh exact-saga Aegis authorization and is stored
   with decision/epoch causation.
6. Keep the old database and backup until application-level reconciliation and
   peer receipt checks succeed.

## Structural integrity refusal

Every reopen runs bounded SQLite `quick_check(1)` before existing state is
trusted. If it refuses, stop all attempts against that path and preserve the
database, owner-lock sentinel, application version, limits, filesystem evidence
and any storage diagnostics. Do not run in-place SQLite repair, delete rows,
change pragmas or allow a fresh database to replace the file.

Restore a qualified stopped backup into a new private directory and follow the
recovery procedure above. The check detects selected structural page/index
damage; a passing result is not proof against valid-looking rollback, malicious
content, torn-write histories or failing storage hardware.

## Online backup

Both `SqliteStore::backup_online` and `SagaSqliteStore::backup_online` create a
bounded SQLite online snapshot while the single owner remains open. Select a new
destination leaf in a trusted private directory; neither that leaf nor its
`.partial` sibling may already exist. Configure positive page/step ceilings and
a bounded retry delay. The operation writes mode `0600` partial output, refuses
when its step budget is exhausted, verifies SQLite structural integrity and the
source file identity, fsyncs the file, then atomically renames and syncs the
parent directory. Failure removes only the partial file created by that call and
never replaces an existing destination.

`BackupPublicationUncertain` means the atomic rename made the destination
visible but syncing its parent directory failed. Do not retry blindly: inspect
and qualify that destination, then either retain it as the one published
snapshot or remove it explicitly before starting a new backup. This result does
not imply that publication failed or that the destination is durable across a
host crash.

The receipt identifies the exact source storage revision represented by the
snapshot. The live owner may continue accepting commands after publication; the
snapshot intentionally does not contain those later revisions. Reopen a copy
with the original owner identity and exact profile limits and verify retained
replay before treating it as recoverable evidence. This profile is a synchronous
single-process operator action, not concurrent writer/load qualification,
continuous archiving, remote-filesystem backup, encryption, rollback detection
or migration.

## Upgrade and migration

The current native profiles are V1 and have no in-place V2 migration. The
digest-pinned logical V1 fixtures are future upgrade inputs, not evidence that an
unimplemented migration works. A future version must open copies of those
fixtures, perform an explicit atomic mapping, retain replay/effect identities and
prove rollback/refusal behavior before this gate can close.
