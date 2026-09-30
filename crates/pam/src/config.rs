//! PAM module argument parsing and runtime configuration.

use std::path::PathBuf;

/// Default socket path for the privileged background daemon.
pub const DEFAULT_SOCKET_PATH: &str = "/run/soos/daemon.sock";

/// Default total execution timeout budget (milliseconds).
pub const DEFAULT_TIMEOUT_MS: u64 = 1000;

/// Minimum allowable timeout budget (milliseconds).
pub const MIN_TIMEOUT_MS: u64 = 10;

/// Maximum allowable timeout budget (milliseconds).
pub const MAX_TIMEOUT_MS: u64 = 5000;

/// Default service identifier if not provided.
pub const DEFAULT_SERVICE: &str = "pam_soos";

/// Default directory holding the administrator disable flag files
/// (`disabled`, `gdm.disable`, `<service>.disable`).
pub const DEFAULT_FLAG_DIR: &str = "/etc/soos";

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

/// Test-only fault to inject inside the `catch_unwind` region (review finding PAM-01).
///
/// The variants are always defined so that [`PamConfig`] has the same shape in every build,
/// but the PAM argument `fault_inject=<mode>` is parsed **only** when the crate is compiled
/// with the opt-in `fault-injection` feature; production builds always keep `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultInject {
    /// Explicit panic with a fixed, secret-free payload.
    Panic,
    /// Arithmetic overflow trapped by `overflow-checks = true` (the class of panic the
    /// release profile deliberately enables).
    Overflow,
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
    /// True when `service` was set by an explicit `service=` PAM argument. When false,
    /// [`PamConfig::apply_pam_service`] replaces it with the `PAM_SERVICE` item.
    pub service_from_args: bool,
    /// Directory holding the administrator disable flag files ([`DEFAULT_FLAG_DIR`]).
    /// Not settable from PAM arguments.
    pub flag_dir: PathBuf,
    /// Optional target UID override specified in PAM arguments.
    pub uid: Option<u32>,
    /// Explicitly disabled via PAM argument.
    pub disabled: bool,
    /// Custom disable flag file path (defaults to checking standard system flags).
    pub disable_file: Option<PathBuf>,
    /// Armed fault-injection mode; only ever `Some` when built with the `fault-injection`
    /// feature (Docker matrix T10). Always `None` in production builds.
    pub fault_inject: Option<FaultInject>,
}

impl PamConfig {
    /// Adopts the `PAM_SERVICE` item as the service name unless `service=` was given.
    ///
    /// The installed PAM lines (`soos-admin gdm enable`, packaging snippets) pass no
    /// `service=` argument, so without this the module always reported `pam_soos` and the
    /// GDM flag never applied (review PAM-05, GitHub #176). The value must be UTF-8, is
    /// trimmed and bounded to [`soos_protocol::MAX_SERVICE_LEN`] bytes on a char boundary;
    /// anything else keeps the current value.
    pub fn apply_pam_service(&mut self, pam_service: &[u8]) {
        if self.service_from_args {
            return;
        }
        let Ok(name) = std::str::from_utf8(pam_service) else {
            return;
        };
        let name = name.trim();
        let mut end = name.len().min(soos_protocol::MAX_SERVICE_LEN);
        while end > 0 && !name.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        if let Some(bounded) = name.get(..end) {
            if !bounded.is_empty() {
                self.service = bounded.to_string();
            }
        }
    }

    /// Returns true if PAM authentication is explicitly disabled for this service or globally.
    ///
    /// Checked flags (all under [`PamConfig::flag_dir`], `/etc/soos` by default):
    /// `disabled` (global), `gdm.disable` (any service whose name contains `gdm`) and
    /// `<service>.disable` (only for service names made of `[A-Za-z0-9._-]` that do not
    /// start with `.`, so a crafted name can never reach outside the flag directory).
    pub fn is_disabled(&self) -> bool {
        if self.disabled {
            return true;
        }

        // Check global disable flag
        if self.flag_dir.join("disabled").exists() {
            return true;
        }

        // Check custom disable file if provided
        if let Some(ref path) = self.disable_file {
            if path.exists() {
                return true;
            }
        }

        // Check service-specific disable flag for GDM (gdm-password, gdm-fingerprint, ...)
        if self.service.contains("gdm") && self.flag_dir.join("gdm.disable").exists() {
            return true;
        }

        // Check the generic per-service flag
        if is_flag_safe_service_name(&self.service)
            && self
                .flag_dir
                .join(format!("{}.disable", self.service))
                .exists()
        {
            return true;
        }

        false
    }
}

/// True when `name` can be used verbatim as a flag file stem inside the flag directory.
fn is_flag_safe_service_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl Default for PamConfig {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            event: None,
            socket_path: PathBuf::from(DEFAULT_SOCKET_PATH),
            service: DEFAULT_SERVICE.to_string(),
            service_from_args: false,
            flag_dir: PathBuf::from(DEFAULT_FLAG_DIR),
            uid: None,
            disabled: false,
            disable_file: None,
            fault_inject: None,
        }
    }
}

/// Longest key name echoed in an [`ConfigWarning::UnknownArgument`] warning (bytes).
const MAX_ECHOED_KEY_LEN: usize = 32;

/// A rejected or adjusted PAM argument (review PAM-17, GitHub #265).
///
/// Each warning is logged once at `LOG_AUTHPRIV | LOG_WARNING` by [`parse_argv`],
/// [`parse_cstrs`] and [`collect_argv_logged`]. A warning carries key names, argument
/// positions and numeric bounds only, never a rejected value: an unknown argument echoes
/// its key only when that key is short and made of `[A-Za-z0-9_-]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigWarning {
    /// The argument at `index` (position in `argv`) has `MAX_ARG_LEN` (256) bytes or more.
    ArgumentTooLong { index: usize },
    /// The `argv` entry at `index` is a null pointer.
    NullArgument { index: usize },
    /// Arguments beyond the first `MAX_ARGC` (64) were ignored.
    TooManyArguments { ignored: usize },
    /// The argument at `index` (position in the parsed list) is not valid UTF-8.
    NotUtf8 { index: usize },
    /// `key=` has a value that cannot be parsed; the default is kept.
    InvalidValue { key: &'static str },
    /// `key=` has an empty value; the default is kept.
    EmptyValue { key: &'static str },
    /// `timeout_ms=` was outside `MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS` and was clamped.
    TimeoutClamped { requested: u64, applied: u64 },
    /// `service=` was longer than `MAX_SERVICE_LEN` bytes and was cut on a character boundary.
    ServiceTruncated,
    /// Unrecognized argument; `key` is `None` when the key is not safe to echo.
    UnknownArgument { key: Option<String> },
}

impl core::fmt::Display for ConfigWarning {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ArgumentTooLong { index } => write!(
                f,
                "PAM argument #{index} is {MAX_ARG_LEN} bytes or longer and was ignored"
            ),
            Self::NullArgument { index } => {
                write!(f, "PAM argument #{index} is a null pointer and was ignored")
            }
            Self::TooManyArguments { ignored } => write!(
                f,
                "{ignored} PAM argument(s) beyond the first {MAX_ARGC} were ignored"
            ),
            Self::NotUtf8 { index } => {
                write!(f, "PAM argument #{index} is not valid UTF-8 and was ignored")
            }
            Self::InvalidValue { key } => write!(
                f,
                "invalid value for PAM argument '{key}'; the default was kept"
            ),
            Self::EmptyValue { key } => write!(
                f,
                "empty value for PAM argument '{key}'; the default was kept"
            ),
            Self::TimeoutClamped { requested, applied } => write!(
                f,
                "timeout_ms={requested} is outside {MIN_TIMEOUT_MS}..={MAX_TIMEOUT_MS} ms; using {applied} ms"
            ),
            Self::ServiceTruncated => write!(
                f,
                "service= is longer than {} bytes and was truncated",
                soos_protocol::MAX_SERVICE_LEN
            ),
            Self::UnknownArgument { key: Some(key) } => {
                write!(f, "unknown PAM argument '{key}' was ignored")
            }
            Self::UnknownArgument { key: None } => write!(
                f,
                "unknown PAM argument with an unprintable or overlong key was ignored"
            ),
        }
    }
}

/// Logs every warning at `LOG_AUTHPRIV | LOG_WARNING` (one line each).
fn log_warnings(warnings: &[ConfigWarning]) {
    for warning in warnings {
        crate::syslog::log_warning(&format!("configuration: {warning}"));
    }
}

/// Parses an iterator of [`&CStr`] references (e.g. from `pam-bindings`) into a [`PamConfig`].
///
/// Rejected arguments keep the default and are logged ([`ConfigWarning`]).
pub fn parse_cstrs<'a, I>(args: I) -> PamConfig
where
    I: IntoIterator<Item = &'a std::ffi::CStr>,
{
    let (config, warnings) = parse_cstrs_with_warnings(args);
    log_warnings(&warnings);
    config
}

/// [`parse_cstrs`] returning the warnings instead of logging them.
///
/// At most `MAX_ARGC` arguments are parsed and an argument of `MAX_ARG_LEN` bytes or
/// more is ignored, exactly like [`parse_argv`].
pub fn parse_cstrs_with_warnings<'a, I>(args: I) -> (PamConfig, Vec<ConfigWarning>)
where
    I: IntoIterator<Item = &'a std::ffi::CStr>,
{
    let mut config = PamConfig::default();
    let mut warnings = Vec::new();
    let mut iter = args.into_iter();
    for (index, cstr) in iter.by_ref().take(MAX_ARGC).enumerate() {
        if cstr.to_bytes().len() >= MAX_ARG_LEN {
            warnings.push(ConfigWarning::ArgumentTooLong { index });
            continue;
        }
        match cstr.to_str() {
            Ok(s) => apply_arg(&mut config, s.trim(), &mut warnings),
            Err(_) => warnings.push(ConfigWarning::NotUtf8 { index }),
        }
    }
    let ignored = iter.count();
    if ignored > 0 {
        warnings.push(ConfigWarning::TooManyArguments { ignored });
    }
    (config, warnings)
}

/// Returns the key of an unknown argument when it is safe to echo in a log line.
fn echoable_key(arg: &str) -> Option<String> {
    let key = arg.split_once('=').map_or(arg, |(k, _)| k);
    let safe = !key.is_empty()
        && key.len() <= MAX_ECHOED_KEY_LEN
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
    safe.then(|| key.to_owned())
}

/// Returns `s` cut to at most `max` bytes on a UTF-8 character boundary.
fn truncate_on_char_boundary(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while end > 0 && !s.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    s.get(..end).unwrap_or_default()
}

fn apply_arg(config: &mut PamConfig, trimmed: &str, warnings: &mut Vec<ConfigWarning>) {
    if trimmed.is_empty() {
        return;
    }
    if let Some(val) = trimmed.strip_prefix("timeout_ms=") {
        match val.trim().parse::<u64>() {
            Ok(parsed) => {
                let applied = parsed.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
                if applied != parsed {
                    warnings.push(ConfigWarning::TimeoutClamped {
                        requested: parsed,
                        applied,
                    });
                }
                config.timeout_ms = applied;
            }
            Err(_) => warnings.push(ConfigWarning::InvalidValue { key: "timeout_ms" }),
        }
    } else if trimmed == "event=password-failed" {
        config.event = Some(PamEvent::PasswordFailed);
    } else if trimmed.starts_with("event=") {
        warnings.push(ConfigWarning::InvalidValue { key: "event" });
    } else if trimmed == "disabled" {
        config.disabled = true;
    } else if let Some(val) = trimmed.strip_prefix("disable_if_file=") {
        let path_str = val.trim();
        if path_str.is_empty() {
            warnings.push(ConfigWarning::EmptyValue {
                key: "disable_if_file",
            });
        } else {
            config.disable_file = Some(PathBuf::from(path_str));
        }
    } else if let Some((key, val)) = trimmed
        .strip_prefix("socket_path=")
        .map(|v| ("socket_path", v))
        .or_else(|| trimmed.strip_prefix("socket=").map(|v| ("socket", v)))
    {
        let path_str = val.trim();
        if path_str.is_empty() {
            warnings.push(ConfigWarning::EmptyValue { key });
        } else {
            config.socket_path = PathBuf::from(path_str);
        }
    } else if let Some(val) = trimmed.strip_prefix("service=") {
        let s = val.trim();
        if s.is_empty() {
            warnings.push(ConfigWarning::EmptyValue { key: "service" });
        } else {
            // Cut on a character boundary (GitHub #265): `s.get(..64)` returned None when
            // byte 64 fell inside a multibyte character and kept the default service.
            let bounded = truncate_on_char_boundary(s, soos_protocol::MAX_SERVICE_LEN);
            if bounded.len() < s.len() {
                warnings.push(ConfigWarning::ServiceTruncated);
            }
            if !bounded.is_empty() {
                config.service = bounded.to_string();
                config.service_from_args = true;
            }
        }
    } else if let Some(val) = trimmed.strip_prefix("uid=") {
        match val.trim().parse::<u32>() {
            Ok(parsed) => config.uid = Some(parsed),
            Err(_) => warnings.push(ConfigWarning::InvalidValue { key: "uid" }),
        }
    } else if let Some(val) = trimmed.strip_prefix("fault_inject=") {
        apply_fault_inject_arg(config, val.trim());
    } else {
        warnings.push(ConfigWarning::UnknownArgument {
            key: echoable_key(trimmed),
        });
    }
}

/// Parses the test-only `fault_inject=<mode>` argument (feature `fault-injection` only).
#[cfg(feature = "fault-injection")]
fn apply_fault_inject_arg(config: &mut PamConfig, mode: &str) {
    config.fault_inject = match mode {
        "panic" => Some(FaultInject::Panic),
        "overflow" => Some(FaultInject::Overflow),
        _ => None,
    };
}

/// Production builds ignore `fault_inject=<mode>` entirely: the hook cannot be armed.
#[cfg(not(feature = "fault-injection"))]
fn apply_fault_inject_arg(_config: &mut PamConfig, _mode: &str) {}

/// Parses PAM `argc` and `argv` into a [`PamConfig`].
///
/// Rejected arguments keep the default and are logged ([`ConfigWarning`]).
///
/// # Safety
///
/// If `argv` is non-null and `argc > 0`, `argv` must point to an array containing at least
/// `argc` pointers. Individual string pointers must either be null or point to null-terminated
/// C strings or valid memory of at least `MAX_ARG_LEN` bytes.
pub unsafe fn parse_argv(argc: i32, argv: *const *const u8) -> PamConfig {
    // SAFETY: same contract as this function.
    let (config, warnings) = unsafe { parse_argv_with_warnings(argc, argv) };
    log_warnings(&warnings);
    config
}

/// [`parse_argv`] returning the warnings instead of logging them.
///
/// # Safety
///
/// Same contract as [`parse_argv`].
pub unsafe fn parse_argv_with_warnings(
    argc: i32,
    argv: *const *const u8,
) -> (PamConfig, Vec<ConfigWarning>) {
    // SAFETY: same contract as this function.
    let (args, mut warnings) = unsafe { collect_argv(argc, argv) };
    let (config, parse_warnings) = parse_cstrs_with_warnings(args);
    warnings.extend(parse_warnings);
    (config, warnings)
}

/// [`collect_argv`] logging its warnings; used by the exported `pam_sm_authenticate`.
///
/// # Safety
///
/// Same contract as [`parse_argv`]; the returned strings borrow the caller's memory and
/// must not outlive the PAM call.
pub unsafe fn collect_argv_logged<'a>(
    argc: i32,
    argv: *const *const u8,
) -> Vec<&'a std::ffi::CStr> {
    // SAFETY: same contract as this function.
    let (args, warnings) = unsafe { collect_argv(argc, argv) };
    log_warnings(&warnings);
    args
}

/// Reads at most `MAX_ARGC` `argv` entries as bounded C strings.
///
/// A null entry, an entry without a NUL byte within `MAX_ARG_LEN` bytes and entries
/// beyond `MAX_ARGC` are skipped with a [`ConfigWarning`].
///
/// # Safety
///
/// Same contract as [`parse_argv`]; the returned strings borrow the caller's memory.
pub unsafe fn collect_argv<'a>(
    argc: i32,
    argv: *const *const u8,
) -> (Vec<&'a std::ffi::CStr>, Vec<ConfigWarning>) {
    let mut args = Vec::new();
    let mut warnings = Vec::new();

    if argv.is_null() || argc <= 0 {
        return (args, warnings);
    }

    let Ok(total) = usize::try_from(argc) else {
        return (args, warnings);
    };
    let count = total.min(MAX_ARGC);

    for index in 0..count {
        // SAFETY: `index < count <= argc`, `argv` is non-null and holds `argc` pointers;
        // pointer arithmetic from a non-null base is never null, so only the entry itself
        // is checked below (GitHub #264).
        let arg_ptr = unsafe { *argv.add(index) };
        if arg_ptr.is_null() {
            warnings.push(ConfigWarning::NullArgument { index });
            continue;
        }
        match extract_bounded_cstr(arg_ptr) {
            Some(arg) => args.push(arg),
            None => warnings.push(ConfigWarning::ArgumentTooLong { index }),
        }
    }

    if total > count {
        warnings.push(ConfigWarning::TooManyArguments {
            ignored: total.saturating_sub(count),
        });
    }

    (args, warnings)
}

/// Borrows a null-terminated C string of fewer than `MAX_ARG_LEN` bytes, reading at
/// most `MAX_ARG_LEN` bytes; `None` when no NUL byte is found within that bound.
fn extract_bounded_cstr<'a>(ptr: *const u8) -> Option<&'a std::ffi::CStr> {
    if ptr.is_null() {
        return None;
    }

    let mut len = 0usize;
    while len < MAX_ARG_LEN {
        // SAFETY: `len < MAX_ARG_LEN` bounds the pointer read within a known small region.
        let byte = unsafe { *ptr.add(len) };
        if byte == 0 {
            // SAFETY: `ptr` points to `len` initialized bytes followed by the NUL byte just read.
            let with_nul = unsafe { std::slice::from_raw_parts(ptr, len.saturating_add(1)) };
            return std::ffi::CStr::from_bytes_with_nul(with_nul).ok();
        }
        len = len.saturating_add(1);
    }

    // Unterminated within bounds
    None
}
