//! Syslog logging and panic interceptor for pam_soos.
//!
//! Provides bounded, secret-free panic logging to the system authentication log
//! (`LOG_AUTHPRIV | LOG_ERR`) and captures panic source locations without stderr pollution.

use std::cell::{Cell, RefCell};
use std::ffi::CString;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Once;

thread_local! {
    static LAST_PANIC_LOC: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Number of soos entry points ([`catch_entry`]) active on this thread.
    static ENTRY_DEPTH: Cell<u32> = const { Cell::new(0) };
}

static INIT_HOOK: Once = Once::new();

/// Installs the pam_soos panic hook once per loaded image, chained to the prior hook.
///
/// The hook only acts on panics raised while a soos entry point runs on the current
/// thread ([`catch_entry`]): it records the file/line/column location for the syslog
/// line and prints nothing, so a display manager's stderr stays clean. Every other panic
/// is forwarded to the hook that was installed before, so the module never takes the
/// process panic output over (review PAM-09, GitHub #263). The hook is installed once
/// and never swapped per call, which is race-free when several threads run PAM
/// concurrently. `pam_soos.so` statically links its own libstd, so the hook lives in the
/// module's own image and nothing outside it refers to it after `dlclose`.
pub fn init_panic_hook() {
    INIT_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if in_entry_point() {
                if let Some(loc) = info.location() {
                    let location_str = format!("{}:{}:{}", loc.file(), loc.line(), loc.column());
                    let _ = LAST_PANIC_LOC.try_with(|cell| {
                        if let Ok(mut slot) = cell.try_borrow_mut() {
                            *slot = Some(location_str);
                        }
                    });
                }
            } else {
                previous(info);
            }
        }));
    });
}

/// True while a soos entry point runs on the current thread.
fn in_entry_point() -> bool {
    ENTRY_DEPTH
        .try_with(|depth| depth.get() > 0)
        .unwrap_or(false)
}

/// Marks the current thread as running a soos entry point until dropped.
struct EntryGuard;

impl EntryGuard {
    fn enter() -> Self {
        let _ = ENTRY_DEPTH.try_with(|depth| depth.set(depth.get().saturating_add(1)));
        Self
    }
}

impl Drop for EntryGuard {
    fn drop(&mut self) {
        let _ = ENTRY_DEPTH.try_with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Runs `f` as a soos entry point: installs the chained hook if needed and catches any
/// panic, whose location is then available through [`take_panic_location`].
///
/// Nested calls are supported; the entry-point marker is restored even when `f` panics.
pub fn catch_entry<F: FnOnce() -> R, R>(f: F) -> std::thread::Result<R> {
    init_panic_hook();
    let _guard = EntryGuard::enter();
    catch_unwind(AssertUnwindSafe(f))
}

/// Returns the panic payload as text, or `fallback` for a non-string payload.
#[must_use]
pub fn panic_summary<'a>(payload: &'a (dyn std::any::Any + Send), fallback: &'a str) -> &'a str {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        fallback
    }
}

/// Retrieves and clears the most recently recorded panic location for the current thread.
#[must_use]
pub fn take_panic_location() -> Option<String> {
    LAST_PANIC_LOC.with(|cell| cell.borrow_mut().take())
}

/// Formats a panic summary and optional source location into a bounded, sanitized log line.
///
/// Ensures internal nul characters are replaced so the message can be safely passed to C `syslog`.
#[must_use]
pub fn format_panic_message(summary: &str, location: Option<&str>) -> String {
    let sanitized_summary: String = summary
        .chars()
        .map(|c| if c == '\0' { ' ' } else { c })
        .collect();

    if let Some(loc) = location {
        let sanitized_loc: String = loc
            .chars()
            .map(|c| if c == '\0' { ' ' } else { c })
            .collect();
        format!("soos-pam: authentication panic caught at {sanitized_loc}: {sanitized_summary}")
    } else {
        format!("soos-pam: authentication panic caught: {sanitized_summary}")
    }
}

/// Dispatches a panic message to the system authentication log facility (`LOG_AUTHPRIV`).
///
/// # Security Invariants
///
/// - NEVER logs passwords, usernames, biometric vectors, or IPC request payloads.
/// - Uses fixed format string `"%s"` to prevent format string injection vulnerabilities.
pub fn log_panic(summary: &str, location: Option<&str>) {
    let message = format_panic_message(summary, location);
    let c_str = match CString::new(message) {
        Ok(s) => s,
        Err(_) => {
            // Fallback to static constant CString if sanitization somehow left an embedded nul
            match CString::new("soos-pam: authentication panic caught") {
                Ok(fallback) => fallback,
                Err(_) => return,
            }
        }
    };

    let fmt = b"%s\0";
    // SAFETY: fmt and c_str are valid null-terminated C strings. LOG_AUTHPRIV | LOG_ERR are standard syslog flags.
    unsafe {
        libc::syslog(
            libc::LOG_AUTHPRIV | libc::LOG_ERR,
            fmt.as_ptr().cast(),
            c_str.as_ptr(),
        );
    }
}

/// Dispatches a warning to the system authentication log (`LOG_AUTHPRIV | LOG_WARNING`).
///
/// Used for rejected PAM arguments (GitHub #265); callers pass key names only, never a
/// rejected value.
pub fn log_warning(message: &str) {
    log_with_priority(libc::LOG_AUTHPRIV | libc::LOG_WARNING, message);
}

/// Dispatches an informational message to the system authentication log facility (`LOG_AUTHPRIV | LOG_INFO`).
pub fn log_info(message: &str) {
    log_with_priority(libc::LOG_AUTHPRIV | libc::LOG_INFO, message);
}

/// Sends `soos-pam: <message>` (NUL bytes replaced) to syslog at `priority`.
fn log_with_priority(priority: libc::c_int, message: &str) {
    let sanitized: String = message
        .chars()
        .map(|c| if c == '\0' { ' ' } else { c })
        .collect();
    let c_str = match CString::new(format!("soos-pam: {sanitized}")) {
        Ok(s) => s,
        Err(_) => return,
    };

    let fmt = b"%s\0";
    // SAFETY: fmt and c_str are valid null-terminated C strings; `priority` combines
    // standard syslog facility and level flags.
    unsafe {
        libc::syslog(priority, fmt.as_ptr().cast(), c_str.as_ptr());
    }
}
