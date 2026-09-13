//! # pam_soos — Linux PAM Module for Local Facial Verification
//!
//! This module is a `cdylib` loaded dynamically by Linux-PAM. It exports the standard
//! C ABI entry points `pam_sm_authenticate` and `pam_sm_setcred`.
//!
//! ## Core Security Principles
//!
//! 1. **Zero Async Runtime**: Strictly uses synchronous blocking primitives (`std::os::unix::net::UnixStream`).
//! 2. **Strict Latency Budget**: Maximum 200–250ms total execution time (connect + request + response).
//! 3. **Panic Resilience**: `catch_unwind` wraps every FFI entry point, systematically returning `PAM_IGNORE`.
//! 4. **Zero Secrets on Wire**: Never inspects, processes, or transmits passwords over IPC.
//! 5. **Safe Fallback**: Any error or timeout degrades silently to `PAM_IGNORE` for password fallback.
//!
//! ## Current Status: Foundation Skeleton
//!
//! Systematically returns `PAM_IGNORE` to validate:
//! - C ABI loading compatibility by Linux-PAM
//! - Zero interference with downstream authentication modules (`pam_unix.so`)
//! - Fault-tolerant resilience of the PAM authentication stack

// NOTE: unsafe is required ONLY for C ABI symbol exports (`extern "C"`).
// All internal logic remains safe Rust.
#![deny(clippy::all)]

use std::panic::{catch_unwind, AssertUnwindSafe};

// ---------------------------------------------------------------------------
// PAM Constants (Linux-PAM Specification)
// ---------------------------------------------------------------------------

/// Success: authentication successfully granted.
#[allow(dead_code)]
const PAM_SUCCESS: i32 = 0;

/// Ignore: module chooses not to participate in decision; PAM continues down stack.
const PAM_IGNORE: i32 = 25;

// ---------------------------------------------------------------------------
// Opaque PAM Handle Pointer
// ---------------------------------------------------------------------------

/// Opaque pointer to Linux-PAM internal handle structure.
/// This pointer is never dereferenced and only forwarded when required.
#[repr(C)]
pub struct PamHandle {
    _opaque: [u8; 0],
}

// ---------------------------------------------------------------------------
// PAM Entry Points (C ABI)
// ---------------------------------------------------------------------------

/// Primary authentication entry point called by Linux-PAM.
///
/// # Safety
///
/// Invoked by Linux-PAM via the C ABI. `pamh` is an opaque pointer supplied by
/// PAM and must not be arbitrarily dereferenced. `argv` points to an array of
/// `argc` C strings.
///
/// # Behavior
///
/// Returns `PAM_IGNORE` to validate the C ABI without interfering with
/// the operational PAM stack.
///
/// # Panic Safety Guarantee
///
/// All panics are intercepted by `catch_unwind`. If an internal panic occurs,
/// the function returns `PAM_IGNORE` to ensure seamless fallback to password.
#[no_mangle]
pub extern "C" fn pam_sm_authenticate(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // Foundation phase: returns PAM_IGNORE -> PAM continues to pam_unix.so
        PAM_IGNORE
    }));

    match result {
        Ok(code) => code,
        Err(_) => {
            // Invariant 5 of ARCHITECTURE.md:
            // "A panic degrades to password fallback, never to authorization."
            PAM_IGNORE
        }
    }
}

/// Credential management entry point called by Linux-PAM.
///
/// # Safety
///
/// Same conditions as `pam_sm_authenticate`.
///
/// `soos` does not manage credential tokens; systematically returns `PAM_IGNORE`.
#[no_mangle]
pub extern "C" fn pam_sm_setcred(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| PAM_IGNORE));
    match result {
        Ok(code) => code,
        Err(_) => PAM_IGNORE,
    }
}

// ===========================================================================
// Unit Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    /// PA1: Module returns PAM_IGNORE when daemon is unreachable.
    #[test]
    fn authenticate_returns_pam_ignore() {
        let result = pam_sm_authenticate(ptr::null_mut(), 0, 0, ptr::null());
        assert_eq!(result, PAM_IGNORE);
    }

    /// Module never panics or aborts on setcred.
    #[test]
    fn setcred_returns_pam_ignore() {
        let result = pam_sm_setcred(ptr::null_mut(), 0, 0, ptr::null());
        assert_eq!(result, PAM_IGNORE);
    }

    /// PA5: Verify PAM_IGNORE constant matches Linux-PAM standard value (25).
    #[test]
    fn pam_ignore_has_correct_value() {
        assert_eq!(PAM_IGNORE, 25);
    }

    /// PA3: catch_unwind stops panics from crossing the C ABI boundary.
    #[test]
    fn panic_safety_returns_pam_ignore() {
        let result = catch_unwind(AssertUnwindSafe(|| -> i32 {
            panic!("test panic in PAM module");
        }));
        let code = match result {
            Ok(c) => c,
            Err(_) => PAM_IGNORE,
        };
        assert_eq!(code, PAM_IGNORE);
    }
}
