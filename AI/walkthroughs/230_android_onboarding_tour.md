# Walkthrough 230 — First-Run Feature Tour in the Native Android App

- **Date**: 2026-10-18
- **Issue**: GitHub #427 (GitHub-only, no backlog id) = PR 3 of epic #363, refs #357 — **Branch**: `feat/android-onboarding-tour`
- **Owner decisions**: those of 2026-10-12 and 2026-10-17 on #363 (a tour on PC and phone); no new decision needed
- **Contract migration**: none (no existing test edited)
- **Matrix criteria**: ATR1–ATR9 (new component `android-onboarding-tour`)

## 1. Context & Objectives

Epic #363 asks for a first-run tour that explains each feature, what it exposes and where to
turn it off. The PC tour (#379) and the web app tour (#414, walkthrough 222) are merged; this
PR brings the same tour to the native Kotlin/Compose app, whose ADR item (5) of 2026-10-17 left
it as a follow-up.

## 2. Architect Design

`AI/architect_spec_android_onboarding_tour.md`.

- `tour/Tour.kt` (pure Kotlin, no `android.` import): `TourStep` (introduction, eight steps
  shared with the web tour, the app's own PC offline alerts), `TourStore` (one Boolean),
  immutable `TourState` (open, index, offered; the index is clamped when read) and a stateless
  `TourController` with `offer`, `replay`, `next`, `back` and `close`. Only `close` writes the
  store, so Skip tour, the system back and Done (through `next`) remember the tour; opening and
  moving never write.
- `ui/TourCard.kt`: one exhaustive `when` from a step to its four `strings.xml` resources and
  the card (counter, heading title, explanation, "Off:" line, where line, Back, Next/Done, Skip
  tour), theme roles only, no animation.
- `ui/SoosApp.kt`: `SoosApp` holds the state, so it survives a trip to the settings screen,
  which never receives it. `HomeScreen` offers the tour when signed in (`auth?.authenticated`)
  and a status is shown, renders it only when signed in (never over the passkey sign-in),
  scrolls to it, closes it on the system back (`BackHandler`, `activity-compose` already
  resolved) and shows **Feature tour** at the bottom under the same guard.
- `data/SettingsStore.kt` implements `TourStore` with the key `tour_seen` in the existing
  private `soos_settings` file, already excluded from backup and device transfer.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md` (self-check): APPROVED with P-1 (the byte-equal web texts say
"this page", accepted, the app-specific where lines say where things are), P-2 (`remember`
loses an open tour on rotation, as every other screen state), P-3 (no JDK on the host: Kotlin
and JVM tests verified by the CI job only) and P-4 (resource drift, covered by ATR-S3).

## 4. Tester Contract

- JVM `android/app/src/test/java/org/soos/remote/TourControllerTest.kt`: twelve tests over the
  step order and shared flags, defaults, the full transition table and the store writes (CI only).
- Invariant `tests/invariants/src/android_tour_contract.rs`: a helper self-test and ATR-S1 to
  ATR-S7. Red run before the implementation: 7 of 8 failed (missing `Tour.kt`, `TourCard.kt`,
  `tour_*` resources, store needles, ADR and documents); the self-test passed.

## 5. Auditor Constraints

`AI/auditor_constraints_android_onboarding_tour.md`, CLEARED with 11 constraints: one Boolean
stored, written on close only, no request or switch, fixed texts, exact off keys, byte-equal
wording, never over settings or sign-in, no panic path, no logging, colour literal or
animation, manifest and dependencies unchanged, tests immutable.

## 6. Implementation

- `strings.xml`: 45 `tour_*` resources. The shared titles, texts and off lines were copied from
  `TOUR_STEPS` of `app.js`, not retyped. The where lines are new: the app has Lock PC, Sign out,
  the Password alerts card and the PC offline alerts switch, but no battery, unlock, push or
  camera card, so those steps point to the web app on the phone; every one names the same
  `remote.toml` keys as the web line (for example `password_alerts = false`,
  `push_notifications = false` and `offline_alerts = false` together), so `soos-remote` never
  exits 78 on the edited file. The one apostrophe is written `\'`.
- The PC offline alerts step repeats the heartbeat card (15 minutes, two failed checks in a
  row, off until turned on) and is turned off with that switch in Settings.
- `SettingsStore`: `override var tourSeen` with `getBoolean(KEY_TOUR_SEEN, false)` and
  `putBoolean(...).apply()`; the heartbeat keys and the no-cookie rule are untouched.

## 7. Candid Review

To be run by an independent reviewer in a fresh context before the push (it writes
`AI/candid_review_report.md`, bound to the diff fingerprint); not part of this local commit.

## 8. Verification Results

- `cargo test -p soos-invariants -- --test-threads=3` with the scoped environment of the
  project rules: the new `android_tour_contract` (8 tests) and every existing Android, remote
  tour and matrix citation contract pass.
- `cargo fmt --all -- --check` and `cargo clippy -p soos-invariants --all-targets -- -D warnings`
  clean.
- The Kotlin build, the JVM tests and lint run in the CI job `Android (debug build)` (ATR1
  pending until its green run id is recorded); the owner check on a phone in light and dark mode
  is ATR9, pending.

## 9. Known Limitations / Follow-ups

- When the app gains unlock, push, the camera view or the battery level, that PR updates the
  matching where lines and `android_tour_contract` (`WEB_APP_STEPS`).
- English only, like the rest of the app.
