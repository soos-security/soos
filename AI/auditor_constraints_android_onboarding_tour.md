# Auditor Constraints — GitHub #427: first-run feature tour in the native Android app

- **Date**: 2026-10-18
- **Inputs audited**: `AI/architect_spec_android_onboarding_tour.md`, `AI/plan_evaluator_report.md`, the new
  contract `tests/invariants/src/android_tour_contract.rs` (red: 7 of 8 tests fail on missing sources, resources,
  needles and documents; the helper self-test passes), the new JVM test
  `android/app/src/test/java/org/soos/remote/TourControllerTest.kt` (does not compile without `tour/Tour.kt`; CI
  only), and the code about to change: `ui/SoosApp.kt`, `data/SettingsStore.kt`, `res/values/strings.xml`.
- **State before the change**: the app stores the PC address, the last passkey credential id and the heartbeat
  switch, count and two flags in `soos_settings`; one runtime prompt (heartbeat); three permissions; no animation
  API in the app.

## Audit Constraints — Issue #427

| # | Constraint | Applies to (file::fn) | Verified by (test / lint / invariant / grep) |
|---|---|---|---|
| 1 | Only one new stored datum: the Boolean `tour_seen` (default `false`) in `soos_settings`, through `SettingsStore`; no other key, file, DataStore, database, `Bundle` or saved state for the tour; the tour sources never touch `SharedPreferences`. | `SettingsStore.kt`, `tour/Tour.kt`, `ui/TourCard.kt` | ATR-S4, AHB-S3, ANA-S8 |
| 2 | Writing happens only on close (Skip tour, system back, Done); opening, offering, Next and Back never write; a replay never clears the flag. | `TourController` | `TourControllerTest`, ATR-S5 |
| 3 | No request and no switch: the tour code holds no `SoosApi`/`SharedApi`/transport/coroutine/heartbeat/settings reference; `TourCard` receives only the state and three callbacks. | `tour/Tour.kt`, `ui/TourCard.kt`, `SoosApp.kt::HomeScreen` | ATR-S5 |
| 4 | Fixed text only: every tour text is a `strings.xml` resource (no literal in `TourCard.kt`), plain ASCII without markup or format placeholders except the counter `Step %1$d of %2$d`, which takes two integers from the model; no server value, host, PC address or credential is shown. | `strings.xml`, `TourCard.kt` | ATR-S2, ATR-S3, ATR-S5 |
| 5 | Accuracy of the off paths: each shared where line names exactly the keys of the web line (dependants included) and `[android]` where the web line does, so a manual edit never makes `soos-remote` exit 78; the app's own heartbeat is turned off with its switch in Settings; features the app does not show (battery, unlock, push, camera) point to the web app. | `strings.xml` | ATR-S2; review against `crates/remote/src/config.rs` through the web contract PTR-S5 |
| 6 | Same wording: titles, explanations and "Off:" lines of the eight shared steps are byte-equal to `TOUR_STEPS` of `app.js`. | `strings.xml` | ATR-S1 |
| 7 | Never over the settings screen or the sign-in state: the tour state is not passed to `SettingsScreen`; the card renders only when `auth?.authenticated == true`; the first-run offer waits for a status view. | `SoosApp.kt` | ATR-S6 |
| 8 | No panic path: no `!!`, no unchecked index (`TourState.step` clamps the index), no `error()`/`require`/`check` in the tour sources. | `tour/Tour.kt`, `ui/TourCard.kt` | grep (0 hits), `TourControllerTest.anOutOfRangeIndexStillNamesAStep` |
| 9 | No logging (ANA11), no colour literal (ANA10), theme roles only, no animation API (nothing to reduce). | tour sources | ATR-S6, ANA-S7, ANA-S8 |
| 10 | Manifest, permissions (exactly INTERNET, POST_NOTIFICATIONS, RECEIVE_BOOT_COMPLETED), one exported component, one guarded job service, one runtime prompt, the dependency catalog and the verification metadata unchanged. | `AndroidManifest.xml`, `libs.versions.toml`, `verification-metadata.xml`, `build.gradle.kts` | ATR-S6, ANA-S4, ANA-S6, ANA-S13, AHB-S1, AHB-S3; `git diff --stat origin/main -- android/gradle android/app/src/main/AndroidManifest.xml` empty |
| 11 | Tests immutable: no existing test file is edited; only `android_tour_contract.rs`, its registration in `lib.rs` and `TourControllerTest.kt` are added. | `tests/`, `android/app/src/test` | `git diff --stat origin/main -- tests android/app/src/test` |

### Clearance: CLEARED
