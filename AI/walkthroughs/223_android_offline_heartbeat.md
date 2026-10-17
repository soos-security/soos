# Walkthrough 223 — Opt-In PC-Offline Heartbeat in the Native Android App

- **Date**: 2026-10-17
- **Issue**: GitHub #415 (GitHub-only, no backlog id), part of epic #359 (Part B tier 3), refs #357 — **Branch**: `feat/android-offline-heartbeat`
- **Owner decision**: comment of 2026-10-17 on #359 (permissions, one platform `JobService`, no WorkManager, `setPersisted(true)`, 15 min, 2 failures, one back-online notice, off by default)
- **Contract migration**: by the lead (owner-approved, commit c8dccd1 on the branch, squashed into this pull request), `tests/invariants/src/android_native_contract.rs::test_ana_s6_manifest_hardening`
- **Matrix criteria**: AHB1–AHB8 (new component `android-offline-heartbeat`); ANA9 and ANA12 amended.

## 1. Context & Objectives

Epic #359 asks for an alert on the owner's phone when the PC goes offline. A PC cannot report
its own disappearance, so tier 3 puts the observer on the phone: the native app checks the PC
in the background and raises a local notification. The owner approved the smallest surface:
two more permissions, one platform job service and no new dependency.

## 2. Architect Design

`AI/architect_spec_android_offline_heartbeat.md`. Pure Kotlin in `org.soos.remote.heartbeat`
(`Heartbeat.kt`: outcomes, notices, state, job spec, `HeartbeatPolicy`, `HeartbeatTexts`;
`HeartbeatController.kt`: the opt-in and the store/scheduler seams) and in `net/SharedApi.kt`
(one `SoosApi` per PC origin for the process). Android adapters: `HeartbeatJobService`,
`PlatformHeartbeatScheduler`, `HeartbeatNotifier`. The settings screen gets a
"PC offline alerts" card.

- **D-1 (session).** The screens and the job share one in-memory client, so a check uses the
  Funnel session while the process lives. The cookie is never stored. Accepted risk: a Funnel
  check refreshes the 15 min idle timer (absolute 8 h unchanged).
- **D-2 (classification).** Any `soos-remote` answer is reachable (`403 login_required` is
  "sign-in needed"); only no answer (network error, timeout, malformed body, a gateway refusal
  without a `result` token) is a failure. A Funnel phone without a session still detects an
  offline PC.
- **D-3 (no network constraint).** Revised during the plan gate: on Android 14 and later a
  JobScheduler network constraint needs `ACCESS_NETWORK_STATE`, which is not approved. Airplane
  mode skips the check, a stopped run is discarded, and a 5 min flex keeps two checks 10 to
  20 min apart. A phone without a network outside airplane mode can raise a false alert
  (accepted risk; the permission is an owner question).

## 3. Plan Evaluation

`AI/plan_evaluator_report.md` (self-check). Round 1: REVISION_REQUIRED on M-1 (the network
constraint would make `enable()` always fail with a `SecurityException` on `targetSdk` 36).
Round 2 on revision 1: APPROVED, minor findings m-1 to m-4 folded in (accepted risk recorded,
a "Notifications are blocked" note when the permission was revoked, the WorkManager text of
#359 superseded, the best-effort limits documented).

## 4. Tester Contract

`AI/tester_contract_android_offline_heartbeat.md`. JVM tests (CI only):
`HeartbeatPolicyTest` (13 tests: thresholds, no duplicates, recovery, the sign-in notice,
classification, clamping), `HeartbeatControllerTest` (15: off by default, opt-in schedules the
persisted job, opt-out cancels, stale job, refused schedule, restart, re-enable, and the
re-schedule on app open after a force stop, a pending job kept, an opted-out heartbeat never
scheduled, a refused re-schedule, a new PC address, a late check of the previous PC),
`HeartbeatTextsTest` (3) and `SharedApiTest` (3). Rust invariants
`tests/invariants/src/android_heartbeat_contract.rs`: AHB-S1 (full allowlist pinned and
exercised through the script's own `check_permissions`), AHB-S2 (manifest service), AHB-S3
(Kotlin static contract), AHB-S4 (ADR, docs, matrix, walkthrough).

Red before the implementation: the four AHB tests and the lead's `test_ana_s6` failed on their
assertions (missing allowlist entries, no `<service`, no `Heartbeat.kt`, ADR not amended); the
24 other Android invariants passed. The only change to an existing test file of this branch is
`run_check_permissions` becoming `pub(crate)` (visibility, no assertion).

## 5. Auditor Constraints

`AI/auditor_constraints_android_offline_heartbeat.md`, C1–C17, CLEARED: the manifest shape,
the exact allowlist, no network constraint or foreground/alarm/backoff API, the status route
as the only call, the cookie in memory only, no logging, fixed texts, the permission checked
before posting, one prompt site, a worker that cannot crash the process, serialized controller
methods, clamped stored values, synchronous state writes, scheduler exceptions mapped to
`false`, no new dependency, and a state reset on a new PC address.

## 6. Implementation

- `AndroidManifest.xml`: `POST_NOTIFICATIONS`, `RECEIVE_BOOT_COMPLETED`, and
  `<service android:name=".heartbeat.HeartbeatJobService" android:exported="false"
  android:permission="android.permission.BIND_JOB_SERVICE" />`.
- `scripts/android_build.sh`: the two names appended to `ALLOWED_PERMISSIONS`.
- `HeartbeatPolicy.next`: a saturating counter; the alert at 2 when not yet notified;
  `REACHABLE` resets everything and gives the back-online notice only after an alert;
  `SIGN_IN_NEEDED` resets the counter, gives the back-online notice after an alert, and
  otherwise "Sign-in needed" at most once per opt-in (no check re-arms it; only turning the
  heartbeat on again or a new PC address does, so it is never repeated after every app run).
- `HeartbeatController`: one process-wide lock; `enable()` resets and schedules (off again when
  refused); `disable()` cancels and resets; `onCheck` cancels a stale job and returns a notice
  only once its state is stored (a failed write cannot repeat notices); `ensureScheduled()`,
  called from `MainActivity.onCreate`, schedules again a job that a force stop dropped
  (`JobScheduler.getPendingJob`), keeps a pending one, and turns the switch off with a note when
  the platform refuses.
- `HeartbeatJobService`: everything runs on a worker thread (storage included); stale job →
  cancel; no valid PC address or airplane mode → nothing; otherwise the worker calls `SharedApi.forOrigin(origin).status()`, applies the result
  unless the run was stopped, posts the notice and always calls `jobFinished(params, false)`.
  `onStopJob` returns `false`.
- `SettingsStore` implements `HeartbeatStore` (off by default, state written with `commit()`,
  clamped on load); a new PC address is saved through `HeartbeatController.savePcUrl()`, which
  resets the state in the same locked step, and `onCheck` drops a late check of the previous
  address. Each job run has its own stop flag.
- `SoosApp`: the home screen uses `SharedApi.forOrigin(pc.origin)`; the settings screen shows
  the card once a valid address is saved and requests `POST_NOTIFICATIONS` only from its
  switch.

## 7. Candid Review

Done by an independent reviewer in a fresh context, which records its findings in
`AI/candid_review_report.md`, bound to the diff fingerprint of this branch. Round 1: APPROVED
with three minor findings and two suggestions, all addressed in spec revision 2 (sign-in notice
once per opt-in, job storage off the main thread, `IllegalStateException` mapped to `false`,
the idle-phone step of AHB8, the walkthrough invariant naming the migrated test). Round 2:
CHANGES_REQUESTED on MAJOR-1 (a force stop dropped the job for good while the switch still
showed "on"), fixed by `ensureScheduled()` (spec revision 3), with MINOR-2 documented and
MINOR-3 (matrix wording) fixed. Round 3: APPROVED with three minor findings,
all fixed in spec revision 4 (a locked `cancelIfDisabled()`, the `login_required` drop limited
to the session that was sent, Battery Saver and Data Saver documented). Round 4: APPROVED
with three minor findings, fixed in spec revision 5 (state reset under the controller lock,
block bodies for early returns, AHB4–AHB6 pending until CI, doc wrapping). Round 5: APPROVED
with two minor races, fixed in spec revision 6 (checks bound to the saved address, a stop flag
per run); round 6 reviews the final diff.

## 8. Verification Results

- `cargo test --locked -p soos-invariants -- --test-threads=3` (private target): all tests
  pass, including AHB-S1 to AHB-S4, the migrated ANA-S6 and the matrix citations of AHB1–AHB7.
- No JDK or Android SDK on the development host: the Kotlin build, the JVM tests, lint and the
  `aapt2` permission check run in the CI job `Android (debug build)` of the pull request.
- `git diff origin/main -- android/gradle android/app/build.gradle.kts android/settings.gradle.kts`
  is empty: no dependency, catalog or verification-metadata change.

## 9. Known Limitations / Follow-ups

- Owner question: `ACCESS_NETWORK_STATE` would allow a network constraint and remove the false
  alert of a phone without any network outside airplane mode.
- Best effort: Doze and App Standby defer checks; a force stop cancels the job until the app is
  opened again; the Tailscale VPN must be on.
- AHB8 is an owner hardware check on a phone.
- The settings switch writes the tiny preferences file synchronously on the main thread
  (`enable`/`disable`); harmless at this size, it could move to a background thread later.
- The open PR #406 (alerts list) touches `SoosApp.kt`, `SoosApi.kt` and the same docs; the
  later of the two needs a rebase.
