# Candid Review Report

- **Date**: 2026-10-23
- **Target Branch**: `chore/rac15-bench-exception`
- **Base (merge-base)**: `fb2a0bf`
- **Reviewed-Diff-Fingerprint**: `c76800de8fa7338cecb6bee7b8332ca40b8beaf4210f5effc1e6b999c92c809c`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/252_rac15_browser_bench_exception.md`,
  `Docs/README_ASSETS.md`, `scripts/remote_browser_tests.sh`,
  `tests/invariants/src/readme_assets_contract.rs`

## 1. Executive Summary

The diff turns the dev-only Playwright bench (`scripts/remote_browser_tests.sh`) into a named, narrow
RAC15 exception, as approved by the owner on 2026-10-23 (additive migration). RAC15 gains one constant
and one exception branch that still flags `readme-capture` in the bench; no RAC15 assertion is changed or
removed. The bench now builds `--example readme_web_fixture_server` by name instead of `--examples`. A new
contract RAC26 pins the narrowness. The change was verified by mutation: RAC26 fails on the previous
script, and RAC15 still catches the fixture name in any other script or workflow. No blocking findings;
two MINOR and one SUGGESTION item below.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/readme_assets_contract.rs` only
  (`git diff --numstat origin/main...HEAD -- tests`: 40 added, 0 deleted).
- Removed or changed assertions: none (0 matches for `^-` lines with `assert` or `#[test]`).
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): none.
- Inline test modules: none touched.
- Changes, with justification:
  - `BROWSER_BENCH_SCRIPT` constant and the RAC15 exception branch (owner-approved additive migration of
    2026-10-23). The branch is evaluated before the generic check and `continue`s only for the exact path
    `scripts/remote_browser_tests.sh`; for that file it still pushes an offender on `readme-capture`.
    The `Dockerfile`/`save.sh`/`run_tests.sh` loop and the fixture-server/soos-remote assertions are
    untouched.
  - New test `test_rac_browser_bench_exception_is_narrow` (RAC26), purely additive.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: the exception widens to other files. The match is an exact equality on the relative path;
  a scratch `scripts/save_helpers_tmp.sh` naming `readme_web_fixture_server` and an edited
  `.github/workflows/browser.yml` naming it both still fail RAC15. PASS.
- Scenario: the capture feature leaks into the bench. Appending a `# readme-capture` comment to the bench
  fails both RAC15 and RAC26. Adding `--features x` to the build line fails RAC26. PASS.
- Scenario: RAC26 is vacuous. Restoring the `origin/main` script (`--examples`) fails RAC26 while RAC15
  passes, so RAC26 is the new, meaningful contract. PASS.
- Scenario: the build line selection picks the wrong line. The only line containing `cargo build` is the
  `report="$(cargo build --locked -p soos-test-fixtures --example readme_web_fixture_server \` line; the
  `[INFO]` and `usage_error` messages do not contain that substring. PASS.
- Scenario: `--all-features` on the build line. RAC26 does not reject it (mutation passed both tests).
  Harmless today: `soos-test-fixtures` declares no `[features]` and depends only on `soos-frame`, and
  with resolver 2 `--all-features` applies only to the selected package, so `readme-capture` (a
  `soos-gui` feature) cannot be enabled. See MINOR finding 1.
- Script behaviour: still `--locked`, still parses cargo's JSON report so `CARGO_TARGET_DIR` is honoured,
  still requires exactly one example executable or exit 2. `./scripts/remote_browser_tests.sh --
  --project=webkit` on this tree: the named build succeeded and 18 passed. PASS.

### PAM Concurrency & Deadlines
- Not applicable: no change under `crates/` or any runtime code. PASS.

### Panic Safety & Fail-Closed
- Not applicable to production code. The new test uses `unwrap_or_else(|| panic!(..))`, which is
  appropriate in a test. PASS.

### Test Integrity & Anti-Weakening
- Zero deleted lines under `tests/`. RAC15 keeps every previous check; the only behavioural change is
  the owner-approved exception for one exact path, which still enforces the feature-name ban.
- `cargo fmt --all -- --check`: clean. `cargo test --locked -p soos-invariants -- --test-threads=3`:
  879 passed, 0 failed. PASS.

### Memory, Bounds & Secrets
- No secrets, frames or embeddings involved. The fixture server remains synthetic and loopback-only. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.githooks/` or workflow change. The Browser workflow already triggers on
  `scripts/remote_browser_tests.sh` (`paths:` at lines 33 and 41), as the walkthrough states.
- `bash -n` and ShellCheck (`koalaman/shellcheck:stable`) on the script: clean. PASS.

### English-Only Policy
- Code, comments, docs, walkthrough, matrix row and commit message are English. Commit subject is 68
  characters and follows Conventional Commits (`test(invariants): ...`). Walkthrough number 252 follows
  251 and is unique. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `tests/invariants/src/readme_assets_contract.rs` (RAC26 negative assertion) — the build-line
  check rejects `--examples`, `--features` and `-F ` but not `--all-features`, while the walkthrough §4
  says RAC26 "refuses any feature flag on the build line". No current impact (the package has no
  features), but a follow-up could add `--all-features` to the rejected needles, or the walkthrough
  wording could be narrowed.
- **[MINOR]** `AI/walkthroughs/252_rac15_browser_bench_exception.md` §3 — "9 added lines" counts the
  3-line constant plus the 6-line branch; the branch itself is 6 lines. Also the `**Branch**` line in the
  header lacks a `- ` bullet and renders joined to the `**Issue**` line. Cosmetic.
- **[SUGGESTION]** RAC26 checks only the first `cargo build` line. A second build line elsewhere in the
  bench would escape the `--features` check (it would still be caught by RAC15 if it named the feature).

## 5. Final Verdict

No CRITICAL or MAJOR finding. The RAC15 migration is exactly the approved additive exception, RAC26 is
meaningful and fails on the previous script, and the script builds and runs.

**VERDICT: APPROVED**
