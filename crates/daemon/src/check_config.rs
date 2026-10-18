//! `soos-daemon --check-config`: validates the configuration and exits (GitHub #441).
//!
//! The check goes through [`DaemonConfig::load_classified`], the loader the daemon start uses,
//! so a file is accepted exactly when the daemon would start with it (same bounded reader, same
//! bounds and validation, same unknown-key policy). It opens no socket, camera, model or Tokio
//! runtime. The result is one bounded English line and a sysexits code.

#![forbid(unsafe_code)]

use std::path::Path;

use crate::config::{ConfigLoadFailure, ConfigOrigin, DaemonConfig, DEFAULT_CONFIG_PATH};

/// Exit code of a valid configuration (`EX_OK`).
pub const EXIT_CONFIG_VALID: u8 = 0;

/// Exit code when the file cannot be opened or read (`EX_NOINPUT`): missing, a dangling
/// symbolic link, not a regular file, or permission denied.
pub const EXIT_CONFIG_UNAVAILABLE: u8 = 66;

/// Exit code when the file is refused (`EX_CONFIG`): larger than the bound, not UTF-8, not
/// valid TOML, a value of the wrong type, or a validation error.
pub const EXIT_CONFIG_INVALID: u8 = 78;

/// Largest diagnostic line, in bytes (without the trailing newline).
pub const MAX_CHECK_DIAGNOSTIC_BYTES: usize = 512;

/// Marker appended to a diagnostic cut at [`MAX_CHECK_DIAGNOSTIC_BYTES`].
const TRUNCATION_MARKER: &str = "...";

/// Prefix of every diagnostic line.
const PREFIX: &str = "soos-daemon: ";

/// Where the diagnostic line is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStream {
    /// Standard output (valid configuration).
    Stdout,
    /// Standard error (refused configuration).
    Stderr,
}

/// Outcome of a configuration check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    /// Process exit code ([`EXIT_CONFIG_VALID`], [`EXIT_CONFIG_UNAVAILABLE`] or
    /// [`EXIT_CONFIG_INVALID`]).
    pub exit_code: u8,
    /// Stream the line is written to.
    pub stream: CheckStream,
    /// The single diagnostic line, without a newline, at most
    /// [`MAX_CHECK_DIAGNOSTIC_BYTES`] bytes.
    pub line: String,
}

/// Checks the configuration the daemon would load from `path_opt`, or else from
/// `system_path` (the runtime defaults when that file does not exist).
#[must_use]
pub fn check_config(path_opt: Option<&Path>, system_path: &Path) -> CheckReport {
    match DaemonConfig::load_classified(path_opt, system_path) {
        Ok((config, ConfigOrigin::File(path))) => {
            let mut line = format!("{PREFIX}configuration '{}' is valid", path.display());
            if !config.warnings.is_empty() {
                line.push_str(&format!(
                    " ({} warning(s), logged at daemon start)",
                    config.warnings.len()
                ));
            }
            report(EXIT_CONFIG_VALID, CheckStream::Stdout, &line)
        }
        Ok((_, ConfigOrigin::RuntimeDefault(path))) => report(
            EXIT_CONFIG_VALID,
            CheckStream::Stdout,
            &format!(
                "{PREFIX}no configuration file at '{}'; the built-in defaults are valid",
                path.display()
            ),
        ),
        Err(failure) => {
            let code = match failure {
                ConfigLoadFailure::Unavailable(_) => EXIT_CONFIG_UNAVAILABLE,
                ConfigLoadFailure::Invalid(_) => EXIT_CONFIG_INVALID,
            };
            report(
                code,
                CheckStream::Stderr,
                &format!("{PREFIX}configuration refused: {}", failure.error()),
            )
        }
    }
}

/// [`check_config`] with the daemon's system path [`DEFAULT_CONFIG_PATH`].
#[must_use]
pub fn check_default_config(path_opt: Option<&Path>) -> CheckReport {
    check_config(path_opt, Path::new(DEFAULT_CONFIG_PATH))
}

fn report(exit_code: u8, stream: CheckStream, text: &str) -> CheckReport {
    CheckReport {
        exit_code,
        stream,
        line: bounded_diagnostic(text),
    }
}

/// Reduces `text` to one safe line: the text before the first newline (a `toml` error quotes
/// the offending source line from its second line on, which is never printed), every other
/// control character replaced with `?`, and at most [`MAX_CHECK_DIAGNOSTIC_BYTES`] bytes, cut on
/// a character boundary and ending with `...` when cut.
#[must_use]
pub fn bounded_diagnostic(text: &str) -> String {
    let first = text.split('\n').next().unwrap_or_default();
    let clean: String = first
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect();
    if clean.len() <= MAX_CHECK_DIAGNOSTIC_BYTES {
        return clean;
    }
    let budget = MAX_CHECK_DIAGNOSTIC_BYTES.saturating_sub(TRUNCATION_MARKER.len());
    let mut out = String::with_capacity(MAX_CHECK_DIAGNOSTIC_BYTES);
    for c in clean.chars() {
        if out.len().saturating_add(c.len_utf8()) > budget {
            break;
        }
        out.push(c);
    }
    out.push_str(TRUNCATION_MARKER);
    out
}
