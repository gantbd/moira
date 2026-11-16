# Unix peer credential profile

Status: selected local OS-account authentication profile for the legacy and
closed-saga daemons; remote identity and operator authority remain separate.

`moira-unix-peer` asks the kernel for the accepted Unix stream's effective UID
and GID and performs an exact lookup in a bounded deployment-supplied registry.
BSD-family systems use `getpeereid`; Linux and Android use `SO_PEERCRED`.
Unsupported Unix targets and credential-retrieval failure refuse the connection.
The registry rejects zero limits, empty configuration, excess entries and
duplicate UID/GID pairs.

Each mapping yields only Moira's `AccessV1` read/mutate facts. It does not map an
account to a workflow, participant, saga, effect, Aegis grant or operator
decision. Unknown peers are refused before the ready frame and before any
command or store dispatch. Mapping debug output redacts the effective identity.

For the saga daemon, `UnixSagaAuthenticatorV1` composes that peer mapping with a
separate injected `OperatorAuthorizer`. Reconciliation never consults the UID/GID
registry. The selected `moira-aegis::AegisOperatorAuthorizerV1` obtains current
command-scoped `Administer` evidence for the exact saga through Aegis's public
client and an explicit time source, returning only `OperatorAccessV1`
attribution. Denial remains `NotAuthorized` without durable mutation.

The boundary authenticates an OS account on one local kernel. It does not
distinguish people, binaries, processes or containers sharing that effective
identity; defend against a compromised same-UID process; authenticate a remote
peer; or protect against a malicious kernel or root. The launcher must own the
registry and trusted parent directory, minimize account sharing, protect the
socket path, and compose a real operator-authority adapter where reconciliation
is enabled.
