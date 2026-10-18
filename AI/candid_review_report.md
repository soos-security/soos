# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/ir-active-cues`
- **Base (merge-base)**: `3f76090` (origin/main after #430)
- **Reviewed-Diff-Fingerprint**: `e638fa05e55619c8166ff4ca254273e0e4b034e6995ee4e1a99f2b5075216895`
- **Audited Files**: `AI/BACKLOG.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_ir_active_cues.md`, `AI/auditor_constraints_ir_active_cues.md`, `AI/tester_contract_ir_active_cues.md`, `AI/walkthroughs/232_ir_active_cues.md`, `Docs/VISION_CRATE.md`, `crates/vision/src/ir_active.rs`, `crates/vision/src/lib.rs`, `crates/vision/tests/ir_active_cues_tests.rs`, `tests/fixtures/mod.rs`, `tests/invariants/src/ir_active_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

The branch adds `soos_vision::ir_active`, a pure, zero-I/O, veto-only evaluation of a lit/unlit
IR frame pair (static pair, response ratio, saturation, reflectance band, inverse-square relief,
corneal glint), synthetic fixtures in `soos-test-fixtures::ir_strobe`, 24 contract tests, 4 static
invariants, and documentation (ADR, matrix IRA29–IRA37, backlog, `Docs/VISION_CRATE.md` §2.4.5,
walkthrough 232). There is no consumer: `soos-daemon`, `soos-policy` and `crates/pam` are
untouched, so no authentication path changes. The diff is add-only for every test file. No
CRITICAL, MAJOR or MINOR finding.

Re-run by this reviewer (`LIBONNXRUNTIME_NO_PKG_CONFIG=1`, dedicated `CARGO_TARGET_DIR`,
`CARGO_INCREMENTAL=0`, debug info off, `CARGO_BUILD_JOBS=3`, `--test-threads=3`):

- `cargo test -p soos-vision --test ir_active_cues_tests`: 24 passed.
- `cargo test -p soos-vision`: every target green (no failure).
- `cargo test -p soos-invariants ir_active`: 4 passed (787 filtered, 791 total, matching the walkthrough).
- `cargo clippy -p soos-vision -p soos-invariants -p soos-test-fixtures --all-targets -- -D warnings`: clean.
- `cargo fmt --check -p soos-vision`: clean.
- A throw-away binary outside the repository printed `ir_active_statistics` and the verdict for every
  fixture; every value of the walkthrough §6 table (face response, ratio, saturated fraction, lit
  mean, relief, glints, result) matches the code's output.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `crates/vision/tests/ir_active_cues_tests.rs` (new), `tests/fixtures/mod.rs`
  (additions only: `ActivePresentation`, `ActivePair`, `active_pair` inside `ir_strobe`),
  `tests/invariants/src/ir_active_contract.rs` (new), `tests/invariants/src/lib.rs` (one module
  declaration added).
- Removed or changed assertion / `#[test]` / proptest lines (`^-` grep): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- `git diff $(git merge-base origin/main HEAD) HEAD -- tests 'crates/*/tests' android/app/src/test`
  contains no removed line: no pre-existing assertion was removed or changed. The existing
  `ir_strobe` consumers (`soos-camera-v4l` strobe tests) use functions the diff does not touch.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Box far outside the frame: clamping gives a window narrower than 16 px → `InvalidGeometry`. PASS.
- Inverted, NaN or infinite box / landmark: rejected before any cast (`face_window`, `finite_point`
  over all five landmarks). PASS.
- Eye-corner square centred outside the crop: `square` returns `None` when `x0 > x1` →
  `InvalidGeometry` (fail closed, never an empty region silently counted). PASS.
- Swapped pair: signed difference gives a negative face response → `NoResponse`. PASS (test).
- Negative or near-zero background response: denominator `max(·, 1)` keeps the ratio finite and
  never inflates it beyond `face_response`; the absolute `min_face_response ≥ 20` still applies. PASS.
- Edge band computed as face sum minus inner window sum: the 16 px minimum side gives band 2 and an
  inner window of 12, never empty; checked explicitly anyway. PASS.
- Cue order matches the ADR, the `Docs/VISION_CRATE.md` table and the doc comment; the static pair
  is checked after input validation and before any statistic, as documented. PASS.
- Single source of truth: defaults are the `IR_ACTIVE_*` constants; `validate()` lower bounds
  (response > 0, ratios ≥ 1, `min_glint_pixels ≥ 1`, `max_glint_fraction > 0`) make it impossible
  to switch the response, relief or glint cue off with a valid configuration, as documented. The
  saturation cue and the reflectance band can be widened to no-ops by a valid configuration; the
  docs only claim the three cues above, so the claim is accurate (see SUGGESTION 2). PASS.
- Scope: no daemon, policy, protocol, PAM or config key change; matches GitHub #431 split item 2. PASS.

### PAM Concurrency & Deadlines

- `crates/pam` is not in the diff; no thread, runtime or blocking call is added anywhere. PASS (N/A).

### Panic Safety & Fail-Closed

- No `unwrap`, `expect`, `panic!`, `unreachable!` or direct indexing in `ir_active.rs`; every slice is
  taken with `slice::get` through `row_slice` with checked multiplication. PASS.
- Casts: `f32 → usize` only on values clamped to `[0, width]`; `f32 → i64` only on finite landmarks
  already proven inside a window of at most 1024 px (eye corners at most `0.30 d` beyond it). PASS.
- Integer sums: at most 1024 × 1024 × 255 per region in `i64` / `u64`; glint counts converted to
  `u32` with `try_from`. PASS.
- Every non-finite statistic becomes `InvalidFrames` (`to_f32`), so the threshold comparisons never
  see NaN; `prop_ira_active_arbitrary_input_never_panics` also checks that any `Ok` is consistent
  with every threshold. PASS.
- `IrActiveCuesClear` has a private field and no constructor besides the evaluation; no `is_live`. PASS.

### Test Integrity & Anti-Weakening

- Each attack fixture is asserted against its exact rejection reason, and
  `test_ira_active_each_cue_alone_vetoes_the_shaded_head` tightens one threshold at a time and
  expects that cue's reason, so a stub returning a fixed error or a silently skipped cue would fail.
  PASS.
- The bona fide test asserts the statistics against the thresholds and
  `eye_region_pixels > glint_pixels`. PASS.
- No pre-existing test modified (section 2). PASS.

### Memory, Bounds & Secrets

- Frame bound 4096 × 4096 checked before any work; crop bound 1024 × 1024 checked before
  allocation; the two crop copies are `Zeroizing<Vec<u8>>` with exact capacity (no reallocation). PASS.
- `Crop::rows` collects at most one borrowed `(&[u8], &[u8])` pair per crop row (at most 1024
  entries, no pixel copy). PASS.
- No logging, no I/O (invariant `test_ira_active_module_is_zero_io_and_silent` and manual read);
  rejection and config error messages carry no value; statistics are aggregates only. PASS.
- No `unsafe`; `#![forbid(unsafe_code)]` kept in `soos-vision`. PASS.

### Supply Chain & Automation

- No `Cargo.toml`, `Cargo.lock`, `deny.toml`, `.github/`, `scripts/` or hook change; `zeroize`,
  `soos-frame` and `soos-inference-ort` were already dependencies of `soos-vision`. PASS (N/A).

### English-Only Policy

- No accented Latin characters in the patch; code, comments and docs are in English. PASS.

### Factual Accuracy of Documentation

- ADR, `Docs/VISION_CRATE.md` §2.4.5, backlog and walkthrough thresholds (20, 1.5, 250, 5 %,
  [50, 200], 1.2, 240, 120, 1, 25 %), bounds (16 px, 8 px, 64 px, 1024 × 1024, 4096 × 4096), region
  geometry (50 % ring, 15 % band, `0.25 d`, `0.30 d` / `0.08 d`, `max(2, 0.15 d)`) and cue order
  match `ir_active.rs`. The §2.4.1 cross-reference exists. IRA29–IRA37 follow IRA1–IRA28 without
  collision; walkthrough 232 follows 231. The matrix statement that the branch is not registered in
  `scripts/sync_issue.py` is true (no match). The walkthrough statistics table matches the code
  output (section 1). PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/vision/src/ir_active.rs:591` — `Crop::rows` allocates a small `Vec` of
  row slice pairs per region; iterating rows directly would avoid the allocation. Bounded, no
  correctness impact.
- **[SUGGESTION]** `crates/vision/src/ir_active.rs:211` — a valid configuration can neutralise the
  saturation and reflectance cues (`max_saturated_fraction = 1`, band `[0, 255]`). This matches the
  documentation, which only promises that the response, relief and glint cues cannot be switched
  off; the consumer PR (split item 4) should decide whether stricter bounds are wanted.

Owner hardware checks IRA36 and IRA37 remain pending (recorded in the matrix and backlog).

## 5. Final Verdict

**VERDICT: APPROVED**
