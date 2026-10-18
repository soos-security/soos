# Walkthrough 241 — README Phone Web App Captures

- **Date**: 2026-10-18
- **Issue**: GitHub #448, part of epic #356 (GitHub-only, no backlog id) — **Branch**: `chore/readme-web-captures`
- **Decision**: ADR 2026-10-13 "README Assets Are Generated From Synthetic Data, Never Hand-Edited" (unchanged)
- **Matrix criteria**: RAC20 and RAC21 (new, verified); RAC18 stays pending (captures produced by the agent; owner
  visual check pending). RAC1–RAC17 and RAC19 unchanged.

## 1. Context & Objectives

PR 1 of #356 (GitHub #381) shipped `scripts/capture_readme_web.sh` and the loopback fixture server, but the
phone web app captures were never committed. The owner authorised the agent to produce them from synthetic
data with the locally installed Google Chrome and to embed them in the README, keeping the owner's visual
check open.

A first run showed that the first-run feature tour of the web app (ADR 2026-10-17, GitHub #414/#418) opens
on the script's throw-away browser profile and covers the top of every capture.

## 2. Design

- `soos_test_fixtures::readme_web` gains, as an addition, `TOUR_SEEN_PATH` (`/readme/tour-seen`),
  `TOUR_STORAGE_KEY` / `TOUR_STORAGE_SEEN` (mirroring `app.js`), the fixed page `TOUR_SEEN_PAGE` and the
  `FixtureResponse::Page` variant. The page sets the app's own tour flag inside `try` and calls
  `window.location.replace("/")`. Nothing else changes in the route table.
- `scripts/capture_readme_web.sh` opens `http://127.0.0.1:$port/readme/tour-seen` instead of `/` and runs the
  browser with `TZ=UTC`, so clock times shown by the page do not reveal the capture host's time zone.
- `soos-remote` is untouched: no flag, route or mode, and its single storage exception stays the only
  `localStorage` use in `crates/remote`.

### 2.1 Fixed synthetic clock (review follow-up)

The first candid review noted that the light and dark locked-alerts captures showed different alert times
(05:29 vs 05:30 PM, two runs across a minute boundary) and that the captures carried the real capture date.
Also as additions only:

- `FIXED_CAPTURE_UNIX_MS` = 1 790 847 660 000 (2026-10-01T09:41:00Z) and `FixtureClock { Real, Fixed }`.
  `serve_with_clock` takes the clock; `serve` delegates with `FixtureClock::Real`, so its behaviour and
  tests are unchanged. The example gains `--clock <real|fixed>` (default `real`).
- `CAPTURE_PATH` (`/readme/capture`) answers `FixtureResponse::CapturePage`: the assets' `index.html`
  passed through `capture_page`, which inserts `<base href="/">` after `<head>` and an inline clock script
  (`CAPTURE_CLOCK_SCRIPT`, a `Date` subclass whose `now()` and argument-less constructor return the fixed
  instant) before the `app.js` element. The files on disk are not modified; the fixture server sends no
  CSP, unlike `soos-remote`.
- The script runs the server with `--clock fixed --serve-ms 240000` and, per theme, two browser runs in
  the same throw-away profile: `/readme/tour-seen` (tour flag), then the screenshot of `/readme/capture`.
  Two consecutive runs gave byte-identical PNGs on the capture host.

## 3. Tests

`tests/fixtures/tests/readme_tour_seen_tests.rs` (new file; no existing test edited):

- `test_rac_tour_seen_page_marks_the_app_tour_flag`: exact path and `GET` only in every scene; the key and
  value match the `app.js` constants; the page stores exactly that flag once, guarded, and replaces itself
  with `/`; no identity or external reference; the script opens the page on the loopback server.
- `test_rac_tour_seen_page_is_served_as_html`: the loopback server answers it as `text/html` without an
  asset file.

`tests/fixtures/tests/readme_fixed_clock_tests.rs` (new file, RAC21):

- `test_rac_fixed_capture_clock_is_pinned`: the instant is pinned to 2026-10-01T09:41:00Z, the clock
  script freezes exactly that value, the fixed clock gives identical bodies, and the locked-alerts last
  alert sits 3 min before it.
- `test_rac_capture_page_freezes_the_page_clock_before_app_js`: exact path and `GET` only; the real
  `index.html` gains only the base element and the clock script, before `app.js`; missing or duplicated
  markers give `None`; the script uses `--clock fixed` and `/readme/capture`.
- `test_rac_fixed_clock_server_serves_the_capture_page`: the fixed-clock server answers the fixed instant
  and the capture page, 404 when `index.html` lacks a marker.

## 4. Assets

Four PNGs, 780×1688 pixels (390×844 CSS px at scale 2), metadata stripped, listed in
`Docs/assets/MANIFEST.toml` by `scripts/capture_readme_assets.sh` (the GUI PNGs regenerated identical):
`Docs/assets/screenshots/web-unlocked-light.png`, `web-unlocked-dark.png`, `web-locked-alerts-light.png`,
`web-locked-alerts-dark.png`, all at the fixed instant (both locked-alerts captures: idle for 12 min,
alerts at 09:38 AM; offline history dated Oct 1). They are embedded in the README "Screenshots" section with alt text and a
caption saying the data is canned and identity-free.

## 5. Verification

- `cargo test -p soos-test-fixtures`, `cargo test -p soos-invariants` (including `readme_assets_contract`,
  `readme_contract`, `matrix_citations` and RAC15 `test_rac_capture_tooling_is_dev_only`), `cargo fmt`,
  `cargo clippy -D warnings`.
- RAC18 stays ⬜ Pending until the owner looks at the four images.
