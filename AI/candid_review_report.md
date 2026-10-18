# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/readme-web-captures`
- **Base (merge-base)**: `1043be6`
- **Reviewed-Diff-Fingerprint**: `a3ff8aa40f4bed3ea3cb98f596f9020d34dd548a146acee66d71063758a65833`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/241_readme_web_captures.md`,
  `Docs/README_ASSETS.md`, `Docs/assets/MANIFEST.toml`,
  `Docs/assets/screenshots/web-unlocked-light.png`, `Docs/assets/screenshots/web-unlocked-dark.png`,
  `Docs/assets/screenshots/web-locked-alerts-light.png`, `Docs/assets/screenshots/web-locked-alerts-dark.png`,
  `README.md`, `scripts/capture_readme_web.sh`, `tests/fixtures/mod.rs`,
  `tests/fixtures/tests/readme_tour_seen_tests.rs`

## 1. Executive Summary

GitHub #448 (part of #356) commits the four phone web app README captures, refreshes the asset
manifest, adds a fixture-only `/readme/tour-seen` page that sets the web app's own first-run tour
flag before loading `/`, opens that page first from `scripts/capture_readme_web.sh` (with `TZ=UTC`),
documents the change and adds matrix row RAC20 while keeping RAC18 Pending. The change is confined
to documentation, assets, the test-fixtures crate and a dev-only script. No production crate,
`crates/remote`, CI or packaging file is touched. No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test/fixture files touched: `tests/fixtures/mod.rs` (fixture library, additions only),
  `tests/fixtures/tests/readme_tour_seen_tests.rs` (new file).
- Removed or changed assertions (`^-` lines with `assert`, `#[test]`, `proptest!`, `should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.

The diff of `tests/fixtures/mod.rs` has zero removed lines; the four hunks only add constants, the
`FixtureResponse::Page` variant, one route arm and one serve arm. No existing test was modified.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Route: `route()` returns `NotFound` for any non-`GET` method before the match, and the new arm
  matches the exact string `/readme/tour-seen`; `?x=1`, trailing slash, `/readme` and `/readme/` are
  `NotFound` (asserted for every scene). PASS.
- Adding a variant to `FixtureResponse`: the only exhaustive match is `serve`, which gains the
  `Page` arm; the crate compiles and clippy is clean. PASS.
- Flag correctness: `crates/remote/assets/app.js:2252-2262` defines `TOUR_STORAGE_KEY =
  "soos-remote-tour-v1"`, `TOUR_STORAGE_SEEN = "seen"` and reads `getItem(KEY) === SEEN`; the page
  writes the same pair on the same loopback origin, and the test pins both constants against
  `app.js`, so a renamed key fails the test. The images confirm no tour overlay. PASS.
- `TZ=UTC` is applied only to the browser invocation; the fixture server and strip step are
  unchanged. PASS.
- RAC18 stays `⬜ Pending` (owner visual check), RAC20 added as Verified with two existing test
  names; `matrix_citations` passes. PASS.
- MANIFEST SHA-256 values match the committed PNG files (`fe0ad0e99436…`, `f28624f28fda…`,
  `f05c0742e68f…`, `a4930aac8a62…`); `readme_assets_contract` passes. PASS.

### PAM Concurrency & Deadlines
- No change in `crates/pam` or any runtime crate. Not applicable. PASS.

### Panic Safety & Fail-Closed
- New library code has no `unwrap`/`expect`/indexing; the `Page` arm propagates the
  `write_response` `io::Result`. Test-only `unwrap` is under the suite's documented `allow`. No
  path to `Allow`/`PAM_SUCCESS` exists in the diff. PASS.

### Test Integrity & Anti-Weakening
- The new tests fail against plausible wrong implementations: a route that ignores the method or
  accepts a prefix/query, a page with the wrong key or value, a missing `try`, a second storage
  write, a non-HTML content type, or a script that opens `/` directly. PASS.
- Ran: `cargo test --locked -q -p soos-test-fixtures` (all green, including the 2 new tests),
  `cargo test --locked -q -p soos-invariants` (792 passed, 0 failed), `cargo fmt --all -- --check`,
  `cargo clippy -p soos-test-fixtures --all-targets -- -D warnings` (clean), `bash -n` on the script.

### Memory, Bounds & Secrets — including image privacy
- The page is a static `&'static str`; no allocation growth. The test asserts it carries no `@`,
  URL scheme, `//`, `src=`/`href=`, `fetch(`, `ts.net`, loopback host, `user`, `login` or the demo
  host. PASS.
- **Image privacy (each PNG opened and inspected visually)**:
  - `web-unlocked-light.png` / `web-unlocked-dark.png`: soos logo, "REMOTE" badge, "Unlocked /
    Active session / Updated 1 s ago", Lock/Unlock buttons, "No failed password attempts", offline
    history with one synthetic interval "Oct 18, 08:32 AM – 04:32 PM · 8 h · Sleep".
  - `web-locked-alerts-light.png` / `web-locked-alerts-dark.png`: "Locked / Active session, idle
    for 11 min", "2 failed password attempts, last at 05:29 PM (sudo)" (05:30 PM in dark),
    detail "sudo, your account, 2 wrong passwords", Acknowledge button.
  - No real photo or face, no real login or user name (only "your account"), no host name, IP
    address, e-mail, passkey id or push endpoint is visible. Only the fixture's synthetic data
    appears. The first-run tour covers none of the four captures. PASS.
  - PNG chunk scan: every file contains only `IHDR`, `IDAT`, `IEND` (no `tEXt`/`iTXt`/`zTXt`/
    `eXIf`/`tIME`), 780×1688 pixels. PASS.

### Supply Chain & Automation
- No `Cargo.toml`/`Cargo.lock`/`deny.toml`/`.github/`/`packaging/` change; `git diff
  origin/main...HEAD -- crates/` is empty, so `soos-remote` is unchanged; nothing under `.github/`
  or `packaging/` references the capture tooling, and RAC15 `test_rac_capture_tooling_is_dev_only`
  passes. The script stays `set -euo pipefail`, offline, loopback-only (RAC16 passes). PASS.

### English-Only Policy
- All added code, comments, docs, alt text and the walkthrough are in English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `Docs/assets/screenshots/web-locked-alerts-{light,dark}.png` — the alert clock time
  differs between the light (05:29 PM) and dark (05:30 PM) captures because each capture uses its
  own wall clock across a minute boundary. Cosmetic only; a future rerun could pin the capture
  clock per scene so both variants match.
- **[SUGGESTION]** `scripts/capture_readme_web.sh` — the captures embed the capture date and UTC
  time of day (Oct 18, ~17:30 UTC). This is not personal data and the time zone is hidden by
  `TZ=UTC`, but a fixed synthetic capture clock would make the images fully reproducible.

## 5. Final Verdict

**VERDICT: APPROVED**
