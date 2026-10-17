# Plan Evaluation Report
- **Date**: 2026-10-17
- **Issue**: GitHub-only (no backlog id) — opt-in PC-offline heartbeat in the native Android app (GitHub #415, part of epic #359 Part B tier 3, refs #357)
- **Branch**: `feat/android-offline-heartbeat`
- **Base commit**: `6eb0746` (+ the lead's contract migration `c8dccd1`)
- **Plan evaluated**: `AI/architect_spec_android_offline_heartbeat.md` §0–§7 (round 1 REVISION_REQUIRED on M-1; this report is round 2, on revision 1)
- **Mode**: self-check by the implementing agent against `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, the owner decision of 2026-10-17 on #359 and the code; the independent candid review runs separately.

## 1. Coverage Matrix

| Acceptance line (GitHub #415 / owner decision) | Spec element | Status |
|---|---|---|
| Manifest: exactly `INTERNET`, `POST_NOTIFICATIONS`, `RECEIVE_BOOT_COMPLETED` | §1.2 manifest; c8dccd1 `test_ana_s6_manifest_hardening` | Mapped |
| One `<service>`, `BIND_JOB_SERVICE`, `exported="false"`; no `<receiver>`/`<provider>`; one exported component | §1.2; c8dccd1; AHB-S2 | Mapped |
| `ALLOWED_PERMISSIONS` + exactly the two names; new invariant pins the full list | §1.3; §6 AHB-S1 | Mapped |
| Runtime prompt only when the heartbeat is turned on | §2.5; AHB-S3 (single `RequestPermission` site) | Mapped |
| No WorkManager, no new dependency, metadata unchanged | §1.2 (no Gradle change); ANA-S2 exact catalog; AHB-S3 (`androidx.work` absent) | Mapped |
| Reboot through `setPersisted(true)`, no receiver | §2.4; `HeartbeatPolicy.JOB.persisted`; AHB-S3 (`setPersisted(spec.persisted)`, no `BOOT_COMPLETED` string) | Mapped |
| 15 min, 2 failures, one back-online notice, off by default | §2.1 table; §3; JVM `HeartbeatPolicyTest`, `HeartbeatControllerTest` | Mapped |
| Status endpoint only, same transport and bounds | §2.4 (`SharedApi` → `UrlConnectionTransport`); AHB-S3 (only `.status()` in the package) | Mapped |
| Cookie never on disk; missing session → "sign-in needed" once, no aggressive retry | D-1, §2.1 (`SIGN_IN_NEEDED` once per episode), §2.4 (`onStopJob` false, `jobFinished(.., false)`) | Mapped |
| No logging; fixed English texts without PC data | §2.6; ANA-S8 (unchanged); AHB-S3 (no `$` in texts); JVM `HeartbeatTextsTest` | Mapped |
| JVM tests: 2 failures → one alert, recovery → one back-online, no duplicates, opt-out cancels | §6 | Mapped |
| ADR item (8), ANA9/ANA12, new rows, docs, walkthrough 223 | §1.3, §7 | Mapped |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Status route answers `200` JSON for tailnet and Funnel sessions | `crates/remote/src/server.rs` `Route::Status` | `fresh_view()` → `200`, or `503 unavailable` (soos JSON) | Yes |
| Funnel request without a session gets `403 login_required` before routing | `server.rs` (refusal before `enter_anonymous`) | as cited | Yes |
| Funnel session idle / absolute | `crates/remote/src/lib.rs` `WEB_SESSION_IDLE_MS`, `WEB_SESSION_ABSOLUTE_MS` | 900 000 / 28 800 000 | Yes |
| A Funnel status request refreshes the idle timer | `server.rs` `Touch::Refresh` for non-asset routes | as cited (accepted risk D-1) | Yes |
| `SoosApi.status()` maps network errors to `Unreachable`, refusals to `Refused(status, result)` | `android/.../net/SoosApi.kt` | as cited; `result` matches `[a-z_]{1,64}` | Yes |
| `SoosApi` drops the session on `403 login_required` | `SoosApi.send` | as cited | Yes |
| Transport: 5 s / 5 s, 16 KiB, no redirect, HTTPS only | `UrlConnectionTransport.kt` | as cited | Yes |
| JUnit 4 and org.json already test dependencies | `android/app/build.gradle.kts` | `testImplementation(libs.junit)`, `libs.org.json` | Yes |
| `minSdk` 34 (so `POST_NOTIFICATIONS` is always a runtime permission) | `build.gradle.kts` | 34 | Yes |
| `JobInfo` minimum period / flex | platform API | 15 min / 5 min | Yes |
| `setPersisted(true)` needs `RECEIVE_BOOT_COMPLETED` only | platform API (`@RequiresPermission`) | as cited | Yes |
| A network constraint is free | platform API, Android 14 behaviour change for `targetSdk` ≥ 34 | needs `ACCESS_NETWORK_STATE` (else `SecurityException` at `schedule`) | **No** → finding M-1 |
| Airplane mode is readable without a permission | `Settings.Global.AIRPLANE_MODE_ON` | readable by any app | Yes |
| Data extraction excludes shared preferences, backup off | `data_extraction_rules.xml`, manifest | as cited | Yes |
| Matrix ids ANA18–ANA21 are used by the open PR #406 | `gh pr diff 406` | ANA18–ANA21 | Yes → new prefix `AHB` |
| `run_check_permissions` harness extracts `ALLOWED_PERMISSIONS`, `fail`, `check_permissions` verbatim | `tests/invariants/src/android_native_contract.rs` | private fn | Yes (made `pub(crate)`) |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario: another app on the phone starts the job service to make the app send
  requests. Excluded: `exported="false"` and `BIND_JOB_SERVICE` (only the system can bind).
- Failure scenario: the heartbeat adds a network destination. Excluded: only the saved PC
  origin, only `GET /api/status`, through the unchanged transport (AHB-S3).
- Result: PASS. RC-1 untouched (no server change).

### Pillar 2 — Deadlines & concurrency (PAM not involved)
- Failure scenario: the UI turns the heartbeat off while a job is in flight; the job then
  writes a stale count. Mitigated: controller lock and the `enabled` re-check inside
  `onCheck`; any stale value is reset by the next `enable()`.
- Failure scenario: the system stops the job and the failed request is counted. Mitigated:
  D-3 (b), a stopped run is discarded.
- Failure scenario: the phone itself is offline and every check fails. Round 1 relied on a
  `NETWORK_TYPE_ANY` constraint, which needs `ACCESS_NETWORK_STATE` on `targetSdk` 36
  (finding M-1). Revised: airplane mode skips the check (D-3 (a)); the remaining cases are an
  accepted risk, documented, with the permission left as an owner question.
- Failure scenario: two checks seconds apart across period windows. Mitigated: flex 5 min (D-3).
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario: an unexpected `RuntimeException` (for example `SecurityException`) in the
  worker thread crashes the process on every period. Mitigated: §2.4 catches it and discards
  the check. `JobScheduler.schedule` exceptions → `enable()` false.
- Result: PASS. The heartbeat never changes lock, unlock or sign-in outcomes.

### Pillar 4 — Dependencies
- Failure scenario: `rememberLauncherForActivityResult` pulls a new artifact. Excluded: it is
  in `androidx.activity:activity-compose` 1.11.0 (already pinned and verified).
- Result: PASS. No catalog, Gradle or metadata change (ANA-S2, ANA-S4 unchanged).

### Pillar 5 — Data confidentiality
- Failure scenario: the notification shows the PC name on the lock screen. Excluded: fixed
  texts (§2.6, JVM test and AHB-S3).
- Failure scenario: the session cookie is persisted to make background checks work. Excluded:
  `SharedApi` keeps it in memory; ANA-S8 still forbids `SessionCookie` next to
  `SharedPreferences`, and the stored keys hold only a count and two flags.
- Result: FINDING m-1 (MINOR, accepted risk D-1): a Funnel check refreshes the idle timer while
  the process lives. Recorded in the ADR amendment and in `Docs/REMOTE_COMPANION.md` §8.

### Pillar 6 — Test integrity
- Wrong implementations the tests must catch: alert after the first failure (`>` vs `>=`),
  a second alert while offline, no back-online notice, a back-online notice without a prior
  alert, a repeated sign-in notice, opt-out without cancel, a non-persisted or 5-min job,
  an allowlist with an extra or misspelled entry. Each has a dedicated assertion (§6).
- Existing tests: only c8dccd1 (lead, owner-approved) changes an assertion; this plan changes
  no assertion (one visibility change of a helper, listed).
- Result: PASS.

## 4. Findings

- **[MAJOR] M-1 (round 1, resolved in revision 1)** The round-1 spec scheduled the job with
  `setRequiredNetworkType(NETWORK_TYPE_ANY)`. On `targetSdk` 34 and later that throws a
  `SecurityException` without `ACCESS_NETWORK_STATE`, which is not in the owner-approved set:
  `enable()` would always fail. Revision 1 (spec D-3) drops the constraint, skips checks in
  airplane mode, records the residual false-alert case as an accepted risk and the permission
  as an owner question. Re-evaluated: resolved.
- **[MINOR] m-1** Funnel idle-timer refresh by the shared in-memory session (D-1) — record as
  an accepted risk in the ADR and §8 (done in the plan).
- **[MINOR] m-2** A revoked notification permission would leave the heartbeat running
  silently — the settings card shows a fixed "Notifications are blocked" note (folded into
  §2.5).
- **[MINOR] m-3** The #359 issue text names WorkManager — record the supersession in the ADR
  amendment (§7).
- **[MINOR] m-4** Doze, App Standby buckets, Tailscale VPN off and force-stop delay or stop
  checks — document in §2j and §8 (best effort, as the issue says).

## 5. Verdict

VALIDATION_VERDICT: APPROVED
