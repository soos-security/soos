//! Host and identity checks (architect spec §2.4, D3, D5a).
//!
//! Both checks are pure over the parsed header list. The identity header is trusted only
//! because the socket is a `0600` file reachable by `tailscaled` and the owner alone; the
//! Host check refuses anything that is not an allowed `*.ts.net` name (or a configured
//! name) so that a misdirected or rebound request learns nothing.

use crate::config::TailscaleLogin;
use crate::{IDENTITY_HEADER, MAX_HOST_LEN, TS_NET_SUFFIX};

/// Identity refusal; every variant → `403`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// No `Tailscale-User-Login` header.
    #[error("identity header missing")]
    Missing,
    /// More than one `Tailscale-User-Login` header.
    #[error("identity header repeated")]
    Repeated,
    /// Not UTF-8, or failing `TailscaleLogin::parse`.
    #[error("identity header malformed")]
    Malformed,
    /// Not in the allowlist.
    #[error("identity not allowed")]
    NotAllowed,
}

/// Host refusal; every variant → `421`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// No `Host` header.
    #[error("host header missing")]
    Missing,
    /// More than one `Host` header.
    #[error("host header repeated")]
    Repeated,
    /// Port other than 443, IP literal, invalid DNS name or a name outside the allowlist.
    #[error("host not allowed")]
    NotAllowed,
}

/// Optional whitespace allowed around a header value (SP / HTAB).
const OWS: [char; 2] = [' ', '\t'];

/// Exactly one header named `name` (ASCII case-insensitive), counted over the whole list.
fn single_header<'a, E>(
    headers: &[(&str, &'a [u8])],
    name: &str,
    missing: E,
    repeated: E,
) -> Result<&'a [u8], E> {
    let mut found = None;
    for (header, value) in headers {
        if header.eq_ignore_ascii_case(name) {
            if found.is_some() {
                return Err(repeated);
            }
            found = Some(*value);
        }
    }
    found.ok_or(missing)
}

/// Pure DNS-name validator shared by the Host check and the `allowed_hosts` configuration:
/// lowercase `name`, 1..=`MAX_HOST_LEN` bytes, labels of `[a-z0-9-]` separated by single
/// dots, no empty label, no leading or trailing `-`, no trailing dot, and a last label that
/// is not made only of digits (an IPv4 literal is never a host).
#[must_use]
pub fn is_valid_host_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_HOST_LEN {
        return false;
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
    {
        return false;
    }
    let mut last_label = "";
    for label in name.split('.') {
        if label.is_empty() || label.starts_with('-') || label.ends_with('-') {
            return false;
        }
        last_label = label;
    }
    !last_label.bytes().all(|b| b.is_ascii_digit())
}

/// Pure. Exactly one Host header; the value is trimmed of optional whitespace,
/// ASCII-lowercased, one trailing `:443` is stripped, and the remainder must be a valid DNS
/// name (see [`is_valid_host_name`]). With `allowed_hosts` empty the name must end with
/// `.ts.net` and have at least one label before it; otherwise it must be a member of
/// `allowed_hosts`. Returns the normalized host (lowercased, `:443` removed), the value the
/// `Origin` comparison of the lock CSRF check uses.
///
/// # Errors
///
/// [`HostError`].
pub fn check_host(
    headers: &[(&str, &[u8])],
    allowed_hosts: &[String],
) -> Result<String, HostError> {
    let raw = single_header(headers, "host", HostError::Missing, HostError::Repeated)?;
    let text = std::str::from_utf8(raw).map_err(|_| HostError::NotAllowed)?;
    let lowered = text.trim_matches(OWS).to_ascii_lowercase();
    let name = lowered.strip_suffix(":443").unwrap_or(&lowered);
    if !is_valid_host_name(name) {
        return Err(HostError::NotAllowed);
    }
    if allowed_hosts.is_empty() {
        let prefix = name
            .strip_suffix(TS_NET_SUFFIX)
            .ok_or(HostError::NotAllowed)?;
        if prefix.is_empty() {
            return Err(HostError::NotAllowed);
        }
    } else if !allowed_hosts.iter().any(|allowed| allowed == name) {
        return Err(HostError::NotAllowed);
    }
    Ok(name.to_string())
}

/// Pure. Header names compared ASCII-case-insensitively; the value is trimmed of optional
/// whitespace, must be valid UTF-8 and pass `TailscaleLogin::parse`, and is compared
/// ASCII-case-insensitively (through the lowercased form) to the allowlist. Nothing about
/// the value is logged or echoed.
///
/// # Errors
///
/// [`AuthError`].
pub fn authorize(
    headers: &[(&str, &[u8])],
    allowed: &[TailscaleLogin],
) -> Result<TailscaleLogin, AuthError> {
    let raw = single_header(
        headers,
        IDENTITY_HEADER,
        AuthError::Missing,
        AuthError::Repeated,
    )?;
    let text = std::str::from_utf8(raw).map_err(|_| AuthError::Malformed)?;
    let login = TailscaleLogin::parse(text.trim_matches(OWS)).ok_or(AuthError::Malformed)?;
    allowed
        .iter()
        .find(|candidate| **candidate == login)
        .cloned()
        .ok_or(AuthError::NotAllowed)
}
