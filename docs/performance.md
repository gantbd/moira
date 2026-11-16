# Performance qualification

`make performance` runs an ignored optimized acceptance test for the selected
in-memory saga-service profile. The target records the Rust compiler, OS and
architecture, then measures 64 successful samples each of admission, point read,
effect acceptance and explicit retry. Setup and compilation are outside the
samples; every measured operation retains its functional assertions.

The provisional reference-host engineering budgets are p95 <= 10 ms and sample
p99 <= 25 ms for each operation class. With 64 samples nearest-rank p99 is the
maximum, not a statistically established production tail. On 2026-11-10 the
`aarch64-apple-darwin` Rust 1.95.0 optimized run produced:

| Operation | p50 ns | p95 ns | sample p99/max ns |
| --- | ---: | ---: | ---: |
| Admit | 83 | 1,292 | 46,667 |
| Saga read | 41 | 42 | 125 |
| Accept effect | 42 | 334 | 10,291 |
| Retry | 42 | 83 | 1,875 |

This is a regression gate for the exact synthetic service/store profile, not a
product SLO. It excludes SQLite transactions, daemon or peer transport, provider
execution, concurrent sagas, large retained evidence, allocation/RSS, process
recovery and power loss. SQLite/peer-process performance and longer durable
process-soak qualification beyond the selected Aegis operator segment remain
separate release work.
