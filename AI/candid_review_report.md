# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/readme-web-captures` (PR #450, GitHub #448, part of #356)
- **Base (merge-base)**: `33b5672`
- **Reviewed-Diff-Fingerprint**: `89750117694721eeadcf13d8d813efaf585724ea03fa9169b25297016443d640`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/241_readme_web_captures.md`,
  `Docs/README_ASSETS.md`, `Docs/assets/MANIFEST.toml`,
  `Docs/assets/screenshots/web-{unlocked,locked-alerts}-{light,dark}.png`, `README.md`,
  `scripts/capture_readme_web.sh`, `tests/fixtures/examples/readme_web_fixture_server.rs`,
  `tests/fixtures/mod.rs`, `tests/fixtures/tests/readme_fixed_clock_tests.rs`,
  `tests/fixtures/tests/readme_tour_seen_tests.rs`

## 1. Executive Summary

Re-review after merging `origin/main`. The merge-base is now `origin/main` itself (`33b5672`), so the
three-dot diff contains only branch work and no foreign change from the merge. The branch commits four
synthetic phone web app README captures, adds two fixture-only pages (`/readme/tour-seen`,
`/readme/capture`) and a fixed synthetic capture clock (`--clock fixed`, 2026-10-01T09:41:00Z,
checked with `date -u -d @1790847660`) to `soos-test-fixtures`, and updates the capture script,
manifest, README, docs, matrix (RAC20 and RAC21 new; RAC18 stays Pending) and walkthrough 241.
`git diff origin/main...HEAD -- crates/` is empty. No CI or packaging file changes. No blocking finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/fixtures/mod.rs` (fixture library, additions only),
  `tests/fixtures/examples/readme_web_fixture_server.rs` (example binary, `--clock` added),
  `tests/fixtures/tests/readme_fixed_clock_tests.rs` (new), `tests/fixtures/tests/readme_tour_seen_tests.rs` (new).
- Removed or changed assertions / `#[test]` / `proptest!` / `should_panic`: **none** (grep empty).
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- `serve` keeps its signature and behavior (it delegates to `serve_with_clock(.., FixtureClock::Real)`),
  so the existing RAC8, RAC9 and RAC19 tests run unchanged. No contract migration needed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- `capture_page` returns `None` unless `<head>` and the `app.js` script element each occur exactly
  once. The real `crates/remote/assets/index.html` satisfies it, and the test proves that removing the
  two insertions gives back the original bytes. A missing marker gives 404, tested over the loopback server. PASS.
- Routes match the exact path and `GET` only (query string, trailing slash, `HEAD`, `POST`, `.html` give NotFound). PASS.
- `--serve-ms 240000` is inside the example's clamp `[1000, 300000]`. PASS.
- Fixed clock: status `checked_unix_ms` equals the pinned instant over the wire. The last alert is at
  the instant minus 180 s, consistent with the 09:38 AM shown in the captures. PASS.
- The tour flag constants are asserted against the literal declarations in `app.js`, so a renamed key
  breaks the test instead of silently showing the tour again. PASS.
- `soos-remote` is unchanged (no crate diff). PASS.

### PAM Concurrency & Deadlines
- No PAM code touched. N/A, PASS.

### Panic Safety & Fail-Closed
- The new fixture code has no `unwrap`/`expect`/indexing. I/O errors map to 404 or are ignored per
  connection. No authorization path is touched; nothing can reach `Allow`/`PAM_SUCCESS`. PASS.

### Test Integrity & Anti-Weakening
- The new tests fail against plausible wrong implementations: wrong instant, clock script inserted
  after `app.js`, extra bytes in the served page, a non-exact route match, a key drift from `app.js`,
  or the script not using `--clock fixed`, `/readme/capture` or `/readme/tour-seen`.
- Ran `cargo test --locked -q -p soos-invariants -p soos-test-fixtures -- --test-threads=3`: all pass
  (809 invariant tests, RAC15 `test_rac_capture_tooling_is_dev_only` included). PASS (see SUGGESTION 1).

### Memory, Bounds & Secrets
- Visual check of all four PNGs (780x1688): only synthetic fixture data ("your account", `sudo`,
  a canned offline period). No photo or face, no login or user name, no host name, IP address,
  e-mail, passkey id or push endpoint. No first-run tour overlay.
- Both locked-alerts captures show identical times (idle for 12 min, "Updated 0 s ago", alerts at
  09:38 AM). The only date shown is Oct 1 (offline history); the real date 2026-10-18 appears nowhere.
- PNG chunks are only `IHDR`, `IDAT` and `IEND` (no `tEXt`, `tIME` or `eXIf`). The SHA-256 of each
  file matches `MANIFEST.toml`.
- `TOUR_SEEN_PAGE` and `CAPTURE_CLOCK_SCRIPT` are asserted free of URLs, `@`, `user`, `login`,
  loopback names and the demo host. The server stays loopback-only and time-bounded. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or packaging change; no CI or packaging file enables the
  capture tooling (RAC15 test green). The script keeps `set -euo pipefail` and installs or downloads
  nothing; `TZ=UTC` only scopes the browser process. PASS.

### English-Only Policy
- All code, comments, docs, matrix rows and the walkthrough are in English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `tests/fixtures/tests/readme_fixed_clock_tests.rs:554` — the loop compares
  `route(.., now)` with `route(.., FixtureClock::Fixed.now_unix_ms())`, the same value on both sides,
  so it is close to tautological. The real determinism is already covered by the pinned instant and
  the over-the-wire `checked_unix_ms` check. Optional: drop it or compare two server responses.
- **[SUGGESTION]** `tests/fixtures/mod.rs` (`CAPTURE_CLOCK_SCRIPT`) — `window.Date` becomes a class,
  so a bare `Date()` call (without `new`) would throw a `TypeError` on the capture page. `app.js` and
  the other assets contain no bare `Date()` call today (grep), so the captures are unaffected.
  Optional hardening only.

## 5. Final Verdict

No CRITICAL, MAJOR or MINOR finding.

**VERDICT: APPROVED**
