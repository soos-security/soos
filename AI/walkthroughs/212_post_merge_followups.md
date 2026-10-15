# Walkthrough 212 — Follow-Ups of the 2026-10-15 Merge Reviews

- **Date**: 2026-10-15
- **Issue**: GitHub-only #398, no backlog id. **Branch**: `fix/post-merge-followups`. It is not registered in `scripts/sync_issue.py`; the commit message carries `Closes #398`.
- **Matrix criteria**: new OBT24, ANA17 and RAC19; ANA13 moved to verified with CI run ids, and the ANA11 note that pointed to it updated.
- **Spec and gates**: `AI/architect_spec_post_merge_followups.md` (§4 Revision 1 wins over §1–§3), `AI/plan_evaluator_report.md` (round 1 REVISION_REQUIRED, round 2 APPROVED), `AI/auditor_constraints_post_merge_followups.md` (CLEARED, 17 constraints).

## 1. Context & Objectives

The candid reviews of three pull requests merged on 2026-10-15 left three non-blocking findings:

1. **#389 (onboarding tour)**: after a `remote.toml` edit, `soos-gui` runs `systemctl --user try-restart soos-remote.service` and reports success. The unit is `Type=exec`, so `try-restart` returns as soon as the new process runs, before `soos-remote` reads its configuration. A refused configuration exits 78, `RestartPreventExitStatus=78` leaves the unit `failed`, and the GUI has already said the change worked.
2. **#391 (Android app)**: `scripts/android_build.sh` `check_permissions` only parsed `uses-permission: name='…'` lines of `aapt2 dump permissions`. A `<uses-permission-sdk-23>` request is printed as `uses-permission-sdk-23: name='…'` and bypassed the allowlist. A local run of the old function against a fake `aapt2` printing `uses-permission-sdk-23: name='android.permission.CAMERA'` printed `[OK]`.
3. **#386 (offline history)**: the README web fixture (`tests/fixtures/mod.rs`, module `readme_web`) did not serve `GET /api/offline`, so web captures (RAC18) could never show the "Offline history" card.

## 2. Architect Design

### Item 1 — restart confirmation
- `companion_state_command()`: the fixed vector `systemctl --user show --property=ActiveState --property=SubState soos-remote.service`.
- `CompanionState` (`Running`, `Changing`, `Stopping`, `AutoRestart`, `Inactive`, `Failed`, `Unknown`) and `parse_companion_state`. A `SubState` starting with `auto-restart` wins over any `ActiveState`; this covers `auto-restart-queued` from systemd 254.
- `RestartCheck { polls, interval, call_timeout }`, `RestartCheck::DEFAULT` = 3 polls, 500 ms apart, 500 ms per call. Clamps: `MAX_RESTART_CHECK_POLLS` 10, `MAX_RESTART_CHECK_INTERVAL` 1 s, call timeout ≤ `USER_SYSTEMCTL_TIMEOUT` (3 s).
- Sequence: write `remote.toml`, probe the state, run `try-restart`. If the probe saw `inactive`, `failed` or `deactivating`, `try-restart` restarted nothing, so the result is `Ok`. Otherwise the executor polls. `failed`, `inactive` or an automatic restart is reported at once. The change counts as confirmed only when the last read is `active`.
- A failed call (error or non-zero exit) is `Unknown` and its output is not parsed. Messages start with `Saved` and carry only a fixed state word.
- **Test-contract constraint.** `feature_control_tests::test_obt_executor_disable_remote_edits_then_restarts` pins the exact runner calls of `SystemFeatureExecutor::new`, so the check cannot live in `new`. It is an explicit policy instead: `with_restart_check`, and `for_desktop` = `new` + `RestartCheck::DEFAULT`. `app.rs` builds `for_desktop`. A source test forbids `SystemFeatureExecutor::new(` anywhere in `crates/gui/src`, so production cannot drop the check silently.

### Item 2 — `aapt2` permission forms
The `sed` pipeline is replaced by a `while read` loop over the dump with the regex `^uses-permission(-sdk-23)?: name='([^']+)'` held in a variable. Every line that starts with `uses-permission` (after optional leading whitespace) must match one of the two forms, or the job fails with `unrecognized aapt2 permission line`. The allowlist, the existing messages and the `INTERNET` requirement are unchanged. `permission:` lines (permissions the app declares) are still ignored. An app with `targetSdk = 36` (35.0.0 is only the build-tools version) has no implied permissions, so `uses-implied-permission` is out of scope.

### Item 3 — offline fixture
`readme_web::route` serves `GET /api/offline` with `{"state":"active","reason":null,"network_monitor":"active","intervals":[…]}`. The intervals are newest first, relative to the capture clock, with saturating arithmetic and `duration_ms = end - start`:
- Scene `unlocked`: one 8 h `sleep`.
- Scene `locked-alerts`: a 12 min `network_lost` and a 40 min `shutdown_or_crash`.

## 3. Plan Evaluation

Round 1 returned REVISION_REQUIRED with 2 MAJOR findings:
- F1: the production path was not pinned by a test. Fixed by `for_desktop` and the source test.
- F2: item 1 had no test plan. Fixed by a sequenced fake runner and the named tests.

There were also 8 MINOR findings. The fixes: SubState prefix, `Stopping`, clamps, saturating fixture times, and the harness defining `APK`. The rest were accepted with reasons: a failed probe still polls, the out-of-scope implied permissions, and RAC19 as a new row because RAC8/RAC9 do not enumerate routes.

Round 2 returned APPROVED. Its notes: correct the "API 35" wording to `targetSdk = 36` (done in the spec), and scan the whole `crates/gui/src` tree in the source test (done).

## 4. Tester Contract (red first)

- `crates/gui/tests/feature_restart_check_tests.rs` (new file, 18 tests, `ScriptedRunner`). Red: it did not compile against the missing API.
- `tests/invariants/src/android_native_contract.rs::test_ana_s13_permission_check_covers_the_sdk_23_form` (appended). It extracts `BUILD_TOOLS_VERSION`, `ALLOWED_PERMISSIONS`, `fail()` and `check_permissions()` verbatim from the script and runs them under `set -euo pipefail` against a fake `aapt2`. It has 10 cases, including trailing `maxSdkVersion` attributes and an indented line. The fake SDK path follows the script's `BUILD_TOOLS_VERSION`. Red on its static assertion.
- `tests/fixtures/tests/readme_fixture_tests.rs::test_rac_web_fixture_serves_the_offline_history` (appended). It checks the keys against `OfflineView` and `OfflineInterval` and the enum variants against `crates/remote/src/offline.rs`, with no identity, `now = 0` saturation, and GET only. Red: `NotFound`.

No existing test, assertion or helper changed. `feature_control_tests.rs` is untouched, and the two other files only gained lines.

## 5. Auditor Constraints

CLEARED with 17 constraints. They include:
- no panic paths;
- the clamps;
- a sleep before each poll (new test `test_obt_restart_check_sleeps_before_each_poll`);
- the output of a failed call is ignored (new test `test_obt_restart_check_ignores_the_output_of_a_failed_call`);
- messages free of `systemctl` output;
- a here-string loop so that `fail` exits the script;
- `local dump` kept separate so `|| fail` sees the `aapt2` status.

## 6. Developer Implementation

- `crates/gui/src/feature_control.rs`: the types and functions above, plus `SystemFeatureExecutor::{for_desktop, with_restart_check}`. `new` is unchanged in behaviour and documented as not confirming.
- `crates/gui/src/app.rs`: `for_desktop`.
- `scripts/android_build.sh`: the loop above.
- `tests/fixtures/mod.rs`: `offline_interval`, `offline_body` and the route.
- Docs: `Docs/GUI_APPLICATION.md` (restart confirmation), `Docs/REMOTE_COMPANION.md` (permission forms, first CI run ids), `Docs/README_ASSETS.md` (offline body).

## 7. Validation

- `cargo test -p soos-gui --test feature_restart_check_tests --test feature_control_tests`: 18 + 43 passed.
- `cargo test -p soos-test-fixtures`: 7 passed in `readme_fixture_tests`.
- `cargo test -p soos-invariants`: 696 passed, matrix citations included.
- `cargo clippy -p soos-gui -p soos-test-fixtures -p soos-invariants --all-targets --all-features -D warnings`: clean.
- The full workspace gate runs in `./save.sh --push-pr`.

ANA13 evidence, verified with `gh run view`:
- Bootstrap run 38446125267 (PR #391, head 8f629a0): failed by design, and the artifact `android-verification-metadata` was uploaded.
- Strict run 38772439975 (head d54e4bd): passed, with `testDebugUnitTest`, `lintDebug`, `BUILD SUCCESSFUL` and the permission check `[OK]`.
- Push run 38773360400 on `main` (3f10f61): passed.

The pull request of this branch touches `scripts/android_build.sh`, so its Android job runs the new filter on real `aapt2` output.

## 8. Owner Checks

- OBT18 (pending, hardware) now also covers the confirmation:
  1. With the companion running, add an unknown key such as `typo_key = true` to `remote.toml` by hand. `soos-remote` uses `deny_unknown_fields`, and the running process does not reread the file.
  2. Turn any key off from the tour. The restart refuses the file (exit 78), and the GUI reports "Saved, but soos-remote stopped after the restart (failed)".
  3. Remove the line and restart the unit.

## 9. Candid Review

The first fresh review returned APPROVED with no critical or major finding. Two of its points were applied before the push, followed by a new review of the new fingerprint:
- The invariant harness now derives the fake `build-tools` directory from the script's `BUILD_TOOLS_VERSION`.
- An indented `uses-permission*` line now fails closed instead of being skipped.

The harness keeps a per-process directory under `std::env::temp_dir()`, because the invariants crate has no dependencies by design.
