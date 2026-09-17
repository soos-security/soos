# Candid Code Review Report: Issue #20 — fix(daemon): TOCTOU-safe socket binding and permission hardening

**Reviewer**: Candid Reviewer Sub-Agent (`candid-reviewer`)  
**Target Branch**: `fix/socket-toctou`  
**Base**: `origin/main`  
**Issue**: Issue #20 (GitHub Issue #59) — `fix(daemon): TOCTOU-safe socket binding and permission hardening`

---

## 1. Diff Scope & Summary

The reviewed diff contains modifications across the following files:
- `crates/daemon/src/config.rs`: Added `socket_group: Option<String>` to `SocketConfig` (defaulting to `Some("soos".to_string())`) with full TOML deserialization support.
- `crates/daemon/src/error.rs`: Added `DaemonError::Validation(#[from] soos_protocol::types::ValidationError)` error propagation.
- `crates/daemon/src/socket.rs`: Replaced vulnerable `exists()` and unisolated `set_permissions` with descriptor-relative operations:
  - `open_and_validate_directory` using safe `fs::OpenOptions` with `O_DIRECTORY | O_NOFOLLOW` and `fstat` validation.
  - Serialization of the socket creation critical section via `nix::fcntl::Flock` on the directory descriptor.
  - Descriptor-relative stale socket inspection and cleanup via `fstatat` and `unlinkat` (`NoRemoveDir`) with `AT_SYMLINK_NOFOLLOW`.
  - Post-bind validation verifying the newly bound inode is a genuine socket.
  - Safe permissions application via `fchmodat` with `NoFollowSymlink`.
  - Hardened group ownership application via `fchownat` with `AT_SYMLINK_NOFOLLOW` and system group discovery.
- `crates/daemon/src/dispatcher.rs`: Enforced early wire protocol validation in `ConnectionDispatcher::handle_request` via `Request::validate()`, rejecting invalid versions and oversized service names fail-closed with `Verdict::ProtocolError` and `ReasonClass::MalformedRequest`.
- `crates/daemon/tests/dispatcher_tests.rs`: Added contractual tests `test_dispatcher_rejects_invalid_protocol_version` and `test_dispatcher_rejects_oversized_service_name`.
- `crates/daemon/tests/socket_tests.rs`: Added contractual tests `test_socket_binding_resists_symlink_race` and `test_socket_ownership_root_soos`.
- `scripts/sync_issue.py`: Registered `"fix/socket-toctou": 20` in `BRANCH_TO_ISSUE`.

---

## 2. 5-Pillar Architectural Audit

### Pillar 1: Logic & Architecture
- **TOCTOU Elimination**: The binding critical section is guarded by `nix::fcntl::Flock` on the verified directory descriptor. Stale node inspection and deletion happen exclusively relative to `dir_lock` using `fstatat` and `unlinkat` with `AT_SYMLINK_NOFOLLOW`.
- **Symlink Protection**: Symlinks at `socket_path` (whether target-pointing or broken) are detected and fail closed with `DaemonError::SocketDirValidation` without following or modifying target files.
- **Ownership Hardening**: The socket owner is set to `root:soos` (0:soos_gid) with fallback and unprivileged accommodation in non-root test environments.
- **Protocol Defense-in-Depth**: Wire validation in the dispatcher prevents invalid requests from reaching downstream biometric evaluation.
- **Verdict**: PASS

### Pillar 2: PAM Concurrency & Real-Time Deadlines
- **Zero PAM Impact**: All socket validation and binding occur at daemon startup prior to servicing requests. Dispatcher `Request::validate()` is an in-memory bound check executing in nanoseconds.
- **Zero Stream Pollution**: Clean structured `tracing` events only; no `println!` or `eprintln!` stdout/stderr pollution.
- **Verdict**: PASS

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Zero Unwraps / Expects in Production**: All operations propagate `Result<(), DaemonError>` using `?`.
- **Safe POSIX Calls**: System call return codes (`ELOOP`, `ENOTDIR`, `ENOENT`, `EOPNOTSUPP`) are handled explicitly.
- **Fail-Closed Fallback**: Request validation failures return `Verdict::ProtocolError`, never `Verdict::Allow`.
- **Verdict**: PASS

### Pillar 4: Test Integrity & Anti-Weakening
- **Zero Test Weakening**: All 4 pre-existing socket tests and 6 pre-existing dispatcher tests remain intact and passing.
- **Immutable Acceptance Contracts**: `test_socket_binding_resists_symlink_race`, `test_socket_ownership_root_soos`, `test_dispatcher_rejects_invalid_protocol_version`, and `test_dispatcher_rejects_oversized_service_name` satisfy the acceptance criteria of Backlog Issue #20.
- **Verdict**: PASS

### Pillar 5: Memory & Secret Bounds
- **Zero Unsafe Code**: All production logic is 100% safe Rust (`OpenOptionsExt` replaces raw unsafe fd conversions).
- **No Secret Leakage**: No credential or biometric data is exposed in logs.
- **Verdict**: PASS

---

## 3. Review Verdict

**VERDICT: APPROVED**
