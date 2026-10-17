# Tester Contract — GitHub #415: opt-in PC-offline heartbeat in the native Android app

Spec: `AI/architect_spec_android_offline_heartbeat.md` (revision 1). Written before the
production code (TDD Red). Every test below is an immutable contract for the developer.

## Red evidence

- Rust invariants (`cargo test --locked -p soos-invariants android_`), before the
  implementation: `test_ahb_s1` (allowlist lacks the two names, `assert_eq!` at the pinned list),
  `test_ahb_s2` (no `<service` in the manifest), `test_ahb_s3` (`heartbeat/Heartbeat.kt`
  missing), `test_ahb_s4` (ADR item (8) lacks the amendment) and the lead's migrated
  `test_ana_s6_manifest_hardening` (permission list) fail; the 24 other Android tests pass.
- Kotlin JVM tests: no JDK/SDK on the development host; they reference only the specified API
  (`org.soos.remote.heartbeat.*`, `org.soos.remote.net.ApiHolder`), so they cannot compile
  before it exists, and they run in the CI job `Android (debug build)` (`testDebugUnitTest`).

## Contract table

| Test | File | Acceptance line |
|---|---|---|
| `defaultsAreTheOwnerDecision` | `android/app/src/test/java/org/soos/remote/HeartbeatPolicyTest.kt` | 15 min, flex 5 min, 2 failures, persisted job, fresh state |
| `classifyMapsEveryApiResult` | same | status `Ok` reachable; `403 login_required` sign-in needed; other soos refusals reachable; no result token or no answer unreachable |
| `oneFailureDoesNotAlert`, `twoConsecutiveFailuresGiveExactlyOneAlert` | same | alert after 2 consecutive failures (catches `>` vs `>=`) |
| `aLongOutageNeverAlertsTwice` | same | no duplicate alerts; saturating counter |
| `recoveryAfterAnAlertGivesExactlyOneBackOnlineNotice` | same | one back-online notice on recovery |
| `aSingleFailureThenRecoveryIsSilent`, `failuresMustBeConsecutive` | same | no back-online without an alert; consecutive only |
| `aSecondOutageAlertsAgain` | same | one alert per outage |
| `signInNeededIsReportedOncePerOptIn`, `signInNeededCountsAsReachable`, `anOutageDuringASignInEpisodeStillAlerts` | same | "sign-in needed" at most once per opt-in; a soos answer proves reachability; never masks an outage |
| `storedStatesAreClamped` | same | stored count clamped, no overflow |
| `offByDefault` | `HeartbeatControllerTest.kt` | off by default, nothing scheduled |
| `optInSchedulesThePersistedFifteenMinuteJobWithAFreshState` | same | opt-in schedules the job spec, resets state |
| `optOutCancelsTheJobAndResetsTheState` | same | opt-out cancels the job |
| `aCheckWhileDisabledCancelsTheStaleJobAndChangesNothing` | same | a stale job is cancelled, nothing written |
| `aRefusedScheduleLeavesTheHeartbeatOff` | same | scheduler failure → off |
| `checksDriveTheStateMachineAndPersistIt`, `aStateRestoredAfterAProcessRestartIsHonoured`, `reEnablingStartsOver` | same | state persisted across process restarts; re-enable starts over and re-arms the sign-in notice |
| `textsAreTheFixedEnglishSentences`, `textsCarryNoPcData`, `backOnlineReplacesTheOfflineNotification` | `HeartbeatTextsTest.kt` | fixed English texts, no PC data, back-online replaces the alert |
| `oneInstancePerOriginSharesTheInMemorySession`, `anotherOriginDropsTheSession` | `SharedApiTest.kt` | the job reuses the in-memory session, `GET /api/status` only; another PC drops it |
| `test_ahb_s1_apk_permission_allowlist_is_pinned` | `tests/invariants/src/android_heartbeat_contract.rs` | full allowlist pinned; WorkManager / foreground / network-constraint / alarm permissions refused |
| `test_ahb_s2_manifest_declares_one_guarded_job_service` | same | one non-exported `BIND_JOB_SERVICE` service, no receiver, no boot action |
| `test_ahb_s3_kotlin_heartbeat_contract` | same | Kotlin static contract (constants, persisted platform job, no network constraint, status only, no cookie storage, fixed texts, one prompt site) |
| `test_ahb_s4_docs_adr_matrix_walkthrough` | same | ADR item (8), §2j/§8/§9, AHB1–AHB8, walkthrough 223 |
| `test_ana_s6_manifest_hardening` (lead, c8dccd1) | `tests/invariants/src/android_native_contract.rs` | exactly 3 permissions, guarded service |

## Revision 2 (spec revision 2, before any commit)

The candid review of 2026-10-17 (MINOR-1) changed the sign-in rule from once per episode to
once per opt-in. The tester updated its own, not yet committed, tests to the revised spec:
`signInNeededIsReportedOncePerEpisode` became `signInNeededIsReportedOncePerOptIn` (its third
case now asserts that a later `REACHABLE` does not re-arm the notice, a stronger assertion) and
`reEnablingStartsOver` gained the re-arm-by-opt-in assertions. `test_ahb_s4` names the migrated
test instead of the branch commit. No assertion was weakened.

## Revision 3 (spec revision 3, before any commit)

New controller tests `openingTheAppReschedulesAJobDroppedByAForceStop`,
`aPendingJobKeepsItsPeriod`, `anOptedOutHeartbeatIsNeverScheduledOnOpen`,
`aRefusedRescheduleTurnsTheSwitchOff`; `aRefusedScheduleLeavesTheHeartbeatOff` gains a
`scheduleRefused` assertion; the fake scheduler gains `isScheduled` (setup). `test_ahb_s3`
gains the `getPendingJob` and `ensureScheduled()` needles. No assertion was weakened.

## Revision 4 (candid review round 3, before any commit)

New tests `cancelIfDisabledCancelsOnlyAStaleJobAndNeverWrites` (controller) and
`aLateRefusalOfARequestSentWithoutASessionKeepsANewSession` (shared client). No existing
assertion changed; the existing `SoosApiTest.loginStoresTheCookieInMemoryAndSendsItBack` still
pins that `login_required` drops the session that was sent.

## Revision 5 (candid review round 4, before any commit)

New controller test `aNewPcAddressStartsOverWithoutABackOnlineNotice`. No assertion changed.

## Revision 6 (candid review round 5, before any commit)

`onCheck` takes the checked address: every controller test call passes `PC` (the fake store's
saved address; argument only). `aNewPcAddressStartsOverWithoutABackOnlineNotice` now goes
through `savePcUrl` and also asserts that the same address keeps the state; new test
`aLateCheckOfThePreviousPcIsDropped`. No assertion was weakened.

## Existing tests touched

- `tests/invariants/src/android_native_contract.rs`: `fn run_check_permissions` becomes
  `pub(crate)` (visibility only, so the new allowlist test reuses the verbatim harness). No
  assertion changed. The only assertion change on the branch is the lead's owner-approved
  c8dccd1.
- No Kotlin test file existing before this branch is modified.
