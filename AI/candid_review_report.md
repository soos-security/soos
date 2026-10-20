# Candid Review Report

- **Date**: 2026-10-20
- **Target Branch**: `feat/remote-motion` (PR #436, GitHub #433 = PR 2 of #358), head `ac7413e` on `origin/main` `da320a4`
- **Base (merge-base)**: `da320a4`
- **Reviewed-Diff-Fingerprint**: `d23230e4018500a7016ca73b029e631a387099b95ebfa7b4bd7f4362c69952a3`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_motion.md`, `AI/auditor_constraints_remote_motion.md`, `AI/tester_contract_motion.md`, `AI/walkthroughs/235_remote_web_motion.md`, `Docs/README_ASSETS.md`, `Docs/REMOTE_COMPANION.md`, `Docs/UI_MOTION.md`, `Docs/assets/MANIFEST.toml`, `Docs/assets/screenshots/web-unlocked-dark.png`, `Docs/assets/screenshots/web-unlocked-light.png`, `crates/remote/assets/app.js`, `crates/remote/assets/style.css`, `scripts/capture_readme_web.sh`, `tests/fixtures/tests/readme_capture_timeout_tests.rs`, `tests/invariants/src/lib.rs`, `tests/invariants/src/remote_motion_contract.rs`

## 1. Executive Summary

This independent review covers the full raw diff (`target/candid_diff.patch`, 3409 lines) against the merge base
`da320a4`. It focuses on the three commits added after the CHANGES_REQUESTED review of `c3d3609`
(`c5700fa`, `860be80`, `ac7413e`). `crates/` and `tests/invariants/` have not changed since that review, so its
verdict on the motion code still holds: class-only hooks, one guarded `startViewTransition`, security truth
written first, a single reduced-motion block, and unchanged CSP, routes, assets and ids. The hunks were re-read.

Every previous finding is resolved:
- MAJOR (UIM7 cites no resolvable evidence): the row now cites `Docs/prototypes/ui_motion_prototype.html`
  and `ui_motion_spec_contract::test_uim_s4_prototype_is_self_contained_and_on_brand`.
  `matrix_citations::test_matrix_claimed_rows_cite_at_least_one_resolvable_evidence` passes.
- MINOR (stale unlocked captures): both PNGs were regenerated and show the star turned 45°. The `sha256sum` of
  every PNG under `Docs/assets/screenshots/` equals its `MANIFEST.toml` entry (`186c80a2…` dark, `db661b7d…`
  light). The locked-alerts and GUI hashes did not change.
  `test_rac_docs_assets_are_listed_in_the_manifest_with_matching_sha256` passes.
- MINOR (duplicate `---`): a scan of the matrix finds no two `---` lines separated only by blank lines.
- SUGGESTION (unbounded browser runs): implemented in `ac7413e` with RAC24.

`cargo test --locked -p soos-invariants -p soos-remote -p soos-test-fixtures -- --test-threads=3`
exited 0: 1333 passed, 0 failed, 0 ignored (53 test binaries). `bash -n scripts/capture_readme_web.sh` passes.

## 2. Test Changes (mechanical listing)

- Test files touched: `tests/fixtures/tests/readme_capture_timeout_tests.rs` (new, RAC24),
  `tests/invariants/src/lib.rs` (adds only `mod remote_motion_contract;`), `tests/invariants/src/remote_motion_contract.rs` (new, RMO1–RMO12).
- Removed or changed assertions (`^-` lines with assert, #[test], proptest or should_panic): one hit, the
  `AI/DECISIONS.md` line of ADR 2026-10-13. A word diff shows a pure addition (an implementation note for
  item (6)), and the line is prose, not a test. No test assertion was removed or changed.
- New escape hatches (`#[ignore]`, `cfg(any())`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

No pre-existing assertion changed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- `scripts/capture_readme_web.sh` runs under `set -euo pipefail`. Its `stop_browser` and `await_browser`
  functions were extracted and run against stand-in browsers (timeout 2 s, kill-after 1 s):
  - A run that exits 0 after 0.3 s: the script continues and returns in about 0.4 s. Bash reaps the background
    child on SIGCHLD, so `tail --pid` does not wait on a zombie. `wait` gets status 0. PASS.
  - A run that exits before `tail` starts: the script continues in 9 ms. PASS.
  - A run that exits 7: the message names the run and the script exits 7. The EXIT trap does nothing because the pid is already cleared. PASS.
  - A hang that honours SIGTERM: message, SIGTERM, and the script exits 124 at about 2 s. PASS.
  - A hang that ignores SIGTERM: SIGKILL after the grace period, exit 124 at about 3 s, and no orphan left
    (`pgrep` finds nothing afterwards). PASS.
  - `exit 124` runs `cleanup` again: `stop_browser` sees an empty `browser_pid` and returns. PASS.
- Process identity: `env … "$browser"` execs the browser. Both Chrome launchers on this host
  (`/usr/bin/google-chrome-stable`, `/opt/google/chrome/google-chrome`) `exec` the real binary, so `$!` is
  the Chrome browser process that receives SIGTERM and SIGKILL. PASS (see the SUGGESTION on process groups).
- Budget: the 45 s bound is at least 5 × `SETTLE_MS` (4000 ms), and 4 × (45 + 5) = 200 s fits within
  `--serve-ms 240000`. A timeout exits the script at once, so at most one run per scene reaches the bound. PASS.
- RAC15 (`test_rac_capture_tooling_is_dev_only`), RAC20 (`readme_tour_seen_tests`), RAC21
  (`readme_fixed_clock_tests`), RAC22 (`test_rac_web_capture_pins_the_browser_locale`; both runs keep the prefix
  `env "${BROWSER_ENV[@]}" "$browser" "${flags[@]}"`) and RAC23 all pass. PASS.

### PAM Concurrency & Deadlines
- Nothing changed under `crates/pam`. N/A.

### Panic Safety & Fail-Closed
- No Rust production code changed. The script fails closed: a browser run that times out or fails stops the
  script before `--strip-png` writes that run's output, and the exit status of a killed Chrome is never trusted. PASS.

### Test Integrity & Anti-Weakening
- RAC24 fails against each plausible wrong implementation tried:
  - a run in the foreground (no trailing `&`);
  - a missing `browser_pid=$!`, or no `await_browser` right after a run;
  - no `timeout` in `await_browser`;
  - no `exit 124` or no `stop_browser` on expiry;
  - a run status that is swallowed;
  - a missing SIGTERM or SIGKILL step;
  - a `cleanup` that does not stop the browser;
  - a bound outside the settle or server budget.

  PASS.
- RAC24 is a static text check of the script, like RAC22. The runtime behaviour was checked by hand above. PASS.

### Memory, Bounds & Secrets
- The regenerated PNGs (780×1688) contain only `IHDR`, `IDAT` and `IEND` chunks. They show the synthetic unlocked
  scene ("Unlocked", "No failed password attempts", one synthetic offline period on Oct 1). They contain no user
  name, host name, e-mail, path or face, and `strings` finds nothing identifying. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or hook file changed. The script stays optional, installs nothing and
  uses only coreutils `timeout` and `tail`. PASS.

### English-Only Policy
- The script comments and messages, the test, matrix row RAC24 and `Docs/README_ASSETS.md` are in English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `scripts/capture_readme_web.sh:79-82`: SIGTERM and SIGKILL reach only the Chrome browser
  pid. Chrome's helper processes normally exit after a SIGKILL once their IPC channel closes. Killing the
  process group (`setsid` on start, then `kill -- -"$pid"`) would make sure of it. Optional: the current
  behaviour meets RAC24.
- **[SUGGESTION]** `scripts/capture_readme_web.sh:49`: a normal run takes about 21 s on the capture host
  (`Docs/README_ASSETS.md`), which is about half the 45 s bound. A slower host may hit a false timeout. The
  script names the run that timed out, so the failure is visible and the script can be rerun. Optional.

## 5. Final Verdict

**VERDICT: APPROVED**
