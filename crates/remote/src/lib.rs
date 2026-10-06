//! soos remote companion (`soos-remote`, GitHub #339).
//!
//! A user-level service that serves the owner's phone a real-time lock status page and a
//! remote lock action over a `0600` Unix socket proxied by `tailscale serve`. It never runs as
//! root, never opens a network socket, never unlocks anything, and treats every logind
//! failure as `unavailable`.
//!
//! This file is the single source of the crate constants (architect spec §3).

#![forbid(unsafe_code)]

pub mod assets;
pub mod config;
pub mod http;
pub mod identity;
pub mod logind;
pub mod routes;
pub mod server;
pub mod session;
pub mod socket;
pub mod status;

/// Largest configuration file accepted (bytes); larger → `ConfigError::TooLarge`.
pub const MAX_CONFIG_BYTES: usize = 16_384;
/// Largest `allowed_logins` list; more → `ConfigError::TooManyLogins`.
pub const MAX_ALLOWED_LOGINS: usize = 8;
/// Longest Tailscale login accepted (bytes).
pub const MAX_LOGIN_LEN: usize = 254;
/// Default logind polling interval while a stream is open (ms).
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 1000;
/// Smallest `poll_interval_ms` accepted.
pub const MIN_POLL_INTERVAL_MS: u64 = 250;
/// Largest `poll_interval_ms` accepted.
pub const MAX_POLL_INTERVAL_MS: u64 = 10_000;
/// Longest socket path (bytes): `sun_path` holds 108 bytes including the NUL.
pub const MAX_SOCKET_PATH_LEN: usize = 107;
/// Directory under `$XDG_RUNTIME_DIR` that holds the socket.
pub const SOCKET_DIR_NAME: &str = "soos-remote";
/// Socket file name.
pub const SOCKET_FILE_NAME: &str = "remote.sock";
/// Concurrent connections; further accepted streams are dropped at once.
pub const MAX_CONNECTIONS: usize = 16;
/// Concurrent `/api/events` streams; further ones get `503`.
pub const MAX_SSE_STREAMS: usize = 4;
/// Largest request head (bytes); larger → `431`.
pub const MAX_REQUEST_HEAD_BYTES: usize = 8192;
/// Most request headers; more → `431`.
pub const MAX_HEADERS: usize = 32;
/// Longest request path without query (bytes); longer → `414`.
pub const MAX_PATH_LEN: usize = 256;
/// Deadline for a complete request head; exceeded → connection closed without response.
pub const REQUEST_HEAD_TIMEOUT_MS: u64 = 5000;
/// Deadline for writing a response or an SSE event; exceeded → connection dropped.
pub const RESPONSE_WRITE_TIMEOUT_MS: u64 = 2000;
/// Interval at which an open stream re-sends the newest reading.
pub const SSE_KEEPALIVE_MS: u64 = 15_000;
/// Longest stream lifetime (30 min); the browser reconnects.
pub const MAX_SSE_STREAM_MS: u64 = 1_800_000;
/// Minimum interval between two accepted lock requests; sooner → `429`.
pub const MIN_LOCK_INTERVAL_MS: u64 = 2000;
/// Bound of every logind method call.
pub const DBUS_CALL_TIMEOUT_MS: u64 = 500;
/// Bound of the system bus connection.
pub const DBUS_CONNECT_TIMEOUT_MS: u64 = 1000;
/// Most sessions `ListSessions` may return.
pub const MAX_LISTED_SESSIONS: usize = 256;
/// Most sessions of the own uid.
pub const MAX_OWN_SESSIONS: usize = 16;
/// Longest logind session id accepted.
pub const MAX_SESSION_ID_LEN: usize = 64;
/// Longest logind error name kept for logging (truncated on a char boundary).
pub const MAX_LOGIND_ERROR_LEN: usize = 256;
/// Pinned system bus address (never derived from the environment).
pub const SYSTEM_BUS_ADDRESS: &str = "unix:path=/run/dbus/system_bus_socket";
/// Bound of one whole snapshot (connect, list, every `GetAll`).
pub const SNAPSHOT_DEADLINE_MS: u64 = 1500;
/// Bound of one whole lock flow (snapshot plus `LockSession`).
pub const LOCK_FLOW_DEADLINE_MS: u64 = 2000;
/// Largest `allowed_hosts` list.
pub const MAX_ALLOWED_HOSTS: usize = 4;
/// Longest host name accepted (bytes).
pub const MAX_HOST_LEN: usize = 253;
/// Suffix every implicit allowed host must carry.
pub const TS_NET_SUFFIX: &str = ".ts.net";
/// Exit status for configuration and root-refusal errors (`EX_CONFIG`, not restarted).
pub const EXIT_CONFIG: u8 = 78;
/// Exit status for runtime failures (restarted by systemd).
pub const EXIT_RUNTIME: u8 = 1;
/// Identity header set by Tailscale Serve (lowercased name).
pub const IDENTITY_HEADER: &str = "tailscale-user-login";
/// CSRF action header required by `POST /api/lock` (lowercased name).
pub const ACTION_HEADER: &str = "x-soos-action";
/// Required value of [`ACTION_HEADER`].
pub const ACTION_LOCK: &str = "lock";
/// Header through which `tailscale serve` forwards the original `*.ts.net` name (D5a′,
/// spec §13.2; lowercased name). When present it alone decides the effective host.
pub const FORWARDED_HOST_HEADER: &str = "x-forwarded-host";
/// Header through which `tailscale serve` forwards the client-facing scheme (D5a′;
/// lowercased name). Mandatory with [`FORWARDED_HOST_HEADER`], optional otherwise; when
/// present it must occur once and equal [`FORWARDED_PROTO_HTTPS`].
pub const FORWARDED_PROTO_HEADER: &str = "x-forwarded-proto";
/// The only accepted [`FORWARDED_PROTO_HEADER`] value (compared ASCII-case-insensitively).
pub const FORWARDED_PROTO_HTTPS: &str = "https";
