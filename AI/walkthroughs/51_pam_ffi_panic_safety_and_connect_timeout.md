# Walkthrough 51 — PAM Hardening: FFI Panic Safety, Non-Blocking Connect Timeout, and Memory Zeroization

## Context & Objectives

- **Issue**: Issue #34 (`fix(pam): FFI panic safety and IPC blocking timeout`) / GitHub Issue #73
- **Branch**: `fix/pam-ffi-timeout`
- **Mission**:
  1. Expand `catch_unwind` to encompass `parse_argv` in `pam_sm_authenticate` and all C ABI entry points (#34.1).
  2. Implement non-blocking `connect()` with strict timeout in `ipc.rs` to avoid blocking indefinitely if the daemon is frozen (#34.2).
  3. Add `zeroize` dependency to `soos-pam` and enforce cleanup for `Request`, `Event`, and IPC buffers (#34.3).

---

## 1. Architectural Design & Compliance (Phase 1 & Phase 1.5)

- **Plan Evaluator Sub-Agent**:
  Audited the implementation plan across the 6 architectural pillars in `AI/plan_evaluator_report.md` with explicit verdict: `VALIDATION_VERDICT: APPROVED`.
- **Architectural & Security Invariants Honored**:
  - `AI/ARCHITECTURE.md` §10 (PAM Module Hardening): No panics may cross the C ABI boundary under any circumstances (including argument parsing or out-of-memory errors). Every failure must degrade to `PAM_IGNORE`.
  - Latency Budget Compliance: Asynchronous runtimes (Tokio) remain strictly forbidden in the PAM module. Non-blocking socket connect (`libc::SOCK_NONBLOCK`) combined with `libc::poll` enforces the strict timeout budget during connection establishment, preventing hangs when daemon queues are saturated or processes are frozen.
  - Zeroization & Data Confidentiality: All sensitive buffers, cryptographic request identifiers (nonces), and IPC structures are systematically scrubbed using `Zeroize` and `Zeroizing<T>` upon drop.

---

## 2. Test Contracts (Phase 2 — TDD Red Phase)

Contractual tests were authored in `crates/pam/tests/ipc_tests.rs` and `crates/pam/src/lib.rs` prior to modifying production code:
1. `ipc_tests::test_ipc_connect_timeout_frozen_daemon`: Creates a listening socket with saturated backlog (0 queue capacity) and verifies that `pam_sm_authenticate` terminates within its configured 100ms timeout (< 250ms budget) and returns `PAM_IGNORE` rather than hanging indefinitely.
2. `ipc_tests::test_request_and_event_zeroize_on_drop`: Verifies that `Request` and `Event` implement `zeroize::Zeroize` and `Drop`, zeroing out request nonces, user IDs, and service strings upon deallocation.
3. `ipc_tests::test_c_abi_all_entry_points_panic_safe`: Validates that all exported C ABI functions (`pam_sm_setcred`, `pam_sm_acct_mgmt`, `pam_sm_chauthtok`, `pam_sm_open_session`, `pam_sm_close_session`) safely return `PAM_IGNORE` and are wrapped in panic-handling logic.
4. `lib::tests::authenticate_catches_parse_argv_panics`: Confirms `pam_sm_authenticate` catches panics or invalid pointer conditions in argument parsing and returns `PAM_IGNORE`.

**Red Phase Execution**:
- `test_request_and_event_zeroize_on_drop` and `ipc_tests` failed compilation initially due to missing `zeroize` dependency and trait implementations.

---

## 3. Implementation (Phase 3 & Phase 4)

- **FFI Panic Safety (`crates/pam/src/lib.rs`)**:
  - Introduced `catch_c_entry<F>` helper that initializes the syslog panic hook, wraps execution in `catch_unwind(AssertUnwindSafe(f))`, captures panic locations, logs via `syslog::log_panic`, and returns `PAM_IGNORE`.
  - Updated `pam_sm_authenticate` to run all logic, including `config::parse_argv(argc, argv)` and `pamh.as_mut()`, within `catch_c_entry`.
  - Updated all other C ABI entry points (`pam_sm_setcred`, `pam_sm_acct_mgmt`, `pam_sm_chauthtok`, `pam_sm_open_session`, `pam_sm_close_session`) to be wrapped in `catch_c_entry`.
  - Wrapped `parse_cstrs(args)` within `catch_unwind` inside `PamHooks::sm_authenticate`.
- **Non-Blocking Connect with Timeout (`crates/pam/src/ipc.rs`)**:
  - Added `connect_with_timeout(path: &Path, timeout: Duration) -> Result<UnixStream, IpcError>` using POSIX `libc::socket(..., SOCK_NONBLOCK)`.
  - Added `FdGuard` RAII struct to prevent descriptor leakage upon error or timeout.
  - Used `libc::poll` to wait for writability on `EINPROGRESS` within `remaining_budget`.
  - Confirmed connection status via `libc::getsockopt(SOL_SOCKET, SO_ERROR)`.
  - Restored blocking mode via `stream.set_nonblocking(false)` for subsequent timed socket I/O.
  - Used `connect_with_timeout` in both `authenticate()` and `notify_event()`.
- **Memory Zeroization (`crates/protocol/src/types.rs` & `crates/pam/Cargo.toml`)**:
  - Added `zeroize = { workspace = true }` to `crates/pam/Cargo.toml`.
  - Implemented `zeroize::Zeroize` and `Drop` for `Request` and `Event` in `crates/protocol/src/types.rs`.
  - Wrapped `request_id`, `encoded` request, `len_buf`, and `full_buf` in `Zeroizing` in `crates/pam/src/ipc.rs`.

---

## 4. Candid Review & Verification Matrix (Phase 5 & Phase 6)

- **Candid Reviewer Audit**:
  The raw git diff against `origin/main` was audited in `AI/candid_review_report.md` with `VERDICT: APPROVED`.
- **Traceability & Verification Matrix**:
  - Updated `AI/VERIFICATION_MATRIX.md` with `PA3`, `PA15`, `PA16`, and new section for `Component: pam-ffi-timeout` (`PFT1`, `PFT2`, `PFT3`).
  - Updated technical documentation in `Docs/PAM_MODULE.md` and `Docs/IPC_PROTOCOL.md`.
  - Synchronized issues via `scripts/sync_issue.py`.

---

## 5. Automated Verification Results

All unit and integration tests pass across the entire workspace:
- `soos-pam`: 53 tests passed (5 unit, 13 config, 23 ipc, 8 pam_bindings, 4 uid_resolution).
- `soos-protocol`: 26 tests passed (16 unit, 10 property tests).
- All workspace crates pass `cargo test --workspace` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
