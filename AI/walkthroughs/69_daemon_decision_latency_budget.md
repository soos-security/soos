# Walkthrough 69: Daemon Decision Latency Budget Calibration

## Objective
Calibrate the daemon internal decision latency budget (`DECISION_BUDGET_MS`) to 220ms, preventing false `Verdict::Unavailable` timeouts when 3 ONNX neural models evaluate under momentary CPU jitter while remaining safely within the PAM synchronous 250ms deadline.

---

## Root Cause Identified
In `crates/daemon/src/pipeline.rs`, `DECISION_BUDGET_MS` was set to a hardcoded 150ms. While nominal execution runs around ~130ms, initial tensor evaluations or background system load can introduce slight execution jitter, reaching 158–173ms. Because the PAM client (`pam_soos.so`) allows a 250ms timeout budget, aborting at 150ms caused spurious fallback to password prompt during live interactive PAM authentication.

---

## Changes
- Updated `DECISION_BUDGET_MS` in `crates/daemon/src/pipeline.rs` from 150ms to 220ms.
- Preserved client-supplied `deadline_monotonic_ns` enforcement and fail-closed timeout guarantees.

---

## Verification & Test Results
- `cargo test -p soos-daemon`: all tests passed cleanly.
- `cargo fmt` and `cargo clippy`: zero warnings.
