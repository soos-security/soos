# Architect Spec — `soos-daemon --check-config` and Validation of the Setup-Written `daemon.toml` (GitHub #441, part of #362)

- **Date**: 2026-10-18
- **Issue**: GitHub #441 (sub-issue of epic #362; no backlog id, so the branch is not registered in
  `scripts/sync_issue.py`; the commit carries `Closes #441` and `Refs #362`)
- **Branch**: `feat/daemon-check-config`
- **Base commit**: `3f76090` (`origin/main`)
- **Origin**: deferred item of ADR 2026-10-15 "Setup User Phase" item (11) ("`soos-daemon
  --check-config` and the TOML editor"); item (7) said an existing `daemon.toml` is never edited
  "until the bounded TOML editor and `soos-daemon --check-config` exist".
- **Lane**: `crates/daemon` (binary entry point, one new library module, a classified loader in
  `config.rs`), `scripts/setup.sh` (root half, `phase_preview` only), additive tests
  (`crates/daemon/tests`, `tests/invariants`, `tests/docker/setup_root_preview_test.sh`), one test
  fixture, `Docs/`, `AI/`. No new dependency, no IPC, PAM, packaging or unit change.

## 0. Owner decisions

Relied on (recorded, none new):
- 2026-10-18 standing rules: work up to an opened pull request; never merge; hardware checks are
  the owner's and are recorded as pending matrix rows.
- ADR 2026-10-15 item (7) and AS-U2: an existing `daemon.toml` is never edited. Unchanged here.
- Triage of #362 for this item: sysexits-style exit codes, one bounded English diagnostic line,
  the same loader, bounds and unknown-key policy as the daemon start.

No open owner question: the exit-code classes are the sysexits meanings (§2.3), the unknown-key
policy is the existing one (§1, F4), and the bounded TOML editor stays deferred.

## 1. Facts (verified on `3f76090`)

| # | Fact | Source |
|---|---|---|
| F1 | `main()` builds the Tokio runtime, then `run()` calls `Cli::parse()` and `DaemonConfig::load_or_default(cli.config)` | `crates/daemon/src/main.rs` |
| F2 | `load_or_default` → `load_or_default_with_system_path(path_opt, DEFAULT_CONFIG_PATH)`: an explicit path is always read (missing = error); otherwise the system path is read and only `NotFound` falls back to `runtime_default()` | `crates/daemon/src/config.rs` 795–822 |
| F3 | Both paths use `soos_camera_v4l::daemon_config::read_daemon_config_text` (O_PATH pin, regular file, ≤ `MAX_DAEMON_CONFIG_BYTES` = 1 MiB, UTF-8, symbolic links followed); errors `NotFound`, `NotARegularFile`, `Unreadable(kind)`, `TooLarge{limit}`, `Malformed` | `crates/camera-v4l/src/daemon_config.rs` |
| F4 | Unknown keys: none of the `*ConfigFile` structs uses `deny_unknown_fields`, so an unknown key or table is ignored silently at start; unknown keys inside `[camera]` become `DaemonConfig::warnings` | `config.rs` 315–400, 708–725; `Docs/DAEMON.md` §1 |
| F5 | `from_toml_str` parses, applies, and runs `validate()` (socket mode, timeouts, thresholds floor, preview, peer limits, presence); errors are `DaemonError::Config(String)`, displayed `Configuration error: …` | `config.rs`, `error.rs` 82–84 |
| F6 | No owner, mode or symbolic-link policy is enforced on `daemon.toml` itself (only the reader rules of F3) | `config.rs`, `daemon_config.rs` |
| F7 | A `toml` 0.8 parse error displays a first line `TOML parse error at line L, column C`, then lines quoting the source line and the message | `toml` 0.8.23 (`Cargo.lock`) |
| F8 | `print_stdout` / `print_stderr` are denied workspace lints; CLIs write with `writeln!(std::io::stdout().lock(), …)` | `Cargo.toml` `[workspace.lints.clippy]`, `crates/lockstatus/src/main.rs` |
| F9 | `phase_preview` creates `${SYSCONFDIR}/soos/daemon.toml` (temporary file, 0644, hard link), pushes `daemon_toml_created` on `JOURNAL`, restarts the unit and waits (`daemon_ready`); on failure removes the file, pops the journal entry, restarts again; `run_system_phase` maps a failure to `fail_and_rollback` (exit 1) or exit 70 when the daemon stays down | `scripts/setup.sh` 1613–1666, 1802–1809 |
| F10 | The creation path runs only when `DAEMON_TOML == /etc/soos/daemon.toml`; the fake-`--sysconfdir` invariant harness never reaches it; the real-root path is `tests/docker/setup_root_preview_test.sh` (required CI job `setup-root-preview`), whose `/usr/libexec/soos/soos-daemon` is a Python stand-in that ignores its arguments and, run directly, would bind the socket and block | `setup_user_phase_contract.rs` u16/u24, Docker script 194–266 |
| F11 | `install.sh` installs the daemon at `${PREFIX}/libexec/soos/soos-daemon`; `setup.sh` already compares that path in `system_up_to_date` | `scripts/setup.sh` 884 |
| F12 | The integration tests of `soos-daemon` can run the binary through `env!("CARGO_BIN_EXE_soos-daemon")` | Cargo |

## 2. Design

### 2.1 Classified loader (`crates/daemon/src/config.rs`)

```rust
/// Where a loaded configuration came from (GitHub #441).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOrigin {
    /// Read from this file (`--config`, or the system path when present).
    File(PathBuf),
    /// No file at the system path: the runtime defaults apply.
    RuntimeDefault(PathBuf),
}

/// Why a configuration was refused, by class (GitHub #441).
#[derive(Debug)]
pub enum ConfigLoadFailure {
    /// The file could not be opened or read: missing, not a regular file, unreadable.
    Unavailable(DaemonError),
    /// The file was read but refused: too large, not UTF-8, does not parse or validate.
    Invalid(DaemonError),
}

impl ConfigLoadFailure {
    pub fn error(&self) -> &DaemonError;
    pub fn into_error(self) -> DaemonError;
}

impl DaemonConfig {
    pub fn load_classified(path_opt: Option<&Path>, system_path: &Path)
        -> Result<(Self, ConfigOrigin), ConfigLoadFailure>;
}
```

- `load_or_default_with_system_path` becomes `load_classified(..).map(|(c, _)| c)
  .map_err(ConfigLoadFailure::into_error)`, so the start path and the check path are the same code
  and every existing error message is unchanged (same `config_read_error` and `from_toml_str`).
- Reader classes: `NotFound`, `NotARegularFile`, `Unreadable` → `Unavailable`; `TooLarge`,
  `Malformed` → `Invalid`; any `from_toml_str` error → `Invalid`. `NotFound` at the system path with
  no `--config` → `Ok((runtime_default(), RuntimeDefault(system_path)))`.

### 2.2 Check module (`crates/daemon/src/check_config.rs`, `#![forbid(unsafe_code)]`)

```rust
pub const EXIT_CONFIG_VALID: u8 = 0;        // EX_OK
pub const EXIT_CONFIG_UNAVAILABLE: u8 = 66; // EX_NOINPUT
pub const EXIT_CONFIG_INVALID: u8 = 78;     // EX_CONFIG
pub const MAX_CHECK_DIAGNOSTIC_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStream { Stdout, Stderr }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport { pub exit_code: u8, pub stream: CheckStream, pub line: String }

pub fn check_config(path_opt: Option<&Path>, system_path: &Path) -> CheckReport;
pub fn check_default_config(path_opt: Option<&Path>) -> CheckReport; // DEFAULT_CONFIG_PATH
pub fn bounded_diagnostic(text: &str) -> String;
```

Lines (all through `bounded_diagnostic`, prefix `soos-daemon: `):
- file valid → stdout, exit 0: `soos-daemon: configuration '<path>' is valid`, plus
  ` (<n> warning(s), logged at daemon start)` when `warnings` is not empty;
- no file at the system path → stdout, exit 0: `soos-daemon: no configuration file at '<path>';
  the built-in defaults are valid`;
- `Unavailable` → stderr, exit 66; `Invalid` → stderr, exit 78:
  `soos-daemon: configuration refused: <error>`.

`bounded_diagnostic`: keeps only the text before the first `\n` (a `toml` error's source quote is
never printed, F7), replaces every other control character with `?`, and truncates to at most
`MAX_CHECK_DIAGNOSTIC_BYTES` bytes on a `char` boundary, ending with `...` when cut.

### 2.3 Exit codes (sysexits)

| Code | Name | When |
|---|---|---|
| 0 | `EX_OK` | the file (or, without one, the defaults) loads and validates exactly as at start |
| 66 | `EX_NOINPUT` | missing explicit file, dangling link, directory/FIFO/device, permission denied |
| 78 | `EX_CONFIG` | larger than 1 MiB, not UTF-8, TOML error, wrong type, validation error |
| 2 | clap usage | unknown option, `--check-config` with `--mock-camera` |

### 2.4 Entry point (`crates/daemon/src/main.rs`)

- New flag `#[arg(long, conflicts_with = "mock_camera")] check_config: bool` documented "Validate
  the configuration file and exit (no socket, camera, model or runtime)".
- `main()` parses the CLI first; with `--check-config` it calls
  `check_default_config(cli.config.as_deref())`, writes the line to its stream, flushes, and
  `std::process::exit(code)` — before any Tokio runtime, logging, panic hook, `mlockall`, pipeline,
  model, camera or socket. Without the flag, `run(cli)` is unchanged.

### 2.5 `scripts/setup.sh` root half

- `DAEMON_BIN="${PREFIX}/libexec/soos/soos-daemon"` next to `ADMIN_BIN`.
- In `phase_preview`, right after `JOURNAL+=(daemon_toml_created)` and before the `info
  "Restarting …"` line:

  ```bash
  if ! run_bounded "${QUERY_TIMEOUT_S}" "${DAEMON_BIN}" --check-config --config "${DAEMON_TOML}"; then
      printf '[FAIL] camera view (daemon): soos-daemon --check-config refused %s (removed again; %s not restarted)\n' "${DAEMON_TOML}" "${UNIT}"
      rm -f -- "${DAEMON_TOML}"
      unset 'JOURNAL[${#JOURNAL[@]}-1]'
      return 1
  fi
  ```

  `return 1` with `PREVIEW_DAEMON_DOWN=false` reaches `fail_and_rollback "camera view (daemon)"`:
  exit 1, the rest of the journal rolled back (`/etc/soos` removed when the run created it), and no
  restart (the `daemon_toml_created` entry, whose rollback runs `try-restart`, was popped).
- Nothing else changes: an existing file is never edited (AS-U2), the `elsewhere`, `link`,
  `manual`, `done` and `allows` paths are untouched, and a timeout (124) is a refusal.

## 3. Tests (TDD red first)

| Id | Test | Where |
|---|---|---|
| DCC1 | classified loader: `Unavailable` for missing explicit file / directory / dangling link; `Invalid` for oversized / non-UTF-8 / bad TOML / validation error; `RuntimeDefault` for an absent system path; same error text as `load_or_default_with_system_path` | `crates/daemon/tests/check_config_tests.rs` |
| DCC2 | `check_config` library: exit, stream and line for each class; warnings counted; default path absent | same |
| DCC3 | `bounded_diagnostic`: first line only, control characters replaced, ≤ 512 bytes on a char boundary | same |
| DCC4 | binary: valid file → 0, one stdout line, empty stderr, no socket created under the configured `socket_dir`, no log line | same |
| DCC5 | binary: setup render fixture `tests/fixtures/setup/daemon.toml` → 0 and its `[preview]` values load | same |
| DCC6 | binary: oversized → 78; bad TOML → 78 without the source line; unknown key → 0 (start parity); invalid value → 78 (start parity); missing / directory / FIFO / dangling link / mode 000 (non-root) → 66; link to a valid file → 0 | same |
| DCC7 | binary: `--check-config --mock-camera` → exit 2 | same |
| DCC8 | setup render (function extracted from `setup.sh`) is byte-equal to the fixture; `phase_preview` extracted with stubs: refusal → file removed, journal entry dropped, no `systemctl`, return 1; success → check before `systemctl restart` | `tests/invariants/src/daemon_check_config_contract.rs` |
| DCC9 | static: `DAEMON_BIN` literal, bounded check call before the restart, failure message; docs, ADR, matrix, walkthrough 240 | same |
| DCC10 | Docker root cases F1 (refusal: exit 1, file and created `/etc/soos` removed, no restart, daemon healthy, companion not run) and F2 (check on the created file before the restart) wired | same + `tests/docker/setup_root_preview_test.sh` |
| DCC11 | Owner hardware: on the Arch host, `sudo /usr/libexec/soos/soos-daemon --check-config` on the real `/etc/soos/daemon.toml`, and a setup run that creates the file | ⬜ Pending |

Docker harness change (test double + new cases, no existing assertion touched): the stand-in
daemon answers `--check-config` (records the call in `${TEST_DIR}/check-config.log`, exits 78 in
the new mode `check-refuses`, 0 otherwise, never binds the socket); `ALL_CASES` gains `F1 F2`.
Without the stand-in change the existing cases would hang (F10), so it is required setup.

## 4. Traceability

- ADR `[2026-10-18] soos-daemon --check-config and Setup Validation of the Created daemon.toml`
  amending items (7) and (11) of ADR 2026-10-15.
- Matrix component `daemon-check-config`, rows DCC1–DCC11 (DCC11 pending, owner hardware).
- `Docs/DAEMON.md` §1.9 (new), `Docs/PACKAGING_AND_PROVISIONING.md` camera-view row and evidence
  paragraph, walkthrough `AI/walkthroughs/240_daemon_check_config.md`.
