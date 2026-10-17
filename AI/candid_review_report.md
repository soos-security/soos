# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `fix/lock-widget-memory`
- **Base (merge-base)**: `9e82795`
- **Reviewed-Diff-Fingerprint**: `52c4e90d3c9bc15178c867d1778632db6a85c24aeccf8bc0137562303c0fa4c1`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_lock_widget_memory.md`,
  `AI/auditor_constraints_lock_widget_memory.md`, `AI/tester_contract_lock_widget_memory.md`,
  `AI/walkthroughs/218_lock_widget_memory.md`, `Docs/LOCKSCREEN_WIDGET.md`, `crates/lock-widget/src/layout.rs`,
  `crates/lock-widget/src/wayland.rs`, `crates/lock-widget/tests/lock_widget_buffer_tests.rs`

## 1. Executive Summary

GitHub #412: `soos-lock-widget` now allocates one shared-memory buffer per surface at configure time and a
second one only when every allocated buffer is still held by the compositor (at most
`MAX_BUFFERS_PER_SURFACE` = 2). Each buffer lives in its own `SlotPool` of exactly `slot_len` bytes
(`buffer_len` rounded up to 64). The change is confined to one user-level, seat-less, `forbid(unsafe_code)`
crate with no PAM, IPC or daemon impact. No defect of MAJOR or CRITICAL severity was found; no pre-existing
test was touched.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: only `crates/lock-widget/tests/lock_widget_buffer_tests.rs` (new file, 160 added lines).
- Removed or changed `assert`/`#[test]`/`proptest!`/`should_panic` lines: **none** (grep empty).
- New escape hatches (`#[ignore]`, `cfg(any())`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` changes: **none**.
- `git diff $(git merge-base origin/main HEAD) HEAD -- '*tests*' 'tests/'`: one file, 160 insertions, 0
  deletions.
- `test_lka9_memory_bounds` (`lock_widget_render_tests.rs:74`, `assert_eq!(BUFFERS_PER_SURFACE, 2)`) is
  unchanged; production keeps `BUFFERS_PER_SURFACE` as an alias of `MAX_BUFFERS_PER_SURFACE` so the contract
  still holds (and passes).

## 3. Deep Reasoning Audit

### Logic & Architecture
- Verified against the locked toolkit (smithay-client-toolkit 0.20.0, the version the workspace declares):
  `SlotPool::new_slot` rounds to `(len + 63) & !63`; `alloc` grows to `max(rv + size, 2 x len)` when the slot
  does not fit; `RawPool::resize` sets `self.len` before `set_len`. So a pool of exactly `slot_len` bytes takes
  its single slot without growth, and the code never calls `resize`. The spec claims hold. PASS.
- `Buffer::canvas` returns `None` while `active_buffers != 0`, so `is_free` matches "not held". PASS.
- Scenario: compositor never releases. Frame 1 uses buffer 0; frame 2 finds none free and
  `needs_extra_buffer(1, false)` allocates buffer 1; frame 3 finds none free, `needs_extra_buffer(2, false)` is
  false, `retry = true`. Bounded at two. PASS.
- Scenario: extra allocation fails. `free` stays `None`, so `retry = true`, no push, no stderr. Retried on every
  33 ms tick by the existing timer (bounded rate). PASS.
- Scenario: first paint of a new buffer. `painted: false` leads to `paint_full` with full damage; the
  partial-damage path only runs on a buffer previously painted fully, so no stale pixels. PASS.
- Scenario: reconfigure to a new size while a buffer is held. `buffers.clear()` drops `Buffer` before
  `SlotPool` (field order) as before; a same-size reconfigure keeps the buffers (unchanged early return). PASS.
- `layout.width/height` equal the configured `width/height` (`Layout::for_surface` stores them), so the extra
  buffer has the same size as the first. PASS.
- Pixel budgets, `buffer_len`, damage, commit, timer and exit behaviour unchanged. PASS.

### PAM Concurrency & Deadlines
- No file in `crates/pam` is touched; the widget is a separate user-level program. N/A, PASS.

### Panic Safety & Fail-Closed
- No `unwrap`/`expect`/panic macro/indexing added in production; `get_mut`, `checked_sub`,
  `checked_next_multiple_of`, `checked_mul` and `try_from` are used. Every allocation failure maps to `None`,
  i.e. "surface not drawn" or "busy". No authentication decision exists in this crate. PASS.

### Test Integrity & Anti-Weakening
- See section 2. The new tests fail against an eager policy (`needs_extra_buffer` always true breaks
  `no_extra_buffer_while_one_is_free` and the prompt-release replay) and against a never-allocating policy
  (holding-compositor replay). The replay mirrors the production selection loop; the Wayland loop itself has no
  CI compositor, recorded as owner hardware row LKA19 (pending). PASS.

### Memory, Bounds & Secrets
- Shared memory is bounded by `MAX_BUFFERS_PER_SURFACE x slot_len` per surface, and the former latent pool
  doubling on unaligned sizes is removed. No state, frame or secret is logged; no new stderr line. PASS.

### Supply Chain & Automation
- `Cargo.lock` and `crates/lock-widget/Cargo.toml` unchanged (`git diff --exit-code` clean). No scripts, hooks
  or workflows touched. PASS.

### English-Only Policy
- All code, comments, docs, ADR, matrix rows and the commit message are in English. PASS.

### Commands re-run (scoped, mandated environment, `--test-threads=3`)
- `cargo test -p soos-lock-widget --locked`: all suites green (7 new LKA18 tests plus the LKA8/LKA9/LKA12/LKA13
  suites).
- `cargo clippy -p soos-lock-widget --locked --all-targets -- -D warnings`: clean.
- `cargo fmt -p soos-lock-widget -- --check`: clean.
- `cargo test -p soos-invariants --locked`: 707 passed.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/lock-widget/src/wayland.rs` (`draw_all`) — on a host where the second `SlotPool::new`
  keeps failing while the compositor holds the first buffer, a memfd creation is attempted on every 33 ms retry
  tick. It is bounded by the existing timer and leaves no lasting side effect; no change required.
- **[SUGGESTION]** `AI/walkthroughs/218_lock_widget_memory.md` — numbers 215 to 217 are absent on this branch
  (presumably reserved by parallel branches); the invariants suite accepts the gap.

## 5. Final Verdict

**VERDICT: APPROVED**
