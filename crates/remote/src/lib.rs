//! soos remote companion (`soos-remote`, GitHub #339).
//!
//! A user-level service that serves the owner's phone a real-time lock status page, a remote
//! lock action and, when `allow_unlock = true`, a remote unlock action (ADR 2026-10-06) over a
//! `0600` Unix socket proxied by `tailscale serve` (tailnet) and, when `allow_funnel = true`,
//! by `tailscale funnel` (public HTTPS). Every unlock needs a fresh passkey assertion with user
//! verification; Funnel callers need a passkey login first (ADR 2026-10-06 "Tailscale Funnel
//! Access and In-House Passkey Authentication for `soos-remote`"). It never runs as root,
//! never opens a network socket, and treats every logind failure as `unavailable`.
//!
//! When `password_alerts = true` it also follows the system journal (a `journalctl` child
//! process) and shows the owner failed-password attempts made on the PC: time, source class,
//! account class, kind and count only, never the typed text (ADR 2026-10-06 "Failed-Password
//! Alerts in `soos-remote` From the System Journal").
//!
//! When `push_notifications = true` it also sends each new failed-password summary to the
//! owner's home-screen web app as a standard, end-to-end encrypted Web Push notification
//! (VAPID, RFC 8291). All keys and all cryptography stay here; the outbound HTTPS request is
//! made by the separate sandboxed `soos-push-sender` unit reached over a `0600` Unix socket,
//! so this service itself still opens no network socket (ADR 2026-10-06 "Web Push
//! Notifications for Failed-Password Alerts Through a Separate Sender Unit").
//!
//! This file is the single source of the crate constants (architect spec §3).

#![forbid(unsafe_code)]

pub mod alerts;
pub mod assets;
pub mod audit;
pub mod auth;
pub mod challenge;
pub mod config;
pub mod credentials;
pub mod enroll;
pub mod http;
pub mod identity;
pub mod journal;
pub mod logind;
pub mod push;
pub mod routes;
pub mod server;
pub mod session;
pub mod socket;
pub mod status;
pub mod webauthn;
pub mod webpush;
pub mod websession;

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
/// Required value of [`ACTION_HEADER`] on `POST /api/lock`.
pub const ACTION_LOCK: &str = "lock";
/// Required value of [`ACTION_HEADER`] on `POST /api/unlock` (ADR 2026-10-06).
pub const ACTION_UNLOCK: &str = "unlock";
/// Minimum interval between two accepted unlock requests; sooner → `429`. Independent of
/// [`MIN_LOCK_INTERVAL_MS`].
pub const MIN_UNLOCK_INTERVAL_MS: u64 = 2000;
/// Bound of one whole unlock flow (snapshot plus `UnlockSession`).
pub const UNLOCK_FLOW_DEADLINE_MS: u64 = 2000;
/// Header through which `tailscale serve` forwards the original `*.ts.net` name (D5a′,
/// spec §13.2; lowercased name). When present it alone decides the effective host.
pub const FORWARDED_HOST_HEADER: &str = "x-forwarded-host";
/// Header through which `tailscale serve` forwards the client-facing scheme (D5a′;
/// lowercased name). Mandatory with [`FORWARDED_HOST_HEADER`], optional otherwise; when
/// present it must occur once and equal [`FORWARDED_PROTO_HTTPS`].
pub const FORWARDED_PROTO_HEADER: &str = "x-forwarded-proto";
/// The only accepted [`FORWARDED_PROTO_HEADER`] value (compared ASCII-case-insensitively).
pub const FORWARDED_PROTO_HTTPS: &str = "https";

// ---------------------------------------------------------------------------------------
// Tailscale Funnel access and in-house passkey authentication (ADR 2026-10-06 "Tailscale
// Funnel Access and In-House Passkey Authentication for `soos-remote`", architect spec
// `AI/architect_spec_remote_passkey_funnel.md` §3.1).
// ---------------------------------------------------------------------------------------

/// Funnel marker header set by `tailscaled` (lowercased name).
pub const FUNNEL_HEADER: &str = "tailscale-funnel-request";
/// The only accepted Funnel marker value.
pub const FUNNEL_HEADER_VALUE: &str = "?1";
/// `X-Forwarded-For` (lowercased name); read only by `identity::client_hint`.
pub const FORWARDED_FOR_HEADER: &str = "x-forwarded-for";
/// Largest request body on a body route.
pub const MAX_AUTH_BODY_BYTES: usize = 8192;
/// Deadline for reading a body, from head completion.
pub const BODY_READ_TIMEOUT_MS: u64 = 5000;
/// Data chunks of a chunked body.
pub const MAX_BODY_CHUNKS: usize = 64;
/// Hex digits of one chunk-size line.
pub const MAX_CHUNK_SIZE_DIGITS: usize = 8;
/// Classified Funnel requests in flight.
pub const MAX_FUNNEL_CONNECTIONS: usize = 8;
/// Funnel requests in flight without a valid web session.
pub const MAX_ANONYMOUS_FUNNEL_CONNECTIONS: usize = 4;
/// Concurrent anonymous body reads.
pub const MAX_ANONYMOUS_BODY_READS: usize = 2;
/// Concurrent anonymous body reads per client hint.
pub const MAX_ANONYMOUS_BODY_READS_PER_HINT: usize = 1;
/// Open SSE streams of Funnel session holders.
pub const MAX_FUNNEL_SSE_STREAMS: usize = 2;
/// Anonymous limiter buckets kept in memory.
pub const MAX_CLIENT_HINTS: usize = 64;
/// Pending anonymous Funnel login challenges.
pub const MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES: usize = 16;
/// Pending login challenges per client hint.
pub const MAX_LOGIN_CHALLENGES_PER_HINT: usize = 2;
/// Decoded `clientDataJSON` bound.
pub const MAX_CLIENT_DATA_JSON_BYTES: usize = 1024;
/// Decoded `attestationObject` bound.
pub const MAX_ATTESTATION_OBJECT_BYTES: usize = 2048;
/// Exact assertion `authenticatorData` length.
pub const ASSERTION_AUTH_DATA_LEN: usize = 37;
/// DER ECDSA signature bound.
pub const MAX_SIGNATURE_BYTES: usize = 72;
/// Credential id bound (WebAuthn L3 §6.5.1).
pub const MAX_CREDENTIAL_ID_BYTES: usize = 1023;
/// Exact user handle length.
pub const USER_HANDLE_BYTES: usize = 16;
/// Challenge length.
pub const CHALLENGE_BYTES: usize = 32;
/// Challenge lifetime.
pub const CHALLENGE_TTL_MS: u64 = 120_000;
/// `timeout` sent to the client.
pub const WEBAUTHN_TIMEOUT_MS: u64 = 120_000;
/// Pending challenges per authenticated pool.
pub const MAX_PENDING_CHALLENGES: usize = 4;
/// Stored passkeys.
pub const MAX_PASSKEYS: usize = 4;
/// Credential store file bound.
pub const MAX_CREDENTIAL_STORE_BYTES: usize = 16_384;
/// Default credential store file name (sibling of `remote.toml`).
pub const CREDENTIALS_FILE_NAME: &str = "remote-passkeys.json";
/// Explicit `credentials_path` bound.
pub const MAX_CREDENTIALS_PATH_LEN: usize = 4096;
/// Live web sessions.
pub const MAX_WEB_SESSIONS: usize = 4;
/// Web session idle lifetime.
pub const WEB_SESSION_IDLE_MS: u64 = 900_000;
/// Web session absolute lifetime (also the cookie `Max-Age`).
pub const WEB_SESSION_ABSOLUTE_MS: u64 = 28_800_000;
/// Session token length.
pub const SESSION_TOKEN_BYTES: usize = 32;
/// Session cookie name.
pub const SESSION_COOKIE_NAME: &str = "__Host-soos_session";
/// Session cookie attributes (never a `Domain`).
pub const SESSION_COOKIE_ATTRIBUTES: &str = "Path=/; Secure; HttpOnly; SameSite=Strict";
/// Failed verifications per limiter key per window.
pub const MAX_AUTH_FAILURES: u32 = 5;
/// Failure window.
pub const AUTH_FAILURE_WINDOW_MS: u64 = 300_000;
/// Challenge issuances per limiter key per window.
pub const MAX_OPTIONS_PER_WINDOW: u32 = 10;
/// Options window.
pub const OPTIONS_WINDOW_MS: u64 = 60_000;
/// Enrollment code symbols.
pub const ENROLL_CODE_LEN: usize = 10;
/// Enrollment code alphabet (Crockford base32).
pub const ENROLL_CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Enrollment code lifetime (s).
pub const ENROLL_CODE_TTL_S: u64 = 300;
/// Wrong codes against one code file.
pub const MAX_ENROLL_CODE_ATTEMPTS: u32 = 3;
/// Enrollment code file name (socket directory).
pub const ENROLL_CODE_FILE_NAME: &str = "enroll-code";
/// Enrollment code file bound.
pub const MAX_ENROLL_CODE_FILE_BYTES: usize = 256;
/// Bounded store lock acquisition.
pub const STORE_LOCK_TIMEOUT_MS: u64 = 500;
/// The only accepted COSE algorithm (ES256).
pub const COSE_ALG_ES256: i64 = -7;
/// `rp.name`, `user.name` and `user.displayName`.
pub const RP_NAME: &str = "soos";
/// `X-Soos-Action` of `POST /api/auth/unlock/options`.
pub const ACTION_UNLOCK_OPTIONS: &str = "unlock-options";
/// `X-Soos-Action` of `POST /api/auth/login/options`.
pub const ACTION_LOGIN_OPTIONS: &str = "login-options";
/// `X-Soos-Action` of `POST /api/auth/login/verify`.
pub const ACTION_LOGIN: &str = "login";
/// `X-Soos-Action` of `POST /api/auth/logout`.
pub const ACTION_LOGOUT: &str = "logout";
/// `X-Soos-Action` of `POST /api/auth/register/options`.
pub const ACTION_REGISTER_OPTIONS: &str = "register-options";
/// `X-Soos-Action` of `POST /api/auth/register/verify`.
pub const ACTION_REGISTER: &str = "register";
/// Longest lingering close (ms) of a refused Funnel request or of a refusal before the
/// request is classified (head errors, `421`, classification `403`): class permits are
/// released before it, so a client keeping a refused socket open holds no slot for longer.
pub const FUNNEL_REFUSAL_LINGER_MS: u64 = 100;

// Compile-time relations of spec §3.1.
const _: () = assert!(WEBAUTHN_TIMEOUT_MS == CHALLENGE_TTL_MS);
const _: () = assert!(
    MAX_AUTH_BODY_BYTES
        >= 4 * MAX_ATTESTATION_OBJECT_BYTES / 3
            + 4 * MAX_CREDENTIAL_ID_BYTES / 3
            + 4 * MAX_CLIENT_DATA_JSON_BYTES / 3
            + 256
);
const _: () = assert!(MAX_SSE_STREAMS < MAX_CONNECTIONS);
const _: () = assert!(MAX_ANONYMOUS_FUNNEL_CONNECTIONS < MAX_FUNNEL_CONNECTIONS);
const _: () = assert!(MAX_FUNNEL_CONNECTIONS < MAX_CONNECTIONS);
const _: () = assert!(MAX_CONNECTIONS - MAX_FUNNEL_CONNECTIONS >= 8);
const _: () = assert!(MAX_ANONYMOUS_BODY_READS < MAX_ANONYMOUS_FUNNEL_CONNECTIONS);
const _: () = assert!(MAX_ANONYMOUS_BODY_READS_PER_HINT <= MAX_ANONYMOUS_BODY_READS);
const _: () = assert!(MAX_FUNNEL_SSE_STREAMS < MAX_SSE_STREAMS);
const _: () = assert!(MAX_LOGIN_CHALLENGES_PER_HINT < MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES);
const _: () = assert!(FUNNEL_REFUSAL_LINGER_MS < RESPONSE_WRITE_TIMEOUT_MS);

// ---------------------------------------------------------------------------------------
// Failed-password alerts (ADR 2026-10-06 "Failed-Password Alerts in `soos-remote` From the
// System Journal", architect spec `AI/architect_spec_remote_auth_alerts.md` §3.1).
// ---------------------------------------------------------------------------------------

/// Longest journal line read (bytes); longer → `LineRead::Overlong`, skipped unparsed.
pub const MAX_JOURNAL_LINE_BYTES: usize = 24_576;
/// Longest `MESSAGE` field accepted (bytes); longer → `EntryError::Shape`.
pub const MAX_MESSAGE_BYTES: usize = 4096;
/// Longest value of any other read journal field (bytes); longer → `EntryError::Shape`.
pub const MAX_FIELD_BYTES: usize = 4096;
/// Longest journal cursor kept (bytes); longer → not kept.
pub const MAX_CURSOR_LEN: usize = 256;
/// Bound of the journal access probe (ms); exceeded → `JournalError::ProbeTimeout`.
pub const JOURNAL_PROBE_TIMEOUT_MS: u64 = 2000;
/// First restart backoff of the journal follower (ms).
pub const JOURNAL_RESTART_MIN_MS: u64 = 1000;
/// Cap of the doubling restart backoff (ms).
pub const JOURNAL_RESTART_MAX_MS: u64 = 60_000;
/// A follower that ran at least this long resets the backoff to the minimum (ms).
pub const JOURNAL_STABLE_RUN_MS: u64 = 60_000;
/// Lines processed between two back-pressure pauses.
pub const JOURNAL_LINES_PER_BATCH: usize = 256;
/// Back-pressure pause after `JOURNAL_LINES_PER_BATCH` lines (ms).
pub const JOURNAL_BATCH_PAUSE_MS: u64 = 50;
/// History rebuilt from the journal at every start (s before now).
pub const HISTORY_REBUILD_WINDOW_S: u64 = 86_400;
/// A password check and a `pam_unix` failure of the same side and account within this
/// window are one attempt (µs).
pub const PAIR_WINDOW_US: u64 = 2_000_000;
/// A helper-only check inherits the class of an anchor at most this old (µs).
pub const ANCHOR_WINDOW_US: u64 = 3_600_000_000;
/// Pending (unpaired) password checks kept; one more resolves the oldest at once.
pub const MAX_PENDING_CHECKS: usize = 16;
/// Recent `pam_unix` failures kept for pairing; the oldest is evicted.
pub const MAX_RECENT_FAILURES: usize = 16;
/// Alert records kept in memory; the oldest is evicted.
pub const MAX_ALERT_HISTORY: usize = 32;
/// Attempts of the same class, account, kind and acknowledgement state within this window
/// join one record (µs).
pub const ALERT_COALESCE_WINDOW_US: u64 = 60_000_000;
/// At most one `alerts` SSE event per stream per this interval (ms).
pub const ALERT_EVENT_MIN_INTERVAL_MS: u64 = 1000;
/// Minimum interval between two acknowledgement requests that reach the rate gate (ms).
pub const MIN_ALERT_ACK_INTERVAL_MS: u64 = 1000;
/// Largest acknowledgement file accepted (bytes); larger → treated as invalid (marker 0).
pub const MAX_ALERTS_ACK_FILE_BYTES: usize = 256;
/// Acknowledgement file name (sibling of the credential store).
pub const ALERTS_ACK_FILE_NAME: &str = "remote-alerts.json";
/// Most `lock_screen_programs` entries.
pub const MAX_LOCK_SCREEN_PROGRAMS: usize = 4;
/// `X-Soos-Action` of `POST /api/alerts/ack`.
pub const ACTION_ALERTS_ACK: &str = "alerts-ack";
/// Header naming the per-start epoch of the acknowledged view (lowercased name).
pub const ALERTS_EPOCH_HEADER: &str = "x-soos-alerts-epoch";
/// Header naming the highest attempt seq of the acknowledged view (lowercased name).
pub const ALERTS_THROUGH_HEADER: &str = "x-soos-alerts-through";
/// Rendering of the epoch: exactly this many lowercase hex characters.
pub const ALERTS_EPOCH_HEX_LEN: usize = 16;
/// Most ASCII digits of the `through` header value.
pub const MAX_ALERTS_THROUGH_DIGITS: usize = 20;
/// Bytes of one read of the bounded journal line reader.
pub const JOURNAL_READ_CHUNK_BYTES: usize = 4096;
/// Idle tick of the follower: pending-check expiry with the wall clock and catch-up (ms).
pub const JOURNAL_IDLE_TICK_MS: u64 = 2000;
/// At most one acknowledgement-file write per this interval caused by a marker lowering
/// (ms); an acknowledgement writes at once.
pub const ALERTS_MARKER_FLUSH_MIN_INTERVAL_MS: u64 = 1000;

// Compile-time relations of the alerts spec §3.1.
const _: () = assert!(MAX_MESSAGE_BYTES * 4 <= MAX_JOURNAL_LINE_BYTES);
const _: () = assert!(JOURNAL_RESTART_MIN_MS < JOURNAL_RESTART_MAX_MS);
const _: () = assert!(PAIR_WINDOW_US < ALERT_COALESCE_WINDOW_US);
const _: () = assert!(ALERT_COALESCE_WINDOW_US < ANCHOR_WINDOW_US);
const _: () = assert!(MAX_CURSOR_LEN <= MAX_FIELD_BYTES);
const _: () = assert!(JOURNAL_IDLE_TICK_MS * 1000 == PAIR_WINDOW_US);
const _: () = assert!(JOURNAL_BATCH_PAUSE_MS < JOURNAL_IDLE_TICK_MS);
const _: () = assert!(JOURNAL_READ_CHUNK_BYTES <= MAX_JOURNAL_LINE_BYTES);
const _: () = assert!(ALERTS_EPOCH_HEX_LEN == 16);

// ---------------------------------------------------------------------------------------
// Web Push (architect spec `AI/architect_spec_remote_web_push.md` §3.1).
// ---------------------------------------------------------------------------------------

/// Stored push subscriptions; a further distinct endpoint is refused, never evicted.
pub const MAX_PUSH_SUBSCRIPTIONS: usize = 4;
/// Largest push store file.
pub const MAX_PUSH_STORE_BYTES: usize = 16_384;
/// Push store file name (sibling of the credential store).
pub const PUSH_STORE_FILE_NAME: &str = "remote-push.json";
/// Largest subscribe or unsubscribe body.
pub const MAX_PUSH_SUBSCRIBE_BODY_BYTES: usize = 2048;
/// Largest notification payload (plaintext JSON).
pub const MAX_PUSH_PLAINTEXT_BYTES: usize = 1024;
/// RFC 8188 record size of the encrypted body.
pub const PUSH_RECORD_SIZE: u32 = 4096;
/// The first send after a quiet period waits this long.
pub const PUSH_COALESCE_MS: u64 = 3000;
/// Spacing between two alert notifications.
pub const PUSH_MIN_INTERVAL_MS: u64 = 30_000;
/// Alert notifications per rolling hour.
pub const PUSH_MAX_PER_HOUR: u32 = 20;
/// A live attempt older than this when recorded is not pushed.
pub const PUSH_MAX_ATTEMPT_AGE_MS: u64 = 300_000;
/// Delays of the retries of one subscription for one message.
pub const PUSH_RETRY_DELAYS_MS: [u64; 2] = [5_000, 30_000];
/// `TTL` header of every notification.
pub const PUSH_TTL_S: u32 = 43_200;
/// `Topic` of alert summaries (an offline phone receives only the newest). ASCII letters and
/// digits only: `web.push.apple.com` answered `400 BadWebPushTopic` to a `-` (observed on the
/// owner's iPhone, 2026-10-06), although RFC 8030 allows the base64url alphabet.
pub const PUSH_TOPIC: &str = "soosalerts";
/// `Topic` of the test notification (never replaces an undelivered alert); same alphabet.
pub const PUSH_TEST_TOPIC: &str = "soostest";
/// `Urgency` of every notification.
pub const PUSH_URGENCY: soos_push_protocol::Urgency = soos_push_protocol::Urgency::High;
/// Gate of `POST /api/push/test`.
pub const PUSH_TEST_MIN_INTERVAL_MS: u64 = 10_000;
/// Shared gate of subscribe and unsubscribe.
pub const PUSH_ROUTE_MIN_INTERVAL_MS: u64 = 1000;
/// Bound of one whole exchange with the sender (connect, write, wait, read).
pub const PUSH_EXCHANGE_TIMEOUT_MS: u64 = 18_000;
/// Bound of the connection to the sender socket.
pub const PUSH_CONNECT_UNIX_TIMEOUT_MS: u64 = 1000;
/// VAPID JWT lifetime (`exp = now + 12 h`, below Apple's 24 h).
pub const VAPID_JWT_LIFETIME_S: u64 = 43_200;
/// A cached VAPID JWT is reused for at most this long.
pub const VAPID_JWT_REUSE_S: u64 = 3600;
/// Longest `vapid_subject`.
pub const MAX_VAPID_SUBJECT_LEN: usize = 256;
/// A Unix clock below this is implausible: no JWT, nothing sent.
pub const MIN_PLAUSIBLE_UNIX_S: u64 = 1_700_000_000;
/// `X-Soos-Action` of `POST /api/push/subscribe`.
pub const ACTION_PUSH_SUBSCRIBE: &str = "push-subscribe";
/// `X-Soos-Action` of `POST /api/push/unsubscribe`.
pub const ACTION_PUSH_UNSUBSCRIBE: &str = "push-unsubscribe";
/// `X-Soos-Action` of `POST /api/push/test`.
pub const ACTION_PUSH_TEST: &str = "push-test";

/// Byte equality usable in constant assertions.
const fn const_bytes_eq(a: &[u8], b: &[u8]) -> bool {
    match (a, b) {
        ([], []) => true,
        ([x, rest_a @ ..], [y, rest_b @ ..]) => *x == *y && const_bytes_eq(rest_a, rest_b),
        _ => false,
    }
}

const _: () = assert!(PUSH_COALESCE_MS < PUSH_MIN_INTERVAL_MS);
const _: () = assert!(PUSH_MAX_PER_HOUR as u64 * PUSH_MIN_INTERVAL_MS <= 3_600_000);
const _: () = assert!(MAX_PUSH_SUBSCRIBE_BODY_BYTES <= MAX_AUTH_BODY_BYTES);
const _: () =
    assert!(MAX_PUSH_PLAINTEXT_BYTES + 1 + 16 + 86 <= soos_push_protocol::MAX_PUSH_BODY_BYTES);
const _: () = assert!(PUSH_RECORD_SIZE as usize > MAX_PUSH_PLAINTEXT_BYTES + 17);
const _: () = assert!(
    PUSH_EXCHANGE_TIMEOUT_MS
        > PUSH_CONNECT_UNIX_TIMEOUT_MS
            + 3 * soos_push_protocol::PUSH_FRAME_IO_TIMEOUT_MS
            + soos_push_protocol::PUSH_SEND_TIMEOUT_MS
);
const _: () = assert!(!const_bytes_eq(
    PUSH_TOPIC.as_bytes(),
    PUSH_TEST_TOPIC.as_bytes()
));
const _: () = assert!(PUSH_TOPIC.len() <= soos_push_protocol::MAX_TOPIC_LEN);
const _: () = assert!(is_ascii_alphanumeric_topic(PUSH_TOPIC.as_bytes()));
const _: () = assert!(is_ascii_alphanumeric_topic(PUSH_TEST_TOPIC.as_bytes()));

/// Compile-time check that a `Topic` uses only ASCII letters and digits (Apple's push
/// service refused `-` with `400 BadWebPushTopic`; `_` is excluded as a precaution).
const fn is_ascii_alphanumeric_topic(bytes: &[u8]) -> bool {
    !bytes.is_empty() && all_ascii_alphanumeric(bytes)
}

/// Every byte is an ASCII letter or digit (an empty slice is `true`).
const fn all_ascii_alphanumeric(bytes: &[u8]) -> bool {
    match bytes {
        [] => true,
        [first, rest @ ..] => first.is_ascii_alphanumeric() && all_ascii_alphanumeric(rest),
    }
}

const _: () = assert!(PUSH_TEST_TOPIC.len() <= soos_push_protocol::MAX_TOPIC_LEN);
const _: () = assert!(VAPID_JWT_REUSE_S < VAPID_JWT_LIFETIME_S);
const _: () = assert!(PUSH_TTL_S <= soos_push_protocol::MAX_PUSH_TTL_S);
const _: () = assert!(MAX_PUSH_SUBSCRIPTIONS <= MAX_PASSKEYS);
