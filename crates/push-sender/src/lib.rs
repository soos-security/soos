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
    PUSH_SYSTEM_INSTANCE_ARG,
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

// ---------------------------------------------------------------------------------------
// Start-up sandbox self-check (ADR 2026-10-12 "soos-remote Security Hardening", S-7)
// ---------------------------------------------------------------------------------------

/// Process mount table read by the start-up sandbox check.
pub const SANDBOX_MOUNTINFO_PATH: &str = "/proc/self/mountinfo";
/// Larger mount tables fail closed (the owner host: ~16 KiB).
pub const MAX_MOUNTINFO_BYTES: usize = 1_048_576;
/// Directory the unit makes inaccessible (`InaccessiblePaths=`).
pub const SANDBOX_HIDDEN_LOG_DIR: &str = "/var/log";
/// Path of the binary relative to the home directory, exactly as in the unit's
/// `ExecStart=%h/.local/bin/soos-push-sender`.
pub const SENDER_EXE_HOME_RELATIVE: &str = ".local/bin/soos-push-sender";

/// Mount point the unit covers with a read-only tmpfs (`TemporaryFileSystem=/run:ro`).
const SANDBOX_RUN_DIR: &str = "/run";
/// Filesystem type of the hiding mounts.
const TMPFS: &str = "tmpfs";
/// Per-mount option of a read-only mount.
const READ_ONLY_OPTION: &str = "ro";

/// Why the sandbox was not detected. Fixed texts: no path is echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    /// The mount table could not be read, was too large or was not UTF-8.
    #[error("mount table unreadable")]
    MountInfoUnreadable,
    /// The home directory could not be derived from the running binary's path.
    #[error("home not resolved")]
    HomeUnresolved,
    /// `/run` is not a read-only tmpfs.
    #[error("/run not hidden")]
    RunVisible,
    /// The home directory is not under a read-only tmpfs, or is mounted back on top of it.
    #[error("home not hidden")]
    HomeVisible,
    /// The system logs directory is listable.
    #[error("system logs readable")]
    LogsReadable,
}

/// The home directory of the running binary: `exe` minus the exact trailing components of
/// [`SENDER_EXE_HOME_RELATIVE`] (compared component-wise). Never read from the environment.
///
/// # Errors
/// [`SandboxError::HomeUnresolved`] when `exe` does not end with those components, or the
/// remainder is relative, empty, `/` or contains a `..` component.
pub fn home_from_exe(exe: &Path) -> Result<std::path::PathBuf, SandboxError> {
    if !exe.ends_with(SENDER_EXE_HOME_RELATIVE) {
        return Err(SandboxError::HomeUnresolved);
    }
    let home = exe
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or(SandboxError::HomeUnresolved)?;
    let has_parent_dir = home
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir));
    if !home.is_absolute() || home.parent().is_none() || has_parent_dir {
        return Err(SandboxError::HomeUnresolved);
    }
    Ok(home.to_path_buf())
}

/// One parsed line of `/proc/self/mountinfo` (proc(5)).
struct MountEntry {
    id: u64,
    parent: u64,
    mount_point: std::path::PathBuf,
    read_only: bool,
    tmpfs: bool,
}

impl MountEntry {
    /// Whether this is a read-only tmpfs.
    const fn hides(&self) -> bool {
        self.read_only && self.tmpfs
    }
}

/// Octal-unescapes a mountinfo field (`\040`, `\011`, `\012`, `\134`, any `\ooo`).
fn unescape_mount_field(field: &str) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStringExt;

    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some((&first, tail)) = rest.split_first() {
        if first == b'\\' {
            if let Some(value) = tail.get(..3).and_then(octal_byte) {
                out.push(value);
                rest = tail.get(3..).unwrap_or_default();
                continue;
            }
        }
        out.push(first);
        rest = tail;
    }
    std::path::PathBuf::from(std::ffi::OsString::from_vec(out))
}

/// The byte of three octal digits, when they are octal and fit a byte.
fn octal_byte(digits: &[u8]) -> Option<u8> {
    let mut value: u32 = 0;
    for &d in digits {
        if !(b'0'..=b'7').contains(&d) {
            return None;
        }
        value = value
            .checked_mul(8)?
            .checked_add(u32::from(d.checked_sub(b'0')?))?;
    }
    u8::try_from(value).ok()
}

/// Parses one mountinfo line; malformed lines are `None` (skipped).
fn parse_mount_line(line: &str) -> Option<MountEntry> {
    let fields: Vec<&str> = line.split(' ').collect();
    let id = fields.first()?.parse::<u64>().ok()?;
    let parent = fields.get(1)?.parse::<u64>().ok()?;
    let mount_point = unescape_mount_field(fields.get(4)?);
    let options = fields.get(5)?;
    let separator = fields
        .iter()
        .skip(6)
        .position(|f| *f == "-")?
        .checked_add(6)?;
    let fstype = fields.get(separator.checked_add(1)?)?;
    Some(MountEntry {
        id,
        parent,
        mount_point,
        read_only: options.split(',').any(|o| o == READ_ONLY_OPTION),
        tmpfs: *fstype == TMPFS,
    })
}

/// Pure sandbox decision (spec §8.2). `mountinfo`: content of `/proc/self/mountinfo`;
/// `exe`: the running binary's path; `logs_listable`: whether [`SANDBOX_HIDDEN_LOG_DIR`] could
/// be listed. Rules, in the order of their errors: the home anchor resolves; some mount at
/// exactly `/run` is a read-only tmpfs; some read-only tmpfs whose mount point (not `/`)
/// equals or component-wise prefixes the home hides it, and no direct child mount of such a
/// tmpfs covers or lies inside the home except the exact binary bind; the logs are not
/// listable. Order-independent over lines; malformed lines are skipped.
///
/// # Errors
/// The first [`SandboxError`] rule that fails.
pub fn evaluate_sandbox(
    mountinfo: &str,
    exe: &Path,
    logs_listable: bool,
) -> Result<(), SandboxError> {
    let home = home_from_exe(exe)?;
    let entries: Vec<MountEntry> = mountinfo.lines().filter_map(parse_mount_line).collect();

    let run = Path::new(SANDBOX_RUN_DIR);
    if !entries.iter().any(|e| e.hides() && e.mount_point == run) {
        return Err(SandboxError::RunVisible);
    }

    let binary_bind = home.join(SENDER_EXE_HOME_RELATIVE);
    let covering: Vec<&MountEntry> = entries
        .iter()
        .filter(|e| {
            e.hides() && e.mount_point.parent().is_some() && home.starts_with(&e.mount_point)
        })
        .collect();
    if covering.is_empty() {
        return Err(SandboxError::HomeVisible);
    }
    let reexposed = covering.iter().any(|tmpfs| {
        entries.iter().any(|child| {
            child.parent == tmpfs.id
                && child.mount_point != binary_bind
                && (home.starts_with(&child.mount_point) || child.mount_point.starts_with(&home))
        })
    });
    if reexposed {
        return Err(SandboxError::HomeVisible);
    }

    if logs_listable {
        return Err(SandboxError::LogsReadable);
    }
    Ok(())
}

/// I/O wrapper of [`evaluate_sandbox`]: a bounded read of [`SANDBOX_MOUNTINFO_PATH`], the
/// running binary's path (`std::env::current_exe`, no environment variable) and whether
/// [`SANDBOX_HIDDEN_LOG_DIR`] can be opened as a directory (its entries are never read).
///
/// # Errors
/// [`SandboxError::MountInfoUnreadable`] when the mount table cannot be read, exceeds
/// [`MAX_MOUNTINFO_BYTES`] or is not UTF-8; [`SandboxError::HomeUnresolved`] when the binary
/// path is unavailable; otherwise the result of [`evaluate_sandbox`].
pub fn probe_sandbox() -> Result<(), SandboxError> {
    let limit = u64::try_from(MAX_MOUNTINFO_BYTES)
        .ok()
        .and_then(|max| max.checked_add(1))
        .ok_or(SandboxError::MountInfoUnreadable)?;
    let file =
        fs::File::open(SANDBOX_MOUNTINFO_PATH).map_err(|_| SandboxError::MountInfoUnreadable)?;
    let mut raw = Vec::new();
    file.take(limit)
        .read_to_end(&mut raw)
        .map_err(|_| SandboxError::MountInfoUnreadable)?;
    if raw.len() > MAX_MOUNTINFO_BYTES {
        return Err(SandboxError::MountInfoUnreadable);
    }
    let mountinfo = String::from_utf8(raw).map_err(|_| SandboxError::MountInfoUnreadable)?;
    let exe = std::env::current_exe().map_err(|_| SandboxError::HomeUnresolved)?;
    let logs_listable = fs::read_dir(SANDBOX_HIDDEN_LOG_DIR).is_ok();
    evaluate_sandbox(&mountinfo, &exe, logs_listable)
}

// ---------------------------------------------------------------------------------------
// System instance mode (ADR 2026-10-23 "Push Sender as a Sandboxed System Service")
// ---------------------------------------------------------------------------------------

/// How the sender runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// The legacy user unit.
    User,
    /// The system template unit for the desktop user `allowed_uid`.
    System {
        /// The only peer uid served.
        allowed_uid: u32,
    },
}

/// Refused system instance start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InstanceError {
    /// The arguments are not exactly `--system-instance <uid>` or nothing.
    #[error("push sender arguments refused")]
    Usage,
    /// Root, or the sender shares the instance uid.
    #[error("push sender instance identity refused")]
    Identity,
}

/// Longest decimal `u32`.
const MAX_UID_DIGITS: usize = 10;
/// Directories `ProtectHome=yes` makes inaccessible in the system unit: read-only binds of
/// systemd's inaccessible tmpfs directory (`/run/user` lies under the hidden `/run`).
const SYSTEM_HIDDEN_HOME_DIRS: [&str; 2] = ["/home", "/root"];

/// Parses the process arguments (without the program name): nothing is [`RunMode::User`];
/// exactly [`PUSH_SYSTEM_INSTANCE_ARG`] followed by a uid of 1 to 10 ASCII digits, without
/// sign or leading zero, other than `0` and `u32::MAX`, is [`RunMode::System`].
///
/// # Errors
/// [`InstanceError::Usage`] for anything else (non-UTF-8 included).
pub fn parse_run_mode<I: IntoIterator<Item = std::ffi::OsString>>(
    args: I,
) -> Result<RunMode, InstanceError> {
    let mut args = args.into_iter();
    let Some(flag) = args.next() else {
        return Ok(RunMode::User);
    };
    let Some(value) = args.next() else {
        return Err(InstanceError::Usage);
    };
    if args.next().is_some() || flag.as_os_str() != PUSH_SYSTEM_INSTANCE_ARG {
        return Err(InstanceError::Usage);
    }
    let text = value.to_str().ok_or(InstanceError::Usage)?;
    let bytes = text.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_UID_DIGITS
        || !bytes.iter().all(u8::is_ascii_digit)
        || bytes.first() == Some(&b'0')
    {
        return Err(InstanceError::Usage);
    }
    let allowed_uid = text.parse::<u32>().map_err(|_| InstanceError::Usage)?;
    if allowed_uid == u32::MAX {
        return Err(InstanceError::Usage);
    }
    Ok(RunMode::System { allowed_uid })
}

/// Refuses root (real or effective uid 0) and an instance uid equal to the sender's own real
/// or effective uid: the system instance runs as a dynamic uid, never as the desktop user.
///
/// # Errors
/// [`InstanceError::Identity`].
pub fn check_instance_identity(uid: u32, euid: u32, allowed_uid: u32) -> Result<(), InstanceError> {
    if uid == 0 || euid == 0 || uid == allowed_uid || euid == allowed_uid {
        return Err(InstanceError::Identity);
    }
    Ok(())
}

/// Pure sandbox decision of `packaging/soos-push-sender@.service` (spec §4.2). Rules, in the
/// order of their errors: some mount at exactly `/run` is a read-only tmpfs; for each of
/// `/home` and `/root`, some mount at exactly that path is a read-only tmpfs and no mount has
/// such a tmpfs as its parent (nothing mounted back); the logs are not listable.
/// Order-independent over lines; malformed lines are skipped.
///
/// # Errors
/// The first [`SandboxError`] rule that fails.
pub fn evaluate_system_sandbox(mountinfo: &str, logs_listable: bool) -> Result<(), SandboxError> {
    let entries: Vec<MountEntry> = mountinfo.lines().filter_map(parse_mount_line).collect();
    let run = Path::new(SANDBOX_RUN_DIR);
    if !entries.iter().any(|e| e.hides() && e.mount_point == run) {
        return Err(SandboxError::RunVisible);
    }
    for dir in SYSTEM_HIDDEN_HOME_DIRS {
        let path = Path::new(dir);
        let hiding: Vec<&MountEntry> = entries
            .iter()
            .filter(|e| e.hides() && e.mount_point == path)
            .collect();
        if hiding.is_empty() {
            return Err(SandboxError::HomeVisible);
        }
        let reexposed = hiding
            .iter()
            .any(|tmpfs| entries.iter().any(|child| child.parent == tmpfs.id));
        if reexposed {
            return Err(SandboxError::HomeVisible);
        }
    }
    if logs_listable {
        return Err(SandboxError::LogsReadable);
    }
    Ok(())
}

/// Bounded read of [`SANDBOX_MOUNTINFO_PATH`] (at most [`MAX_MOUNTINFO_BYTES`], UTF-8).
fn read_mount_table() -> Result<String, SandboxError> {
    let limit = u64::try_from(MAX_MOUNTINFO_BYTES)
        .ok()
        .and_then(|max| max.checked_add(1))
        .ok_or(SandboxError::MountInfoUnreadable)?;
    let file =
        fs::File::open(SANDBOX_MOUNTINFO_PATH).map_err(|_| SandboxError::MountInfoUnreadable)?;
    let mut raw = Vec::new();
    file.take(limit)
        .read_to_end(&mut raw)
        .map_err(|_| SandboxError::MountInfoUnreadable)?;
    if raw.len() > MAX_MOUNTINFO_BYTES {
        return Err(SandboxError::MountInfoUnreadable);
    }
    String::from_utf8(raw).map_err(|_| SandboxError::MountInfoUnreadable)
}

/// I/O wrapper of [`evaluate_system_sandbox`]: the bounded mount table and whether
/// [`SANDBOX_HIDDEN_LOG_DIR`] can be opened as a directory (its entries are never read). No
/// environment variable and no binary path are used.
///
/// # Errors
/// [`SandboxError::MountInfoUnreadable`], otherwise the result of [`evaluate_system_sandbox`].
pub fn probe_system_sandbox() -> Result<(), SandboxError> {
    let mountinfo = read_mount_table()?;
    let logs_listable = fs::read_dir(SANDBOX_HIDDEN_LOG_DIR).is_ok();
    evaluate_system_sandbox(&mountinfo, logs_listable)
}

/// The listening socket handed over by systemd (`StandardInput=socket`): accepted only when
/// `fd` is an `AF_UNIX` `SOCK_STREAM` socket in the listening state.
///
/// # Errors
/// [`SocketError::Io`] for any other descriptor (it is closed).
pub fn listener_from_fd(fd: std::os::fd::OwnedFd) -> Result<UnixListener, SocketError> {
    use nix::sys::socket::{getsockname, sockopt, SockType, SockaddrStorage};
    use std::os::fd::AsRawFd;

    let kind = getsockopt(&fd, sockopt::SockType).map_err(|_| SocketError::Io)?;
    if kind != SockType::Stream {
        return Err(SocketError::Io);
    }
    if !getsockopt(&fd, sockopt::AcceptConn).map_err(|_| SocketError::Io)? {
        return Err(SocketError::Io);
    }
    let local: SockaddrStorage = getsockname(fd.as_raw_fd()).map_err(|_| SocketError::Io)?;
    if local.as_unix_addr().is_none() {
        return Err(SocketError::Io);
    }
    Ok(UnixListener::from(fd))
}
