# Architect Spec — GitHub #415: opt-in PC-offline heartbeat in the native Android app

Part of #359 (Part B tier 3), refs #357. Branch `feat/android-offline-heartbeat`, base
`origin/main` 6eb0746 plus the lead's contract migration c8dccd1
(`tests/invariants/src/android_native_contract.rs::test_ana_s6_manifest_hardening`).

## 0. Owner decision applied (2026-10-17, comment on #359)

| Decision | Spec element |
|---|---|
| Permissions: exactly `POST_NOTIFICATIONS` (runtime prompt, asked only when the heartbeat is turned on) and `RECEIVE_BOOT_COMPLETED`, besides `INTERNET` | §1.2 manifest, §2.5 UI, §3 allowlist |
| One platform `JobService`, not exported, guarded by `android.permission.BIND_JOB_SERVICE`; no WorkManager, no new dependency, `verification-metadata.xml` unchanged | §1.2, §2.4 |
| Reboot survival through `JobInfo.Builder.setPersisted(true)`; no `<receiver>` | §2.4 |
| Defaults: check every 15 min, alert after 2 consecutive failures, one "PC back online" notification on recovery; off by default, turned on in the app settings | §2.1, §2.3, §2.5 |
| The check reuses the authenticated `GET /api/status` over the same HTTPS transport and bounds; the session cookie is never stored on disk; a check that needs a missing session reports "sign-in needed" once and does not retry aggressively; no logging of URLs or responses; fixed English notification texts without PC name, address or user data | §2.1, §2.2, §2.4, §2.6 |

Decision D-1 (architect, recorded in the ADR amendment): the UI and the heartbeat share one
process-wide `SoosApi` per PC origin (`SharedApi`), so a check made while the app process is
alive uses the in-memory Funnel session when one exists. This is the reading of the owner's
"a session that no longer exists". Consequence (accepted risk, same class as the service-worker
request of ADR 2026-10-09 item (2)): while the process lives, a check over Funnel refreshes the
session idle timer (`WEB_SESSION_IDLE_MS` 900 000); the absolute lifetime
(`WEB_SESSION_ABSOLUTE_MS` 28 800 000) is unchanged and the cookie is still memory only.
A side effect: the session also survives leaving the home screen for the settings screen
inside one process (before, a new `SoosApi` was built per composition).

Decision D-2: a `soos-remote` JSON refusal proves that the PC and `soos-remote` answered.
`403 login_required` is `SIGN_IN_NEEDED` and every other refusal carrying a `result` token is
`REACHABLE`; only a network error, a timeout, a malformed or oversized body, or a non-`soos`
answer (no `result` token, for example a `tailscale serve` gateway `502`) is `UNREACHABLE`.
So a Funnel phone without a session still detects an offline PC, and a sign-in notice never
masks an outage.

Decision D-3 (revised during the plan gate): the job has **no network constraint**. Apps
that target Android 14 or later must hold `ACCESS_NETWORK_STATE` to call
`JobInfo.Builder.setRequiredNetworkType`/`setRequiredNetwork` (otherwise `schedule` throws a
`SecurityException`), and that permission is outside the owner-approved set. Instead:
(a) a check is skipped, with no request and nothing counted, while airplane mode is on
(`Settings.Global.AIRPLANE_MODE_ON`, readable without a permission); (b) a check that the
system stops (`onStopJob`) is discarded, never counted; (c) the period has a flex of 5 min
(`JobInfo.getMinFlexMillis()`), so two checks are 10 to 20 min apart (without flex they could
run seconds apart across two windows, and two quick failures from one network blip would
alert). Accepted risk: a phone without any network outside airplane mode (Wi-Fi and mobile
data both off, no coverage) or with the Tailscale VPN off counts failures and can raise a
"PC went offline" notice; the notice text says only that soos could not reach the PC. Adding
`ACCESS_NETWORK_STATE` for a real network constraint is an owner decision (recorded as an
open question in the ADR amendment).

## 1. Scope & Blast Radius

### 1.1 Rust

No production Rust change. `tests/invariants`: new module `android_heartbeat_contract.rs`
(registered in `lib.rs`), and `run_check_permissions` of `android_native_contract.rs` becomes
`pub(crate)` so the new allowlist test reuses the verbatim-extraction harness (visibility only,
no assertion touched).

### 1.2 Android (`android/app/src/main`)

| File | Change |
|---|---|
| `AndroidManifest.xml` | + `POST_NOTIFICATIONS`, `RECEIVE_BOOT_COMPLETED`; + one `<service android:name=".heartbeat.HeartbeatJobService" android:exported="false" android:permission="android.permission.BIND_JOB_SERVICE" />`; header comment updated (must not contain `<service`, `<receiver` or `<uses-permission` text) |
| `heartbeat/Heartbeat.kt` (new, pure) | `CheckOutcome`, `Notice`, `HeartbeatState`, `HeartbeatStep`, `HeartbeatJobSpec`, `HeartbeatPolicy`, `HeartbeatTexts`, `NoticeText` |
| `heartbeat/HeartbeatController.kt` (new, pure) | `HeartbeatScheduler`, `HeartbeatStore`, `HeartbeatController` |
| `heartbeat/HeartbeatJobService.kt` (new, Android) | the `JobService` |
| `heartbeat/PlatformHeartbeatScheduler.kt` (new, Android) | `JobScheduler` adapter |
| `heartbeat/HeartbeatNotifier.kt` (new, Android) | channel + notification |
| `net/SharedApi.kt` (new, pure) | `ApiHolder` + `object SharedApi` |
| `data/SettingsStore.kt` | implements `HeartbeatStore`; a new PC address also resets the heartbeat state |
| `MainActivity.kt` | builds the controller and passes it to `SoosApp` |
| `ui/SoosApp.kt` | `SharedApi.forOrigin` instead of `remember { SoosApi(...) }`; heartbeat card in the settings screen (shown only once a valid PC address is saved) with the runtime prompt |

No change to `build.gradle.kts`, `libs.versions.toml`, `verification-metadata.xml`,
`settings.gradle.kts`, the network security config or the data-extraction rules.

### 1.3 Scripts, CI, docs

- `scripts/android_build.sh`: `ALLOWED_PERMISSIONS` gains exactly
  `"android.permission.POST_NOTIFICATIONS"` and `"android.permission.RECEIVE_BOOT_COMPLETED"`;
  its comment and the `[OK]` message are updated. `check_permissions` itself is unchanged.
- `.github/workflows/android.yml`: unchanged.
- `AI/DECISIONS.md`: item (8) of ADR 2026-10-13 "Native Android App" amended in place
  ("*amended 2026-10-17 …*"), and item (2)'s "offline heartbeat (#359) are later PRs" left as
  history; ADR 2026-10-13 #376 unchanged (it already defers tier 3 to "its own ADR": the
  amendment records the owner decision on #359 as that ADR decision).
- `Docs/REMOTE_COMPANION.md` §2j (heartbeat paragraph, owner check), §8 (limitations), §9
  (the Android heartbeat no longer "needs its own ADR").
- `AI/VERIFICATION_MATRIX.md`: ANA9, ANA12 updated; new rows AHB1–AHB8 (prefix `AHB`, because
  ANA18–ANA21 are taken by the open PR #406).
- Walkthrough `AI/walkthroughs/223_android_offline_heartbeat.md`.

## 2. Types & Signatures (Kotlin)

### 2.1 `org.soos.remote.heartbeat` — pure (no `android.*` import)

```kotlin
enum class CheckOutcome { REACHABLE, SIGN_IN_NEEDED, UNREACHABLE }
enum class Notice { PC_OFFLINE, PC_BACK_ONLINE, SIGN_IN_NEEDED }

/** failures in 0..FAILURES_BEFORE_ALERT (saturating). */
data class HeartbeatState(
    val failures: Int = 0,
    val offlineNotified: Boolean = false,
    val signInNotified: Boolean = false,
)
data class HeartbeatStep(val state: HeartbeatState, val notice: Notice?)
data class HeartbeatJobSpec(val jobId: Int, val intervalMs: Long, val flexMs: Long, val persisted: Boolean)

object HeartbeatPolicy {
    const val JOB_ID = 359
    const val INTERVAL_MS = 900_000L         // 15 min = JobInfo.getMinPeriodMillis()
    const val FLEX_MS = 300_000L             // 5 min  = JobInfo.getMinFlexMillis()
    const val FAILURES_BEFORE_ALERT = 2
    val JOB = HeartbeatJobSpec(JOB_ID, INTERVAL_MS, FLEX_MS, persisted = true)

    fun classify(result: ApiResult<StatusView>): CheckOutcome
    fun next(state: HeartbeatState, outcome: CheckOutcome): HeartbeatStep
    fun sanitize(state: HeartbeatState): HeartbeatState   // clamps failures into 0..2
}
```

`classify`: `Ok` → `REACHABLE`; `Refused(403, "login_required")` → `SIGN_IN_NEEDED`;
`Refused(_, non-null result)` → `REACHABLE`; `Refused(_, null)` and `Unreachable` →
`UNREACHABLE`.

`next(state, outcome)` (input sanitized first):

| outcome | `offlineNotified` before | new state | notice |
|---|---|---|---|
| `UNREACHABLE` | false, `failures + 1 < 2` | failures + 1 | none |
| `UNREACHABLE` | false, `failures + 1 >= 2` | failures 2, offlineNotified | `PC_OFFLINE` |
| `UNREACHABLE` | true | failures 2 (saturated), unchanged | none |
| `REACHABLE` | true | fresh, signIn unchanged | `PC_BACK_ONLINE` |
| `REACHABLE` | false | fresh, signIn unchanged | none |
| `SIGN_IN_NEEDED` | true | failures 0, offline false, signIn unchanged | `PC_BACK_ONLINE` |
| `SIGN_IN_NEEDED` | false, signIn false | failures 0, signInNotified | `SIGN_IN_NEEDED` |
| `SIGN_IN_NEEDED` | false, signIn true | failures 0, signInNotified | none |

`signInNotified` is never reset by a check (revision 2, candid review MINOR-1): a background
check from an ended app process has no Funnel session by design, so re-arming it on
`REACHABLE` would repeat the notice after every app run although the offline detection works
without a session. It is reset only by `enable()`, `disable()` and a new PC address: at most
one sign-in notice per opt-in.

Revision 2 (candid review of 2026-10-17, MINOR-1 to MINOR-3 and two suggestions): sign-in
notice once per opt-in (above); the job service does its storage reads and writes on the worker
thread; `PlatformHeartbeatScheduler` also maps `IllegalStateException` (job-count limit) to
`false`; the owner check AHB8 adds an idle phone with the PC up (no false alert); the
walkthrough invariant names the migrated test instead of the branch commit.

Revision 3 (candid review round 2, MAJOR-1): a force stop removes the persisted job while the
stored switch stays on. `HeartbeatScheduler` gains `isScheduled(jobId)` (backed by
`JobScheduler.getPendingJob`) and `HeartbeatController` gains `ensureScheduled()`, called from
`MainActivity.onCreate`: off → nothing; on and pending → untouched (the period is kept); on
and missing → scheduled again with the state kept; refused → the switch is turned off and the
card shows the "could not be scheduled" note (`scheduleRefused`, memory only). Round-2 MINOR-2
(an outage during which notifications were blocked is not reported later) is documented in
§2j.

Revision 4 (candid review round 3, MINOR-1 to MINOR-3): the job cancels a stale job through
`HeartbeatController.cancelIfDisabled()` (one locked step, no write) instead of reading
`enabled` and calling `disable()`; `SoosApi.send` applies the response's cookie update and
the `login_required` drop under the client's lock and drops the session only when it is still
the one sent with the request, so a late refusal of a job request never signs the screens out;
Battery Saver, Data Saver and the restricted battery setting are listed as false-alert causes
(§2j, §8, ADR) and the AHB8 idle step runs with Battery Saver too.

Revision 5 (candid review round 4, MINOR-1 to MINOR-3 and suggestion 5): a new PC address
resets the heartbeat state through `HeartbeatController.resetState()` (under the lock) instead
of the preferences setter; the controller methods with an early `return` use block bodies;
AHB4–AHB6 stay pending until the CI job runs the JVM tests; two over-long doc lines rewrapped.

Revision 6 (candid review round 5, MINOR-1 and MINOR-2): `HeartbeatStore` exposes `pcUrl`;
`HeartbeatController.savePcUrl(url)` (replacing `resetState`) resets the state and saves the
address in one locked step, and `onCheck(checkedUrl, outcome)` drops, without a write, the
result of a check made for an address that is no longer the saved one. The job service gives
each run its own stop flag (`onStopJob` sets the current run's flag).

```kotlin
data class NoticeText(val title: String, val body: String)
object HeartbeatTexts {
    const val CHANNEL_ID = "pc_status"
    const val CHANNEL_NAME = "PC status"
    const val CHANNEL_DESCRIPTION = "Alerts when your PC stops answering and when it is back."
    fun of(notice: Notice): NoticeText            // fixed English, see §2.6
    fun notificationId(notice: Notice): Int       // offline and back-online share id 1, sign-in id 2
}
```

### 2.2 `HeartbeatController.kt` — pure

```kotlin
interface HeartbeatScheduler {
    fun schedule(spec: HeartbeatJobSpec): Boolean
    fun cancel(jobId: Int)
}
interface HeartbeatStore {
    var heartbeatEnabled: Boolean
    fun loadHeartbeatState(): HeartbeatState
    fun saveHeartbeatState(state: HeartbeatState): Boolean
}
class HeartbeatController(store: HeartbeatStore, scheduler: HeartbeatScheduler) {
    val enabled: Boolean
    fun enable(): Boolean      // reset state, enabled = true, schedule(JOB); on false: enabled = false, cancel, return false
    fun disable()              // cancel(JOB_ID), enabled = false, reset state
    fun onCheck(outcome: CheckOutcome): Notice?   // disabled → cancel(JOB_ID), null, no write
}
```

All three methods hold one process-wide lock (`synchronized` on a private companion object),
so a toggle in the UI and a finishing job never interleave inside the controller.

### 2.3 `net/SharedApi.kt` — pure

```kotlin
class ApiHolder(private val transport: () -> HttpTransport) {
    @Synchronized fun forOrigin(origin: String): SoosApi   // same origin → same instance; other origin → new instance (old session dropped)
}
object SharedApi { fun forOrigin(origin: String): SoosApi }  // ApiHolder { UrlConnectionTransport() }
```

### 2.4 Android adapters

- `PlatformHeartbeatScheduler(context)`: `JobInfo.Builder(spec.jobId, ComponentName(context,
  HeartbeatJobService::class.java)).setPeriodic(spec.intervalMs, spec.flexMs)
  .setPersisted(spec.persisted)` and no network constraint (D-3);
  `schedule` returns `RESULT_SUCCESS`; `IllegalArgumentException`/`SecurityException`/`IllegalStateException` → `false`.
  `cancel` → `JobScheduler.cancel(jobId)`.
- `HeartbeatJobService : JobService`:
  - `onStartJob` (revision 2: all work on one worker `Thread`, storage included, and
    returns `true`): controller disabled → `controller.disable()` (cancels); no valid saved PC
    address, or airplane mode on → nothing counted; otherwise
    `SharedApi.forOrigin(origin).status()` → `classify` → unless stopped,
    `controller.onCheck` → `HeartbeatNotifier.show(notice)`. Always `jobFinished(params,
    false)`. Any `RuntimeException` in the worker discards the check (nothing counted, nothing
    shown).
  - `onStopJob`: marks the run stopped (the result is discarded), returns `false` (no
    backoff retry; the periodic schedule continues).
- `HeartbeatNotifier(context).show(notice)`: returns without posting unless
  `checkSelfPermission(POST_NOTIFICATIONS) == PERMISSION_GRANTED`; creates the channel
  (`IMPORTANCE_DEFAULT`), posts `Notification.Builder(context, CHANNEL_ID)` with the monochrome
  launcher drawable as small icon, the fixed title and body, `CATEGORY_STATUS`, auto-cancel and
  a `PendingIntent.getActivity` to `MainActivity` (`FLAG_IMMUTABLE`). No other data.

### 2.5 UI (settings screen)

A "PC offline alerts" card with a `Switch`, shown in the settings screen only when a valid PC
address is saved. Turning it on: if `POST_NOTIFICATIONS` is granted, `controller.enable()`;
otherwise the runtime prompt (`ActivityResultContracts.RequestPermission`) and `enable()` only
when granted; a denial leaves the switch off with a fixed message. Turning it off:
`controller.disable()`. The permission is never requested anywhere else. When the heartbeat is
on but the permission was later revoked, the card shows a fixed note ("Notifications are
blocked …") instead of failing silently (plan-evaluation finding m-2).

### 2.6 Fixed texts

| Notice | Title | Body |
|---|---|---|
| `PC_OFFLINE` | `PC went offline` | `soos could not reach your PC in the last two checks.` |
| `PC_BACK_ONLINE` | `PC back online` | `soos can reach your PC again.` |
| `SIGN_IN_NEEDED` | `Sign-in needed` | `Open soos and sign in with your passkey to see the PC status.` |

No `$` template, no PC name, host, address, status value or time.

## 3. Constants & Config

| Name | Location | Value | Sentinel / bound |
|---|---|---|---|
| `HeartbeatPolicy.JOB_ID` | `heartbeat/Heartbeat.kt` | 359 | the app's only job |
| `INTERVAL_MS` | same | 900 000 | platform minimum period |
| `FLEX_MS` | same | 300 000 | platform minimum flex |
| `FAILURES_BEFORE_ALERT` | same | 2 | stored count clamped into `0..2` (negative → 0) |
| `ALLOWED_PERMISSIONS` | `scripts/android_build.sh` | the 4 existing + `POST_NOTIFICATIONS`, `RECEIVE_BOOT_COMPLETED` | exact names, order fixed |
| Preference keys | `SettingsStore` | `heartbeat_enabled` (default `false`), `heartbeat_failures`, `heartbeat_offline_notified`, `heartbeat_sign_in_notified` | missing → off / fresh state |

## 4. Error Taxonomy

| Situation | Result |
|---|---|
| Notification permission refused or revoked | heartbeat stays off (prompt) / checks run, nothing posted |
| `JobScheduler` refuses (`RESULT_FAILURE`, exception) | `enable()` returns `false`, switch off, fixed message |
| Job runs while disabled (stale) | job cancelled, nothing counted |
| No valid PC address | nothing counted |
| Airplane mode on | no request, nothing counted |
| `onStopJob` before the answer | result discarded |
| Unexpected runtime exception in the worker | discarded, job finished |

## 5. Invariants Touched

ANA9 (manifest, migrated by c8dccd1), ANA11 (Kotlin hygiene: no logging, transport bounds,
cookie never stored), ANA12 / ANA17 (APK permission allowlist), ANA8 (catalog unchanged),
RC-1 (no server change). New: AHB1–AHB8.

## 6. Test Hooks for Tester

- `HeartbeatPolicy.classify/next/sanitize`, `HeartbeatTexts`, `HeartbeatController` with fake
  `HeartbeatStore` and `HeartbeatScheduler`, `ApiHolder` with a fake `HttpTransport` — JVM
  tests under `android/app/src/test/java/org/soos/remote/` (CI only).
- Rust invariants `tests/invariants/src/android_heartbeat_contract.rs`: the full allowlist and
  its behaviour through `run_check_permissions`; manifest service shape; Kotlin static rules
  (no WorkManager, no boot receiver code, no network constraint, only `.status()` in the
  heartbeat package, constants, `setPersisted`, fixed texts, single runtime prompt site, no
  logging); docs, ADR, matrix, walkthrough.

## 7. Documentation Drift / ADR

- ADR 2026-10-13 "Native Android App" item (8) says "`INTERNET` only" and "one exported
  component": amended in place (the exported count stays one).
- `Docs/REMOTE_COMPANION.md` §9 says "the watchdog and the Android heartbeat need their own
  ADR": the Android part is now decided (ADR amendment), the watchdog part stays.
- Issue #359 tier 3 text says "WorkManager": superseded by the owner decision of 2026-10-17
  (platform `JobScheduler`, no new dependency).
