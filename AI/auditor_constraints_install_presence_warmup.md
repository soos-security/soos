# Audit Constraints — GitHub #329 (install readiness race, foreign-owned build directory, presence wake settle)

- **Branch**: `fix/install-readiness-and-presence-warmup` (base `origin/main` `b16f567`)
- **Inputs audited**: `AI/architect_spec_install_presence_warmup.md` rev 3 (APPROVED; owner
  defaults: presence-only settle trigger, `PRESENCE_WAKE_SETTLE_MS = 1000`, strict foreign-owner
  check), `AI/plan_evaluator_report.md` (round 3), `AI/tester_contract_install_presence_warmup.md`,
  `tests/invariants/src/install_presence_warmup_contract.rs`,
  `crates/daemon/tests/presence_wake_settle_tests.rs`,
  `crates/daemon/tests/presence_wake_settle_consensus_tests.rs`, and the code about to change:
  `scripts/wait_daemon_ready.sh`, `scripts/install.sh` (A.2/A.3), `crates/daemon/src/inference.rs`
  (`RequestDeadline`), `crates/daemon/src/consensus.rs`, `crates/daemon/src/presence/{mod,worker}.rs`,
  `crates/daemon/src/dispatcher.rs` (must stay unchanged), plus the camera readiness sources
  (`crates/camera-v4l/src/{v4l_impl,capture,mock}.rs`) for the wake race.

## Fail-open analysis (summary)

1. **Can the `not_before` bound produce `Allow` on fewer or other frames than the rules require?**
   No, provided constraints 1–6 hold. A skipped capture is never passed to `aggregator.record`, so
   `Allow` still needs `k = 3` *recorded* consecutive passing captures, all stamped at or after the
   bound, and any recorded spoof still vetoes. Skipping a capture is equivalent to not sampling it;
   the loop already samples sparsely (`latest_frame` polled every `FRAME_POLL_INTERVAL_MS`, so
   intermediate frames are never seen), so "consecutive" already means "consecutive sampled
   captures" today. The bound is fixed before the loop and depends only on the worker clock, never
   on a PAD/match outcome. A frozen camera holding only pre-bound captures ends `Pending` ⇒
   `NoMatch` (fail closed).
2. **Can it be set from the PAM path?** No. The IPC `Request` carries only
   `deadline_monotonic_ns`, which reaches `RequestDeadline::compute` (bound 0). `compute` is the
   only constructor; only `with_not_before` sets the bound and only the presence worker may call
   it (constraint 7). `dispatcher.rs` is unchanged (constraint 8).
3. **Arithmetic / clocks**: `start_ns + settle_ns` saturates to `u64::MAX` ⇒ every capture skipped
   ⇒ `Pending` (fail closed); `compute(now_ns = u64::MAX)` saturates and the `Instant` outer bound
   (1900 ms) still ends the loop. Capture stamps (`capture.rs` `clock_ns`, `CLOCK_MONOTONIC`) and the
   worker `clock_fn` share one domain; the test `SpyCamera` restamps with the test clock.
4. **Deadline budget**: the woken window is `compute(not_before.max(start), 0, now,
   DECISION_BUDGET_MS + RESPONSE_WRITE_MARGIN_MS + settle)`: client bound = bound + 900 ms, outer
   bound = now + 1900 ms; both finite. The non-woken window is bit-identical to today's.
   `can_start*`/`remaining*` ignore the bound; skipped captures never reach admission, so the
   estimate-decay path (`decay_estimate` when `frames_evaluated() == 0`) is not triggered by
   skips (constraint 3).
5. **Camera ready-flag race** (`V4lCameraManager::is_ready` = `!idle_expired && ready`):
   (a) idle timeout already elapsed but the capture loop has not suspended yet ⇒ `woke = true`, a
   settle is applied to a stream that kept running: only ~1 s slower (safe). (b) idle timeout
   crossed between the `is_ready()` sample and `notify_activity()` ⇒ `woke = false`, the stream
   resumes, wake frames may be evaluated ⇒ possible veto as today (fail-safe; same as spec §5.4
   residual (a)). Neither direction can produce an allow.

## Audit Constraints — Issue #329

| # | Constraint | Applies to (file::fn) | Verified by (test / lint / invariant / grep) |
|---|---|---|---|
| 1 | The lower-bound filter is `frame.timestamp_mono_ns < deadline.not_before_ns()` with **no** `timestamp > 0` exemption (unlike `is_frame_fresh`): a capture stamped 0 with a bound > 0 is skipped, never treated as settled; with bound 0 nothing is skipped. | `consensus.rs::run_face_consensus` | review of the diff; `test_iwp_not_before_zero_is_the_pam_consensus`, `test_iwp_not_before_skips_earlier_captures` |
| 2 | The filter runs right after `last_sequence = Some(frame.sequence)` and **before** the freshness check, the Background preemption check, the admission (`can_start`), `decay_estimate`, `acquire_*`/`try_acquire_background`, `process_frame` and `aggregator.record`. A skipped capture leaves `last_capture_stale`, the aggregator and `spoof_capture` untouched and holds no inference permit. | `consensus.rs::run_face_consensus` | review; `test_iwp_not_before_past_the_deadline_fails_closed` (0 inference, 0 PAD), IWP8 (`inferences() == 3`) |
| 3 | A skipped capture never calls `ctx.inference.decay_estimate()` (a settle must not decay the shared latency estimate used by PAM admission). | `consensus.rs::run_face_consensus` | review / `grep -n decay_estimate crates/daemon/src/consensus.rs` (single existing call site, inside the admission branch) |
| 4 | No other consensus behaviour changes: no aggregator reset, no veto downgrade, no change to `PadAggregator::with_defaults`, `k`, thresholds, `FrameEvaluation` mapping, freshness or `MAX_FRAME_AGE_NS`. `run_face_consensus` keeps its signature; the consensus never names `PRESENCE_WAKE_SETTLE*`. | `consensus.rs` | `test_iwp_settle_is_presence_only`, `presence_unlock_contract::test_pau_pad_consensus_is_built_only_in_consensus_rs`, IWP9 tests, full `presence_consensus_tests` |
| 5 | `RequestDeadline` gains exactly one private `not_before_ns: u64` field; `compute` sets it to 0; `with_not_before(self, ns) -> Self` is `const`, `#[must_use]`, changes only that field; `not_before_ns()` is a `const` getter. `deadline_ns`, `remaining*`, `is_expired_at`, `can_start*` ignore it. No public constructor other than `compute`. | `inference.rs::RequestDeadline` | `test_iwp_not_before_zero_is_the_pam_consensus` (`with_not_before(0) == deadline`), review |
| 6 | Worker settle arithmetic is overflow-free: `PRESENCE_WAKE_SETTLE_MS` → ns via `saturating_mul(1_000_000)` (or `Duration` + `u64::try_from(..).unwrap_or(u64::MAX)`), bound via `saturating_add`, budget via `saturating_add`. No `as` casts that truncate, no `unwrap`/`expect`, no indexing. | `presence/worker.rs::scan` | `cargo clippy --locked -p soos-daemon --all-targets --all-features -- -D warnings` (workspace `arithmetic_side_effects`, `unwrap_used`) |
| 7 | `woke` is sampled with `camera.is_ready()` **before** `notify_activity()`; the bound uses the worker's `self.clock_fn` (same as `ctx.clock`), never `Instant` or a raw clock read; when `woke == false` the deadline is exactly today's (`compute(start_ns, 0, now, DECISION_BUDGET_MS + RESPONSE_WRITE_MARGIN_MS)`, `with_not_before` not called or called with 0). `with_not_before` is called only in `presence/worker.rs`. | `presence/worker.rs::scan` | IWP8 (sampling after `notify_activity` sees a ready mock ⇒ red), IWP10; `grep -rn with_not_before crates/*/src` ⇒ only `inference.rs` (definition) and `presence/worker.rs` |
| 8 | `crates/daemon/src/dispatcher.rs`, `crates/daemon/src/config.rs` (`DAEMON_DEFAULT_WARMUP_FRAMES = 0`), `crates/protocol`, `crates/pam`, `crates/camera-v4l` are not modified. | — | `git diff --exit-code origin/main -- crates/daemon/src/dispatcher.rs crates/daemon/src/config.rs crates/protocol crates/pam crates/camera-v4l`; `test_iwp_settle_is_presence_only`; `warmup_default_tests` |
| 9 | `pub const PRESENCE_WAKE_SETTLE_MS: u64 = 1000;` is defined once, in `presence/mod.rs`, with a doc comment; not configurable (no `PresenceConfig` field, no TOML key). | `presence/mod.rs` | `test_iwp_settle_constant_value`, `test_iwp_settle_is_presence_only`; `grep -rn PRESENCE_WAKE_SETTLE_MS crates/*/src` |
| 10 | Logging: at most one `debug!` per woken scan carrying only `settle_ms`; no frame, embedding, score, similarity, user name or timestamp field; no new `info!/warn!/error!` in consensus or presence. The message must not contain field keys from the forbidden list (`frame`, `data`, `score`, ...). | `presence/worker.rs`, `consensus.rs` | `presence_unlock_contract::test_pau_presence_logs_no_biometric_field_above_debug`; review |
| 11 | `wait_daemon_ready.sh`: every command whose failure is expected (the status attempt, `find`, `timeout`) is evaluated in an `if`/`||` context so `set -euo pipefail` never aborts the script mid-poll (`x="$(false)"` exits under `set -e`). No pipe between the attempt and its capture (pipefail/SIGPIPE would mask the exit code). | `scripts/wait_daemon_ready.sh` | IWP1–IWP3, `installer_templates_contract::test_wait_daemon_ready_*`; `bash -n`; review |
| 12 | Deadline arithmetic uses only `SECONDS` **deltas** (`start=$SECONDS` before the socket wait; `SECONDS` may be inherited from the environment) — never `date`. The loop is do-while: at least one attempt once the socket exists; the stop test `SECONDS - start >= TIMEOUT_S + 1` runs after each failed attempt, before sleeping. Exit codes stay 0/1/2; `Exit codes: 0 ready, 1 not ready, 2 usage error.` stays verbatim in the header. | `scripts/wait_daemon_ready.sh` | IWP1, IWP2 (≥ 3 attempts in 2 s), IWP4, existing `test_wait_daemon_ready_fails_closed_and_bounded` |
| 13 | Per-attempt bound: `timeout --kill-after=1 5 "${ADMIN_BIN}" --format json --socket-path "${SOCKET_PATH}" status` (no `--foreground`). Use it only after a one-time capability probe (e.g. `timeout --kill-after=1 5 true >/dev/null 2>&1`) succeeds; otherwise run unwrapped and print one degraded-mode warning on **stderr** (a `timeout` without `--kill-after`, e.g. busybox, would otherwise exit 125 on every attempt and fail every install with exit 70). Exit 124/125/126/127/137 count as failed attempts. | `scripts/wait_daemon_ready.sh` | IWP3 (`timeout` present); review of the probe |
| 14 | stdout discipline: an attempt that exited 124 or 137 contributes no stdout; on success print exactly the captured stdout of the successful attempt once (`printf '%s\n'`), on timeout the last *completed* attempt's stdout once, else nothing; all diagnostics on stderr. Earlier reports are never printed. | `scripts/wait_daemon_ready.sh` | IWP2 (both tests), IWP3 (`partial` never printed); `systemd_unit_acceptance_test.sh` `$(...)` capture |
| 15 | No temporary file, no write anywhere, no new privileged action in `wait_daemon_ready.sh`. If the last attempt's stderr is shown on timeout, capture it in memory; if a file is unavoidable it must come from `mktemp` (O_EXCL, 0600) and be removed by an `EXIT` trap — never a predictable path (the helper runs as root). | `scripts/wait_daemon_ready.sh` | `grep -nE 'mktemp|>\s*/tmp|tee ' scripts/wait_daemon_ready.sh`; review |
| 16 | `check_build_target_dir` is strictly read-only: it never runs `chown`, `chmod`, `rm`, `mkdir`, `runuser`, `cargo`, `check_build_deps.sh`, and never `sudo`; the `chown`/`chmod` lines are printed advice only. It reports through `preflight_fail` (no direct `exit`), runs only under `--build`, before the A.3 `DO_BUILD` block (so the existing `PREFLIGHT_ERRORS -eq 0` guard skips `run_release_build`) and before the `[DRY-RUN] Would run:` line. | `scripts/install.sh` A.3 | IWP5 (exit 2, empty call log, nothing staged), IWP5 dry-run, IWP6; `grep` of the function body |
| 17 | The frozen strings of spec §4.3 stay byte-identical and `run_release_build` keeps the root-without-invoking-user refusal (`return 1` ⇒ exit 40). `resolve_build_user` must not change who builds. | `scripts/install.sh::run_release_build` | `arch_faillock_ci_contract::test_install_build_exports_the_prefix_bindir`, `packaging_ownership_contract::test_install_build_as_root_drops_to_invoking_user_or_refuses`, IWP7 |
| 18 | The build user's uid is resolved with `id -u -- "${user}"` (the `root_shims` `id` only fakes the single-argument `id -u`), captured in an `if`/`||` context, and must match `^[0-9]+$` before it is passed to `find -uid`; non-resolution ⇒ pass silently (spec §4.2 row 4); uid 0 ⇒ pass. The current-user case uses `id -un`. | `scripts/install.sh::resolve_build_user`, `check_build_target_dir` | IWP5 (under the shim, `nobody` must resolve to 65534, not 0), IWP7 |
| 19 | `find` invocation: `find -H <T> ! -uid "${uid}" -print -quit` and `find -H <T> -type d ! -perm -u+w -print -quit` (or one combined walk), where `<T>` can never be parsed as an option or predicate (absolute path, or `./`-prefixed when relative) and is always double-quoted (paths with spaces, globs). No `-L`, no `head` pipe (SIGPIPE under pipefail). The exit status is captured explicitly; `find` non-zero with no hit (permission denied, filesystem loop) ⇒ preflight error naming `<T>` with the fix (fail closed). The walk is not a security control: nothing is authorized from its result. | `scripts/install.sh::check_build_target_dir` | IWP5, IWP6; review; `shellcheck --severity=error` (CI lint job) |
| 20 | `<T>`: absent and not a symlink ⇒ pass; a dangling symlink, or an existing non-directory ⇒ preflight error `The cargo target directory <T> is not a directory.` (fail closed). | `scripts/install.sh::check_build_target_dir` | review |
| 21 | Every untrusted string echoed by the root installer — `<T>`, the first offending path from `find`, and the user name inside the suggested command — is printed through `printf '%q'` (neutralises spaces, newlines and terminal escape sequences in user-controlled file names; keeps the suggested command paste-safe). Plain paths print verbatim, as the tests expect. | `scripts/install.sh::check_build_target_dir` | IWP5/IWP6 exact-string assertions; review |
| 22 | `find` is added to the A.2 required-tool check only when `--build` is given (the existing list stays unchanged otherwise). | `scripts/install.sh` A.2 | review; existing `installer_contract` restricted-PATH tests |
| 23 | No new dependency, crate, feature, `unsafe`, `println!/eprintln!` in daemon code; `#![forbid(unsafe_code)]` in `consensus.rs` unchanged. | daemon crate | `cargo deny --locked check` unchanged inputs; `grep -n unsafe` on changed files; clippy |
| 24 | Test integrity: no existing test, assertion or fixture (`crates/daemon/tests/common/mod.rs`, `packaging_ownership_contract::root_shims`, `installer_contract`) is modified; the 20 new tests are not edited by the developer. | all | `git diff origin/main --stat -- tests crates/*/tests` ⇒ only the 3 new files and the `mod` line in `tests/invariants/src/lib.rs` |
| 25 | Gate: `cargo fmt --all -- --check`; `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`; full `cargo test --locked -p soos-daemon --all-features --all-targets` and `-p soos-invariants`; the three contract commands of the tester contract run 10× without a flake; `bash -n` + `shellcheck` on both scripts. | all | CI Quality job + local run |

### Pre-existing violations / risks found (not introduced by this change)

- **P-1 (install.sh, pre-existing)**: a relative `CARGO_TARGET_DIR` is resolved against the
  installer's cwd for `DEFAULT_ARTIFACT_DIR` (and now the check), but against `WORKSPACE_ROOT` by
  cargo (`cd "$1"` in the `runuser` login shell). `sudo`'s `env_reset` usually strips
  `CARGO_TARGET_DIR`, and `runuser ... bash -lc` may let the user's profile override it, so the
  installer's target and cargo's target can differ. Fail-safe (the A.4 artifact check catches a
  missing build); out of scope, worth a follow-up issue.
- **P-2 (wait_daemon_ready.sh)**: `soos-admin status` runs `systemctl show` without a timeout;
  covered for this helper by constraint 13 only (the admin CLI itself is unchanged).
- No `unwrap/expect/panic!/todo!/unreachable!` and no `unsafe` in the production code of
  `consensus.rs`, `inference.rs`, `presence/{mod,worker}.rs` (grep clean).

### Findings for the tester / architect (non-blocking)

- **T1 (tester, residual)**: IWP5's fixture makes the *whole* target tree foreign to `nobody`, so
  an implementation that only `stat`s `<T>` itself would pass it; the real-world case (a user-owned
  `target/` with root-owned files below) cannot be built without root. Covered by constraint 19
  (review/grep for the recursive `find -H ... ! -uid`). Optional extra: a static invariant asserting
  `install.sh` contains a `find -H` walk with `! -uid` and `-print -quit`.
- **T2 (tester, residual)**: the `timestamp_mono_ns == 0` sentinel (constraint 1) is untested
  because every test camera stamps captures; enforced by review.
- **T3 (tester, residual)**: the degraded paths (no `timeout`, `timeout` without `--kill-after`)
  are untested; enforced by review of constraint 13.
- **A1 (architect)**: spec §4.2 says the run stops "before `check_build_deps.sh`"; in `--dry-run`
  the existing branch runs the read-only `check_build_deps.sh`. Running it after a target-directory
  error is harmless (read-only, invokes no `cargo`) and IWP5 dry-run allows it; the developer may
  keep it.
- **A2 (architect)**: constraint 13's capability probe is a small addition to spec §3.2 (the spec
  only tests that `timeout` is on `PATH`); it keeps the documented degraded path reachable on hosts
  whose `timeout` lacks `--kill-after` and changes no tested behaviour.
- **Test power check**: the new tests bite: IWP1/IWP2/IWP3/IWP5/IWP6/IWP8/IWP9/IWP11/IWP12/IWP13 are
  red on the base for the right reason (tester contract evidence); the three green-on-base tests
  (IWP4 static, IWP7 no-false-positive, IWP10 no-settle-when-streaming) each kill a named wrong
  implementation. Skips (no `nobody`, root runner, no `timeout`) do not apply on the CI Quality
  runner. No existing test is modified (`git status`: only new files plus the `mod` line).

### Clearance: CLEARED
