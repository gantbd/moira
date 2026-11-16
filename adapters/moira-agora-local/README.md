# Moira Agora local transport

`moira-agora-local` supplies the missing private Unix `ServicePortV2` for
Agora's transport-neutral public client. It owns only connection mechanics:

- canonical V2 handshake and server-incarnation evidence;
- the server-issued client identity;
- bounded length-prefixed frames and the negotiated post-connect frame budget;
- I/O deadlines; and
- truthful `NotSent` versus `OutcomeUnknown` classification.

It does not authenticate a participant, choose a room, create publication
content, interpret Agora outcomes or retain a replay namespace. Those remain
Agora, `moira-agora`, durable publisher-store and deployment responsibilities.
Malformed or unexpected replies after a complete write are outcome-unknown,
never safe automatic failure.

The package deliberately sits beside `moira-agora`: the semantic adapter keeps
its production dependency closure limited to Agora's public client/protocol,
while local transport composition opts into `agora-wire` separately.

`make qualify-agora-local-port` builds in an isolated lockfile, crosses Agora's
actual Aegis-authorized local daemon, executes a public-client capability request
and disconnects cleanly. Agora engine/host/service packages remain qualification
dependencies and do not enter Moira's root dependency graph.
