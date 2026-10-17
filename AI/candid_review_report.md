# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `feat/android-offline-heartbeat`
- **Base (merge-base)**: `aa25f78` (rebased from `6eb0746`; `origin/main` gained #417, #413 and #402)
- **Reviewed-Diff-Fingerprint**: `21d010487a548c839022a99993c04871f8e6eff582fa486b0b587979acc15cc2`
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

## 0. Re-Review After the Rebase Onto `origin/main` `aa25f78`

Independent re-review, without author context, of the branch after its rebase from `6eb0746` onto
`aa25f78`. `origin/main` gained three commits in between: #417 (`085ba63`, zbus 5 -> 6 migration in
`Cargo.toml`/`Cargo.lock`, `crates/daemon/src/presence/logind.rs`, `crates/remote/src/{logind,offline,session}.rs`,
their tests and three invariant files), #413 (`01ce6fa`, lazy second shm buffer of `soos-lock-widget`)
and #402 (`aa25f78`, the phone-companion user phase of `scripts/setup.sh`, merged just before this one).

Rebase integrity, checked mechanically:
- `git range-diff 6eb0746..10e4daf aa25f78..1b6cb07`: commit 1 (`4248d07`, the owner-approved
  ANA-S6 contract migration, formerly `c8dccd1`) is `=`; commit 2 (`1b6cb07`, formerly `10e4daf`) is
  `!` only because of context lines in `AI/DECISIONS.md` (neighbouring ADR lines changed on main) and
  the pre-images of `AI/candid_review_report.md` and `AI/plan_evaluator_report.md`, which main rewrote.
- The `+`/`-` lines of `git diff 6eb0746 10e4daf` and `git diff aa25f78 1b6cb07` (both excluding the
  two report files) are byte-identical (2534 lines), and both report files are byte-identical between
  `10e4daf` and `1b6cb07`. The branch's own changes are therefore unchanged by the rebase.

Semantic interaction with what main gained:
- The branch touches no Rust production crate, no `Cargo.*`, no workflow; #417 changed only Rust
  crates, lock file and invariant files the branch does not edit (`remote_companion_contract.rs`,
  `presence_unlock_contract.rs`, `portable_crates_contract.rs`). The Android app reaches
  `soos-remote` only over HTTPS (`GET /api/status`, unchanged by #417), so the zbus 6 migration does
  not reach it.
- #413 touches `crates/lock-widget` only: no interaction.
- #402 and the branch both edit `Docs/REMOTE_COMPANION.md` and `tests/invariants/src/lib.rs` in
  disjoint hunks (#402: §2j setup note, §4 one-command install, the `setup_user_phase_contract`
  registration; branch: §2j PC offline alerts and owner check step 6, §8, §9, the
  `android_heartbeat_contract` registration). The merged document reads coherently. `scripts/setup.sh`
  installs no Android component and #402 does not touch `scripts/android_build.sh`, `android/` or
  `.github/workflows/android.yml`.

Executed evidence (this review): `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/android-offline-heartbeat
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=4 cargo test --locked
-p soos-invariants -- --test-threads=4`: 735 passed, 0 failed (AHB-S1..S4, the migrated ANA-S6 and main's
new SET20-SET39 together); `cargo clippy --locked -p soos-invariants --all-targets -- -D warnings`: clean;
`cargo fmt --all -- --check`: clean. The JVM unit tests (AHB4-AHB6) cannot run on this host (no JDK/SDK);
no Android input changed in the rebase, and they run in the CI job `Android (debug build)`.

Test assertions: the step-3 greps on the new frozen patch (2853 lines) find exactly the two removed
assertions at patch lines 2776-2777 and the removed `"<service"` needle at line 2822, all in
`test_ana_s6_manifest_hardening`: the owner-approved 2026-10-17 migration recorded in its own commit
`4248d07` and in the ADR item (8) amendment and matrix ANA9. No new `#[ignore]`/`cfg(any())`/`should_panic`/
tolerance, no inline test module change, no other pre-existing assertion changed.

Sections 1-4 below are the previous APPROVED review (fingerprint `62858401…`, base `6eb0746`), re-read
against the rebased code; their findings still apply to the unchanged branch content. Updated
references: the MINOR on branch SHAs now also applies to `c8dccd1` -> `4248d07` (still cited as
`c8dccd1` in the matrix row ANA9, now `AI/VERIFICATION_MATRIX.md:2632`, and in the spec, tester
contract, auditor constraints, plan report and walkthrough); the `invariants` count of section 1 is now
735; the ragged-line SUGGESTION in `Docs/REMOTE_COMPANION.md` now refers to line 1663 (line 1214 is
unchanged; #402 grew the document after it).

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

- **[MINOR]** `AI/VERIFICATION_MATRIX.md:2632` (also
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
- **[SUGGESTION]** `Docs/REMOTE_COMPANION.md:1214,1663` — line 1214 exceeds the document's
  ~100-column wrap and line 1663 leaves a short ragged line; rewrap.

## 5. Final Verdict

**VERDICT: APPROVED**
