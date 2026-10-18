# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/readme-web-captures` (PR #450, GitHub #448, part of #356)
- **Base (merge-base)**: `2e510b8`
- **Reviewed-Diff-Fingerprint**: `ac2162a2ecc946c4a1eb1ff9961bda0876eb6e62341995570f65f0546dcf236d`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/241_readme_web_captures.md`,
  `Docs/README_ASSETS.md`, `Docs/assets/MANIFEST.toml`,
  `Docs/assets/screenshots/web-{unlocked,locked-alerts}-{light,dark}.png`, `README.md`,
  `scripts/capture_readme_web.sh`, `tests/fixtures/examples/readme_web_fixture_server.rs`,
  `tests/fixtures/mod.rs`, `tests/fixtures/tests/readme_fixed_clock_tests.rs`,
  `tests/fixtures/tests/readme_tour_seen_tests.rs`

## 1. Executive Summary

Independent re-review after the rebase onto `origin/main` `2e510b8`, which replaced the earlier
merge-of-main commit with a linear history. Main gained #442, #447, #446 and #452 since the old
merge-base `33b5672`. The frozen patch contains only branch work: 15 files, no `crates/`, `Cargo.*`,
`.github/` or packaging change.

Content proof against the pre-rebase branch `origin/chore/readme-web-captures` (`747681e`): every
branch file except `README.md` and `AI/VERIFICATION_MATRIX.md` has the same blob as before (fixtures,
example, both test files, script, the four PNGs, `MANIFEST.toml`, `Docs/README_ASSETS.md`,
walkthrough 241). For `README.md` and `AI/VERIFICATION_MATRIX.md`, the added and removed lines of the
branch against `2e510b8` are identical to the old branch's lines against the main it had merged
(`33b5672`). The remaining blob differences come only from main's own changes (#446 README rows and
security paragraph, #442/#446 matrix rows). No blocking finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/fixtures/mod.rs` (fixture library), `tests/fixtures/examples/readme_web_fixture_server.rs`
  (example binary), `tests/fixtures/tests/readme_fixed_clock_tests.rs` (new),
  `tests/fixtures/tests/readme_tour_seen_tests.rs` (new).
- Removed or changed assertions / `#[test]` / `#[tokio::test]` / `proptest!` / `should_panic`: **none** (grep empty).
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- Every removed code line under `tests/fixtures` was checked by hand. It is fixture-server plumbing
  for the clock parameter, not a check: the `use ... serve` import and the `serve(..)` call of the
  example (now `serve_with_clock(.., clock)`), the `handle(..)` signature (gains
  `clock: FixtureClock`), the `route(.., now_unix_ms())` call (now `clock.now_unix_ms()`), and the
  worker's `handle(..)` call. `serve` keeps its signature and delegates to
  `serve_with_clock(.., FixtureClock::Real)`, so the existing RAC8, RAC9 and RAC19 tests run
  unchanged. No contract migration needed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Interaction with #446: #446 rewrote the "Phone companion `soos-remote`" feature row (the
  `allow_funnel` / `[android]` rule) and the security paragraph (three tours, two kinds of offline
  alerts). The branch's README hunk sits only in the "Screenshots" section after the GUI table and
  does not overlap those lines. The captions still match the pictures (unlocked: no failed password,
  one synthetic offline period; locked: idle, two failed `sudo` passwords to acknowledge) and do not
  contradict #446: the captures show no tour because the fixture marks it seen, and the README makes
  no claim about the tour being visible in them. PASS.
- #447 (release dry run) and #452 (setup lock-widget flag) touch no file of this branch; the release
  dry-run contract does not inspect `Docs/assets` or PNG sizes. PASS.
- `capture_page` returns `None` unless `<head>` and `<script src="app.js"></script>` occur exactly
  once; the current `crates/remote/assets/index.html` has each once (lines 3 and 97). A missing marker
  gives 404. PASS.
- Routes: `route` rejects every method other than `GET` before the path match; `/readme/tour-seen` and
  `/readme/capture` are exact matches. PASS.
- `FIXED_CAPTURE_UNIX_MS` = 1 790 847 660 000 is 2026-10-01 09:41:00 UTC (`date -u -d @1790847660`).
  `--serve-ms 240000` is inside the example's clamp `[1000, 300000]`. PASS.
- The tour flag constants match `crates/remote/assets/app.js` (`TOUR_STORAGE_KEY = "soos-remote-tour-v1"`,
  `TOUR_STORAGE_SEEN = "seen"`). PASS.

### PAM Concurrency & Deadlines
- No PAM code touched. N/A, PASS.

### Panic Safety & Fail-Closed
- The new fixture code has no `unwrap`/`expect`/indexing; I/O errors map to 404 or are ignored per
  connection. No authorization path is touched; nothing can reach `Allow`/`PAM_SUCCESS`. PASS.

### Test Integrity & Anti-Weakening
- Step 3 output reviewed above: no pre-existing assertion removed or changed.
- Ran `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/readme-web-captures
  CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=3
  cargo test --locked -p soos-invariants -p soos-test-fixtures -- --test-threads=3` on the rebased
  tree: 843 passed, 0 failed, 0 ignored (828 invariants, 3 fixture generators, 3 fixed clock,
  7 README fixture, 2 tour-seen). PASS.

### Memory, Bounds & Secrets
- Visual check of all four PNGs (780x1688): only synthetic fixture data ("your account", `sudo`,
  09:38 AM alerts, an Oct 1 sleep period). No face or photo, no login or user name, no host name,
  IP address, e-mail, passkey id or push endpoint, no tour overlay.
- PNG chunks are only `IHDR`, `IDAT` and `IEND` (no `tEXt`, `tIME` or `eXIf`); `strings` finds no
  path, user or address text.
- `sha256sum` of each PNG equals its `MANIFEST.toml` entry:
  `web-locked-alerts-dark` `4cb600f4…af33`, `web-locked-alerts-light` `02a30d2b…c71b`,
  `web-unlocked-dark` `c150088e…8d98`, `web-unlocked-light` `03838f7b…6a35`. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or packaging change. The script keeps `set -euo pipefail`,
  installs or downloads nothing, and `TZ=UTC` only scopes the browser process. PASS.

### English-Only Policy
- All code, comments, docs, matrix rows and the walkthrough are in English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `README.md` "Screenshots" — the first sentence still says "All screenshots are
  generated ... by `scripts/capture_readme_assets.sh` ... the user `alice`", while the web captures
  come from `scripts/capture_readme_web.sh` (named in the next paragraph). Optional: say "The GUI
  screenshots" there. Not introduced by the rebase.
- **[SUGGESTION]** `tests/fixtures/tests/readme_fixed_clock_tests.rs` — the per-route loop compares
  `route(.., now)` with `route(.., FixtureClock::Fixed.now_unix_ms())`, the same value on both sides,
  so it is close to tautological; the pinned instant and the over-the-wire `checked_unix_ms` check
  carry the real contract. Carried over from the previous review.
- **[SUGGESTION]** `tests/fixtures/mod.rs` (`CAPTURE_CLOCK_SCRIPT`) — a bare `Date()` call (without
  `new`) would throw on the capture page because `window.Date` becomes a class; the assets contain no
  such call today. Carried over from the previous review.

RAC18 stays Pending for the owner's visual check.

## 5. Final Verdict

No CRITICAL, MAJOR or MINOR finding.

**VERDICT: APPROVED**
