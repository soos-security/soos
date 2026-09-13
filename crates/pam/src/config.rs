//! PAM module argument parsing and runtime configuration.

use std::path::PathBuf;

/// Default socket path for the privileged background daemon.
pub const DEFAULT_SOCKET_PATH: &str = "/run/soos/daemon.sock";

/// Default total execution timeout budget (milliseconds).
pub const DEFAULT_TIMEOUT_MS: u64 = 250;

/// Minimum allowable timeout budget (milliseconds).
pub const MIN_TIMEOUT_MS: u64 = 10;

/// Maximum allowable timeout budget (milliseconds).
pub const MAX_TIMEOUT_MS: u64 = 5000;

/// Default service identifier if not provided.
pub const DEFAULT_SERVICE: &str = "pam_soos";

/// Maximum length of a PAM argument string (bytes) to prevent unbounded reads.
const MAX_ARG_LEN: usize = 256;

/// Maximum number of arguments parsed from PAM.
const MAX_ARGC: usize = 64;

/// Optional special operational mode specified via PAM arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PamEvent {
    /// Password authentication failed; notify daemon for intrusion telemetry.
    PasswordFailed,
}

/// Parsed configuration for the PAM module execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PamConfig {
    /// Latency budget in milliseconds.
    pub timeout_ms: u64,
    /// Special event mode (e.g. password-failed).
    pub event: Option<PamEvent>,
    /// Path to the daemon Unix domain socket.
    pub socket_path: PathBuf,
    /// PAM service name.
    pub service: String,
}

impl Default for PamConfig {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            event: None,
            socket_path: PathBuf::from(DEFAULT_SOCKET_PATH),
            service: DEFAULT_SERVICE.to_string(),
        }
    }
}

/// Parses PAM `argc` and `argv` into a [`PamConfig`].
///
/// # Safety
///
/// If `argv` is non-null and `argc > 0`, `argv` must point to an array containing at least
/// `argc` pointers. Individual string pointers must either be null or point to null-terminated
/// C strings or valid memory of at least [`MAX_ARG_LEN`] bytes.
pub unsafe fn parse_argv(argc: i32, argv: *const *const u8) -> PamConfig {
    let mut config = PamConfig::default();

    if argv.is_null() || argc <= 0 {
        return config;
    }

    let count = match usize::try_from(argc) {
        Ok(c) => c.min(MAX_ARGC),
        Err(_) => return config,
    };

    for i in 0..count {
        // SAFETY: `i` is bounded by `count <= MAX_ARGC` and `argv` is non-null.
        let arg_ptr_ptr = unsafe { argv.add(i) };
        if arg_ptr_ptr.is_null() {
            continue;
        }

        // SAFETY: `arg_ptr_ptr` was verified non-null.
        let arg_ptr = unsafe { *arg_ptr_ptr };
        if arg_ptr.is_null() {
            continue;
        }

        let arg_str = match extract_bounded_str(arg_ptr) {
            Some(s) => s,
            None => continue,
        };

        if let Some(val) = arg_str.strip_prefix("timeout_ms=") {
            if let Ok(parsed) = val.trim().parse::<u64>() {
                config.timeout_ms = parsed.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
            }
        } else if arg_str == "event=password-failed" {
            config.event = Some(PamEvent::PasswordFailed);
        } else if let Some(val) = arg_str.strip_prefix("socket_path=") {
            let path_str = val.trim();
            if !path_str.is_empty() {
                config.socket_path = PathBuf::from(path_str);
            }
        } else if let Some(val) = arg_str.strip_prefix("service=") {
            let s = val.trim();
            if !s.is_empty() {
                let bounded_len = s.len().min(soos_protocol::MAX_SERVICE_LEN);
                if let Some(sub) = s.get(..bounded_len) {
                    config.service = sub.to_string();
                }
            }
        }
    }

    config
}

/// Extracts a string from a null-terminated C pointer with a strict byte limit.
fn extract_bounded_str<'a>(ptr: *const u8) -> Option<&'a str> {
    let mut len = 0usize;
    while len < MAX_ARG_LEN {
        // SAFETY: `len < MAX_ARG_LEN` bounds the pointer read within a known small region.
        let byte = unsafe { *ptr.add(len) };
        if byte == 0 {
            break;
        }
        len = len.saturating_add(1);
    }

    if len == MAX_ARG_LEN {
        // Unterminated within bounds
        return None;
    }

    // SAFETY: `ptr` points to at least `len` valid initialized bytes ending before a null byte.
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(slice).ok()
}
