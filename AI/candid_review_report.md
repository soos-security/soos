# Candid Review Report

- **Date**: 2026-10-20
- **Target Branch**: `chore/readme-capture-locale`
- **Base (merge-base)**: `6c4ecd4`
- **Reviewed-Diff-Fingerprint**: `27fb23cf40112f2760ac6a2610c33f49495cb304557a91955fe0d3dfd3938c2d`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/246_readme_capture_locale.md`, `Docs/README_ASSETS.md`, `README.md`, `scripts/capture_readme_web.sh`, `tests/fixtures/tests/readme_capture_locale_tests.rs` (new), `tests/fixtures/tests/readme_fixed_clock_tests.rs`

## 1. Executive Summary

Documentation and dev-only capture tooling change for GitHub #458 (part of #356): the optional web
capture script pins the browser time zone and locale, the README Screenshots intro names both capture
scripts, and a new fixture test strengthens the fixed-clock check. No production crate, PAM, daemon or
`soos-remote` code changes; no committed PNG or manifest changes.

## 2. Test Changes

- `tests/fixtures/tests/readme_capture_locale_tests.rs`: new file, two new tests (RAC22, RAC23).
- `tests/fixtures/tests/readme_fixed_clock_tests.rs`: three comment lines added above the existing loop;
  no assertion, `#[test]` or code line removed or changed (mechanical grep of removed assert/test lines: empty).
- No `#[ignore]`, `should_panic`, tolerance or inline test module added or removed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Both browser runs use `env "${BROWSER_ENV[@]}"`: checked that no run kept a bare `TZ=UTC` prefix, so the
  tour-seen run and the screenshot run share the same locale (a mismatch would only matter for
  `localStorage`, which is locale-independent). PASS.
- `--lang=en-US` is in the shared `flags=(…)` array, so the dark/light `flags+=` additions keep it. PASS.
- Effect verified empirically, not assumed: unpinned script under `LANG/LC_ALL=de_DE.UTF-8` changed all four
  PNGs (24 h clock); pinned script under the same locale plus `TZ=Asia/Tokyo` and under the host default
  gave byte-identical PNGs, so no regeneration or manifest refresh is needed. PASS.
- `scripts/capture_readme_web.sh` still contains `http://127.0.0.1:$port/readme/capture`, `--clock fixed`,
  `--user-data-dir`, `mktemp -d` and none of the RAC16 forbidden words (`env` is not forbidden). PASS.
- README claim checked against the fixture: the web bodies carry `"account":"owner"`, so a first draft
  saying "no user name at all" was inaccurate and was reworded to "no login, host name or address" (the
  RAC9 wording). PASS after fix.

### PAM Concurrency & Deadlines
- Not touched. PASS.

### Panic Safety & Fail-Closed
- No production code. Test code uses `unwrap`/`expect` under the same `#![allow]` as sibling suites;
  slices use `.get(..)` (clippy `indexing_slicing` clean). PASS.

### Test Integrity & Anti-Weakening
- RAC22 fails on the `origin/main` script (no `BROWSER_ENV` definition) and passes after the change.
  A plausible wrong implementation (pin only on the screenshot run, or `--lang` only in the dark branch)
  fails the "every browser run" and "flags block" checks.
- RAC23 would fail if the fixed-clock server used the real clock (bodies differ from `route` at the fixed
  instant and between requests 1.1 s apart), and its instant-dependence check prevents the vacuous
  self-comparison of the old loop. It passes on the current fixture by design (coverage, stated in the
  walkthrough). The unlocked-scene alerts body is constant and is asserted as such instead of skipped.
- Server lifetime 4 s versus about 1.2 s of requests; listeners bound before the threads start: no race. PASS.

### Memory, Bounds & Secrets
- Loopback-only listeners, bounded read timeout (3 s), temp directory per process id, removed at the end.
  No identity or secret in any new text. PASS.

### Supply Chain & Automation
- No `Cargo.*`, workflow or hook change; the script installs and downloads nothing. PASS.

### English-Only Policy
- All added text is English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR, fixed]** `README.md:69` — "no user name at all" contradicted the fixture's `"account":"owner"`;
  reworded (commit `2e908e4`).

No open CRITICAL or MAJOR finding.

## 5. Final Verdict

**VERDICT: APPROVED**
