# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `test/deflake-gui-store-task`
- **Base (merge-base)**: `74782b8`
- **Reviewed-Diff-Fingerprint**: `ffbb716da4e88858cd9375ae08dda714ad5fb2016f8290040011557984507848`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/226_deflake_gui_store_task.md`, `Docs/GUI_APPLICATION.md`, `crates/gui/tests/store_task_tests.rs`

## 1. Executive Summary

This is a test-only change. The `wait_outcome` helper in `crates/gui/tests/store_task_tests.rs` checked its 10 s deadline before polling. A test thread descheduled across the deadline therefore exited without a final poll. The helper is rebuilt on a generic `poll_until(deadline_passed, poll, pause)` that polls first and checks the deadline after. Two new tests, DGS1 and DGS2, cover the ordering and the bound. No production, PAM, IPC, Cargo or automation file changes. No SGU test body, assertion, message or `#[test]` attribute changes. There is one minor documentation remark and one cosmetic suggestion, and no CRITICAL or MAJOR finding.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `crates/gui/tests/store_task_tests.rs` only.
- Removed or changed check lines: one, at patch line 158: `assert!(outcomes.is_empty(), "one outcome per task");` in the old `wait_outcome` body. Justification: the helper was rewritten, and the new `poll` closure keeps the same assertion with the same condition and message. It still fires exactly when an outcome was popped and others remain. The panic text `no store task outcome within 10 s` and the 10 s bound are kept. The assertion moved inside a helper and was not weakened.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): none.
- Changes to inline `mod tests`: none.
- New tests: `test_dgs_wait_polls_once_more_after_a_stall_past_the_deadline`, `test_dgs_wait_still_gives_up_after_the_timeout`.
- `git diff 74782b8 HEAD -- tests 'crates/*/tests' android/app/src/test` touches only the file above. No pre-existing assertion was removed or changed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- I traced the old and new ordering by hand on the DGS1 scenario:
  - The old order runs check(false), poll(None), pause (sets the flag), check(true) and returns `None`. DGS1 therefore fails red, as the walkthrough states (`left: None`, `right: Some(2)`).
  - The new order runs poll(None), check(false), pause, poll(Some(2)) and returns `Some(2)`.
  
  PASS.
- DGS2 takes `started` before `after(30 ms)` builds its deadline. `poll_until` returns `None` only when `Instant::now() >= deadline`, so `took >= 30 ms` holds by construction. The 10 s upper bound is generous, so the test is not load-sensitive. PASS.
- `after()` computes its deadline when it is called, at the start of `wait_outcome`, as before. The bound is still 10 s. PASS.
- The new helper adds at most one poll after the deadline. It cannot loop forever, because the real-clock predicate stays true once it becomes true. PASS.
- I checked the production claims against `crates/gui/src/store_tasks.rs`:
  - The worker runs the job under `catch_unwind`, then calls `tx.send`, then `notify` (lines 177-183).
  - `poll` drains `try_iter` and synthesizes `WORKER_LOST_MESSAGE` for a finished worker that sent no outcome (lines 229-243).
  - `worker_failure_tests::wait_outcome` receives first and checks the deadline after (lines 274-283).
  - `BiometricStore::with_lock_timeout` exists, and `LOCK_WAIT` is 600 ms.
  
  All of these claims are accurate.
- I checked the CI evidence with `gh run view 39062359937 --attempt 1 --log-failed`:
  - `running 6 tests` at 19:29:08.651.
  - The enroll test reports `FAILED` at 19:29:20.889.
  - The delete test reports `ok` at 19:29:20.8917, 2.7 ms later.
  - The run reports `finished in 12.24s`, with the panic at old line 91, the helper's `panic!`.
  
  All of this is accurate. See MINOR 1 for the one inference stated as fact.

### PAM Concurrency & Deadlines

There is no `crates/pam` change, so this pillar does not apply. PASS.

### Panic Safety & Fail-Closed

Only test code changes. `unwrap_or_else(|| panic!(..))` sits in a test helper covered by the file's existing test lint allowance. No production path is touched. PASS.

### Test Integrity & Anti-Weakening

- I reviewed the step 3 output above. The new body keeps the single removed assertion line word for word.
- The SGU test bodies do not change in the diff: `test_sgu_enroll_submit_never_blocks_on_a_held_store_lock`, `test_sgu_delete_submit_never_blocks_and_keeps_the_template_when_busy` and the other four.
- DGS1 fails against the plausible wrong implementation, the old order. DGS2 fails against an unbounded implementation or one that returns early (`took >= 30 ms`, `took < 10 s`, `polls >= 1`). PASS.

### Memory, Bounds & Secrets

The wait stays bounded: 10 s in SGU use and 30 ms in DGS2. Nothing new is logged, and no frames, embeddings or credentials are involved. PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. PASS.

### English-Only Policy

All added text is in English ("Synchronisation" is British spelling, still English). The commit subject `test(gui): deflake the held-store-lock enroll task test` follows Conventional Commits with one scope and is under 72 characters. PASS.

### Commands re-run by the reviewer

All commands ran with `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/deflake-gui-store-task CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=3`:

- `cargo fmt -p soos-gui -- --check`: clean.
- `cargo clippy -p soos-gui --all-targets -- -D warnings`: clean.
- `cargo test -p soos-gui --test store_task_tests -- --test-threads=3`: 8 passed, which matches the walkthrough.
- `cargo test -p soos-invariants -- --test-threads=3`: 771 passed.
- I did not reproduce the 50/50 load run cited in DGS1. The evidence that matters is the deterministic DGS1 red/green pair, as the walkthrough itself says.

## 4. Detailed Findings & Action Items

- **[MINOR]** `AI/walkthroughs/226_deflake_gui_store_task.md:17`: The walkthrough says the failure was at the *second* `wait_outcome` deadline, about 19:29:19.3, and that the panic came 1.6 s late. The CI backtrace shows only that the test function called `wait_outcome`, without the call-site line, so it does not show which of the two calls failed. If it was the first call, the deadline was about 19:29:18.66 and the panic came about 2.2 s late. The root-cause conclusion holds either way: the test thread was descheduled past its deadline. Optional correction: present this as an inference ("assuming the second call").
- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md:1795-1796`: the SGU1 and SGU2 rows have a stray space before the period ("` . Note:`"). Cosmetic.

## 5. Final Verdict

**VERDICT: APPROVED**
