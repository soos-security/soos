//! Opt-in fault-injection hook for `pam_soos.so` (Cargo feature `fault-injection`).
//!
//! Review finding PAM-01 / TCI-01 (GitHub #148): the shipped module is a release build, and
//! the panic-safety story (`catch_unwind` -> syslog -> `PAM_IGNORE`) is only true if the
//! release profile unwinds. Production code contains no panic path by design, so a test-only
//! hook is the only way to make the *release-built* shared object panic on demand. The Docker
//! matrix case T10 builds this feature into a separate target directory, loads the module
//! under a distinct name and arms it with the PAM argument `fault_inject=<panic|overflow>`.
//!
//! Guarantees:
//! - this module is compiled only with `--features fault-injection` (never a default feature,
//!   never enabled by packaging or install scripts; enforced by `tests/invariants`);
//! - the hook runs inside the `catch_unwind` region of `authenticate_with_config`, before any
//!   socket activity, so an armed panic can only ever degrade to `PAM_IGNORE`;
//! - the panic payload is a fixed, secret-free string.

use crate::config::FaultInject;

/// Fixed payload of the explicit-panic mode (secret-free, logged to syslog by the catcher).
pub const PANIC_PAYLOAD: &str = "fault injection: explicit panic requested by fault_inject=panic";

/// Triggers the armed fault, if any. Returns normally when `mode` is `None`.
///
/// `Some(FaultInject::Panic)` raises a panic through `std::panic::panic_any` with
/// [`PANIC_PAYLOAD`]; `Some(FaultInject::Overflow)` performs an arithmetic overflow that
/// `overflow-checks = true` turns into a panic (the profile setting the release build relies on).
#[allow(
    clippy::panic,
    reason = "Fault injection: raising a panic is the sole purpose of this feature-gated test hook; it is compiled only with --features fault-injection and never shipped"
)]
pub fn trigger(mode: Option<FaultInject>) {
    match mode {
        None => {}
        Some(FaultInject::Panic) => std::panic::panic_any(String::from(PANIC_PAYLOAD)),
        Some(FaultInject::Overflow) => overflow(),
    }
}

/// Deliberate `u64` overflow; panics when the build has `overflow-checks = true`.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "Fault injection: the overflow is the intended effect; black_box defeats constant folding"
)]
fn overflow() {
    let base = std::hint::black_box(u64::MAX);
    let bumped = base + u64::from(std::hint::black_box(1u8));
    std::hint::black_box(bumped);
}
