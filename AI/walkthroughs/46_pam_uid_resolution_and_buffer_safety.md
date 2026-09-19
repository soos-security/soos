# Walkthrough 46 — Robust UID Resolution and Telemetry Event Buffer Safety

## Context & Objectives

- **Issue**: Issue #29 (`fix(pam): Robust UID resolution and buffer safety`) / GitHub #68
- **Branch**: `fix/pam-uid-resolution`
- **Mission**:
  1. Implement dynamic buffer growth for POSIX `getpwnam_r` in `pam_soos`, starting with `sysconf(_SC_GETPW_R_SIZE_MAX)` (minimum 1024 bytes), doubling on `ERANGE`, and capping at 64KB to safely support enterprise directory backends (LDAP, Active Directory / SSSD) without OOM vulnerabilities (#29.1).
  2. Include `uid: Option<u32>` in the `Event` wire payload for `PasswordFailed` telemetry notifications, enabling `soos-daemon` and `EvidenceStore` to accurately attribute snapshots to the targeted user rather than the caller process UID (e.g. GDM / root) (#29.2).

---

## 1. Architectural Design & Compliance (Phase 1 & Phase 1.5)

- **Plan Evaluator Sub-Agent**:
  Evaluated the proposed implementation plan across the 6 architectural pillars in `AI/plan_evaluator_report.md`, authoring formal approval: `VALIDATION_VERDICT: APPROVED`.
- **Architectural Invariants Honored**:
  - `AI/ARCHITECTURE.md` §5 (PAM Module): Retained reentrant `libc::getpwnam_r` POSIX user lookup with fail-closed fallback to `libc::getuid()` or `PAM_IGNORE`.
  - Wire Framing & Schemas: Updated `Event` wire schema in `soos-protocol` to carry `uid: Option<u32>`, keeping total serialized frame size within `MAX_MESSAGE_SIZE` (4,096 bytes).
  - Real-Time Deadlines: Maintained strict 20ms write timeout for `notify_event` and synchronous sub-millisecond execution for UID resolution.
  - Memory Hygiene: Capped dynamic buffer growth at `MAX_PW_BUF_SIZE = 64 * 1024` (64KB), eliminating unbounded heap exhaustion attack vectors.

---

## 2. Test Contracts (Phase 2 — TDD Red Phase)

Contractual unit and integration tests were authored before production code:
1. `test_getpwnam_r_handles_erange_retry` (`crates/pam/tests/uid_resolution_tests.rs`):
   Initializes lookup with a 4-byte buffer to force `libc::getpwnam_r` to return `ERANGE` for `"root"`, verifying that dynamic buffer growth doubles allocations across retries and successfully resolves UID 0.
2. `test_getpwnam_r_caps_at_max_buffer_size` (`crates/pam/tests/uid_resolution_tests.rs`):
   Verifies that when `max_size` is too small to accommodate the user entry, the function fails closed and returns `None`.
3. `test_getpwnam_r_nominal_resolution` (`crates/pam/tests/uid_resolution_tests.rs`):
   Verifies default dynamic sizing against system user `"root"`.
4. `test_getpwnam_r_nonexistent_user_returns_none` (`crates/pam/tests/uid_resolution_tests.rs`):
   Verifies unknown usernames cleanly return `None`.
5. `test_password_failed_event_includes_uid` (`crates/pam/tests/ipc_tests.rs`):
   Verifies that calling `pam_soos::ipc::notify_event` transmits `event.uid == Some(1001)` over the IPC stream.
6. `test_12_4_password_failed_event_captures_evidence_snapshot` (`crates/daemon/tests/pipeline_integration_tests.rs`):
   Verifies that the daemon associates intrusion snapshots with the `target_uid` supplied in the `Event` payload rather than the caller's `peer_uid`.

All contractual tests failed as expected during Phase 2 (unresolved functions and missing struct fields).

---

## 3. Implementation (Phase 3 & Phase 4)

- **Protocol Layer (`crates/protocol`)**:
  - Added `pub uid: Option<u32>` to `Event` struct in `crates/protocol/src/types.rs`.
  - Updated roundtrip codec test in `crates/protocol/src/codec.rs`.
  - Updated `arb_valid_event()` strategy in `crates/protocol/tests/property_tests.rs` to fuzz both `Some(u32)` and `None`.
- **PAM Module Layer (`crates/pam`)**:
  - Implemented `INITIAL_PW_BUF_SIZE = 1024` and `MAX_PW_BUF_SIZE = 64 * 1024`.
  - Added `initial_buffer_size()` using `libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX)` with `usize::try_from(sc)` to safely reject negative return values and clamp between 1KB and 64KB.
  - Implemented `resolve_username_to_uid` and `resolve_username_to_uid_with_bounds` with checked growth (`checked_mul(2)`), ceiling clamping, and fail-closed termination.
  - Updated `notify_event(config, uid, event_kind)` to send `uid: Some(uid)`.
- **Daemon Layer (`crates/daemon`)**:
  - In `crates/daemon/src/dispatcher.rs` `handle_event`, extracted `let target_uid = event.uid.unwrap_or(peer_uid);` and recorded snapshots under `target_uid`.

---

## 4. Candid Review & Traceability (Phase 5 & Phase 6)

- **Candid Reviewer Sub-Agent**:
  Audited the raw git diff against `origin/main` across 5 pillars in `AI/candid_review_report.md` with verdict: `VERDICT: APPROVED`.
- **Traceability Updates**:
  - Updated `Docs/IPC_PROTOCOL.md` documenting `uid` in `Event`.
  - Updated `AI/VERIFICATION_MATRIX.md` recording criteria `PA13` and `PA14` as Verified.
  - Checked off Sub-issues #29.1 and #29.2 in `AI/BACKLOG.md`.
  - Synchronized GitHub Issue #68 via `scripts/sync_issue.py`.

---

## 5. Verification Results

- `cargo test --workspace`: 100% pass (0 failures, 0 ignored).
- `cargo clippy --all-targets --all-features -- -D warnings`: Clean (0 warnings).
- `cargo fmt --check`: Clean (0 formatting discrepancies).
