# Walkthrough 29 — PAM Module `pam-bindings` 0.3.0 Migration & Syslog Logging

## Context & Objectives

- **Issue**: Backlog Issue #14 / GitHub Issue #21 (`feat(pam): Migrate to pam-bindings 0.3.0 with PamHooks trait`)
- **Target Branch**: `feat/pam-bindings-migration`
- **Scope**:
  - Migrate `crates/pam` (`pam_soos.so`) to `pam-bindings` 0.3.0.
  - Implement `PamHooks` trait on `SoosPam`.
  - Maintain fail-closed `catch_unwind` wrapping.
  - Implement bounded syslog logging on caught panics (`LOG_AUTHPRIV | LOG_ERR`) without secret leakage.
  - Ensure zero test weakening and strict compliance with all architectural invariants.

---

## Changes Made

### 1. Workspace Configuration & Scaffolding
- Added `pam_bindings = { package = "pam-bindings", version = "0.3.0" }` to `[workspace.dependencies]` in root `Cargo.toml`.
- Registered `pam_bindings = { workspace = true }` in `crates/pam/Cargo.toml`.
- Added `crates/pam/build.rs` to detect system `libpam.so` / `libpam.so.0` across standard library directories (`/lib/x86_64-linux-gnu`, `/usr/lib/x86_64-linux-gnu`, etc.) and ensure linker flags are emitted for native compilation without requiring developer packages.

### 2. Syslog Panic Logging Module (`crates/pam/src/syslog.rs`)
- Implemented `format_panic_message` to produce bounded log messages:
  `soos-pam: authentication panic caught at {location}: {summary}`.
- Sanitized embedded nul characters (`\0`) to prevent C string truncations.
- Dispatched messages via `libc::syslog(libc::LOG_AUTHPRIV | libc::LOG_ERR, b"%s\0", ...)`.
- Registered an internal panic hook via `std::sync::Once` to capture panic file/line/column in thread-local storage while silencing stderr to prevent graphical display manager stream corruption.

### 3. Argument Parsing (`crates/pam/src/config.rs`)
- Added `parse_cstrs` to safely parse `Vec<&CStr>` from `PamHooks` without raw pointers.
- Unified option extraction (`timeout_ms=`, `event=password-failed`, `socket=`, `socket_path=`, `service=`, `uid=`) via a shared `apply_arg` helper.

### 4. PAM Module Implementation (`crates/pam/src/lib.rs`)
- Defined `pub struct SoosPam;`.
- Implemented `pam_bindings::module::PamHooks for SoosPam`:
  - `sm_authenticate`: invokes `SoosPam::authenticate_with_config` wrapped in `catch_unwind`, logging panics to syslog and returning `PAM_IGNORE`.
  - `sm_setcred`: returns `PAM_IGNORE`.
- Resolved target UID via explicit argument override, `pamh.get_user(None)` lookup with reentrant POSIX `libc::getpwnam_r`, or `libc::getuid()` fallback.
- Exported Linux-PAM C ABI functions (`pam_sm_authenticate`, `pam_sm_setcred`, `pam_sm_acct_mgmt`, `pam_sm_chauthtok`, `pam_sm_open_session`, `pam_sm_close_session`).

### 5. Automated Tests & Invariants
- Authored 8 contractual integration tests in `crates/pam/tests/pam_bindings_tests.rs`.
- Added PA11 (`test_pam_crate_uses_pam_bindings_and_implements_pam_hooks`) and PA12 (`test_pam_crate_has_syslog_panic_logging_without_secrets`) to `tests/invariants/src/lib.rs`.
- Verified all 18 existing `ipc_tests.rs` and 13 `config_tests.rs` continue to pass cleanly without modification.

---

## Verification Evidence

| Step | Command | Result |
|---|---|---|
| Red Phase | `cargo test -p soos-pam --test pam_bindings_tests` | ❌ Failed initially (unresolved symbols) |
| Unit & Integration Tests | `cargo test -p soos-pam` | ✅ 4 unit, 13 config, 18 IPC, 8 pam-bindings tests passed |
| Invariant Tests | `cargo test -p soos-invariants` | ✅ 12/12 invariant tests passed |
| Workspace Tests | `cargo test --all` | ✅ 100% test pass across all workspace crates |
| Release Build | `cargo build --release` | ✅ Clean build; dynamic symbols verified via `nm -D` |
| Code Formatting | `cargo fmt --check` | ✅ Formatted cleanly |
| Clippy Lints | `cargo clippy --all-targets --all-features -- -D warnings` | ✅ 0 warnings, 0 errors |
| Candid Review | `./scripts/candid_review.sh` | ✅ All 7 audits passed |

---

## Acceptance Matrix Update

- **PA11**: `pam-bindings` 0.3.0 `PamHooks` trait implementation — **Verified**
- **PA12**: Syslog panic logging without secret leakage — **Verified**
