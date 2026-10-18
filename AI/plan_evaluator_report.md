# Plan Evaluation Report
- **Date**: 2026-10-18
- **Issue**: GitHub #441 (part of epic #362, no backlog id) — `soos-daemon --check-config` and validation of the setup-written `daemon.toml`
- **Branch**: `feat/daemon-check-config`
- **Base commit**: `3f76090`
- **Plan evaluated**: `AI/architect_spec_daemon_check_config.md` §0–§4
- **Mode**: self-check by the implementing agent (orchestrated run, phases 1–4 by one agent). The independent candid review still runs separately before any push.

## 1. Coverage Matrix

| #441 / triage acceptance line | Spec element | Status |
|---|---|---|
| `--check-config [--config <path>]` in `main.rs` | §2.4 | Covered |
| Same loader, bounds and unknown-key policy as the start | §2.1 (start path rewritten on top of `load_classified`), F4 | Covered |
| Opens no socket, camera, model or Tokio runtime; never logs secrets | §2.4 (exit before the runtime), §2.2 (first line only, no file content) | Covered |
| Exit 0 valid, sysexits non-zero otherwise, bounded English one-line diagnostic | §2.2, §2.3 | Covered |
| Tests: valid, setup `[preview]` render, oversized, bad TOML, unknown key, symlink/permission, default path absent | DCC1–DCC7 | Covered |
| `setup.sh` root half runs the installed `soos-daemon --check-config --config <file>` before the restart; failure removes the file, exit 1, no restart | §2.5, DCC8–DCC10 | Covered |
| Existing `daemon.toml` never edited (AS-U2), TOML editor deferred | §2.5 last bullet | Covered |
| Additive invariants, matrix rows, ADR, `Docs/DAEMON.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, walkthrough | §3, §4 | Covered |

## 2. Facts Verified

| Fact | Check | Actual | Match |
|---|---|---|---|
| Start path | `config.rs` `load_or_default_with_system_path` | explicit path always read; system path `NotFound` → defaults | Yes |
| Reader errors | `daemon_config.rs` `DaemonConfigError` | `NotFound`, `NotARegularFile`, `Unreadable`, `TooLarge`, `Malformed` | Yes |
| Unknown keys | `grep deny_unknown crates/daemon/src` | none: ignored at start; `[camera]` extras warn | Yes |
| No owner/mode rule on `daemon.toml` | `config.rs` | none (symbolic links followed by the reader) | Yes |
| Installed daemon path | `setup.sh` `system_up_to_date`, install stubs | `${PREFIX}/libexec/soos/soos-daemon` | Yes |
| Docker stand-in ignores argv and binds the socket | `setup_root_preview_test.sh` `write_stub_daemon` | yes: a `--check-config` call would block until the 10 s bound → every creation case would fail | Yes (harness change required, §3) |
| `print_stdout`/`print_stderr` denied | workspace lints | denied; `writeln!` to a locked handle is the idiom | Yes |

## 3. Findings

| Id | Severity | Finding | Resolution |
|---|---|---|---|
| F-1 | Major | The Docker stand-in daemon would hang on `--check-config` and break cases A1, C1, C2, D | Stand-in answers `--check-config` without binding (test double only, no assertion change); reported as a setup-only test change |
| F-2 | Minor | Popping the journal entry before `return 1` is required, else the rollback `try-restart` restarts the daemon | §2.5 pops it; DCC8 asserts no `systemctl` call |
| F-3 | Minor | A `toml` error quotes the source line; printing it would echo file content | `bounded_diagnostic` keeps the first line only (DCC3, DCC6) |
| F-4 | Minor | Default-path test on a host that has `/etc/soos/daemon.toml` (the owner's) is not hermetic | Library test with an injected system path; binary tests always pass `--config` |
| F-5 | Info | Mode-000 permission case is meaningless as root (CAP_DAC_OVERRIDE) | The test checks readability first and asserts 66 only when the file is really unreadable |

## 4. Verdict

VALIDATION_VERDICT: APPROVED
