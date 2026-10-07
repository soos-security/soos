#![forbid(unsafe_code)]
//! Outbound Web Push sender of the soos remote companion (ADR 2026-10-06 "Web Push
//! Notifications for Failed-Password Alerts Through a Separate Sender Unit", architect spec
//! `AI/architect_spec_remote_web_push.md` §8).
//!
//! The sender owns no key and holds nothing at rest: `soos-remote` hands it, over a `0600`
//! Unix socket, an already-encrypted body, an already-signed `Authorization` value and an
//! allowlisted endpoint; the sender performs exactly one HTTPS `POST` and answers the
//! classified outcome. Server-side request forgery is refused in three layers: the endpoint
//! is re-validated when the frame is decoded, every resolved address must be public (the
//! [`FilteringResolver`] is the only resolver of the agent, so the connector never performs
//! a second lookup), and redirects, proxies and plain HTTP are disabled.

use std::fmt;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::SocketAddr;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nix::fcntl::OFlag;
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use soos_push_protocol::{
    classify_status, decode_request, encode_reply, frame_len, is_public_address, DeliveryReply,
    DeliveryRequest, Outcome, Urgency, MAX_PUSH_FRAME_BYTES, MAX_PUSH_RESPONSE_BODY_BYTES,
    MAX_PUSH_RESPONSE_HEADER_BYTES, MAX_RESOLVED_ADDRESSES, PUSH_CONNECT_TIMEOUT_MS,
    PUSH_FRAME_IO_TIMEOUT_MS, PUSH_HOSTS, PUSH_SEND_TIMEOUT_MS, PUSH_SOCKET_FILE_NAME,
};
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use ureq::Agent;
use zeroize::Zeroizing;

/// The port of every push service (https).
const HTTPS_PORT: u16 = 443;
/// Fixed `User-Agent` of every outbound request.
const USER_AGENT: &str = "soos-push-sender";

// ---------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------

/// Socket setup failure. Fixed texts: no path is echoed.
#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    /// The socket directory is missing, not a directory, not owned or not private.
    #[error("push socket directory refused")]
    Directory,
    /// Something other than a stale socket of the uid is at the socket path.
    #[error("push socket path occupied")]
    Occupied,
    /// An I/O operation failed.
    #[error("push socket setup failed")]
    Io,
}

/// Start-up failure of the sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SenderError {
    /// Refused to run as root.
    #[error("the push sender refuses to run as root")]
    Root,
}

/// Refuses root (real or effective uid 0).
///
/// # Errors
/// [`SenderError::Root`] when either uid is 0.
pub fn check_not_root(uid: u32, euid: u32) -> Result<(), SenderError> {
    if uid == 0 || euid == 0 {
        return Err(SenderError::Root);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------------------

/// The outbound policy; every field is a constant of `soos-push-protocol`. The security
/// switches (https only, no redirect, no environment proxy) are fixed in the agent
/// construction and never relaxed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendPolicy {
    /// Plain HTTP refused.
    pub https_only: bool,
    /// Redirects followed (0: a 3xx is returned and classified `rejected`).
    pub max_redirects: u32,
    /// Whether a proxy from the environment is used.
    pub use_env_proxy: bool,
    /// Bound of the whole exchange.
    pub timeout_global_ms: u64,
    /// Bound of the TCP and TLS connection.
    pub timeout_connect_ms: u64,
    /// Bound of the response head.
    pub max_response_header_bytes: usize,
    /// Bytes of the response body read and discarded.
    pub max_response_body_bytes: usize,
}

impl Default for SendPolicy {
    fn default() -> Self {
        Self {
            https_only: true,
            max_redirects: 0,
            use_env_proxy: false,
            timeout_global_ms: PUSH_SEND_TIMEOUT_MS,
            timeout_connect_ms: PUSH_CONNECT_TIMEOUT_MS,
            max_response_header_bytes: MAX_PUSH_RESPONSE_HEADER_BYTES,
            max_response_body_bytes: MAX_PUSH_RESPONSE_BODY_BYTES,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------------------

/// Why a lookup failed (transient).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LookupError {
    /// The lookup failed.
    #[error("push host lookup failed")]
    Failed,
    /// The lookup timed out.
    #[error("push host lookup timed out")]
    Timeout,
}

/// DNS seam: production uses the system lookup, tests inject a closure.
pub type AddressLookup = Arc<dyn Fn(&str) -> Result<Vec<SocketAddr>, LookupError> + Send + Sync>;

/// Pure filter: `host` must be a `PUSH_HOSTS` element (else `Refused`, lookup not called);
/// a lookup error is `Retry`; an empty answer or any non-public address is `Refused`;
/// otherwise every address with port 443, IPv4 first (stable), at most 16.
///
/// # Errors
/// [`Outcome::Refused`] or [`Outcome::Retry`] as described.
pub fn filter_addresses(
    host: &str,
    lookup: &(dyn Fn(&str) -> Result<Vec<SocketAddr>, LookupError> + Send + Sync),
) -> Result<Vec<SocketAddr>, Outcome> {
    if !PUSH_HOSTS
        .iter()
        .any(|allowed| allowed.as_bytes() == host.as_bytes())
    {
        return Err(Outcome::Refused);
    }
    let answer = lookup(host).map_err(|_| Outcome::Retry)?;
    if answer.is_empty() || answer.iter().any(|addr| !is_public_address(addr.ip())) {
        return Err(Outcome::Refused);
    }
    let (v4, v6): (Vec<SocketAddr>, Vec<SocketAddr>) =
        answer.into_iter().partition(SocketAddr::is_ipv4);
    Ok(v4
        .into_iter()
        .chain(v6)
        .map(|addr| SocketAddr::new(addr.ip(), HTTPS_PORT))
        .take(MAX_RESOLVED_ADDRESSES)
        .collect())
}

/// Marker carried inside `ureq::Error::Other` when the resolver refuses a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("push host address refused")]
pub struct ResolveRefused;

/// The only resolver of the production agent: [`filter_addresses`] over the injected lookup
/// or the system lookup.
#[derive(Clone)]
pub struct FilteringResolver {
    lookup: Option<AddressLookup>,
}

impl fmt::Debug for FilteringResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FilteringResolver")
    }
}

impl Resolver for FilteringResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let Some(host) = uri.host() else {
            return Err(ureq::Error::Other(Box::new(ResolveRefused)));
        };
        let filtered = match &self.lookup {
            Some(lookup) => filter_addresses(host, &**lookup),
            None => {
                let system = |_: &str| -> Result<Vec<SocketAddr>, LookupError> {
                    DefaultResolver::default()
                        .resolve(uri, config, timeout)
                        .map(|addrs| addrs.iter().copied().collect())
                        .map_err(|e| match e {
                            ureq::Error::Timeout(_) => LookupError::Timeout,
                            _ => LookupError::Failed,
                        })
                };
                filter_addresses(host, &system)
            }
        };
        match filtered {
            Ok(addrs) => {
                let mut out = self.empty();
                for addr in addrs {
                    out.push(addr);
                }
                Ok(out)
            }
            Err(Outcome::Refused) => Err(ureq::Error::Other(Box::new(ResolveRefused))),
            Err(_) => Err(ureq::Error::HostNotFound),
        }
    }
}

/// Maps a failed send to the reply: the resolver marker is `Refused`, anything else `Retry`.
#[must_use]
pub fn send_error_outcome(error: &ureq::Error) -> DeliveryReply {
    let refused = matches!(error, ureq::Error::Other(inner) if inner.is::<ResolveRefused>());
    DeliveryReply {
        outcome: if refused {
            Outcome::Refused
        } else {
            Outcome::Retry
        },
        status: None,
        retry_after_s: None,
    }
}

// ---------------------------------------------------------------------------------------
// Delivery
// ---------------------------------------------------------------------------------------

/// One outbound attempt.
pub trait Deliverer: Send + Sync {
    /// Performs the request and classifies the result.
    fn deliver(&self, request: &DeliveryRequest) -> DeliveryReply;
}

/// Production deliverer: one ureq agent (rustls, ring, webpki roots) built with the
/// filtering resolver.
pub struct UreqDeliverer {
    agent: Agent,
    max_body: u64,
}

impl fmt::Debug for UreqDeliverer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UreqDeliverer")
    }
}

impl UreqDeliverer {
    /// Production: the system lookup.
    #[must_use]
    pub fn new(policy: SendPolicy) -> Self {
        Self::build(policy, FilteringResolver { lookup: None })
    }

    /// Test seam: the same construction with an injected lookup.
    #[must_use]
    pub fn with_lookup(policy: SendPolicy, lookup: AddressLookup) -> Self {
        Self::build(
            policy,
            FilteringResolver {
                lookup: Some(lookup),
            },
        )
    }

    fn build(policy: SendPolicy, resolver: FilteringResolver) -> Self {
        let config = Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .proxy(None)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_millis(policy.timeout_global_ms)))
            .timeout_connect(Some(Duration::from_millis(policy.timeout_connect_ms)))
            .max_response_header_size(policy.max_response_header_bytes)
            .user_agent(USER_AGENT)
            .build();
        let agent = Agent::with_parts(config, DefaultConnector::default(), resolver);
        Self {
            agent,
            max_body: u64::try_from(policy.max_response_body_bytes).unwrap_or(u64::MAX),
        }
    }
}

fn urgency_text(urgency: Urgency) -> &'static str {
    match urgency {
        Urgency::VeryLow => "very-low",
        Urgency::Low => "low",
        Urgency::Normal => "normal",
        Urgency::High => "high",
    }
}

impl Deliverer for UreqDeliverer {
    fn deliver(&self, request: &DeliveryRequest) -> DeliveryReply {
        let mut builder = self
            .agent
            .post(request.endpoint.as_str())
            .header("TTL", request.ttl_s.to_string())
            .header("Urgency", urgency_text(request.urgency))
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("Authorization", request.authorization.as_str());
        if let Some(topic) = &request.topic {
            builder = builder.header("Topic", topic.as_str());
        }
        match builder.send(&request.body[..]) {
            Ok(mut response) => {
                let status = response.status().as_u16();
                let reply = classify_status(
                    status,
                    response
                        .headers()
                        .get("retry-after")
                        .map(ureq::http::HeaderValue::as_bytes),
                );
                // The outcome is decided; read a bounded part of the body and discard it.
                let _ = response
                    .body_mut()
                    .with_config()
                    .limit(self.max_body)
                    .read_to_vec();
                reply
            }
            Err(error) => send_error_outcome(&error),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------------------

/// Installs the only subscriber of the process: stderr without colours and a fixed filter
/// (dependency targets off). No `log` bridge is ever installed and no environment variable
/// changes the filter, so the `log` records of the HTTP and TLS crates are discarded. An
/// installation error leaves the process without logs.
pub fn install_logging() {
    use tracing_subscriber::filter::{LevelFilter, Targets};
    use tracing_subscriber::layer::SubscriberExt;

    let filter = Targets::new()
        .with_default(LevelFilter::INFO)
        .with_target("ureq", LevelFilter::OFF)
        .with_target("ureq_proto", LevelFilter::OFF)
        .with_target("rustls", LevelFilter::OFF);
    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::io::stderr),
        )
        .with(filter);
    let _ = tracing::subscriber::set_global_default(subscriber);
}

// ---------------------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------------------

/// Fills `buf` before `deadline` (one cumulative deadline, never a per-read timeout).
fn read_exact_before(stream: &mut UnixStream, buf: &mut [u8], deadline: Instant) -> bool {
    let mut filled = 0;
    while filled < buf.len() {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        if remaining.is_zero() || stream.set_read_timeout(Some(remaining)).is_err() {
            return false;
        }
        let Some(rest) = buf.get_mut(filled..) else {
            return false;
        };
        match stream.read(rest) {
            Ok(0) => return false,
            Ok(n) => filled = filled.saturating_add(n),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    true
}

fn frame_deadline() -> Instant {
    let now = Instant::now();
    // An unrepresentable deadline (never in practice) fails closed: the read ends at once.
    now.checked_add(Duration::from_millis(PUSH_FRAME_IO_TIMEOUT_MS))
        .unwrap_or(now)
}

/// Serves one accepted connection: peer uid check, one bounded request frame, one delivery,
/// one reply, close.
pub fn serve_connection(stream: UnixStream, own_uid: u32, deliverer: &dyn Deliverer) {
    let mut stream = stream;
    match getsockopt(&stream, PeerCredentials) {
        Ok(credentials) if credentials.uid() == own_uid => {}
        _ => return,
    }
    let mut prefix = [0u8; 4];
    if !read_exact_before(&mut stream, &mut prefix, frame_deadline()) {
        return;
    }
    let Ok(len) = frame_len(prefix, MAX_PUSH_FRAME_BYTES) else {
        return;
    };
    let mut payload = Zeroizing::new(vec![0u8; len]);
    if !read_exact_before(&mut stream, &mut payload, frame_deadline()) {
        return;
    }
    let reply = match decode_request(&payload) {
        Ok(request) => deliverer.deliver(&request),
        Err(_) => DeliveryReply {
            outcome: Outcome::Refused,
            status: None,
            retry_after_s: None,
        },
    };
    drop(payload);
    let Ok(frame) = encode_reply(&reply) else {
        return;
    };
    if stream
        .set_write_timeout(Some(Duration::from_millis(PUSH_FRAME_IO_TIMEOUT_MS)))
        .is_err()
    {
        return;
    }
    let _ = stream.write_all(&frame);
    let _ = stream.flush();
}

// ---------------------------------------------------------------------------------------
// Socket setup
// ---------------------------------------------------------------------------------------

/// Opens `dir` without following a symlink and checks it through the descriptor.
fn checked_dir(dir: &Path, uid: u32, created: bool) -> Result<(), SocketError> {
    let handle = fs::OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC).bits())
        .open(dir)
        .map_err(|_| SocketError::Directory)?;
    let meta = handle.metadata().map_err(|_| SocketError::Directory)?;
    if !meta.is_dir() || meta.uid() != uid {
        return Err(SocketError::Directory);
    }
    if created && meta.permissions().mode() & 0o7777 != 0o700 {
        handle
            .set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(|_| SocketError::Io)?;
        return Ok(());
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(SocketError::Directory);
    }
    Ok(())
}

/// Binds `dir/push.sock` (`0600`) in the private directory `dir` (created `0700` when
/// absent). A stale socket of the uid is replaced; any other file or a symlink is an error
/// and stays untouched.
///
/// # Errors
/// [`SocketError`] naming the refused step.
pub fn bind_socket(dir: &Path, uid: u32) -> Result<UnixListener, SocketError> {
    let created = match fs::symlink_metadata(dir) {
        Ok(_) => false,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(dir)
                .map_err(|_| SocketError::Io)?;
            true
        }
        Err(_) => return Err(SocketError::Directory),
    };
    checked_dir(dir, uid, created)?;
    let path = dir.join(PUSH_SOCKET_FILE_NAME);
    match fs::symlink_metadata(&path) {
        Ok(meta) => {
            if meta.file_type().is_socket() && meta.uid() == uid {
                fs::remove_file(&path).map_err(|_| SocketError::Io)?;
            } else {
                return Err(SocketError::Occupied);
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(_) => return Err(SocketError::Io),
    }
    let listener = UnixListener::bind(&path).map_err(|_| SocketError::Io)?;
    if fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).is_err() {
        let _ = fs::remove_file(&path);
        return Err(SocketError::Io);
    }
    Ok(listener)
}
