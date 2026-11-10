# Agora integration profile

Status: qualified for the local durable receiver.

## Ownership

Agora owns participants, rooms, membership, roles, messages, participant-neutral
addressing, ordered events, and replay-safe reply publication. Moira owns whether
an observed conversation fact becomes a workflow, which participant reference is
the intended workflow recipient, task/execution correlation, uncertainty, and
response-publication intent.

Addressing is neither authority nor a workflow command. Human and agent authors
and recipients use the same projection. Harness selection and participant
execution remain outside both the projection and Agora.

## Ingress sequence

```text
Agora MessageAccepted outbox event
        |
        v
ConversationMessageFactV1        (read-only, pending, replay-stable)
        |
        | host policy + participant-reference mapping
        v
Moira AdmitMessage                (explicit durable decision)
        |
        v
ChronosSubmissionPrepared         (stable effect identity)
```

`pending_message_fact` resolves only an event retained by the receiver's own
Agora checkpoint and room/source catalogue. It returns event sequence, opaque
message reference, author, and canonical addressed IDs. It omits message body and
participant kind. It does not acknowledge the event or mutate Moira.

The host selects whether to admit and supplies the opaque `ParticipantRefV1` for
one intended recipient. Matching replay returns the original workflow and Chronos
submission identity. Reusing the message reference with another participant
mapping is `InputConflict` and leaves state unchanged.

## Recovery

The Agora checkpoint/outbox and source-room catalogue persist the input fact.
The Moira SQLite store persists the explicit admission and its effect identity.
A lost admission reply reopens and replays without another workflow. A lost final
Agora publication reply replays the exact keyed publication without another room
message. These are two independent durable boundaries, not a distributed
transaction.

Acknowledging the Agora event removes it from the pending-fact surface. Hosts
must commit or otherwise durably account for their admission decision before
acknowledging delivery. This profile does not supply an automated inbox pump,
retry loop, authorization adapter, or network transport.
