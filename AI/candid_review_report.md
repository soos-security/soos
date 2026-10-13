# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `chore/macos-port-foundation`
- **Base (merge-base)**: `e86993a`
- **Reviewed-Diff-Fingerprint**: `f45b735ce6f17458d8ac79a74cc940c86a523fb8a78b81fed120955fbf2a18cd`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `.github/workflows/ci.yml`, `AGENTS.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_macos_port_foundation.md`, `AI/auditor_constraints_macos_port_foundation.md`, `AI/tester_contract_macos_port_foundation.md`, `AI/walkthroughs/198_macos_port_foundation.md`, `Cargo.lock`, `Cargo.toml`, `Docs/CAMERA_V4L_CRATE.md`, `Docs/CI_CD_AND_SECURITY.md`, `Docs/FRAME_CRATE.md`, `Docs/MACOS_PORT.md`, `Docs/README.md`, `Docs/VISION_CRATE.md`, `crates/camera-v4l/Cargo.toml`, `crates/camera-v4l/src/frame.rs`, `crates/camera-v4l/src/sensor.rs`, `crates/camera-v4l/tests/frame_drop_wipe_tests.rs`, `crates/frame/Cargo.toml`, `crates/frame/src/{frame,lib,sensor}.rs`, `crates/frame/tests/frame_contract_tests.rs`, `crates/vision/Cargo.toml`, `crates/vision/src/{color,error,ir_liveness,pipeline}.rs`, 13 files under `crates/vision/tests/`, `scripts/candid_review.sh`, `scripts/pr_checks_gate.sh`, `scripts/pr_loop.sh`, `tests/fixtures/{Cargo.toml,mod.rs}`, `tests/fixtures/tests/fixture_generators_tests.rs`, `tests/invariants/src/lib.rs`, `tests/invariants/src/portable_crates_contract.rs`

## 1. Executive Summary

I reviewed the frozen patch (`target/candid_diff.patch`, 55 files, merge-base `e86993a`) on my own.
I checked it against GitHub #367 and its parent #360, including the owner decisions of 2026-10-12
and 2026-10-13: a private `soos-macos`, `sudo_local` first, RGB-only as a convenience factor, ad-hoc
signing, and a **non-required** macOS CI job.

- **The type move changes nothing.** `diff` of `git show e86993a:crates/camera-v4l/src/frame.rs`
  against `crates/frame/src/frame.rs` is empty, and `SensorType` moved word for word.
  `impl Zeroize for Frame` and `impl Drop for Frame` (which wipes the whole buffer) moved with the
  type. `soos-camera-v4l` re-exports the same types with `pub use`. Every consumer (daemon,
  enrollment CLI) therefore keeps the same public paths and trait impls. The workspace clippy run
  (`--workspace --all-targets --all-features -D warnings`) is clean.
- **Test changes stay within the allowed scope.** The mechanical listing finds only these changes:
  - `use` lines in 13 vision test files, the fixtures generator test and `tests/fixtures/mod.rs`;
  - the dependency swap in the fixtures manifest;
  - `"frame"` added to the business-crate list.

  No assertion was removed or changed.
- **The portability guard can fail.** I added `tokio` to `crates/frame/Cargo.toml` and refreshed
  the lock. `test_portable_crates_have_no_linux_only_dependencies` then failed with
  `soos-frame reaches ["tokio", "tokio-macros"]`. I reverted the change, and `git status` is clean.
- **The merge gate fails closed.** I ran `scripts/pr_checks_gate.sh` with a fake `gh` that returned:
  - failed, cancelled and pending checks;
  - a count mismatch, an extra record, empty output and an empty name;
  - an unknown bucket and a non-zero gh exit;
  - advisory checks only, and names that nearly match the advisory one.

  I also ran it against the real PR #374. Every state that is not clean either blocks or waits.
  After the `CI Success` poll is green, `pr_loop.sh` runs a final gate right before the atomic
  `--match-head-commit` merge.
- **The macOS job has a clean supply chain.**
  - Every action uses a SHA pin that a Linux job already uses.
  - It has no secrets and does not widen `permissions:` (the workflow default is `contents: read`).
  - It sets `persist-credentials: false`, and caches are saved only on `main`.
  - It is not a `CI Success` dependency.

There is no CRITICAL or MAJOR finding. Two MINOR and three SUGGESTION items follow in section 4.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- `grep -nE '^-[^-].*(assert|#\[test\]|#\[tokio::test|proptest!|#\[should_panic)'` returns
  **nothing**. No assertion or test attribute was removed or changed.
- `grep -nE '^\+.*(#\[ignore|#\[cfg\(any\(\)\)\]|should_panic|tolerance|epsilon)'` returns one hit,
  at line 990 of the patch. It is prose in `Docs/MACOS_PORT.md` ("embeddings within tolerance as the
  CPU baseline"), not a way to skip a test.
- `grep -nE '^[-+].*mod tests'` returns nothing.
- Every `-`/`+` line in a test file that already existed:
  - `crates/vision/tests/{bench,color,embedding_lfw_evaluation,face_quality_gate,frame_robustness,gcv_review,ir_pad_policy,ir_sensor_policy,mjpeg_bounds,pad_crop_real_model,pad_ensemble,pipeline,zeroize}_tests.rs`:
    each swaps one `use soos_camera_v4l::{…}` for `use soos_frame::{…}` with the same imported
    names. The #360 acceptance criteria allow this ("apart from import-path updates that do not
    touch assertions").
  - `tests/fixtures/mod.rs`: two `use` swaps. `tests/fixtures/Cargo.toml`: the dependency changes
    from `soos-camera-v4l` to `soos-frame`, so the fixtures no longer pull in `v4l`.
  - `tests/fixtures/tests/fixture_generators_tests.rs`: one `use` swap.
  - `tests/invariants/src/lib.rs`: registers `mod portable_crates_contract;` and appends `"frame"`
    to `business_crates` in `test_business_crates_forbid_unsafe_code`. This makes the check
    stricter and is required by the #360 acceptance criteria.
- New test files, all additions:
  - `crates/frame/tests/frame_contract_tests.rs` (6 tests);
  - `crates/camera-v4l/tests/frame_drop_wipe_tests.rs` (3 tests);
  - `tests/invariants/src/portable_crates_contract.rs` (13 tests).
- `crates/camera-v4l/tests/frame_zeroize_tests.rs` is untouched. `crates/vision/tests/zeroize_tests.rs`
  changes only by its `use` swap. Both still pass.

## 3. Deep Reasoning Audit

### Logic & Architecture

- *Scenario: the move changes behaviour.* The byte-level `diff` of the old and new `frame.rs` is
  empty, and the `SensorType` enum and its derives are identical.
  `test_frame_types_keep_their_derives` pins the derives (`Debug, Clone, PartialEq, Eq` and
  `Copy, Hash`). **PASS**
- *Scenario: a consumer sees a different type.* `soos_camera_v4l::Frame`, `::frame::PixelFormat` and
  `::sensor::SensorType` are `pub use` re-exports. `test_camera_frame_types_are_the_soos_frame_types`
  passes values between both paths at compile time, and daemon and enrollment-cli compile under
  clippy. **PASS**
- *Scenario: `soos-vision` still reaches v4l indirectly.* The `cargo tree` guard covers normal and
  build edges with `--target all --all-features`. The vision manifest and sources no longer
  reference `soos_camera_v4l`, which `test_mac_vision_and_fixtures_do_not_depend_on_camera_v4l` also
  asserts. **PASS**
- *Scenario: the guard can never fail.* Four things show it can:
  - it asserts that the queried root package appears in the output;
  - a witness test over camera-v4l, pam, daemon and gui;
  - a self-contained stub `v4l` manifest;
  - my own `tokio` injection, which made it fail.

  It uses `cargo tree` instead of the `cargo metadata` named in #360. Both query the resolved graph
  with features and targets, so they are equivalent for this check. **PASS**
- *Scenario: scope against the acceptance criteria.* This first PR covers the #360 items that apply
  to it:
  - the ADR and the `soos-frame` crate;
  - a test proving the buffer is wiped on drop;
  - `frame` in the business-crate list;
  - the guard and a proof that it can fail;
  - `soos-biometric-store` classified as "portable by inspection", with its list of POSIX calls;
  - the non-required macOS job;
  - `Docs/MACOS_PORT.md` with the owner checklist;
  - the matrix rows and walkthrough 198 (the number is reserved, as line 6 of walkthrough 199
    confirms).

  There is no `AI/BACKLOG.md` entry (see MINOR-1). **PASS with MINOR**

### PAM Concurrency & Deadlines

- *Scenario: the change touches `crates/pam`.* It is not in the patch, and the `soos-pam` dependency
  graph in `Cargo.lock` is unchanged. **PASS (not applicable)**

### Panic Safety & Fail-Closed

- *Scenario: new `unwrap`, `expect` or indexing in production code.* The production code of
  `soos-frame` is only the moved code (`checked_div(..).unwrap_or(0)` and saturating arithmetic).
  **PASS**
- *Scenario: `pr_loop.sh` merges with a failed or pending required check.*
  - If the watch fails for any reason, the script loops on the gate:
    - `0` only when at least one non-advisory check exists and all of them are `pass` or `skipping`;
    - `3` (pending) waits within the original `WATCH_START` budget and exits 1 when it runs out;
    - anything else exits 1: fail, cancel, malformed output, a gh error, or a missing helper (127).
  - Next, `CI Success` for the exact `TARGET_HEAD_SHA` must be `success` (unchanged).
  - A final gate snapshot must then return exactly `0` before `gh pr merge --match-head-commit`.
  - If the watch succeeds, the final snapshot still runs.
  - The exit status is captured with `|| gate_rc=$?` under `set -euo pipefail`.

  **PASS**
- *Scenario: the helper lets a PR through on malformed gh output.* I simulated each of these cases:
  - empty output, a count of `0`, or a count mismatch;
  - a record with no tab or an extra tab, or an empty name;
  - the unknown bucket `neutral`, or gh exiting with 8;
  - advisory names with a trailing space or a near match.

  All of them block, and a final record without a newline is still read. The advisory match
  compares the exact job name as a quoted literal. The name is unique across workflows, which the
  invariant asserts. The real `gh pr checks 374` output uses bare job names, so the allowlist entry
  matches what gh emits. **PASS**

### Test Integrity & Anti-Weakening

- *Scenario: the wipe-on-drop test passes against a `Frame` that does not wipe.* The probe allocator
  checks every byte of the watched allocation at `dealloc`. The capacity (4096) is larger than the
  length (3000), so the unused capacity is checked too. `test_drop_probe_detects_an_unwiped_buffer`
  proves the probe reports non-zero bytes for a plain `Vec`. The hooks use only atomics and never
  allocate, and a mutex runs the tests one at a time. **PASS**
- *Scenario: the gate tests only check the happy path.* `test_mac_pr_checks_gate_decisions` covers 21
  fixtures plus 2 argument cases and asserts the exact gh argv. **PASS**
- The step-3 listing shows no removed or weakened assertion. **PASS**

### Memory, Bounds & Secrets

- *Scenario: zeroization is lost in the move.* `impl zeroize::Zeroize for Frame` and
  `impl Drop for Frame` are in `soos-frame`. `test_mac_frame_crate_is_pure_and_forbids_unsafe` and
  `std::mem::needs_drop::<Frame>()` assert this. As the probe confirms, `zeroize` for `Vec<u8>` also
  wipes the unused capacity. **PASS**
- *Scenario: `unsafe` in a business crate.* `soos-frame` starts with `#![forbid(unsafe_code)]` and is
  in both business-crate lists. The only new `unsafe` is the test-only allocator in the adapter crate
  `soos-camera-v4l`, and each block has a `// SAFETY:` comment. **PASS**
- `Frame` derives `Debug`, which prints the pixel buffer. This already existed and is preserved
  exactly, since #360 asks to preserve redaction, not to add it. See SUGGESTION-3.

### Supply Chain & Automation

- *Scenario: an unpinned or new third-party action.*
  - `actions/checkout`, `Swatinem/rust-cache` and `actions/cache/{restore,save}` are pinned to a
    40-hex SHA with a version comment. Each line is identical to one a Linux job already uses, which
    the invariant asserts.
  - The toolchain comes from the preinstalled `rustup` and `rust-toolchain.toml`.
  - `ort-sys` fetches ONNX Runtime with its own SHA-256 check, through the existing bounded-retry
    script.
  - `Cargo.lock` gains no new crate except the workspace-local `soos-frame`.

  **PASS**
- *Scenario: a privilege or secret is exposed.*
  - There are no `secrets.*` and no job-level `permissions:` (the workflow has `contents: read`).
  - `persist-credentials: false` is set.
  - Cache saves only run on `refs/heads/main`, so a PR cannot poison the cache, and the cache key
    includes `runner.os`.
  - No `${{ github.event.* }}` appears inside `run:`.

  **PASS**
- *Scenario: the macOS job silently becomes required, or its failures are hidden.* It is not in
  `ci-success.needs`, has no `continue-on-error`, and is the only advisory entry in the gate. **PASS**

### English-Only Policy

- *Scenario: non-English text in code, docs, scripts or the commit message.* The only non-ASCII
  characters are em dashes and arrows. The commit message and all added docs are in English. **PASS**

## 4. Detailed Findings & Action Items

- **[MINOR]** `AI/VERIFICATION_MATRIX.md:2333`: the #360 acceptance criteria ask for an
  `AI/BACKLOG.md` entry. The matrix instead calls #367 a "GitHub-only issue: no backlog id", and no
  backlog line is added. Six earlier GitHub-only issues in this register did the same. The owner
  should accept this deviation explicitly, or the entry can be added during traceability.
- **[MINOR]** `scripts/pr_loop.sh:172`: the fail-fast `--watch` still waits for the advisory macOS
  job. A slow macOS queue can hold every merge for up to 45 minutes, the watch limit. If a
  non-advisory check is still pending when the watch times out, the pending loop has no time left
  and blocks. This never merges anything wrongly; it only slows merges down. A later change could
  watch with the gate helper alone.
- **[SUGGESTION]** `crates/camera-v4l/Cargo.toml:21`: only `tests/frame_zeroize_tests.rs` still uses
  `zeroize`, so it could move to `[dev-dependencies]`.
- **[SUGGESTION]** `.github/workflows/ci.yml:544`: `macos-latest` changes whenever GitHub rotates
  its images (#360 suggested `macos-14`). Pinning `macos-15` (arm64) would keep the advisory job's
  environment reproducible. It is acceptable as is because the job is non-required.
- **[SUGGESTION]** `crates/frame/src/frame.rs:48`: the derived `Debug` of `Frame` prints the raw
  buffer. This already existed and moved unchanged, so it is not a regression. Now that another
  repository shares the type, a redacted `Debug` could be a follow-up issue.

## 5. Final Verdict

**VERDICT: APPROVED**
