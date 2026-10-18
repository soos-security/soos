# Walkthrough 237 — setup.sh Daemon Start Bounds, Tailscale Substitution Rule, Target-Directory Owner Test

- **Date**: 2026-10-18
- **Issue**: GitHub #437, part of #362 (GitHub-only, no backlog id) — **Branch**: `fix/setup-start-bounds`
- **Decision**: none new. The change applies the existing rule "every blocking call has an explicit, honest bound" to `scripts/setup.sh`. It does not touch the deferred journal-group and linger decisions (AS-U / OD-U4 of ADR 2026-10-15 "Setup User Phase").
- **Matrix criteria**: SET40–SET42 (new, verified), SET43 (new, owner hardware row, pending). SET3 and every other existing row are unchanged.

## 1. Context & Objectives

Three follow-ups of the setup work were still open on `main` (3f76090):

1. **Daemon start bounds.** `setup.sh` wrapped every `systemctl start|restart|stop|try-restart soos-daemon.service` in `run_bounded "${QUERY_TIMEOUT_S}"` (10 s). `packaging/soos-daemon.service` is `Type=notify` with `TimeoutStartSec=60`: `systemctl start` returns only after `READY=1`, and model verification and inference warm-up run before that point. A start slower than 10 s was killed by setup even though systemd would still have accepted it. The `[preview]` step then removed the `daemon.toml` it had just created and reported a failure. The enrollment hand-off reported `did not start again`.
2. **Tailscale print rule.** SET3 (`test_set_s3_no_pam_tailscale_remote_or_network_lock_action`) and the `quoted_at` helper of `setup_user_phase_contract.rs` decide "printed, not run" by counting quote characters before the word. A `"$(tailscale serve ...)"` has one `"` before the word, so it counted as a printed message.
3. **Target-directory owner.** `check_target_dir_owner` (preflight refusal "holds files not owned by the build user") had no setup test. Only the `install.sh` counterpart was covered (`install_presence_warmup_contract.rs`).

## 2. Architect Design

- New constant `readonly UNIT_START_TIMEOUT_S=90` in `setup.sh`: `TimeoutStartSec` (60) plus a 30 s margin for a graceful stop (the first half of a restart) and the job hand-off. It is used for exactly the six daemon-unit calls: the `try-restart` in the rollback journal, the `start` of the EXIT trap, the two `restart`s of the camera-view step, and the `stop` and `start` of the enrollment hand-off.
- `QUERY_TIMEOUT_S` (10 s) stays the bound of the read-only queries (`unit_query is-enabled|is-active`, `getent`, `tailscale version`) and of every `systemctl --user` call of the companion (those `Type=exec` units are out of scope).
- No new ADR. This is a bound correction under existing rules.

### Plan self-check (plan-evaluator)

- *Restart = stop + start.* The unit sets no `TimeoutStopSec`, so a stop that hangs falls back to systemd's `DefaultTimeoutStopSec` (90 s by default). The 30 s margin covers a graceful stop: the daemon exits on SIGTERM. It does not cover a stop that hangs. In that case `timeout` ends the `systemctl` client, the systemd job continues, and setup reports the failure. This is the same fail-closed path as before, reached later. Adding `TimeoutStopSec` to the unit was left out of scope because it would change the shipped unit and its own contracts.
- *Total worst case.* The camera-view step can run two restarts and two readiness waits: 2 × (90 + 2 kill grace) + 2 × ~37.5 s ≈ 260 s. It is still bounded. Before, the bound was 10 s, but it gave the wrong answer.
- *Docker job `setup-root-preview`.* Case C2 makes the restart fail at once (the stand-in daemon refuses the config). It does not depend on the 10 s bound, so this change does not lengthen it.
- *Existing contracts.* `test_set_daemon_stop_marker_cleared_only_after_restart` still finds `systemctl start "${UNIT}"` in `phase_enrollment`. No other test names `QUERY_TIMEOUT_S`.

## 3. Tester Contract (written first)

All three tests are new and were appended to `tests/invariants/src/setup_contract.rs`. No existing assertion was changed.

| Row | Test | Red evidence |
|---|---|---|
| SET40 | `test_set_s6_daemon_unit_calls_bounded_by_unit_start_timeout`: parses `TimeoutStartSec` from `[Service]` of the shipped unit and `UNIT_START_TIMEOUT_S` / `QUERY_TIMEOUT_S` from `setup.sh`. Requires a margin of 10 to 60 s. Requires every `systemctl start\|restart\|stop\|try-restart "${UNIT}"` to be `run_bounded "${UNIT_START_TIMEOUT_S}"` (at least 6). `QUERY_TIMEOUT_S` never bounds such a call, `UNIT_START_TIMEOUT_S` never bounds a `--user` call, and the read-only unit query stays on `QUERY_TIMEOUT_S`. | Failed on `main`: `setup.sh must declare readonly UNIT_START_TIMEOUT_S=<seconds> exactly once`. |
| SET41 | `test_set_s7_no_tailscale_command_inside_command_substitution`: a small shell scanner (contexts: double quote, single quote, `$( )`, backticks, `( )`, `${ }`, unquoted here-document bodies; `#` comments, `\` escapes, `<<<` here-strings). It flags any `tailscale <word>` with `<word>` ≠ `version` when any enclosing context is a substitution, even through inner quotes (`$(bash -c 'tailscale …')`). The scanner is checked first on 17 good and bad snippets, then must end balanced on `setup.sh` with no finding. | Guard test: it passes on `main`. Mutation check: inserting `info "node: $(tailscale status --json)"` into `setup.sh` made SET41 fail while SET3 still passed (the gap is real), and the line was reverted. |
| SET42 | `test_set_target_dir_foreign_owner_refused_before_build`: harness with `<checkout>/target/release/stale`, and `STUB_UID` set to real uid + 1, so `id -u` / `getent passwd alice` resolve the build user to a uid that owns none of the files (no root needed). It asserts exit 2, the message naming `'alice'`, the exact `sudo chown -R alice: <target>`, "nothing was changed", no `cargo`, `sudo`, `runuser`, `install.sh` or `install_rustup.sh` log line, only read-only calls, and an identical whole-tree snapshot. | Coverage test: it passes on `main`. Mutation check: turning the `check_target_dir_owner` call into a no-op made it fail, and the line was reverted. |

## 4. Auditor Constraints

- C-1: every new bound is finite and declared `readonly`, and no call becomes unbounded.
- C-2: the user-phase rules and the `--user` units are unchanged.
- C-3: the tests run without root, use only the existing stub harness and restricted `PATH`, and reach no real `systemctl`, `sudo`, `cargo` or `tailscale`.
- C-4: the scanner panics, and so fails the test, when it cannot follow the script (unbalanced contexts, a here-document without a delimiter or terminator). A parser regression cannot pass silently.
- C-5: no secret, frame or embedding is involved, and nothing new is logged.

## 5. Developer Changes

- `scripts/setup.sh`: the documented `UNIT_START_TIMEOUT_S=90` constant, and the six daemon-unit calls moved to it.
- `tests/invariants/src/setup_contract.rs`: helpers `daemon_unit_seconds`, `setup_readonly_seconds`, `scan_shell`, `tailscale_in_substitution`, and the three tests above.
- `Docs/PACKAGING_AND_PROVISIONING.md` §3.0: new "Time bounds" paragraph.
- `AI/VERIFICATION_MATRIX.md`: rows SET40–SET43.

## 6. Verification

```
LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/setup-start-bounds CARGO_INCREMENTAL=0 \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=3 \
  cargo test -p soos-invariants --lib -- --test-threads=3
cargo clippy -p soos-invariants --all-targets -- -D warnings
cargo fmt --all -- --check
bash -n scripts/setup.sh
```

## 7. Owner Checks (pending)

- SET43: on the Arch host with the real models, run `./scripts/setup.sh` with the camera view on, after a reboot (cold cache) and with enrollment. Expected: `[DONE] camera view (daemon)`, `/etc/soos/daemon.toml` kept, and the enrollment hand-off starts the daemon again. Compare the start-to-`READY=1` time in `journalctl -u soos-daemon.service`. The Docker job `setup-root-preview` (SET34) also runs on the PR in CI.
