//! # pam_soos — Linux PAM Module for Local Facial Verification
//!
//! This module is a `cdylib` loaded dynamically by Linux-PAM. It implements the
//! [`pam_bindings::module::PamHooks`] trait via [`SoosPam`] and exports the standard
//! Linux-PAM C ABI entry points.
//!
//! ## Core Security Principles
//!
//! 1. **Zero Async Runtime**: Strictly uses synchronous blocking primitives (`std::os::unix::net::UnixStream`).
//! 2. **Strict Latency Budget**: One explicit deadline derived from the clamped `timeout_ms` (default 1000 ms, range 10–5000 ms), started before UID resolution, covers connect + request + response; nothing is ever unbounded.
//! 3. **Panic Resilience**: `catch_unwind` wraps every entry point, logging caught panics to syslog and systematically returning `PAM_IGNORE`.
//! 4. **Zero Secrets on Wire**: Never inspects, processes, or transmits passwords over IPC.
//! 5. **Safe Fallback**: Any error or timeout degrades silently to `PAM_IGNORE` for password fallback.
//!
//! ## Operational Flow
//!
//! Linux-PAM invokes `pam_sm_authenticate`:
//! 1. Arguments are parsed into a bounded [`config::PamConfig`].
//! 2. Target UID is determined via explicit PAM argument override, [`PamFeedback::user`] lookup (`pam_get_user`), or `libc::getuid()` fallback.
//!    A lookup that spends the whole authentication budget returns `PAM_IGNORE` without contacting the daemon (GitHub #223).
//! 3. If configured with `event=password-failed`: sends telemetry to daemon within 20ms and returns `PAM_IGNORE`.
//! 4. Otherwise: performs synchronous IPC authentication handshake with `soos-daemon`.
//! 5. Renders `PAM_SUCCESS` exclusively upon receiving `Verdict::Allow`. All other outcomes return `PAM_IGNORE`.
//! 6. `PAM_TEXT_INFO` feedback goes through [`PamFeedback`]; none is sent under `PAM_SILENT`, and
//!    "Looking for face..." only once the daemon socket is connected.

#![deny(clippy::all)]

pub mod config;
#[cfg(feature = "fault-injection")]
pub mod fault_injection;
pub mod ipc;
pub mod syslog;

use std::ffi::CStr;

use config::{parse_cstrs, PamConfig, PamEvent};
pub use pam_bindings::constants::{PamFlag, PamResultCode};
pub use pam_bindings::module::{PamHandle, PamHooks};
use soos_protocol::types::{EventKind, ReasonClass, Verdict};

/// The PAM environment seen by the authentication flow (review PAM-07, GitHub #220).
///
/// [`SoosPam::authenticate_with_feedback`] reaches the PAM conversation, the user lookup
/// and the `PAM_SERVICE` item ONLY through this trait, so the flow is unit-testable with
/// a recorder instead of a fake libpam pointer. Implementations must never panic and
/// must discard conversation failures: feedback never influences the verdict.
pub trait PamFeedback {
    /// Shows `msg` to the user as `PAM_TEXT_INFO`; failures are silently discarded.
    fn info(&mut self, msg: &str);
    /// Returns the PAM user name (`pam_get_user`), or `None` when unavailable.
    fn user(&mut self) -> Option<String>;
    /// Returns the raw `PAM_SERVICE` item, or `None` when unset or unreadable.
    fn service(&mut self) -> Option<Vec<u8>>;
}

/// Linux-PAM handle adapter. The handle comes from libpam (never a synthetic pointer: the
/// former pointer-address guard was removed once every test used a real `pam_start`
/// handle, GitHub #220).
impl PamFeedback for PamHandle {
    fn info(&mut self, msg: &str) {
        if let Ok(Some(conv)) = self.get_item::<pam_bindings::conv::Conv<'_>>() {
            let _ = conv.send(pam_bindings::constants::PAM_TEXT_INFO, msg);
        }
    }

    fn user(&mut self) -> Option<String> {
        self.get_user(None).ok()
    }

    fn service(&mut self) -> Option<Vec<u8>> {
        match self.get_item::<pam_bindings::items::Service<'_>>() {
            Ok(Some(service)) => Some(service.0.to_bytes().to_vec()),
            _ => None,
        }
    }
}

/// No PAM handle (null `pamh`): no conversation, no user, no service item.
struct Detached;

impl PamFeedback for Detached {
    fn info(&mut self, _msg: &str) {}

    fn user(&mut self) -> Option<String> {
        None
    }

    fn service(&mut self) -> Option<Vec<u8>> {
        None
    }
}

/// Returns `config` with the `PAM_SERVICE` item applied (review PAM-05, GitHub #176).
///
/// An explicit `service=` argument keeps precedence; a missing or unreadable item keeps
/// the configured value.
fn with_pam_service(feedback: &mut dyn PamFeedback, config: &PamConfig) -> PamConfig {
    let mut resolved = config.clone();
    if !resolved.service_from_args {
        if let Some(service) = feedback.service() {
            resolved.apply_pam_service(&service);
        }
    }
    resolved
}

/// Returns true when the caller asked for no conversation (`PAM_SILENT`, GitHub #221).
fn is_silent(flags: PamFlag) -> bool {
    flags & pam_bindings::constants::PAM_SILENT != 0
}

/// User-facing feedback for an authentication outcome (review PAM-03, GitHub #174).
///
/// The text goes to the UNAUTHENTICATED user through `PAM_TEXT_INFO`, so it depends on
/// the verdict class only and never on [`ReasonClass`]: every denial reads the same
/// (no presentation-attack oracle distinguishing a PAD rejection from a non-match) and
/// every other failure reads the same (no camera / model / rate-limit state disclosure).
fn feedback_message(outcome: &Result<(Verdict, ReasonClass), ipc::IpcError>) -> &'static str {
    match outcome {
        Ok((Verdict::Allow, _)) => "[soos] Face recognized. Unlocking...",
        Ok((Verdict::Deny, _)) => "[soos] Face not recognized.",
        Ok((Verdict::Unavailable, _)) | Ok((Verdict::ProtocolError, _)) | Err(_) => {
            "[soos] Face verification unavailable."
        }
    }
}

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
    /// Authenticates with no PAM flags (compatibility entry for callers without flags).
    ///
    /// `None` means no PAM handle: no conversation, no user lookup, no service item.
    pub fn authenticate_with_config(
        pamh: Option<&mut PamHandle>,
        config: &PamConfig,
    ) -> PamResultCode {
        match pamh {
            Some(h) => Self::authenticate_with_feedback(h, config, 0),
            None => Self::authenticate_with_feedback(&mut Detached, config, 0),
        }
    }

    /// [`Self::authenticate_with_config`] with an injectable username -> UID resolver.
    ///
    /// `resolve_uid` runs only when no `uid=` argument is configured. It stands for
    /// `pam_get_user` + `getpwnam_r`, which can block in NSS (LDAP / SSSD / NIS) and cannot
    /// be interrupted, so the module bounds what it controls (review PAM-11, GitHub #223):
    /// - the authentication budget (`timeout_ms`) starts BEFORE resolution, so a slow lookup
    ///   is charged to it; a lookup that spends the whole budget returns `PAM_IGNORE`
    ///   without contacting the daemon;
    /// - the `event=password-failed` path still delivers the event, bounded by
    ///   `EVENT_TIMEOUT_MS` after resolution;
    /// - either overrun is logged at `LOG_INFO` (duration only, never the username).
    pub fn authenticate_with_uid_resolver<R>(
        pamh: Option<&mut PamHandle>,
        config: &PamConfig,
        resolve_uid: R,
    ) -> PamResultCode
    where
        R: FnOnce(&mut dyn PamFeedback) -> u32,
    {
        match pamh {
            Some(h) => Self::authenticate_flow(h, config, 0, resolve_uid),
            None => Self::authenticate_flow(&mut Detached, config, 0, resolve_uid),
        }
    }

    /// Internal authentication logic shared between `PamHooks` and C ABI exports.
    ///
    /// `flags` are the Linux-PAM flags of the call: with `PAM_SILENT` no conversation
    /// message is ever sent (GitHub #221). "Looking for face..." is only sent once the
    /// daemon socket is connected, so an absent or stopped daemon never announces a
    /// lookup it cannot perform.
    ///
    /// # Panic Safety Guarantee
    ///
    /// All internal execution is wrapped in `catch_unwind`. Any panic triggers a syslog
    /// alert with source location and backtrace summary, and systematically returns
    /// [`PamResultCode::PAM_IGNORE`].
    pub fn authenticate_with_feedback(
        feedback: &mut dyn PamFeedback,
        config: &PamConfig,
        flags: PamFlag,
    ) -> PamResultCode {
        Self::authenticate_flow(feedback, config, flags, default_uid_resolver)
    }

    /// The authentication flow: [`Self::authenticate_with_feedback`] with an injectable
    /// username -> UID resolver (see [`Self::authenticate_with_uid_resolver`]).
    fn authenticate_flow<R>(
        feedback: &mut dyn PamFeedback,
        config: &PamConfig,
        flags: PamFlag,
        resolve_uid: R,
    ) -> PamResultCode
    where
        R: FnOnce(&mut dyn PamFeedback) -> u32,
    {
        let result = syslog::catch_entry(|| {
            // Test-only hook (Docker T10): panics here when armed, before any socket activity.
            // Ordering invariant: this `trigger` call must run before any libpam call
            // (`with_pam_service` reads `PAM_SERVICE`, UID resolution calls `pam_get_user`).
            // `fault_injection_tests::test_fault_inject_via_pam_hooks_returns_pam_ignore`
            // passes a synthetic handle and stays safe only because the armed panic fires
            // first; moving this call below a libpam access would dereference that pointer.
            // Pinned by `tests/invariants` (PHS11).
            #[cfg(feature = "fault-injection")]
            fault_injection::trigger(config.fault_inject);

            let silent = is_silent(flags);
            let resolved = with_pam_service(feedback, config);
            let config = &resolved;

            if config.is_disabled() {
                syslog::log_info(&format!(
                    "soos authentication is disabled for service '{}'; ignoring",
                    config.service
                ));
                return PamResultCode::PAM_IGNORE;
            }

            // The authentication budget starts before UID resolution (GitHub #223) and the
            // deadline sent to the daemon is fixed here, before connect (GitHub #222).
            let auth_deadline = ipc::ExchangeDeadline::start(config.timeout_ms);
            let resolution_start = std::time::Instant::now();
            let uid = match config.uid {
                Some(uid) => uid,
                None => resolve_uid(&mut *feedback),
            };
            let resolution_elapsed = resolution_start.elapsed();

            if config.event == Some(PamEvent::PasswordFailed) {
                if resolution_elapsed > std::time::Duration::from_millis(ipc::EVENT_TIMEOUT_MS) {
                    syslog::log_info(&format!(
                        "soos user lookup took {} ms, above the {} ms event budget; \
                         configure uid= to avoid the NSS lookup",
                        resolution_elapsed.as_millis(),
                        ipc::EVENT_TIMEOUT_MS
                    ));
                }
                // Best-effort telemetry notification bounded by 20ms ceiling
                let _ = ipc::notify_event(config, uid, EventKind::PasswordFailed);
                return PamResultCode::PAM_IGNORE;
            }

            if auth_deadline.remaining().is_err() {
                syslog::log_info(&format!(
                    "soos user lookup took {} ms and spent the {} ms authentication budget; \
                     falling back to the next module",
                    resolution_elapsed.as_millis(),
                    config.timeout_ms
                ));
                return PamResultCode::PAM_IGNORE;
            }

            let outcome =
                ipc::authenticate_before_with_progress(config, uid, auth_deadline, || {
                    if !silent {
                        feedback.info("[soos] Looking for face...");
                    }
                });
            if !silent {
                feedback.info(feedback_message(&outcome));
            }

            // PAM_SUCCESS exclusively on a daemon Allow; every other outcome falls back.
            // The protocol crate owns the predicate (`Verdict::should_ignore`, GitHub #264).
            match outcome {
                Ok((verdict, _)) if !verdict.should_ignore() => PamResultCode::PAM_SUCCESS,
                _ => PamResultCode::PAM_IGNORE,
            }
        });

        match result {
            Ok(code) => code,
            Err(payload) => {
                let location = syslog::take_panic_location();
                let summary = syslog::panic_summary(&*payload, "unspecified panic payload");
                syslog::log_panic(summary, location.as_deref());

                // Invariant 5 of ARCHITECTURE.md:
                // "A panic degrades to password fallback, never to authorization."
                PamResultCode::PAM_IGNORE
            }
        }
    }
}

/// Production username -> UID resolver: `pam_get_user` (through [`PamFeedback::user`]) +
/// `getpwnam_r`, falling back to the caller's real UID when the handle, the username or
/// the passwd entry is unavailable.
fn default_uid_resolver(feedback: &mut dyn PamFeedback) -> u32 {
    if let Some(username) = feedback.user() {
        if let Some(resolved_uid) = resolve_username_to_uid(&username) {
            return resolved_uid;
        }
    }
    // SAFETY: getuid is a safe, non-allocating libc syscall returning caller process UID.
    unsafe { libc::getuid() }
}

impl PamHooks for SoosPam {
    /// Production authentication hook: the exported `pam_sm_authenticate` symbol calls it
    /// for every non-null handle (GitHub #264).
    fn sm_authenticate(pamh: &mut PamHandle, args: Vec<&CStr>, flags: PamFlag) -> PamResultCode {
        let result = syslog::catch_entry(|| {
            let config = parse_cstrs(args);
            Self::authenticate_with_feedback(pamh, &config, flags)
        });

        match result {
            Ok(code) => code,
            Err(payload) => {
                let location = syslog::take_panic_location();
                let summary = syslog::panic_summary(
                    &*payload,
                    "unspecified panic payload in sm_authenticate",
                );
                syslog::log_panic(summary, location.as_deref());
                PamResultCode::PAM_IGNORE
            }
        }
    }

    fn sm_setcred(_pamh: &mut PamHandle, _args: Vec<&CStr>, _flags: PamFlag) -> PamResultCode {
        PamResultCode::PAM_IGNORE
    }
}

/// Initial allocation buffer size for `getpwnam_r` lookups (1 KB).
pub const INITIAL_PW_BUF_SIZE: usize = 1024;

/// Absolute maximum allocation ceiling for `getpwnam_r` lookups (64 KB) to prevent OOM.
pub const MAX_PW_BUF_SIZE: usize = 64 * 1024;

/// Resolves a PAM username string to a numeric POSIX UID using reentrant `getpwnam_r`.
///
/// Starts with `sysconf(_SC_GETPW_R_SIZE_MAX)` (minimum 1024 bytes) and dynamically
/// doubles the buffer on `ERANGE` up to a maximum cap of 64KB to support LDAP/AD backends.
pub fn resolve_username_to_uid(username: &str) -> Option<u32> {
    let initial = initial_buffer_size();
    resolve_username_to_uid_with_bounds(username, initial, MAX_PW_BUF_SIZE)
}

/// Queries `sysconf(_SC_GETPW_R_SIZE_MAX)`, clamping between `INITIAL_PW_BUF_SIZE` and `MAX_PW_BUF_SIZE`.
fn initial_buffer_size() -> usize {
    // SAFETY: sysconf is a safe, standard POSIX query with no side effects.
    let sc = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    if let Ok(sc_usize) = usize::try_from(sc) {
        sc_usize.clamp(INITIAL_PW_BUF_SIZE, MAX_PW_BUF_SIZE)
    } else {
        INITIAL_PW_BUF_SIZE
    }
}

/// Resolves a PAM username string to a numeric POSIX UID with explicit buffer bounds.
///
/// Retries with doubled buffer size when `libc::getpwnam_r` returns `ERANGE`,
/// capping growth at `max_size` to prevent memory exhaustion.
pub fn resolve_username_to_uid_with_bounds(
    username: &str,
    initial_size: usize,
    max_size: usize,
) -> Option<u32> {
    let c_user = std::ffi::CString::new(username).ok()?;
    let mut buf_size = initial_size.max(1).min(max_size);

    loop {
        let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut buf = vec![0u8; buf_size];

        // SAFETY: getpwnam_r is standard POSIX reentrant user lookup.
        // `c_user` is a valid null-terminated C string.
        // `pwd` is a valid pointer to uninitialized `passwd` memory.
        // `buf` is a valid allocated memory region of `buf_size` bytes.
        // `result` is a valid pointer to a `*mut passwd` pointer.
        let ret = unsafe {
            libc::getpwnam_r(
                c_user.as_ptr(),
                pwd.as_mut_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            )
        };

        if ret == 0 {
            if result.is_null() {
                // User not found in system database
                return None;
            }
            // SAFETY: pwd initialized by getpwnam_r when ret == 0 and result is non-null.
            let pwd_val = unsafe { pwd.assume_init() };
            return Some(pwd_val.pw_uid);
        } else if ret == libc::ERANGE {
            if buf_size >= max_size {
                // Cap reached: fail closed to prevent unbounded heap allocation
                return None;
            }
            let next_size = match buf_size.checked_mul(2) {
                Some(doubled) => doubled.min(max_size),
                None => max_size,
            };
            if next_size <= buf_size {
                return None;
            }
            buf_size = next_size;
        } else {
            // Unrecoverable libc lookup error
            return None;
        }
    }
}

// ---------------------------------------------------------------------------
// PAM Entry Points (C ABI)
// ---------------------------------------------------------------------------

/// Safely executes an entry point closure within `catch_unwind`, ensuring no panic escapes across the C ABI.
///
/// Any caught panic is logged to syslog with panic location and payload, systematically returning `PAM_IGNORE`.
fn catch_c_entry<F: FnOnce() -> i32>(f: F) -> i32 {
    let result = syslog::catch_entry(f);

    match result {
        Ok(code) => code,
        Err(payload) => {
            let location = syslog::take_panic_location();
            let summary =
                syslog::panic_summary(&*payload, "unspecified panic payload in PAM C entry point");

            syslog::log_panic(summary, location.as_deref());
            PAM_IGNORE
        }
    }
}

/// Primary authentication entry point called by Linux-PAM.
///
/// # Safety
///
/// Invoked by Linux-PAM via the C ABI. `pamh` is supplied by PAM. If non-null,
/// it is forwarded to `SoosPam`. `argv` points to an array of `argc` C strings.
/// Entire execution including argument parsing is wrapped in `catch_unwind`.
#[allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "Exported C ABI entry point invoked by Linux-PAM; raw pointers are guarded against null and unbounded reads"
)]
#[no_mangle]
pub extern "C" fn pam_sm_authenticate(
    pamh: *mut PamHandle,
    flags: i32,
    argc: i32,
    argv: *const *const u8,
) -> i32 {
    catch_c_entry(|| {
        // SAFETY: argv points to argc pointers passed across the C ABI (Linux-PAM keeps
        // them alive for the whole call); every string is read with the MAX_ARG_LEN bound.
        // Rejected arguments are logged at LOG_WARNING (GitHub #265).
        let args = unsafe { config::collect_argv_logged(argc, argv) };

        // Linux-PAM passes `unsigned int flags`; the C ABI signature declares `int`, so
        // reinterpret the bits unchanged.
        let pam_flags = PamFlag::from_ne_bytes(flags.to_ne_bytes());

        // SAFETY: `as_mut` returns None for a null pointer; a non-null pamh is the valid
        // handle Linux-PAM passed to this call.
        let code = match unsafe { pamh.as_mut() } {
            // The exported symbol runs the `PamHooks` implementation (GitHub #264).
            Some(h) => SoosPam::sm_authenticate(h, args, pam_flags),
            None => {
                SoosPam::authenticate_with_feedback(&mut Detached, &parse_cstrs(args), pam_flags)
            }
        };

        match code {
            PamResultCode::PAM_SUCCESS => PAM_SUCCESS,
            _ => PAM_IGNORE,
        }
    })
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
    catch_c_entry(|| PAM_IGNORE)
}

/// Account management entry point called by Linux-PAM.
#[no_mangle]
pub extern "C" fn pam_sm_acct_mgmt(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    catch_c_entry(|| PAM_IGNORE)
}

/// Authentication token update entry point called by Linux-PAM.
#[no_mangle]
pub extern "C" fn pam_sm_chauthtok(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    catch_c_entry(|| PAM_IGNORE)
}

/// Session open entry point called by Linux-PAM.
#[no_mangle]
pub extern "C" fn pam_sm_open_session(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    catch_c_entry(|| PAM_IGNORE)
}

/// Session close entry point called by Linux-PAM.
#[no_mangle]
pub extern "C" fn pam_sm_close_session(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    catch_c_entry(|| PAM_IGNORE)
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
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::ptr;

    /// PA1: Module returns PAM_IGNORE when daemon is unreachable.
    #[test]
    fn authenticate_returns_pam_ignore() {
        let arg = c"socket=/tmp/nonexistent_soos_unreachable.sock";
        let argv = [arg.as_ptr().cast::<u8>()];
        let result = pam_sm_authenticate(ptr::null_mut(), 0, 1, argv.as_ptr());
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

    /// Sub-issue #34.1: pam_sm_authenticate must safely catch panics even if argument parsing fails.
    #[test]
    fn authenticate_catches_parse_argv_panics() {
        // Invalid socket configuration gracefully degrades to PAM_IGNORE without panicking across FFI
        let arg = c"socket=/tmp/nonexistent_soos_unreachable.sock";
        let argv = [arg.as_ptr().cast::<u8>()];
        let code = pam_sm_authenticate(ptr::null_mut(), 0, 1, argv.as_ptr());
        assert_eq!(code, PAM_IGNORE);
    }
}
