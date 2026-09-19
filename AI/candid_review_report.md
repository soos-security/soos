# Candid Review Report

- **Date**: 2026-09-19
- **Target Branch / Commit**: `fix/pam-ffi-timeout`
- **Audited Files**:
  - `crates/pam/Cargo.toml`
  - `crates/pam/src/lib.rs`
  - `crates/pam/src/ipc.rs`
  - `crates/pam/tests/ipc_tests.rs`
  - `crates/protocol/src/types.rs`
  - `scripts/sync_issue.py`

## 1. Executive Summary

This pull request implements comprehensive security hardening for the Linux-PAM module `pam_soos.so` under Issue #34 (GitHub #73). The changes eliminate risks of undefined behavior across the C ABI by encapsulating argument parsing and all exported PAM symbols within `catch_unwind`, implement non-blocking socket connection (`connect_with_timeout`) with strict POSIX `poll` timeouts to prevent host process stalls if the daemon freezes, and enforce complete memory zeroization on drop for requests, nonces, and IPC communication buffers.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**: State transitions, non-blocking connection with `libc::poll`, and subsequent blocking socket I/O transitions are completely sound.
- **Socket Path Validation**: `path_bytes.len() >= 108` check strictly protects against `sockaddr_un` buffer overflow.
- **Descriptor Lifetime**: RAII wrapper `FdGuard` disarms via `.into_raw()` only when ownership is transferred to `UnixStream`, preventing any file descriptor leaks on connection timeout or error.

### PAM Concurrency & Deadlines
- **Pass**: Zero async runtime (Tokio) inside `crates/pam`.
- **Strict Real-Time Latency**: `connect_with_timeout` dynamically computes remaining time from the total budget (`remaining_budget(start_time, total_timeout)`). Saturated or frozen daemon sockets cannot stall beyond the configured `timeout_ms` (250ms nominal, 20ms telemetry).
- **Stream Isolation**: Zero `println!`, `eprintln!`, or `dbg!` calls in PAM production code.

### Panic Safety & Fallback
- **Pass**: All exported C ABI entry points (`pam_sm_authenticate`, `pam_sm_setcred`, `pam_sm_acct_mgmt`, `pam_sm_chauthtok`, `pam_sm_open_session`, `pam_sm_close_session`) are encapsulated via `catch_c_entry` (`catch_unwind`).
- **Argument Parsing Protected**: `config::parse_argv(argc, argv)` and `pamh.as_mut()` execute strictly inside `catch_unwind`, ensuring allocation errors or pointer faults degrade safely to `PAM_IGNORE` rather than unwinding across the C boundary.
- **Fail-Closed Guarantee**: Caught panics are logged to syslog and systematically return `PAM_IGNORE`. No pathway allows error-to-success conversion.

### Test Integrity & Anti-Weakening
- **Pass**: Zero existing tests were modified or weakened.
- **Contractual Tests Added**:
  - `test_ipc_connect_timeout_frozen_daemon`: Contractually verifies that saturated backlog / frozen daemon sockets time out within budget (< 250ms) and return `PAM_IGNORE`.
  - `test_request_and_event_zeroize_on_drop`: Contractually asserts that `Request` and `Event` zeroize nonces, UIDs, and strings on drop.
  - `test_c_abi_all_entry_points_panic_safe`: Contractually asserts that all C ABI exports return `PAM_IGNORE` without panicking.
  - `authenticate_catches_parse_argv_panics`: Unit test verifying `pam_sm_authenticate` argument parsing panic containment.

### Memory & Secret Bounds
- **Pass**: `Request` and `Event` in `soos-protocol` implement `zeroize::Zeroize` and `Drop`.
- **Buffer Sanitization**: `request_id`, `encoded` request, length header, and `full_buf` response are wrapped in `Zeroizing` or cleared.
- **Unsafe Audit**: Unsafe blocks in `crates/pam` are strictly confined to POSIX socket syscalls and PAM C ABI boundary glue, each documented with an explicit `// SAFETY:` rationale.

## 3. Detailed Findings & Action Items

- **[MINOR / RESOLVED]** `crates/pam/src/ipc.rs:134` — Replaced `std::mem::forget(self)` with `self.0 = -1` in `FdGuard::into_raw` to eliminate `clippy::mem_forget` on `Drop` types.
- **[MINOR / RESOLVED]** `crates/pam/src/ipc.rs:175` — Safely mapped `libc::AF_UNIX` using `sa_family_t::try_from(libc::AF_UNIX).unwrap_or(0)` to prevent integer truncation warnings.

## 4. Final Verdict

**VERDICT: APPROVED**
