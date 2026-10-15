# Plan Evaluation Report
- **Date**: 2026-10-15
- **Issue**: Post-merge follow-ups of 2026-10-15 (no backlog id) (GitHub #398)
- **Branch**: `fix/post-merge-followups`
- **Base commit**: `640e1db`
- **Spec evaluated**: `AI/architect_spec_post_merge_followups.md`

## 1. Coverage Matrix

| Acceptance line (GitHub #398) / TDD test | Spec element | Status |
|---|---|---|
| 1a. After restarting a running unit, poll its state through the existing `ProgramRunner` (bounded, a few `systemctl --user show` calls, at most about 3 s, off the UI thread); report `failed`, `inactive` or automatic restart | §1 `RestartCheck`, `companion_state_command`, `parse_companion_state`, behaviour steps 3–7 | Covered by design; the production path that enables it is not pinned by any test (F1) |
| 1b. Testable with a fake runner; existing tests unchanged | §1 "Test-contract constraint" (opt-in `with_restart_check`, `new` unchanged) | Partial: no test is named or specified for item 1 (F2) |
| 2a. Match both `uses-permission` forms with exact names, allowlist unchanged, refuse any other `uses-permission*` line | §2 new loop with `re="^uses-permission(-sdk-23)?: name='([^']+)'"` | Covered (see F4, F5 for harness and prefix details) |
| 2b. Invariant test in `tests/invariants` | §2 dynamic test (fake `aapt2`) + static check | Covered (F4: harness misses `APK`) |
| 2c. ANA13 verified with real run ids | §2 "ANA13 evidence" | Covered; run ids verified (section 2) |
| 3a. Synthetic identity-free `GET /api/offline` body with exactly the `OfflineView`/`OfflineInterval` keys | §3 route + two scenes | Covered (F6: `reason` value and saturation arithmetic underspecified) |
| 3b. Tests checking keys against remote sources | §3 test list (uses `struct_fields`) | Covered |
| 3c. Matrix RAC8/RAC9 updated additively | Spec adds RAC19 and leaves RAC8/RAC9 unchanged | Deviation from issue wording (F7) |
| Walkthrough 212, branch `fix/post-merge-followups` | Header | Covered |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Unit is `Type=exec`, `Restart=on-failure`, `RestartPreventExitStatus=78` | `packaging/soos-remote.service` | `Type=exec`, `Restart=on-failure`, `RestartSec=5`, `RestartPreventExitStatus=78` | Yes |
| `DisableRemote` runs `try-restart` with a 3 s deadline, error prefix `Saved, but soos-remote did not restart:` | `feature_control.rs` `disable_remote`, `USER_SYSTEMCTL_TIMEOUT = 3 s` | Same | Yes |
| `MAX_UNIT_SHOW_BYTES` exists | `feature_control.rs` | `1024` | Yes |
| `SystemFeatureExecutor` built only in `app.rs` and `feature_control_tests.rs` | `app.rs` l. 201–204; tests l. 701–853 | Same (no other construction site; `onboarding_audit_tests` uses `SystemProgramRunner` only) | Yes |
| Existing test pins `new` to exactly one runner call for `DisableRemote` | `feature_control_tests.rs::test_obt_executor_disable_remote_edits_then_restarts` | `assert_eq!(calls, &[(try-restart, Some(USER_SYSTEMCTL_TIMEOUT))])` | Yes |
| `RecordingRunner` returns one fixed `show_output` for every call | `feature_control_tests.rs` l. 536–560 | Same: cannot script a probe → poll sequence | Spec silent (F2) |
| `feature_control.rs` must not contain `"systemctl"`, `"sh"`, `"-c"` literals | `test_obt_feature_control_runs_no_shell_and_no_relative_program` | Same; the planned `journalctl` error text and the `user_systemctl` helper do not violate it | OK (keep it so) |
| `app.rs` must not contain `Command::new` and must use `FeatureRunner` | `onboarding_view_tests::test_obt_app_ui_code_still_spawns_no_process` | Same; the planned wiring keeps both | OK |
| `check_permissions` reads only `uses-permission: name='...'` | `scripts/android_build.sh` l. 108–129 (`sed -n "s/^uses-permission: name='\([^']*\)'.*/\1/p"`) | Same: `uses-permission-sdk-23:` lines are dropped before the allowlist | Yes |
| `check_permissions` depends on `ALLOWED_PERMISSIONS`, `BUILD_TOOLS_VERSION`, `fail()` | script l. 35, 42–47, 49–52 | Also depends on `readonly APK=...` (l. 36) via `[[ -f "$APK" ]]` | Incomplete (F4) |
| Existing `test_ana_s10_build_script` needles | `android_native_contract.rs` l. 669–700 | `check_permissions`, `"android.permission.INTERNET"`, `readonly BUILD_TOOLS_VERSION="35.0.0"` must stay | OK if the loop rewrite keeps them |
| Bootstrap run 38446125267 | `gh run view` | `failure`, `pull_request`, `feat/android-native-app`, head `8f629a0`; log: "Gradle dependency verification metadata is empty: this bootstrap run generated it"; artifact `android-verification-metadata` (44 760 bytes) uploaded | Yes |
| Strict green run 38772439975 | `gh run view` | `success`, `pull_request`, head `d54e4bd` (last PR #391 commit); `compileDebugUnitTestKotlin` + `testDebugUnitTest` executed, `lintDebug`, `BUILD SUCCESSFUL in 3m 14s`, `[OK] android_build: APK permissions ...`; `ci` mode → `run_strict` (`--dependency-verification strict`) | Yes |
| Push run on `main` 38773360400 | `gh run view` | `success`, `push`, `main`, head `3f10f61` (#391 merge commit), same four markers | Yes |
| ANA13 says "run id recorded in the PR" | PR #391 body | No run id appears in the PR body | The matrix row must carry the ids itself (the spec does) |
| `OfflineView` keys | `crates/remote/src/offline.rs` l. 847–857 | `state`, `reason`, `network_monitor`, `intervals` | Yes |
| `OfflineInterval` keys | `offline.rs` l. 105–116 (`deny_unknown_fields`) | `reason`, `start_unix_ms`, `end_unix_ms`, `duration_ms` | Yes |
| Enum serializations | `offline.rs` l. 82–93, 815–844 (`rename_all = "snake_case"`) | `active`; `network_lost`, `sleep`, `shutdown_or_crash`; monitor `active`; the view's `reason` is `Option<OfflineUnavailableReason>` (`null` unless unavailable) | Yes (F6 on `reason`) |
| `route` is GET-only and has no `/api/offline` | `tests/fixtures/mod.rs` `readme_web::route` | Same; no existing test lists `/api/offline` as `NotFound`, so adding it breaks no assertion | Yes |
| `renderOffline` contract | `crates/remote/assets/app.js` l. 861–911 | `disabled` hides the card; `active` lists rows with `dateTime(start)`, `clockTime(end)`, `durationText(duration_ms)`, reason label | The two scenes render the card |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: the restart check runs `systemctl` as root or through `pkexec`. It runs through `user_systemctl` (`/usr/bin/systemctl --user`), which is only reached after the existing `running_as_root` refusal. The fixture route is not served from `crates/remote`; it stays in `tests/fixtures` (RAC15).
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- No PAM, daemon or IPC code is touched. GUI side: every new `systemctl show` call carries `Some(call_timeout)`; the polling runs on the `FeatureRunner` worker.
- Failure scenario: `RestartCheck { call_timeout: Duration::ZERO, .. }` makes `SystemProgramRunner` kill every probe at once, every poll is `Unknown`, and every edit ends "not confirmed running". `DEFAULT` is fine, but the field is public and unclamped.
- Failure scenario: worst-case latency of a `DisableRemote` is 0.5 s probe + 3 s `try-restart` + 3 × (0.5 s + 0.5 s) = 6.5 s, during which the runner answers `Busy`. The issue says "at most about 3 s" for the polling part; 3.5 s for probe plus polls is close enough.
- Result: FINDING (MINOR, F8).

### Pillar 3 — Panic safety & fail-closed
- Failure scenario (fail-open): production builds the executor with plain `new` (the wiring line is dropped or reverted in a later rebase). Every new test still passes because they build their own executor with `with_restart_check`, and the GUI goes back to reporting success for a refused configuration. Nothing in the spec pins `app.rs`.
- Failure scenario (wrong mapping): an implementation that maps `ActiveState=activating` to `Changing` before looking at `SubState` never reports `AutoRestart`, because systemd maps `SubState=auto-restart` to `ActiveState=activating` (service.c state translation table). With `RestartSec=5` the unit stays in `activating/auto-restart` for the whole 1.5 s polling window, so the result is "not confirmed running" instead of "stopped after the restart (restarting)". Still an error, so not fail-open, but tests must pin the `SubState` priority.
- Failure scenario (early success): an implementation that returns `Ok` on the first `Running` poll misses a configuration refusal that lands between polls 1 and 3. The spec's step 6 ("Running → remember; continue") excludes this, but only a sequenced test catches it.
- Result: FINDING (MAJOR F1, MAJOR F2).

### Pillar 4 — Dependencies
- No new crate, no feature change. The invariant test uses `bash` from the host, as other invariants already do (`installer_contract`, `systemd_unit_acceptance_contract`, ...).
- Result: PASS.

### Pillar 5 — Data confidentiality
- `systemctl show` output is never logged and the error carries only a fixed state word. The fixture body is identity-free by construction, and a new no-identity test is planned. The `remote.toml` contents never reach an error string, as before.
- Result: PASS.

### Pillar 6 — Test integrity
- **Opt-in `with_restart_check`, legitimate design or dodge?** The existing test `test_obt_executor_disable_remote_edits_then_restarts` pins the exact call vector of an executor built with `new`. Putting the probe and polls inside `new` would break that assertion, which the owner's hard constraint forbids. Making the confirmation an explicit, composable policy (`new` = edit + restart; `with_restart_check` = edit + restart + confirmation) is a legitimate design. It is ordinary policy injection, it keeps every existing contract literally true, and `RestartCheck` has to be injectable anyway so tests can use a zero interval. It becomes a dodge only if production is not provably on the stronger policy. The spec states the wiring in prose (`app.rs` … `.with_restart_check(RestartCheck::DEFAULT)`) but no test asserts it. That is F1. With F1 fixed, the design is acceptable. The doc comment of `new` must also say that it does not confirm the restart.
- No existing test is modified. The `app.rs` source-scan tests (`Command::new` absent, `FeatureRunner` present, single `self.tasks.submit(`) stay green with the planned one-line wiring change.
- Test power: item 2 dynamic tests fail on a plausible wrong implementation (sdk-23 still dropped → "non-allowlisted sdk-23 name fails" goes red). Item 3 tests fail on a missing key or a wrong enum spelling. Item 1 has no specified tests, which is F2.
- Result: FINDING (MAJOR F1, F2).

### systemd semantics claims
These are checked against the systemd source as I know it (`systemctl(1)`, `job_type_collapse()`, the `service.c` state table). No systemd was run.
- `Type=exec`: a start or restart job completes once `execve()` of the main binary succeeded, so `systemctl try-restart` (blocking, no `--no-block`) returns before `soos-remote` parses its configuration. **Correct.**
- `try-restart` on an inactive or failed unit: `JOB_TRY_RESTART` collapses to `JOB_NOP` when the unit is `UNIT_IS_INACTIVE_OR_DEACTIVATING` (inactive, failed, *and deactivating*), otherwise to `JOB_RESTART` (including `activating`, which covers `auto-restart`). The spec's step 5 (probe `Inactive`/`Failed` → no polling) is **correct**. The spec misses `deactivating`: a probe that sees `deactivating` (`Changing`) leads to a no-op restart followed by polls that see `inactive`, which is reported as "stopped after the restart (inactive)". The message is misleading but the outcome harmless (F9).
- Exit 78 under `RestartPreventExitStatus=78`: `Result=exit-code`, `ActiveState=failed`, `SubState=failed`, no restart. **Correct.**
- Runtime exit 1 under `Restart=on-failure`: `SubState=auto-restart` with `ActiveState=activating` for `RestartSec=5`. **Correct**, but systemd ≥ 254 also has the brief SubStates `auto-restart-queued` (ActiveState `activating`) and `dead-before-auto-restart` / `failed-before-auto-restart` (ActiveState `inactive`/`failed`). The last two already map to error states. `auto-restart-queued` would be `Changing` under the spec (F9).
- Clean exit 0 of the main process (`Type=exec`, no `RemainAfterExit`): `inactive/dead`, reported as stopped. Correct.

### aapt2 `dump permissions` output forms
These come from my knowledge of `DumpManifest.cpp` (`UsesPermission::Print`). aapt2 is not installed here and was not run. CI logs do not print the dump.
- `uses-permission: name='X'` and, for `<uses-permission-sdk-23>` / `<uses-permission-sdk-m>`, `uses-permission-sdk-23: name='X'`. Either may be followed by ` maxSdkVersion='N'`, and by ` requiredFeature='…'` / ` requiredNotFeature='…'` on newer build tools. The spec's regex keeps the prefix anchor and ignores the tail, so all of these are accepted. **Correct.**
- `optional-permission: name='X'` (`android:required="false"`) and `permission: X` (declared permission) also appear. The first duplicates a `uses-permission` line that is still checked; the second is a declaration, not a request. Ignoring both is fine.
- `uses-implied-permission: name='X' reason='…'` is, to my knowledge, printed by `dump badging` and not by `dump permissions`. I am not certain of that. The spec's prefix filter `uses-permission*` would silently ignore such a line (F5).

## 4. Findings

- **[MAJOR] F1 — The production path of the restart check is not pinned by any test.** Every planned test builds its own executor with `with_restart_check`. If `app.rs` keeps (or regains) plain `new`, all tests stay green while the GUI again reports success for a refused configuration, which is the defect the issue exists to fix. **Required plan change:** add a single production constructor in `feature_control.rs`, e.g. `SystemFeatureExecutor::for_desktop(paths, runner) -> Self` = `new(paths, runner).with_restart_check(RestartCheck::DEFAULT)`. Add a behavioural test that an executor from that constructor (temp paths, sequenced fake runner) makes the probe, the `try-restart` and at least one state poll. Add a source test in `crates/gui/tests` (same `include_str!("../src/app.rs")` pattern as `onboarding_view_tests`) asserting that `app.rs` builds its executor only through that constructor: zero occurrences of `SystemFeatureExecutor::new(` and one of the production constructor. Also document on `new` that it does not confirm the restart. With this, the opt-in design is legitimate.

- **[MAJOR] F2 — Item 1 has no test plan.** The spec lists tests for items 2 and 3 only, so the issue line "testable with a fake runner" maps to nothing concrete. The existing `RecordingRunner` returns one fixed output for every call and cannot script probe → poll sequences. **Required plan change:** specify a new sequenced fake runner (new helper, existing helpers untouched) and name tests that fail on plausible wrong implementations, at least:
  1. `parse_companion_state` table: `active/running` → Running; `activating/start` → Changing; `activating/auto-restart` and `inactive/auto-restart` → AutoRestart (SubState wins over ActiveState); `failed/failed` → Failed; `inactive/dead` → Inactive; any order, trimmed, other lines ignored; missing or unknown `ActiveState` → Unknown.
  2. `companion_state_command()` exact vector (`/usr/bin/systemctl --user show --property=ActiveState --property=SubState soos-remote.service`).
  3. Exact call sequence and arguments for a running unit: probe (`Some(call_timeout)`, `MAX_UNIT_SHOW_BYTES`) → `try-restart` (`Some(USER_SYSTEMCTL_TIMEOUT)`, 0) → N polls.
  4. Probe `Inactive` / `Failed` → `Ok(())` with exactly two calls (probe and `try-restart`), no poll.
  5. Running then Failed on a later poll → `Err` starting with `Saved` and naming `failed`. This catches an early return on the first `Running`.
  6. `AutoRestart` → error naming `restarting`; `Inactive` after the restart → error naming `inactive`.
  7. All polls `Changing`/`Unknown`/call error → "not confirmed running"; last poll `Running` → `Ok`.
  8. `polls: 0` → exactly one poll; `polls: 1000` → exactly `MAX_RESTART_CHECK_POLLS` polls.
  9. `try-restart` failure under a check → existing error text, no poll.
  10. `RestartCheck::DEFAULT` values pinned (3, 500 ms, 500 ms) and the worst-case total ≤ the stated bound.
  11. No error text contains the raw `systemctl show` output (feed a marker string and assert it is absent).

- **[MINOR] F3 — Probe error is treated like a running unit.** A probe that fails (`Unknown`) on a unit that was in fact inactive leads to a no-op `try-restart` followed by polls that read `inactive`, and the result is a false "stopped after the restart" error. Rare (the probe and polls use the same command). Either accept it and document it in the spec, or treat a probe `Unknown` followed by polls that all read `Inactive` as `Ok` with the "applies at next start" meaning.

- **[MINOR] F4 — Invariant harness misses `APK`.** `check_permissions` also reads the global `readonly APK=...` (`[[ -f "$APK" ]]`), and the loop must keep `local dump`. **Plan change:** the harness sets `APK` to an existing temporary file and runs under `set -euo pipefail` as the script does. Name the extraction anchors (`readonly ALLOWED_PERMISSIONS=(` … `)`, `fail() {` … `}`, `check_permissions() {` … `\n}`) so the tester does not guess. Add cases: a plain line with ` maxSdkVersion='28'` passes; an sdk-23 line with a trailing attribute and a non-allowlisted name fails.

- **[MINOR] F5 — Prefix filter vs. other request forms.** Consider failing closed on any line starting with `uses-` that is not one of the two accepted forms, so that a `uses-implied-permission:` line, if a future aapt2 prints it in `dump permissions`, is refused rather than silently ignored. If the filter stays `uses-permission*`, state in the spec why implied permissions are out of scope.

- **[MINOR] F6 — Fixture body details.** State that `reason` is `null` in both scenes (it is `Some` only when `Unavailable`). State that `start` and `end` are computed with `saturating_sub` from `now_unix_ms` and `duration_ms = end - start` of those saturated values, so that `route(..., 0)` (used by existing tests) still satisfies `start <= end` and the arithmetic. A fixed "8 h" duration constant would violate it at `now = 0`. Add `/api/offline` to the new no-identity test's path list.

- **[MINOR] F7 — Matrix wording deviates from the issue.** The issue asks for RAC8/RAC9 to be "updated additively"; the spec adds RAC19 and leaves RAC8/RAC9 unchanged. Either is acceptable, but the spec must say why (e.g. RAC8's text "fixed GET API paths" stays true, and a separate row keeps the evidence clear). The commit/PR should then say the issue checkbox is satisfied by RAC19.

- **[MINOR] F8 — `RestartCheck` fields are public and only `polls` is clamped.** Either clamp `call_timeout` to a floor (e.g. ≥ 100 ms) and `interval` to a ceiling, or document that only `DEFAULT` is used in production (enforced by F1). State the full worst-case bound including `try-restart` (6.5 s with `DEFAULT`).

- **[MINOR] F9 — systemd state edge cases.** Map any `SubState` starting with `auto-restart` (including `auto-restart-queued`, systemd ≥ 254) to `AutoRestart`. Treat a probe of `ActiveState=deactivating` like `Inactive` (no polling), because `try-restart` collapses to a no-op for deactivating units.

- **[MINOR] F10 — ANA13 evidence scope.** The cited green runs used the old filter that dropped sdk-23 lines. The Android job of this branch's own PR (its path filter includes `scripts/android_build.sh`) is the first run of the new filter against the real APK. The ANA17 row (or ANA13 note) should cite that PR run id once it is green, and the spec should say so.

## 5. Round 1 Verdict

Two MAJOR findings (F1: production wiring of the opt-in check unpinned; F2: no item-1 test plan). The opt-in `with_restart_check` design is legitimate under the zero-test-weakening constraint, but only once a test proves that production uses it.

Round 1: REVISION_REQUIRED.

## 6. Round 2 — Re-evaluation of spec §4 "Revision 1"

| Finding | Revision 1 answer | Assessment |
|---|---|---|
| F1 (MAJOR) | `SystemFeatureExecutor::for_desktop(paths, runner)` = `new(..).with_restart_check(RestartCheck::DEFAULT)`; `app.rs` uses it; `new` documents that it does not confirm; behavioural test `for_desktop_confirms_the_restart` plus source test `app_builds_the_desktop_executor` (no `SystemFeatureExecutor::new(` in `app.rs`, exactly one `SystemFeatureExecutor::for_desktop(`) | **Resolved.** Production is now provably on the confirming policy. The only other executor in `crates/gui/src` is `app/capture.rs::NoFeatureExecutor` (README captures, `readme-capture` feature), which is intentionally inert and unaffected. Reverting `app.rs` to `new` turns `app_builds_the_desktop_executor` red. The opt-in design is therefore legitimate, not a dodge. |
| F2 (MAJOR) | New `ScriptedRunner` (records program, args, deadline, `max_stdout`; queued answers) and 13 named `test_obt_restart_check_*` tests | **Resolved.** Each of the plausible wrong implementations listed in round 1 is caught: SubState priority (`parse_states`), early `Ok` on the first `Running` (`reports_a_unit_that_fails_after_running`), polling after a non-running probe (`skips_polls_when_the_unit_was_not_running`, exactly 2 calls), missing clamps, wrong deadline or bound (`confirms_a_running_unit`), leaked systemd text (`errors_carry_no_systemctl_output`). `RecordingRunner` and every existing test stay untouched. |
| F3 | Keep polling after a failed probe; document the possible "stopped after the restart (inactive)" message in `Docs/GUI_APPLICATION.md` | Accepted. It is conservative and fails toward reporting a problem, never toward a false success. |
| F4 | Extraction anchors named; harness defines `APK` and `ANDROID_HOME` and runs under `set -euo pipefail`; `local dump` kept; trailing `maxSdkVersion='28'` on both forms | Resolved. |
| F5 | Kept `uses-permission*` with a rationale | Accepted. Every `uses-permission*` line is either an exact accepted form or fails closed. One factual slip: the rationale says the app targets "API 35", but `android/app/build.gradle.kts` has `compileSdk = 36` / `targetSdk = 36` (35.0.0 is the build-tools version). The conclusion holds either way, because implied permissions only exist for very old target SDKs. Fix the wording in the walkthrough or docs (non-blocking). |
| F6 | `reason: null`; all wall times computed with `saturating_sub`; `duration_ms = end - start`; no-identity list extended | Resolved. |
| F7 | RAC8/RAC9 unchanged, with a stated reason; new row RAC19 | Accepted. RAC8 says "the fixed `GET` API paths" and RAC9 enumerates only the five existing types, so both stay true. Say in the PR body that RAC19 satisfies the issue's checkbox. |
| F8 | `MAX_RESTART_CHECK_INTERVAL` = 1 s; `interval` ≤ 1 s, `call_timeout` ≤ `USER_SYSTEMCTL_TIMEOUT`; worst case 6.5 s with `DEFAULT` | Resolved for the upper bound. Residual (non-blocking): there is no lower floor on `call_timeout`. A zero timeout makes every state call fail, which gives "not confirmed running": fail-safe, and production is pinned to `DEFAULT` by F1. With a non-default policy the theoretical maximum is 3 s (probe) + 3 s (`try-restart`) + 10 × (1 s + 3 s) = 46 s, still bounded and off the UI thread. |
| F9 | New `CompanionState::Stopping` (`deactivating`); any `auto-restart*` SubState → `AutoRestart`; a probe of `Stopping` skips the polls | Resolved. It matches `job_type_collapse()`, under which `try-restart` does nothing for inactive, failed and deactivating units. |
| F10 | ANA13 cites the #391 runs; ANA17 holds the sdk-23 filter; this branch's Android PR run id is recorded in the pull request | Accepted. |

### Round 2 adversarial checks
- **Pillar 3 (fail-closed):** if `for_desktop` is implemented as plain `new` (policy forgotten inside the constructor), `for_desktop_confirms_the_restart` fails because it needs probe → `try-restart` → at least one poll. If `parse_companion_state` checks `ActiveState` first, `parse_states` fails on `activating/auto-restart`. PASS.
- **Pillar 6 (test integrity):** no existing test or helper is modified. The new `app.rs` source test adds a constraint without weakening any existing one: `Command::new` stays absent and `FeatureRunner` stays present. PASS.
- **New observations (MINOR, non-blocking):**
  - (a) Optionally scan the whole of `crates/gui/src` for `SystemFeatureExecutor::new(` outside `feature_control.rs`, not only `app.rs`, so that a future second construction site cannot bypass the check.
  - (b) Fix the targetSdk wording from F5.

No CRITICAL or MAJOR finding remains.

VALIDATION_VERDICT: APPROVED
