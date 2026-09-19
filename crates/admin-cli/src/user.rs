//! User provisioning and system group management for `soos`.

use std::process::Command;

use crate::error::AdminCliError;

/// Default system group for biometric authentication access.
pub const DEFAULT_SOOS_GROUP: &str = "soos";

/// Validates a username against standard POSIX naming conventions:
/// - Must not be empty.
/// - Maximum length of 32 characters.
/// - Must start with a lowercase ASCII letter or underscore (`[a-z_]`).
/// - Subsequent characters must be lowercase alphanumeric, underscore, or dash (`[a-z0-9_-]`).
/// - May optionally end with `$` (for POSIX machine/system accounts).
pub fn validate_username(username: &str) -> Result<(), AdminCliError> {
    if username.is_empty() || username.len() > 32 {
        return Err(AdminCliError::InvalidUsername(username.to_string()));
    }

    let mut chars = username.chars().peekable();
    let first = match chars.next() {
        Some(c) => c,
        None => return Err(AdminCliError::InvalidUsername(username.to_string())),
    };

    if !first.is_ascii_lowercase() && first != '_' {
        return Err(AdminCliError::InvalidUsername(username.to_string()));
    }

    while let Some(c) = chars.next() {
        if chars.peek().is_none() && c == '$' {
            // Trailing dollar sign is permitted for machine accounts
            break;
        }

        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '_' && c != '-' {
            return Err(AdminCliError::InvalidUsername(username.to_string()));
        }
    }

    Ok(())
}

/// Dispatches user group addition via an injected command runner (for unit testing and dry runs).
pub fn add_user_to_group_with_runner<F>(
    username: &str,
    group: &str,
    runner: F,
) -> Result<(), AdminCliError>
where
    F: FnOnce(&str, &[&str]) -> Result<(), AdminCliError>,
{
    validate_username(username)?;
    runner("usermod", &["-aG", group, username])
}

/// Adds an existing system user to a target system group via `usermod -aG <group> <username>`.
pub fn add_user_to_group(username: &str, group: &str) -> Result<(), AdminCliError> {
    validate_username(username)?;

    // Verify user exists on system
    match nix::unistd::User::from_name(username) {
        Ok(Some(_)) => {}
        Ok(None) => return Err(AdminCliError::UserNotFound(username.to_string())),
        Err(err) => {
            return Err(AdminCliError::CommandFailed(format!(
                "user lookup failed: {err}"
            )))
        }
    }

    let output = Command::new("usermod")
        .arg("-aG")
        .arg(group)
        .arg(username)
        .output()
        .map_err(|err| AdminCliError::CommandFailed(format!("failed to spawn usermod: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let err_msg = if stderr.is_empty() {
            format!("usermod exited with code {:?}", output.status.code())
        } else {
            stderr
        };
        return Err(AdminCliError::CommandFailed(err_msg));
    }

    Ok(())
}

/// Adds a user to the default `soos` biometric system group.
pub fn add_user_to_soos_group(username: &str) -> Result<(), AdminCliError> {
    add_user_to_group(username, DEFAULT_SOOS_GROUP)
}
