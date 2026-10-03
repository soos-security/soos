# Walkthrough 178 — Install Readiness Health Poll, Build Directory Ownership, Presence Wake Settle

- **Date**: 2026-10-03
- **Issue**: GitHub #329 (GitHub-only, no backlog sub-issue; the branch is not registered in
  `scripts/sync_issue.py`, the commit carries `Closes #329`) — **Branch**:
  `fix/install-readiness-and-presence-warmup`
- **Matrix criteria**: IWP1–IWP13 (`AI/VERIFICATION_MATRIX.md` section
  `install-readiness-presence-warmup`), all `✅ Verified`
- **Phase documents**: `AI/architect_spec_install_presence_warmup.md` (rev 3),
  `AI/plan_evaluator_report.md` (round 3, `APPROVED`), `AI/tester_contract_install_presence_warmup.md`,
  `AI/auditor_constraints_install_presence_warmup.md` (`CLEARED`, C1–C25)
- **ADR**: 2026-10-03 "Presence Wake Settle and Health-Polled Install Readiness (GitHub #329)"

## 1. Context & Objectives

Three defects were found while upgrading a real Arch install to presence auto-unlock (owner's
host, 2026-10-03):

1. **Readiness race (D1)**: after `systemctl restart` of an active unit, `install.sh` step 9 ran
   `soos-admin status` once at daemon uptime 0 s. The camera was still `starting`
   (`"is_healthy": false`), `soos-admin status` exited 1 and the installer exited 70 ("socket exists
   but 'soos-admin status' failed") although the daemon was healthy about 0.5 s later. The wait
   bounded only the socket, never the health. CI never saw it: the Docker systemd harness uses the
   mock camera, which is ready at once.
2. **Root-owned build directory (D2)**: `install.sh --build` builds as the invoking user (GitHub #308),
   but a `target/` left by an older root build made cargo fail late with
   `failed to open .../target/release/.cargo-build-lock: Permission denied`, reported only as
   `Release build failed` (exit 40) with no fix.
3. **Presence first-scan veto (D3)**: when a presence scan woke the camera from auto-standby, the
   first frames (auto-exposure still converging) were classified as spoof and the scan was vetoed
   (journal: resume at 22:44:28.046, `Presentation attack detected ... captures_evaluated=2` at
   28.493); the next scan 2 s later, camera still streaming, unlocked with 3 captures. The daemon's
   `warmup_frames` default is 0 (DMN-16, PAM instant wake), so nothing discarded those frames.

## 2. Architect Design

- **D1**: `scripts/wait_daemon_ready.sh` keeps its options, exit codes (0/1/2) and the immediate
  missing-manifest failure. One bound (`--timeout`, bash `SECONDS` deltas plus a 1 s granularity
  margin) covers the socket wait and a health poll that re-runs `soos-admin status` every 0.5 s
  until it exits 0. Each attempt runs under `timeout --kill-after=1 5` (constant
  `ATTEMPT_TIMEOUT_S`); only the final report is printed.
- **D2**: a read-only `check_build_target_dir` in `scripts/install.sh` Phase A, under `--build`
  only, before the build block and the `[DRY-RUN] Would run:` line; `resolve_build_user` gives the
  account cargo will run as. Failure goes through the preflight counter (exit 2, nothing modified).
- **D3**: `pub const PRESENCE_WAKE_SETTLE_MS: u64 = 1000` in `crates/daemon/src/presence/mod.rs`;
  `RequestDeadline` gains a private `not_before_ns` (0 from `compute`), `with_not_before` and
  `not_before_ns()`; `run_face_consensus` (signature unchanged) skips captures stamped before the
  bound; the presence worker sets the bound only when the scan itself woke the camera.
- Owner decisions (recommended defaults accepted): presence-only settle trigger,
  `PRESENCE_WAKE_SETTLE_MS = 1000` (not configurable), strict foreign-owner criterion.

## 3. Plan Evaluation

Round 3 `APPROVED`. Round 1: P1 (frozen `--build` strings), P2 (`SECONDS` granularity), P3 (`find`
in the tool check), P4 (partial output of a killed attempt) resolved; P5 (IWP12 cannot tell a
filter from a sleep) accepted as a residual covered by the worker tests. P6 (found by the
tester's reference validation): a separate `run_face_consensus_not_before` entry point would break
`presence_unlock_contract::test_pau_pad_consensus_is_built_only_in_consensus_rs`; revision 3 moved
the bound into `RequestDeadline`.

## 4. Tester Contract

20 new tests, no existing test or fixture changed:

- `tests/invariants/src/install_presence_warmup_contract.rs` (11): readiness poll, final-report-only
  output, timeout with last report, bounded attempt (IWP1–IWP3), exit 70 kept (IWP4), foreign-owned
  target refused also in a dry run (IWP5), non-writable target refused (IWP6), owned/absent target
  proceeds (IWP7), settle presence-only (IWP11), docs and ADR (IWP13).
- `crates/daemon/tests/presence_wake_settle_tests.rs` (5): IWP8 (no PAD call before the settle; the
  2026-10-03 trace now unlocks), IWP9 (settled spoof still vetoes; constant spoof never unlocks),
  IWP10 (streaming camera, no settle).
- `crates/daemon/tests/presence_wake_settle_consensus_tests.rs` (4): IWP11/IWP12 on the consensus.

Red evidence on the base `b16f567`: 17 red (assertion, or compile error on exactly the specified
API), 3 green by design (IWP4 characterisation, IWP7 and IWP10 no-false-positive tests).

## 5. Auditor Constraints

All 25 constraints met:

- C1–C4: the filter is `frame.timestamp_mono_ns < deadline.not_before_ns()` with no `> 0`
  exemption, placed right after `last_sequence` is updated and before freshness, Background
  preemption, admission, `decay_estimate`, permit acquisition, inference and `aggregator.record`;
  a skipped capture leaves `last_capture_stale` untouched (the skip is the first arm of the
  `if / else if is_frame_fresh / else stale` chain). Nothing else in the consensus changed.
- C5: one private field, `compute` sets 0, `with_not_before` is `const` + `#[must_use]` (struct
  update syntax), `not_before_ns()` is a `const` getter; budget methods ignore the bound.
- C6–C7, C9–C10: `woke = !camera.is_ready()` before `notify_activity()`; bound from the worker's
  `clock_fn`, `saturating_mul` / `saturating_add`; the non-woken window is `compute(start_ns, 0, now,
  DECISION_BUDGET_MS + RESPONSE_WRITE_MARGIN_MS)` with bound 0; one `debug!` carrying `settle_ms`
  only; `with_not_before` is called only in `presence/worker.rs`.
- C8: `git diff --exit-code origin/main -- crates/daemon/src/dispatcher.rs crates/daemon/src/config.rs
  crates/protocol crates/pam crates/camera-v4l` is empty.
- C11–C15: the attempt runs as `report="$(...)" || rc=$?` (no pipe); `SECONDS` deltas only;
  do-while loop with the stop test after each failed attempt; a one-time
  `timeout --kill-after=1 5 true` capability probe, otherwise unwrapped with one stderr warning;
  exit 124/137 discard the attempt's output; no temporary file, no write.
- C16–C22: `check_build_target_dir` only reads (`[[ -e/-L/-d ]]`, `id -u -- "${user}"`,
  `find -H <T> ! -uid <uid> -print -quit`, `find -H <T> -type d ! -perm -u+w -print -quit`); a
  relative target is `./`-prefixed; a `find` failure without a hit fails closed; a dangling symlink
  or non-directory is refused; every untrusted string (target, first offending path, user) is
  printed through `printf %q` with a new `preflight_fail_verbatim` helper (the existing `error()`
  uses `echo -e`, which would re-interpret the escapes `printf %q` produces); `find` joins the A.2
  tool check under `--build` only; `run_release_build` and its frozen strings are untouched.
- C23–C25: no dependency, no `unsafe`, no `println!`; gates below.

## 6. Implementation

| File | Change |
|---|---|
| `scripts/wait_daemon_ready.sh` | Health poll within one bound, per-attempt `timeout`, final-report-only stdout, `did not report healthy within <T> s` error |
| `scripts/install.sh` | `find` in A.2 under `--build`; `resolve_build_user`, `preflight_fail_verbatim`, `check_build_target_dir` before the A.3 build block |
| `crates/daemon/src/inference.rs` | `RequestDeadline::not_before_ns` field, `with_not_before`, `not_before_ns()` |
| `crates/daemon/src/consensus.rs` | Lower-bound skip in `run_face_consensus` |
| `crates/daemon/src/presence/mod.rs` | `PRESENCE_WAKE_SETTLE_MS = 1000` |
| `crates/daemon/src/presence/worker.rs` | `woke` sampled before `notify_activity`; settle window and budget |
| `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/DAEMON.md`, `README.md` | Health poll, `--build` ownership check and its fixes, presence wake settle |
| `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md` | ADR and IWP1–IWP13 |

Deviations from the spec:

- `resolve_build_user` is a new function used only by the check; `run_release_build` was left
  byte-identical instead of being refactored to call it (same identity rules, smallest diff, frozen
  strings trivially kept).
- The stderr of the status attempts is always discarded (spec §3.1 allowed showing the last one on
  timeout; constraint C15 made it optional and in-memory only). The timeout message points the
  operator at `soos-admin status` and the journal instead.
- An attempt that printed nothing contributes no report (no empty line on stdout).

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) passes on this diff. The independent Phase 5 review (report
`AI/candid_review_report.md`, bound to the diff fingerprint) is run by the orchestrator.

## 8. Verification Results

- `cargo fmt --all -- --check`: pass.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --workspace --all-targets --all-features`: all green (see the developer
  report for counts); the only failure seen in one run was
  `cli_deadline_json_tests::test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum` (admin-cli,
  untouched), the load-only flake diagnosed in walkthrough 176; it passed on every re-run.
- Contract commands, 10 consecutive runs each, all green:
  `cargo test --locked -p soos-invariants --all-features install_presence_warmup` (11),
  `cargo test --locked -p soos-daemon --all-features --test presence_wake_settle_tests` (5),
  `cargo test --locked -p soos-daemon --all-features --test presence_wake_settle_consensus_tests` (4).
- `cargo deny --locked check`: pass (no dependency change).
- `bash -n` and `shellcheck -S warning` on both scripts: `wait_daemon_ready.sh` clean;
  `install.sh` reports only the pre-existing SC2034 (`BOLD` unused), identical on `origin/main`.
- `python3 scripts/sync_issue.py --check`: pass.

## 9. Known Limitations / Follow-ups

- **P-1 (pre-existing)**: a relative `CARGO_TARGET_DIR` is resolved against the installer's working
  directory for `DEFAULT_ARTIFACT_DIR` (and now the ownership check), but against the checkout by
  cargo (`cd "$1"` in the `runuser` login shell, whose profile may also override it). Fail-safe
  (the artifact check catches a missing build); worth a follow-up issue.
- **P-2 (pre-existing)**: `soos-admin status` runs `systemctl show` without a timeout. The readiness
  helper bounds each attempt with `timeout`; the admin CLI itself is unchanged.
- The settle covers only wakes caused by the presence scan itself: a camera woken by a PAM request
  less than 1 s before a scan gets no settle (that scan may be vetoed as before; the next succeeds).
  A camera-level stream-start stamp (spec option E) would cover it.
- `PRESENCE_WAKE_SETTLE_MS` is a constant; a `[presence] wake_settle_ms` key needs a migration of
  the GitHub #323 struct-literal tests. 1000 ms was chosen from one camera's trace; measurements on
  more cameras could tune it.
- Untested by design (review-enforced): the `timestamp_mono_ns == 0` sentinel, and the degraded
  readiness paths (no `timeout`, `timeout` without `--kill-after`).
