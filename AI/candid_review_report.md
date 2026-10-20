# Candid Review Report

- **Date**: 2026-10-20
- **Target Branch**: `chore/readme-capture-locale`
- **Base (merge-base)**: `7ab2eae` (`origin/main` after #460; the branch was rebased from `6c4ecd4` onto it)
- **Reviewed-Diff-Fingerprint**: `a5c8ba98018d5f569373354310e9826d37e184aa0bc8f86cb0611e5b71324dee`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/246_readme_capture_locale.md`, `Docs/README_ASSETS.md`, `README.md`, `scripts/capture_readme_web.sh`, `tests/fixtures/tests/readme_capture_locale_tests.rs` (new), `tests/fixtures/tests/readme_fixed_clock_tests.rs`

## 1. Executive Summary

This is an **independent review that replaces the author's self-review** previously recorded in this
file: it was produced by a separate reviewer with no authoring context, working only from the frozen
patch (`target/candid_diff.patch`) and the code it lands in.

Scope (GitHub #458, part of #356): the optional, dev-only web capture script pins the browser time
zone and locale through one `BROWSER_ENV` array plus `--lang=en-US --accept-lang=en-US`; the README
"Screenshots" intro now attributes the `soos-gui` captures and the phone web app captures to their
respective scripts; a new fixture test file adds RAC22 (script contract) and RAC23 (server-level
fixed-clock check); one comment is added in `readme_fixed_clock_tests.rs`. No production crate, PAM,
daemon or `soos-remote` code changes; `Docs/assets/` (PNGs and `MANIFEST.toml`) is byte-identical to
`origin/main`.

Re-review after rebase: the branch was rebased from `6c4ecd4` onto `7ab2eae` (#460, setup.sh
scanners: `tests/invariants/src/setup_scanner_contract.rs`, `setup_user_phase_contract.rs`,
`tests/invariants/src/lib.rs`, matrix rows, walkthrough 245). The newly frozen patch, compared with
the patch reviewed at fingerprint `27fb23cf…` (both excluding `index` lines and this report), is
byte-identical, so the branch's own changes are unchanged. No interaction with #460: #460 touches no
file of this diff except `AI/VERIFICATION_MATRIX.md` (separate rows, no RAC row) and this report; its
scanners read only `scripts/setup.sh`, not `scripts/capture_readme_web.sh`. Walkthrough numbering
stays sequential (245 from #460, 246 here). `bash -n` and the full test command were re-run on the
rebased tree and are green.

## 2. Test Changes

Mechanical listing from the frozen patch:

- Test files touched: `tests/fixtures/tests/readme_capture_locale_tests.rs` (new, 2 tests),
  `tests/fixtures/tests/readme_fixed_clock_tests.rs` (3 comment lines added).
- Removed/changed `assert`/`#[test]`/`proptest!`/`should_panic` lines: **none** (grep empty).
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` changes: **none**.

The only edit to an existing test file is a comment above the existing equality loop of
`test_rac_fixed_capture_clock_is_pinned`; that loop and all its assertions are untouched. The
comment accurately states the loop is weak (`route` is pure, so it compares a body with itself) and
points to RAC23. No contract migration is needed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Script: `BROWSER_ENV=(TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 LANGUAGE=en_US)` is a plain bash
  array; `env "${BROWSER_ENV[@]}" "$browser" "${flags[@]}" …` passes each assignment as a separate,
  correctly quoted word, and `env` execs the browser with those variables set. Works under the
  script's `set -euo pipefail` (array non-empty). `bash -n` passes. Both browser runs (tour-seen,
  then screenshot) use it; the old bare `TZ=UTC` prefixes are gone, and `TZ=UTC` is preserved inside
  the array. → PASS.
- Flags: `--accept-lang` is a valid Chrome (headless) switch and sets `Accept-Language` /
  `navigator.languages`. `--lang` is a valid Chrome switch, but on Linux Chrome derives the default
  ICU locale mainly from `LANGUAGE`/`LC_ALL`/`LANG`, so the environment pin is what actually decides
  `toLocaleTimeString([], …)` output there; `--lang` is redundant but harmless (see SUGGESTION). The
  author's reported evidence (German locale + `TZ=Asia/Tokyo` host gives byte-identical PNGs with the
  pin, a 24 h clock without it) is consistent with this. → PASS.
- README claims checked against the fixture data in `tests/fixtures/mod.rs`: the web fixture's only
  person-like field is `"account":"owner"` in the locked-scene alert history, which is an account
  *class* rendered by `app.js` (`ALERT_ACCOUNTS.owner` = "your account"), not a login or user name;
  `DEMO_HOST` is documented as never served. So "canned, identity-free data with no login, host name
  or address" is true, and the old intro's claim that *all* screenshots use the user `alice` is
  correctly narrowed to the `soos-gui` captures. "with a pinned `en-US` locale" matches the script.
  → PASS.
- `Docs/README_ASSETS.md` §4 text matches the script exactly (variables, values, flags). → PASS.

### PAM Concurrency & Deadlines

Not applicable: no file under `crates/pam` or any runtime crate changes. → PASS (N/A).

### Panic Safety & Fail-Closed

No production code changes; `unwrap`/`expect`/`panic!` appear only in the new integration test file
under an explicit `#![allow(..., reason = ...)]`, consistent with the other fixture tests. → PASS.

### Test Integrity & Anti-Weakening

- RAC22 mutation checks run by the reviewer (script temporarily swapped, then restored; working tree
  verified clean afterwards):
  - `origin/main` script (no `BROWSER_ENV`): test FAILS (line 111, definition count).
  - `--accept-lang=en-US` removed from the flags: test FAILS (line 155).
  - one browser run with the `env "${BROWSER_ENV[@]}"` prefix stripped: test FAILS (line 119).
  The test also forbids a second `--lang=` (`--accept-lang=` does not match `--lang=`), any line
  starting with `TZ=`, and requires exactly two browser runs (tour-seen and screenshot). Comment-line
  and `\`-continuation handling is correct, so commented-out code cannot satisfy it.
- RAC23 is non-vacuous: it first proves the bodies depend on the instant (`route` at +61 s differs for
  status, offline, locked alerts and the event prelude; the unlocked alerts body has no timestamp and
  is asserted equal), then checks over real HTTP that two `--clock fixed` servers answer exactly the
  fixed-instant `route` bodies with `200 OK`, `Content-Type: application/json` and matching
  `Content-Length` (format confirmed against `write_response`), byte-identical after a >1 s sleep, and
  that the scenes differ. A server accidentally using `FixtureClock::Real` would fail the equality with
  the fixed-instant body. The author honestly labels RAC23 as coverage, not a red-phase contract.
- Server lifetime (4 s) exceeds the request window (~1.1 s plus requests); `serve_with_clock` returns
  `Ok(())` at its deadline, so the joins terminate. The temporary asset directory is per-PID and
  removed. → PASS.

### Memory, Bounds & Secrets

No secrets, frames or embeddings involved; fixture data is synthetic. The test binds loopback only.
→ PASS.

### Supply Chain & Automation

- No `Cargo.*`, `deny.toml`, `.github/` or `.githooks/` change; no new dependency.
- RAC15 (capture tooling dev-only, no CI enables it): a search of `.github/` for `capture_readme`,
  `chrome` and `chromium` is empty, and
  `soos-invariants::readme_assets_contract::test_rac_capture_tooling_is_dev_only` passes. The script
  still installs and downloads nothing. → PASS.
- `Docs/assets/` (web and `gui-*` PNGs, `MANIFEST.toml`): `git diff origin/main -- Docs/assets` is
  empty, and the invariants suite (manifest checks included) passes. → PASS.

### English-Only Policy

All added code, comments, docs, matrix rows and the walkthrough are in English. → PASS.

### Verification Run by the Reviewer

- `bash -n scripts/capture_readme_web.sh`: OK.
- `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/readme-capture-locale CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=3 cargo test --locked -p soos-test-fixtures -p soos-invariants -- --test-threads=3`:
  all green (invariants 847 passed, after the rebase onto #460; `readme_capture_locale_tests` 2 passed; `readme_fixed_clock_tests`,
  `readme_tour_seen_tests`, `readme_fixture_tests`, `fixture_generators_tests` and the fixtures unit
  tests all passed; 0 failed, 0 ignored).

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `scripts/capture_readme_web.sh:94` — on Linux, Chrome's `--lang` has little or no
  effect on the default ICU locale (the environment decides); the environment pin is the effective
  control. Keeping `--lang` is harmless; a future comment could say the environment is the
  authoritative pin on Linux.
- **[SUGGESTION]** `Docs/README_ASSETS.md:104` — the edited paragraph leaves one line well over the
  surrounding wrap width ("…byte-identical PNGs (checked on the capture host with the same");
  cosmetic reflow only.

No CRITICAL, MAJOR or MINOR findings.

## 5. Final Verdict

**VERDICT: APPROVED**
