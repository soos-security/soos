# Walkthrough 111 — Fail-Closed Daemon Configuration, Socket Binding Without `groupadd`, Least-Privilege Unit

- **Date**: 2026-09-30
- **Issues**: GitHub #199 (DMN-08), #202 (DMN-13), #203 (DMN-14)
- **Branch**: `fix/p2-daemon-config-socket-systemd`
- **Matrix criteria**: DCS1–DCS12 (new, component `daemon-config-socket-systemd`)
- **ADR**: 2026-09-30 "Fail-Closed `daemon.toml` Validation, No Runtime `groupadd`, Least-Privilege Daemon Unit"
- **Scope**: `crates/daemon/src/config.rs`, `crates/daemon/src/socket.rs`,
  `packaging/soos-daemon.service`, three new test files, docs. No pre-existing test was changed.

---

## 1. Findings

| ID | Problem |
|---|---|
| DMN-08 | `daemon.toml` values were copied without checks: `socket_mode = 438` (0o666) gave a world-writable socket, `connection_timeout_ms = 0` timed out every request, `rate_limit.max_attempts = 0` rejected every UID, an invalid `log_level` was dropped silently. (Thresholds were already floored by GitHub #170; `max_concurrent_connections = 0` was already refused by `PeerLimitsConfig::validate`.) |
| DMN-13 | On `EOPNOTSUPP` from `fchmodat(AT_SYMLINK_NOFOLLOW)` (glibc < 2.32, musl) the code fell back to a path-based `fs::set_permissions`, which follows symlinks; `resolve_socket_group` spawned `groupadd --system` as root, which can only fail under `ProtectSystem=strict`. |
| DMN-14 | The unit ran as unrestricted root: no capability bounding set, no syscall filter, no kernel / namespace / realtime protection, network namespace shared, no ordering before the display manager. |

While writing the DMN-14 tests a further defect appeared: `DeviceAllow=/dev/video* rw` matches
nothing. `systemd.resource-control(5)`: "such globbing wildcards are not available for device
node path specifications". With `DevicePolicy=closed`, the camera was therefore not allowed by the
unit at all. `DeviceAllow=char-video4linux rw` (group `81 video4linux` in `/proc/devices`) fixes it.

## 2. Specification

- `crates/daemon/src/config.rs`: constants `ALLOWED_SOCKET_MODES` = `[0o660, 0o600]`,
  `MIN_CONNECTION_TIMEOUT_MS` = 100, `MAX_CONNECTION_TIMEOUT_MS` = 10 000, `MAX_LOG_LEVEL_LEN` = 256;
  `SocketConfig::validate`, `DispatcherConfig::validate`, `PipelineConfig::validate` and
  `DaemonConfig::validate` (which also calls the existing preview and peer-limit checks). Every
  failure is `DaemonError::Config` naming the setting. `from_toml_str` calls `validate()` last.
- `PipelineConfig::validate` re-runs `ThresholdConfig::builder().build_with_security_floor()` on the
  stored thresholds (a config built in code with `ThresholdConfig::new_raw` is no longer trusted)
  and requires the vision mirror to equal them bit for bit.
- `crates/daemon/src/socket.rs`: `bind_socket` calls `SocketConfig::validate` before opening the
  directory; `resolve_socket_group` only looks the group up; new
  `chmod_socket_node_nofollow(dir, name, mode)` is the `EOPNOTSUPP` fallback.
- Unit: see the ADR and `Docs/PACKAGING_AND_PROVISIONING.md` §8.

## 3. Tests first (red evidence)

New files: `crates/daemon/tests/config_validation_tests.rs` (20 tests),
`crates/daemon/tests/socket_hardening_tests.rs` (9 tests),
`crates/daemon/tests/systemd_hardening_tests.rs` (7 tests).

Before the implementation (`cargo test --locked -p soos-daemon --all-features --test ...`):

- `config_validation_tests`, `socket_hardening_tests`: did not compile —
  `unresolved imports soos_daemon::config::ALLOWED_SOCKET_MODES, MAX_CONNECTION_TIMEOUT_MS, MAX_LOG_LEVEL_LEN, MIN_CONNECTION_TIMEOUT_MS`,
  `no method named validate found for struct DaemonConfig` / `SocketConfig`,
  `unresolved import soos_daemon::socket::chmod_socket_node_nofollow`.
- `systemd_hardening_tests`: 6 of 7 failed (`CapabilityBoundingSet`, `SystemCallFilter`,
  kernel/namespace protections, `IPAddressDeny`, `char-video4linux`, `Before=display-manager.service`
  missing); `test_unit_never_grants_dangerous_capabilities` passed (it guards against regressions).

After: all 36 new tests pass, and the pre-existing `systemd_test`, `hardening_tests`,
`socket_tests`, `config_tests` and `threshold_config_tests` pass unchanged.

## 4. Implementation notes

- The chmod fallback opens `/proc/self/fd/<dirfd>/<name>` with `O_PATH | O_NOFOLLOW | O_CLOEXEC`
  (relative to the directory descriptor already validated and locked, no `unsafe`), checks
  `S_IFSOCK` with `fstat` on that descriptor, then `chmod`s `/proc/self/fd/<n>`, which resolves to
  the pinned inode. A symlink is opened as itself by `O_PATH | O_NOFOLLOW` and is refused by the
  type check; names with more than one component are refused before any open. Without `/proc`
  the fallback fails closed.
- Unit checks on the development host (systemd 262), with `ExecStart` replaced by `/usr/bin/true`
  in a scratch copy: `systemd-analyze verify` exits 0; `systemd-analyze security --offline=yes`
  exposure 7.5 (before) → 1.7 (after). Every syscall group the daemon needs is inside
  `@system-service` (`@memlock` for `mlockall`, `@chown`, `ioctl`, `sched_setaffinity`,
  `mbind`, `clone3`, `rseq`), checked with `systemd-analyze syscall-filter`.
- `/proc/<pid>/cgroup` is mode 0444 and has no ptrace access check, so dropping `CAP_SYS_PTRACE`
  from the bounding set does not affect the session policy as long as `ProtectProc=` and
  `ProcSubset=` stay unset (asserted by `test_unit_keeps_peer_proc_and_camera_access`).

## 5. Not verified without hardware / a real host

- Camera capture, ONNX Runtime inference and `mlockall` under the new sandbox (no V4L2 device
  or deployed models were exercised under systemd here).
- `Before=display-manager.service` orders start-up only; with `Type=simple` systemd considers the
  daemon started when it is forked, not when the socket listens. A `Type=notify` readiness
  signal is a follow-up.
- Whether `CAP_DAC_OVERRIDE` can be dropped (all state is `root:root`, so it should be unneeded).

## 6. Test change proposal (not applied)

`DeviceAllow=/dev/video* rw` is kept only because `systemd_test::test_systemd_unit_file_sandboxing_directives`
and `hardening_tests::test_systemd_hardening_directives_complete` assert that literal. Proposal:
replace `"DeviceAllow=/dev/video* rw"` with `"DeviceAllow=char-video4linux rw"` in both tests and
drop the no-op line from the unit.

## 7. Gate

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast` (1110 passed, 0 failed)
and `./scripts/candid_review.sh` pass.
