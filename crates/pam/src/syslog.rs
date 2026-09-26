//! Syslog logging and panic interceptor for pam_soos.
//!
//! Provides bounded, secret-free panic logging to the system authentication log
//! (`LOG_AUTHPRIV | LOG_ERR`) and captures panic source locations without stderr pollution.

use std::cell::RefCell;
use std::ffi::CString;
use std::sync::Once;

thread_local! {
    static LAST_PANIC_LOC: RefCell<Option<String>> = const { RefCell::new(None) };
}

static INIT_HOOK: Once = Once::new();

/// Initializes the process-local panic hook if not already registered.
///
/// The registered hook intercepts panics originating in `pam_soos`, records their
/// file/line/column location in thread-local storage, and suppresses stderr printing
/// to avoid corrupting graphical display manager output streams.
pub fn init_panic_hook() {
    INIT_HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            if let Some(loc) = info.location() {
                let location_str = format!("{}:{}:{}", loc.file(), loc.line(), loc.column());
                LAST_PANIC_LOC.with(|cell| {
                    *cell.borrow_mut() = Some(location_str);
                });
            }
        }));
    });
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

/// Dispatches an informational message to the system authentication log facility (`LOG_AUTHPRIV | LOG_INFO`).
pub fn log_info(message: &str) {
    let sanitized: String = message
        .chars()
        .map(|c| if c == '\0' { ' ' } else { c })
        .collect();
    let c_str = match CString::new(format!("soos-pam: {sanitized}")) {
        Ok(s) => s,
        Err(_) => return,
    };

    let fmt = b"%s\0";
    // SAFETY: fmt and c_str are valid null-terminated C strings. LOG_AUTHPRIV | LOG_INFO are standard syslog flags.
    unsafe {
        libc::syslog(
            libc::LOG_AUTHPRIV | libc::LOG_INFO,
            fmt.as_ptr().cast(),
            c_str.as_ptr(),
        );
    }
}
