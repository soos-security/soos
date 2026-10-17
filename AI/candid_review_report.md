# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `feat/android-offline-heartbeat`
- **Base (merge-base)**: `6eb0746`
- **Reviewed-Diff-Fingerprint**: `6285840149ecb8a7749ab11d5e8b5941e45a7284511e61cd4266f1d15c28b6c3`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/architect_spec_android_offline_heartbeat.md`,
  `AI/auditor_constraints_android_offline_heartbeat.md`,
  `AI/tester_contract_android_offline_heartbeat.md`,
  `AI/walkthroughs/223_android_offline_heartbeat.md`, `Docs/REMOTE_COMPANION.md`,
  `android/app/src/main/AndroidManifest.xml`,
  `android/app/src/main/java/org/soos/remote/MainActivity.kt`,
  `android/app/src/main/java/org/soos/remote/data/SettingsStore.kt`,
  `android/app/src/main/java/org/soos/remote/heartbeat/Heartbeat.kt`,
  `android/app/src/main/java/org/soos/remote/heartbeat/HeartbeatController.kt`,
  `android/app/src/main/java/org/soos/remote/heartbeat/HeartbeatJobService.kt`,
  `android/app/src/main/java/org/soos/remote/heartbeat/HeartbeatNotifier.kt`,
  `android/app/src/main/java/org/soos/remote/heartbeat/PlatformHeartbeatScheduler.kt`,
  `android/app/src/main/java/org/soos/remote/net/SharedApi.kt`,
  `android/app/src/main/java/org/soos/remote/net/SoosApi.kt`,
  `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt`,
  `android/app/src/test/java/org/soos/remote/HeartbeatControllerTest.kt`,
  `android/app/src/test/java/org/soos/remote/HeartbeatPolicyTest.kt`,
  `android/app/src/test/java/org/soos/remote/HeartbeatTextsTest.kt`,
  `android/app/src/test/java/org/soos/remote/SharedApiTest.kt`, `scripts/android_build.sh`,
  `tests/invariants/src/android_heartbeat_contract.rs`,
  `tests/invariants/src/android_native_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

The diff adds the opt-in PC-offline heartbeat to the native Android app (GitHub #415, part of
#359 tier 3): one non-exported platform `JobService` guarded by `BIND_JOB_SERVICE`, a persisted
15 min / 5 min flex periodic job without a network constraint, a pure state machine (alert after
2 consecutive failures, one back-online notice, one sign-in notice per opt-in), a pure
controller under a process-wide lock, a process-wide `SoosApi` per origin (`SharedApi`), a
session-drop guard in `SoosApi.send`, the settings-screen switch with the only runtime
`POST_NOTIFICATIONS` prompt, the two new permissions in the manifest and in the APK allowlist,
and docs, ADR amendment, matrix rows AHB1–AHB8 and walkthrough 223.

No production Rust is touched; no PAM, daemon or IPC path is affected. The Kotlin was read for
compile errors and Android API misuse (`minSdk` 34, `targetSdk` 36, Kotlin 2.2.21): none found.
`cargo test --locked -p soos-invariants` passes (711 passed, 0 failed, including AHB-S1..S4 and
the migrated ANA-S6); `cargo fmt --all -- --check` and
`cargo clippy -p soos-invariants --all-targets -- -D warnings` are clean. No CRITICAL or MAJOR
finding; two MINOR and two SUGGESTION items below.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Test files touched:

- `android/app/src/test/.../HeartbeatControllerTest.kt` — new.
- `android/app/src/test/.../HeartbeatPolicyTest.kt` — new.
- `android/app/src/test/.../HeartbeatTextsTest.kt` — new.
- `android/app/src/test/.../SharedApiTest.kt` — new.
- `tests/invariants/src/android_heartbeat_contract.rs` — new.
- `tests/invariants/src/lib.rs` — registers the new module only.
- `tests/invariants/src/android_native_contract.rs` — modified (below).

Removed or changed checks:

- patch lines 2776–2777, `test_ana_s6_manifest_hardening`:
  `assert_eq!(permissions.len(), 1, "exactly one permission")` and
  `assert!(permissions[0].contains("android.permission.INTERNET"))` are replaced by an exact
  sorted equality against `[INTERNET, POST_NOTIFICATIONS, RECEIVE_BOOT_COMPLETED]`. Still an
  exact-set check (an extra or missing permission fails). Justified by the owner decision of
  2026-10-17 on #359, which names exactly these two permissions; recorded as the ANA9
  amendment in the matrix and as the lead's contract migration in the walkthrough.
- patch line 2822: `"<service"` removed from the forbidden list of the same test. Replaced by
  "at most one `<service>`, each with `BIND_JOB_SERVICE` and `exported=\"false\"`", plus the
  stricter AHB-S2 (exactly one service with the exact name, no intent filter on it). Justified
  by the same owner decision (one platform `JobService`). `<receiver>` and `<provider>` stay
  forbidden; the exported-component count (1) is unchanged.
- `run_check_permissions` becomes `pub(crate)`: visibility only, no assertion change.

New escape hatches (`#[ignore]`, `should_panic`, tolerance): none. Inline `mod tests` changes:
none. No pre-existing Kotlin test file is modified (`SoosApiTest.kt` untouched; its
`login_required drops the session` case still holds because the session sent equals the stored
one).

## 3. Deep Reasoning Audit

### Logic & Architecture

- `classify`: `Ok` → reachable; `Refused(403, "login_required")` → sign-in; other refusal with
  a token → reachable; `Refused(_, null)` (gateway 502, non-soos body) and `Unreachable` →
  failure. Tried: a captive portal or foreign server — HTTPS to a `ts.net` host with system CAs
  fails the handshake (IOException → `Unreachable`); a 200 with a non-soos body fails parsing →
  `Unreachable`. PASS.
- `next`: walked the full table incl. saturation (`Int.MAX_VALUE`/`Int.MIN_VALUE` stored counts
  are clamped before `+1`, no overflow), outage during a sign-in episode, recovery via
  `login_required`, second outage. Matches spec §2.1. PASS.
- Controller: `enable` resets state then schedules; refusal turns the switch off and cancels;
  `ensureScheduled` keeps a pending job's period and the state; `onCheck` drops results while
  disabled (and cancels) or for a different saved address; a notice is returned only after a
  successful synchronous `commit()`, so a failing write cannot repeat a notice each period.
  `savePcUrl` resets state and saves the address under the same lock. PASS.
- Job service: work off the main thread; `cancelIfDisabled` before any request; airplane mode
  and invalid address count nothing; stopped run discarded; `jobFinished(params, false)` in
  `finally`; `onStopJob` returns `false` (no backoff). `cancel()` of a running job from inside
  the worker triggers `onStopJob` and is harmless. PASS.
- `SoosApi.send` guard: tried the race "job sends without a session, UI signs in, late 403" —
  the session is kept (pinned by `SharedApiTest`). Tried "logout while a job request is in
  flight" — `soos-remote` emits `Set-Cookie` only on login verify and logout
  (`crates/remote/src/server.rs`), so a status response cannot resurrect a cleared session.
  PASS.
- `SharedApi` race across an address change (job read the old address, UI switched to the new
  one): the result is dropped by `onCheck`, but `forOrigin(old)` replaces the holder's client
  → MINOR-2.
- Docs reference a branch commit SHA that disappears with the squash merge → MINOR-1.

### PAM Concurrency & Deadlines

Not affected: no change under `crates/pam` or any Rust production crate. The Android check is
bounded by the existing transport (5 s connect, 5 s read, 16 KiB body, no redirects) and by the
platform job time limit. PASS.

### Panic Safety & Fail-Closed

No Rust production code. Kotlin: no `!!` in production code; `PlatformHeartbeatScheduler` maps
`IllegalArgumentException`, `SecurityException`, `IllegalStateException` to `false`; the worker
catches `RuntimeException` and still finishes the job. The heartbeat never grants anything (it
only reads `/api/status` and posts local notifications), so no fail-open authorization path
exists. PASS.

### Test Integrity & Anti-Weakening

See §2: the only changed assertions are the owner-approved ANA-S6 migration, replaced by
equally exact or stricter checks. New JVM tests fail against plausible wrong implementations:
alerting after one failure, re-alerting during a long outage, missing back-online, re-arming
the sign-in notice on `REACHABLE`, counting a stale-address check, dropping a session on a late
refusal (would fail `aLateRefusalOfARequestSentWithoutASessionKeepsANewSession`). AHB-S1 runs
the script's own `check_permissions` and checks refusal of near-miss and non-approved names.
JVM results AHB4–AHB6 are honestly left "Pending CI" in the matrix. PASS.

### Memory, Bounds & Secrets

Stored state is a boolean, a clamped int and two flags; the session cookie stays in memory
(`SharedApi` holds no storage, pinned by AHB-S3). Notification texts are fixed English without
host, URL or status values (`HeartbeatTextsTest`, AHB-S3). `PendingIntent` is explicit and
`FLAG_IMMUTABLE`. No logging added. Accepted risk (documented in ADR and §8): while the process
lives, a Funnel check refreshes the session idle timer; the absolute 8 h lifetime is unchanged.
PASS.

### Supply Chain & Automation

No dependency, catalog, Gradle or verification-metadata change; no workflow change.
`scripts/android_build.sh` only extends `ALLOWED_PERMISSIONS` with the two owner-approved names
and updates a message; `check_permissions` logic unchanged. `RECEIVE_BOOT_COMPLETED` is required
by `setPersisted(true)`; `ACCESS_NETWORK_STATE` correctly avoided (no network constraint).
Lint: `NotificationPermission` satisfied (declared), `MissingPermission` satisfied
(`checkSelfPermission` in the same method), job service guarded by `BIND_JOB_SERVICE`;
`warningsAsErrors = false`. PASS.

### English-Only Policy

All added code, comments, docs and texts are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `AI/VERIFICATION_MATRIX.md:2601` (also
  `AI/walkthroughs/223_android_offline_heartbeat.md:6`,
  `AI/architect_spec_android_offline_heartbeat.md:4,290`) — the ANA9 amendment cites the branch
  commit `c8dccd1`, which becomes unreachable after the squash merge into `main`. Required
  change: cite the migrated test (`android_native_contract::test_ana_s6_manifest_hardening`)
  and the PR number instead of the branch SHA, at least in the matrix row.
- **[MINOR]** `android/app/src/main/java/org/soos/remote/heartbeat/HeartbeatJobService.kt:39` —
  a job that read the previous address before the user saved a new one calls
  `SharedApi.forOrigin(oldOrigin)`, which replaces the holder's client for the new origin; the
  screen keeps its remembered instance, but the next job gets a fresh client without the
  in-memory session (a spurious "Sign-in needed" over Funnel is possible). The outcome itself is
  correctly dropped by `onCheck`. Suggested change: re-read `settings.pcUrl` right before the
  request and skip when it changed, or make `ApiHolder` not replace a client for a lookup that
  does not match the current saved origin. Narrow race; not blocking.
- **[SUGGESTION]** `android/app/src/main/java/org/soos/remote/heartbeat/HeartbeatController.kt:44,76`
  — `enable()`/`disable()` run a synchronous `SharedPreferences.commit()` and `JobScheduler`
  binder calls on the main thread from the switch handler. Cheap today; consider moving them off
  the main thread if StrictMode is ever enabled.
- **[SUGGESTION]** `Docs/REMOTE_COMPANION.md:1214,1648` — line 1214 exceeds the document's
  ~100-column wrap and line 1648 leaves a short ragged line; rewrap.

## 5. Final Verdict

**VERDICT: APPROVED**
