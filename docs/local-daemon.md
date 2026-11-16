# Private local daemon profile

Status: qualified bounded Unix connection profiles and selected local OS-account
authentication for the native legacy and closed saga services. Executable
packaging, supervision, remote identity and deployment grant provisioning remain open.

`moira-local-daemon` binds only a new filesystem path, creates a mode-`0600` Unix
socket and refuses replacement of an existing entry. The embedding launcher owns
trusted-parent selection. An authentication adapter derives explicit `read` and
`mutate` facts from the accepted peer before any service command is decoded or
dispatched. Those facts are not credentials, workflow authority or durable state.

After authentication the daemon sends a canonical V1 ready frame declaring the
frame and per-connection request ceilings. It applies finite read/write deadlines,
accepts a configured finite sequence of bounded length-prefixed requests, and
closes cleanly only at an inter-frame EOF. A partial frame is `BeforeDispatch`.
Failure while writing a reply after service execution is `OutcomeUnknown`; callers
must reconnect and inspect/replay rather than infer success or failure.

The services check access before touching their durable stores. Mutation denial
is a typed `NotAuthorized` refusal and cannot consume an identity. Saga operator
commands additionally require command-scoped verified facts from the
authentication adapter; denial returns `NotAuthorized` without dispatch, while
credentials and grants never enter saga DTOs. `moira-unix-peer` selects the local
account profile: it obtains effective UID/GID from the kernel and maps an exact
bounded registry entry only to ordinary read/mutate facts. Its saga composition
delegates operator authorization to a separate injected component and never
infers Aegis authority from a local UID. `moira-aegis` supplies the selected
implementation of that service-owned port through Aegis's public client, an
explicit time source and exact-saga `Administer` checks.

Health contains only readiness, active/accepted/refused connections,
outcome-unknown count and saturation. Drop removes the socket only when its
device/inode still match the entry created by this daemon.

The legacy real-socket journey covers authenticated negotiation, socket mode,
durable admission, status inspection, teardown, SQLite/daemon reopen, exact replay
without a second workflow, health and owned-path cleanup. The saga journey covers
admission, uncertainty, authorized reconciliation, content-safe receipt export,
daemon/SQLite reopen, exact stale-request replay and denied reconciliation without
revision change. A separate subprocess journey connects a real Aegis daemon to
the Moira saga authenticator over Aegis's public client/wire, then drives Moira
over its public client/wire and verifies the live authority decision is retained
in the operator receipt. Focused connection tests cover both frame families,
multi-request clean EOF and reply-loss classification. This is not a complete
peer workflow, network, same-UID hostile-process, launch-supervisor or remote
credential profile. See [the exact credential boundary](unix-peer-credentials.md).

The separate `make qualify-hermes-process` profile runs Hermes in its own helper
process, discards Moira's first append reply, exits that process, reopens the
ledger in another Hermes process and verifies through public wire frames that the
stable Moira event replays at sequence zero without duplication.
