# Candid Review Report

- **Date**: 2026-09-19
- **Target Branch / Commit**: `fix/pam-uid-resolution`
- **Audited Files**:
  - `crates/pam/src/lib.rs`
  - `crates/pam/src/ipc.rs`
  - `crates/pam/tests/uid_resolution_tests.rs`
  - `crates/pam/tests/ipc_tests.rs`
  - `crates/protocol/src/types.rs`
  - `crates/protocol/src/codec.rs`
  - `crates/protocol/tests/property_tests.rs`
  - `crates/daemon/src/dispatcher.rs`
  - `crates/daemon/tests/pipeline_integration_tests.rs`
  - `Docs/IPC_PROTOCOL.md`
  - `scripts/sync_issue.py`

## 1. Executive Summary

This pull request implements robust UID resolution in `pam_soos` and telemetry event attribution in `soos-protocol` and `soos-daemon`. It resolves POSIX `getpwnam_r` buffer exhaustion (`ERANGE`) when querying users in enterprise directories (LDAP, Active Directory / SSSD) by dynamically doubling buffer allocations up to a strict 64KB cap. Additionally, it wires the resolved target UID into the `Event` wire payload on `PasswordFailed`, enabling `EvidenceStore` snapshots to accurately record the targeted account rather than the calling process UID (e.g. GDM / root).

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**: State transitions, error handling, and bounds checking are correct.
- `resolve_username_to_uid_with_bounds` starts with clamped initial sizing (`sysconf(_SC_GETPW_R_SIZE_MAX)` or 1024), doubles buffer size upon receiving `libc::ERANGE` using checked arithmetic (`checked_mul(2)`), and halts when exceeding `max_size` (64KB).
- `handle_event` in `soos-daemon` accurately reads `target_uid = event.uid.unwrap_or(peer_uid)` and stores evidence snapshots indexed by `target_uid`.
- Full alignment with §5 PAM Module and Issue #29 requirements.

### PAM Concurrency & Deadlines
- **Pass**: Zero Tokio or asynchronous runtime invocations within `pam_soos`.
- Strict 20ms write timeout maintained for `notify_event`.
- Reentrant `libc::getpwnam_r` is safe for multi-threaded PAM consumers and executes within sub-millisecond budgets.
- Output isolation verified: zero `println!`, `eprintln!`, or `dbg!` macro usages in PAM pathways.

### Panic Safety & Fallback
- **Pass**: Zero `unwrap()`, `expect()`, or panicking branches in production code (`crates/pam/src/lib.rs`, `crates/pam/src/ipc.rs`).
- All conversions from libc types use safe constructs (`usize::try_from(sc)`, `checked_mul`).
- `resolve_username_to_uid` gracefully returns `None` on unresolvable users or buffer exhaustion, falling back to `libc::getuid()` or `PAM_IGNORE`.
- All C ABI entry points remain shielded by `catch_unwind`.

### Test Integrity & Anti-Weakening
- **Pass**: Zero pre-existing tests were weakened, modified, or deleted.
- Contractual unit and integration tests authored before production implementation:
  - `test_getpwnam_r_handles_erange_retry`: Verifies recovery and doubling from a tiny 4-byte buffer to successful resolution of root UID 0.
  - `test_getpwnam_r_caps_at_max_buffer_size`: Verifies fail-closed behavior when cap prevents resolution.
  - `test_password_failed_event_includes_uid`: Verifies wire transmission of `event.uid == Some(1001)`.
  - `test_12_4_password_failed_event_captures_evidence_snapshot`: Verifies daemon records snapshot with `target_uid` distinct from caller `peer_uid`.

### Memory & Secret Bounds
- **Pass**: Allocations are bounded by `MAX_PW_BUF_SIZE` (64KB), eliminating heap exhaustion (OOM) attack vectors.
- FFI pointers are checked for nullity (`result.is_null()`).
- No passwords, tokens, or biometric templates are exposed in the `Event` schema or logs.
- Memory zeroization invariants remain intact.

## 3. Detailed Findings & Action Items
- None. All checks passed with zero warnings or deficiencies.

## 4. Final Verdict
**VERDICT: APPROVED**
