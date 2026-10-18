# Walkthrough 241 — README Phone Web App Captures

- **Date**: 2026-10-18
- **Issue**: GitHub #448, part of epic #356 (GitHub-only, no backlog id) — **Branch**: `chore/readme-web-captures`
- **Decision**: ADR 2026-10-13 "README Assets Are Generated From Synthetic Data, Never Hand-Edited" (unchanged)
- **Matrix criteria**: RAC20 (new, verified); RAC18 stays pending (captures produced by the agent; owner
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

## 3. Tests

`tests/fixtures/tests/readme_tour_seen_tests.rs` (new file; no existing test edited):

- `test_rac_tour_seen_page_marks_the_app_tour_flag`: exact path and `GET` only in every scene; the key and
  value match the `app.js` constants; the page stores exactly that flag once, guarded, and replaces itself
  with `/`; no identity or external reference; the script opens the page on the loopback server.
- `test_rac_tour_seen_page_is_served_as_html`: the loopback server answers it as `text/html` without an
  asset file.

## 4. Assets

Four PNGs, 780×1688 pixels (390×844 CSS px at scale 2), metadata stripped, listed in
`Docs/assets/MANIFEST.toml` by `scripts/capture_readme_assets.sh` (the GUI PNGs regenerated identical):
`Docs/assets/screenshots/web-unlocked-light.png`, `web-unlocked-dark.png`, `web-locked-alerts-light.png`,
`web-locked-alerts-dark.png`. They are embedded in the README "Screenshots" section with alt text and a
caption saying the data is canned and identity-free.

## 5. Verification

- `cargo test -p soos-test-fixtures`, `cargo test -p soos-invariants` (including `readme_assets_contract`,
  `readme_contract`, `matrix_citations` and RAC15 `test_rac_capture_tooling_is_dev_only`), `cargo fmt`,
  `cargo clippy -D warnings`.
- RAC18 stays ⬜ Pending until the owner looks at the four images.
