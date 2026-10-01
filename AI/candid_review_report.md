# Candid Review Report

- **Date**: 2026-10-01
- **Target Branch**: `fix/p3fu2-batch` (GitHub #289; merges `fix/p3fu2-diagnostics`, the camera follow-ups and `fix/p3fu2-storage-daemon`)
- **Base (merge-base)**: `789beb7` (`origin/main`)
- **Reviewed-Diff-Fingerprint**: `759e7b6596502e59581b201dc18df8b58aaf3e8e9ca252c6d0f3a3775ceedf7d`
- **Fingerprint provenance**: `./scripts/candid_subagent.sh --prepare` on a clean working tree, and recomputed with the
  pinned `review_diff` options against the committed tree `HEAD^{tree}` (the input of the pre-push / CI `--rev` gate):
  both give the value above.
- **Audited Files** (54): `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/walkthroughs/151_diagnostics_parity_and_config_reader.md`,
  `AI/walkthroughs/152_camera_alias_and_v4l_guard_followups.md`,
  `AI/walkthroughs/153_migration_lock_and_shutdown_bound.md`, `Cargo.lock`, `Docs/BIOMETRIC_STORE_CRATE.md`,
  `Docs/CAMERA_V4L_CRATE.md`, `Docs/DAEMON.md`, `Docs/ENROLLMENT_CLI.md`, `Docs/GUI_APPLICATION.md`,
  `Docs/IPC_PROTOCOL.md`, `crates/admin-cli/Cargo.toml`, `crates/admin-cli/src/daemon_config.rs`,
  `crates/admin-cli/src/test_pam.rs`, `crates/admin-cli/tests/camera_config_shared_reader_tests.rs`,
  `crates/admin-cli/tests/test_pam_request_id_tests.rs`, `crates/biometric-store/src/error.rs`,
  `crates/biometric-store/src/lib.rs`, `crates/biometric-store/src/store.rs`,
  `crates/biometric-store/tests/store_lock_tests.rs`, `crates/camera-v4l/Cargo.toml`,
  `crates/camera-v4l/src/daemon_config.rs`, `crates/camera-v4l/src/error.rs`, `crates/camera-v4l/src/lib.rs`,
  `crates/camera-v4l/src/sensor.rs`, `crates/camera-v4l/src/status.rs`, `crates/camera-v4l/src/v4l_guard.rs`,
  `crates/camera-v4l/src/v4l_impl.rs`, `crates/camera-v4l/tests/daemon_config_reader_tests.rs`,
  `crates/camera-v4l/tests/supervisor_alias_hint_tests.rs`, `crates/camera-v4l/tests/v4l_panic_hook_filter_tests.rs`,
  `crates/camera-v4l/tests/v4l_teardown_guard_tests.rs`, `crates/daemon/src/main.rs`, `crates/daemon/src/shutdown.rs`,
  `crates/daemon/tests/pcx_wire_routing_tests.rs`, `crates/daemon/tests/shutdown_exit_bound_tests.rs`,
  `crates/enrollment-cli/Cargo.toml`, `crates/enrollment-cli/src/lib.rs`, `crates/enrollment-cli/src/main.rs`,
  `crates/enrollment-cli/src/service.rs`, `crates/enrollment-cli/tests/daemon_config_shared_reader_tests.rs`,
  `crates/enrollment-cli/tests/migrate_open_existing_tests.rs`, `crates/gui/Cargo.toml`,
  `crates/gui/src/camera_source.rs`, `crates/gui/src/ipc_camera.rs`, `crates/gui/tests/common/stamps.rs`,
  `crates/gui/tests/ipc_camera_failure_tests.rs`, `crates/gui/tests/ipc_camera_tests.rs`,
  `crates/gui/tests/ipc_preview_freshness_tests.rs`, `tests/invariants/src/camera_alias_guard_contract.rs`,
  `tests/invariants/src/diagnostics_parity_contract.rs`, `tests/invariants/src/installer_templates_contract.rs`,
  `tests/invariants/src/lib.rs`.

## 1. Executive Summary

The batch adds (A) diagnostics parity (`test-pam` nonce binding, GUI refusal freshness, one shared bounded
`daemon.toml` reader), (B) camera hardening (by-id alias hint for plain `/dev/videoN`, a silent panic-hook
filter for caught `v4l` panics, guarded MMAP stream creation/teardown), and (C) storage/daemon hardening
(directory `flock` for store mutations, migration identity check, never-create `open_existing`, daemon exit
bounded by `Runtime::shutdown_timeout`). I tried to break each part (deadlock, unbounded waits, fail-open, IR
downgrade, swallowed unguarded panics, leaked values, corrupting shutdown) and found no CRITICAL or MAJOR
defect. Test changes are exactly the allowed set. Two MINOR robustness issues and a few suggestions are listed
below. The affected crate suites (`soos-biometric-store`, `soos-camera-v4l`, `soos-admin-cli`,
`soos-enrollment-cli`, `soos-invariants`, the GUI IPC suites, the daemon shutdown and PCX routing suites) pass
locally with `--all-features`.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- `grep '^-[^-].*(assert|#[test]|...)'` on the frozen patch: **no hit**; no assertion, test attribute or
  proptest was removed or changed.
- `grep '^+.*(#[ignore|#[cfg(any())]|should_panic|tolerance|epsilon)'`: **no hit**.
- Inline test modules added: `mod concurrency_tests` at the end of `crates/biometric-store/src/store.rs`
  (MLS1/MLS2 races through the private `migrate_template_with_hook`), `mod tests` at the end of
  `crates/camera-v4l/src/v4l_impl.rs` (teardown settlement, `CAG`). Both new, no existing module touched.
- Existing test files modified (full diffs read):
  - `tests/invariants/src/installer_templates_contract.rs`: message only, "5 in 60 s" → "5 in 320 s"; the
    condition (`StartLimitIntervalSec=320`, `StartLimitBurst=5`) is byte-identical and matches
    `packaging/soos-daemon.service:15-16`. OWNER-APPROVED, allowed.
  - `crates/daemon/tests/pcx_wire_routing_tests.rs`: `SETRESUID_SYSCALL` selects `SYS_setresuid32` on
    `x86`/`arm`; `SAFETY` comment updated; setup only.
  - `crates/gui/tests/ipc_camera_tests.rs`, `ipc_camera_failure_tests.rs`: fake-daemon fixtures stamp from
    CLOCK_MONOTONIC via `common/stamps.rs` instead of `1`/`2`; required because the production code now
    enforces freshness; no assertion changed. Setup only, allowed.
  - `tests/invariants/src/lib.rs`: two new `mod` lines. Allowed.
- New test files (19 test files in total touched, all other ones new): reviewed for strength; e.g.
  `v4l_panic_hook_filter_tests.rs` asserts both silence of guarded panics and delivery of unguarded panics
  on another thread concurrently with a guarded call, nesting, and depth reset; `concurrency_tests` would fail
  against an implementation that rewrites without the lock (resurrection / overwrite) or without the identity
  check (unlocked remove/replace/in-place rewrite).

## 3. Deep Reasoning Audit

### Logic & Architecture

- **Store lock deadlock**: `migrate_template_with_hook` takes the lock and calls the private
  `write_template`, never the locking `enroll` (store.rs:393 ff.); `migrate_legacy_templates` locks per UID,
  not around the loop; no caller holds the guard across another `enroll`/`delete`. `flock` is per open file
  description, so two stores in one process contend correctly (proven by the threaded tests). → PASS.
- **Read paths never block**: `get`/`exists`/`list_enrolled`/`template_format` and dry runs never call
  `lock_store`; the daemon never calls `enroll`/`delete` (grep of `crates/daemon/src`). → PASS.
- **flock on an `O_DIRECTORY` fd**: valid on Linux local filesystems; fd opened `O_RDONLY|O_DIRECTORY|
  O_NOFOLLOW` with Rust's default `O_CLOEXEC` (no leak into `pkexec` children); released on drop and on
  process death. → PASS (NFS caveat as suggestion).
- **Timeout bounded**: `LOCK_NB` loop, sleep `min(10 ms, remaining)`, `EAGAIN|EINTR` retried until
  `elapsed >= lock_timeout`, any other errno → `Io`; `Duration::ZERO` = one attempt. Errors are propagated
  (`LockTimeout`), never mapped to success. → PASS.
- **Identity check**: dev/ino/len/mtime(+nsec) recorded from the fstat of the read fd; compared with the fstat
  of the write handle (`open_existing_template_for_overwrite`, which already rejects symlinks/non-regular
  files and checks ino/dev) and once more via `symlink_metadata` just before `rename`; a missing file maps to
  `ChangedConcurrently`, the temp file is removed. Residual check-to-rename window and same-size same-tick
  in-place rewrites are documented accepted limits (the lock is the primary layer). → PASS.
- **`open_existing`**: `symlink_metadata` then the same `validate_store_dir` as `new`; never creates; NotFound
  surfaces as `Io(NotFound)` and `open_template_store_checked` maps exactly that to "skipped". → PASS.
- **Alias lookup**: only fills a missing `by_id_name`; uses the bounded `by_id_aliases` (64 entries, sorted,
  dangling skipped); IR-token alias preferred, else smallest; rule 1 of the classifier is only ever positive,
  so `Rgb → Infrared` is the only possible move (scenario: an alias without IR token on an IR node by card name
  — still IR via rule 2). → PASS.
- **Guarded teardown**: no `?` between `mmap_stream_guarded` and `guarded_v4l_drop(source)`; the streaming
  error path is now also torn down through the guard; `StreamTeardown` maps to `Io` (backoff), not
  re-resolution. Double panic abort is a documented, pre-existing limit. → PASS.
- **Daemon `main`**: `new_multi_thread().enable_all()` equals `tokio::main` defaults; logging, panic hook,
  `mlockall`, pipeline init, sd_notify and signal handling are unchanged inside `run()` and in the same order;
  `Err` still returns from `main` (exit 1); the startup error path uses a zero budget. → PASS.
- **Shutdown cutting writes**: template and evidence writes go temp file → fsync → `rename(2)`
  (evidence store.rs:374, :599, :859); a blocking job killed at process exit leaves at worst an orphan temp
  file, never a torn final file. → PASS (orphan cleanup as suggestion).
- **Shared reader**: admin mirrors the daemon (mistyped key → `Malformed`), enroll/gui degrade per key; logic
  matches the ADR and the CVF3 contract stays untouched. → PASS.
- **MINOR**: `soos-enroll` reads `daemon.toml` twice (notes in `warn_camera_config_notes`, settings again in
  `build_full_service`), see §4.

### PAM Concurrency & Deadlines

- `crates/pam` is not touched; `soos-camera-v4l` is not a dependency of `crates/pam` (its own panic hook in
  `syslog.rs` is unaffected by the filter). `test-pam` uses the existing bounded client path. → PASS.

### Panic Safety & Fail-Closed

- **Panic-hook filter**: installed once via `Once`, `take_hook` chained, skipped while the thread is panicking
  (no `set_hook` panic); the daemon installs its hook earlier (main.rs before `initialize_pipeline`), so the
  filter chains to it. Per-thread depth counter read with `try_with` (TLS destruction → report). Scenario: an
  unguarded panic on another thread while a guarded call is active → reported (test case 3); after the guard
  returns on the same thread → reported (case 5). Reentrancy: the wrapper never calls hook APIs. → PASS.
- **GUI freshness**: the check runs only after a nonce match on a `Response` refusal and turns a stale one into
  `IpcPreviewError::Protocol`; no `Response` path yields a preview frame; clock failure → 0 → rejected. → PASS.
- **`test-pam`**: nonce mismatch evaluated first and always reported as `PAM_IGNORE`; `PAM_SUCCESS` only when
  bound, fresh and `Allow`. → PASS.
- No new `unwrap`/`expect`/indexing in production code of the diff (all `u64::try_from(..).unwrap_or`). → PASS.

### Test Integrity & Anti-Weakening

- See §2: only the allowed changes; every new test file was read and can fail against a plausible wrong
  implementation. → PASS.

### Memory, Bounds & Secrets

- **daemon.toml reader**: `O_NONBLOCK|O_CLOEXEC` open (FIFO behind a symlink returns at once), `fstat` of the
  handle (regular file only), size ≤ 1 MiB, and `take(MAX+1)` re-checked after the read (growth after fstat).
  Warnings name keys and defaults only; errors carry an `ErrorKind`, never content; the path is sanitized. → PASS
  (device-node side effect as suggestion).
- **Nonce**: never printed in `test_pam.rs` (only "does not match"). → PASS.
- **Lock error text**: store path and wait time only. Migration errors: UID only. → PASS.
- Alias scan bounded (64). Store temp files `0600` + `O_NOFOLLOW` unchanged. → PASS.

### Supply Chain & Automation

- `toml` moves from `soos-admin-cli`/`soos-enrollment-cli` to `soos-camera-v4l` (already locked, no new crate);
  `nix` gains the `time` feature in `soos-gui`. No script, hook or workflow change. → PASS.

### English-Only Policy

- Code, comments, docs, ADRs and walkthroughs are English (spot-checked plus a keyword grep of added lines). → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/biometric-store/src/store.rs:560` (`lock_store`), reached from `crates/gui/src/app.rs:1089`
  and `:1240` — in direct (non-Polkit) mode the GUI calls `store.enroll` / `store.delete` on the egui UI thread;
  with the new lock a concurrent `soos-enroll` holding the store can now freeze the window for up to
  `STORE_LOCK_TIMEOUT` (5 s). Bounded and fail-closed, but a UI stall. Correction: run these two calls on the
  existing task worker (as the Polkit path does) or use `with_lock_timeout` with a short bound in the GUI.
- **[MINOR]** `crates/enrollment-cli/src/main.rs:66` (`warn_camera_config_notes`) — the notes come from a
  separate read of `daemon.toml`, and `build_full_service` reads it again; a file replaced in between makes the
  printed notes describe a configuration other than the one applied. Correction: resolve once with
  `resolve_camera_device_from_config_reported` inside `build_full_service` and print that call's notes.

## 5. Suggestions (optional, not blocking)

- **[SUGGESTION]** `crates/camera-v4l/src/daemon_config.rs:176` — opening a device node behind a symlink with
  `O_RDONLY` still runs the driver's `open` (side effects for nodes such as watchdogs). Requires root to plant,
  and the daemon follows the same link; consider an `O_PATH` open + `fstat`, then reopening a regular file via
  `/proc/self/fd/N`.
- **[SUGGESTION]** `crates/biometric-store/src/store.rs:560` — on NFS, `flock` is emulated with POSIX locks and an
  exclusive lock on an `O_RDONLY` fd fails with `EBADF`; the result is a fail-closed `Io` error. A note in
  `Docs/BIOMETRIC_STORE_CRATE.md` would help operators.
- **[SUGGESTION]** `crates/admin-cli/src/test_pam.rs:226` — `PamTestReport.verdict` still shows the raw daemon
  verdict (possibly `Allow`) for a nonce-mismatched or stale response; `pam_result` is authoritative, but JSON
  consumers may read `verdict`. Consider a `bound_to_request` / `fresh` field.
- **[SUGGESTION]** Evidence and template temp files orphaned by an abandoned blocking job at shutdown are not
  swept at startup; a bounded sweep of `*.tmp` in the store directories would keep them from accumulating.

## 6. Final Verdict

No CRITICAL or MAJOR finding. The two MINOR findings are bounded robustness issues with no security impact.

Gate: `./scripts/candid_subagent.sh` → Layer 1 "Candid Review PASSED", Layer 2 "Dual-Layer Candid Review PASSED"
(expected fingerprint `759e7b6596502e59581b201dc18df8b58aaf3e8e9ca252c6d0f3a3775ceedf7d`, working tree).

**VERDICT: APPROVED**
