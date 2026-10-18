# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/android-onboarding-tour`
- **Base (merge-base)**: `54adaed` (= `origin/main` after `git fetch origin`, includes #426 and #422; previous base `74782b8`)
- **Reviewed-Diff-Fingerprint**: `07228264ec3bbec0b3e14a75acbcd57f91f1f41c868680259df1e3b180c3121b`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_android_onboarding_tour.md`, `AI/auditor_constraints_android_onboarding_tour.md`, `AI/walkthroughs/230_android_onboarding_tour.md`, `Docs/REMOTE_COMPANION.md`, `android/app/src/main/java/org/soos/remote/data/SettingsStore.kt`, `android/app/src/main/java/org/soos/remote/tour/Tour.kt`, `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt`, `android/app/src/main/java/org/soos/remote/ui/TourCard.kt`, `android/app/src/main/res/values/strings.xml`, `android/app/src/test/java/org/soos/remote/TourControllerTest.kt`, `tests/invariants/src/android_tour_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

Re-review after the rebase of the branch from `74782b8` onto `origin/main` `54adaed`, which
gained #426 (`crates/gui/tests/store_task_tests.rs` wait helper, matrix DGS1-DGS2, walkthrough
226) and #422 (Android alerts acknowledgement feedback: `alerts/AlertsFeedback.kt`,
`SoosApp.kt` refresh/acknowledge rewrite, invariant `android_alerts_feedback_contract`,
walkthrough 227, `Docs/REMOTE_COMPANION.md` owner check).

Rebase integrity: `git range-diff 74782b8..66b36bc 54adaed..bd16875` shows only context-line
changes (the hunk anchors moved past the DGS rows, the `android_alerts_feedback_contract` module
and the #422 owner-check step). `git diff --numstat` of the feature commit is identical before
and after (15 files), and the SHA-256 of the added/removed lines of every file is identical
before and after (15/15 "same"). The branch's own changes are therefore unchanged.

Semantic interaction: the only file both #422 and this branch touch is `SoosApp.kt`. The merged
`HomeScreen` keeps the #422 `refresh(carried)` / `acknowledge` / `AlertsFeedback.afterRefresh`
logic byte-for-byte; the tour only adds parameters, the `homeReady`/`tourShown` guards, one
`BackHandler`, the shared `scroll` state and two composables at the top and bottom of the column.
It reads no alerts state and calls no API. #426 touches only `crates/gui/tests` and has no
overlap. `cargo test --locked -p soos-invariants -- --test-threads=3` on the rebased tree:
782 passed, 0 failed, including `android_alerts_feedback_contract`, `remote_tour_contract`
and the 8 `android_tour_contract` tests.

The original content review (below) still holds for the unchanged branch diff: a first-run
feature tour for the native Android app: a pure-Kotlin step model and
controller (`tour/Tour.kt`), a Compose card (`ui/TourCard.kt`), 50 fixed `strings.xml`
resources, one Boolean `tour_seen` in the existing private `SettingsStore`, home-screen wiring
in `SoosApp.kt`, the JVM tests, and a static Rust contract (`android_tour_contract`, 8 tests).
No server, PAM, daemon, manifest or dependency change. No CRITICAL or MAJOR finding; two MINOR
and one SUGGESTION.

## 2. Test Changes

Mechanical listing of the frozen patch:

- Test files touched: `android/app/src/test/java/org/soos/remote/TourControllerTest.kt` (new),
  `tests/invariants/src/android_tour_contract.rs` (new), `tests/invariants/src/lib.rs`
  (+10 lines: one doc comment and one `#[cfg(all(test, unix))] mod android_tour_contract;`).
- `grep '^-[^-].*(assert|#[test]|@Test)'`: **no match**. No line was removed from any test.
- `git diff 54adaed --stat -- tests 'crates/*/tests' android/app/src/test`: 3 files, 1184
  insertions, 0 deletions (identical to the pre-rebase count).
- No new `#[ignore]`, `@Ignore`, `should_panic`, tolerance or epsilon.

No pre-existing assertion was removed or changed; #426's `store_task_tests` and #422's
`AlertsFeedbackTest` / `android_alerts_feedback_contract` are untouched by this branch.

Duplicate check: `AI/DECISIONS.md` holds exactly one "Phone Onboarding Tour in the Native
Android App" ADR; the matrix gains exactly one new `## Component` section (173 on main, 174
after) with ATR1-ATR9 each present once; no matrix row id is duplicated; walkthrough numbers
226, 227 and 230 are distinct.

## 3. Deep Reasoning Audit

### Logic & Architecture

- `TourController.offer`: no-op unless `statusShown`, not yet offered and not open; else opens
  iff `!store.tourSeen`. Tried: offer while already open (no-op), offer after close (no-op, no
  store read), offer with `seen = true` (marked offered, stays closed). PASS.
- `next` on the last step calls `close` (Done remembers); `back` stops at index 0; `close` on a
  closed tour writes nothing. Index is always 0..LAST by construction; `step`/`number` clamp
  defensively. PASS.
- Wiring: `LaunchedEffect(homeReady, status != null)` offers only once a status view exists and
  `auth.authenticated` is true; `tourShown = open && homeReady`, so a sign-out or the Funnel
  sign-in state hides the card and disables the `BackHandler`. Settings screen never renders it
  (separate branch of `SoosApp`). The replay button is under the same `homeReady` guard, last in
  the column ("at the bottom of the home screen" is accurate); the card is first in the column
  and `scroll.scrollTo(0)` runs when it appears ("at the top" is accurate). PASS.
- Scenario (after #422): a refused acknowledgement triggers `refresh(carried)`; a Funnel
  session expiry sets `auth.authenticated = false` and `status = null`, so `tourShown` turns
  false and the card and `BackHandler` go away; after a new sign-in `status != null` re-runs
  the offer effect, which is a no-op because `offered` is already true. PASS.
- Scenario: tour open, user opens Settings and returns: `tourState` lives in `SoosApp`, so the
  tour resumes at the same step without writing the flag. Acceptable.
- Scenario: rotation / process death: `remember` (not `rememberSaveable`) loses the open state;
  the tour is re-offered from the introduction if never closed. No configChanges in the
  manifest. Acknowledged in walkthrough 230 (P-2). MINOR, see 4.

### PAM Concurrency & Deadlines

Not applicable: no change in `crates/pam` or any blocking PAM path. PASS.

### Panic Safety & Fail-Closed

- Kotlin: `TourStep.entries[index.coerceIn(0, LAST)]` cannot go out of bounds; the `when (step)`
  map is exhaustive (compile-time). `stringResource(R.string.tour_step_counter, number, TOTAL)`
  matches `%1$d`/`%2$d`. No `!!`. PASS.
- No authorization path is touched; the tour calls no action, so it cannot produce an unlock or
  lock. PASS.

### Test Integrity & Anti-Weakening

- Additions only (section 2).
- `TourControllerTest` would fail against plausible wrong implementations: writing the flag on
  open, re-offering after close, Done not remembering, Back past the introduction, replay not
  resetting the index, a closed tour reacting to Next. Read/write counters on the fake store
  pin "only close writes". I checked that it compiles against the code as written (Kotlin
  2.2.21, `entries`, `var` destructuring, `assertSame` on the unchanged state instances).
  It runs only in CI (no JDK on the host; `java`/`gradle` not found) — noted, not a finding.
- `android_tour_contract` re-run on the rebased tree: 8/8 pass (782/782 invariants), including byte-equality of the eight shared
  steps with `TOUR_STEPS` of `crates/remote/assets/app.js` after decoding Android escapes.
  PASS.

### Memory, Bounds & Secrets

- Storage: one Boolean `tour_seen` in the existing `MODE_PRIVATE` `soos_settings` file. No
  credential, session cookie, server data or PC address is read by the tour code. Nothing is
  logged. No new I/O, no network. PASS.

### Supply Chain & Automation

- No `Cargo.*`, Gradle catalog, `verification-metadata.xml`, manifest, `.github/` or script
  change. `androidx.activity.compose.BackHandler` comes from the already-declared
  `activity-compose` 1.11.0. PASS.

### English-Only Policy

- Patch grepped for non-ASCII French accents: none. Commit subject
  `feat(android): add the first-run feature tour to the native app` is English, one scope,
  under 72 characters. PASS.

### Factual Accuracy of Doc and String Claims

Verified against the code:

- "every 15 minutes", "one alert after two failed checks in a row": `Heartbeat.kt`
  (15 min period, `FAILURES_BEFORE_ALERT = 2`), and the same sentence in the Settings card. TRUE.
- "Settings, then the PC offline alerts switch": `HeartbeatCard` is in `SettingsScreen`, title
  "PC offline alerts". "only then asks to show notifications": the permission prompt runs only
  when the switch is turned on. TRUE.
- "Password alerts card of the home screen": `alerts_title` = "Password alerts". TRUE.
- "Lock PC on the home screen": the `Lock PC` button exists. TRUE.
- "remove its [android] table": `config.rs` refuses `[android]` without `allow_funnel`
  (exit 78). "Without an [android] table, Features in soos-gui does the same": GUI
  `RemoteSwitch::Funnel` clears `allow_funnel` and `camera_view_funnel`. TRUE.
- Push and camera where lines: `PUSH_DISABLE_LABEL = "Disable notifications"`,
  `CAMERA_STOP_LABEL = "Stop camera view"` in `app.js`; GUI `Push` clears
  `push_notifications` and `offline_alerts`. TRUE.
- `battery_status` key exists (`config.rs`). The app shows no battery, unlock, push or camera
  view (checked `HomeScreen`). TRUE.
- Docs counter "Step 1 of 10" to "Step 10 of 10": `TOTAL = 10`. TRUE.

## 4. Detailed Findings & Action Items

- **[MINOR]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:82` — `tourState` uses
  `remember`, so an open tour is lost on rotation or process recreation and re-offered from the
  introduction if never closed. Matches the other screen state and is documented (walkthrough
  230, P-2); a `rememberSaveable` with a `Saver` would keep the step. No action required.
- **[MINOR]** `android/app/src/main/res/values/strings.xml:72` — "In this app, Sign out ends
  this phone's session" while the Sign out button is shown only in Funnel mode and signed in.
  Not wrong (there is no app session to end over the tailnet), but could say "over Funnel".
- **[SUGGESTION]** The byte-equal shared texts say "this page" inside the native app. Accepted
  by design (ADR item (2), P-1); listed only for the owner check ATR9.

## 5. Final Verdict

**VERDICT: APPROVED**
