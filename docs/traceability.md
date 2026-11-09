# Traceability ledger

The deterministic volatile workflow slice is implemented. Encoded durable
recovery and production connectors remain open; reconstruction from an in-memory
checkpoint is not crash-recovery evidence. Open is not a pass.

| Requirement | Planned implementation | Current evidence | Status |
| --- | --- | --- | --- |
| M1–M2 authority/correlation | non-cloneable workflow engine, optimistic revision, fixed-width opaque workflow/effect/source references | Chronos admission/attempt, Daimon result, and Agora publication behavior plus strict lint | Partial; persistence correlations open |
| M3 recovery | validated semantic checkpoint plus future canonical codec/store protocol | reconstruction at Chronos, Daimon, Agora-pending, and terminal boundaries preserves effect/replay identity; Hermes append lost-reply qualification | Partial; encoding and atomic persistence open |
| M4 idempotency | admitted-message index and retained Chronos/Daimon/Agora effect-receipt mappings | matching identity replay, conflicting input/effect refusal, Daimon qualification, and Agora lost-reply qualification | Satisfied for volatile reference path; restart durability open |
| M5 effect knowledge | qualified Keystone lattice plus attributed external evidence | exhaustive Chronos/Keystone/Moira correspondence, Daimon `Uncertain` fencing, and exact Agora publication receipt | Partial; durable evidence open |
| M6 saga semantics | explicit step transitions | model document | Open |
| M7 bounds | explicit workflow-count limit and checked revision/identity counters | constructor/capacity expansion still needed | Partial |
| M8 authority | Aegis validated decision adapted into Daimon authority evidence | Moira→Aegis→Daimon qualification uses scoped execute grant and attributed decision | Partial; production credential/adapter binding open |
| M9 isolation | deterministic core with explicit commands/receipts; Keystone effect lattice is the sole mechanism dependency | source boundary, behavior tests, strict lint | Satisfied for implemented volatile slice |

