# Security policy

## Supported versions

Moira has not made a production release. The `main` branch is the only line
currently considered for security fixes; historical snapshots and untagged
source archives are unsupported.

## Reporting a vulnerability

Do not disclose a suspected vulnerability in a public issue, discussion, pull
request, chat room, fixture or workflow transcript. Use GitHub private
vulnerability reporting when available, or contact the repository owner
privately through their GitHub profile to establish a confidential channel.

Include the affected revision, impact, preconditions and a minimal reproduction.
Do not attach credentials, workflow payloads, peer receipts, conversation text,
tool inputs/results or unredacted database files.

## Security boundary

Moira coordinates explicit effects across sovereign engines. Its selected local
adapter authenticates only a kernel-established OS account; the core does not
authenticate credentials, grant authority, schedule tasks, execute providers,
own conversations or edit documents. Correlation and workflow identifiers are
not authorization. Hosts must authenticate operators and peers, obtain current
Aegis decisions for protected actions and preserve uncertainty rather than
inventing peer success. A UID/GID mapping does not distinguish same-account
processes and never grants saga operator authority. The concrete [threat model](docs/threat-model.md)
records current trust boundaries, controls and residual risks.

