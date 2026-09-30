# Walkthrough 140 — Daemon `Type=notify` Readiness and Residual Daemon Hardening Items

- Branch: `fix/p2-daemon-hardening`
- Issues: GitHub #203 (DMN-14), #201 (DMN-12), #205 (DMN-16), from `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`
- Matrix: component `daemon-systemd-readiness`, rows DHX1–DHX5
- ADR (`AI/DECISIONS.md`, 2026-09-30): "Daemon Unit Uses `Type=notify` Readiness"

## 1. Starting point

Most of the three findings were already resolved on `main`:

- #203: the least-privilege sandbox (capability bounding set, syscall filter, kernel / namespace /
  realtime protections, private network, `DeviceAllow=char-video4linux rw`,
  `Before=display-manager.service`) landed in walkthrough 111.
- #201: `mlockall` refusal logged at `warn`, recorded in `HealthState::memory_locked`, and
  `Docs/MEMORY_PROTECTION_AND_SWAP.md` corrected to present `LockedBuffer` as "available, not
  wired" (walkthrough 113, rows DMW1/DMW2).
- #205: the running daemon uses one `warmup_frames` value, `DAEMON_DEFAULT_WARMUP_FRAMES` = 0,
  with or without `daemon.toml` (walkthrough 113, rows DMW4/DMW5).

The remaining #203 defect (walkthrough 111 §5): with `Type=simple`, systemd considers the daemon
started as soon as it is forked, so `Before=display-manager.service` does not guarantee that
`/run/soos/daemon.sock` listens when GDM shows its first prompt. Model verification and the
inference warm-up take time before the bind, and the first login silently fell back to the password.

## 2. Design

- `crates/daemon/src/sd_notify.rs` (safe Rust, no new dependency): `notify_to(Option<&OsStr>, &str)`
  implements the `sd_notify(3)` datagram protocol. It accepts only an absolute path or a non-empty
  abstract `@name` of at most `MAX_NOTIFY_SOCKET_PATH_LEN` = 107 bytes, sends one single-line
  `KEY=VALUE` datagram from an unbound `AF_UNIX` socket with `NOTIFY_WRITE_TIMEOUT` = 1000 ms, and
  returns `NotifyOutcome::NotSupervised` when no socket is given. `notify_ready()` /
  `notify_stopping()` read `NOTIFY_SOCKET`.
- `crates/daemon/src/main.rs`: `READY=1` after `bind_socket` and before the accept loop;
  `STOPPING=1` on graceful shutdown. Failures are logged at `warn` and never stop the daemon.
- `packaging/soos-daemon.service`: `Type=notify`, `NotifyAccess=main`, `TimeoutStartSec=60`.
  A start that never reports readiness fails after 60 s; `Restart=on-failure` with
  `StartLimitBurst=5` bounds the retries.
- Sandbox compatibility: the notify socket is a filesystem path (`/run/systemd/notify`), reachable
  under `PrivateNetwork=yes`, `RestrictAddressFamilies=AF_UNIX`, `SystemCallFilter=@system-service`
  and `ProtectSystem=strict` (sending to a socket inode is not a filesystem write).

## 3. Tests first (red evidence)

New files: `crates/daemon/tests/sd_notify_tests.rs` (8 tests) and
`crates/daemon/tests/systemd_readiness_tests.rs` (4 tests).

Before the implementation (`cargo test --locked -p soos-daemon --all-features --test ...`):

- `sd_notify_tests`: did not compile — `unresolved import soos_daemon::sd_notify`.
- `systemd_readiness_tests`: 3 of 4 failed (`test_unit_uses_notify_readiness`,
  `test_unit_start_timeout_is_explicit_and_bounded`, `test_daemon_reports_ready_only_after_socket_bind`);
  `test_unit_keeps_display_manager_ordering` passed (regression guard).

After: all 12 pass, together with the unchanged `systemd_hardening_tests`, `systemd_test` and
`hardening_tests`.

Unit check on the development host, with `ExecStart` replaced by `/usr/bin/true` in a scratch copy:
`systemd-analyze verify` exits 0 and `systemd-analyze security --offline=yes` still reports 1.7.

## 4. Audit

- No `unsafe`, no `unwrap` / `expect` in production code; `main.rs` keeps `#![forbid(unsafe_code)]`.
- Bounded: one fixed datagram, input length checked before building the address, write timeout.
- Only fixed keywords are sent; nothing derived from requests, frames, embeddings or keys is sent or
  logged. The IPC socket, its mode and the PAM module are untouched.

## 5. Open items (need a user decision or a real host)

- **#201 Status field**: exposing `memory_locked` in `StatusResponse` changes the Postcard layout and
  breaks the existing struct literals in `crates/admin-cli/tests/status_tests.rs`,
  `crates/admin-cli/tests/wire_tag_tests.rs` and `crates/protocol/tests/property_tests.rs` (row DMW3,
  DHX5). Not done.
- **#205 `DaemonConfig::default()`**: still 20 while the running daemon uses
  `DAEMON_DEFAULT_WARMUP_FRAMES` = 0. Aligning either way changes an existing test:
  `config_tests::test_pipeline_default_sensor_preference_is_prefer_ir` pins 20 for `default()`, and
  `config_tests::test_daemon_toml_default_warmup_frames_is_zero` plus
  `warmup_default_tests::test_daemon_default_warmup_frames_constant_is_zero` pin 0 for the runtime.
  Switching the runtime to the camera default 20 (safer for IR auto-exposure after standby, costs
  wake latency) is a user decision.
- **Real host**: GDM ordering with the notify handshake, camera capture, ORT threads and `mlockall`
  under the sandbox (DHX4); whether `CAP_DAC_OVERRIDE` can be dropped.

## 6. Gate

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast` and
`./scripts/candid_review.sh`.
