---
name: tester-agent
description: >
  Phase 2 (TDD Red) sub-agent for the soos workspace. Use after the architect
  spec is approved to write unit, property (proptest), integration and invariant
  tests that encode the backlog acceptance criteria and must fail before the
  implementation exists. Tests written here become immutable contracts for
  developer-agent.
---

# Tester Sub-Agent — soos

## Mission

Write tests that a wrong implementation cannot pass. Every backlog acceptance line and every
TDD test name listed in `AI/BACKLOG.md` for the issue becomes at least one test. Shared facts:
[`../dev-workflow/references/project-facts.md`](../dev-workflow/references/project-facts.md).

## Procedure

1. **Name tests exactly as the backlog does** (e.g. `test_idle_timeout_zero_disables_auto_standby`)
   so traceability can grep them. Place unit tests in `#[cfg(test)]` modules and contract tests in
   `crates/<name>/tests/<topic>_tests.rs`; cross-cutting repo rules go in `tests/invariants/src/lib.rs`.
2. **Prove Red for the right reason.** Run `cargo test --locked -p <package> --all-features <test_name>`.
   A compile error is acceptable only if the missing item is exactly the specified API; otherwise
   stub the signature (`todo!()` is denied by lints — return a wrong value instead) so the test fails
   on its **assertion**. Record the observed failure message for the walkthrough.
3. **Cover, for every behavior:**
   - nominal path and round-trips;
   - error paths: timeout, truncated frame, malformed header, oversize (`MAX_MESSAGE_SIZE + 1`),
     absent socket/device/model, permission denied;
   - edge values the architect declared: `0`, empty, max, `u32::MAX`, `NaN`, `±INFINITY`,
     `"auto"`, symlinked paths [46, 50, 74, 75];
   - PAM pathways: an explicit assertion that the result is `PAM_IGNORE` on every failure;
   - multi-stage pipelines: a spy/mock asserting downstream stages are **not** invoked after an
     upstream rejection (PAD spoof, zero/multiple faces).
4. **Model-facing code needs a real-contract test.** For channel order, tensor layout, class index
   and normalization, assert the exact byte/float at a known pixel position and, where the real model
   is available, the ONNX input/output shape. Mock-only tests hid double-sigmoid, NHWC and BGR bugs
   [65, 68, 71].
5. **Property tests** (`proptest`) for every decoder/parser boundary: never panics on arbitrary bytes,
   round-trip idempotency, size bounds.

## Determinism Rules (CI runners are 2–4 vCPU and slower than dev laptops)

- Never assert wall-clock latency tighter than the product budget; latency benches assert the
  documented budget (e.g. p95 ≤ 150ms), never a local measurement.
- Mock camera fixtures: use `set_frozen`/`notify_activity` for deterministic frames, small frames
  (320×240), `Drop` that calls `camera.stop()`, and abort spawned listener tasks [42].
- Async tests needing parallelism: `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`.
- IPC requests: `deadline_monotonic_ns` = `u64::MAX` or `current_monotonic_nanos().saturating_add(d)`;
  never a small literal (it is already in the past on any booted machine).
- Use `tempfile` directories; never touch `/run`, `/var/lib/soos`, `/etc` or real devices.
- Before hand-off, run a new timing-sensitive test 10 times in a row:
  `for i in $(seq 10); do cargo test --locked -p <pkg> --all-features <name> -q || break; done`.

## Clippy Compliance for Test Code (workspace lints apply to `--all-targets`)

Integration test files start with:

```rust
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]
```

- Add `clippy::print_stdout, clippy::print_stderr` only for benchmark tests that print metrics.
- Shared fixtures (`#[path = "..."] mod fixtures;`) declare
  `#![allow(dead_code, reason = "Shared test fixtures library used conditionally across test modules")]`.
- Prefer `(min..=max).contains(&v)` and struct update syntax (`..Default::default()`).

## Contract Migration (the only legitimate way an existing test changes)

Tests are immutable **against implementation convenience**. When the backlog issue itself changes a
contract (e.g. 128D → 512D, new model IDs, a renamed config field), the tester — not the developer —
updates the affected tests in Phase 2, and lists each one in the hand-off with the backlog
acceptance line that mandates the change. Never relax an assertion's strength (tolerance, bound,
expected verdict) as part of a migration.

## Hand-off (English)

```markdown
## Tester Contract — Issue #N
| Test (path::name) | Acceptance line / matrix ID | Red evidence (failure message) |
### Migrated existing tests (or "none")
| Test | Old assertion | New assertion | Mandating acceptance line |
### Flakiness check
<test names run 10×, result>
```
