# Audit Constraints — Push Sender as a Sandboxed System Service

- **Date**: 2026-10-23
- **Branch**: `fix/push-sender-system-service`
- **Inputs**: `AI/architect_spec_push_sender_system_service.md`, `AI/plan_evaluator_report.md` (APPROVED),
  tester contract commit `521b559` (PSS3–PSS13 red).

## Audit Constraints

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| A1 | No `unwrap`/`expect`/`panic!`/indexing/unchecked arithmetic in the new production code; the uid parser uses `checked_mul`/`checked_add` or `str::parse::<u32>` after a digit-only check | `push-sender/src/lib.rs::parse_run_mode`, `evaluate_system_sandbox`, `listener_from_fd`; `remote/src/push.rs` | workspace lints (`clippy -D warnings`), RMC-S36 |
| A2 | `#![forbid(unsafe_code)]` stays in `push-protocol`, `push-sender`, `remote`; the inherited socket is obtained with `stdin().as_fd().try_clone_to_owned()` and `UnixListener::from(OwnedFd)`, never `from_raw_fd` | `push-sender/src/main.rs::run_system_instance`, `lib.rs::listener_from_fd` | PSS9, RMC-S33 forbid check |
| A3 | No new environment read in the sender (only `XDG_RUNTIME_DIR` in `main.rs`); no `LISTEN_FDS`/`LISTEN_PID`; the library reads none | sender sources | RSH12, PSS9 |
| A4 | Every `tracing` macro of the sender has exactly one constant string (no uid, path or peer value in a log line) | `push-sender/src/main.rs` | RMC-S36 |
| A5 | Fail closed before serving: arguments, identity (root or the instance uid), sandbox, socket type each exit 78 (`EXIT_CONFIG`, `RestartPreventExitStatus=78`) | `main.rs::main`, `run_system_instance` | PSS8, PSS9 |
| A6 | Peer authentication: `serve_connection(stream, allowed_uid, …)` is the only serving path of the system instance; the instance uid comes only from argv (`%i`, set by root through the unit instance name) | `main.rs::run_system_instance` | PSS9 (order), unchanged `serve_connection` tests |
| A7 | `listener_from_fd` accepts only `AF_UNIX` + `SOCK_STREAM` + `SO_ACCEPTCONN`; anything else is `SocketError::Io` and the fd is dropped | `lib.rs::listener_from_fd` | PSS7 |
| A8 | The system sandbox check reads the mount table with the existing bound (`MAX_MOUNTINFO_BYTES`, `take`) and never follows the environment or the binary path | `lib.rs::probe_system_sandbox` | PSS6, code review |
| A9 | Client: per exchange the first existing candidate only; owner check on the inode and peer uid equal to the candidate's `peer_uid`; every refusal is `Unavailable`; no new log line in `push.rs` | `remote/src/push.rs::UnixPushTransport::exchange` | PSS10, RMC-S37 |
| A10 | `push.rs` reads no environment; the derived default path is passed in by `main.rs` (which already reads `XDG_RUNTIME_DIR`) | `remote/src/main.rs::push_wiring` | RMC env invariant (`main.rs` reads only `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME`, `HOME`) |
| A11 | Unit: `DynamicUser=yes`, no capability, no `User=`/`PrivateUsers=`, `StandardOutput=journal`/`StandardError=journal` with `StandardInput=socket`; socket `0600` owned by `%i`, directories `0755` root | `packaging/soos-push-sender@.{socket,service}` | PSS1, Docker PSS14 |
| A12 | `install.sh`: the new files are journaled through `tracked_install` (rolled back on failure), installed only when the artifact exists; nothing is enabled; `--dry-run` exits before any change (existing early exit) | `scripts/install.sh` | PSS11, existing installer tests |
| A13 | `uninstall.sh`: instances are disabled only on a live install with systemd (`-z DESTDIR`, `--skip-systemd` honoured), file names come from a glob of the wants directory and are matched against `soos-push-sender@<digits>.socket` before use; never follows a symlink outside; dry run exits first | `scripts/uninstall.sh` | PSS11, shellcheck |
| A14 | `setup.sh`: the `systemctl enable --now` of the socket runs only in the root half, bounded by `run_bounded`, journaled for rollback; the uid in the unit name is the resolved numeric `TARGET_UID`; the companion runs as the user and only disables the legacy user unit (journaled, re-enabled on rollback); `--dry-run` only queries | `scripts/setup.sh` | PSS12, SET dry-run invariants, shellcheck |
| A15 | No secret logged anywhere (push bodies, authorization, endpoints) — unchanged code paths | sender, remote | RMC-S37, RMC-S41 |

### Pre-existing violations found (not introduced by this change)

None found in the touched functions.

### Clearance: CLEARED
