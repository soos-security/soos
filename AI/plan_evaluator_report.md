# Plan Evaluation Report — Issue #21: Async Cancellation Safety on Socket Writes

- **Target Issue**: Issue #21 (GitHub #60) — `fix(daemon): Async cancellation safety on socket writes`
- **Target Branch**: `fix/async-cancel-safety`
- **Evaluator**: Independent Plan Evaluator Sub-Agent
- **Date**: 2026-09-17

---

## 1. Executive Summary

The proposed implementation addresses async cancellation hazards during Unix domain socket writes in `soos-daemon` and adds response completeness verification in `pam_soos.so`.

By decoupling the request handling timeout from socket write transmission, `soos-daemon` guarantees that async timeout cancellations occur strictly *before* any byte has been written to the client stream. The pre-encoded in-memory response buffer is subsequently written using `write_all` and `flush` under a dedicated write timeout. On the client side, `pam_soos.so` introduces `IpcError::TruncatedResponse` and verifies that total received bytes match the declared wire frame size before invoking postcard deserialization, ensuring fail-closed fallback to `PAM_IGNORE`.

---

## 2. Six Architectural Pillars Audit

### Pillar 1: Architectural Alignment & Threat Model
- **Evaluation**: PASS
- **Details**: The architecture maintains the strict separation of privilege between the unprivileged PAM module (`pam_soos.so`) and the privileged root daemon (`soos-daemon`). Communication occurs strictly over the private Unix domain stream socket (`/run/soos/daemon.sock`). Kernel `SO_PEERCRED` validation remains mandatory on all connection requests prior to processing.

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Evaluation**: PASS
- **Details**: Zero asynchronous runtimes or Tokio tasks are introduced into `crates/pam`. `pam_soos.so` strictly uses synchronous `std::os::unix::net::UnixStream` with cumulative read and write timeouts bounded by `config.timeout_ms` (200–250ms). Zero stream pollution (`println!`, `eprintln!`, `dbg!`) is introduced in PAM production code.

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Evaluation**: PASS
- **Details**: All PAM FFI entry points remain guarded by `catch_unwind`, systematically returning `PAM_IGNORE` upon any error or unexpected condition. Production code in both `crates/daemon` and `crates/pam` strictly avoids `unwrap()` and `expect()`. Truncated responses are mapped to `IpcError::TruncatedResponse`, which cleanly degrades to `PAM_IGNORE`.

### Pillar 4: Dependency Isolation & Banned Crates
- **Evaluation**: PASS
- **Details**: No banned crates (`opencv`, `nokhwa`) or new external dependencies are introduced. `#![forbid(unsafe_code)]` remains intact where declared. Standard `tokio::io::AsyncWriteExt` is used within `crates/daemon` (which already runs Tokio), and synchronous `std::io::Read` within `crates/pam`.

### Pillar 5: Data Confidentiality & Zeroization
- **Evaluation**: PASS
- **Details**: No passwords, raw embeddings, or unredacted biometric vectors travel across the IPC socket or appear in log lines. The wire schema continues to use zeroized `Response` types.

### Pillar 6: Test Integrity & TDD Contracts
- **Evaluation**: PASS
- **Details**: Acceptance tests are specified directly from `AI/BACKLOG.md` sub-issues #21.1 and #21.2:
  - `test_timeout_during_write_does_not_corrupt_response` in `crates/daemon/tests/dispatcher_tests.rs`
  - `test_pam_ipc_detects_truncated_response` in `crates/pam/tests/ipc_tests.rs`
  No existing test assertions will be weakened or bypassed.

---

## 3. Findings & Recommendations

- **Write Timeout Tuning**: Ensure write operations in `soos-daemon` use a separate write timeout (defaulting to e.g. `Duration::from_millis(100)` or matching connection timeout) so that an uncooperative or frozen client cannot exhaust worker permits indefinitely during the transmission phase.
- **Counted Stream Reader**: In `pam_soos::ipc`, count the exact received bytes across chunked reads to provide accurate `expected` and `received` diagnostics in `IpcError::TruncatedResponse`.

---

## 4. Final Verdict

VALIDATION_VERDICT: APPROVED
