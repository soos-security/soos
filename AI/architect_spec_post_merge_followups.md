# Architect Spec — Post-Merge Follow-Ups of 2026-10-15 (GitHub #398)

- **Issue**: GitHub #398 (no backlog id; review findings of #389, #391 and #386)
- **Branch**: `fix/post-merge-followups` (GitHub-only review-finding branch, not registered in
  `scripts/sync_issue.py`; the commit carries `Closes #398`)
- **Base**: `origin/main` 640e1db
- **Walkthrough**: 212
- **Matrix**: new rows OBT24, ANA17, RAC19; ANA13 (and the ANA11 JVM-test note that points to it)
  moved to verified with CI evidence. Existing rows OBT12, RAC8 and RAC9 are left unchanged.

## 0. Blast Radius

| Item | Files | Consumers |
|---|---|---|
| 1 | `crates/gui/src/feature_control.rs`, `crates/gui/src/app.rs` (construction only) | `SystemFeatureExecutor` is built only in `app.rs` and in `crates/gui/tests/feature_control_tests.rs` |
| 2 | `scripts/android_build.sh` (`check_permissions` only) | CI job `Android (debug build)` (`.github/workflows/android.yml`, path filter includes the script) |
| 3 | `tests/fixtures/mod.rs` (`readme_web::route`) | `tests/fixtures/examples/readme_web_fixture_server.rs`, `scripts/capture_readme_web.sh`, `tests/fixtures/tests/readme_fixture_tests.rs` |

No crate gains a dependency. No PAM, daemon, IPC, protocol or biometric path is touched.

## 1. Item 1 — `soos-gui`: confirm the companion after `try-restart`

### Problem
`DisableRemote` edits `remote.toml` and runs `systemctl --user try-restart soos-remote.service`
(3 s deadline). The unit is `Type=exec`: `try-restart` returns once the new process is executed,
before `soos-remote` parses its configuration. A refused configuration exits `EXIT_CONFIG` 78,
`RestartPreventExitStatus=78` leaves the unit `failed`, and the GUI has already reported success.
A runtime exit (status 1) puts the unit in `activating`/`auto-restart` (`Restart=on-failure`).

### Test-contract constraint
`feature_control_tests::test_obt_executor_disable_remote_edits_then_restarts` asserts that an
executor built with `SystemFeatureExecutor::new` makes exactly one runner call (the `try-restart`)
for a `DisableRemote` edit. That assertion is immutable. The state check is therefore an explicit,
opt-in policy of the executor: `new` keeps its current behaviour and call sequence; the production
GUI builds its executor with `with_restart_check(RestartCheck::DEFAULT)`. The new tests use the
same constructor as production.

### API (all in `crates/gui/src/feature_control.rs`)

```rust
/// Most state polls of a restart check (larger values are clamped).
pub const MAX_RESTART_CHECK_POLLS: u32 = 10;

/// `systemctl --user show --property=ActiveState --property=SubState soos-remote.service`.
pub fn companion_state_command() -> (&'static str, Vec<String>);

/// State of the companion unit as read by a restart check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionState {
    /// `ActiveState=active` (and not `SubState=auto-restart`).
    Running,
    /// `activating`, `reloading`, `refreshing` or `deactivating` (not `auto-restart`).
    Changing,
    /// `SubState=auto-restart` (any `ActiveState`): the service exited and systemd waits to
    /// restart it (`Restart=on-failure`).
    AutoRestart,
    /// `ActiveState=inactive`.
    Inactive,
    /// `ActiveState=failed` (e.g. exit 78, never restarted).
    Failed,
    /// Missing or unrecognised `ActiveState`.
    Unknown,
}

/// Maps `ActiveState=` / `SubState=` lines (any order, values trimmed, other lines ignored).
pub fn parse_companion_state(text: &str) -> CompanionState;

/// How a restart of the companion is confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartCheck {
    /// State polls after the restart (0 is treated as 1, values above
    /// `MAX_RESTART_CHECK_POLLS` are clamped).
    pub polls: u32,
    /// Pause before each poll.
    pub interval: Duration,
    /// Deadline of each `systemctl --user show` call (also the pre-restart probe).
    pub call_timeout: Duration,
}

impl RestartCheck {
    /// Production policy: 3 polls, 500 ms apart, 500 ms per call
    /// (worst case 0.5 s probe + 3 × (0.5 s + 0.5 s) = 3.5 s; typical about 1.5 s).
    pub const DEFAULT: RestartCheck;
}

impl SystemFeatureExecutor {
    /// `new` plus a confirmation of the companion state after each `remote.toml` restart.
    #[must_use]
    pub fn with_restart_check(self, check: RestartCheck) -> Self;
}
```

### Behaviour of `disable_remote` with a check (unchanged without one)
1. Refusals (root, no config dir, missing file, unsupported layout, symlink) and the no-op edit are
   unchanged: no program runs.
2. Write `remote.toml` atomically (unchanged).
3. **Probe** once: `companion_state_command()` with `Some(check.call_timeout)`,
   `max_stdout = MAX_UNIT_SHOW_BYTES`. A failed call or non-zero exit is `Unknown`.
4. `try-restart` exactly as today (`USER_SYSTEMCTL_TIMEOUT`); its failure keeps today's error
   `"Saved, but soos-remote did not restart: …"` and no poll runs.
5. If the probe was `Inactive` or `Failed`, `try-restart` restarted nothing: return `Ok(())`
   without polling (the edit applies at the next start).
6. Otherwise (`Running`, `Changing`, `AutoRestart`, `Unknown`) poll up to `polls` times
   (clamped to `1..=MAX_RESTART_CHECK_POLLS`): sleep `interval`, run the state command
   (same deadline and bound).
   - `Failed`, `Inactive` or `AutoRestart` → return at once
     `Err("Saved, but soos-remote stopped after the restart (<state>): see journalctl --user -u soos-remote")`
     where `<state>` is `failed`, `inactive` or `restarting`.
   - `Running` → remember; continue.
   - `Changing`, `Unknown`, call error → remember not running; continue.
7. After the last poll: `Running` → `Ok(())`; otherwise
   `Err("Saved, but soos-remote was not confirmed running after the restart")`.

Every error string starts with `Saved` (the file is written). No poll happens on the UI thread
(`FeatureRunner` worker). Total time is bounded by `1 + polls` calls of `call_timeout` each plus
`polls × interval`. Nothing read from systemd is logged; the error carries only a fixed state word.

### Production wiring
`app.rs`: `SystemFeatureExecutor::new(FeaturePaths::system(), Arc::new(SystemProgramRunner)).with_restart_check(RestartCheck::DEFAULT)`.

## 2. Item 2 — `scripts/android_build.sh` permission filter

`aapt2 dump permissions` prints `uses-permission: name='X'` and, for a `<uses-permission-sdk-23>`
element, `uses-permission-sdk-23: name='X'` (both may carry trailing attributes such as
`maxSdkVersion='N'`). Today only the first form reaches the allowlist.

New loop (allowlist, messages of the existing checks and the INTERNET requirement unchanged):

```bash
local line name allowed permission found=0
local re="^uses-permission(-sdk-23)?: name='([^']+)'"
while IFS= read -r line; do
    [[ "$line" == uses-permission* ]] || continue
    [[ "$line" =~ $re ]] || fail "unrecognized aapt2 permission line: ${line}"
    name="${BASH_REMATCH[2]}"
    # allowlist check as today; found=1 for android.permission.INTERNET
done <<< "$dump"
```

Any line starting with `uses-permission` that is not exactly one of the two forms fails closed.
Lines such as `package:` or `permission:` (a permission the app declares) are not requests and are
ignored as today.

Invariant (new test in `tests/invariants/src/android_native_contract.rs`): extract the
`ALLOWED_PERMISSIONS` array, `BUILD_TOOLS_VERSION`, `fail()` and `check_permissions()` from the
script, run them with `bash` against a fake `aapt2` in a temporary `ANDROID_HOME`, and assert:
allowlisted plain + sdk-23 lines pass; a non-allowlisted name in the sdk-23 form fails naming it; a
non-allowlisted plain name fails; an unknown `uses-permission-*` form fails; an output without
INTERNET fails. Plus a static check that the script names the sdk-23 form.

ANA13 evidence: bootstrap run 38446125267 (PR #391 head 8f629a0, failed by design, artifact
`android-verification-metadata` uploaded); strict green run 38772439975 (PR #391 head d54e4bd,
`testDebugUnitTest`, `lintDebug`, `BUILD SUCCESSFUL`, permission check `[OK]`) and the push run on
`main` 38773360400.

## 3. Item 3 — README web fixture `GET /api/offline`

`readme_web::route` serves `GET /api/offline` → `200` JSON with exactly the keys of
`soos-remote` `OfflineView` (`state`, `reason`, `network_monitor`, `intervals`) and, per interval,
of `OfflineInterval` (`reason`, `start_unix_ms`, `end_unix_ms`, `duration_ms`); values from the
real snake_case enums (`OfflineState::Active`, `NetworkMonitor::Active`, `OfflineReason`). Intervals
newest first, `start <= end`, `duration_ms == end - start`, all relative to `now_unix_ms`
(saturating), no identity.

- `Unlocked`: `active`, monitor `active`, one `sleep` interval (8 h, ended 1 h ago).
- `LockedWithAlerts`: `active`, monitor `active`, `network_lost` (12 min, ended 30 min ago) then
  `shutdown_or_crash` (40 min, ended 1 day ago).

Only `GET` (other methods stay `NotFound`). Tests (new, `readme_fixture_tests.rs`): served as
`Json{200}` for both scenes, keys equal the remote structs (read from `crates/remote/src/offline.rs`
with the existing `struct_fields` helper), reasons and states are serialized variant names of the
remote enums, order/arithmetic, no identity token, scenes differ, `HEAD`/`POST` not found.
`Docs/README_ASSETS.md` lists the offline body.

## 4. Revision 1 (plan evaluation of 2026-10-15, findings F1–F10)

Where this section and §1–§3 differ, this section wins.

- **F1 (production path pinned).** New constructor
  `pub fn for_desktop(paths: FeaturePaths, runner: Arc<dyn ProgramRunner>) -> Self` =
  `new(paths, runner).with_restart_check(RestartCheck::DEFAULT)`. `app.rs` uses it; the doc of
  `new` states that it does not confirm the restart. Tests: a behavioural test of `for_desktop`
  (probe → `try-restart` → at least one poll, `DEFAULT` deadlines) and a source test over
  `include_str!("../src/app.rs")`: no `SystemFeatureExecutor::new(`, exactly one
  `SystemFeatureExecutor::for_desktop(`.
- **F2 (test plan).** New sequenced fake runner `ScriptedRunner` in `feature_control_tests.rs`
  (records `(program, args, deadline, max_stdout)`; answers each call from a queue of
  `Result<ProgramOutput, String>`, success with empty output when the queue is empty). New tests
  (prefix `test_obt_restart_check_`):
  `parse_states` (table incl. `SubState=auto-restart` and `auto-restart-queued` winning over any
  `ActiveState`, `deactivating`, unknown, empty, reversed line order),
  `state_command_is_a_fixed_user_vector`,
  `confirms_a_running_unit` (call order probe/try-restart/polls, deadlines, `MAX_UNIT_SHOW_BYTES`),
  `skips_polls_when_the_unit_was_not_running` (probe `inactive`, `failed`, `deactivating`: exactly
  2 calls, `Ok`),
  `reports_a_unit_that_fails_after_running` (Running then Failed on a later poll → error naming
  `failed`, polling stops there),
  `reports_auto_restart_and_inactive`,
  `reports_an_unconfirmed_unit` (only `activating` / unknown / call errors → "not confirmed"),
  `clamps_polls_interval_and_timeout` (0 → 1 poll, 1000 → 10 polls, call timeout above the bound
  clamped),
  `restart_failure_runs_no_poll`,
  `default_policy_values`,
  `errors_carry_no_systemctl_output` (a poll output with a marker string never reaches the error),
  `for_desktop_confirms_the_restart`, `app_builds_the_desktop_executor`.
- **F3 (failed probe).** A failed or unparsable probe still leads to polling (conservative: the
  unit may be running). If the unit was in fact stopped, the GUI reports "stopped after the restart
  (inactive)", which is a true statement about the unit; documented in `Docs/GUI_APPLICATION.md`.
- **F4 (harness).** The invariant harness extracts, by text anchors, `readonly BUILD_TOOLS_VERSION=`
  (one line), `readonly ALLOWED_PERMISSIONS=(` up to the next line `)`, `fail() {` and
  `check_permissions() {` each up to the next line `}`; it defines `APK` (an existing temporary
  file) and `ANDROID_HOME`, and runs under `set -euo pipefail`. `check_permissions` keeps
  `local dump`. Cases include a trailing `maxSdkVersion='28'` attribute on both forms.
- **F5 (prefix).** Out of scope by design: only lines that start with `uses-permission` are
  requests of the merged manifest; `permission:` lines are declarations and
  `uses-implied-permission` is not printed for an app targeting API 36 (`targetSdk = 36`; 35.0.0 is the build-tools version; implied permissions only
  exist for very old target SDKs). Failing on every `uses-` line could reject a valid build on a
  future aapt2 without a security gain; every `uses-permission*` line is either one of the two
  exact forms or fails closed.
- **F6 (fixture).** `reason` is `null` in both scenes. Every wall time is computed with
  `saturating_sub` from `now_unix_ms` and `duration_ms = end - start` (so `route(.., 0)` stays
  consistent: all zero). The new test applies the no-identity token list to `/api/offline` too.
- **F7 (matrix).** RAC8 and RAC9 do not enumerate routes (RAC8 says "the fixed `GET` API paths",
  RAC9 lists types), so they stay unchanged and the new route is the new row RAC19.
- **F8 (bounds).** `MAX_RESTART_CHECK_INTERVAL` = 1 s; `interval` and `call_timeout` are clamped to
  `≤ MAX_RESTART_CHECK_INTERVAL` and `≤ USER_SYSTEMCTL_TIMEOUT` respectively. Worst case with
  `DEFAULT`: probe 0.5 s + `try-restart` 3 s + 3 × (0.5 s + 0.5 s) = 6.5 s, on the worker thread;
  typical about 1.5 s.
- **F9 (systemd).** `CompanionState` gains `Stopping` (`ActiveState=deactivating`, not
  auto-restart); any `SubState` starting with `auto-restart` is `AutoRestart`. A probe of
  `Inactive`, `Failed` or `Stopping` skips the polls. During polls, `Stopping` is "not running,
  keep polling" (final: not confirmed).
- **F10 (ANA13).** ANA13 is about the first CI runs of the foundation PR and cites #391's runs.
  The sdk-23 filter is ANA17 (invariant test); the Android job of this branch's pull request
  exercises the new filter on real `aapt2` output and its run id is recorded in the pull request.
