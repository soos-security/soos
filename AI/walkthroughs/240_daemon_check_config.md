# Walkthrough 240 — `soos-daemon --check-config` and Setup Validation of the Created `daemon.toml`

- **Date**: 2026-10-18
- **Issue**: GitHub #441, part of epic #362 (GitHub-only, no backlog id) — **Branch**: `feat/daemon-check-config`
- **Decision**: ADR 2026-10-18 "`soos-daemon --check-config` and Setup Validation of the Created `daemon.toml`", amending items (7) and (11) of ADR 2026-10-15 "Setup User Phase"
- **Matrix criteria**: DCC1–DCC10 (new, verified), DCC11 (owner hardware, pending). SET20–SET39 unchanged.

## 1. Context & Objectives

ADR 2026-10-15 deferred `soos-daemon --check-config`. Until now `scripts/setup.sh` created
`/etc/soos/daemon.toml` (the `[preview]` section of the phone camera view), restarted the daemon on it
and only then found out, through the readiness gate, whether the daemon accepted the file. The goals:

- a configuration check that accepts a file exactly when the daemon would start with it, and opens
  nothing;
- a setup root half that checks the file it created with the installed daemon before restarting, and
  on a refusal removes it without touching the running daemon.

An existing `daemon.toml` is still never edited (AS-U2); the bounded TOML editor stays deferred.

## 2. Architect Design

- `config.rs`: `DaemonConfig::load_classified(path_opt, system_path) -> Result<(DaemonConfig,
  ConfigOrigin), ConfigLoadFailure>`. `load_or_default_with_system_path` (the start path) now
  delegates to it, so the start and the check share one loader and one error text.
  `ConfigLoadFailure::Unavailable` covers the reader's `NotFound`, `NotARegularFile` and
  `Unreadable`; `Invalid` covers `TooLarge`, `Malformed` and every parse or validation error.
- `check_config.rs` (`#![forbid(unsafe_code)]`): `check_config` / `check_default_config` return a
  `CheckReport { exit_code, stream, line }`; `bounded_diagnostic` keeps the first line, replaces
  control characters and bounds the line to 512 bytes.
- `main.rs`: `--check-config` (conflicts with `--mock-camera`). `main()` parses the CLI first and,
  with the flag, writes the line and exits before the runtime is built.
- Exit codes: 0 `EX_OK`, 66 `EX_NOINPUT`, 78 `EX_CONFIG`; clap keeps 2.
- `setup.sh`: `DAEMON_BIN="${PREFIX}/libexec/soos/soos-daemon"`; in `phase_preview`, after
  `JOURNAL+=(daemon_toml_created)` and before the restart, `run_bounded "${QUERY_TIMEOUT_S}"
  "${DAEMON_BIN}" --check-config --config "${DAEMON_TOML}"`. On failure: `[FAIL]` line, file removed,
  journal entry popped (its rollback would `try-restart`), `return 1` → `fail_and_rollback`, exit 1.

## 3. Plan Evaluation

Self-check (`AI/plan_evaluator_report.md`, APPROVED). Main finding F-1: the real-root Docker
harness's stand-in daemon ignored its arguments, so a `--check-config` call would have bound the
socket and blocked until the 10 s bound, failing every creation case. The stand-in now answers
`--check-config` before anything else (test double only).

## 4. Tester Contract (red first)

- `crates/daemon/tests/check_config_tests.rs` (DCC1–DCC7): classified loader with start-path text
  parity; library reports for each class and for an absent system path (injected, so the host's own
  `/etc/soos/daemon.toml` never matters); diagnostic bounds; the real binary on a valid file (no
  socket directory created, no log line), on the setup fixture, and on oversized, bad TOML (a canary
  key is never echoed), wrong type, invalid value, unknown keys (accepted, like the start), missing,
  directory, FIFO, dangling link, link to a valid file and mode 000; the usage conflict. Red: the
  module and the loader did not exist.
- `tests/invariants/src/daemon_check_config_contract.rs` (DCC8–DCC10): the setup render equals
  `tests/fixtures/setup/daemon.toml`; the real `phase_preview` and its helpers are extracted
  verbatim from `setup.sh` and run without root with a stub daemon (exit 78 or 0) and a stub
  `systemctl`; static order and traceability; Docker wiring. Red: no check call, no `DAEMON_BIN`,
  no docs or Docker cases. The render test was green from the start (the fixture is the existing
  render).
- `tests/docker/setup_root_preview_test.sh`: cases F1 (check refused) and F2 (check before the
  restart) added to the default list; the stand-in records each check in
  `/var/lib/soos-setup-test/check-config.log` and refuses in the new mode `check-refuses`.

Incident during the red run: the first harness version defined `systemctl` as a shell function,
but `run_bounded` executes `timeout systemctl …`, which looks `systemctl` up in `PATH`. The host's
real `systemctl restart soos-daemon.service` was therefore invoked; polkit asked for authorization
on the development host and the host daemon was restarted once (it came back active with its own
unchanged configuration; nothing was written under `/etc`). The harness now puts a stub
`systemctl` first in `PATH` and aborts (exit 99) unless `command -v systemctl` resolves to it.

## 5. Auditor Constraints

`AI/auditor_constraints_daemon_check_config.md` C1–C13: no panic path, `forbid(unsafe_code)`, the
check returns before the runtime, one loader, no file content in the diagnostic, `writeln!` with
ignored write errors, exact exit codes, bounded absolute-path call in `setup.sh` with no restart on
refusal, AS-U2 untouched, no SET3 word, additive tests only, a harness that cannot reach a host
command, no new dependency.

## 6. Developer Implementation

- `crates/daemon/src/config.rs`: `ConfigOrigin`, `ConfigLoadFailure` (with `error`, `into_error`,
  private `from_read_error`), `load_classified`; `load_or_default_with_system_path` rewritten on top.
- `crates/daemon/src/check_config.rs` (new), `crates/daemon/src/lib.rs` (`pub mod check_config`).
- `crates/daemon/src/main.rs`: the flag, `run_check_config`, `run(cli)` takes the parsed CLI.
- `scripts/setup.sh`: `DAEMON_BIN` and the check block in `phase_preview`.

## 7. Verification

Scoped runs with `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3` and
`--test-threads=3`:

- `cargo test -p soos-daemon --test check_config_tests` — 7 passed.
- `cargo test -p soos-daemon` — every suite green (existing `config_tests`,
  `config_validation_tests` and the other configuration suites unchanged).
- `cargo test -p soos-invariants` — 792 passed (matrix citations, daemon docs contract, the setup
  contracts SET1–SET39 and the new DCC8–DCC10 included).
- `cargo fmt --check`, `cargo clippy -p soos-daemon -p soos-invariants --all-targets -- -D warnings`
  and `shellcheck -x scripts/setup.sh tests/docker/setup_root_preview_test.sh` — clean.
- `bash tests/docker/setup_root_preview_test.sh` (real root, real systemd, pinned image) — cases
  A1, A2, B, C1, C2, D, E1, E2, F1 and F2 passed, host sysctls unchanged. A first attempt stopped at
  the harness's own `--in-container` guard before any case ran (`/proc/1/comm` read right after the
  container start); the re-run passed unchanged.
- Read-only on the development host: the built binary reports the host's `/etc/soos/daemon.toml`
  valid (exit 0). This is not the owner check DCC11, which stays pending: `sudo
  /usr/libexec/soos/soos-daemon --check-config` with the installed binary, a broken copy with
  `--config`, and a setup run that creates the file.

## 8. Scope Kept Out

Packaging `soos-setup` as `/usr/bin/soos-setup` (#361 Q3/Q5), editing an existing `daemon.toml` (the
bounded TOML editor), PAM activation, the journal group and linger.
