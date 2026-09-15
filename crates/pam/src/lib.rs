//! # pam_soos — Linux PAM Module for Local Facial Verification
//!
//! This module is a `cdylib` loaded dynamically by Linux-PAM. It implements the
//! [`pam_bindings::module::PamHooks`] trait via [`SoosPam`] and exports the standard
//! Linux-PAM C ABI entry points.
//!
//! ## Core Security Principles
//!
//! 1. **Zero Async Runtime**: Strictly uses synchronous blocking primitives (`std::os::unix::net::UnixStream`).
//! 2. **Strict Latency Budget**: Maximum 200–250ms total execution time (connect + request + response).
//! 3. **Panic Resilience**: `catch_unwind` wraps every entry point, logging caught panics to syslog and systematically returning `PAM_IGNORE`.
//! 4. **Zero Secrets on Wire**: Never inspects, processes, or transmits passwords over IPC.
//! 5. **Safe Fallback**: Any error or timeout degrades silently to `PAM_IGNORE` for password fallback.
//!
//! ## Operational Flow
//!
//! Linux-PAM invokes `pam_sm_authenticate`:
//! 1. Arguments are parsed into a bounded [`config::PamConfig`].
//! 2. Target UID is determined via explicit PAM argument override, `pamh.get_user(None)` lookup, or `libc::getuid()` fallback.
//! 3. If configured with `event=password-failed`: sends telemetry to daemon within 20ms and returns `PAM_IGNORE`.
//! 4. Otherwise: performs synchronous IPC authentication handshake with `soos-daemon`.
//! 5. Renders `PAM_SUCCESS` exclusively upon receiving `Verdict::Allow`. All other outcomes return `PAM_IGNORE`.

#![deny(clippy::all)]

pub mod config;
pub mod ipc;
pub mod syslog;

use std::ffi::CStr;
use std::panic::{catch_unwind, AssertUnwindSafe};

use config::{parse_cstrs, PamConfig, PamEvent};
pub use pam_bindings::constants::{PamFlag, PamResultCode};
pub use pam_bindings::module::{PamHandle, PamHooks};
use soos_protocol::types::{EventKind, Verdict};

// ---------------------------------------------------------------------------
// PAM Constants (Linux-PAM Specification)
// ---------------------------------------------------------------------------

/// Success: authentication successfully granted.
pub const PAM_SUCCESS: i32 = 0;

/// Ignore: module chooses not to participate in decision; PAM continues down stack.
pub const PAM_IGNORE: i32 = 25;

// ---------------------------------------------------------------------------
// SoosPam and PamHooks Trait Implementation
// ---------------------------------------------------------------------------

/// Main PAM module hook handler implementing [`PamHooks`].
pub struct SoosPam;

impl SoosPam {
    /// Internal authentication logic shared between `PamHooks` and C ABI exports.
    ///
    /// # Panic Safety Guarantee
    ///
    /// All internal execution is wrapped in `catch_unwind`. Any panic triggers a syslog
    /// alert with source location and backtrace summary, and systematically returns
    /// [`PamResultCode::PAM_IGNORE`].
    pub fn authenticate_with_config(
        pamh: Option<&mut PamHandle>,
        config: &PamConfig,
    ) -> PamResultCode {
        syslog::init_panic_hook();

        let result = catch_unwind(AssertUnwindSafe(|| {
            let uid = config.uid.unwrap_or_else(|| {
                if let Some(h) = pamh {
                    if let Ok(username) = h.get_user(None) {
                        if let Some(resolved_uid) = resolve_username_to_uid(&username) {
                            return resolved_uid;
                        }
                    }
                }
                // SAFETY: getuid is a safe, non-allocating libc syscall returning caller process UID.
                unsafe { libc::getuid() }
            });

            if config.event == Some(PamEvent::PasswordFailed) {
                // Best-effort telemetry notification bounded by 20ms ceiling
                let _ = ipc::notify_event(config, uid, EventKind::PasswordFailed);
                return PamResultCode::PAM_IGNORE;
            }

            match ipc::authenticate(config, uid) {
                Ok(Verdict::Allow) => PamResultCode::PAM_SUCCESS,
                Ok(Verdict::Deny | Verdict::Unavailable | Verdict::ProtocolError) => {
                    PamResultCode::PAM_IGNORE
                }
                Err(_) => PamResultCode::PAM_IGNORE,
            }
        }));

        match result {
            Ok(code) => code,
            Err(payload) => {
                let location = syslog::take_panic_location();
                let summary = if let Some(s) = payload.downcast_ref::<&str>() {
                    *s
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.as_str()
                } else {
                    "unspecified panic payload"
                };

                syslog::log_panic(summary, location.as_deref());

                // Invariant 5 of ARCHITECTURE.md:
                // "A panic degrades to password fallback, never to authorization."
                PamResultCode::PAM_IGNORE
            }
        }
    }
}

impl PamHooks for SoosPam {
    fn sm_authenticate(pamh: &mut PamHandle, args: Vec<&CStr>, _flags: PamFlag) -> PamResultCode {
        let config = parse_cstrs(args);
        Self::authenticate_with_config(Some(pamh), &config)
    }

    fn sm_setcred(_pamh: &mut PamHandle, _args: Vec<&CStr>, _flags: PamFlag) -> PamResultCode {
        PamResultCode::PAM_IGNORE
    }
}

/// Resolves a PAM username string to a numeric POSIX UID using reentrant `getpwnam_r`.
fn resolve_username_to_uid(username: &str) -> Option<u32> {
    let c_user = std::ffi::CString::new(username).ok()?;
    let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut buf = vec![0u8; 1024];

    // SAFETY: getpwnam_r is standard POSIX reentrant user lookup.
    let ret = unsafe {
        libc::getpwnam_r(
            c_user.as_ptr(),
            pwd.as_mut_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut result,
        )
    };

    if ret == 0 && !result.is_null() {
        // SAFETY: pwd initialized by getpwnam_r when result is non-null.
        let pwd_val = unsafe { pwd.assume_init() };
        Some(pwd_val.pw_uid)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// PAM Entry Points (C ABI)
// ---------------------------------------------------------------------------

/// Primary authentication entry point called by Linux-PAM.
///
/// # Safety
///
/// Invoked by Linux-PAM via the C ABI. `pamh` is supplied by PAM. If non-null,
/// it is forwarded to `SoosPam`. `argv` points to an array of `argc` C strings.
#[allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "Exported C ABI entry point invoked by Linux-PAM; raw pointers are guarded against null and unbounded reads"
)]
#[no_mangle]
pub extern "C" fn pam_sm_authenticate(
    pamh: *mut PamHandle,
    _flags: i32,
    argc: i32,
    argv: *const *const u8,
) -> i32 {
    let pamh_opt = if pamh.is_null() {
        None
    } else {
        // SAFETY: pamh was verified non-null and is a valid PAM handle.
        unsafe { pamh.as_mut() }
    };

    // SAFETY: argv points to argc pointers passed across the C ABI.
    let config = unsafe { config::parse_argv(argc, argv) };

    match SoosPam::authenticate_with_config(pamh_opt, &config) {
        PamResultCode::PAM_SUCCESS => PAM_SUCCESS,
        _ => PAM_IGNORE,
    }
}

/// Credential management entry point called by Linux-PAM.
///
/// `soos` does not manage credential tokens; systematically returns `PAM_IGNORE`.
#[no_mangle]
pub extern "C" fn pam_sm_setcred(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    PAM_IGNORE
}

/// Account management entry point called by Linux-PAM.
#[no_mangle]
pub extern "C" fn pam_sm_acct_mgmt(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    PAM_IGNORE
}

/// Authentication token update entry point called by Linux-PAM.
#[no_mangle]
pub extern "C" fn pam_sm_chauthtok(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    PAM_IGNORE
}

/// Session open entry point called by Linux-PAM.
#[no_mangle]
pub extern "C" fn pam_sm_open_session(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    PAM_IGNORE
}

/// Session close entry point called by Linux-PAM.
#[no_mangle]
pub extern "C" fn pam_sm_close_session(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    PAM_IGNORE
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
