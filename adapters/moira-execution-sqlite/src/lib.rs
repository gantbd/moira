//! Explicit local execution-receiver package. Chronos admission/attempt and
//! Daimon dispatch/settlement remain peer-owned semantics; this crate owns only
//! their coupled SQLite intent/receipt projection for the selected local slice.
#![forbid(unsafe_code)]
#![cfg(unix)]

pub mod chronos;
pub mod daimon;
mod daimon_codec;

#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum Fault {
    BeforeCommit,
    AfterCommit,
}
