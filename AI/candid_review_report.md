# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/daemon-check-config`
- **Base (merge-base)**: `3f76090`
- **Reviewed-Diff-Fingerprint**: `f16a70df61b4de1216a2cb7067590028ab9e0ed2ad73a20a2068938e060c6bfe`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_daemon_check_config.md`, `AI/auditor_constraints_daemon_check_config.md`, `AI/walkthroughs/240_daemon_check_config.md`, `Docs/DAEMON.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `crates/daemon/src/check_config.rs`, `crates/daemon/src/config.rs`, `crates/daemon/src/lib.rs`, `crates/daemon/src/main.rs`, `crates/daemon/tests/check_config_tests.rs`, `scripts/setup.sh`, `tests/docker/setup_root_preview_test.sh`, `tests/fixtures/setup/daemon.toml`, `tests/invariants/src/daemon_check_config_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

The change adds `soos-daemon --check-config [--config <FILE>]` (GitHub #441) and makes the setup
root half validate the `daemon.toml` it has just created with the installed daemon before the
restart. The start path `load_or_default_with_system_path` now delegates to the new
`DaemonConfig::load_classified`, and the refactor is behavior-preserving: an explicit path is read
with the same bounded reader and `config_read_error` text as the old `load_from_path`, and only a
`NotFound` at the system path (without `--config`) yields the runtime defaults, as before. The check
returns from `main()` before the Tokio runtime, logging, the panic hook, `mlockall`, the pipeline and
the socket. No pre-existing test assertion was removed or changed. Scoped tests, clippy and fmt are
green. Only MINOR / SUGGESTION findings remain.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `crates/daemon/tests/check_config_tests.rs` (new), `tests/invariants/src/daemon_check_config_contract.rs` (new), `tests/invariants/src/lib.rs` (one `mod` declaration appended, no test touched), `tests/fixtures/setup/daemon.toml` (new fixture), `tests/docker/setup_root_preview_test.sh` (modified).
- Removed or changed `assert` / `#[test]` / `proptest!` / `#[should_panic]` lines: **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: none.
- `tests/docker/setup_root_preview_test.sh`: the only removed lines are `readonly ALL_CASES=(A1 A2 B C1 C2 D E1 E2)` (extended with `F1 F2`, no case dropped) and `sed -n '2,65p'` in `usage()` (header grew to line 74, verified). The stand-in daemon gains a `--check-config` branch that records the call and never starts or binds; this is a test-double change required so the existing creation cases do not block on the socket. No existing `case_*` assertion is altered. Setup-only, justified by the ADR item (6).
- `git diff $(git merge-base origin/main HEAD) HEAD -- tests 'crates/*/tests' android/app/src/test`: additions only, as listed above.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Old vs new loader: `path_opt = Some(p)` previously called `load_from_path(p)` = `read_daemon_config_text(p).map_err(config_read_error(p, ..))` then `from_toml_str`; new code is identical (DCC1 asserts identical error text for every class). `path_opt = None`: `NotFound` → runtime default, any other read error → error naming the system path, as before. PASS.
- Classification: `NotFound | NotARegularFile | Unreadable` → 66; `TooLarge | Malformed` → 78; parse/validation → 78. Matches `Docs/DAEMON.md` §1.9 table and the ADR. PASS.
- `--mock-camera` only flips `use_mock_camera` after validation, so the conflict does not hide a validation difference; config-level acceptance is exactly the start's. PASS.
- `setup.sh`: the check sits between `JOURNAL+=(daemon_toml_created)` and the restart; on failure the file is removed and the last journal entry (that very entry) is popped, so `fail_and_rollback` does not `try-restart`; `/etc/soos` created by the run is still rolled back via `soos_conf_dir_created` (DCC8 asserts `rc=1 journal=soos_conf_dir_created`). `DAEMON_BIN="${PREFIX}/libexec/soos/soos-daemon"` matches `install.sh` (`TARGET_LIBEXEC_DIR`) and the unit's `ExecStart`. Stale-binary scenario: `phase_system` skips install only when installed files equal the build, so the checked binary is the freshly built one. PASS.

### PAM Concurrency & Deadlines
- No change in `crates/pam`. The setup call is bounded by `run_bounded "${QUERY_TIMEOUT_S}"` (10 s). PASS.

### Panic Safety & Fail-Closed
- `check_config.rs` has `#![forbid(unsafe_code)]`, no `unwrap/expect/panic/indexing` (only `unwrap_or_default`, `saturating_*`). Write failures are ignored; the exit code still carries the result. Any non-zero (including a timeout or a missing binary) removes the file: fail-closed toward "no new config". PASS.

### Test Integrity & Anti-Weakening
- No pre-existing assertion touched (section 2). New tests would fail on plausible wrong implementations: wrong class for directory/FIFO (DCC1/DCC6), falling back to defaults for a directory at the system path (DCC1), quoting the TOML source line (canary in DCC6), socket directory creation (DCC4), check after restart or twice (DCC8). The invariant harness now aborts (exit 99) unless the `systemctl` stub is first in `PATH`, closing the host-`systemctl` reach described in the walkthrough. PASS.

### Memory, Bounds & Secrets
- Input bounded by the existing 1 MiB reader (`MAX_DAEMON_CONFIG_BYTES = 1024 * 1024`, verified). Output bounded to 512 bytes, first line only, control characters replaced, cut on a char boundary (DCC3). No frames, embeddings or credentials involved. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or hook change. Docker harness logic not executed locally (CI job `setup-root-preview`); reviewed by reading: stand-in writes `CHECK_LOG` under `TEST_DIR=/var/lib/soos-setup-test`, matching the test's `readonly CHECK_LOG`; mode `check-refuses` is set after the healthy start in `reset_case`, and an unknown mode in the start branch serves `healthy`. PASS (CI pending).

### English-Only Policy
- Code, comments, docs, commit message in English; no French tokens found in added lines. PASS.

### Doc factual accuracy
- `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 exists; `Docs/DAEMON.md` §1.1–§1.7 are the validated tables and §1.9 is new after §1.8; walkthrough 240 is unique; DCC1–DCC11 rows exist; "bounded by 10 s" equals `QUERY_TIMEOUT_S=10`. PASS (see MINOR 2).

### Commands re-run (scoped, `CARGO_TARGET_DIR=~/.cache/soos-targets/daemon-check-config`, `--test-threads=3`)
- `cargo test -p soos-daemon --test check_config_tests`: 7 passed.
- `cargo test -p soos-daemon` config suites (`config_tests`, `config_validation_tests`, `evidence_global_cap_config_tests`, `inference_threads_config_tests`, `ir_emitter_daemon_config_tests`, `presence_config_tests`, `threshold_config_tests`): all passed.
- `cargo test -p soos-invariants`: 792 passed.
- `cargo clippy -p soos-daemon -p soos-invariants --all-targets -- -D warnings`: clean. `cargo fmt --check`: clean. `bash -n` on both scripts: clean.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/setup.sh:1658` — the `[FAIL]` line says `--check-config refused` for any non-zero status, including a 10 s timeout or a missing/unexecutable binary (exit 124/126/127). Behavior is correct (fail-closed); the wording can mislead an operator. Optional: include the exit status in the message.
- **[MINOR]** `scripts/setup.sh:488` (pre-existing helper) — `run_bounded` runs the command unbounded when `timeout` is absent, so "bounded by 10 s" in `Docs/PACKAGING_AND_PROVISIONING.md` and the ADR holds only where coreutils `timeout` exists (always true on supported distributions). No change required.
- **[SUGGESTION]** `tests/invariants/src/daemon_check_config_contract.rs` `run_phase_preview` — the per-run directory under `std::env::temp_dir()` is removed before use but not after; a `tempfile::TempDir` would avoid leftovers.

## 5. Final Verdict

**VERDICT: APPROVED**
