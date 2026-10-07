//! Host and identity checks (architect spec §2.4, D3, D5a′).
//!
//! Both checks are pure over the parsed header list. Trust basis (spec §13.1, §13.2): the
//! socket is a `0600` file in a `0700` directory reachable by `tailscaled` and the owner
//! alone; `tailscale serve` was observed (Tailscale 1.102.4) to overwrite exactly three
//! client-supplied headers with the real values, `X-Forwarded-Host`, `X-Forwarded-Proto` and
//! `Tailscale-User-Login`, and these three are the only headers the host and identity checks
//! rely on. Behind Serve the `Host` header is the proxy's backend name, so it is not inspected
//! when `X-Forwarded-Host` is present; `X-Forwarded-For` is ignored. The owner forging their
//! own headers on the socket is no escalation. The host check refuses anything whose effective
//! host is not an allowed `*.ts.net` name (or a configured name), or whose proxied transport
//! is not `https`, so that a misdirected, rebound or plain-HTTP request learns nothing.

use crate::config::TailscaleLogin;
use crate::{
    FORWARDED_FOR_HEADER, FORWARDED_HOST_HEADER, FORWARDED_PROTO_HEADER, FORWARDED_PROTO_HTTPS,
    FUNNEL_HEADER, FUNNEL_HEADER_VALUE, IDENTITY_HEADER, MAX_HOST_LEN, TS_NET_SUFFIX,
};

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
    /// Both the identity and the Funnel markers are present.
    #[error("identity and funnel markers both present")]
    Ambiguous,
    /// A Funnel request while `allow_funnel` is false.
    #[error("funnel access disabled")]
    FunnelDisabled,
}

/// Host refusal; every variant → `421`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// Neither `X-Forwarded-Host` nor `Host` is present.
    #[error("host header missing")]
    Missing,
    /// The header that decides the effective host (`X-Forwarded-Host` when present,
    /// otherwise `Host`) occurs more than once.
    #[error("host header repeated")]
    Repeated,
    /// Port other than 443, IP literal, invalid DNS name, a name outside the allowlist, or an
    /// `X-Forwarded-Proto` that is absent while `X-Forwarded-Host` is present, repeated, or
    /// not `https`.
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

/// Pure. The *effective host* is decided by `X-Forwarded-Host` alone whenever that header
/// is present (set by `tailscale serve`; the `Host` header is then not inspected at all);
/// otherwise it is the single `Host` header (direct local clients). Evaluation order (spec
/// §13.2): (1) `X-Forwarded-Host` repeated → [`HostError::Repeated`]; (2) with one
/// `X-Forwarded-Host`, `X-Forwarded-Proto` must occur exactly once and equal `https`
/// (ASCII-case-insensitive, OWS-trimmed) → otherwise [`HostError::NotAllowed`]; without it,
/// `Host` must occur exactly once ([`HostError::Missing`] / [`HostError::Repeated`]) and
/// `X-Forwarded-Proto`, if present, must still be exactly one `https`; (3) the selected value
/// is trimmed of optional whitespace, ASCII-lowercased, one trailing `:443` is stripped, and
/// the remainder must be a valid DNS name (see [`is_valid_host_name`]); with `allowed_hosts`
/// empty the name must end with `.ts.net` and have at least one label before it, otherwise it
/// must be a member of `allowed_hosts`. Returns the normalized effective host (lowercased,
/// `:443` removed), the value the `Origin` comparison of the lock CSRF check uses.
///
/// # Errors
///
/// [`HostError`].
pub fn check_host(
    headers: &[(&str, &[u8])],
    allowed_hosts: &[String],
) -> Result<String, HostError> {
    let raw = match single_header(
        headers,
        FORWARDED_HOST_HEADER,
        HostError::Missing,
        HostError::Repeated,
    ) {
        Ok(forwarded) => {
            forwarded_proto_ok(headers, true)?;
            forwarded
        }
        Err(HostError::Missing) => {
            let host = single_header(headers, "host", HostError::Missing, HostError::Repeated)?;
            forwarded_proto_ok(headers, false)?;
            host
        }
        Err(err) => return Err(err),
    };
    normalize_host(raw, allowed_hosts)
}

/// Transport rule of D5a′: `X-Forwarded-Proto`, when present, must occur exactly once and its
/// OWS-trimmed value must equal `https` ASCII-case-insensitively. With `required` an absent
/// header is refused as well (a proxied request must state its scheme); otherwise absent is
/// accepted (direct local clients send no forwarding header).
fn forwarded_proto_ok(headers: &[(&str, &[u8])], required: bool) -> Result<(), HostError> {
    enum Lookup {
        Missing,
        Repeated,
    }
    let raw = match single_header(
        headers,
        FORWARDED_PROTO_HEADER,
        Lookup::Missing,
        Lookup::Repeated,
    ) {
        Ok(raw) => raw,
        Err(Lookup::Missing) if !required => return Ok(()),
        Err(Lookup::Missing | Lookup::Repeated) => return Err(HostError::NotAllowed),
    };
    let text = std::str::from_utf8(raw).map_err(|_| HostError::NotAllowed)?;
    if text
        .trim_matches(OWS)
        .eq_ignore_ascii_case(FORWARDED_PROTO_HTTPS)
    {
        Ok(())
    } else {
        Err(HostError::NotAllowed)
    }
}

/// Shared normalisation and validation of the selected raw host value (step 3 of
/// [`check_host`]); the only place a host name is validated against the allowlist.
fn normalize_host(raw: &[u8], allowed_hosts: &[String]) -> Result<String, HostError> {
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

// ---------------------------------------------------------------------------------------
// Request classification and client hint (architect spec §2.1, §2.4).
// ---------------------------------------------------------------------------------------

/// Which proxy path a request came through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathClass {
    /// Tailnet request with an allowed `Tailscale-User-Login`.
    Tailnet,
    /// Public request through Tailscale Funnel.
    Funnel,
}

/// The authenticated caller of a request (before any web-session check).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// Allowed tailnet identity.
    Tailnet(TailscaleLogin),
    /// Anonymous Funnel client.
    Funnel,
}

impl Caller {
    /// The path class of the caller.
    #[must_use]
    pub fn class(&self) -> PathClass {
        match self {
            Self::Tailnet(_) => PathClass::Tailnet,
            Self::Funnel => PathClass::Funnel,
        }
    }
}

/// Pure (spec §2.1). Counts, over the whole header list (names compared
/// ASCII-case-insensitively, exact hyphenated names only, `_` never normalised to `-`),
/// `L` = occurrences of `Tailscale-User-Login` and `F` = occurrences of
/// `Tailscale-Funnel-Request`, then decides in this order:
///   1. `L >= 1 && F >= 1` → [`AuthError::Ambiguous`];
///   2. `F == 0` → [`authorize`] → [`Caller::Tailnet`];
///   3. `F >= 2` → [`AuthError::Repeated`];
///   4. the OWS-trimmed value is not exactly `?1` → [`AuthError::Malformed`];
///   5. `!allow_funnel` → [`AuthError::FunnelDisabled`];
///   6. otherwise [`Caller::Funnel`].
///
/// `Host`, `X-Forwarded-Host` and `X-Forwarded-For` are never read here.
///
/// # Errors
///
/// [`AuthError`] (every variant → `403 {"result":"forbidden"}`).
pub fn classify_request(
    headers: &[(&str, &[u8])],
    allowed: &[TailscaleLogin],
    allow_funnel: bool,
) -> Result<Caller, AuthError> {
    let logins = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(IDENTITY_HEADER))
        .count();
    let mut markers = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(FUNNEL_HEADER));
    let Some((_, marker)) = markers.next() else {
        return authorize(headers, allowed).map(Caller::Tailnet);
    };
    if logins > 0 {
        return Err(AuthError::Ambiguous);
    }
    if markers.next().is_some() {
        return Err(AuthError::Repeated);
    }
    if trim_ows_bytes(marker) != FUNNEL_HEADER_VALUE.as_bytes() {
        return Err(AuthError::Malformed);
    }
    if !allow_funnel {
        return Err(AuthError::FunnelDisabled);
    }
    Ok(Caller::Funnel)
}

/// Optional whitespace (SP / HTAB) trimmed from both ends of a raw header value.
fn trim_ows_bytes(value: &[u8]) -> &[u8] {
    let is_ows = |b: &u8| *b == b' ' || *b == b'\t';
    let start = value.iter().position(|b| !is_ows(b)).unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|b| !is_ows(b))
        .map_or(start, |p| p.saturating_add(1));
    value.get(start..end.max(start)).unwrap_or_default()
}

/// Rate-limit bucket of an anonymous Funnel caller (spec §2.4). NEVER an authorization
/// input and never logged: `Debug` prints `<redacted>`. Holds the masked address bytes only
/// (IPv4 mapped into 16 bytes, IPv6 masked to its /64 prefix) or the shared unknown bucket.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientHint([u8; 16]);

impl ClientHint {
    /// Shared bucket for a missing, repeated or malformed `X-Forwarded-For`.
    pub const UNKNOWN: ClientHint = ClientHint([0xff; 16]);
}

impl std::fmt::Debug for ClientHint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientHint(<redacted>)")
    }
}

/// Longest `X-Forwarded-For` value parsed (the longest textual IPv6 address).
const MAX_FORWARDED_FOR_LEN: usize = 45;

/// Pure (spec §2.4). Exactly one `X-Forwarded-For` header whose OWS-trimmed value parses as
/// one `core::net::IpAddr` (no list, no port, no brackets, at most 45 bytes) gives its hint;
/// IPv4 and IPv4-mapped IPv6 share the same hint; IPv6 keeps its first 8 bytes. Anything
/// else is [`ClientHint::UNKNOWN`]. Called only for Funnel callers.
#[must_use]
pub fn client_hint(headers: &[(&str, &[u8])]) -> ClientHint {
    let Ok(raw) = single_header(headers, FORWARDED_FOR_HEADER, (), ()) else {
        return ClientHint::UNKNOWN;
    };
    let value = trim_ows_bytes(raw);
    if value.is_empty() || value.len() > MAX_FORWARDED_FOR_LEN {
        return ClientHint::UNKNOWN;
    }
    let Some(addr) = std::str::from_utf8(value)
        .ok()
        .and_then(|text| text.parse::<core::net::IpAddr>().ok())
    else {
        return ClientHint::UNKNOWN;
    };
    let v6 = match addr {
        core::net::IpAddr::V4(v4) => return ClientHint(v4.to_ipv6_mapped().octets()),
        core::net::IpAddr::V6(v6) => v6,
    };
    if let Some(v4) = v6.to_ipv4_mapped() {
        return ClientHint(v4.to_ipv6_mapped().octets());
    }
    let mut bytes = v6.octets();
    for byte in bytes.iter_mut().skip(8) {
        *byte = 0;
    }
    ClientHint(bytes)
}
