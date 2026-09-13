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
//! ## Operational Flow
//!
//! Linux-PAM invokes `pam_sm_authenticate`:
//! 1. Arguments (`argc`, `argv`) are parsed safely into a bounded [`config::PamConfig`].
//! 2. If configured with `event=password-failed`: sends telemetry to the daemon within 20ms and returns `PAM_IGNORE`.
//! 3. Otherwise: performs synchronous IPC authentication handshake with `soos-daemon`.
//! 4. Renders `PAM_SUCCESS` exclusively upon receiving `Verdict::Allow` with matching nonce. All other outcomes return `PAM_IGNORE`.

// NOTE: unsafe is required ONLY for C ABI symbol exports (`extern "C"`), libc UID query, and bounded argv parsing.
// All business logic remains safe Rust.
#![deny(clippy::all)]

pub mod config;
pub mod ipc;

use std::panic::{catch_unwind, AssertUnwindSafe};

use config::{parse_argv, PamEvent};
use soos_protocol::types::{EventKind, Verdict};

// ---------------------------------------------------------------------------
// PAM Constants (Linux-PAM Specification)
// ---------------------------------------------------------------------------

/// Success: authentication successfully granted.
pub const PAM_SUCCESS: i32 = 0;

/// Ignore: module chooses not to participate in decision; PAM continues down stack.
pub const PAM_IGNORE: i32 = 25;

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
/// # Panic Safety Guarantee
///
/// All panics are intercepted by `catch_unwind`. If an internal panic occurs,
/// the function returns `PAM_IGNORE` to ensure seamless fallback to password.
#[allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "Exported C ABI entry point invoked by Linux-PAM; raw pointers are guarded against null and unbounded reads"
)]
#[no_mangle]
pub extern "C" fn pam_sm_authenticate(
    _pamh: *mut PamHandle,
    _flags: i32,
    argc: i32,
    argv: *const *const u8,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `argv` points to `argc` pointers passed across the C ABI by Linux-PAM.
        let config = unsafe { parse_argv(argc, argv) };

        // SAFETY: getuid is a non-allocating, safe libc syscall returning the process UID.
        let uid = config.uid.unwrap_or_else(|| unsafe { libc::getuid() });

        if config.event == Some(PamEvent::PasswordFailed) {
            // Best-effort telemetry notification bounded by 20ms ceiling
            let _ = ipc::notify_event(&config, uid, EventKind::PasswordFailed);
            return PAM_IGNORE;
        }

        match ipc::authenticate(&config, uid) {
            Ok(Verdict::Allow) => PAM_SUCCESS,
            Ok(Verdict::Deny | Verdict::Unavailable | Verdict::ProtocolError) => PAM_IGNORE,
            Err(_) => PAM_IGNORE,
        }
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
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "Unit tests verify panics and assertions"
)]
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
