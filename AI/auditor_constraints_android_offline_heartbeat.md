# Audit Constraints — GitHub #415: opt-in PC-offline heartbeat in the native Android app

Inputs: spec revision 1, `AI/tester_contract_android_offline_heartbeat.md`, the current
`android/` sources, `scripts/android_build.sh`, the manifest contracts (ANA-S6 after c8dccd1).

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| C1 | Manifest: exactly `INTERNET`, `POST_NOTIFICATIONS`, `RECEIVE_BOOT_COMPLETED`; one `<service>` `exported="false"` with `BIND_JOB_SERVICE`; no receiver, provider, intent filter on the service or boot action; header comment free of tag text | `AndroidManifest.xml` | ANA-S6, AHB-S2 |
| C2 | The allowlist gains exactly the two names, one quoted name per line; `check_permissions` unchanged | `scripts/android_build.sh` | AHB-S1, ANA-S10, ANA-S13 |
| C3 | No network constraint (`ACCESS_NETWORK_STATE` is not approved); no expedited, foreground, alarm or backoff APIs | `heartbeat/*` | AHB-S3 |
| C4 | The only request is `SoosApi.status()` on the saved, validated PC origin through `SharedApi` (`UrlConnectionTransport`: HTTPS, no redirect, 5 s / 5 s, 16 KiB, system CAs) | `HeartbeatJobService` | AHB-S3, ANA-S8 |
| C5 | The session cookie stays in memory: no `SessionCookie`, `Cookie` or `SharedPreferences` in the heartbeat package; `SharedApi` holds no storage; stored keys hold a count and flags only | `heartbeat/*`, `SharedApi.kt`, `SettingsStore.kt` | AHB-S3, ANA-S8 |
| C6 | No logging or printing anywhere (no `Log.`, `print(`, `println(`, `printStackTrace`); no URL, body or status value reaches a notification | all Kotlin | ANA-S8, AHB-S3, `HeartbeatTextsTest` |
| C7 | Notification texts are compile-time constants without templates; no large icon, subtext, actions or style; `PendingIntent` is `FLAG_IMMUTABLE` and explicit (`MainActivity`) | `Heartbeat.kt`, `HeartbeatNotifier.kt` | AHB-S3 |
| C8 | Nothing is posted without `POST_NOTIFICATIONS` granted (`checkSelfPermission` in the same method as `notify`, also satisfying lint `MissingPermission`) | `HeartbeatNotifier.show` | AHB-S3, CI lint |
| C9 | The runtime prompt is requested only from the heartbeat switch (one site) | `ui/SoosApp.kt` | AHB-S3 |
| C10 | The worker thread never crashes the process: `RuntimeException` → check discarded; `jobFinished(params, false)` always called once; `onStopJob` returns `false` and discards the result | `HeartbeatJobService` | AHB-S3, review |
| C11 | Controller methods are serialized by one lock; `onCheck` re-reads `enabled`; disabled → `cancel`, no write | `HeartbeatController` | `HeartbeatControllerTest` |
| C12 | Stored values are sanitized on load (`failures` clamped into `0..2`) and the counter saturates (no overflow) | `HeartbeatPolicy.sanitize`, `SettingsStore` | `HeartbeatPolicyTest` |
| C13 | Background writes use `commit()` (the result returned to the controller) so a state survives the job process being killed right after `jobFinished` | `SettingsStore.saveHeartbeatState` | review |
| C14 | `JobScheduler.schedule` exceptions (`IllegalArgumentException`, `SecurityException`, `IllegalStateException`) map to `false` | `PlatformHeartbeatScheduler` | review |
| C15 | No new dependency: catalog, `build.gradle.kts`, `settings.gradle.kts` and `verification-metadata.xml` unchanged; `activity-compose` 1.11.0 already provides the permission launcher | `android/gradle/*`, `android/app/build.gradle.kts` | ANA-S2, ANA-S4, `git diff --stat` |
| C16 | The forbidden words of ANA-S5 (`webview`, `analytics`, …) and colour literals of ANA-S7 do not appear in any new file | new files | ANA-S5, ANA-S7 |
| C17 | A new PC address is saved and resets the heartbeat state in one locked step, and a late check of the previous address is dropped (no write, no notice for another PC); each job run has its own stop flag | `HeartbeatController.savePcUrl`/`onCheck`, `ui/SoosApp.kt`, `HeartbeatJobService` | `HeartbeatControllerTest` |

### Pre-existing violations found (not introduced by this change)

None. Note: `UrlConnectionTransport` does not bound DNS resolution time (platform resolver);
unchanged and outside this change; the job runs on its own worker thread, so a slow resolver
only delays that run.

### Clearance: CLEARED
