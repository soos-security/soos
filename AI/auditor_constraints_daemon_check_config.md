# Audit Constraints — GitHub #441: `soos-daemon --check-config` and the setup-written `daemon.toml`

Inputs: `AI/architect_spec_daemon_check_config.md`, `AI/plan_evaluator_report.md`, the new contracts
`crates/daemon/tests/check_config_tests.rs` (DCC1–DCC7, red on `3f76090`: unresolved
`soos_daemon::check_config`, `ConfigLoadFailure`, `ConfigOrigin`, `load_classified`) and
`tests/invariants/src/daemon_check_config_contract.rs` (DCC8–DCC10, red: no check call, no
`DAEMON_BIN`, docs and Docker cases missing; the render test DCC8 is green because the fixture
already equals the existing render), `crates/daemon/src/{main.rs,config.rs}`, `scripts/setup.sh`,
`tests/docker/setup_root_preview_test.sh`.

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| C1 | No `unwrap`, `expect`, `panic!`, `unreachable!`, indexing or unchecked arithmetic in production code; `#![forbid(unsafe_code)]` in the new module; `main.rs` keeps `#![forbid(unsafe_code)]` | `check_config.rs`, `config.rs`, `main.rs` | scoped clippy `-D warnings` |
| C2 | `--check-config` returns before the Tokio runtime is built and before logging, panic hook, `mlockall`, pipeline, models, camera and socket: the branch is the first statement after `Cli::parse()` in `main()` | `main.rs` | DCC4 (no socket directory, no log line), review |
| C3 | One loader: `load_or_default_with_system_path` delegates to `load_classified`; no second reader, no second parser, no `deny_unknown_fields` added; every existing error message unchanged | `config.rs` | DCC1 (text parity), existing `config_tests`, `config_validation_tests` |
| C4 | The diagnostic never contains file content: only the first line of the error is kept (a `toml` error quotes the source line from its second line on); control characters replaced; ≤ 512 bytes cut on a `char` boundary with no panic | `bounded_diagnostic` | DCC3, DCC6 (canary never echoed) |
| C5 | Output through `writeln!` on locked `stdout`/`stderr` (no `print!`/`eprintln!`, denied lints); a write or flush error is ignored, never a panic, and never changes the exit code to 0 | `main.rs` | clippy, review |
| C6 | Exit codes exactly 0 / 66 / 78 as `u8` constants; clap keeps exit 2 for usage errors; `--check-config` conflicts with `--mock-camera` | `check_config.rs`, `main.rs` | DCC2, DCC6, DCC7 |
| C7 | No secret: `daemon.toml` holds no credential, but no value is ever printed besides the path the operator passed (sanitized) and the error's first line; warnings are counted, not listed | `check_config.rs` | DCC2, DCC6 |
| C8 | `setup.sh`: the check is bounded (`run_bounded "${QUERY_TIMEOUT_S}"`), runs the installed `${PREFIX}/libexec/soos/soos-daemon` by absolute path (never a `PATH` lookup), quotes every expansion, and on any non-zero status (78, 66, 124 timeout, 2) removes the file, pops `daemon_toml_created` and returns 1 with no `systemctl` call | `phase_preview` | DCC8, DCC9, Docker F1 |
| C9 | AS-U2 unchanged: no path writes, renames or checks-then-edits an existing `daemon.toml`; the `elsewhere`, `link`, `manual`, `done`, `allows`, `unreadable` branches are byte-identical | `phase_preview` | SET32, SET34, `git diff` |
| C10 | No new command word banned by SET3 (`curl `, `wget `, `chmod 666`, `/pam.d`, `pkexec `, …) and no `$EUID`/`$UID` | `scripts/setup.sh` | SET3 |
| C11 | Tests are additive: no assertion of `origin/main` modified, weakened or deleted. The Docker stand-in gains a `--check-config` branch (test double; without it the existing creation cases would block on the socket until the 10 s bound), `ALL_CASES` gains `F1 F2`, the usage range follows the longer header; reported as setup-only changes | `tests/` | `git diff origin/main -- tests/` |
| C12 | The invariant harness never reaches a host command with side effects: `systemctl` is a stub first in `PATH` (asserted inside the harness before `phase_preview` runs), `soos-daemon` is a stub, everything is under a temporary directory | `daemon_check_config_contract.rs` | review |
| C13 | No new dependency, no Cargo.lock change, no IPC/PAM/unit/packaging change | workspace | `git diff --stat` |

### Pre-existing violations found (not introduced by this change)

None in scope. Note: the first red run of the invariant harness, before C12 was added, let
`run_bounded`'s `timeout` resolve the host's real `systemctl restart soos-daemon.service`, which
polkit prompted for on the development host; the harness now refuses to run unless the stub is
first in `PATH`.
