# Plan Evaluation Report: Issue #20 — fix(daemon): TOCTOU-safe socket binding and permission hardening

**Evaluator**: Plan Evaluator Sub-Agent (`plan-evaluator`)  
**Target Issue**: Backlog Issue #20 (GitHub Issue #59) — `fix(daemon): TOCTOU-safe socket binding and permission hardening`  
**Target Branch**: `fix/socket-toctou`  
**Architecture Reference**: `AI/ARCHITECTURE.md` §4 (Socket Path and Permissions, Wire Protocol Framing, Async Boundaries)

---

## Executive Summary

The proposed implementation plan addresses security vulnerabilities in the Unix domain socket lifecycle of `soos-daemon`:
1. **Sub-issue #20.1**: Eliminates TOCTOU and symlink race conditions during socket creation, directory validation, stale socket cleanup, binding, and permission application using `O_DIRECTORY | O_NOFOLLOW` parent directory descriptors, `flock()` critical section serialization, and `fstatat` / `unlinkat` / `fchmodat` descriptor-relative operations.
2. **Sub-issue #20.2**: Hardens socket ownership to `root:soos` (`0:soos_gid`) via `fchownat` with `AT_SYMLINK_NOFOLLOW`, with automatic system group discovery/creation for `soos`.
3. **Sub-issue #20.3**: Enforces early wire protocol validation in `ConnectionDispatcher::handle_request` via `Request::validate()`, rejecting invalid protocol versions and oversized service names with `Verdict::ProtocolError` and `ReasonClass::MalformedRequest`.

---

## 6-Pillar Compliance Audit

### 1. Architectural Alignment & Threat Model
- **Boundary Preservation**: The privileged daemon (`soos-daemon`) resides at UID 0 and handles socket binding at `/run/soos/daemon.sock` with mode `0660` and owner `root:soos`.
- **Symlink Race Protection**: Directory validation uses `open` with `O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW` and `fstat` on the directory file descriptor, guaranteeing an unprivileged local attacker cannot redirect socket creation to sensitive system files (e.g. `/etc/shadow`).
- **TOCTOU Elimination**: Critical section serialization via `flock` and atomic directory descriptor operations (`fstatat`, `unlinkat`, `fchmodat` with `AT_SYMLINK_NOFOLLOW`) completely close the time-of-check to time-of-use window.
- **Group Isolation**: Only members of the `soos` group have read/write access to the socket.
- **Verdict**: PASS

### 2. PAM Real-Time Latency & Concurrency
- **Non-blocking Operations**: Socket directory validation, binding, and ownership setting execute during daemon initialization prior to accepting connections, adding 0ms to PAM request latency.
- **Dispatcher Wire Validation**: `Request::validate()` is an in-memory boundary check on `version` and `service.len()` taking < 100 nanoseconds, safely within the 150ms daemon decision budget.
- **Zero Output Pollution**: All diagnostic output uses `tracing` structured logging; zero `println!` or `eprintln!` stream pollution.
- **Verdict**: PASS

### 3. Panic Safety & Fail-Closed Behavior
- **Zero Unwraps / Expects in Production**: All error paths in `socket.rs` and `dispatcher.rs` propagate `Result<(), DaemonError>` using `?`.
- **Fail-Closed Protocol Validation**: Requests failing `req.validate()` immediately trigger a rejection response with `Verdict::ProtocolError` and `ReasonClass::MalformedRequest`, closing the stream without executing pipeline or policy logic.
- **Verdict**: PASS

### 4. Dependency Isolation & Banned Crates
- **Banned Dependencies**: No forbidden crates (`opencv`, `nokhwa`) are introduced.
- **Standard POSIX Primitives**: Utilizes `nix` (already in workspace with `features = ["socket", "fs", "user"]`) and `libc`.
- **Verdict**: PASS

### 5. Data Confidentiality & Zeroization
- **No Secret Leakage**: No passwords, biometric embeddings, or private data are logged or exposed.
- **Safe Error Reporting**: Validation errors indicate schema violations (unsupported version or service length) without reflecting unbounded user input into logs.
- **Verdict**: PASS

### 6. Test Integrity & TDD Contracts
- **Test Contracts**:
  - `test_socket_binding_resists_symlink_race`: Verifies that broken symlinks, target file symlinks, and stale symlinks are rejected without following or modifying target files.
  - `test_socket_ownership_root_soos`: Verifies that socket ownership is set to `root:soos` when running as root, and verifies group resolution and permission enforcement.
  - `test_dispatcher_rejects_invalid_protocol_version`: Verifies that requests with `version != CURRENT_VERSION` are rejected with `ProtocolError`.
  - `test_dispatcher_rejects_oversized_service_name`: Verifies that requests with `service.len() > MAX_SERVICE_LEN` are rejected with `ProtocolError`.
- **Zero Weakening**: All existing tests in `crates/daemon/tests/socket_tests.rs` and `crates/daemon/tests/dispatcher_tests.rs` remain intact and must pass.
- **Verdict**: PASS

---

## Conclusion & Verdict

The implementation plan is thoroughly compliant with all security invariants of `soos`.

**VALIDATION_VERDICT: APPROVED**
