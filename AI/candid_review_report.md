# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/readme-web-captures`
- **Base (merge-base)**: `1043be6`
- **Reviewed-Diff-Fingerprint**: `c3b85266bf8766f73cb52c9e2846e6fbe33127e43886838f1aa93f0c5ae2d695`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/241_readme_web_captures.md`,
  `Docs/README_ASSETS.md`, `Docs/assets/MANIFEST.toml`,
  `Docs/assets/screenshots/web-unlocked-light.png`, `Docs/assets/screenshots/web-unlocked-dark.png`,
  `Docs/assets/screenshots/web-locked-alerts-light.png`, `Docs/assets/screenshots/web-locked-alerts-dark.png`,
  `README.md`, `scripts/capture_readme_web.sh`, `tests/fixtures/examples/readme_web_fixture_server.rs`,
  `tests/fixtures/mod.rs`, `tests/fixtures/tests/readme_fixed_clock_tests.rs`,
  `tests/fixtures/tests/readme_tour_seen_tests.rs`

## 1. Executive Summary

GitHub #448 (part of #356) commits the four phone web app README captures and their manifest
entries, adds two fixture-only pages (`/readme/tour-seen` sets the app's own tour flag;
`/readme/capture` serves the assets' `index.html` with `<base href="/">` and an inline Date-freezing
script inserted before `app.js`), adds a fixed synthetic capture clock (`FIXED_CAPTURE_UNIX_MS` =
2026-10-01T09:41:00Z, `FixtureClock`, `serve_with_clock`, example flag `--clock <real|fixed>`), runs
the capture script with `--clock fixed` and `TZ=UTC`, documents it (README, `Docs/README_ASSETS.md`
§4, walkthrough 241) and adds matrix rows RAC20/RAC21 (Verified) while RAC18 stays Pending.

The previous review's finding (light/dark locked-alerts showing different times and the real
capture date) is resolved: all four images now show the fixed instant. `crates/` is unchanged
(`git diff origin/main...HEAD -- crates/` is empty), no CI or packaging file references the capture
tooling, and no existing test assertion was changed or removed. No CRITICAL, MAJOR or MINOR finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test/fixture files touched: `tests/fixtures/mod.rs` (fixture library; additions plus the private
  `handle` gaining a `clock` parameter), `tests/fixtures/examples/readme_web_fixture_server.rs`
  (example binary, `serve` → `serve_with_clock`), `tests/fixtures/tests/readme_fixed_clock_tests.rs`
  (new, RAC21), `tests/fixtures/tests/readme_tour_seen_tests.rs` (new, RAC20).
- Removed/changed assertions (`^-` lines with `assert`, `#[test]`, `proptest!`, `should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.

`serve` keeps its signature and real-clock behaviour: it now delegates to
`serve_with_clock(..., FixtureClock::Real)`, and `FixtureClock::Real.now_unix_ms()` calls the same
`now_unix_ms()` used before. The pre-existing fixture tests (`readme_fixture_tests`, which call
`serve`/`route`) are untouched and pass.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Fixed instant: `date -u -d @1790847660` → 2026-10-01 09:41:00 UTC; the Rust constant and the
  literal in `CAPTURE_CLOCK_SCRIPT` agree, and the test pins both to a separately written constant
  and checks the script text contains it. PASS.
- Date override: `FixedDate extends Date` with argument-less constructor and `now()` returning the
  fixed value; `new Date(value)`, `Date.parse`, `Date.UTC` are inherited unchanged. Every `Date`
  use in `crates/remote/assets/app.js` was checked (`Date.now()` for idle minutes, staleness,
  deadlines; `new Date(unixMs)` for formatting): none calls `Date()` as a plain function, which a
  class would reject. Timers computed with the frozen clock (`Math.max(0, deadline - Date.now())`)
  stay bounded. PASS.
- `<base href="/">` on `/readme/capture`: `app.js`, `style.css`, the manifest, icons and `sw.js`
  resolve to `/`, as on `/`. The service worker has no `fetch` listener (only `push`,
  `pushsubscriptionchange`, `notificationclick`), so the first run's registration cannot serve a
  cached unfrozen `index.html` to the capture run. PASS.
- `capture_page` returns `None` on a missing or duplicated marker and the server then answers 404
  rather than an unfrozen page, so a future `index.html` change fails loudly in the capture and in
  `test_rac_capture_page_freezes_the_page_clock_before_app_js` (which also proves that only the two
  insertions differ from the real file on disk). PASS.
- Route table: both pages are exact-path and `GET`-only; query strings, trailing slash and `.html`
  variants give `NotFound`, tested for every scene. PASS.
- `--serve-ms 240000` is within the example's clamp (1 000–300 000); two browser runs per theme at a
  4 s virtual-time budget fit easily. PASS.
- Images agree with the fixture: last alert = fixed − 180 s = 09:38 (tested), offline interval
  Oct 1 00:41–08:41, 8 h. PASS.

### PAM Concurrency & Deadlines

No change in `crates/pam` or any production crate. The fixture server keeps its per-stream I/O
timeouts and overall deadline; the new handler branches add no unbounded wait. N/A / PASS.

### Panic Safety & Fail-Closed

No production code changed. Fixture code adds no `unwrap`/`expect`; I/O errors of the capture page
become 404. No path towards `Allow`/`PAM_SUCCESS` is touched. PASS.

### Test Integrity & Anti-Weakening

- Section 2 listing: additions only. PASS.
- The new tests can fail against plausible wrong implementations: a wrong instant, a script
  returning the real time, a clock script placed after `app.js`, a page rewriting more than the two
  insertions, a server ignoring the clock (the `checked_unix_ms` check over loopback), a script not
  passing `--clock fixed` or screenshotting `/` instead of `/readme/capture`. PASS.
- RAC18 stays `⬜ Pending` (owner visual check); RAC20 and RAC21 are `✅ Verified` and cite existing
  test names (the `matrix_citations` invariant passes). PASS.
- Executed: `cargo test --locked -q -p soos-test-fixtures` (all green, incl. 3 RAC21 and 2 RAC20
  tests) and `cargo test --locked -q -p soos-invariants` (792 passed, incl. RAC15
  `test_rac_capture_tooling_is_dev_only` and the README asset contract).

### Memory, Bounds & Secrets

- Image inspection (each of the four PNGs opened and viewed):
  - `web-unlocked-light.png` / `web-unlocked-dark.png`: "Unlocked", "Active session", "Updated 0 s
    ago", Lock now enabled, "No failed password attempts", Offline history "Oct 1, 12:41 AM –
    08:41 AM · 8 h · Sleep".
  - `web-locked-alerts-light.png` / `web-locked-alerts-dark.png`: "Locked", "Active session, idle
    for 12 min", "Updated 0 s ago", "2 failed password attempts, last at 09:38 AM (sudo)", entry
    "09:38 AM sudo, your account, 2 wrong passwords", Acknowledge. Light and dark show the
    **identical** date and times.
  - No real photo or face, no login or user name ("your account" only), no host name, IP address,
    e-mail, passkey id or push endpoint. No first-run tour overlay in any image. Nothing shows the
    real capture date (2026-10-18); every time derives from 2026-10-01 09:41 UTC.
- PNG chunks: each file contains only `IHDR`, `IDAT`, `IEND`, no trailing bytes, 780×1688. The
  SHA-256 of each file matches its `Docs/assets/MANIFEST.toml` entry (`source = "synthetic"`).
- `CAPTURE_CLOCK_SCRIPT` and `TOUR_SEEN_PAGE` contain no URL, `@`, identity or demo host, and the
  clock script touches no storage (tested). PASS.

### Supply Chain & Automation

No `Cargo.toml`/`Cargo.lock`/`deny.toml`/`.github/` change. A search for `capture_readme_web`,
`readme_web_fixture_server` and `readme-capture` in `.github/` and `packaging/` finds nothing (RAC15
also passes). The script still installs and downloads nothing and stays off CI; `TZ=UTC` is scoped
to the browser invocations. PASS.

### English-Only Policy

All code, comments, docs, walkthrough and commit subjects on the branch are English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `scripts/capture_readme_web.sh:98` — `app.js` formats times with
  `toLocaleTimeString([], …)`, so the browser's UI locale (inherited from `LANG` on the capture
  host) decides the 12 h / 24 h format and month names. Byte-identical reruns hold only on the same
  host locale; consider pinning `LANG=en_US.UTF-8` / `--lang=en-US` next to `TZ=UTC`, or naming the
  locale in the `Docs/README_ASSETS.md` §4 reproducibility caveat. Optional.
- **[SUGGESTION]** `tests/fixtures/mod.rs` `CAPTURE_CLOCK_SCRIPT` — a plain `Date()` call (without
  `new`) would throw under the class-based override. `app.js` has no such call today; a function
  wrapper would be more robust if the app ever adds one. Optional.

## 5. Final Verdict

No CRITICAL, MAJOR or MINOR findings; the two suggestions are optional.

**VERDICT: APPROVED**
