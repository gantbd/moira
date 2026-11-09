# Operating rigor

Moira follows the workspace discipline established by Chronos:

- use closed, versioned command, outcome, refusal, intent, and evidence enums;
- keep the semantic core deterministic with time, identity, randomness, and
  evidence supplied explicitly;
- check all bounds and counter exhaustion before publication;
- make refusal atomic and preserve uncertain effects behind a fence;
- separate identity, authority, correlation, effect knowledge, and presentation;
- never translate timeout, process exit, or telemetry absence into completion;
- treat compensation as a new authorized effect, never as rollback fiction;
- make migrations explicit mappings with executable laws;
- state exactly which tests and environments support each claim.

A feature is not ready for implementation until review can identify its state
space, authority, bounds, identity/composition laws, effect boundary, recovery
meaning, and executable evidence.

