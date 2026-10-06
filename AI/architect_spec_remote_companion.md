# Architect Spec — GitHub #339: Remote Companion (`soos-remote`) for Real-Time Lock Status and Remote Lock

- **Branch**: `feat/remote-companion` (GitHub-only issue, not registered in `scripts/sync_issue.py`; commits carry `Refs #339`, never `Closes #339`: the branch is **not merged** until the owner says so, it is reviewed through a draft PR)
- **Base commit**: `222665f`
- **Revision**: 3 — revision 1 was evaluated `REVISION_REQUIRED` (findings F1–F12), revision 2 resolved them (*[R2: Fn]*) and was evaluated `REVISION_REQUIRED` (findings R2-1…R2-6, `AI/plan_evaluator_report.md`); revision 3 changes are marked *[R3: R2-n]*. Change logs in §11 and §12.
- **Owner decisions (2026-10-05, binding, not reopened here)**:
  1. Phone access from an iPhone, so a web page installable as a PWA, not a native app.
  2. No hosted website and no cloud relay: the server runs on the PC and is reached only through the owner's Tailscale tailnet (`tailscale serve` terminates HTTPS with the node's `*.ts.net` certificate).
  3. New crate and binary `soos-remote`, a **user-level** systemd service running as the session owner, never root; `soos-daemon`, `pam_soos.so` and the IPC protocol are untouched.
  4. Only the owner may use it.
  5. This issue is the low-risk scope: real-time session status and remote lock. Push notifications, live camera and remote unlock are out of scope (future issues, higher risk).
  6. Development branch only; draft PR for CI; no merge without an explicit owner go.

---

## 0. Evidence Gathered Before Designing

| Question | Evidence | Consequence |
|---|---|---|
| Does an existing tool already cover it? | KDE Connect iOS (v0.4.3): one-way "Run Command" only, no lock state; GSConnect: GNOME Shell only; lnxlink + Home Assistant: needs an always-on HA server plus an MQTT broker; HASS.Agent: Windows only; no maintained "logind web lock" project found. | A small dedicated component is justified. |
| Where does the lock state live? | systemd-logind `org.freedesktop.login1.Session` properties `LockedHint`, `IdleHint`, `IdleSinceHint`, `Active`, `State`, `Class`, `Remote`, `Seat`, `User` (systemd 262 man page; already read by `crates/daemon/src/presence/logind.rs`). | Read over the system bus with the same bounded `zbus` pattern as presence. |
| Who sets `LockedHint`? | GNOME, Plasma and niri set it natively; swaylock, swaylock-plugin and swayidle 1.9 never do. On the owner's driftwm host the locker wrapper calls `SetLockedHint` around the locker (verified on host 2026-10-05). | The status is only as truthful as the desktop's `LockedHint`; documented as a requirement, never guessed. |
| How does a remote lock reach the locker? | `Manager.LockSession(id)` / `Session.Lock()` only emit the `Lock` signal on the session; the desktop must listen (GNOME/Plasma natively; sway-family via `swayidle lock <cmd>`). Host check: `swayidle -w … lock ~/.config/driftwm/lock.sh` is running. | The lock result is observed through `LockedHint`, not assumed from the call's success. |
| Who may call `Lock`? | logind `bus_session_method_lock` verifies polkit action `org.freedesktop.login1.lock-sessions` with `good_user = session owner`: the session owner (and root) pass without a prompt. The **same action** also guards `Unlock`. | A user-level service needs no polkit rule. A system service would need a polkit grant that also authorizes **unlock**; on the owner's host any logind `Unlock` signal kills the locker (`swayidle unlock 'pkill -USR1 …'`). A user-level service gains no capability the owner's account does not already hold. |
| Can `tailscale serve` proxy to a Unix socket? | Host `tailscale serve --help` (1.102.4): "you can also specify a Unix domain socket (e.g., `unix:/tmp/myservice.sock`)"; `--bg` persists the config across reboots. | `soos-remote` opens **no TCP socket at all** (`RestrictAddressFamilies=AF_UNIX`). |
| Can the identity header be trusted? | Tailscale Serve sets `Tailscale-User-Login` / `Tailscale-User-Name` and strips client-supplied copies; the headers are absent for Funnel and for tagged source devices. On loopback TCP any local process could forge them. | Trust them only because the backend is a `0600` Unix socket owned by the owner: only `tailscaled` (root) and the owner can connect, and the owner forging their own login is no escalation. Missing header → refused. |
| iOS PWA behaviour | Safari standalone web apps support `EventSource` while in the foreground; iOS suspends backgrounded web apps and drops their connections within seconds. | The page re-reads the status and reopens the stream on every `visibilitychange` → visible and `pageshow`. No background channel in this issue. |
| Cost of an always-on server | A current-thread Tokio service with an idle Unix listener: a few MB RSS, ~0 % CPU. | Poll logind only while at least one live stream is open; zero D-Bus traffic when nobody watches. |
| HTTP stack dependencies | `Cargo.lock` already contains `httparse 1.10.1` and `http 1.5.0` (through `ureq`); `hyper`, `axum` and `tower` are absent. | Use `httparse` (MIT/Apache-2.0, no new version) with a minimal, bounded HTTP/1.1 responder; no `hyper`/`axum` (fewer new crates, no `multiple-versions` risk, every bound explicit). |

---

## 1. Scope & Blast Radius

### 1.1 Crates, modules and files

| Area | Path | Change |
|---|---|---|
| New crate | `crates/remote/` (package `soos-remote`, lib + bin `soos-remote`; manifest keys `version.workspace`, `edition.workspace`, `license.workspace`, `publish.workspace = true`, `[lints] workspace = true` *[R2: F9]*; dev-dependencies `tokio = { workspace = true, features = ["test-util"] }` and `tempfile` *[R2: F4]*) | `Cargo.toml`, `src/lib.rs`, `src/main.rs`, `src/config.rs`, `src/identity.rs`, `src/http.rs`, `src/routes.rs`, `src/session.rs`, `src/logind.rs`, `src/status.rs`, `src/server.rs`, `src/socket.rs`, `src/assets.rs`, `assets/index.html`, `assets/app.js`, `assets/style.css`, `assets/manifest.webmanifest`, `assets/icon.svg`, `assets/apple-touch-icon.png`, `tests/server_tests.rs`. |
| Workspace | `Cargo.toml` | `members += "crates/remote"`; `[workspace.dependencies]` += `httparse = "1.10"` and `soos-remote = { path = "crates/remote", version = "0.1.0" }`. `nix` gains no feature (`user` is already enabled). |
| Lockfile | `Cargo.lock` | No new external crate expected (all deps already locked); verified by `cargo deny --locked check`. |
| Invariants | `tests/invariants/src/lib.rs` | `"remote"` added to `test_business_crates_forbid_unsafe_code`; `mod remote_companion_contract;`. |
| Invariants | `tests/invariants/src/remote_companion_contract.rs` (new) | Static contracts of §8. |
| Invariants *[R2: F1]* | `tests/invariants/src/presence_unlock_contract.rs` (**existing test, contract migration**, §2.12) | `test_pau_zbus_is_used_only_by_the_daemon` allows exactly two manifests (`crates/daemon`, `crates/remote`) instead of one; every other check is kept. |
| Review tooling *[R2: F9]* | `scripts/candid_review.sh` | `remote` appended to `BUSINESS_CRATES` (layer-1 forbid-unsafe check). |
| Packaging | `packaging/soos-remote.service` (new, **user** unit) | §2.9. |
| Install | `scripts/install_remote.sh` (new, user-level, never `sudo`) | §2.10. |
| Docs | `Docs/REMOTE_COMPANION.md` (new), `Docs/README.md` (index line), `AI/ARCHITECTURE.md` (line 22 zbus scope, §8 tree and forbid-unsafe list, new §13 "Remote Companion") *[R2: F1, F9]*, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` (zbus line 120: daemon **and** soos-remote) *[R2: F1]*, `AI/MOCK_STRATEGY.md` (new "Remote Companion Doubles" section) *[R2: F9]*, `.claude/skills/dev-workflow/references/project-facts.md` (workspace map row, constants rows) *[R2: F9]*, `AI/DECISIONS.md` (ADR, §9), `AGENTS.md` (workspace tree line), `AI/VERIFICATION_MATRIX.md` (rows RMC1–RMCn), walkthrough `AI/walkthroughs/NN_remote_companion.md`. |

### 1.2 Not touched (explicitly)

- `crates/daemon`, `crates/pam`, `crates/protocol`, `crates/policy` and every other existing crate: no code change, no new dependency on `soos-remote`, no new IPC request kind.
- `scripts/install.sh`, `scripts/uninstall.sh`, `packaging/arch/PKGBUILD`, `packaging/debian/*`, `packaging/rpm/soos.spec`: the companion is a per-user, opt-in, development-stage component; system-wide distribution packaging is **deferred** to a follow-up issue opened once the owner approves the merge (recorded in the ADR, §9). `scripts/install_remote.sh` is the only installer.
- The owner's personal compositor and locker files (never in the repository).

### 1.3 Downstream consumers of changed public items

None: no existing public item changes. `soos-remote` is a leaf crate; no workspace crate may depend on it (static contract RMC-S4).

---

## 2. Design

### 2.1 Decisions

| ID | Decision | Rejected alternative |
|---|---|---|
| D1 Process | User-level systemd service (`systemctl --user`), the session owner's UID. `main` refuses to start when the real **or** effective UID is 0 (pure `check_not_root(uid, euid)`, exit code `EXIT_CONFIG`), before reading any configuration *[R2: F3]*. | System service + polkit grant (the grant also authorizes `Unlock`); a route inside `soos-daemon` (violates ADR 2026-09-12 "Zero network sockets" spirit and puts an HTTP parser in a root process). |
| D2 Transport | One `0600` Unix stream socket at `$XDG_RUNTIME_DIR/soos-remote/remote.sock` (parent dir `0700`), proxied by `tailscale serve --bg unix:<path>`. No `TcpListener`, no `std::net`, unit `RestrictAddressFamilies=AF_UNIX`. | Loopback TCP (identity headers forgeable by any local process); binding the tailnet IP (needs TLS in-process and LocalAPI whois). |
| D3 Identity | Every request (assets included) must carry exactly one `Tailscale-User-Login` header whose value is in `allowed_logins`; otherwise `403`. Empty or missing allowlist → the service refuses to start. | Passwords/tokens in the page (secrets on the phone, no gain over the tailnet identity for a lock-only scope). |
| D4 CSRF | `POST /api/lock` requires header `X-Soos-Action: lock` (non-simple → a cross-origin page cannot send it without a CORS preflight, which is never answered), `Sec-Fetch-Site` absent or `same-origin`, and `Origin` absent or equal to `https://<Host>`. | Cookies / tokens (no session state to protect). |
| D5a Host *[R2: F5]* | Every request needs exactly one `Host` header equal (ASCII case-insensitive, optional `:443` stripped) to one of `allowed_hosts`; when `allowed_hosts` is not configured, any syntactically valid DNS name ending in `.ts.net` is accepted. Anything else → `421`. Checked before identity. The docs forbid `tailscale serve --http` and `tailscale funnel` for this socket. Phase 4 verifies on the owner's host that `tailscale serve unix:` forwards the original `Host` (recorded in the walkthrough); if it does not, the design returns to the architect. | Accepting any Host (DNS rebinding would read the status if the socket were ever exposed over plain HTTP). |
| D5 HTTP | Minimal HTTP/1.1 over `httparse`: one request per connection, `Connection: close` on every response, no request body accepted, bounded head size, header count, head deadline, write deadline and concurrent connections. | `hyper`/`axum` (more supply chain for 3 routes). |
| D6 Real time *[R2: F2, F6; R3: R2-1, R2-4]* | `GET /api/events` Server-Sent Events, fed by one poller task and a `tokio::sync::watch::Sender<Option<Reading>>` (§2.6). The poller reads logind every `poll_interval_ms` **only while ≥ 1 stream is open** and publishes **every** read (`send_replace`, fresh `checked_unix_ms` and a strictly increasing `seq`), and resets the channel to `None` when the subscriber count drops to 0. A new stream never replays an older value: its first event is a **fresh `own_sessions` read made by the stream itself after accept** (under `SNAPSHOT_DEADLINE_MS`), stamped with the next `seq` from the same shared counter; afterwards it consumes channel values whose `seq` is greater than the last one it sent (a monotonic counter, never the wall clock, so an NTP step cannot silence it), and sends an event when the view (state, active, idle, idle_since, ignoring `checked_unix_ms`) differs from the last one sent; in addition, every `SSE_KEEPALIVE_MS` it re-sends the newest reading it holds (fresh `checked_unix_ms`), so the timestamp keeps advancing in a steady state. Each stream also watches its read half: EOF or an error ends the stream at once. The poller runs inside the server's `JoinSet`; if it ends, the server shuts down and the process exits `EXIT_RUNTIME`. A stream closes after `MAX_SSE_STREAM_MS` (the browser reconnects). The UI shows `Unreachable` when no event arrived for `STALE_UI_MS` (measured with the browser's own clock from event arrival, not by comparing `checked_unix_ms` with the phone's clock). | D-Bus `PropertiesChanged` match (adds a signal subscription and queue handling for at most 1 s of latency gain). |
| D7 Session choice | Own UID only; candidate = `Class == "user"`, explicit `Remote == false`, non-empty seat; prefer `Active`; ties broken by the smallest session ID under the order (byte length, then bytes), so `"9"` < `"10"` *[R2: F10]*. No candidate → `no_session`. | Any session of the UID (would lock an SSH or remote session). |
| D8 Fail safe status | Any logind failure yields `state = "unavailable"`; the service never reports `unlocked` unless a fresh read returned `LockedHint == false` for the selected session. | Keeping the last known state on error (stale "unlocked" is misleading). |
| D9 Lock | `POST /api/lock` → fresh snapshot → `Manager.LockSession(<id>)` (the whole flow — connect, list, per-session `GetAll`, lock call — runs under one `LOCK_FLOW_DEADLINE_MS`; every snapshot, whether from the poller, `/api/status` or a new stream, runs under one `SNAPSHOT_DEADLINE_MS`, mirroring the presence ADR amendment (ii); overrun → `Timeout` *[R2: F8]*) → `202 {"result":"lock_requested"}`. Success is confirmed by the UI through `LockedHint`, not by the call. One lock per `MIN_LOCK_INTERVAL_MS` (`429`). | Synchronous wait for `LockedHint` in the request (ties a connection to the locker's speed). |
| D10 No unlock | The crate never names `UnlockSession`, `Unlock` or `SetLockedHint` (static contract RMC-S3). | — |
| D11 Assets | Static files embedded with `include_str!` / `include_bytes!` (a 180×180 PNG `apple-touch-icon.png` for the iOS home screen, which ignores SVG icons *[R2: F12]*), served with a strict CSP (`default-src 'self'`); no inline script, no CDN, no npm build. | Inline `<script>` (needs `'unsafe-inline'`). |
| D12 Logging | `tracing` to the journal (`tracing-subscriber` fmt writer to stderr, captured by journald); startup errors go through `tracing::error!` too, so no `clippy::print_stderr` allow is needed in the crate *[R2: F11]*; never logs header values, logins, the `Host`, paths of requests other than the route enum, or response bodies. Denials log the reason class only. | — |

### 2.2 Module layout

```
crates/remote/
├── Cargo.toml
├── assets/{index.html, app.js, style.css, manifest.webmanifest, icon.svg, apple-touch-icon.png}
├── src/
│   ├── lib.rs        #![forbid(unsafe_code)]; pub mod …; constants (single source, §3)
│   ├── main.rs       #![forbid(unsafe_code)]; args, config load, socket prep, run server, SIGTERM/SIGINT
│   ├── config.rs     RemoteConfig, TailscaleLogin, file parsing + validation (pure + bounded read)
│   ├── identity.rs   authorize() (pure)
│   ├── http.rs       parse_request_head() (pure, httparse), Response builder, write helpers
│   ├── routes.rs     Route, route(), check_lock_csrf() (pure)
│   ├── session.rs    SessionProps, select_session(), status_from() (pure)
│   ├── logind.rs     SessionSource trait, ZbusSessionSource (bounded zbus calls)
│   ├── status.rs     SessionStatus, StatusView (serde JSON), poller
│   ├── socket.rs     prepare_socket_dir(), bind_listener()
│   ├── assets.rs     embedded assets table
│   └── server.rs     accept loop, connection limit, request handling, SSE, lock rate limit
└── tests/server_tests.rs   end-to-end over a temp Unix socket with a mock SessionSource
```

`main.rs` builds a **current-thread** Tokio runtime (`tokio::runtime::Builder::new_current_thread().enable_all()`).

### 2.3 `config.rs`

```rust
/// Validated Tailscale login (`Tailscale-User-Login` value), stored ASCII-lowercased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailscaleLogin(String);

impl TailscaleLogin {
    /// `None` unless 1..=MAX_LOGIN_LEN bytes, printable ASCII (0x21..=0x7E), no `,` `;` `"`.
    pub fn parse(raw: &str) -> Option<Self>;
    pub fn as_str(&self) -> &str;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteConfig {
    /// 1..=MAX_ALLOWED_LOGINS entries, duplicates removed (after lowercasing), order kept.
    pub allowed_logins: Vec<TailscaleLogin>,
    /// Absolute path, at most MAX_SOCKET_PATH_LEN bytes, parent directory named by it.
    pub socket_path: PathBuf,
    /// MIN_POLL_INTERVAL_MS..=MAX_POLL_INTERVAL_MS.
    pub poll_interval_ms: u64,
    /// *[R2: F5]* 0..=MAX_ALLOWED_HOSTS DNS names (lowercased, ≤ MAX_HOST_LEN, labels
    /// [a-z0-9-], no leading/trailing '-', no port); empty = "any `*.ts.net` name".
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("configuration file not found")]                 NotFound,
    #[error("configuration file unreadable")]                Unreadable,
    #[error("configuration file larger than {max} bytes")]   TooLarge { max: usize },
    #[error("configuration is not valid TOML for soos-remote")] Syntax,
    #[error("allowed_logins is empty")]                       NoAllowedLogins,
    #[error("too many allowed_logins (max {max})")]           TooManyLogins { max: usize },
    #[error("invalid login at index {index}")]                InvalidLogin { index: usize },
    #[error("poll_interval_ms out of range")]                 PollIntervalOutOfRange,
    #[error("socket_path must be absolute and at most {max} bytes")] InvalidSocketPath { max: usize },
    #[error("XDG_RUNTIME_DIR is not set or not absolute")]    NoRuntimeDir,
    #[error("no configuration path: neither --config, XDG_CONFIG_HOME nor HOME is usable")] NoConfigPath,
    #[error("too many allowed_hosts (max {max})")]            TooManyHosts { max: usize },       // [R2: F5]
    #[error("invalid host at index {index}")]                 InvalidHost { index: usize },      // [R2: F5]
    #[error("soos-remote must not run as root")]              RunningAsRoot,                     // [R2: F3]
}

/// *[R2: F3]* Pure. `Err(RunningAsRoot)` when `uid == 0 || euid == 0`.
pub fn check_not_root(uid: u32, euid: u32) -> Result<(), ConfigError>;

/// Pure: parses and validates TOML text. `runtime_dir` is the value of XDG_RUNTIME_DIR.
pub fn parse_config(text: &str, runtime_dir: Option<&Path>) -> Result<RemoteConfig, ConfigError>;
/// Bounded read (≤ MAX_CONFIG_BYTES, read through `take(MAX_CONFIG_BYTES + 1)`) then parse_config.
pub fn load_config(path: &Path, runtime_dir: Option<&Path>) -> Result<RemoteConfig, ConfigError>;
/// Default config path: $XDG_CONFIG_HOME/soos/remote.toml, else $HOME/.config/soos/remote.toml
/// (each must be absolute); pure over the two values.
pub fn default_config_path(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> Result<PathBuf, ConfigError>;
```

File format (`deny_unknown_fields`):

```toml
allowed_logins = ["owner@example.com"]   # required, 1..=8
# socket_path = "/run/user/1000/soos-remote/remote.sock"   # optional
# poll_interval_ms = 1000                                   # optional, 250..=10000
# allowed_hosts = ["mypc.tail1234.ts.net"]                  # optional, 0..=4 [R2: F5]
```

Sentinels: missing `allowed_logins` or `[]` → `NoAllowedLogins` (refuse to start). `poll_interval_ms = 0` or outside the range → `PollIntervalOutOfRange` (rejected, never clamped: same fail-closed rule as ADR 2026-09-30 "Fail-Closed `daemon.toml` Validation"). Missing `socket_path` → `<XDG_RUNTIME_DIR>/soos-remote/remote.sock`; `XDG_RUNTIME_DIR` unset, empty or relative → `NoRuntimeDir` (never a `/tmp` fallback). Relative `socket_path`, a path ending in `/`, or a path with no parent → `InvalidSocketPath`. A missing config file → `NotFound` (refuse to start: no default allowlist exists).

### 2.4 `identity.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("identity header missing")]    Missing,
    #[error("identity header repeated")]   Repeated,
    #[error("identity header malformed")]  Malformed,
    #[error("identity not allowed")]       NotAllowed,
}

/// *[R2: F5]* Pure. Exactly one Host header; `:443` suffix stripped; any other port,
/// IP literal, invalid DNS name or a name outside `allowed_hosts` (or, when that list is
/// empty, not ending in `.ts.net`) → Err(HostError) → 421 Misdirected Request. `Ok` returns
/// the **normalized** host (lowercased, `:443` removed) — the value the `Origin` check of
/// `check_lock_csrf` compares against (`Origin` normalized the same way) *[R3: R2-6]*.
pub fn check_host(headers: &[(&str, &[u8])], allowed_hosts: &[String]) -> Result<String, HostError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HostError { #[error("host header missing")] Missing, #[error("host header repeated")] Repeated, #[error("host not allowed")] NotAllowed }

/// Pure. Header names compared ASCII-case-insensitively; the value is trimmed of
/// optional whitespace, must be valid UTF-8 and pass `TailscaleLogin::parse`, and is
/// compared ASCII-case-insensitively to the allowlist. Every AuthError → 403.
pub fn authorize<'a>(
    headers: &[(&str, &'a [u8])],
    allowed: &[TailscaleLogin],
) -> Result<TailscaleLogin, AuthError>;
```

### 2.5 `http.rs` and `routes.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method { Get, Head, Post, Other }

/// Owned, bounded request head (no body is ever read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHead {
    pub method: Method,
    /// Path without query string; ≤ MAX_PATH_LEN bytes.
    pub path: String,
    /// ≤ MAX_HEADERS entries; names lowercased; values ≤ MAX_REQUEST_HEAD_BYTES total.
    pub headers: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    #[error("request head incomplete")]  Incomplete,       // need more bytes (not a response)
    #[error("request head too large")]   HeadTooLarge,     // 431
    #[error("too many headers")]         TooManyHeaders,   // 431
    #[error("malformed request")]        Malformed,        // 400 (also HTTP/1.0 and HTTP/2 preface)
    #[error("request body not allowed")] BodyNotAllowed,   // 413 (Content-Length > 0) / 400 (Transfer-Encoding)
    #[error("path too long")]            PathTooLong,      // 414
}

/// Pure. `buf` holds the bytes read so far (≤ MAX_REQUEST_HEAD_BYTES). Requires HTTP/1.1.
pub fn parse_request_head(buf: &[u8]) -> Result<RequestHead, HttpError>;

pub struct Response { pub status: u16, pub content_type: &'static str, pub body: Vec<u8>, pub extra_headers: Vec<(&'static str, String)> }
/// Serializes status line + mandatory headers (§2.8) + Content-Length + body.
pub fn encode_response(response: &Response) -> Vec<u8>;
/// SSE response head (no Content-Length; `Content-Type: text/event-stream`).
pub fn encode_sse_head() -> Vec<u8>;
/// One SSE event: `event: status\ndata: <json>\n\n` (json has no newline).
pub fn encode_sse_event(json: &str) -> Vec<u8>;
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route { Asset(AssetId), Status, Events, Lock, NotFound, MethodNotAllowed }

/// Pure. GET/HEAD for assets, /api/status and /api/events; POST only for /api/lock.
/// `/` → Asset(Index). Query strings are ignored. Anything else → NotFound;
/// a known path with a wrong method → MethodNotAllowed (405, `Allow` header).
pub fn route(method: Method, path: &str) -> Route;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CsrfError {
    #[error("action header missing")]      MissingActionHeader,
    #[error("cross-site request")]         CrossSite,
    #[error("origin mismatch")]            OriginMismatch,
}
/// Pure; §2.1 D4. `host` is the request Host header (required for the comparison when Origin is present).
pub fn check_lock_csrf(head: &RequestHead, normalized_host: &str) -> Result<(), CsrfError>;   // every error → 403
```

Route table:

| Method + path | Response |
|---|---|
| `GET /`, `/index.html` | `index.html`, `text/html; charset=utf-8` |
| `GET /app.js` | `text/javascript; charset=utf-8` |
| `GET /style.css` | `text/css; charset=utf-8` |
| `GET /manifest.webmanifest` | `application/manifest+json` |
| `GET /icon.svg` | `image/svg+xml` |
| `GET /apple-touch-icon.png` *[R2: F12]* | `image/png` |
| `GET /api/status` | `200 application/json` `StatusView` (fresh logind read) |
| `GET /api/events` | `200 text/event-stream`; `503` when `MAX_SSE_STREAMS` are open |
| `POST /api/lock` | `202 {"result":"lock_requested"}`, `409 {"result":"no_session"}`, `409 {"result":"already_locked"}`, `429 {"result":"rate_limited"}`, `503 {"result":"unavailable"}` |
| `HEAD` of a GET route | same headers, empty body |

### 2.6 `session.rs`, `logind.rs`, `status.rs`

```rust
/// Pure projection of one session's logind properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionProps {
    pub id: String,            // validated: ASCII alphanumeric, 1..=MAX_SESSION_ID_LEN
    pub uid: Option<u32>,
    pub active: bool,          // Active == true || State == "active"
    pub remote: Option<bool>,  // Some only for a boolean
    pub seat: Option<String>,  // non-empty only
    pub class: Option<String>, // non-empty only
    pub locked: bool,          // LockedHint must be boolean true
    pub idle: bool,            // IdleHint must be boolean true
    pub idle_since_unix_s: Option<u64>, // IdleSinceHint / 1_000_000; 0 or ill-typed → None
}

/// Pure mapping of a `Properties.GetAll(Session)` reply; Malformed when `Id` is missing,
/// ill-typed or differs from `expected_id`.
pub fn session_props_from_properties<S: BuildHasher>(expected_id: &str, properties: &HashMap<String, OwnedValue, S>) -> Result<SessionProps, SourceError>;

/// Pure; D7. Returns the chosen local seat session of `uid`, or None.
pub fn select_session(sessions: &[SessionProps], uid: u32) -> Option<&SessionProps>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    #[error("system bus unavailable")] BusUnavailable,
    #[error("logind call timed out")]  Timeout,
    #[error("logind call failed")]     Call,        // error name never logged beyond MAX_LOGIND_ERROR_LEN
    #[error("logind reply malformed")] Malformed,
    #[error("too many sessions")]      TooManySessions,
}

/// Mockable logind access (the tester's injection point).
pub trait SessionSource: Send + Sync + 'static {
    /// Every session of `uid` (ListSessions filtered on the uid column, then GetAll each);
    /// more than MAX_LISTED_SESSIONS listed or MAX_OWN_SESSIONS of the uid → TooManySessions;
    /// a session that vanished between the two calls is skipped.
    fn own_sessions(&self, uid: u32) -> impl Future<Output = Result<Vec<SessionProps>, SourceError>> + Send;
    /// `Manager.LockSession(id)`.
    fn lock_session(&self, id: &str) -> impl Future<Output = Result<(), SourceError>> + Send;
}

/// Production: pinned system bus `unix:path=/run/dbus/system_bus_socket`, connect bounded by
/// DBUS_CONNECT_TIMEOUT_MS, every call bounded by DBUS_CALL_TIMEOUT_MS, max_queued 16,
/// connection dropped and reopened lazily after a transport error or timeout.
pub struct ZbusSessionSource { /* tokio::sync::Mutex<Option<zbus::Connection>> */ }
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState { Locked, Unlocked, NoSession, Unavailable }

/// JSON body of /api/status and of every SSE event. Never contains the uid, user name,
/// session id, seat or any identity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StatusView {
    pub state: SessionState,
    pub active: bool,                     // false unless state ∈ {Locked, Unlocked}
    pub idle: bool,                       // idem
    pub idle_since_unix_s: Option<u64>,   // None unless idle
    pub checked_unix_ms: u64,             // SystemTime::now at the read; 0 if the clock is before the epoch
}

/// Pure; D8. Ok(sessions) → select_session → Locked/Unlocked/NoSession; Err(_) → Unavailable.
pub fn status_from(result: &Result<Vec<SessionProps>, SourceError>, uid: u32, checked_unix_ms: u64) -> StatusView;
```

```rust
/// *[R3: R2-1, R2-4]* One logind read as carried on the watch channel (internal, not serialized).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// Strictly increasing per process (shared `AtomicU64`, starts at 1, `checked_add`;
    /// overflow is unreachable in practice and ends the poller → EXIT_RUNTIME).
    pub seq: u64,
    pub view: StatusView,
}
/// Pure: true when the two views differ in anything but `checked_unix_ms`.
pub fn view_changed(previous: &StatusView, next: &StatusView) -> bool;
```

Poller *[R3: R2-1]*: spawned once at start inside the server `JoinSet`; waits on "subscriber count > 0" (`tokio::sync::Notify` + `AtomicUsize`); while > 0, every `poll_interval_ms` it reads `own_sessions` under `SNAPSHOT_DEADLINE_MS`, builds the view with `status_from` and **`send_replace(Some(Reading { seq: next, view }))` on every read** (no change filter at the channel: change detection belongs to each stream, see D6); as soon as the count returns to 0 it `send_replace(None)` and waits again. `/api/status` performs its own fresh read and never consults the channel.

### 2.7 `socket.rs`

```rust
#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    #[error("socket directory is not a directory or is a symlink")] NotADirectory,
    #[error("socket directory is not owned by the service user")]   WrongOwner,
    #[error("socket path exists and is not a socket")]               NotASocket,
    #[error("socket setup failed: {0}")]                             Io(std::io::ErrorKind),
}

/// Creates `parent` with mode 0700 when absent (`DirBuilder::mode(0o700)`, not recursive past
/// one missing level: the runtime dir itself must exist); when present: `symlink_metadata`
/// must be a directory (not a symlink) owned by `uid`, then `set_permissions(0o700)`.
pub fn prepare_socket_dir(parent: &Path, uid: u32) -> Result<(), SocketError>;

/// Removes a stale socket at `path` only when `symlink_metadata` says it is a socket owned by
/// `uid` (anything else → NotASocket, never unlinked); binds `tokio::net::UnixListener`;
/// `set_permissions(0o600)`. The 0700 parent closes the bind→chmod window.
pub fn bind_listener(path: &Path, uid: u32) -> Result<tokio::net::UnixListener, SocketError>;
```

On clean shutdown (SIGTERM/SIGINT) the socket file is removed. `uid` comes from `nix::unistd::getuid()` (safe API).

### 2.8 `server.rs`

- Accept loop over the `UnixListener`; a `tokio::sync::Semaphore` of `MAX_CONNECTIONS`; when no permit is free the accepted stream is dropped at once (no response, bounded work).
- Per connection: read into a buffer capped at `MAX_REQUEST_HEAD_BYTES` until `parse_request_head` stops returning `Incomplete`, all under `REQUEST_HEAD_TIMEOUT_MS` (timeout → close without response). Then, in this order: `HttpError` → its status; `check_host` → `421` *[R2: F5]*; `authorize` → `403`; `route`; for `Lock`, `check_lock_csrf` → `403`; dispatch.
- Every non-SSE response is computed fully into a `Vec<u8>` and written under `RESPONSE_WRITE_TIMEOUT_MS`, then the stream is shut down (mirrors the daemon rule "compute, then write with its own timeout").
- Mandatory headers on every response, error responses included: `Cache-Control: no-store`, `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `X-Frame-Options: DENY`, `Connection: close`.
- SSE (§2.1 D6): `MAX_SSE_STREAMS` counted with an `AtomicUsize` guard (decrement on drop); first event from a fresh read made after accept; the head and every event are written under `RESPONSE_WRITE_TIMEOUT_MS` (a slow reader is dropped); every `SSE_KEEPALIVE_MS` the latest status is re-sent; the read half is polled concurrently and EOF/error ends the stream *[R2: F6]*; the stream ends after `MAX_SSE_STREAM_MS` or at shutdown.
- Supervision *[R2: F2]*: the accept loop and the poller run in one `tokio::task::JoinSet`; the first task to finish (other than by shutdown) triggers shutdown and `main` exits with `EXIT_RUNTIME`.
- Lock: a `tokio::sync::Mutex<Option<tokio::time::Instant>>` of the last accepted lock; a lock within `MIN_LOCK_INTERVAL_MS` of it → `429`, no logind call. Then `own_sessions(uid)` → `select_session`: none → `409 no_session`; selected and `locked` → `409 already_locked`; else `lock_session(id)`: `Ok` → `202`, `Err` → `503`. The interval is recorded only when `lock_session` is called.
- Graceful shutdown on SIGTERM/SIGINT: stop accepting, close streams, remove the socket.

### 2.9 User unit `packaging/soos-remote.service`

```ini
[Unit]
Description=soos remote companion (lock status and remote lock over Tailscale Serve)
Documentation=https://github.com/Mysticaly622/soos/blob/main/Docs/REMOTE_COMPANION.md

[Service]
Type=exec
ExecStart=%h/.local/bin/soos-remote
Restart=on-failure
RestartSec=5
# EXIT_CONFIG (78, EX_CONFIG): configuration / root refusal errors are not retried [R2: F7]
RestartPreventExitStatus=78
NoNewPrivileges=yes
RestrictAddressFamilies=AF_UNIX
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
SystemCallArchitectures=native
UMask=0077

[Install]
WantedBy=default.target
```

### 2.10 `scripts/install_remote.sh`

`set -euo pipefail`; refuses to run as root (`EUID == 0` → exit 1). Steps: `cargo build --release --locked -p soos-remote`; `install -Dm755` to `$HOME/.local/bin/soos-remote`; `install -Dm644 packaging/soos-remote.service` to `${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/soos-remote.service`; writes `${XDG_CONFIG_HOME:-$HOME/.config}/soos/remote.toml` from a commented template **only when absent** (mode 0600, `allowed_logins = []`, so the service refuses to start until the owner fills it in); `systemctl --user daemon-reload`. Never enables the unit, never runs `tailscale`, never calls `sudo`. Prints the next steps (fill the login, `systemctl --user enable --now soos-remote`, `tailscale serve --bg unix:$XDG_RUNTIME_DIR/soos-remote/remote.sock`). `--uninstall` stops/disables the unit, removes the binary and the unit, keeps the config.

### 2.11 Web UI (`assets/`)

- English strings (project language policy).
- One status card: `Locked`, `Unlocked`, `No session`, `Unavailable` (logind), `Unreachable` (fetch/stream failure: the PC is off, asleep, or off the tailnet); active/idle line with "idle for N min" computed client-side from `idle_since_unix_s`; "Updated N s ago" from `checked_unix_ms`.
- "Lock now" button: enabled only for `Unlocked`; sends `fetch('/api/lock', {method: 'POST', headers: {'X-Soos-Action': 'lock'}})`; then shows "Lock requested…" and, if the stream has not reported `locked` within `LOCK_CONFIRM_UI_MS` (5000 ms, a UI constant in `app.js`), "The desktop did not confirm the lock (LockedHint unchanged)".
- `EventSource('/api/events')`; on error: shows `Unreachable` after one failed `fetch('/api/status')`; on `visibilitychange` → visible and `pageshow`: closes and reopens the stream and fetches `/api/status`.
- PWA: `manifest.webmanifest` (`display: standalone`, `start_url: "/"`, `scope: "/"`, icon `icon.svg`), `<meta name="apple-mobile-web-app-capable">`, light/dark via `prefers-color-scheme`. No service worker in this issue.
- No `innerHTML` with server data (`textContent` only).
- *[R3: R2-6]* Footer: a plain `<a href="https://github.com/Mysticaly622/soos">source</a>` link (ADR item (9), AGPL-3.0 §13); allowed by RMC-S8 because it loads nothing.

### 2.12 Contract migration *[R2: F1]*

`tests/invariants/src/presence_unlock_contract.rs::test_pau_zbus_is_used_only_by_the_daemon` encodes the 2026-10-02 decision "zbus is daemon-only". Issue #339 is an owner-approved scope change of that decision (the remote companion must read logind), so the contract is **migrated, not weakened**:

- the allowed set changes from `{crates/daemon/Cargo.toml}` to exactly `{crates/daemon/Cargo.toml, crates/remote/Cargo.toml}`; every other manifest under `crates/` and `tests/` must still not mention `zbus`;
- `crates/remote/Cargo.toml` must declare exactly `zbus = { workspace = true }` (same assertion style as the daemon line);
- the PAM assertion (`no zbus`, `no dbus` in `crates/pam/Cargo.toml`) is unchanged;
- the test keeps its name `test_pau_zbus_is_used_only_by_the_daemon` (cited by the `✅ Verified` matrix row PAU17, `AI/VERIFICATION_MATRIX.md`; a rename would break `matrix_citations`) *[R3: R2-2]*; a doc comment on it cites ADR 2026-10-05 item (7), and PAU17 gains an italic `*(scope widened to soos-remote by ADR 2026-10-05, GitHub #339)*` annotation; the lockfile pin test `test_pau_lockfile_pins_zbus_5_without_libdbus` is unchanged.

This edit is made by the tester in Phase 2 and is reviewed as a contract change.

---

## 3. Constants & Config Summary (single source: `crates/remote/src/lib.rs`)

| Constant | Value | At the bound |
|---|---|---|
| `MAX_CONFIG_BYTES` | 16 384 | larger file → `TooLarge` |
| `MAX_ALLOWED_LOGINS` | 8 | more → `TooManyLogins` |
| `MAX_LOGIN_LEN` | 254 | longer → `InvalidLogin` / `AuthError::Malformed` |
| `DEFAULT_POLL_INTERVAL_MS` / `MIN_` / `MAX_` | 1000 / 250 / 10 000 | outside → `PollIntervalOutOfRange` |
| `MAX_SOCKET_PATH_LEN` | 107 | longer → `InvalidSocketPath` (`sun_path` is 108 bytes incl. NUL) |
| `SOCKET_DIR_NAME` / `SOCKET_FILE_NAME` | `soos-remote` / `remote.sock` | — |
| `MAX_CONNECTIONS` | 16 | further accepted streams dropped at once |
| `MAX_SSE_STREAMS` | 4 | further `/api/events` → `503` |
| `MAX_REQUEST_HEAD_BYTES` | 8192 | → `431` |
| `MAX_HEADERS` | 32 | → `431` |
| `MAX_PATH_LEN` | 256 | → `414` |
| `REQUEST_HEAD_TIMEOUT_MS` | 5000 | connection closed, no response |
| `RESPONSE_WRITE_TIMEOUT_MS` | 2000 | connection / stream dropped |
| `SSE_KEEPALIVE_MS` | 15 000 | — |
| `MAX_SSE_STREAM_MS` | 1 800 000 (30 min) | stream closed, browser reconnects |
| `MIN_LOCK_INTERVAL_MS` | 2000 | → `429` |
| `DBUS_CALL_TIMEOUT_MS` / `DBUS_CONNECT_TIMEOUT_MS` | 500 / 1000 | → `Timeout` → `unavailable` / `503` |
| `MAX_LISTED_SESSIONS` / `MAX_OWN_SESSIONS` | 256 / 16 | → `TooManySessions` → `unavailable` |
| `MAX_SESSION_ID_LEN` | 64 | invalid ids skipped |
| `MAX_LOGIND_ERROR_LEN` | 256 | logged error names truncated on a char boundary |
| `SYSTEM_BUS_ADDRESS` | `unix:path=/run/dbus/system_bus_socket` | never from the environment |
| `SNAPSHOT_DEADLINE_MS` *[R2: F8]* | 1500 | whole snapshot → `Timeout` → `unavailable` |
| `LOCK_FLOW_DEADLINE_MS` *[R2: F8]* | 2000 | whole lock flow → `503` |
| `MAX_ALLOWED_HOSTS` / `MAX_HOST_LEN` *[R2: F5]* | 4 / 253 | → `TooManyHosts` / `InvalidHost`; a request Host over the bound → `421` |
| `TS_NET_SUFFIX` *[R2: F5]* | `.ts.net` | — |
| `EXIT_CONFIG` / `EXIT_RUNTIME` *[R2: F3, F7]* | 78 / 1 | 78 is in `RestartPreventExitStatus` |
| `STALE_UI_MS` (UI constant in `app.js`) *[R2: F2]* | 45 000 | UI shows `Unreachable` |
| `IDENTITY_HEADER` / `ACTION_HEADER` / `ACTION_LOCK` | `tailscale-user-login` / `x-soos-action` / `lock` | — |
| `FORWARDED_HOST_HEADER` / `FORWARDED_PROTO_HEADER` / `FORWARDED_PROTO_HTTPS` *[R5: R4-1, R4-2]* | `x-forwarded-host` / `x-forwarded-proto` / `https` | header names compared ASCII-case-insensitively, as `IDENTITY_HEADER`; the only place these strings are written |

These are independent of the daemon presence constants of the same names (different binary, different crate, no shared runtime); the remote crate does not import `soos-daemon` (it would pull ONNX Runtime and V4L into a user service).

---

## 4. Error Taxonomy

| Error | Where | Outcome |
|---|---|---|
| `ConfigError::*` (incl. `RunningAsRoot`) | start | `tracing::error!` with the message (no secrets), exit `EXIT_CONFIG` = 78, not restarted (`RestartPreventExitStatus=78`) *[R2: F3, F7, F11]* |
| `HostError::*` *[R2: F5]* | request | `421` |
| poller or accept task ended | runtime | shutdown, exit `EXIT_RUNTIME` = 1, systemd restarts *[R2: F2]* |
| `SocketError::{NotADirectory, WrongOwner, NotASocket}` *[R3: R2-5]* | start | `tracing::error!`, exit `EXIT_CONFIG` = 78 (persistent, not restarted) |
| `SocketError::Io(_)` | start | exit `EXIT_RUNTIME` = 1 (restarted by systemd) |
| `HttpError::Incomplete` past the deadline | connection | close, no response |
| `HttpError::{HeadTooLarge, TooManyHeaders}` | request | `431` |
| `HttpError::Malformed` | request | `400` |
| `HttpError::BodyNotAllowed` | request | `413` (Content-Length > 0) or `400` (Transfer-Encoding) |
| `HttpError::PathTooLong` | request | `414` |
| `AuthError::*` | request | `403` (body `{"result":"forbidden"}`), checked **before** routing so an unauthorized client learns nothing about routes |
| `CsrfError::*` | `/api/lock` | `403` |
| `SourceError::*` | status | `state = "unavailable"` |
| `SourceError::*` | lock | `503 {"result":"unavailable"}` |

There is no PAM, IPC or `Verdict` path in this crate: no error can authenticate anything, and the only state-changing action is a lock request.

## 5. Latency Budget

Not on the authentication path. Status freshness: ≤ `poll_interval_ms` (1 s default) + `SNAPSHOT_DEADLINE_MS` while a stream is open *[R3: R2-6]*.

## 6. Invariants Touched

- ARCHITECTURE §2 invariants 1–6: untouched (no PAM, no daemon, no IPC change).
- ADR 2026-09-12 "IPC … Zero network sockets": unchanged for the PAM ↔ daemon IPC; `soos-remote` itself also opens no network socket (Unix socket only). New ADR (§9) records the companion boundary.
- New companion invariants (ARCHITECTURE §13, matrix RMC rows):
  - RC-1 `soos-remote` never runs as root (refused by `check_not_root` at start *[R2: F3]*), never opens an `AF_INET`/`AF_INET6` socket, never calls `UnlockSession`/`Unlock`/`SetLockedHint`.
  - RC-2 Every request is refused (`403`) unless it carries exactly one allowlisted `Tailscale-User-Login`; an empty allowlist refuses to start.
  - RC-3 A logind failure is never reported as `unlocked`, and no `unlocked` older than the stream's own first read is ever sent *[R2: F2]*.
  - RC-4 Every read, write, connection count and stream lifetime is bounded (constants §3).
  - RC-5 No identity, header value, session id or user name is logged or returned in a body.

## 7. Test Hooks for the Tester

- Pure functions: `parse_config`, `default_config_path`, `TailscaleLogin::parse`, `authorize`, `parse_request_head`, `route`, `check_lock_csrf`, `encode_response`, `encode_sse_event`, `session_props_from_properties`, `select_session`, `status_from`.
- `SessionSource` trait: a mock with scripted `own_sessions` results and a spy on `lock_session` ids and call count.
- `socket::prepare_socket_dir` / `bind_listener` over `tempfile::TempDir` (symlink, regular file, foreign-owner cases where feasible without root: owner mismatch is tested by passing a different `uid`).
- `server::serve(listener, Arc<ServerState<S>>, shutdown)` public entry (state holds config, uid, source, clocks); `tests/server_tests.rs` drives it with raw HTTP over `tokio::net::UnixStream`.
- Time: lock rate limit and SSE timings use `tokio::time` so tests can run under `#[tokio::test(start_paused = true)]` (needs the `test-util` dev feature *[R2: F4]*).
- *[R2: F2]* Mandatory paused-time test: open a stream, let the mock report `unlocked`, close it, switch the mock to an error (or `locked`), open a new stream: its first event must be `unavailable` (or `locked`), never the old `unlocked`; and a test that a poller task ending makes `serve` return an error.
- *[R3: R2-1]* Mandatory paused-time test: with a mock that keeps reporting the same `unlocked` view, an open stream receives a re-sent event every `SSE_KEEPALIVE_MS` whose `checked_unix_ms` is strictly greater than the previous one, and no extra event between keep-alives; *[R3: R2-4]* a test where the mock clock goes backwards still delivers the next change.
- *[R2: F3]* `check_not_root` table test; *[R2: F5]* `check_host` table test and an end-to-end `421`; *[R2: F6]* a closed client frees its SSE slot without waiting for a keep-alive.

## 8. Static Contracts (`tests/invariants/src/remote_companion_contract.rs`)

| ID | Contract |
|---|---|
| RMC-S1 | `crates/remote/src/lib.rs` and `main.rs` contain `#![forbid(unsafe_code)]` (also via `test_business_crates_forbid_unsafe_code`). |
| RMC-S2 | No file under `crates/remote/src` names `TcpListener`, `TcpStream`, `UdpSocket` or `std::net`. |
| RMC-S3 | With `//` line comments and `/* */` block comments stripped, no file under `crates/remote/src` contains the exact strings `"UnlockSession"`, `"Unlock"` or `"SetLockedHint"` (string literals: the only way to name a D-Bus method through `call_method`) *[R2: F10]*. |
| RMC-S4 | No other workspace `Cargo.toml` depends on `soos-remote`; `crates/remote/Cargo.toml` does not depend on `soos-daemon`, `soos-pam`, `ort`, `v4l`. |
| RMC-S5 | `packaging/soos-remote.service` has `RestrictAddressFamilies=AF_UNIX`, `NoNewPrivileges=yes`, `UMask=0077`, no `User=`/`Group=` line. |
| RMC-S6 | `scripts/install_remote.sh` refuses root (`EUID` check), and no non-comment line runs `sudo`, `tailscale` or `systemctl --user enable` as a command (the commands may only appear inside quoted `echo`/`printf` instruction text). |
| RMC-S7 | `Docs/REMOTE_COMPANION.md` exists and documents `tailscale serve --bg unix:`, `allowed_logins`, the `LockedHint` requirement and the out-of-scope list (remote unlock). |
| RMC-S8 | `assets/index.html` has no inline `<script>` body, and no `src=`, `href=` of a `<link>`/`<script>`, or `url(` in `style.css`, points to an absolute URL (`http://`, `https://`, `//`) — plain `<a href>` source links are allowed *[R2: F12]*. |
| RMC-S9 *[R2: F1; R3: R2-3]* | `crates/remote/Cargo.toml` declares exactly `zbus = { workspace = true }` (no feature override). `crates/remote/src/lib.rs` defines `SYSTEM_BUS_ADDRESS = "unix:path=/run/dbus/system_bus_socket"` and `crates/remote/src/logind.rs` opens the connection only through `zbus::connection::Builder::address(`. With comments stripped, no file under `crates/remote/src` contains: `Connection::system`, `Connection::session`, `Builder::system`, `Builder::session`, `env::var`, `DBUS_SYSTEM_BUS_ADDRESS`, `DBUS_SESSION_BUS_ADDRESS`, `object_server`, `#[proxy`, `zbus::proxy`, `receive_signal`, `MessageStream`, `CacheProperties::Yes`, `CacheProperties::Lazily` (no generated proxy, no property cache: every read is a fresh `Properties.GetAll`, as presence). |
| RMC-S10 *[R2: F3]* | `crates/remote/src/main.rs` calls `check_not_root` before `load_config`. |
| RMC-S11 *[R2: F7]* | `packaging/soos-remote.service` has `RestartPreventExitStatus=78` and `crates/remote/src/lib.rs` defines `EXIT_CONFIG: u8 = 78`. |
| RMC-S12 *[R5: R4-5]* | New test `test_rmc_s12_installer_scripts_are_executable` in `tests/invariants/src/remote_companion_contract.rs`: for each of `scripts/install_remote.sh` and `scripts/install.sh` (paths resolved through the same repository-root helper as the other RMC contracts), `std::fs::metadata(path)?.permissions().mode() & 0o111 != 0` (`std::os::unix::fs::PermissionsExt`). The checkout honours the git mode, so no `git` subprocess is needed (consistent with every other invariant: none shells out). The mode change `100644 → 100755` of `scripts/install_remote.sh` is already staged and lands in this cycle's commit; the test fails on a `100644` checkout of the script. |
| RMC-S7b *[R5: R4-4]* | New test `test_rmc_s7b_documentation_describes_the_effective_host` (the existing RMC-S7 test is **not** edited): `Docs/REMOTE_COMPANION.md` contains `X-Forwarded-Host`, `X-Forwarded-Proto` and `effective host`, and no longer contains the strings `pending owner verification` or `(pending)`. |

## 9. Documentation Drift / ADR

Draft ADR line for `AI/DECISIONS.md`:

> **[2026-10-05] Remote Companion `soos-remote`: User-Level, Unix Socket Behind Tailscale Serve, Status and Lock Only (GitHub #339; owner decisions 2026-10-05; development branch `feat/remote-companion`, not merged until the owner approves):** (1) A new leaf crate `crates/remote` (binary `soos-remote`) runs as a **user-level** systemd service of the session owner, never root; `soos-daemon`, `pam_soos.so` and the IPC protocol are unchanged and no crate depends on it. (2) It listens only on a `0600` Unix socket in a `0700` directory under `$XDG_RUNTIME_DIR` and opens no network socket (`RestrictAddressFamilies=AF_UNIX`); remote reachability, HTTPS and the caller identity come from `tailscale serve --bg unix:<path>` on the owner's tailnet. No hosted service or cloud relay exists. (3) Every request needs exactly one `Tailscale-User-Login` header in the non-empty `allowed_logins` allowlist (trusted only because the socket is reachable by `tailscaled` and the owner alone); the lock action additionally needs `X-Soos-Action: lock` and a same-origin `Origin`/`Sec-Fetch-Site`. (4) Scope: real-time status from logind `LockedHint`/`IdleHint` over Server-Sent Events (polling only while a stream is open) and remote lock via `Manager.LockSession`; a logind failure is reported as `unavailable`, never `unlocked`. The status depends on the desktop setting `LockedHint` and the lock on the desktop honouring the logind `Lock` signal (sway-family: swayidle `lock` hook). (5) Remote unlock, push notifications and live camera are out of scope and each needs its own ADR; the crate never calls `UnlockSession`/`Unlock`/`SetLockedHint`. (6) System-wide packaging (`install.sh`, deb/rpm/Arch) is deferred to a follow-up after merge approval; `scripts/install_remote.sh` installs per user without root. (7) *Amends the zbus scope of the 2026-10-02 presence ADR and `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`*: `zbus` (same workspace pin, `tokio` only, pinned system bus address, no environment-derived bus) is allowed in exactly two crates, `soos-daemon` and `soos-remote`; `pam_soos.so` never links a D-Bus client. (8) `soos-remote` refuses to run as root, and requires the `Host` to be an allowed `*.ts.net` name; `tailscale serve --http` and `tailscale funnel` must not be used for it. (9) AGPL-3.0 §13: the companion serves only the owner (allowlist), and the page links the source repository.

Other doc updates listed in §1.1. Drift found *[R2: F1]*: ADR 2026-10-02 presence, `AI/ARCHITECTURE.md:22` and `Docs/SECURITY_AND_QUALITY_GUIDELINES.md:120` state zbus is daemon-only; resolved by ADR item (7) and the contract migration §2.12.

---

## 10. Acceptance Mapping (GitHub #339)

| Acceptance item | Spec element |
|---|---|
| Real-time locked / active / idle / no-session status | §2.6 `StatusView`, poller, §2.8 SSE, §2.11 UI |
| Remote lock | §2.5 `Route::Lock`, §2.8 lock flow, D9 |
| Phone (iPhone) access without native app | §2.11 PWA, §2.10/Docs Tailscale steps |
| No hosted site / no cloud | D2, ADR (2) |
| User-level, daemon untouched | D1, §1.2, RMC-S4/S5 |
| No TCP socket | D2, RMC-S2/S5 |
| Owner only | D3, RC-2, `authorize` |
| Out of scope respected (no unlock) | D10, RMC-S3 |

---

## 11. Revision 2 Change Log

| Finding | Resolution |
|---|---|
| F1 zbus daemon-only contract | §1.1 rows, §2.12 migration, ADR item (7), RMC-S9, doc updates |
| F2 stale status replay | D6 rewrite (reset to `None`, fresh first read, keep-alive re-sends status, `JoinSet` supervision), RC-3, mandatory paused-time test, `STALE_UI_MS` |
| F3 root not refused | D1, `check_not_root`, `RunningAsRoot`, RMC-S10 |
| F4 `test-util` | dev-dependency in §1.1 |
| F5 Host / rebinding | D5a, `check_host`, `allowed_hosts`, `421`, Phase 4 host verification, ADR item (8) |
| F6 dead streams hold slots | D6 read-half watch, §2.8, test hook |
| F7 config error restart loop | `EXIT_CONFIG = 78`, `RestartPreventExitStatus=78`, RMC-S11 |
| F8 no overall logind deadline | `SNAPSHOT_DEADLINE_MS`, `LOCK_FLOW_DEADLINE_MS`, D9 |
| F9 missing sync points | §1.1 rows (candid_review.sh, ARCHITECTURE forbid list, MOCK_STRATEGY, project-facts, manifest keys) |
| F10 RMC-S3 wording, tie-break | RMC-S3 rewrite, D7 order |
| F11 stderr lint | D12: `tracing::error!`, no `print_stderr` allow |
| F12 iOS icon, RMC-S8, AGPL | D11 PNG apple-touch-icon, RMC-S8 rewrite, ADR item (9) |

---

## 12. Revision 3 Change Log

| Finding | Resolution |
|---|---|
| R2-1 §2.6 vs D6 contradiction | `Reading { seq, view }`, poller `send_replace` on every read, `None` reset, per-stream change detection + keep-alive re-send; steady-state paused-time test |
| R2-2 PAU17 citation | test name kept; PAU17 annotation |
| R2-3 narrow RMC-S9 | RMC-S9 extended to the presence bus rules (no proxy, no cache, no env bus, `Builder::address(` only) |
| R2-4 wall-clock filter | monotonic `seq` filter; UI staleness measured from event arrival |
| R2-5 SocketError exit | persistent errors → 78, `Io` → 1 |
| R2-6 small gaps | §5 deadline, UI source link, `check_host` doc split and normalized-host `Origin` comparison |

---

## 13. Revision 4 — D5a Host Forwarding Verified on the Owner's Host (2026-10-06)

D5a required a Phase 4 check on the owner's host that `tailscale serve unix:` forwards the
original `Host`, and returned the design to the architect if it did not. It does not.

### 13.1 Evidence (owner's host, Tailscale 1.102.4, `tailscale serve --bg unix:/run/user/1000/soos-remote/remote.sock`, "tailnet only")

A capture listener on the socket (service stopped for the capture, identity values redacted) received, for `curl https://arch.<tailnet>.ts.net/api/status` from the same tailnet:

```
GET /api/status HTTP/1.1
Host: localhost
Tailscale-User-Login: <redacted>
X-Forwarded-For: 100.x.y.z
X-Forwarded-Host: arch.<tailnet>.ts.net
X-Forwarded-Proto: https
```

A second request sent with forged `X-Forwarded-Host: evil.com`, `X-Forwarded-Proto: http` and
`Tailscale-User-Login: forged@x` reached the socket with the **real** values
(`X-Forwarded-Host: arch.<tailnet>.ts.net`, `X-Forwarded-Proto: https`, the real login): Serve
overwrites all three. The deployed revision-3 build answered `421 misdirected_request` end to end.

### 13.2 Decision D5a′ (replaces D5a's Host rule; everything else in D5a stands)

- **Effective host.** *[R5: R4-2]* The effective host of a request is decided by
  `X-Forwarded-Host` alone whenever that header is present: when it occurs exactly once its value
  is the effective host, and the `Host` header is **not inspected at all** (absent, single or
  repeated `Host` give the same result). When `X-Forwarded-Host` is absent the single `Host`
  header is the effective host under the unchanged D5a rules (direct local clients and tests).
  `HostError::Missing` therefore means that **neither** `X-Forwarded-Host` **nor** `Host` is
  present. Rationale (for the auditor): Go's reverse proxy always emits exactly one `Host`
  (`localhost` for Unix backends today, §13.1), so inspecting it adds nothing, and a local client
  forging either header on the `0600` socket is the owner; not pinning `Host: localhost` keeps the
  check valid if Serve ever forwards the original name.
- **Normalisation.** *[R5: R4-3]* The effective host, whichever header it came from, goes through
  the same pipeline as the D5a `Host`: OWS trimmed, ASCII-lowercased, one trailing `:443`
  stripped, then `is_valid_host_name`, then the `*.ts.net` / `allowed_hosts` rule. So
  `X-Forwarded-Host: " PC.Tail1234.TS.NET:443 "` → `Ok("pc.tail1234.ts.net")`, and a port other
  than `:443`, an IP literal (`100.64.0.1`), a trailing dot, an empty value, a comma-separated
  list (`a.ts.net, b.ts.net`) or any invalid DNS name → `NotAllowed`. `X-Forwarded-Host` repeated
  → `HostError::Repeated` (decided before the proto check, so "XFH twice + bad proto" is
  `Repeated`).
- **Transport.** *[R5: R4-1]* When `X-Forwarded-Host` is present, `X-Forwarded-Proto` **MUST**
  be present exactly once and its value, OWS-trimmed, MUST equal `https` ASCII-case-insensitively
  (`HTTPS`, `" https "` accepted); absent, repeated (even two identical `https`) or any other value
  → `NotAllowed` (`421`). When `X-Forwarded-Host` is absent, `X-Forwarded-Proto`, if present, must
  still be exactly one `https`; absent is accepted (direct local clients and tests send neither
  header). This turns the documented ban on `tailscale serve --http` into an enforced one on
  every proxied request, not only when the proxy volunteers the header.
- **Evaluation order** (fixed, so the tester can pin the variant): (1) count `X-Forwarded-Host`:
  two or more → `Repeated`; (2) if one: proto rule (mandatory) → `NotAllowed` on failure; if
  none: `Host` count → `Missing` / `Repeated`, then proto rule (optional) → `NotAllowed` on
  failure; (3) normalise and validate the effective host → `NotAllowed` on failure; (4) return the
  normalised host. `check_host` stays pure, allocation-bounded (`MAX_HOST_LEN` 253, at most
  `MAX_HEADERS` 32 headers) and panic-free; the header names are the §3 constants
  `FORWARDED_HOST_HEADER` / `FORWARDED_PROTO_HEADER` / `FORWARDED_PROTO_HTTPS`.
- **Trust argument** (same as D3). *[R5: R4-9]* The socket is `0600`, reachable only by
  `tailscaled` (root) and the owner. On the owner's host (§13.1) Serve was observed to
  **overwrite** exactly three client-supplied headers with the real values: `X-Forwarded-Host`,
  `X-Forwarded-Proto` and `Tailscale-User-Login`; these are the only headers D5a′ and D3 rely on.
  `X-Forwarded-For` was also sent by Serve but is **ignored by the service** (never read, never
  compared, never logged); no claim is made about it. The owner forging their own headers on the
  socket is no escalation.
- **Origin.** The `Origin` comparison of `check_lock_csrf` uses the normalised **effective host**
  returned by `check_host` (signature unchanged; `server.rs` already passes the returned value):
  with the captured head, `Origin: https://pc.tail1234.ts.net` → lock proceeds, while
  `Origin: https://localhost` → `OriginMismatch` → `403`.

### 13.3 Changes

| Item | Change |
|---|---|
| `crates/remote/src/lib.rs` *[R5]* | Add `pub const FORWARDED_HOST_HEADER: &str = "x-forwarded-host";`, `pub const FORWARDED_PROTO_HEADER: &str = "x-forwarded-proto";`, `pub const FORWARDED_PROTO_HTTPS: &str = "https";` (§3 row). |
| `crates/remote/src/identity.rs::check_host` | D5a′ as in §13.2 (effective host, normalisation, mandatory proto with XFH, evaluation order); signature unchanged: `pub fn check_host(headers: &[(&str, &[u8])], allowed_hosts: &[String]) -> Result<String, HostError>`. Doc comment rewritten to describe the effective host. |
| `HostError` doc comments *[R5: R4-2]* | Variants, `Display` texts and the `421` mapping unchanged (so `test_rmc_identity_errors_never_echo_values` and the message test keep passing). Doc comments become: `Missing` — "Neither `X-Forwarded-Host` nor `Host` is present."; `Repeated` — "The header that decides the effective host (`X-Forwarded-Host` when present, otherwise `Host`) occurs more than once."; `NotAllowed` — "Port other than 443, IP literal, invalid DNS name, a name outside the allowlist, or an `X-Forwarded-Proto` that is absent while `X-Forwarded-Host` is present, repeated, or not `https`." |
| Unit tests (`crates/remote/tests/identity_tests.rs`, **new** test functions only; the three existing `check_host` tests are not edited and stay valid because they send no `X-Forwarded-*` header) *[R5: R4-1, R4-2, R4-3]* | `check_host` rows — effective host: XFH `pc.tail1234.ts.net` + `Host: localhost` → `Ok("pc.tail1234.ts.net")`; XFH + `Host` absent → OK; XFH + `Host` repeated (two `Host` lines) → OK; neither XFH nor `Host` → `Missing`; `Host` once + XFH absent → unchanged D5a rules (existing tests); XFH twice → `Repeated`; XFH twice + `X-Forwarded-Proto: http` → `Repeated`. Normalisation: XFH `" PC.Tail1234.TS.NET:443 "` → `Ok("pc.tail1234.ts.net")`; XFH `pc.tail1234.ts.net:8443` → `NotAllowed`; XFH `100.64.0.1` → `NotAllowed`; XFH `evil.com` → `NotAllowed`; XFH `a.ts.net, b.ts.net` → `NotAllowed`; XFH empty → `NotAllowed`; XFH `pc.tail1234.ts.net.` → `NotAllowed`. Transport (every row carries proto `https` unless stated): XFH + proto absent → `NotAllowed`; XFH + `X-Forwarded-Proto: http` → `NotAllowed`; XFH + `X-Forwarded-Proto: https` **twice** (identical values, so a first-header-wins implementation fails) → `NotAllowed`; XFH + `X-Forwarded-Proto: HTTPS` → OK; XFH + `X-Forwarded-Proto: " https "` (OWS-padded) → OK; `Host` only + proto absent → OK; `Host` only + proto `https` → OK; `Host` only + proto `http` → `NotAllowed`; `Host` only + proto `https` twice → `NotAllowed`. Allowlist (`allowed_hosts = ["mypc.tail1234.ts.net"]`): XFH `mypc.tail1234.ts.net` + `Host: other.tail1234.ts.net` → `Ok("mypc.tail1234.ts.net")`; XFH `other.tail1234.ts.net` + `Host: mypc.tail1234.ts.net` → `NotAllowed`. |
| End-to-end tests (`crates/remote/tests/server_tests.rs`, **new** test functions; `with_identity` and every existing test untouched) *[R5: R4-3, R4-8]* | A `serve_head()` helper sends the §13.1 capture **verbatim in shape**, with the synthetic names already used by `server_tests` (`Host: localhost`, `Tailscale-User-Login: owner@example.com`, `X-Forwarded-For: 100.64.0.1`, `X-Forwarded-Host: pc.tail1234.ts.net`, `X-Forwarded-Proto: https`). Assertions: `GET /api/status` → `200`; `POST /api/lock` with `X-Soos-Action: lock` and `Origin: https://pc.tail1234.ts.net` → `202 {"result":"lock_requested"}` (mock session unlocked, one `LockSession` call); same with `Origin: https://localhost` → `403 {"result":"forbidden"}` and **no** `LockSession` call; the captured head with `X-Forwarded-Proto: http` → `421 {"result":"misdirected_request"}`; the captured head without `X-Forwarded-Proto` → `421`; the captured head with `X-Forwarded-Host: evil.com` → `421`; and an `allowed_hosts = ["mypc.tail1234.ts.net"]` server accepts XFH `mypc.tail1234.ts.net` with `Host: localhost` (`200`). The `421` responses carry the mandatory headers (`Connection: close`, `Cache-Control: no-store`, …) like every other error. No test and no fixture contains a real login, tailnet name, 100.x address or profile URL. |
| Static contracts (§8) *[R5: R4-4, R4-5]* | RMC-S12 (installer scripts executable, `std::fs::metadata` only) and RMC-S7b (documentation names `X-Forwarded-Host`, `X-Forwarded-Proto`, `effective host` and no longer `pending owner verification` / `(pending)`). Decision on the optional RMC-S7 extension: the existing `test_rmc_s7_operator_documentation_exists_and_covers_the_requirements` is **not** edited; RMC-S7b is a separate new test carrying the new needles, so no existing test changes in this cycle. |
| `Docs/REMOTE_COMPANION.md` *[R5: R4-4, R4-6, R4-7, R4-8]* | §2 trust bullets (lines 47–52): "The `Host` header must be…" → "The **effective host** (`X-Forwarded-Host` as set by `tailscale serve`, or `Host` for a direct local client) must be an allowed `*.ts.net` name…", and "`Origin` absent or equal to `https://<Host>`" → `https://<effective host>`; add that a proxied request must carry `X-Forwarded-Proto: https`, so the `--http` ban is enforced (`421`), not only documented. §4: the "Owner verification of the `Host` forwarding (pending)" section is **renamed** (no `(pending)` left) and rewritten with the verified behaviour (Serve sends `Host: localhost` and the real name in `X-Forwarded-Host`), in **redacted form only** (`<pc>.<tailnet>.ts.net`, `<login>`, `100.x.y.z`; never the real login, tailnet name, 100.x address or profile URL). §5 `allowed_hosts` row: "accepted in `Host`" → "accepted as the effective host (`X-Forwarded-Host`, or `Host`)". §7 `421` row: cause → "the effective host is not an allowed `*.ts.net` name / not in `allowed_hosts`, or `X-Forwarded-Proto` is missing or not `https` (`tailscale serve --http`, a port other than 443)"; fix → "use `tailscale serve --bg unix:` on port 443; set `allowed_hosts` if the name differs". §8 residual bullet "The `Host` forwarding … is pending owner verification" is **replaced** (not supplemented) by: "The host and transport checks rely on `tailscale serve` setting `X-Forwarded-Host` and `X-Forwarded-Proto` (verified with Tailscale 1.102.4); a Serve release that stops sending them makes every proxied request `421` (fail-closed, section 7), never `200`." Shortcuts section, see the next two rows. |
| Shortcuts section — factual fix *[R5: R4-6]* | `Self.DNSName` from `tailscale status --json` ends with a trailing dot (`arch.<tailnet>.ts.net.`), which `check_host` rejects (existing row `pc.tail1234.ts.net.` → `NotAllowed`): the doc must say "drop the trailing dot", or point to the name printed by `tailscale serve status` / shown in the Safari address bar. The list of possible `result` values gains `misdirected_request` and the sentence "any other value, or a `421`/`403`, is a configuration problem: see section 7". |
| Shortcuts section — unverifiable iOS claims *[R5: R4-7]* | Two statements cannot be checked from the repository and the agents have no iOS device: (a) *Get Contents of URL* `POST` with an empty *Request Body* sends no body (the default body type is JSON; an empty JSON body `{}` would be `413`); (b) a non-2xx response still feeds *Get Dictionary Value* instead of raising a Shortcuts error. **Resolution for this cycle: hedge the text.** The doc recommends *Request Body: File* with no file selected (the documented way to send an empty body), says "if the shortcut reports an error instead of a notification, the service refused the request (`403`, `421`, `413`): check section 7 and that the body is empty", and does not assert either iOS behaviour as fact. **Who verifies and how:** the owner, on their iPhone, by running the "Lock PC" shortcut once with the desktop unlocked and observing (1) the notification text `lock_requested`, (2) the desktop locking, and once more within 2 s observing `rate_limited` (a non-2xx body reaching the notification confirms (b)); the result is recorded as RMC21 evidence in the matrix and the hedge may then be removed in a follow-up. "Reviewed for accuracy" in the previous revision meant this review by the plan evaluator (R4-6, R4-7); nothing else is claimed. |
| `AI/DECISIONS.md` | ADR 2026-10-05 item (8) amended to D5a′: "requires the **effective host** (`X-Forwarded-Host` set by `tailscale serve`, otherwise `Host`) to be an allowed `*.ts.net` name and, on a proxied request, `X-Forwarded-Proto: https`; verified on the owner's host with Tailscale 1.102.4 on 2026-10-06; `tailscale serve --http` and `tailscale funnel` must not be used for it and are refused with `421`". |
| `AI/VERIFICATION_MATRIX.md` *[R5: R4-4]* | RMC4 text rewritten to D5a′ (effective host from `X-Forwarded-Host`, else `Host`; `Host` not inspected when XFH is present; mandatory `X-Forwarded-Proto: https` with XFH, optional-but-`https` without; same normalisation), evidence column extended with the new `identity_tests` and `server_tests` names. **RMC20 moved from ⬜ Pending to ✅ Verified** with the §13.1 evidence (owner's host, Tailscale 1.102.4, `tailscale serve --bg unix:`, 2026-10-06: `Host: localhost`, real name in `X-Forwarded-Host`, `X-Forwarded-Proto: https`, forged client headers overwritten; the end-to-end test reproduces the head); its text is reworded from "forwards the original `Host`" to "sets `X-Forwarded-Host` to the original `*.ts.net` name and `X-Forwarded-Proto: https`". **RMC21 stays ⬜ Pending** (phone check) and gains the R4-7 shortcut verification steps in its evidence column. RMC16 evidence gains RMC-S12; RMC19 evidence gains RMC-S7b. |
| `AI/walkthroughs/183_remote_companion.md` *[R5: R4-4, R4-8]* | Line 8 ("RMC20 and RMC21 pending owner checks") → "RMC20 verified 2026-10-06 (D5a′), RMC21 pending"; line 73 (evaluator required the Phase 4 verification) → add "performed 2026-10-06, see §9"; line 153 ("the `tailscale` host check is recorded as pending (constraint 35)") → "performed by the owner on 2026-10-06 (constraint 35 still forbids the agents to run `tailscale`)"; §9 records the §13.1 evidence **only in redacted form** (`arch.<tailnet>.ts.net`, `<redacted>` login, `100.x.y.z`, no profile URL) and D5a′. The redaction rule holds for every repository file: the walkthrough, `Docs/`, the matrix, test fixtures and commit messages. |
| `scripts/install_remote.sh` *[R5: R4-5]* | The git mode change `100644 → 100755` is already **staged** (index `100755`, `HEAD` `100644`) and lands in this cycle's commit; the documented `./scripts/install_remote.sh` invocation failed with "permission denied" on the `100644` checkout. RMC-S12 (§8) is the static contract (`tests/invariants/src/lib.rs` already asserts that `scripts/install.sh` is executable, so RMC-S12 is the first contract covering `scripts/install_remote.sh` and is redundant for `scripts/install.sh`, auditor A7; no invariant shells out to `git`). |

### 13.4 Revision 5 Change Log *[R5]*

| Finding | Resolution |
|---|---|
| R4-1 proto optional with XFH (MAJOR) | §13.2 "Transport": proto mandatory, exactly one, `https` case-insensitive OWS-trimmed when XFH present; optional-but-`https` otherwise; rows "XFH + proto absent → `NotAllowed`", "`Host` only + proto absent → OK"; e2e `421` without proto |
| R4-2 `Host` with XFH under-specified (MAJOR) | §13.2 "Effective host": `Host` not inspected when XFH present; `Missing` = neither header; fixed evaluation order; `HostError` doc comments; rows XFH + `Host` absent / repeated → OK, neither → `Missing` |
| R4-3 test power | §13.2 "Normalisation" explicit; rows: proto `https` twice, `HTTPS`, `" https "`, `" PC.Tail1234.TS.NET:443 "`, `:8443`, `100.64.0.1`, allowlist cross rows; e2e verbatim head, `202` with `Origin: https://<xfh>`, `403` with `Origin: https://localhost` |
| R4-4 docs / matrix drift list | §13.3 rows for `Docs/REMOTE_COMPANION.md` §2, §4, §5, §7, §8; RMC4 text; RMC20 → ✅ with §13.1 evidence; RMC21 pending; walkthrough lines 8, 73, 153; RMC-S7b instead of editing RMC-S7 |
| R4-5 script mode contract | §8 RMC-S12 (`std::fs::metadata(...).permissions().mode() & 0o111 != 0`, both installer scripts, no `git` subprocess); staged mode change noted |
| R4-6 Shortcuts factual nit | trailing dot of `Self.DNSName` dropped / `tailscale serve status`; `misdirected_request` listed with a pointer to section 7 |
| R4-7 unverifiable iOS claims | text hedged (*Request Body: File*, error sentence); owner verification procedure named (who, how, recorded as RMC21 evidence) |
| R4-8 redaction | redaction rule for every repository file; e2e fixtures use `pc.tail1234.ts.net` / `owner@example.com` / `100.64.0.1` |
| R4-9 trust wording | §13.2 "Trust argument" names the three overwritten headers; `X-Forwarded-For` stated as ignored by the service |
