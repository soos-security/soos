# Plan Evaluation Report (round 3)
- **Date**: 2026-10-03
- **Issue**: GitHub #329 — install readiness race, root-owned build directory hint, presence first-scan spoof veto (GitHub-only, no `AI/BACKLOG.md` entry)
- **Branch**: `fix/install-readiness-and-presence-warmup`
- **Base commit**: `b16f567`
- **Plan**: `AI/architect_spec_install_presence_warmup.md` revision 3

## 0. Findings of Round 1 and Resolution
| Round | Finding | Resolution | Status |
|---|---|---|---|
| 1 | P1 MAJOR — the `resolve_build_user` refactor did not freeze the `--build` strings pinned by `arch_faillock_ci_contract::test_install_build_exports_the_prefix_bindir` (`runuser ... SOOS_BINDIR="$2" cargo build ...`, `SOOS_BINDIR="${PREFIX}/bin" "${BUILD_CMD[@]}"`) and `packaging_ownership_contract::test_install_build_as_root_drops_to_invoking_user_or_refuses` (`Refusing to build as root`, empty call log) | Spec §4.3: byte-identical, separate check function that never runs `runuser`/`cargo`; fixture users do not resolve, so the check passes for them | Resolved |
| 1 | P2 MINOR — `SECONDS` granularity and evaluation point undefined | §3.1 item 2: evaluated after each failed attempt, before sleeping, with a `+ 1` s margin; worst case stated | Resolved |
| 1 | P3 MINOR — `find` not in the tool preflight | §4.2: added to A.2 under `--build`; `find` is already in the restricted PATH of `installer_contract::restricted_path` | Resolved |
| 1 | P4 MINOR — partial stdout of a killed attempt | §3.1 item 4: discarded; nothing printed when no attempt completed | Resolved |
| 1 | P5 MINOR — IWP12 cannot tell a filter from a sleep | Accepted residual (recorded in the tester contract); IWP8 asserts no PAD call before the settle on the worker path | Accepted |
| 2 | Round 2 was APPROVED, but the Phase-2 validation of the tests against a throwaway reference implementation of revision 2 found a missed fact: **P6 MAJOR** — revision 2 added a `run_face_consensus_not_before` entry point called by the worker, which breaks the existing contract `presence_unlock_contract::test_pau_pad_consensus_is_built_only_in_consensus_rs` (it requires `run_face_consensus(` in `dispatcher.rs` **and** `presence/worker.rs`). The round-2 "test integrity" pillar had checked the presence test files but not the presence invariants. | Revision 3 (§5.2): the lower bound travels in the request window, `RequestDeadline::with_not_before(ns)` / `not_before_ns()` (0 from `compute`); `run_face_consensus` keeps its signature and both callers keep calling it; no existing test changes | Resolved |

## 1. Coverage Matrix
| Acceptance line (issue #329) | Spec element | Status |
|---|---|---|
| 1. The readiness wait polls until healthy within its documented bound instead of failing on the first unhealthy answer | §3 D1, IWP1–IWP4 | Covered |
| 1. Exit 70 kept on a real timeout | §3.1 item 5, IWP4 (`install.sh` mapping unchanged) | Covered |
| 2. Detect a non-writable target directory / root-owned files under it before building | §4.2, IWP5, IWP6 | Covered |
| 2. Print the exact fix (`sudo chown -R <user>: <target>`) | §4.2 D2 message, IWP5 | Covered |
| 2. Fail closed with nothing installed | §4.2 (Phase A `preflight_fail`, exit 2, before `runuser`/`cargo`/Phase B), IWP5 | Covered |
| 3. Discard the first frames after a wake for presence scans | §5.2–5.3 (`PRESENCE_WAKE_SETTLE_MS`, `run_face_consensus_not_before`), IWP8, IWP10, IWP12 | Covered |
| 3. Without changing the PAM instant-wake contract | §5.1 option C, dispatcher unchanged, IWP11 (+ existing `warmup_default_tests`) | Covered |
| 3. Without changing PAD thresholds / consensus | §5.2 ("everything else unchanged"), §5.4, IWP9 | Covered |
| Caller task: a real spoof is still vetoed; discarding never turns a veto into an allow on the same frames | §5.4 items 1–2, IWP9, IWP8 (no PAD call before the settle) | Covered |
| Caller task: trade-offs stated as owner decisions with defaults | §8 D1–D3 | Covered |

## 2. Facts Verified Against Code
| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Readiness waits only for the socket, then runs status once | `scripts/wait_daemon_ready.sh` steps 2–3 | single `"${ADMIN_BIN}" --format json ... status`, `exit 1` on failure | Yes |
| `soos-admin status` exits 1 when unhealthy after printing | `crates/admin-cli/src/main.rs` `Commands::Status` | `println!` then `if !report.is_healthy { std::process::exit(1) }` | Yes |
| `install.sh` maps readiness failure to 70 | `scripts/install.sh` step 9 | `exit 70` after `wait_daemon_ready.sh` | Yes |
| Default readiness timeout 30, range 1..300 | `wait_daemon_ready.sh` | `TIMEOUT_S=30`, `MAX_TIMEOUT_S=300` | Yes |
| `--build` builds as invoking user via `runuser` | `install.sh::run_release_build` | yes (#308) | Yes |
| Artifact/target base `${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/target}` | `install.sh` `DEFAULT_ARTIFACT_DIR` | same base + `/release` | Yes |
| Daemon `warmup_frames` default 0 | `crates/daemon/src/config.rs` `DAEMON_DEFAULT_WARMUP_FRAMES` | 0 | Yes |
| Presence scan: `notify_activity`, `wake_camera`, consensus at once | `presence/worker.rs::scan` steps 11–12 | yes, `MAX_CAMERA_WAKE_WAIT_MS` 1200 | Yes |
| Any spoof vetoes, `k = 3` | `consensus.rs` + `soos_policy::PadAggregator` | `ConsensusDecision::SpoofVetoed` breaks the loop | Yes |
| `DECISION_BUDGET_MS` / `RESPONSE_WRITE_MARGIN_MS` | `pipeline.rs` / `inference.rs` | 900 / 50 | Yes |
| Real camera `is_ready()` false in standby and until the first post-resume frame | `camera-v4l/src/v4l_impl.rs` (`idle_expired`, `withdraw_frames`), `capture.rs` (`is_ready.store(true)` after the first published frame) | yes | Yes |
| Capture stamps on `CLOCK_MONOTONIC`, presence clock `CLOCK_MONOTONIC` | `capture.rs` `monotonic_nanos`, worker `current_monotonic_nanos` | same domain | Yes |
| Camera `idle_timeout` default 10 s > presence `scan_interval` 2 s | `camera-v4l/src/config.rs`, `presence/config.rs` | 10 s / 2000 ms | Yes |
| Existing invariant pins `run_face_consensus(` in `presence/worker.rs` and `dispatcher.rs` | `tests/invariants/src/presence_unlock_contract.rs:509-520` | yes (P6) | Revision 3 keeps both calls |
| `RequestDeadline` has private fields only, built through `compute`, derives `Copy, PartialEq, Eq` | `crates/daemon/src/inference.rs:73-107` | yes: a new private field is invisible to every caller | Yes |
| Test `SpyCamera` restamps captures with the test clock | `crates/daemon/tests/common/mod.rs` | `restamped.timestamp_mono_ns = (self.clock)()` | Yes |
| `MockCameraManager::set_starved` clears readiness and frames; `notify_activity` republishes when not starved | `camera-v4l/src/mock.rs` | yes | Yes |
| `PresenceConfig` built by struct literal in #323 tests | `presence_config_tests.rs`, `common::fast_presence_config` | yes (no new field possible without migration) | Yes |
| `nobody` resolves on the dev host | `id nobody` | uid 65534 | Yes |
| `find` walk cost on this worktree's `target/` (109k inodes) | measured | 0.16 s for both walks | — |

## 3. Pillar Analysis
### Pillar 1 — Architecture & threat model
- Failure scenario considered: the settle filter is keyed on a value an attacker controls and lets a spoof slip through. The filter is keyed only on the capture timestamp versus a bound fixed before any evaluation (§5.4.1); a pre-settle capture is never evaluated, so it can neither pass nor veto; post-settle rules are unchanged, so the attacker must still produce 3 consecutive live, matching captures. A spoof shown during the settle and replaced by the owner's live face afterwards unlocks — that is the owner being present, as today on the second scan.
- Failure scenario: the build check runs `find` as root over a user-controlled tree. `-H` resolves only `<T>`; below it no symlink is followed; the walk is read-only. A user could make the walk long (many files) — it only delays the user's own install.
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered: the settle leaks into the PAM path. The dispatcher keeps calling `run_face_consensus` (= `not_before_ns` 0, never `< 0`); `DAEMON_DEFAULT_WARMUP_FRAMES` stays 0; IWP11 adds a static check that `dispatcher.rs` never names the settle. A PAM request arriving during a presence settle does not wait on presence (no permit held while skipping) and preempts at the first post-settle capture.
- Failure scenario: the PAM window inherits a lower bound by accident. Only `with_not_before` sets it; `compute` (the dispatcher's only constructor) sets 0, asserted by `test_iwp_not_before_zero_is_the_pam_consensus`; the dispatcher never names `with_not_before` (IWP11 invariant).
- Failure scenario: the settle eats the presence decision budget so every woken scan ends `Pending`. §5.3 starts the 900 ms client budget at the settle end and extends the outer bound by the settle.
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: a frozen camera after a wake holds only pre-settle captures. They are never evaluated; the deadline ends the run with `Pending` ⇒ `NoMatch`, session stays locked (IWP12). A frame stamped 0 with a settle is skipped (never treated as settled). Shell side: a `find` error with no hit fails the preflight closed; an unresolvable build user passes the check but `runuser` then fails (exit 40, nothing installed). Readiness: a status binary that hangs is killed per attempt; exit 1 ⇒ 70.
- Result: PASS.

### Pillar 4 — Dependencies
- Failure scenario considered: new tools missing on a target host. `timeout` (coreutils) is optional with a documented degraded path; `find` (findutils) is checked in the preflight. No crate added.
- Result: PASS.

### Pillar 5 — Data confidentiality
- Failure scenario considered: the readiness helper now prints more. It prints at most one status report (no secret fields: socket path, readiness booleans, pid, uptime, unit state). Presence logs one value-free debug line per woken scan; no frame, embedding or score.
- Result: PASS.

### Pillar 6 — Test integrity
- Failure scenario considered: existing tests break and get edited. Round 3 re-ran the **whole** `soos-daemon` and `soos-invariants` suites (all targets) against a throwaway reference implementation of revision 3: all green, including `presence_unlock_contract`. Checked each: `installer_templates_contract::test_wait_daemon_ready_succeeds_when_socket_and_status_answer` (first attempt succeeds, JSON printed, args logged — unchanged), `test_wait_daemon_ready_fails_closed_and_bounded` (always-failing admin with `--timeout 2`: fails after ≤ ~3 s; the no-socket case still names `journalctl`), `systemd_unit_acceptance_test.sh` (`$(...)` capture still holds one JSON document), POA4/AFC7 (§4.3), presence worker tests (camera ready at scan start ⇒ no settle ⇒ identical timing), `presence_consensus_tests` (`run_face_consensus` signature unchanged). No migration needed.
- Test power: a wrong implementation that applies the settle on every scan fails IWP10; one that ignores the wake fails IWP8 (PAD called within the first 800 ms); one that downgrades a veto fails IWP9; one that only waits for the socket fails IWP1; one that prints every attempt fails IWP2; one that checks only `target/release` writability as root (always true) fails IWP5 under the fake-root shim; one that runs the check after `runuser` fails IWP5's empty call log.
- Result: PASS (residual P5 accepted).

## 4. Findings
- **[MINOR] R2-1** — IWP10 asserts a wall-clock upper bound (the non-woken scan finishes in less than `PRESENCE_WAKE_SETTLE_MS`). The bound (1000 ms) is above the product decision budget (900 ms), so it respects the determinism rule; the tester must run it 10× before hand-off.
- **[MINOR] R2-2** — IWP7's positive case (runner's own name as `SUDO_USER`) and IWP6 cannot run as root (root writes everything, root is never the invoking user); both skip on a root runner. CI's `Quality` job runs as a non-root runner user, so they execute there.

## 5. Verdict
VALIDATION_VERDICT: APPROVED
