//! Pure wire contract shared by `soos-remote` and `soos-push-sender` (ADR 2026-10-06 "Web
//! Push Notifications for Failed-Password Alerts Through a Separate Sender Unit", architect
//! spec `AI/architect_spec_remote_web_push.md` §2).
//!
//! This crate is the single source of the push endpoint allowlist and its validation, the
//! public-address predicate used against server-side request forgery, the bounded frame
//! codec spoken over the sender's Unix socket, the HTTP status classification and every
//! constant both processes share. It performs no I/O and depends on no network crate.

#![forbid(unsafe_code)]

use std::fmt;
use std::net::IpAddr;

use base64ct::{Base64UrlUnpadded, Encoding};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

// ---------------------------------------------------------------------------------------
// Constants (spec §2.1)
// ---------------------------------------------------------------------------------------

/// Exact host allowlist of the push services (Apple, Google FCM, Mozilla autopush).
pub const PUSH_HOSTS: [&str; 3] = [
    "web.push.apple.com",
    "fcm.googleapis.com",
    "updates.push.services.mozilla.com",
];
/// Longest accepted endpoint URL, in bytes.
pub const MAX_PUSH_ENDPOINT_BYTES: usize = 1024;
/// Largest encrypted push body (RFC 8291 §4: a push service need not accept more).
pub const MAX_PUSH_BODY_BYTES: usize = 4096;
/// Longest `Authorization` header value.
pub const MAX_AUTHORIZATION_BYTES: usize = 1024;
/// Largest `TTL` (28 days).
pub const MAX_PUSH_TTL_S: u32 = 2_419_200;
/// Longest `Topic` (RFC 8030 §5.4).
pub const MAX_TOPIC_LEN: usize = 32;
/// Largest request frame payload (after the 4-byte length prefix).
pub const MAX_PUSH_FRAME_BYTES: usize = 12_288;
/// Largest reply frame payload.
pub const MAX_PUSH_REPLY_BYTES: usize = 256;
/// Frame format version.
pub const PUSH_FRAME_VERSION: u32 = 1;
/// Bound of each frame read or write, on both sides.
pub const PUSH_FRAME_IO_TIMEOUT_MS: u64 = 2000;
/// Sender: bound of the whole outbound exchange (DNS included).
pub const PUSH_SEND_TIMEOUT_MS: u64 = 10_000;
/// Sender: bound of the TCP and TLS connection.
pub const PUSH_CONNECT_TIMEOUT_MS: u64 = 5000;
/// Cap of an honoured `Retry-After`, in seconds.
pub const MAX_RETRY_AFTER_S: u32 = 300;
/// Sender: addresses kept after filtering.
pub const MAX_RESOLVED_ADDRESSES: usize = 16;
/// Sender: bound of the response head.
pub const MAX_PUSH_RESPONSE_HEADER_BYTES: usize = 16_384;
/// Sender: bytes of the response body read (and discarded).
pub const MAX_PUSH_RESPONSE_BODY_BYTES: usize = 1024;
/// Sender socket directory under `$XDG_RUNTIME_DIR` (created by `RuntimeDirectory=`).
pub const PUSH_SOCKET_DIR_NAME: &str = "soos-push";
/// Sender socket file name.
pub const PUSH_SOCKET_FILE_NAME: &str = "push.sock";
/// Sender exit code of a configuration error (`EX_CONFIG`).
pub const EXIT_CONFIG: i32 = 78;
/// Sender exit code of a runtime error.
pub const EXIT_RUNTIME: i32 = 1;

const _: () = assert!(
    MAX_PUSH_FRAME_BYTES
        >= 4 * MAX_PUSH_BODY_BYTES / 3 + MAX_PUSH_ENDPOINT_BYTES + MAX_AUTHORIZATION_BYTES + 512
);
const _: () = assert!(PUSH_CONNECT_TIMEOUT_MS < PUSH_SEND_TIMEOUT_MS);
/// Unpadded base64url length of [`MAX_PUSH_BODY_BYTES`] bytes.
const MAX_PUSH_BODY_TEXT_LEN: usize = (MAX_PUSH_BODY_BYTES * 4).div_ceil(3);

const _: () = assert!(MAX_PUSH_REPLY_BYTES < MAX_PUSH_FRAME_BYTES);

// ---------------------------------------------------------------------------------------
// Endpoint (spec §2.2)
// ---------------------------------------------------------------------------------------

/// Why an endpoint was refused. Fixed texts: the value is never echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    /// Empty or longer than [`MAX_PUSH_ENDPOINT_BYTES`].
    #[error("push endpoint length out of bounds")]
    TooLong,
    /// Not an `https://` URL.
    #[error("push endpoint is not https")]
    NotHttps,
    /// The authority carries user information.
    #[error("push endpoint has user info")]
    UserInfo,
    /// The authority carries a port or an IP literal.
    #[error("push endpoint has a port")]
    Port,
    /// The host is not one of [`PUSH_HOSTS`].
    #[error("push service not allowed")]
    HostNotAllowed,
    /// The path or a byte of the URL is not allowed.
    #[error("push endpoint path refused")]
    BadPath,
}

/// A validated push endpoint (capability URL: never logged, never echoed).
#[derive(Clone, PartialEq, Eq)]
pub struct PushEndpoint {
    raw: String,
    host: &'static str,
}

impl fmt::Debug for PushEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PushEndpoint(<redacted>)")
    }
}

const HTTPS_PREFIX: &str = "https://";

fn is_path_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.' | b'_' | b'~' | b'/' | b':' | b'=' | b'+' | b'%'
        )
}

impl PushEndpoint {
    /// Validates `raw` against the rules of spec §2.2, in order.
    ///
    /// # Errors
    /// The first rule `raw` breaks, as an [`EndpointError`].
    pub fn parse(raw: &str) -> Result<Self, EndpointError> {
        let bytes = raw.as_bytes();
        if bytes.is_empty() || bytes.len() > MAX_PUSH_ENDPOINT_BYTES {
            return Err(EndpointError::TooLong);
        }
        if !bytes.iter().all(|b| (0x21..=0x7e).contains(b)) {
            return Err(EndpointError::BadPath);
        }
        let rest = raw
            .strip_prefix(HTTPS_PREFIX)
            .ok_or(EndpointError::NotHttps)?;
        let slash = rest.find('/').ok_or(EndpointError::BadPath)?;
        let (authority, path) = rest.split_at(slash);
        if authority.contains('@') {
            return Err(EndpointError::UserInfo);
        }
        if authority.contains(':') || authority.contains('[') {
            return Err(EndpointError::Port);
        }
        let host = PUSH_HOSTS
            .iter()
            .copied()
            .find(|h| h.as_bytes() == authority.as_bytes())
            .ok_or(EndpointError::HostNotAllowed)?;
        if path.len() < 2 || !path.bytes().all(is_path_byte) || path.contains("//") {
            return Err(EndpointError::BadPath);
        }
        if path
            .split('/')
            .any(|segment| segment == "." || segment == "..")
        {
            return Err(EndpointError::BadPath);
        }
        Ok(Self {
            raw: raw.to_owned(),
            host,
        })
    }

    /// The full endpoint URL.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The allowlisted host (a [`PUSH_HOSTS`] element).
    #[must_use]
    pub fn host(&self) -> &'static str {
        self.host
    }

    /// `https://<host>`: the VAPID `aud` (RFC 8292 §2: origin, no path, no trailing slash).
    #[must_use]
    pub fn origin(&self) -> String {
        format!("{HTTPS_PREFIX}{}", self.host)
    }
}

// ---------------------------------------------------------------------------------------
// Public addresses (spec §2.3)
// ---------------------------------------------------------------------------------------

/// `(network, prefix length)` ranges refused for IPv4.
const V4_REFUSED: [(u32, u32); 15] = [
    (0x0000_0000, 8),  // 0.0.0.0/8
    (0x0a00_0000, 8),  // 10.0.0.0/8
    (0x6440_0000, 10), // 100.64.0.0/10 (CGNAT, tailnet, MagicDNS)
    (0x7f00_0000, 8),  // 127.0.0.0/8
    (0xa9fe_0000, 16), // 169.254.0.0/16
    (0xac10_0000, 12), // 172.16.0.0/12
    (0xc000_0000, 24), // 192.0.0.0/24
    (0xc000_0200, 24), // 192.0.2.0/24
    (0xc058_6300, 24), // 192.88.99.0/24
    (0xc0a8_0000, 16), // 192.168.0.0/16
    (0xc612_0000, 15), // 198.18.0.0/15
    (0xc633_6400, 24), // 198.51.100.0/24
    (0xcb00_7100, 24), // 203.0.113.0/24
    (0xe000_0000, 4),  // 224.0.0.0/4
    (0xf000_0000, 4),  // 240.0.0.0/4
];

/// Ranges refused inside `2000::/3` (everything outside it is refused).
const V6_REFUSED: [(u128, u32); 3] = [
    (0x2001_0000_0000_0000_0000_0000_0000_0000, 23), // IETF special purpose, Teredo
    (0x2001_0db8_0000_0000_0000_0000_0000_0000, 32), // documentation
    (0x2002_0000_0000_0000_0000_0000_0000_0000, 16), // 6to4
];

fn in_v4(addr: u32, (net, len): (u32, u32)) -> bool {
    let mask = u32::MAX
        .checked_shl(32_u32.saturating_sub(len))
        .unwrap_or(0);
    addr & mask == net
}

fn in_v6(addr: u128, (net, len): (u128, u32)) -> bool {
    let mask = u128::MAX
        .checked_shl(128_u32.saturating_sub(len))
        .unwrap_or(0);
    addr & mask == net
}

/// Pure. True only for globally routable unicast addresses (integer masks, never text).
#[must_use]
pub fn is_public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let bits = u32::from(v4);
            !V4_REFUSED.iter().any(|range| in_v4(bits, *range))
        }
        IpAddr::V6(v6) => {
            let bits = u128::from(v6);
            in_v6(bits, (0x2000_0000_0000_0000_0000_0000_0000_0000, 3))
                && !V6_REFUSED.iter().any(|range| in_v6(bits, *range))
        }
    }
}

// ---------------------------------------------------------------------------------------
// Frames (spec §2.4)
// ---------------------------------------------------------------------------------------

/// The RFC 8030 `Urgency` header value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Urgency {
    /// `very-low`
    VeryLow,
    /// `low`
    Low,
    /// `normal`
    Normal,
    /// `high`
    High,
}

/// One outbound push request. No `Debug`: the authorization and the body are secret-bearing.
#[derive(Clone, PartialEq, Eq)]
pub struct DeliveryRequest {
    /// The validated endpoint.
    pub endpoint: PushEndpoint,
    /// `vapid t=<jwt>, k=<key>`; 1..=[`MAX_AUTHORIZATION_BYTES`] printable ASCII.
    pub authorization: Zeroizing<String>,
    /// 1..=[`MAX_PUSH_TTL_S`].
    pub ttl_s: u32,
    /// The `Urgency` header.
    pub urgency: Urgency,
    /// 1..=[`MAX_TOPIC_LEN`] base64url characters, or `None`.
    pub topic: Option<String>,
    /// The encrypted body, 1..=[`MAX_PUSH_BODY_BYTES`].
    pub body: Zeroizing<Vec<u8>>,
}

/// What happened to one outbound request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// 2xx.
    Delivered,
    /// 404 or 410: the subscription no longer exists.
    Gone,
    /// 3xx or a 4xx other than 404, 410, 429.
    Rejected,
    /// 429, 5xx or a transport error.
    Retry,
    /// Refused locally (endpoint, address filter, malformed frame).
    Refused,
}

/// The sender's answer to one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryReply {
    /// The classified outcome.
    pub outcome: Outcome,
    /// The push service status, when a response was received (100..=599).
    pub status: Option<u16>,
    /// Seconds, 1..=[`MAX_RETRY_AFTER_S`], only with [`Outcome::Retry`].
    pub retry_after_s: Option<u32>,
}

/// Why a frame was refused. Fixed texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The frame ended early.
    #[error("push frame truncated")]
    Truncated,
    /// The length prefix exceeds the bound.
    #[error("push frame too large")]
    TooLarge,
    /// Not the expected JSON object.
    #[error("push frame is not valid")]
    Json,
    /// Unknown frame version.
    #[error("push frame version not supported")]
    Version,
    /// The endpoint is refused.
    #[error("push frame endpoint refused: {0}")]
    Endpoint(EndpointError),
    /// The authorization value is refused.
    #[error("push frame authorization refused")]
    Authorization,
    /// The TTL is out of bounds.
    #[error("push frame ttl out of bounds")]
    Ttl,
    /// The topic is refused.
    #[error("push frame topic refused")]
    Topic,
    /// The body is refused.
    #[error("push frame body refused")]
    Body,
    /// The reply is inconsistent.
    #[error("push reply refused")]
    Reply,
}

/// `Option<T>` that must be present (it may be `null`, never absent).
fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Serialize)]
struct RequestOut<'a> {
    v: u32,
    endpoint: &'a str,
    authorization: &'a str,
    ttl: u32,
    urgency: Urgency,
    topic: Option<&'a str>,
    body: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestIn {
    v: u32,
    endpoint: String,
    authorization: String,
    ttl: u32,
    urgency: Urgency,
    #[serde(deserialize_with = "required_option")]
    topic: Option<String>,
    body: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyWire {
    v: u32,
    outcome: Outcome,
    #[serde(deserialize_with = "required_option")]
    status: Option<u16>,
    #[serde(deserialize_with = "required_option")]
    retry_after_s: Option<u32>,
}

fn check_authorization(value: &str) -> Result<(), FrameError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_AUTHORIZATION_BYTES
        || !value.starts_with("vapid t=")
        || !bytes.iter().all(|b| (0x20..=0x7e).contains(b))
    {
        return Err(FrameError::Authorization);
    }
    Ok(())
}

fn check_ttl(ttl: u32) -> Result<(), FrameError> {
    if ttl == 0 || ttl > MAX_PUSH_TTL_S {
        return Err(FrameError::Ttl);
    }
    Ok(())
}

fn check_topic(topic: Option<&str>) -> Result<(), FrameError> {
    if let Some(topic) = topic {
        if topic.is_empty()
            || topic.len() > MAX_TOPIC_LEN
            || !topic
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(FrameError::Topic);
        }
    }
    Ok(())
}

fn check_body(body: &[u8]) -> Result<(), FrameError> {
    if body.is_empty() || body.len() > MAX_PUSH_BODY_BYTES {
        return Err(FrameError::Body);
    }
    Ok(())
}

fn check_reply(reply: &DeliveryReply) -> Result<(), FrameError> {
    if let Some(status) = reply.status {
        if !(100..=599).contains(&status) {
            return Err(FrameError::Reply);
        }
    }
    if let Some(after) = reply.retry_after_s {
        if reply.outcome != Outcome::Retry || after == 0 || after > MAX_RETRY_AFTER_S {
            return Err(FrameError::Reply);
        }
    }
    Ok(())
}

fn with_prefix(payload: &[u8]) -> Result<Zeroizing<Vec<u8>>, FrameError> {
    let len = u32::try_from(payload.len()).map_err(|_| FrameError::TooLarge)?;
    let total = payload.len().checked_add(4).ok_or(FrameError::TooLarge)?;
    let mut frame = Zeroizing::new(Vec::with_capacity(total));
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Encodes a request as a frame (4-byte big-endian length prefix + compact JSON).
///
/// # Errors
/// The first invalid field, as the decoder would report it.
pub fn encode_request(req: &DeliveryRequest) -> Result<Zeroizing<Vec<u8>>, FrameError> {
    check_authorization(&req.authorization)?;
    check_ttl(req.ttl_s)?;
    check_topic(req.topic.as_deref())?;
    check_body(&req.body)?;
    let body = Zeroizing::new(Base64UrlUnpadded::encode_string(&req.body));
    let wire = RequestOut {
        v: PUSH_FRAME_VERSION,
        endpoint: req.endpoint.as_str(),
        authorization: &req.authorization,
        ttl: req.ttl_s,
        urgency: req.urgency,
        topic: req.topic.as_deref(),
        body: &body,
    };
    let json = Zeroizing::new(serde_json::to_vec(&wire).map_err(|_| FrameError::Json)?);
    if json.len() > MAX_PUSH_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    with_prefix(&json)
}

/// Decodes a request payload (without its prefix) and re-runs every validation.
///
/// # Errors
/// [`FrameError`] naming the refused part.
pub fn decode_request(payload: &[u8]) -> Result<DeliveryRequest, FrameError> {
    if payload.len() > MAX_PUSH_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    let wire: RequestIn = serde_json::from_slice(payload).map_err(|_| FrameError::Json)?;
    // Every secret-bearing string is wiped on drop from here on.
    let authorization = Zeroizing::new(wire.authorization);
    let body_text = Zeroizing::new(wire.body);
    if wire.v != PUSH_FRAME_VERSION {
        return Err(FrameError::Version);
    }
    let endpoint = PushEndpoint::parse(&wire.endpoint).map_err(FrameError::Endpoint)?;
    check_authorization(&authorization)?;
    check_ttl(wire.ttl)?;
    check_topic(wire.topic.as_deref())?;
    // Bound the text before decoding it.
    if body_text.is_empty() || body_text.len() > MAX_PUSH_BODY_TEXT_LEN {
        return Err(FrameError::Body);
    }
    let body =
        Zeroizing::new(Base64UrlUnpadded::decode_vec(&body_text).map_err(|_| FrameError::Body)?);
    check_body(&body)?;
    Ok(DeliveryRequest {
        endpoint,
        authorization,
        ttl_s: wire.ttl,
        urgency: wire.urgency,
        topic: wire.topic,
        body,
    })
}

/// Encodes a reply as a frame.
///
/// # Errors
/// [`FrameError::Reply`] for an inconsistent reply.
pub fn encode_reply(reply: &DeliveryReply) -> Result<Vec<u8>, FrameError> {
    check_reply(reply)?;
    let wire = ReplyWire {
        v: PUSH_FRAME_VERSION,
        outcome: reply.outcome,
        status: reply.status,
        retry_after_s: reply.retry_after_s,
    };
    let json = serde_json::to_vec(&wire).map_err(|_| FrameError::Json)?;
    if json.len() > MAX_PUSH_REPLY_BYTES {
        return Err(FrameError::TooLarge);
    }
    let frame = with_prefix(&json)?;
    Ok(frame.to_vec())
}

/// Decodes a reply payload (without its prefix).
///
/// # Errors
/// [`FrameError`] naming the refused part.
pub fn decode_reply(payload: &[u8]) -> Result<DeliveryReply, FrameError> {
    if payload.len() > MAX_PUSH_REPLY_BYTES {
        return Err(FrameError::TooLarge);
    }
    let wire: ReplyWire = serde_json::from_slice(payload).map_err(|_| FrameError::Json)?;
    if wire.v != PUSH_FRAME_VERSION {
        return Err(FrameError::Version);
    }
    let reply = DeliveryReply {
        outcome: wire.outcome,
        status: wire.status,
        retry_after_s: wire.retry_after_s,
    };
    check_reply(&reply)?;
    Ok(reply)
}

/// Validates a 4-byte big-endian prefix against `max` before any allocation.
///
/// # Errors
/// [`FrameError::TooLarge`] when the announced length exceeds `max`.
pub fn frame_len(prefix: [u8; 4], max: usize) -> Result<usize, FrameError> {
    let len = usize::try_from(u32::from_be_bytes(prefix)).map_err(|_| FrameError::TooLarge)?;
    if len > max {
        return Err(FrameError::TooLarge);
    }
    Ok(len)
}

// ---------------------------------------------------------------------------------------
// Status classification (spec §2.5)
// ---------------------------------------------------------------------------------------

/// Classifies a push service status; `retry_after` is the raw `Retry-After` value.
#[must_use]
pub fn classify_status(status: u16, retry_after: Option<&[u8]>) -> DeliveryReply {
    let outcome = match status {
        200..=299 => Outcome::Delivered,
        404 | 410 => Outcome::Gone,
        429 | 500..=599 => Outcome::Retry,
        _ => Outcome::Rejected,
    };
    let retry_after_s = if outcome == Outcome::Retry {
        retry_after.and_then(parse_retry_after)
    } else {
        None
    };
    DeliveryReply {
        outcome,
        status: (100..=599).contains(&status).then_some(status),
        retry_after_s,
    }
}

/// `Retry-After` as delta-seconds: 1..=10 ASCII digits, 0 ignored, capped at
/// [`MAX_RETRY_AFTER_S`]; anything else (an HTTP date included) is `None`.
#[must_use]
pub fn parse_retry_after(raw: &[u8]) -> Option<u32> {
    if raw.is_empty() || raw.len() > 10 || !raw.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut value: u64 = 0;
    for digit in raw {
        value = value
            .saturating_mul(10)
            .saturating_add(u64::from(digit.saturating_sub(b'0')));
    }
    if value == 0 {
        return None;
    }
    let capped = value.min(u64::from(MAX_RETRY_AFTER_S));
    u32::try_from(capped).ok()
}
