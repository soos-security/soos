# Walkthrough 18 — PAM Module: Synchronous IPC Client Integration

> **Date**: 2026-09-13  
> **Target**: Issue #3 (`pam` Module — IPC Client Integration)  
> **Branch**: `feat/pam-ipc-client`  
> **Verification Matrix**: PA1, PA2, PA7, PA8  

---

## 1. Overview & Objectives

Issue #3 implements the synchronous IPC client inside `crates/pam/` (`pam_soos.so`). As a dynamic PAM module loaded into security-sensitive host processes (`login`, `sudo`, display managers), `pam_soos.so` must adhere to strict real-time deadlines (200–250ms), forbid any asynchronous runtimes (zero Tokio), protect all FFI boundaries with `catch_unwind`, never log or handle user passwords, and fail closed to `PAM_IGNORE` under any error or timeout.

### Architectural Invariants Enforced
1. **Zero Asynchronous Runtime (`PA4`)**: Strictly executes synchronous `std::os::unix::net::UnixStream`. Tokio and all asynchronous runtimes remain completely forbidden in the PAM pathway.
2. **Strict Latency Budget (`PA2`)**: Total execution budget is constrained to 200–250ms (configurable via `timeout_ms=...`). Individual read and write timeouts are configured dynamically against the remaining budget. Sockets are closed immediately upon receipt of response.
3. **Fail-Closed Fallback (`PA1`, Invariant 5)**: Offline daemon, socket errors, malformed responses, or timeouts systematically degrade to `PAM_IGNORE`. An error is never converted into `PAM_SUCCESS`.
4. **Panic Safety Guarantee (`PA3`)**: All FFI boundaries (`pam_sm_authenticate`, `pam_sm_setcred`) are intercepted by `catch_unwind(AssertUnwindSafe(...))` systematically returning `PAM_IGNORE`.
5. **Zero Passwords or Secrets (`PA6`)**: The module never inspects, prompts for, or transmits passwords over IPC or across any boundary.
6. **Telemetry Event Notification (`#3.3`)**: When configured with `event=password-failed`, sends `EventKind::PasswordFailed` to `/run/soos/daemon.sock` within a strict 20ms ceiling and immediately returns `PAM_IGNORE`.
7. **Anti-Replay Nonce Validation**: Each request carries a 256-bit cryptographic random `request_id` generated via `getrandom`. The received response must match the nonce bit-for-bit.

---

## 2. Multi-Agent TDD Implementation Cycle

### Phase 1 — Architect Sub-Agent
- Designed configuration parser and IPC modules:
  - `crates/pam/src/config.rs`: `PamConfig`, `PamEvent`, `parse_argv` with bounded C string scanning (`MAX_ARG_LEN = 256`, `MAX_ARGC = 64`).
  - `crates/pam/src/ipc.rs`: `IpcError`, `authenticate` (synchronous handshake within budget), `notify_event` (best-effort notification within 20ms).
  - `crates/pam/src/lib.rs`: Exports C ABI entry points `pam_sm_authenticate` and `pam_sm_setcred` with `catch_unwind`.
- Configured workspace dependencies: `getrandom = "0.3"`, `libc = "0.2"`, `soos-protocol = { workspace = true }`. Added `crate-type = ["cdylib", "rlib"]` for cargo test integration.

### Phase 1.5 — Plan Evaluator Sub-Agent
- Evaluated proposed implementation plan against `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, and `AI/VERIFICATION_MATRIX.md` across the 6 core pillars:
  - Pillar 1: Architectural Alignment & Threat Model (PASS)
  - Pillar 2: PAM Real-Time Latency & Concurrency (PASS)
  - Pillar 3: Panic Safety & Fail-Closed Behavior (PASS)
  - Pillar 4: Dependency Isolation & Banned Crates (PASS)
  - Pillar 5: Data Confidentiality & Zeroization (PASS)
  - Pillar 6: Test Integrity & TDD Contracts (PASS)
- Authored report in `AI/plan_evaluation_report.md` with **`VALIDATION_VERDICT: APPROVED`**.

### Phase 2 — Tester Sub-Agent (TDD Red Phase)
- Authored comprehensive contract tests before production implementation:
  - `crates/pam/tests/config_tests.rs`: default configuration, negative/zero argc, timeout parsing and clamping, custom socket and service, corrupted arguments safety.
  - `crates/pam/tests/ipc_tests.rs`: offline daemon fallback (`PA1`), slow daemon timeout (`PA2`), nominal `Allow` -> `PAM_SUCCESS`, `Deny` -> `PAM_IGNORE`, `Unavailable` -> `PAM_IGNORE`, nonce mismatch rejection, oversized payload rejection, password-failed event within 20ms, password-failed offline fallback, setcred fallback.
- Confirmed compilation failure against initial skeletons (TDD Red state established).

### Phase 3 — Auditor Sub-Agent
- Audited interface specifications:
  - Validated `#![deny(clippy::all)]` and workspace lints in `crates/pam`.
  - Audited error paths for fail-closed fallback to `PAM_IGNORE`.
  - Confirmed zero `unwrap()`, `expect()`, `panic!()`, `todo!()`, or `unimplemented!()` in PAM production code.
  - Confirmed zero `println!`, `eprintln!`, `print!`, `eprint!`, or `dbg!` macro calls.
  - Confirmed memory bounds: `MAX_MESSAGE_SIZE` (4096) checked prior to body allocation; bounded C string scan.

### Phase 4 — Developer Sub-Agent (TDD Green Phase)
- Implemented production code satisfying all pre-written and edge-case tests:
  - Bounded argv parsing in `crates/pam/src/config.rs` with whitespace trimming, null pointer checks, and optional target `uid=` override.
  - Dynamic latency budget tracking in `crates/pam/src/ipc.rs` recalculating remaining budget before both header and payload reads to enforce cumulative deadlines.
  - Direct single-allocation framed response buffer eliminating redundant secondary `Vec` and `memcpy`.
  - Strict 20ms ceiling enforcement in `notify_event` without invalid extra grace period.
  - Safe monotonic time extraction via `libc::clock_gettime(libc::CLOCK_MONOTONIC, ...)`.
  - Integrated IPC client in `crates/pam/src/lib.rs`.
- Ran test suite: **30/30 tests green** in `soos-pam` (98/98 tests green workspace-wide).
- Formatting verified: `cargo fmt --check` (100% compliant).
- Linter verified: `cargo clippy --all-targets --all-features -- -D warnings` (zero warnings).

### Phase 5 — Candid Reviewer Sub-Agent
- Executed cold diff review across the 5 pillars against `origin/main`.
- Validated absence of Tokio, absence of panics, bounded timeouts, fail-closed fallback, and test integrity.
- Authored `AI/candid_review_report.md` with **`VERDICT: APPROVED`**.
- Passed automated dual-layer candid audit (`./scripts/candid_subagent.sh`).

### Phase 6 — Traceability Sub-Agent
- Synchronized `AI/BACKLOG.md` marking sub-issues `#3.1`, `#3.2`, `#3.3`, and `#3.4` as completed (`[x]`).
- Synchronized `AI/VERIFICATION_MATRIX.md` marking criteria `PA1`, `PA2`, `PA7`, and `PA8` as **`☑ Validated`**.
- Authored this sequential walkthrough (`18_pam_ipc_client_integration.md`).

---

## 3. Verification Evidence

| Criteria | Description | Verification Evidence | Status |
|---|---|---|---|
| **PA1** | Returns `PAM_IGNORE` when daemon is unavailable | `test_ipc_offline_daemon_returns_ignore`, Docker `pamtester` | ☑ Validated |
| **PA2** | Returns `PAM_IGNORE` on timeout (> 250ms) | `test_ipc_slow_daemon_timeout`, `test_ipc_slow_daemon_body_timeout` (cumulative deadline respected) | ☑ Validated |
| **PA3** | `catch_unwind` wraps all FFI entry points | `tests::panic_safety_returns_pam_ignore` | ☑ Validated |
| **PA4** | NEVER starts Tokio runtime | Invariant test `test_pam_crate_has_no_tokio_dependency` | ☑ Validated |
| **PA5** | Zero `unwrap()` or `expect()` in production code | Invariant test `test_pam_crate_has_no_unwraps_or_expects` | ☑ Validated |
| **PA6** | Neither reads nor transmits passwords | Invariant test `test_protocol_request_and_response_have_no_sensitive_fields` | ☑ Validated |
| **PA7** | Correct C ABI (loadable by Linux-PAM) | C ABI exports `pam_sm_authenticate` & `pam_sm_setcred`, Docker test | ☑ Validated |
| **PA8** | Absent module = PAM authentication remains functional | Universal stack ordering and fail-closed non-interference | ☑ Validated |

---

## 4. Test Results Summary

```text
test tests::test_business_crates_forbid_unsafe_code ... ok
test tests::test_pam_crate_has_no_tokio_dependency ... ok
test tests::test_workspace_cargo_toml_enforces_overflow_checks ... ok
test tests::test_protocol_request_and_response_have_no_sensitive_fields ... ok
test tests::test_pam_crate_has_no_unwraps_or_expects_in_production_code ... ok
test tests::test_pam_crate_has_no_stdout_or_stderr_prints_in_production_code ... ok
test tests::test_no_nokhwa_in_any_cargo_toml ... ok
test tests::test_no_opencv_in_any_cargo_toml ... ok

test test_default_config_on_null_argv ... ok
test test_negative_or_zero_argc_returns_default ... ok
test test_parse_argc_exceeding_max_argc_is_bounded ... ok
test test_parse_custom_socket_and_service ... ok
test test_parse_empty_socket_and_service_preserves_default ... ok
test test_parse_event_password_failed ... ok
test test_parse_timeout_clamping ... ok
test test_parse_timeout_ms ... ok
test test_parse_uid_override ... ok
test test_parse_unterminated_string_exceeding_max_arg_len ... ok
test test_parse_whitespace_padded_arguments ... ok
test test_parse_zero_timeout_clamps_to_min ... ok
test test_safely_ignores_unknown_or_corrupted_arguments ... ok

test test_setcred_always_returns_ignore ... ok
test test_ipc_notify_event_offline ... ok
test test_ipc_offline_daemon_returns_ignore ... ok
test test_ipc_password_failed_daemon_offline_returns_ignore ... ok
test test_ipc_deny_returns_ignore ... ok
test test_ipc_empty_response_returns_ignore ... ok
test test_ipc_nominal_allow_returns_success ... ok
test test_ipc_notify_event_direct ... ok
test test_ipc_oversized_response_rejected ... ok
test test_ipc_request_id_mismatch_returns_ignore ... ok
test test_ipc_password_failed_event_notification ... ok
test test_ipc_unavailable_returns_ignore ... ok
test test_ipc_direct_authenticate_timeout_mapping ... ok
test test_ipc_slow_daemon_timeout ... ok
test test_ipc_slow_daemon_body_timeout ... ok

test result: ok. 98 passed; 0 failed; 0 ignored; finished in 0.45s
```
