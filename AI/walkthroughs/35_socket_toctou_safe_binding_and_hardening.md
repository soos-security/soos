# Walkthrough 35 — TOCTOU-Safe Socket Binding & Permission Hardening

## Overview
This walkthrough documents the design, implementation, and verification of **Issue #20** (`fix/socket-toctou`): **TOCTOU-safe socket binding and permission hardening** for `soos-daemon`.

Prior to this issue, socket binding and cleanup performed unisolated path checks (`exists()`, `remove_file()`, `set_permissions()`), leaving potential race windows where a malicious unprivileged process could swap the socket node with a symbolic link targeting sensitive system files. Additionally, the daemon did not enforce `root:soos` group ownership and lacked wire protocol validation in the connection dispatcher.

---

## Key Changes

### 1. TOCTOU Elimination & Symlink Protection (`crates/daemon/src/socket.rs`)
- **Safe Directory Descriptor Verification**:
  - Implemented `open_and_validate_directory` using `std::fs::OpenOptions` with `libc::O_DIRECTORY | libc::O_NOFOLLOW`.
  - Directory invariants are asserted directly on the open file descriptor via `fstat`: verified genuine directory, not world-writable (`mode & 002 == 0`), and root-owned (`uid == 0`).
  - Completely safe Rust implementation with zero unsafe blocks.
- **Critical Section Serialization**:
  - Bound directory operations are synchronized using RAII `nix::fcntl::Flock` (`LockExclusiveNonblock`).
  - Guarantees mutual exclusion between concurrent daemons, preventing TOCTOU races during stale check and bind.
- **Descriptor-Relative Stale Socket Handling**:
  - Stale nodes are inspected using `fstatat` with `AT_SYMLINK_NOFOLLOW`.
  - If a stale path is a symlink or non-socket file, `bind_socket` fails closed immediately with `DaemonError::SocketDirValidation`.
  - Legitimate stale sockets are unlinked descriptor-relative via `nix::unistd::unlinkat(..., NoRemoveDir)`.
- **Post-Bind Invariant Verification & Permission Hardening**:
  - Immediately following `UnixListener::bind`, `fstatat` validates that the created node is genuine socket.
  - Permissions (`0660`) are enforced using `fchmodat` with `NoFollowSymlink`.
  - Ownership is applied using `fchownat` with `AT_SYMLINK_NOFOLLOW`.

### 2. Group Ownership Hardening (`root:soos`)
- Added `socket_group: Option<String>` to `SocketConfig` (defaulting to `Some("soos".to_string())`) with full TOML deserialization support.
- Added `resolve_socket_group` helper which resolves the GID for `soos` and, if running as root, provisions the system group via `groupadd --system soos` if missing.
- Sets socket ownership to `0:soos_gid` while accommodating unprivileged unit testing environments.

### 3. Early Wire Protocol Validation (`crates/daemon/src/dispatcher.rs`)
- Integrated `req.validate()` at the entry point of `ConnectionDispatcher::handle_request`.
- Requests with `version != CURRENT_VERSION` or `service.len() > MAX_SERVICE_LEN` (64 bytes) are immediately rejected with `Verdict::ProtocolError` and `ReasonClass::MalformedRequest`, closing the stream fail-closed.
- Added `DaemonError::Validation(ValidationError)` error variant to `crates/daemon/src/error.rs`.

---

## Verification Results

### Automated Contract Tests
- `crates/daemon/tests/socket_tests.rs`:
  - `test_socket_binding_resists_symlink_race`: Verifies that broken symlinks and symlinks targeting existing victim files are rejected without unlinking, truncating, or altering victim file permissions.
  - `test_socket_ownership_root_soos`: Verifies `root:soos` socket ownership in privileged environments and target group assignment in unprivileged test mode.
  - `test_socket_created_with_0660_permissions`: Verifies strict `0660` socket permissions.
  - `test_socket_recreation_cleans_up_stale_socket`: Verifies clean cleanup of stale sockets across restarts.
  - `test_socket_directory_validation_rejects_symlink`: Verifies symlink directories are rejected.
  - `test_socket_directory_validation_rejects_world_writable`: Verifies world-writable directories are rejected.
- `crates/daemon/tests/dispatcher_tests.rs`:
  - `test_dispatcher_rejects_invalid_protocol_version`: Asserts that requests with invalid version return `Verdict::ProtocolError` with `ReasonClass::MalformedRequest`.
  - `test_dispatcher_rejects_oversized_service_name`: Asserts that requests with service name > 64 bytes return `Verdict::ProtocolError` with `ReasonClass::MalformedRequest`.

### Quality Gates
```text
cargo test --workspace                         -> 100% PASS (0 failures)
cargo fmt --check                              -> PASS
cargo clippy --all-targets --all-features      -> PASS (0 warnings)
./scripts/candid_review.sh                     -> PASS (all invariants verified)
```
