# Walkthrough 36: Async Cancellation Safety on Socket Writes

> **Issue**: Issue #21 (GitHub #60) — `fix(daemon): Async cancellation safety on socket writes`  
> **Branch**: `fix/async-cancel-safety`  
> **Status**: Completed & Verified  

---

## 1. Context & Objectives

In asynchronous networking architectures leveraging Tokio (such as `soos-daemon`), futures wrapped in `tokio::time::timeout` are cancelled and dropped immediately when the deadline expires. When socket write operations (`AsyncWriteExt::write_all`) are executed inside the same timeout that bounds pipeline inference:
1. An async timeout trigger during socket writes drops the in-flight write future, leaving whatever bytes were already committed to the kernel socket buffer in place.
2. Dropping the socket sends a FIN/RST to the client, leaving the PAM module with an incomplete, truncated wire frame that causes deserialization errors.
3. Conversely, on the PAM client side, unexpected disconnection during framing was previously reported as generic `IpcError::Io(UnexpectedEof)` rather than an explicit completeness failure.

### Objectives
- **Sub-issue #21.1**: Decouple request reading & pipeline verification from socket transmission in `soos-daemon`, ensuring that async timeout cancellations strictly happen before any byte is written to the client stream.
- **Sub-issue #21.2**: Implement strict response completeness validation in `pam_soos.so`, introducing `IpcError::TruncatedResponse` with byte-counted verification and fail-closed fallback to `PAM_IGNORE`.

---

## 2. Architect Design & Types

### Two-Phase Connection Lifecycle in `soos-daemon`
```
┌────────────────────────────────────────────────────────┐
│ Phase 1: Request Reading & Verification                │
│ Runs under timeout(connection_timeout, ...)            │
│ 1. Read framed request/event                           │
│ 2. Verify peer credentials via SO_PEERCRED             │
│ 3. Execute neural verification pipeline                │
│ 4. Fully serialize Response to in-memory Vec<u8>       │
│ *ZERO socket writes occur during this phase*           │
└───────────────────────────┬────────────────────────────┘
                            │
            Timeout? ───────┴───────► Stream closed with 0 bytes sent
                            │         (Zero partial response writes)
                            ▼
┌────────────────────────────────────────────────────────┐
│ Phase 2: Response Transmission                         │
│ Runs outside request processing timeout                │
│ 1. Write full pre-encoded frame via write_all          │
│ 2. Flush socket under dedicated write timeout          │
└────────────────────────────────────────────────────────┘
```

### Typed Error and Output Structures
- `ProcessedOutput` and `ResponseOutput` in `crates/daemon/src/dispatcher.rs`:
  ```rust
  #[derive(Debug)]
  struct ProcessedOutput {
      encoded_response: Option<Vec<u8>>,
      completion_error: Option<DaemonError>,
  }

  #[derive(Debug)]
  struct ResponseOutput {
      encoded_response: Vec<u8>,
      completion_error: Option<DaemonError>,
  }
  ```
- `IpcError::TruncatedResponse` in `crates/pam/src/ipc.rs`:
  ```rust
  #[derive(Debug)]
  pub enum IpcError {
      ...
      /// Response stream was truncated before complete frame was received.
      TruncatedResponse { expected: usize, received: usize },
      ...
  }
  ```

---

## 3. Tester Contracts & TDD Red Phase

Two contractual tests were authored prior to production implementation:

### Test 1: `test_timeout_during_write_does_not_corrupt_response` (`dispatcher_tests.rs`)
- Simulates an incomplete request where the client stalls mid-stream under a tight daemon timeout.
- Asserts that the daemon cleanly closes the stream without sending any corrupt or partial response bytes (`n == 0`).
- Asserts that whenever a response is delivered, it is 100% complete and decodes cleanly into a valid `Response`.

### Test 2: `test_pam_ipc_detects_truncated_response` (`ipc_tests.rs`)
- Case 1: Simulates a server announcing 64 bytes of body but writing only 16 bytes before dropping the connection.
  - Asserts that `pam_soos::ipc::authenticate` returns `Err(IpcError::TruncatedResponse { expected: 68, received: 20 })`.
- Case 2: Simulates a server sending only 2 bytes of the 4-byte Big-Endian length header.
  - Asserts that `pam_soos::ipc::authenticate` returns `Err(IpcError::TruncatedResponse { expected: 4, received: 2 })`.
- Asserts that calling the C FFI entry point `pam_sm_authenticate` degrades safely to `PAM_IGNORE`.

---

## 4. Developer Implementation

### 1. `crates/daemon/src/dispatcher.rs`
- Refactored `handle_connection` to isolate `self.read_and_process(&mut stream)` under `connection_timeout`.
- Implemented `write_response` executing `stream.write_all(&encoded_resp).await?; stream.flush().await?;` with dedicated timeout protection.
- Updated `handle_request` to return `Result<ResponseOutput, DaemonError>` across all 17 response generation points.

### 2. `crates/pam/src/ipc.rs`
- Implemented `read_exact_counted` tracking exact byte counts across stream reads.
- Replaced direct `read_exact` calls in `authenticate` with byte-counted verification on both the 4-byte length header and the body slice.
- Added total received bytes completeness validation (`total_received == total_capacity`) prior to `decode`.

---

## 5. Candid Reviewer Findings

The independent candid review executed via `./scripts/candid_review.sh` confirmed:
- Zero `unsafe` additions across all modified crates.
- Zero `unwrap()`, `expect()`, or panicking calls in PAM production pathways.
- Zero Tokio runtime usage in `crates/pam`.
- Zero stdout/stderr stream pollution.
- All code and deliverables strictly adhere to the English-only policy.
- Report published to `AI/candid_review_report.md` with **VERDICT: APPROVED**.

---

## 6. Automated Verification Results

```bash
cargo test -p soos-pam
# 19 passed; 0 failed (including test_pam_ipc_detects_truncated_response)

cargo test -p soos-daemon
# 46 passed; 0 failed (including test_timeout_during_write_does_not_corrupt_response)

cargo test --workspace
# All workspace tests passed cleanly with 0 failures

cargo fmt --check
# Clean formatting verified

cargo clippy --all-targets --all-features -- -D warnings
# Zero warnings across all targets and features
```
