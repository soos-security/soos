# Walkthrough 246 — README Web Capture Locale and a Stronger Fixed-Clock Test

- **Date**: 2026-10-20
- **Issue**: GitHub #458, part of epic #356 (GitHub-only, no backlog id), follow-ups of the PR #450 reviews — **Branch**: `chore/readme-capture-locale`
- **Decision**: ADR 2026-10-13 "README Assets Are Generated From Synthetic Data, Never Hand-Edited" (unchanged)
- **Matrix criteria**: RAC22 and RAC23 (new, verified). RAC18 stays pending (owner visual check). RAC1–RAC17,
  RAC19–RAC21 unchanged.

## 1. Context & Objectives

PR #450 committed the four phone web app captures, made reproducible by a fixed synthetic clock and
`TZ=UTC` (walkthrough 241). Its reviews left three follow-ups:

1. The browser locale was not pinned. `crates/remote/assets/app.js` formats times with
   `toLocaleTimeString([], …)` and `toLocaleString([], …)`, i.e. with the browser's default locale, which
   Chrome on Linux takes from the environment. A capture host with another locale would show another clock
   format or month names.
2. The README "Screenshots" intro said that all screenshots come from `scripts/capture_readme_assets.sh`
   with the user `alice`; that is true only for the `soos-gui` captures.
3. `readme_fixed_clock_tests::test_rac_fixed_capture_clock_is_pinned` has a loop comparing `route` at one
   instant with `route` at the same instant: `route` is pure, so the loop cannot fail.

## 2. Design

- `scripts/capture_readme_web.sh` defines once
  `BROWSER_ENV=(TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 LANGUAGE=en_US)`; both browser runs (the
  tour-seen page, then the screenshot of `/readme/capture`) start as
  `env "${BROWSER_ENV[@]}" "$browser" "${flags[@]}" …`, and the shared flags gain `--lang=en-US` and
  `--accept-lang=en-US`. `TZ=UTC` is kept inside `BROWSER_ENV`. The fixture server and `cargo` runs are
  unaffected (their output does not depend on the locale).
- `README.md`: the intro names both scripts — the `soos-gui` captures (`capture_readme_assets.sh`, drawn
  face, user `alice`, mock models) and the phone web app captures (`capture_readme_web.sh`, canned,
  identity-free data with no user name); the web caption mentions the pinned `en-US` locale.
- `Docs/README_ASSETS.md` §4 describes the pinned environment and flags.
- No production crate changes. `soos-remote` and the asset files are untouched.

## 3. Tests (new file `tests/fixtures/tests/readme_capture_locale_tests.rs`)

- `test_rac_web_capture_pins_the_browser_locale` (RAC22): reads the script without comments, with `\`
  continuations joined; exactly one `BROWSER_ENV=` definition equal to the pinned one; exactly two browser
  runs, each starting with `env "${BROWSER_ENV[@]}" "$browser" "${flags[@]}" `, one on the tour-seen page
  and one taking `--screenshot=` of the capture page; no line starts with `TZ=`; the `flags=( … )` block
  carries `--lang=en-US` and `--accept-lang=en-US` exactly once, and no other `--lang=` exists. It fails on
  the script of `origin/main` (no `BROWSER_ENV`) and passes after the change.
- `test_rac_fixed_clock_server_bodies_are_pinned_and_instant_dependent` (RAC23): for both scenes,
  `route` at `FIXED_CAPTURE_UNIX_MS + 61 s` gives different status and offline bodies, a different alerts
  body for the locked scene (the unlocked scene has no alert and its body is checked to be constant) and a
  different event stream prelude, which starts with the fixed status and alerts bodies; two `--clock fixed`
  loopback servers (one per scene) then answer every clocked JSON API with exactly the `route` body at the
  fixed instant (`200 OK`, `application/json`, matching `Content-Length`), byte-identical on a second
  request more than one second later; the two scenes' bodies differ. This test passes already on the
  current fixture: it is coverage that the existing loop could not give, not a red-phase contract.
- The existing test is unchanged except for a comment above the weak loop pointing to RAC23 (no assertion
  touched).

## 4. Captures

The committed PNGs were not regenerated: they are unchanged.

- `CHROME=/usr/bin/google-chrome-stable scripts/capture_readme_web.sh` (Google Chrome 155.0.8059.84, host
  `LANG=en_US.UTF-8`) with the pinned script: the four PNGs are byte-identical to the committed ones
  (`git status` clean under `Docs/`).
- The same run with `LANG=de_DE.UTF-8 LC_ALL=de_DE.UTF-8 LANGUAGE=de TZ=Asia/Tokyo` in the caller's
  environment: byte-identical again.
- The script of `origin/main` with the German locale: all four PNGs change (the alert reads "last at 09:38"
  in 24 h format instead of "09:38 AM"); those files were discarded with `git checkout -- Docs`.

`Docs/assets/MANIFEST.toml` therefore needs no refresh. RAC18 stays pending for the owner's visual check.

## 5. Verification

- `cargo test -p soos-test-fixtures --test readme_capture_locale_tests --test readme_fixed_clock_tests
  --test readme_tour_seen_tests --test readme_fixture_tests` green.
- `soos-invariants` README contracts (`readme_assets_contract`, `readme_contract`) green; `cargo fmt --check`
  and `cargo clippy -p soos-test-fixtures --all-targets -- -D warnings` clean.
