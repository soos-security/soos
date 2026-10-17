# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `fix/lock-widget-memory`
- **Base (merge-base)**: `085ba63` (rebased from `6eb0746`; `origin/main` gained only #417)
- **Reviewed-Diff-Fingerprint**: `823d38c9fcc7642d4ca5e8697f363bcbec67a0b5d4392ce2fb96fcdec7240e9e`
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

This report re-reviews the branch after its rebase onto `origin/main` `085ba63` (#417: zbus 5 -> 6 migration
in `Cargo.toml`/`Cargo.lock`, the `crates/daemon` and `crates/remote` logind code, docs, matrix rows PAU17/RMC17/
RMC19 and three invariant files). The previous report (fingerprint `f2b3b471…`, base `6eb0746`) was APPROVED;
the report before it (fingerprint `52c4e90d…`, base `9e82795`) was APPROVED too. Their findings are carried
forward below.

### Rebase verification

- `git range-diff origin/fix/lock-widget-memory~3..origin/fix/lock-widget-memory 085ba63..HEAD`: the fix commit
  (`4fd0d5d` = `df2508d`) and the earlier post-rebase review commit (`3ea706c` = `d2fbeb1`) are identical; the
  first review commit (`6982101` ! `8628321`) differs only in `AI/candid_review_report.md`, the file that
  conflicted because main rewrote it for #417 (branch side kept, now superseded by this report).
- Per-file patch-id comparison (`git diff 6eb0746 <old tip>` vs `git diff 085ba63 HEAD`, `git patch-id
  --stable`): identical for `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_lock_widget_memory.md`,
  `AI/auditor_constraints_lock_widget_memory.md`, `AI/plan_evaluator_report.md`,
  `AI/tester_contract_lock_widget_memory.md`, `AI/walkthroughs/218_lock_widget_memory.md`,
  `Docs/LOCKSCREEN_WIDGET.md`, `crates/lock-widget/src/layout.rs`, `crates/lock-widget/src/wayland.rs` and
  `crates/lock-widget/tests/lock_widget_buffer_tests.rs`. The code under review is byte-identical to the
  previously approved diff.
- Content proof: `git diff <old tip> HEAD` equals `git diff 6eb0746 085ba63` (both excluding the report) except
  for one blob `index` header line of `AI/VERIFICATION_MATRIX.md`, which both sides edit; the rebase added
  exactly main's #417 changes and nothing else.
- `AI/VERIFICATION_MATRIX.md`: #417 edits rows PAU17, RMC17 and RMC19; this branch adds LKA18/LKA19 (lines
  2431-2432). Disjoint rows, no conflict markers anywhere in `AI/`, `Docs/` or `crates/lock-widget`.
- `AI/DECISIONS.md`: still exactly one #401 entry (line 144, with the amendment note), the #411 clarification
  (line 145) and this branch's ADR (line 146); #417 does not touch the file.

### Interaction with the zbus 6 migration

- `soos-lock-widget` has no `zbus`/`zvariant` dependency (`crates/lock-widget/Cargo.toml`, and `cargo tree
  --locked -p soos-lock-widget` lists no zbus crate); the branch leaves `Cargo.toml`, `Cargo.lock` and
  `crates/lock-widget/Cargo.toml` untouched relative to `085ba63`.
- #417's invariant edits (`portable_crates_contract.rs`, `presence_unlock_contract.rs`,
  `remote_companion_contract.rs`) do not mention the lock widget, and the full `soos-invariants` suite passes on
  the rebased tree. No semantic interaction.
- Walkthrough numbering: main adds 221 (#417); this branch's 218 sits between 217 and 221. Numbers 213, 215,
  216, 219 and 220 are absent (reserved by parallel branches); the invariants suite accepts the gaps.

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

### Commands re-run after the rebase onto `085ba63` (mandated environment, `--test-threads=4`)
- `cargo test --locked -p soos-lock-widget -p soos-invariants -- --test-threads=4`: exit 0, 752 passed, 0
  failed, 0 ignored (invariants 707; lock-widget buffer 7, CLI 5, render 16, scene 17).

### Commands re-run after the earlier rebase onto `6eb0746`
- Same command: exit 0, 752 passed, 0 failed, 0 ignored.

### Commands run before the first rebase (scoped, mandated environment, `--test-threads=3`)
- `cargo test -p soos-lock-widget --locked`: all suites green (7 new LKA18 tests plus the LKA8/LKA9/LKA12/LKA13
  suites).
- `cargo clippy -p soos-lock-widget --locked --all-targets -- -D warnings`: clean.
- `cargo fmt -p soos-lock-widget -- --check`: clean.
- `cargo test -p soos-invariants --locked`: 707 passed.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/lock-widget/src/wayland.rs` (`draw_all`) — on a host where the second `SlotPool::new`
  keeps failing while the compositor holds the first buffer, a memfd creation is attempted on every 33 ms retry
  tick. It is bounded by the existing timer and leaves no lasting side effect; no change required.
- **[SUGGESTION]** `AI/walkthroughs/218_lock_widget_memory.md` — after the rebase, 217 (#411) and 221 (#417) are
  present and numbers 213, 215, 216, 219 and 220 are absent (presumably reserved by parallel branches); the
  invariants suite accepts the gaps.

## 5. Final Verdict

**VERDICT: APPROVED**
