# Architect Spec — Install Readiness Race, Foreign-Owned Build Directory, Presence Wake Settle

- **Issue**: GitHub #329 (GitHub-only, no `AI/BACKLOG.md` entry; the commit carries `Closes #329`)
- **Branch**: `fix/install-readiness-and-presence-warmup` (not registered in `scripts/sync_issue.py`)
- **Base commit**: `b16f567`
- **Revision**: 3 (round-1 findings P1–P5 and round-3 finding P6 addressed, §10)
- **Evidence** (owner's Arch host, 2026-10-03): see the issue; summarised per defect below.

## 1. Root Causes (read from the code)

### 1.1 D1 — the readiness wait fails on the first unhealthy answer

`scripts/wait_daemon_ready.sh` polls **only for the socket file** (step 2: `while [[ ! -S sock ]]`,
two polls per second, `--timeout` bound), then runs `soos-admin --format json --socket-path <sock> status`
**exactly once** (step 3) and exits 1 when it fails. `soos-admin status` (`crates/admin-cli/src/main.rs`,
`Commands::Status`) prints the JSON report and then `std::process::exit(1)` whenever
`report.is_healthy` is `false`; the daemon reports `is_healthy = false` while the camera is
`starting` (`camera_ready = false`). After `systemctl restart` the socket exists as soon as `READY=1`
is sent (`Type=notify`), i.e. before the first camera frame, so the single status query ran at
uptime 0 s, printed `"is_healthy": false` and failed; `install.sh` step 9 mapped that to exit 70.
Nothing waits for *health*: the bound only ever covered the socket. CI never saw it because the
Docker systemd harness uses the mock camera, which is ready at once.

### 1.2 D2 — a root-owned `target/` breaks `install.sh --build`

`run_release_build` (A.3) builds as the invoking user (`runuser -u <user>`) since GitHub #308, but
never checks that this user can write the cargo target directory. A `target/` (or
`target/release`) left by an older **root** build holds root-owned files, and cargo fails late with
`failed to open: <repo>/target/release/.cargo-build-lock: Permission denied`; the installer then
prints `Release build failed` (exit 40) without saying how to fix it.

### 1.3 D3 — the first presence scan after a camera wake evaluates auto-exposure frames

`PresenceWorker::scan` (step 11) calls `notify_activity()`, waits for `is_ready()`
(`wake_camera`, ≤ `MAX_CAMERA_WAKE_WAIT_MS`), then runs `run_face_consensus` immediately. On a
resume from auto-standby the real capture loop (`camera-v4l/src/capture.rs::run_capture_loop`)
publishes the first frame as soon as `warmup_frames` (daemon default **0**, ADR 2026-09-30
"One Effective Daemon `warmup_frames` Default", DMN-16: the PAM instant-wake contract) are
discarded, i.e. at once. The first ~0.5 s of frames are under/over-exposed while the sensor's
auto-exposure converges; MiniFASNet classifies such frames as spoof and the `PadAggregator`
vetoes the whole scan on the first spoof capture (evidence: resume at 22:44:28.046, veto at
28.493 with `captures_evaluated=2`; the next scan 2 s later, camera still streaming, unlocked with
3 captures). The camera then stays streaming between scans (`idle_timeout` 10 s > `scan_interval`
2 s, each scan calls `notify_activity`), so only the **first scan after a wake** is affected.

## 2. Scope & Blast Radius

| Area | Files | Consumers |
|---|---|---|
| D1 | `scripts/wait_daemon_ready.sh` | `scripts/install.sh` step 9 (unchanged call, exit 70 kept), `tests/docker/systemd_unit_acceptance_test.sh` (captures stdout with `$(...)` and greps `"is_healthy": true` — keeps working: stdout carries exactly one JSON document), `installer_templates_contract` (existing readiness tests stay valid, §6.3) |
| D2 | `scripts/install.sh` (A.3 only) | `packaging_ownership_contract::test_install_build_as_root_drops_to_invoking_user_or_refuses` (stays valid, §6.3), `arch_faillock_ci_contract` (`--build` under shims) |
| D3 | `crates/daemon/src/inference.rs` (`RequestDeadline::with_not_before`), `crates/daemon/src/consensus.rs` (lower-bound filter), `crates/daemon/src/presence/mod.rs` (new const), `crates/daemon/src/presence/worker.rs` (step 11/12) | dispatcher (unchanged: keeps `run_face_consensus`), presence tests |
| Docs | `Docs/PACKAGING_AND_PROVISIONING.md` (readiness, `--build` ownership hint), `Docs/DAEMON.md` (presence step 10), `AI/DECISIONS.md` (ADR) | — |

No IPC, protocol, PAM module, policy, vision, camera-v4l or config-schema change. No new
dependency. `crates/pam` untouched (no PAM deadline impact).

## 3. D1 — Bounded Health Poll in `scripts/wait_daemon_ready.sh`

### 3.1 Behaviour

1. Option parsing, `--timeout` validation (1..300, exit 2) and the **immediate** missing-manifest
   failure (exit 1, names `download_models.sh`, no status query) are unchanged.
2. One budget covers the whole wait. The socket wait keeps its existing poll counter
   (`TIMEOUT_S * 2` polls of 0.5 s). The health poll uses the bash `SECONDS` builtin
   (`start=$SECONDS` taken before the socket wait; never `date`): after each **failed** attempt,
   and before sleeping, the poll stops when `SECONDS - start >= TIMEOUT_S + 1` (the `+ 1` absorbs
   the whole-second granularity so a `--timeout 1` wait is never shorter than 1 s). Worst case:
   `TIMEOUT_S + 1 + ATTEMPT_TIMEOUT_S + 1` seconds (§3.2).
3. Socket wait: unchanged semantics (`-S`, 0.5 s poll); a path that is not a Unix socket never
   triggers a status query. Expiry: the existing message `soos-daemon is not ready: no socket at
   <sock> after <T> s.` + `Inspect: systemctl status ...; journalctl ...`, exit 1.
4. Health poll: repeat `soos-admin --format json --socket-path <sock> status` every
   `POLL_INTERVAL` = 0.5 s while it exits non-zero and the deadline has not passed. Each attempt's
   stdout is captured in a variable (bounded by the size of one status report); stderr of the
   attempt is discarded except for the last attempt on timeout (it is
   `AdminCliError` text only, no secret).
   - **Success** (exit 0, i.e. `is_healthy = true`): print the captured stdout of **that** attempt
     once to stdout (exactly one JSON document, no earlier unhealthy report) and exit 0.
   - **Deadline passed**: print the captured stdout of the **last** attempt once to stdout (the
     operator sees the last state, e.g. `camera_ready: false`), then on stderr
     `[ERROR] soos-daemon did not report healthy within <T> s ('<admin> status' kept failing).`
     and `Inspect: soos-admin status; journalctl -u soos-daemon.service -n 50`; exit 1.
     At least one attempt is always made once the socket exists, even if the socket appeared at
     the very end of the bound. An attempt killed by the per-attempt bound (exit 124 or 137)
     contributes **no** stdout (its partial output is discarded); when no attempt completed,
     nothing is printed to stdout before the error.
5. Exit codes unchanged: 0 ready, 1 not ready, 2 usage. `install.sh` keeps mapping non-zero to
   exit 70 (`soos was installed, but the daemon is not ready`).

### 3.2 Per-attempt bound

`soos-admin status` bounds its socket I/O (500 ms) but also runs `systemctl show` without a
timeout. Each attempt therefore runs under coreutils
`timeout --kill-after=1 <ATTEMPT_TIMEOUT_S> <admin> ...` with `ATTEMPT_TIMEOUT_S = 5` when
`timeout` is on `PATH` (coreutils: every supported distribution ships it); without it the attempt
runs unwrapped (degraded, documented). A timed-out attempt (exit 124/137) counts as a failed
attempt. `timeout` signals its whole process group (no `--foreground`), so a child of a hung
`soos-admin` cannot keep the command-substitution pipe open.

### 3.3 Constants (script-local, `readonly`)

| Name | Value | Meaning |
|---|---|---|
| `MAX_TIMEOUT_S` | 300 (existing) | upper bound of `--timeout` |
| `POLL_INTERVAL_S` | `0.5` | socket and health poll period |
| `ATTEMPT_TIMEOUT_S` | 5 | bound of one `soos-admin status` attempt |

## 4. D2 — Build Target Directory Ownership Preflight (`scripts/install.sh`, A.3)

### 4.1 Build identity resolution (refactor, same semantics)

Extract `resolve_build_user` from `run_release_build`:
- running as root (`id -u` = 0): the invoking user (`invoking_user`, unchanged); empty when
  unknown (then the existing root-owned/non-root-owned checkout decision of `run_release_build`
  applies unchanged);
- not root: the current user (`id -un`).

### 4.2 Check `check_build_target_dir <user>` (read-only)

`BUILD_TARGET_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/target}"` (the same base as
`DEFAULT_ARTIFACT_DIR`).

| Condition | Result |
|---|---|
| `--build` not given | check not run |
| `BUILD_TARGET_DIR` absent (and not a dangling symlink) | pass (cargo creates it as the build user) |
| build user empty (root build in a root-owned checkout) or resolves to uid 0 | pass (root can write) |
| build user does not resolve (`id -u -- <user>` fails) | pass, no message (the later `runuser` fails closed with exit 40; nothing is installed) |
| `BUILD_TARGET_DIR` exists but is not a directory | preflight error `The cargo target directory <T> is not a directory.` |
| some entry under it (`find -H <T>`, first hit only, `-print -quit`) has an owner uid ≠ the build user's uid | preflight error, D2 message below |
| else some directory under it owned by the user lacks `u+w` (`find -H <T> -type d ! -perm -u+w -print -quit`) | preflight error, D2b message below |
| `find` exits non-zero with no hit (unreadable subtree) | preflight error naming `<T>` with the D2 fix (fail closed) |

D2 message (two `error` lines; `<T>` printed with `printf %q`, which is the raw path when it has
no shell-special character):

```
[ERROR] The cargo target directory <T> holds files not owned by the build user '<U>' (first: <path>), e.g. left by an earlier root build; cargo would fail with 'Permission denied'.
[ERROR] Fix it with: sudo chown -R <U>: <T>
```

D2b message: `... holds a directory the build user '<U>' cannot write (first: <path>).` and
`Fix it with: chmod -R u+w <T>`.

- The check runs in Phase A **before** `run_release_build` and before the `--dry-run` plan line,
  through `preflight_fail`: the run stops with `Preflight failed with N error(s); nothing was
  modified.` and **exit 2**, before `check_build_deps.sh`, `runuser` or `cargo` is invoked and
  before Phase B (no file, directory, group or unit is touched).
- `--dry-run --build` reports the same error (exit 2).
- Cost: one `find` walk of the target tree in the healthy case (stops at the first hit
  otherwise); read-only; no symlink is followed below `<T>` (`-H` only resolves `<T>` itself).
- Root installs into a root-owned checkout (CI containers, build user empty) are unchanged.
- `find` is added to the A.2 required-tool check when `--build` is given.

### 4.3 Frozen strings (existing contracts, P1)

The refactor keeps byte-identical, and in the same order of execution:
- the `runuser -u "${build_user}" -- bash -lc 'cd "$1" && ./scripts/check_build_deps.sh && SOOS_BINDIR="$2" cargo build --release --locked --workspace' _ "${WORKSPACE_ROOT}" "${SOOS_BUILD_BINDIR}"` call
  (`arch_faillock_ci_contract::test_install_build_exports_the_prefix_bindir`);
- the non-root build line `SOOS_BINDIR="${PREFIX}/bin" "${BUILD_CMD[@]}"` (same test);
- the root-without-invoking-user refusal (`Refusing to build as root ...`, `return 1`, exit 40,
  empty call log) inside `run_release_build`
  (`packaging_ownership_contract::test_install_build_as_root_drops_to_invoking_user_or_refuses`).

The new check is a separate function called from A.3 before `run_release_build`; it never
invokes `runuser`, `cargo` or `check_build_deps.sh`. The fixture users `sudouser`, `doasuser`
and `pkexecuser` of those tests do not resolve, so the check passes for them (§4.2 row 4).

## 5. D3 — Presence-Only Wake Settle

### 5.1 Decision (trade-off stated for the owner, §8 D1–D2)

Options considered:

| Option | PAM instant wake | PAD rules | Verdict |
|---|---|---|---|
| A. Raise `[pipeline] warmup_frames` default | **changed** (every PAM wake slower) | unchanged | rejected (DMN-16 contract) |
| B. Ignore / downgrade a PAD veto when it happens on a wake frame | unchanged | **changed**: an evaluated spoof result would be discarded, a veto could become an allow | rejected (security) |
| C. Presence-only *time-based* settle: after a scan that **woke** the camera, never evaluate a frame captured before `wake_ready + PRESENCE_WAKE_SETTLE_MS` | unchanged | unchanged | **chosen** |
| D. Presence-only *frame-count* settle (skip the first N frames) | unchanged | unchanged | rejected: depends on fps / idle throttle; time is what auto-exposure needs |
| E. Camera-level "stream started at" timestamp in `CameraManager` | unchanged | unchanged | deferred: adapter-crate trait change; C covers the observed case |

### 5.2 Types, constants, signatures

`crates/daemon/src/presence/mod.rs`:

```rust
/// Settle period after a presence scan woke the camera: frames captured earlier (sensor
/// auto-exposure still converging) are never evaluated. Presence only; the PAM path keeps
/// the instant-wake contract (`DAEMON_DEFAULT_WARMUP_FRAMES` = 0).
pub const PRESENCE_WAKE_SETTLE_MS: u64 = 1000;
```

Bound: a constant, `1 ..= MIN_SCAN_INTERVAL_MS` (1000) by construction; not configurable in
this change (adding a `PresenceConfig` field would break the struct-literal contract tests of
GitHub #323; see §8 D2).

`crates/daemon/src/inference.rs` (the request time window carries the lower bound, so
`run_face_consensus` keeps its signature and stays the single call of both callers — pinned by
`presence_unlock_contract::test_pau_pad_consensus_is_built_only_in_consensus_rs`, which requires
`run_face_consensus(` in `dispatcher.rs` **and** `presence/worker.rs`):

```rust
pub struct RequestDeadline {
    deadline_ns: u64,
    outer_deadline: Instant,
    /// Captures stamped before this monotonic instant are never evaluated (0: no bound).
    not_before_ns: u64,
}

impl RequestDeadline {
    /// `compute` is unchanged and sets `not_before_ns = 0`.
    /// Returns this window with the evaluation lower bound `not_before_ns`.
    #[must_use]
    pub const fn with_not_before(self, not_before_ns: u64) -> Self;
    /// The evaluation lower bound (0 when none was set).
    #[must_use]
    pub const fn not_before_ns(&self) -> u64;
}
```

`remaining*`, `can_start*`, `is_expired_at` and `deadline_ns` ignore the lower bound.

`crates/daemon/src/consensus.rs::run_face_consensus` (signature unchanged): a new capture with
`timestamp_mono_ns < deadline.not_before_ns()` is consumed (its sequence is remembered)
without any preemption check, inference, PAD, match or aggregator record, and leaves
`last_capture_stale` unchanged. Everything else (freshness, admission, preemption, `k = 3`,
any spoof vetoes) is unchanged. The consensus never names the presence constant.

Sentinels: `not_before_ns = 0` (every `compute`d deadline, hence every PAM request) ⇒ no
filter (a frame stamped `0` is never `< 0`). A frame stamped `0` with `not_before_ns > 0` is
skipped (unknown capture time is never treated as settled; fail closed: no decision, never an
allow). `not_before_ns` after the deadline ⇒ no capture is evaluated ⇒
`Decided { Pending, frames_evaluated: 0 }` (presence `NoMatch`), no inference.

### 5.3 Worker change (`PresenceWorker::scan`, steps 11–12)

```text
11.  let woke = !camera.is_ready();          // sampled BEFORE notify_activity
     camera.notify_activity();
     if !wake_camera(camera, MAX_CAMERA_WAKE_WAIT_MS) { return CameraUnavailable }
12.  start_ns = clock()?                       // Err ⇒ Aborted(InternalError), unchanged
     settle = if woke { PRESENCE_WAKE_SETTLE_MS } else { 0 }
     not_before_ns = if woke { start_ns + settle_ns (saturating) } else { 0 }
     deadline = RequestDeadline::compute(
         not_before_ns.max(start_ns), 0, Instant::now(),
         DECISION_BUDGET_MS + RESPONSE_WRITE_MARGIN_MS + settle)
         .with_not_before(not_before_ns)
     run = run_face_consensus(&ctx, template, deadline)          // unchanged call
```

- The consensus budget after the settle is the unchanged `DECISION_BUDGET_MS` (900 ms): the
  settle never eats the decision budget. A not-woken scan computes exactly today's deadline.
- PAM priority unchanged: a PAM request during the settle does not wait for presence (presence
  holds no inference permit while skipping); the first post-settle capture re-checks
  `interactive_demand()` and returns `Preempted`. Shutdown: the worker is stopped/aborted at an
  await point as today; the post-consensus `stop_requested` check is unchanged.
- Rate limit: unchanged (one attempt per scan, recorded before the wake).
- Logging: `debug!(settle_ms, "Camera woken by presence; captures before the settle are not
  evaluated")` once per woken scan; no frame data.

### 5.4 Security analysis (PAD veto on wake frames)

1. A capture before the settle end is **never evaluated**: it yields neither a pass nor a spoof,
   so the filter cannot turn a veto into an allow on the same frames — no evaluated result is
   ever discarded, the aggregator is never reset within a scan, and the filter decision depends
   only on the capture timestamp and a bound fixed **before** any evaluation, never on a PAD or
   match outcome.
2. After the settle, the aggregator rules are unchanged: `Allow` needs `k = 3` consecutive passing
   captures; **any** spoof capture vetoes the scan (`SpoofVetoed`, warn log, no unlock). A real
   photo/screen/mask presented to a settled camera is vetoed exactly as today; a well-exposed
   attack frame is if anything easier for PAD.
3. An attacker gains nothing from the settle window: what was in front of the camera during the
   first second is irrelevant; the unlock still needs 3 consecutive live, matching captures of the
   enrolled owner afterwards, plus every existing guard (grace, re-check, account guard, kill
   switch, 1000 ms allow-to-unlock).
4. Residuals (accepted, fail-safe direction): (a) a camera woken by something else (a PAM request)
   less than 1 s before a presence scan finds `is_ready() = true` and gets no settle — the scan may
   be vetoed as today and the next scan 2 s later succeeds; (b) the first unlock after a wake is
   ~1 s slower.

### 5.5 Latency budget

Presence is not on the PAM path; PAM budget arithmetic is untouched (dispatcher unchanged,
`DAEMON_DEFAULT_WARMUP_FRAMES` = 0). Worst-case woken presence scan: wake ≤ 1200 ms + settle
1000 ms + consensus ≤ 950 ms + re-checks (each ≤ 500 ms) — one tick, ticks are sequential.

## 6. Acceptance Criteria — new matrix component `install-readiness-presence-warmup`

| # | Criterion | Test method (Phase 2) |
|---|---|---|
| IWP1 | `wait_daemon_ready.sh` re-queries `soos-admin status` every 0.5 s while it exits non-zero and succeeds as soon as one attempt exits 0, within one `--timeout` deadline shared with the socket wait | Invariant `install_presence_warmup_contract::test_iwp_readiness_polls_until_status_is_healthy` |
| IWP2 | Only the final attempt's stdout is printed (one JSON document): on success the healthy report, never an earlier unhealthy one; on timeout the last report once, then a stderr error naming the bound and `journalctl`, exit 1 | `test_iwp_readiness_prints_only_the_final_status`, `test_iwp_readiness_times_out_after_polling_with_last_status` |
| IWP3 | A hanging `soos-admin status` cannot extend the wait past `--timeout` + 1 + `ATTEMPT_TIMEOUT_S` + 1 s (killed attempts print nothing) | `test_iwp_readiness_attempt_is_bounded` |
| IWP4 | Missing manifest (immediate), non-socket path (no query), usage errors (exit 2) and `install.sh`'s exit 70 mapping are unchanged | existing `installer_templates_contract::test_wait_daemon_ready_*`, `test_iwp_install_keeps_exit_70_on_readiness_failure` |
| IWP5 | `install.sh --build` with a target directory holding a file not owned by the build user fails in the preflight (exit 2, `nothing was modified`) with `sudo chown -R <user>: <target>`, before `runuser`, `cargo` or any staged file; also under `--dry-run` | `test_iwp_build_refuses_foreign_owned_target_dir`, `test_iwp_dry_run_build_reports_foreign_owned_target_dir` |
| IWP6 | A target directory owned by the build user but holding a non-writable directory fails the same way with `chmod -R u+w <target>` (non-root runner only) | `test_iwp_build_refuses_non_writable_target_dir` |
| IWP7 | A target owned by the build user, an absent target, an unresolvable build user and a root build keep today's behaviour (the build runs) | `test_iwp_build_proceeds_with_an_owned_target_dir` + existing `packaging_ownership_contract::test_install_build_as_root_drops_to_invoking_user_or_refuses` |
| IWP8 | A presence scan that wakes the camera evaluates no capture (no PAD call, no inference) before `PRESENCE_WAKE_SETTLE_MS` after the wake, and unlocks when wake-time captures look like a spoof but settled captures are live (the 2026-10-03 trace) | Daemon integration `presence_wake_settle_tests::test_iwp_woken_scan_skips_captures_before_the_settle`, `test_iwp_woken_scan_unlocks_after_spoof_looking_wake_frames` |
| IWP9 | A spoof after the settle still vetoes a woken scan (no unlock, attempt spent); a constant spoof never unlocks a woken scan | `test_iwp_spoof_after_the_settle_still_vetoes`, `test_iwp_constant_spoof_never_unlocks_a_woken_scan` |
| IWP10 | A scan that finds the camera streaming (not woken) applies no settle | `test_iwp_scan_of_a_streaming_camera_has_no_settle` |
| IWP11 | PAM path unchanged: a `compute`d deadline has `not_before_ns = 0` and `run_face_consensus` then behaves as before; the dispatcher never references the settle; `PRESENCE_WAKE_SETTLE_MS` = 1000 lives in `presence/mod.rs` only; `DAEMON_DEFAULT_WARMUP_FRAMES` stays 0 | `test_iwp_settle_constant_value`, `test_iwp_not_before_zero_is_the_pam_consensus`, invariant `test_iwp_settle_is_presence_only`, existing `warmup_default_tests::test_daemon_default_warmup_frames_constant_is_zero` |
| IWP12 | `run_face_consensus` with `RequestDeadline::with_not_before`: captures stamped before `not_before_ns` are never evaluated; a `not_before_ns` beyond the deadline gives `Pending` with 0 captures and 0 inferences (fail closed) | `presence_consensus`-style tests `test_iwp_not_before_skips_earlier_captures`, `test_iwp_not_before_past_the_deadline_fails_closed` |
| IWP13 | Docs and ADR: `Docs/PACKAGING_AND_PROVISIONING.md` documents the health poll and the `chown` fix, `Docs/DAEMON.md` the presence wake settle, `AI/DECISIONS.md` an ADR "Presence Wake Settle" | Invariant `test_iwp_docs_and_adr_describe_the_changes` |

## 7. Test Hooks for the Tester

- Readiness: fake `soos-admin` scripts (counter file, scripted exit codes, `sleep` for a hang),
  a bound `UnixListener` as the socket (pattern of `installer_templates_contract::ready_fixture`).
- Build check: `packaging_ownership_contract::root_shims` (`id -u` → 0, logging `runuser`/`cargo`)
  with `SUDO_USER=nobody` (a real account, uid 65534, never the test runner) and a scratch
  `CARGO_TARGET_DIR` owned by the runner; the positive case uses the runner's own name (skipped
  when the runner is root); the non-writable case runs unshimmed as a non-root runner with only a
  logging `cargo` shim.
- Presence: `common::build_worker` + `test_clock!`; a **woken** camera is simulated with the
  existing mock API — `camera.inner.set_starved(true)` before the scan and an `on_wake` hook that
  calls `set_starved(false)` (the spy's `is_ready()` is then `false` before `notify_activity` and
  `true` after it). PAD outcome over time: `parts.pad.set_result(..)` from a thread spawned in the
  hook; PAD call count `parts.pad.call_count()`; inference instants via `set_extractor_hook`.
  `SpyCamera` restamps captures with the test clock, so `not_before_ns` and capture stamps share
  one clock domain. No fixture change is required.

## 8. Decisions for the Owner (recommended defaults applied)

- **D1 — settle mechanism**: presence-only, time-based, triggered when the scan itself woke the
  camera (option C). Recommended. Alternative E (camera-level stream-start stamp) also covers
  wakes by other callers but changes the `CameraManager` trait.
- **D2 — settle length and configurability**: constant `PRESENCE_WAKE_SETTLE_MS = 1000`. The trace
  shows captures ~0.45 s after the resume still classified as spoof and captures at ~2.5 s live;
  1000 ms is the smallest round value clear of the observed failure; a hardware measurement on more
  cameras could tune it. Making it a `[presence] wake_settle_ms` key is a follow-up (needs a
  migration of the #323 struct-literal tests).
- **D3 — ownership criterion of the build check**: any entry under the target directory not owned
  by the build user fails (strict; a deliberately shared, group-writable target directory owned by
  another account is refused too, with the `chown` fix). Recommended.

## 9. Documentation Drift / ADR

- ADR to add (Phase 6): **[2026-10-03] Presence Wake Settle and Health-Polled Install Readiness
  (GitHub #329)** — presence-only 1000 ms settle after a scan wakes the camera (§5), PAM instant
  wake and PAD rules unchanged, follow-up of the DMN-16 note "a time-based warm-up ... needs
  hardware measurement"; `wait_daemon_ready.sh` polls health within its bound; `install.sh --build`
  refuses a foreign-owned target directory with the `chown` fix.
- `Docs/PACKAGING_AND_PROVISIONING.md` §Start and readiness currently says the helper waits for
  the socket "then prints" the status: drift after D1, to update.

## 10. Plan-Evaluator Round 1 — Resolution

| Finding | Resolution |
|---|---|
| P1 MAJOR — pinned `--build` strings | §4.3 freezes them; the check is a separate function |
| P2 MINOR — `SECONDS` granularity | §3.1 item 2: check after each failed attempt, `+ 1` s, worst case stated |
| P3 MINOR — `find` dependency | §4.2: added to the A.2 tool check under `--build` |
| P4 MINOR — partial output of a killed attempt | §3.1 item 4: discarded |
| P5 MINOR — filter vs sleep indistinguishable in IWP12 | accepted residual, recorded in the tester contract; IWP8 proves no PAD call before the settle |
| P6 MAJOR (round 3, found by the Phase-2 reference validation) — a new `run_face_consensus_not_before` entry point breaks `presence_unlock_contract::test_pau_pad_consensus_is_built_only_in_consensus_rs` (requires `run_face_consensus(` in `presence/worker.rs`) | §5.2: the bound travels in `RequestDeadline::with_not_before`; `run_face_consensus` keeps its signature and both callers keep calling it |
