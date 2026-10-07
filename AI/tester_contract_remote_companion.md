# Tester Contract — GitHub #339: Remote Companion (`soos-remote`)

- **Phase**: 2 (Tester Agent, TDD Red), branch `feat/remote-companion`, base `222665f`
- **Spec**: `AI/architect_spec_remote_companion.md` revision 3 (`APPROVED`, `AI/plan_evaluator_report.md` round 3), plus the two round-3 MINOR findings R3-1 (`seq` reserved when a read **starts**; `checked_unix_ms` from the clock injected through `ServerState`, never `SystemTime::now`) and R3-2 (RMC-S9 also forbids `serve_at`, `request_name`, `#[interface`, `SignalStream`, `zbus::blocking`, `Address::system`).
- **Criteria**: spec §7 test hooks, §8 static contracts RMC-S1–RMC-S11, §2.12 contract migration, invariants RC-1–RC-5, decisions D1–D12, §4 error taxonomy, §3 constants.
- **Total**: **115 new tests** (`test_rmc_*` / `prop_rmc_*`): 97 in `crates/remote/tests/` (7 files, 5 of them proptests) and 18 in `tests/invariants/src/remote_companion_contract.rs`, plus **1 migrated existing invariant** (§2.12).

All the tests are hermetic: configuration files, socket directories and sockets live in `tempfile` directories; logind is `MockSource` (no D-Bus); the HTTP client is raw bytes over `tokio::net::UnixStream`; the Unix clock is injected. Nothing touches `$XDG_RUNTIME_DIR`, `$HOME`, `/run`, `/etc` or a real bus.

## Scaffolding created in Phase 2 (the skill's stub rule)

So that every test fails on its **assertion** (or on exactly the specified API), the crate exists as signatures only:

| Path | Content |
|---|---|
| `Cargo.toml` | `members += "crates/remote"`; `[workspace.dependencies]` += `httparse = "1.10"`, `soos-remote = { path = "crates/remote", version = "0.1.0" }` (spec §1.1) |
| `Cargo.lock` | one new `[[package]] soos-remote` entry, **no new external crate** (verified by `cargo deny --locked check`: ok) |
| `crates/remote/Cargo.toml` | spec §1.1 keys; deps `tokio nix tracing tracing-subscriber thiserror serde serde_json toml clap httparse zbus` (all `workspace = true`); dev-deps `tokio` + `test-util`, `tempfile`, `proptest = "1"`; `[lints] workspace = true` |
| `crates/remote/src/lib.rs` | `#![forbid(unsafe_code)]`, the ten `pub mod`, every §3 constant with its specified value (single source) |
| `crates/remote/src/main.rs` | `#![forbid(unsafe_code)]`; exits `EXIT_RUNTIME` and does nothing else (RMC-S10 is red) |
| `config.rs`, `identity.rs`, `http.rs`, `routes.rs`, `session.rs`, `logind.rs`, `status.rs`, `socket.rs`, `assets.rs`, `server.rs` | every spec type, enum, error variant and function signature; each function returns a deliberately wrong value (`None`, `Err(first variant)`, empty `Vec`, `NotFound`, `false`, …); `serve` accepts and drops every connection until `shutdown` resolves |

No `assets/`, `packaging/soos-remote.service`, `scripts/install_remote.sh` or `Docs/REMOTE_COMPANION.md` exists yet: the matching invariants assert existence first and are red with a clear message.

## Test doubles (`crates/remote/tests/server_tests.rs`)

| Double | Spec §7 hook | Notes |
|---|---|---|
| `MockSource` | `SessionSource` mock with scripted `own_sessions` and a spy on `lock_session` | Settable answer (`set_unlocked`/`set_locked`/`set_no_session`/`set_sessions`/`set_error`), settable `lock_session` result, `hold_next(n)` / `hold_lock_next(n)` gates that block the next *n* calls until `release()` (the answer is snapshotted when the call **starts**, modelling a slow read), counters (`reads()`, `wait_reads_at_least`), recorded `lock_ids()` and `uids()`. |
| `TestClock` | "clocks" held by `ServerState` | `BASE_UNIX_MS + virtual elapsed + offset`; `shift_ms(±)` for the backward-clock test. Injected through `ServerState::with_unix_clock`. |
| `LogCapture` | RC-5 | `tracing_subscriber::fmt` writer at `TRACE`, scoped with `set_default` on the test thread. |
| `FrozenClock` | paused time | A live `spawn_blocking` task: tokio inhibits auto-advance while it runs (`runtime/blocking/schedule.rs`), released on drop or after 20 s of real time. |

### Deterministic virtual time (important for the developer)

Under `#[tokio::test(start_paused = true)]` the current-thread scheduler **auto-advances the paused clock to the next timer whenever an I/O wake-up arrives during a park** (the I/O wake is pushed to the local queue without setting the time driver's `did_wake`): an instrumented run showed a plain `UnixStream::connect` jumping virtual time by 118 s. With a real server holding a 5 s head timer, that would fire the timer in the middle of a request. The harness therefore **freezes** the clock (`FrozenClock`) and moves it only with `tokio::time::advance` (`Harness::advance_ms`, `step_ms`): I/O is real and costs zero virtual time; timers fire exactly when the test says so. A scratch probe (200 request/response exchanges against a tiny tokio server with a 5 s per-connection timeout) confirmed: elapsed virtual time 0 after the exchanges, the idle connection alive at 4999 ms and closed at exactly 5000 ms. Consequences for the implementation: time-based behaviour must use `tokio::time` (`sleep`, `timeout`, `Instant`) — a `std::time` or `SystemTime` deadline would never fire under the frozen clock, and `checked_unix_ms` must come from the injected clock (R3-1).

## API the tests rely on that the spec names but does not fully specify (binding for the developer)

```rust
// server.rs
pub type UnixClock = Arc<dyn Fn() -> u64 + Send + Sync>;        // checked_unix_ms source
pub struct ServerState<S: SessionSource> { /* private */ }
impl<S: SessionSource> ServerState<S> {
    pub fn new(config: RemoteConfig, uid: u32, source: S) -> Self; // production: SystemTime clock, counter at 0
    pub fn with_unix_clock(self, clock: UnixClock) -> Self;        // test hook (R3-1)
    pub fn with_seq_start(self, seq_start: u64) -> Self;           // test hook: value the shared reading
        // counter holds before the first reservation; a reservation is `checked_add(1)`, so
        // `u64::MAX` makes the very first one fail and ends the poller (spec §2.6 overflow rule)
    pub fn config(&self) -> &RemoteConfig; pub fn uid(&self) -> u32; pub fn source(&self) -> &S;
    pub fn unix_clock(&self) -> &UnixClock; pub fn seq_start(&self) -> u64;
}
#[derive(Debug, thiserror::Error)]
pub enum ServeError { PollerEnded, Accept(std::io::ErrorKind), TaskPanicked }
pub async fn serve<S: SessionSource>(listener: UnixListener, state: Arc<ServerState<S>>,
    shutdown: impl Future<Output = ()> + Send) -> Result<(), ServeError>;
    // clean shutdown → Ok(()) and the socket file at `config.socket_path` is removed (§2.7);
    // the poller ending → Err(ServeError::PollerEnded)

// assets.rs
pub enum AssetId { Index, AppJs, StyleCss, Manifest, IconSvg, AppleTouchIcon }
pub struct Asset { pub content_type: &'static str, pub body: &'static [u8] }
pub fn asset(id: AssetId) -> Asset;

// logind.rs
pub enum SourceError { BusUnavailable, Timeout, Call, Malformed, TooManySessions } // Call carries no text
impl ZbusSessionSource { pub fn new() -> Self; pub async fn is_connected(&self) -> bool; }

// socket.rs
pub enum SocketError { NotADirectory, WrongOwner, NotASocket, Io(std::io::ErrorKind) }
```

Module paths used by the tests: `soos_remote::config::{TailscaleLogin, RemoteConfig, ConfigError, check_not_root, parse_config, load_config, default_config_path}`, `soos_remote::identity::{authorize, check_host, AuthError, HostError}`, `soos_remote::http::{Method, RequestHead, HttpError, Response, parse_request_head, encode_response, encode_sse_head, encode_sse_event}`, `soos_remote::routes::{Route, route, CsrfError, check_lock_csrf}`, `soos_remote::assets::{AssetId, Asset, asset}`, `soos_remote::session::{SessionProps, session_props_from_properties, select_session}`, `soos_remote::logind::{SessionSource, SourceError, ZbusSessionSource}`, `soos_remote::status::{SessionState, StatusView, Reading, status_from, view_changed}`, `soos_remote::socket::{SocketError, prepare_socket_dir, bind_listener}`, `soos_remote::server::{ServerState, ServeError, UnixClock, serve}`; the §3 constants from the crate root. `RemoteConfig` fields are `pub` and built directly by the server harness.

### Spec ambiguities resolved in Phase 2 (binding)

1. **Streams subscribe before their first read.** A stream registers as a subscriber (waking the poller) *before* performing its own fresh first read; otherwise no change between that read and the first channel value could be caught, and `test_rmc_poller_ending_makes_serve_return_an_error` relies on the poller being woken by a stream whose own reservation fails.
2. **`env::var` scope of RMC-S9.** The spec forbids `env::var` in every file, but `main.rs` must read `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME` and `HOME` (§2.3). The contract forbids `env::var`, `env::vars`, `std::env` and `env!(` in every file **except `main.rs`**, and in `main.rs` every `env::var*("…")` literal must be one of those three names; `DBUS_*_BUS_ADDRESS` is forbidden everywhere including `main.rs`.
3. **`default_config_path`**: an empty or relative `XDG_CONFIG_HOME` is treated as unset (XDG rule) and falls back to `$HOME/.config/soos/remote.toml`; an unset, empty or relative `HOME` then gives `NoConfigPath`.
4. **Non-numeric or negative `Content-Length`** → `HttpError::Malformed` (400); any strictly positive value → `BodyNotAllowed` (413); `0` is accepted. `Transfer-Encoding` of any value → `BodyNotAllowed` (400 on the wire).
5. **Request target**: only origin-form (`/…`) parses; `*`, absolute-form (`http://…`) and a path without a leading `/` → `Malformed`. The path is kept verbatim (no percent-decoding, no dot-segment normalisation); those forms simply do not route (`404`). A path over `MAX_PATH_LEN` **without the query** → `PathTooLong`; the query is not counted.
6. **Head size**: any buffer longer than `MAX_REQUEST_HEAD_BYTES` → `HeadTooLarge` even when complete; a buffer of exactly the bound that is still incomplete → `HeadTooLarge` (never `Incomplete`); a complete head of exactly the bound parses.
7. **`route` ignores query strings itself** (`route(Get, "/api/status?x=1") == Status`), as its spec doc comment says, in addition to the parser stripping them.
8. **`Allow` header** values: `GET, HEAD` for the read routes, `POST` for `/api/lock`. Status phrases: `200 OK`, `202 Accepted`, `400 Bad Request`, `403 Forbidden`, `404 Not Found`, `405 Method Not Allowed`, `409 Conflict`, `414 URI Too Long`, `421 Misdirected Request`, `429 Too Many Requests`, `431 Request Header Fields Too Large`, `503 Service Unavailable`; `413` only needs the `HTTP/1.1 413 ` prefix.
9. **Error bodies**: every `403` (identity or CSRF) carries `{"result":"forbidden"}` as `application/json`; `/api/lock` bodies as in the route table; the `/api/events` `503` only needs the status and the mandatory headers; `404`/`405` bodies are free (mandatory headers and `Content-Length` still required). No `Server` or `Date` header is ever sent.
10. **`check_lock_csrf`**: `X-Soos-Action` must be exactly `lock` (any other value counts as missing); `Sec-Fetch-Site` `none`, `same-site`, `cross-site`, empty or malformed → `CrossSite`; a repeated `Origin`, `Origin: null`, a trailing `/`, a non-`https` scheme or a port other than `443` → `OriginMismatch`; the comparison target is the normalized host returned by `check_host`.
11. **`check_host`**: `ts.net` itself, a leading dot, a trailing dot, an IP literal (v4 or bracketed v6), an empty port (`host:`), any port but `443`, and any name over `MAX_HOST_LEN` are `NotAllowed`; optional whitespace around the value is trimmed; the same normalisation (lowercase, `:443` stripped) applies when `allowed_hosts` is configured; a configured list may hold non-`.ts.net` names.
12. **Config `allowed_hosts` validation**: labels `[a-z0-9-]` after lowercasing, no leading/trailing `-`, no empty label, no port, no scheme, no trailing dot, 1..=`MAX_HOST_LEN` bytes.
13. **`socket_path`**: `""`, `/`, a trailing `/`, a relative path and more than 107 bytes → `InvalidSocketPath`; a runtime dir so long that the default `<dir>/soos-remote/remote.sock` exceeds 107 bytes → `InvalidSocketPath` too.
14. **`load_config`**: a directory (EISDIR) is `Unreadable`, only ENOENT is `NotFound`; the file is read through `take(MAX_CONFIG_BYTES + 1)` so an 8× oversize file is `TooLarge` without being read.
15. **`IdleSinceHint`** is integer-divided by 1 000 000 (999 999 µs → `Some(0)`; `0` → `None`; a non-`u64` → `None`); a non-boolean `Remote` is `None` (unknown, never `false`); a non-structure `User` → `uid: None`.
16. **`select_session` tie-break** is `(id.len(), id.bytes())`, so `"9" < "10"` and `"c9" < "10"`; class comparison is exact (`"User"` is not `"user"`).
17. **`bind_listener`** refuses a symlink, a regular file, a directory and a socket not owned by the given `uid` with `NotASocket` and never unlinks them; a missing parent is `Io(NotFound)`. **`prepare_socket_dir`** creates one missing level only (`Io(_)` otherwise), tightens an existing own directory to `0700`, and refuses a symlink or a file (`NotADirectory`) or a foreign owner (`WrongOwner`) without touching their mode.
18. **Lock rate limit** is measured with `tokio::time::Instant`: a second lock 1999 ms after an accepted one is `429`, 2001 ms after is `202` (exactly 2000 ms is not asserted).
19. **Keep-alive** cadence is measured from the previous event: the tests accept the keep-alive within [14 s, 16 s] after the last event sent, and require every re-sent reading to be at most `SSE_KEEPALIVE_MS` old.
20. **Hung reads** are cut at exactly `SNAPSHOT_DEADLINE_MS` after the read started (`unavailable` at 1500 ms, not at 1499 ms); a hung lock flow is cut within `LOCK_FLOW_DEADLINE_MS`; neither is retried.
21. **Connection limit**: the 17th accepted stream is dropped with zero bytes; the 16 idle ones stay open until exactly `REQUEST_HEAD_TIMEOUT_MS` after accept and are then closed with zero bytes.
22. **Static RMC-S6 reading of "quoted instruction text"**: a line of `scripts/install_remote.sh` that mentions `sudo`, `tailscale` or `systemctl --user enable` must start with `echo` or `printf` (heredocs are not accepted by the contract).
23. **Extra RMC-S3 literals** forbidden alongside the three of the spec: `"UnlockSessions"`, `"TerminateSession"`, `"KillSession"`, `"ActivateSession"`; exactly one `"LockSession"` literal exists, in `logind.rs`.
24. **RMC-S9 positive checks**: `logind.rs` (comments stripped) contains `zbus::connection::Builder::address(` exactly once in the crate, `SYSTEM_BUS_ADDRESS`, `.max_queued(`, `DBUS_CALL_TIMEOUT_MS` and `DBUS_CONNECT_TIMEOUT_MS`; `::system(` and `::session(` are also forbidden (presence parity).
25. **RMC-S10 positive checks**: `main.rs` calls `getuid()` and `geteuid()`, `check_not_root(` before `load_config(`, `default_config_path(`, `new_current_thread()`, names `EXIT_CONFIG` and `EXIT_RUNTIME`, never `new_multi_thread` or `#[tokio::main`.
26. **Logging hygiene**: no tracing macro (`trace!` to `error!`) in production code may use a field key in {`login`, `logins`, `host`, `hosts`, `origin`, `header`, `headers`, `body`, `session`, `session_id`, `sid`, `id`, `user`, `user_name`, `username`, `name`, `value`, `values`, `identity`, `request_path`, `target`, `uri`, `url`} or ending in `_login`/`_host`/`_header`/`_id`/`_name`, nor inline captures such as `{login`, `{host`, `{path`; `SystemTime::now` may appear only in `main.rs` or `server.rs` (the production clock); production code never `.unwrap()`/`.expect(`/prints and never `allow`s the panic, indexing, arithmetic or print lints.
27. **UI constants and strings asserted** (`app.js`): `STALE_UI_MS = 45000`, `LOCK_CONFIRM_UI_MS = 5000`, `EventSource`, `visibilitychange`, `pageshow`, `textContent`, `X-Soos-Action`, the three API paths, `Unreachable`, `Lock now`, `Lock requested`, `LockedHint unchanged`; no `innerHTML`/`outerHTML`/`insertAdjacentHTML`/`document.write`/`eval(`/`new Function`/`http://`/`https://`. `index.html` starts with a doctype, has no inline `<script>` body, no inline `on*=` handler, references `app.js`, `style.css`, `manifest.webmanifest`, `apple-touch-icon.png`, `apple-mobile-web-app-capable` and the plain source link `<a href="https://github.com/Mysticaly622/soos">`; `style.css` has no remote `url(`, no `@import`, uses `prefers-color-scheme`; the manifest has `"display": "standalone"`, `"start_url": "/"`, `"scope": "/"`, `icon.svg`; `apple-touch-icon.png` is a 180×180 PNG (IHDR parsed).
28. **Documentation needles** (Phase 6 deliverables, red until then): `Docs/REMOTE_COMPANION.md` contains `tailscale serve --bg unix:`, `allowed_logins`, `allowed_hosts`, `LockedHint`, `Tailscale-User-Login`, `remote unlock`, `systemctl --user`, `soos-remote.service`, `install_remote.sh`, `RestrictAddressFamilies=AF_UNIX`, `funnel`, "out of scope", "push notification", "live camera"; `Docs/README.md` indexes it; `AI/ARCHITECTURE.md` contains "Remote Companion" and `soos-remote`; `AI/MOCK_STRATEGY.md` has "Remote Companion Doubles"; the `` `zbus` 5 `` line of `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` names `soos-remote`; `project-facts.md` lists `` `crates/remote` ``; `AI/DECISIONS.md` contains "Remote Companion `soos-remote`" and "GitHub #339".

## Tester Contract — GitHub #339

### Counts per spec element

| Spec element | Tests | Files |
|---|---|---|
| §3 constants (guard) | 1 | `config_tests.rs` |
| §2.3 `TailscaleLogin`, `parse_config`, `load_config`, `default_config_path`, `check_not_root` (D1) | 17 (1 proptest) | `config_tests.rs` |
| §2.4 `authorize` (D3, RC-2), `check_host` (D5a) | 9 | `identity_tests.rs` |
| §2.5 `parse_request_head`, `encode_response`, `encode_sse_*` (D5, §2.8 headers, RC-4) | 16 (4 proptests) | `http_tests.rs` |
| §2.5 `route`, `check_lock_csrf` (D4), `asset` (D11) | 7 | `routes_tests.rs` |
| §2.6 `session_props_from_properties`, `select_session` (D7), `status_from` (D8), `view_changed`, JSON shape (RC-5) | 10 | `session_tests.rs` |
| §2.7 `prepare_socket_dir`, `bind_listener` (D2) | 9 | `socket_tests.rs` |
| §2.8 server: Host/identity/HTTP errors/headers, status, routing, assets | 10 | `server_tests.rs` |
| §2.8 lock flow (D9), CSRF end to end (D4), bodies, deadlines (F8) | 4 | `server_tests.rs` |
| §2.1 D6 / §7 mandatory paused-time tests: first fresh read, change-only, keep-alive (R2-1), stale replay (F2), poller gating, poll interval, slot release (F6), max stream, poller end, backward clock (R2-4), `seq` at read start (R3-1) | 11 | `server_tests.rs` |
| §2.8 connection limit, head timeout, shutdown (§2.7), logging (RC-5, D12) | 4 | `server_tests.rs` |
| §8 RMC-S1–RMC-S11 (+ R3-2), workspace registration, lint and logging hygiene, documentation | 18 | `tests/invariants/src/remote_companion_contract.rs` |
| §2.12 migration | 1 (migrated) | `tests/invariants/src/presence_unlock_contract.rs` |

### Contract table

Red evidence codes:
- **A**: assertion failure against the Phase 2 stubs (message quoted in the red evidence section).
- **A-S**: `server_tests.rs` — against the stubs the harness setup panics first (`TailscaleLogin::parse(LOGIN)` is `None`: "called `Option::unwrap()` on a `None` value", `server_tests.rs` `Harness::start_with`); with a temporary lowercasing `parse` (scratch run only, stub restored byte-identical afterwards) every test reaches its own assertion, quoted below.
- **G**: passes against the stubs (guard): constants, fixed error messages, serde shape, or a refusal that the stub's wrong default happens to produce. Each one still fails against a plausible wrong implementation (a changed constant, an echoed header value, an accepted bad login).
- **F**: red on a missing Phase 4/6 deliverable (file existence asserted first).

| Test (path::name) | Spec element | Red |
|---|---|---|
| `crates/remote/tests/config_tests.rs::test_rmc_constants_match_the_spec` | §3 | G |
| `…::test_rmc_login_parse_accepts_and_lowercases_printable_ascii` | §2.3 `TailscaleLogin::parse` | A |
| `…::test_rmc_login_parse_rejects_empty_oversize_and_forbidden_bytes` | §2.3, `MAX_LOGIN_LEN` | G |
| `…::prop_rmc_login_parse_never_panics_and_is_idempotent` | §2.3 | G |
| `…::test_rmc_parse_config_minimal_defaults` | §2.3 defaults | A |
| `…::test_rmc_parse_config_full_file` | §2.3 dedup/lowercase/hosts | A |
| `…::test_rmc_parse_config_rejects_missing_or_empty_logins` | D3, RC-2 `NoAllowedLogins` | A |
| `…::test_rmc_parse_config_login_count_bound` | `MAX_ALLOWED_LOGINS` | A |
| `…::test_rmc_parse_config_rejects_invalid_login_with_index` | `InvalidLogin { index }` | A |
| `…::test_rmc_parse_config_poll_interval_bounds` | `PollIntervalOutOfRange` (never clamped) | A |
| `…::test_rmc_parse_config_rejects_unknown_fields_and_bad_toml` | `deny_unknown_fields`, `Syntax` | G |
| `…::test_rmc_parse_config_socket_path_rules` | `InvalidSocketPath`, 107 bytes | A |
| `…::test_rmc_parse_config_runtime_dir_rules` | `NoRuntimeDir`, no `/tmp` fallback | A |
| `…::test_rmc_parse_config_host_rules` | D5a `TooManyHosts` / `InvalidHost` | A |
| `…::test_rmc_check_not_root_table` | D1 / RMC-S10 `check_not_root` | A |
| `…::test_rmc_default_config_path_prefers_xdg_config_home` | §2.3 `default_config_path` | A |
| `…::test_rmc_load_config_reads_a_bounded_file` | `MAX_CONFIG_BYTES`, `NotFound`, `Unreadable`, `TooLarge` | A |
| `…::test_rmc_config_error_messages_are_fixed_english_text` | §4 | G |
| `crates/remote/tests/identity_tests.rs::test_rmc_authorize_accepts_exactly_one_allowlisted_login_case_insensitively` | D3 | A |
| `…::test_rmc_authorize_rejects_missing_header` | RC-2 `Missing` | A |
| `…::test_rmc_authorize_rejects_repeated_header` | D3 `Repeated` | A |
| `…::test_rmc_authorize_rejects_malformed_values` | `Malformed` | A |
| `…::test_rmc_authorize_rejects_unknown_login_and_never_matches_substrings` | RC-2 `NotAllowed` | A |
| `…::test_rmc_check_host_requires_exactly_one_host_header` | D5a `Missing` / `Repeated` | A |
| `…::test_rmc_check_host_table_without_allowlist` | D5a `*.ts.net`, normalisation | A |
| `…::test_rmc_check_host_table_with_allowlist` | D5a `allowed_hosts` | A |
| `…::test_rmc_identity_errors_never_echo_values` | §4 | G |
| `crates/remote/tests/http_tests.rs::test_rmc_parse_request_head_nominal` | §2.5 `RequestHead` | A |
| `…::test_rmc_parse_request_head_methods` | `Method` | A |
| `…::test_rmc_parse_request_head_incomplete` | `Incomplete` | G |
| `…::test_rmc_parse_request_head_rejects_http10_h2_preface_and_garbage` | `Malformed` (400) | A |
| `…::test_rmc_parse_request_head_too_large` | `HeadTooLarge` (431) | A |
| `…::test_rmc_parse_request_head_too_many_headers` | `TooManyHeaders` (431) | A |
| `…::test_rmc_parse_request_head_rejects_bodies` | `BodyNotAllowed` (413/400) | A |
| `…::test_rmc_parse_request_head_path_length_bound` | `PathTooLong` (414) | A |
| `…::test_rmc_parse_request_head_strips_query_and_keeps_path_verbatim` | §2.5 | A |
| `…::prop_rmc_parse_request_head_never_panics` | RC-4 | A |
| `…::prop_rmc_parse_request_head_round_trips_header_values` | §2.5 | A |
| `…::prop_rmc_parse_request_head_any_positive_content_length_is_a_body` | D5 | A |
| `…::test_rmc_encode_response_has_mandatory_headers_and_content_length` | §2.8 mandatory headers | A |
| `…::test_rmc_encode_response_status_lines` | §2.5 route table statuses | A |
| `…::test_rmc_encode_sse_head_and_event` | §2.5 `encode_sse_*` | A |
| `…::prop_rmc_encode_response_length_matches_body` | §2.5 | A |
| `crates/remote/tests/routes_tests.rs::test_rmc_route_table` | §2.5 route table | A |
| `…::test_rmc_route_wrong_method_and_unknown_paths` | 404 / 405 | A |
| `…::test_rmc_assets_table_content_types_and_bodies` | D11 | A |
| `…::test_rmc_check_lock_csrf_requires_the_action_header` | D4 `MissingActionHeader` | A |
| `…::test_rmc_check_lock_csrf_sec_fetch_site` | D4 `CrossSite` | A |
| `…::test_rmc_check_lock_csrf_origin` | D4 / R2-6 `OriginMismatch` | A |
| `…::test_rmc_csrf_error_messages` | §4 | G |
| `crates/remote/tests/session_tests.rs::test_rmc_session_props_from_get_all_reply_nominal` | §2.6 mapping | A |
| `…::test_rmc_session_props_malformed_id` | `Malformed` | G |
| `…::test_rmc_session_props_tolerates_missing_or_ill_typed_optionals` | §2.6 | A |
| `…::test_rmc_session_props_validates_the_id` | `MAX_SESSION_ID_LEN` | A |
| `…::test_rmc_select_session_prefers_active_local_user_seat_session` | D7 tie-break | A |
| `…::test_rmc_select_session_excludes_remote_greeter_seatless_and_other_uids` | D7 exclusions | A |
| `…::test_rmc_status_from_maps_results_fail_safe` | D8, RC-3 | A |
| `…::test_rmc_view_changed_ignores_checked_unix_ms_only` | D6 `view_changed`, `Reading` | A |
| `…::test_rmc_status_view_json_shape_has_no_identity` | RC-5 | G |
| `…::test_rmc_source_error_messages_are_fixed` | §4 | G |
| `crates/remote/tests/socket_tests.rs::test_rmc_prepare_socket_dir_creates_0700_when_absent` | §2.7 | A |
| `…::test_rmc_prepare_socket_dir_refuses_two_missing_levels` | §2.7 | A |
| `…::test_rmc_prepare_socket_dir_tightens_an_existing_directory` | §2.7 | A |
| `…::test_rmc_prepare_socket_dir_refuses_symlink_and_regular_file` | §2.7 `NotADirectory` | A |
| `…::test_rmc_prepare_socket_dir_refuses_a_foreign_owner` | §2.7 `WrongOwner` | A |
| `…::test_rmc_bind_listener_binds_0600_and_replaces_a_stale_socket` | D2 `0600` | A |
| `…::test_rmc_bind_listener_refuses_non_socket_paths_without_unlinking` | §2.7 `NotASocket` | A |
| `…::test_rmc_bind_listener_in_a_missing_directory_is_io` | §4 `Io` → `EXIT_RUNTIME` | A |
| `…::test_rmc_socket_error_messages` | §4 | G |
| `crates/remote/tests/server_tests.rs::test_rmc_host_is_checked_before_identity_and_routing` | D5a, §2.8 order, 421 | A-S |
| `…::test_rmc_allowed_hosts_config_is_enforced_end_to_end` | D5a / R2 F5 | A-S |
| `…::test_rmc_identity_is_required_before_routing` | RC-2, §4 403 body | A-S |
| `…::test_rmc_http_errors_map_to_statuses` | §4 400/413/414/431 | A-S |
| `…::test_rmc_every_response_carries_the_mandatory_headers` | §2.8 | A-S |
| `…::test_rmc_status_is_a_fresh_read_and_fails_safe` | D8, RC-3, RC-5 | A-S |
| `…::test_rmc_head_returns_headers_without_body` | §2.5 HEAD | A-S |
| `…::test_rmc_unknown_route_and_wrong_method` | 404 / 405 `Allow` | A-S |
| `…::test_rmc_assets_are_served_with_content_types` | D11 | A-S |
| `…::test_rmc_lock_flow_and_rate_limit` | D9, §2.8 lock, `MIN_LOCK_INTERVAL_MS` | A-S |
| `…::test_rmc_lock_requires_csrf_headers` | D4 end to end | A-S |
| `…::test_rmc_lock_with_a_body_is_refused_before_any_logind_call` | D5 | A-S |
| `…::test_rmc_deadlines_bound_hung_logind_calls` | R2 F8 `SNAPSHOT_DEADLINE_MS`, `LOCK_FLOW_DEADLINE_MS` | A-S |
| `…::test_rmc_events_first_event_is_a_fresh_read` | D6 | A-S |
| `…::test_rmc_events_emit_on_change_only` | D6 | A-S |
| `…::test_rmc_events_keepalive_resends_with_advancing_timestamp` | R3 / R2-1 mandatory | A-S |
| `…::test_rmc_events_new_stream_never_replays_a_stale_unlocked` | R2 F2 mandatory, RC-3 | A-S |
| `…::test_rmc_poller_runs_only_while_a_stream_is_open` | D6 | A-S |
| `…::test_rmc_poll_interval_from_config_drives_the_poller` | §2.3 `poll_interval_ms` | A-S |
| `…::test_rmc_events_stream_limit_and_slot_release` | `MAX_SSE_STREAMS`, R2 F6 | A-S |
| `…::test_rmc_events_stream_closes_after_max_stream_ms` | `MAX_SSE_STREAM_MS` | A-S |
| `…::test_rmc_poller_ending_makes_serve_return_an_error` | R2 F2 mandatory, §2.8 supervision | A-S |
| `…::test_rmc_backward_wall_clock_does_not_silence_changes` | R2-4 / R3 mandatory | A-S |
| `…::test_rmc_seq_is_reserved_when_a_read_starts` | R3-1 | A-S |
| `…::test_rmc_connection_limit_drops_excess_without_response` | `MAX_CONNECTIONS`, `REQUEST_HEAD_TIMEOUT_MS` | A-S |
| `…::test_rmc_request_head_timeout_closes_without_response` | `REQUEST_HEAD_TIMEOUT_MS` | A-S |
| `…::test_rmc_shutdown_removes_the_socket_and_ends_streams` | §2.7 / §2.8 shutdown | A-S |
| `…::test_rmc_server_never_logs_identity_or_request_data` | RC-5, D12 | A-S |
| `tests/invariants/src/remote_companion_contract.rs::test_rmc_crate_is_registered_in_the_workspace` | §1.1 | G |
| `…::test_rmc_manifest_inherits_workspace_keys_and_test_util` | §1.1, R2 F4/F9 | G |
| `…::test_rmc_forbid_unsafe_list_and_review_tooling_include_remote` | §1.1 `candid_review.sh` | A |
| `…::test_rmc_s1_lib_and_main_forbid_unsafe_code` | RMC-S1 | G |
| `…::test_rmc_s2_no_network_socket_type_is_named` | RMC-S2 | G |
| `…::test_rmc_s3_no_unlock_or_locked_hint_literal` | RMC-S3, D9 | A |
| `…::test_rmc_s4_remote_is_a_leaf_crate` | RMC-S4 | G |
| `…::test_rmc_s5_s11_user_unit_is_sandboxed_and_never_retries_config_errors` | RMC-S5, RMC-S11 | F |
| `…::test_rmc_s6_installer_refuses_root_and_never_runs_privileged_commands` | RMC-S6 | F |
| `…::test_rmc_s7_operator_documentation_exists_and_covers_the_requirements` | RMC-S7 | F |
| `…::test_rmc_architecture_and_strategy_documents_describe_the_companion` | §1.1 docs, ADR | A |
| `…::test_rmc_s8_assets_exist_load_nothing_remote_and_have_no_inline_script` | RMC-S8, D11 | F |
| `…::test_rmc_s8_app_js_is_textcontent_only_and_carries_the_ui_constants` | RMC-S8, §2.11 | F |
| `…::test_rmc_s9_zbus_manifest_line_and_pinned_system_bus` | RMC-S9 | A |
| `…::test_rmc_s9_bus_rules_match_the_presence_worker` | RMC-S9 + R3-2 needles | G |
| `…::test_rmc_s10_main_refuses_root_before_loading_configuration` | RMC-S10, D1, §2.2 | A |
| `…::test_rmc_production_code_never_panics_or_prints` | D12, R3-1 clock | G |
| `…::test_rmc_logging_never_names_identity_header_or_session_fields` | RC-5 | G |

### Migrated existing tests

| Test | Old assertion | New assertion | Mandating acceptance line |
|---|---|---|---|
| `tests/invariants/src/presence_unlock_contract.rs::test_pau_zbus_is_used_only_by_the_daemon` | `crates/daemon/Cargo.toml` declares `zbus = { workspace = true }`; every other manifest under `crates/` and `tests/` contains no `zbus`; PAM has no `zbus`/`dbus` | **exactly** `{crates/daemon/Cargo.toml, crates/remote/Cargo.toml}` declare `zbus = { workspace = true }` and name `zbus` exactly once each (no feature override); both must exist; every other manifest still contains no `zbus`; the PAM assertion is unchanged; name kept (matrix PAU17) | Spec §2.12 / ADR 2026-10-05 item (7), GitHub #339 (owner-approved scope change of the 2026-10-02 "zbus is daemon-only" decision). Phase 6 adds the PAU17 italic annotation *(scope widened to soos-remote by ADR 2026-10-05, GitHub #339)*. |

No other existing test was edited. `test_business_crates_forbid_unsafe_code` (`tests/invariants/src/lib.rs`) gained `"remote"` in its list, which only **strengthens** it (spec §1.1).

### Pre-existing tests that are red because the crate is registered (developer / traceability, not a contract change)

| Test | Why red | Who fixes it |
|---|---|---|
| `tests/invariants/src/maintainer_hygiene_contract.rs::test_workspace_maps_list_every_member_and_real_tests_dirs` | `AGENTS.md` and `AI/ARCHITECTURE.md` workspace trees must list `── remote/` (GitHub #239) | Phase 4/6 documentation (spec §1.1 lists both files) |

### Flakiness check

- `crates/remote/tests/server_tests.rs` (28 tests, all time-sensitive) run 10× in a row: 10/10 identical outcomes, identical failure-message set (md5 `75b835ee…` for every run), wall-clock `0.00s` per run against the stubs. The suite never sleeps on real time: the 20 s `WALL_CLOCK_FALLBACK` only engages when the server never answers (observed in the scratch run for the two tests that wait on the server — `test_rmc_deadlines_bound_hung_logind_calls`, `test_rmc_poller_ending_makes_serve_return_an_error` — which then failed cleanly on "within the bound: Elapsed(())").
- Frozen-clock mechanism validated against a scratch tokio server (not in the repository): 200 exchanges → 0 ms virtual; 5 s per-connection timer alive at 4999 ms, fired at 5000 ms.
- The pure suites and the invariants are deterministic by construction (no clock, no I/O beyond tempdirs).

### Red evidence (observed)

Commands: `cargo test --locked -p soos-remote --all-features --no-fail-fast` and `cargo test --locked -p soos-invariants --all-features remote_companion_contract`.

| Suite | Result |
|---|---|
| `config_tests` | 5 passed (G), 13 failed |
| `http_tests` | 1 passed (G), 15 failed |
| `identity_tests` | 1 passed (G), 8 failed |
| `routes_tests` | 1 passed (G), 6 failed |
| `session_tests` | 3 passed (G), 7 failed |
| `socket_tests` | 1 passed (G), 8 failed |
| `server_tests` | 0 passed, 28 failed |
| invariants `remote_companion_contract` | 7 passed (G), 11 failed (10 red + the pre-existing registration test outside the module); whole crate 455 passed, 11 failed |

Representative failure messages (stubs):

- `test_rmc_login_parse_accepts_and_lowercases_printable_ascii`: `"Owner@Example.COM" must be a valid login`
- `test_rmc_parse_config_minimal_defaults`: `called Result::unwrap() on an Err value: Syntax`
- `test_rmc_parse_config_rejects_missing_or_empty_logins`: `assertion left == right failed` (`Err(Syntax)` vs `Err(NoAllowedLogins)`)
- `test_rmc_parse_config_socket_path_rules`: `assertion left == right failed: "relative/remote.sock"`
- `test_rmc_check_not_root_table`: `assertion left == right failed` (`Ok(())` vs `Err(RunningAsRoot)`)
- `test_rmc_load_config_reads_a_bounded_file`: `called Result::unwrap() on an Err value: NotFound`
- `test_rmc_authorize_*`: the allowlist cannot be built (`called Option::unwrap() on a None value` from `TailscaleLogin::parse`)
- `test_rmc_check_host_table_without_allowlist`: `assertion left == right failed` (`Err(Missing)` vs `Ok("pc.tail1234.ts.net")`)
- `test_rmc_parse_request_head_nominal`: `called Result::unwrap() on an Err value: Incomplete`
- `test_rmc_parse_request_head_rejects_http10_h2_preface_and_garbage`: `assertion left == right failed: "GET / HTTP/1.0\r\nHost: a.ts.net\r\n\r\n"`
- `test_rmc_parse_request_head_too_large`: `assertion failed: parse_request_head(&exact).is_ok()`
- `test_rmc_encode_response_has_mandatory_headers_and_content_length` / `_status_lines` / `test_rmc_encode_sse_head_and_event`: `response head terminator` (empty encoding)
- `prop_rmc_parse_request_head_never_panics`: `Test failed: assertion failed: (left == right)` (`Incomplete` instead of `HeadTooLarge` over the bound)
- `test_rmc_route_table`: `assertion left == right failed` (`NotFound` vs `Asset(Index)`)
- `test_rmc_route_wrong_method_and_unknown_paths`: `assertion left == right failed: /`
- `test_rmc_assets_table_content_types_and_bodies`: `assertion left == right failed: Index` (`application/octet-stream`)
- `test_rmc_check_lock_csrf_requires_the_action_header`: `assertion left == right failed` (`Err(MissingActionHeader)` vs `Ok(())`)
- `test_rmc_session_props_from_get_all_reply_nominal`: `called Result::unwrap() on an Err value: Malformed`
- `test_rmc_select_session_prefers_active_local_user_seat_session`: `assertion left == right failed: smallest active id`
- `test_rmc_select_session_excludes_remote_greeter_seatless_and_other_uids`: `assertion left == right failed: other uid`
- `test_rmc_status_from_maps_results_fail_safe`: `assertion left == right failed` (`Unavailable`/`checked 0` vs `Locked`/`42`)
- `test_rmc_view_changed_ignores_checked_unix_ms_only`: `assertion failed: view_changed(&base, &view(SessionState::Locked, true, false, None, 1))`
- `test_rmc_prepare_socket_dir_creates_0700_when_absent`: `called Result::unwrap() on an Err value: Os { code: 2, kind: NotFound }` (nothing created)
- `test_rmc_prepare_socket_dir_refuses_symlink_and_regular_file` / `_two_missing_levels` / `_a_foreign_owner`: `called Result::unwrap_err() on an Ok value: ()`
- `test_rmc_prepare_socket_dir_tightens_an_existing_directory`: `assertion left == right failed` (`0o755` vs `0o700`)
- `test_rmc_bind_listener_binds_0600_and_replaces_a_stale_socket`: `called Result::unwrap() on an Err value: Io(Unsupported)`
- `test_rmc_bind_listener_in_a_missing_directory_is_io`: `Io(Unsupported)` (expected `Io(NotFound)`)
- `server_tests::*` (28): `called Option::unwrap() on a None value` at `Harness::start_with` (`TailscaleLogin::parse`)

Server suite with the temporary lowercasing `parse` (scratch run; stub restored):

- 13 request/response tests: `server closed the connection without a response`
- 12 stream tests: `stream closed without a head`
- `test_rmc_connection_limit_drops_excess_without_response`: `idle connection closed early: Ok(0)`
- `test_rmc_request_head_timeout_closes_without_response`: `the connection is kept until the head timeout`
- `test_rmc_deadlines_bound_hung_logind_calls`: `a read must start within the bound: Elapsed(())` (after the 20 s wall-clock fallback)
- `test_rmc_poller_ending_makes_serve_return_an_error`: `serve returns within the bound: Elapsed(())` (after the 20 s wall-clock fallback)

Invariants:

- `test_rmc_forbid_unsafe_list_and_review_tooling_include_remote`: `scripts/candid_review.sh BUSINESS_CRATES must include remote (spec §1.1): BUSINESS_CRATES=(protocol policy vision …)`
- `test_rmc_s3_no_unlock_or_locked_hint_literal`: `logind.rs names Manager.LockSession through a string literal (D9)`
- `test_rmc_s5_s11_…`: `packaging/soos-remote.service must exist (spec §2.9)`
- `test_rmc_s6_…`: `scripts/install_remote.sh must exist (spec §2.10)`
- `test_rmc_s7_…`: `Docs/REMOTE_COMPANION.md must exist (RMC-S7)`
- `test_rmc_architecture_and_strategy_documents_describe_the_companion`: `AI/ARCHITECTURE.md documents the Remote Companion (spec §1.1)`
- `test_rmc_s8_assets_exist_…`: `crates/remote/assets/index.html must exist (D11)`; `test_rmc_s8_app_js_…`: `crates/remote/assets/app.js must exist (D11)`
- `test_rmc_s9_zbus_manifest_line_and_pinned_system_bus`: `logind.rs connects to SYSTEM_BUS_ADDRESS`
- `test_rmc_s10_main_refuses_root_before_loading_configuration`: `main.rs must call check_not_root (RMC-S10, D1)`
- migrated `test_pau_zbus_is_used_only_by_the_daemon`: **passes** (both manifests declare the exact line; every other manifest is zbus-free)

### Quality gates at hand-off

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean (stubs and all test suites compile under the workspace lints).
- `cargo test --locked --workspace --all-targets --all-features`: red only in `soos-remote` (85 of 97) and `soos-invariants` (11, of which 10 are this contract and 1 the pre-existing workspace-tree test); every other crate unchanged and green.
- `cargo deny --locked check`: `advisories ok, bans ok, licenses ok, sources ok` (no new external crate).
- `./scripts/candid_review.sh`: `Candid Review PASSED` (layer 1).

### Notes for Phase 3 (auditor) and Phase 4 (developer)

- The frozen-clock harness means every server-side bound must be a `tokio::time` primitive; a `std::thread::sleep`, a `SystemTime` deadline or a blocking D-Bus call would hang the tests until the 20 s fallback and then fail.
- `test_rmc_connection_limit_drops_excess_without_response` connects 16 idle clients before the 17th: the semaphore permit must be taken at accept time (before reading the head) and the head timer must start at accept.
- The keep-alive and the max-stream tests move time in 15 s jumps once per iteration: the stream must re-send "the newest reading it holds" even when the poller's publish and the keep-alive timer fire in the same jump (ordering between the two tasks is not guaranteed; strictly increasing `checked_unix_ms` across keep-alives still holds because at least one poller read happens per jump).
- `test_rmc_seq_is_reserved_when_a_read_starts` holds a poller read across a new stream's first read and release: the reservation must happen before `own_sessions` is awaited (R3-1), and a stream must drop any reading whose `seq` is not above the last one it sent.
- `proptest` writes `crates/remote/tests/*.proptest-regressions` files on failing runs; they are scratch artifacts of the red state and were deleted — do not commit them.

---

## Revision 4 — D5a′ Effective Host From `X-Forwarded-Host` (spec §13, 2026-10-06)

Scope of this cycle: spec §13.3 only (D5a′: effective host decided by `X-Forwarded-Host`,
mandatory `X-Forwarded-Proto: https` on a proxied request; installer git mode `100755`
static contract; documentation needles). No existing test was modified; the three existing
`check_host` tests and every `server_tests` fixture send no `X-Forwarded-*` header and stay
valid under D5a′. Every fixture uses the synthetic names of the suites (`pc.tail1234.ts.net`,
`owner@example.com`, `100.64.0.1`); no real login, tailnet name, 100.x address or profile
URL appears anywhere (R4-8).

Stub added for compilation (the exactly specified §3 API, nothing more):
`crates/remote/src/lib.rs` gains `FORWARDED_HOST_HEADER = "x-forwarded-host"`,
`FORWARDED_PROTO_HEADER = "x-forwarded-proto"` and `FORWARDED_PROTO_HTTPS = "https"`.
`check_host` itself is untouched, so every behavioural test fails on its assertion.

### Tests added

| Test (path::name) | Acceptance line / matrix ID | Red evidence (failure message) |
|---|---|---|
| `crates/remote/tests/identity_tests.rs::test_rmc_forwarded_header_constants` | spec §3 / §13.3 `lib.rs` row | G (green by construction: pins the constant values) |
| `identity_tests.rs::test_rmc_check_host_effective_host_comes_from_x_forwarded_host` | §13.2 "Effective host", "Evaluation order"; RMC4 (R4-2) — XFH + `Host: localhost` → OK; XFH + `Host` absent → OK; XFH + `Host` repeated → OK; XFH + a `Host` that is refused alone → OK; neither header → `Missing`; `X-Forwarded-For` alone → `Missing`; XFH twice → `Repeated`; XFH twice + proto `http` → `Repeated`; `Host` twice without XFH → `Repeated` | `assertion left == right failed: Host: localhost is not inspected when X-Forwarded-Host is present` — `left: Err(NotAllowed)`, `right: Ok("pc.tail1234.ts.net")` |
| `identity_tests.rs::test_rmc_check_host_normalises_the_forwarded_host` | §13.2 "Normalisation"; RMC4 (R4-3) — `" PC.Tail1234.TS.NET:443 "` → OK; `a.ts.net`, 253-byte name → OK; `:8443`, `:443:443`, `100.64.0.1`, IPv6 literal, `evil.com`, `localhost`, `a.ts.net, b.ts.net`, empty, OWS-only, trailing dot, `ts.net`, `pc.ts.net.evil.com`, `p_c.ts.net`, `https://…`, non-UTF-8, 254 bytes → `NotAllowed`, each also with a valid `Host` beside the refused XFH | `assertion left == right failed` — `left: Err(NotAllowed)`, `right: Ok("pc.tail1234.ts.net")` |
| `identity_tests.rs::test_rmc_check_host_requires_https_forwarded_proto` | §13.2 "Transport"; RMC4 (R4-1) — XFH + proto absent / `http` / `https` twice (identical) / `https, https` / `https:` / `wss` / empty / non-UTF-8 / `httpsx` → `NotAllowed`; XFH + `HTTPS`, `" https "`, `"\thttps\t"` → OK; `Host` only + proto absent / `https` / `HTTPS` → OK; `Host` only + `http` / `https` twice / empty → `NotAllowed` | `assertion left == right failed: scheme compared ASCII-case-insensitively` — `left: Err(NotAllowed)`, `right: Ok("pc.tail1234.ts.net")` |
| `identity_tests.rs::test_rmc_check_host_allowlist_applies_to_the_forwarded_host` | §13.2 + `allowed_hosts`; RMC4, RC-2 — XFH `mypc…` + `Host: other…` → `Ok("mypc.tail1234.ts.net")`; XFH `MYPC…:443` + `Host: localhost` → OK; XFH `other…` + `Host: mypc…` → `NotAllowed`; listed XFH + proto `http` → `NotAllowed` | `assertion left == right failed` — `left: Err(NotAllowed)`, `right: Ok("mypc.tail1234.ts.net")` |
| `crates/remote/tests/server_tests.rs::test_rmc_serve_head_status_and_lock_end_to_end` | §13.1 capture reproduced by `serve_head()` (`Host: localhost`, `Tailscale-User-Login`, `X-Forwarded-For: 100.64.0.1`, `X-Forwarded-Host: pc.tail1234.ts.net`, `X-Forwarded-Proto: https`); §13.2 "Origin"; RMC20, RMC4 (R4-3) — `GET /api/status` → `200` (shape, mandatory headers, one logind read); `POST /api/lock` + `Origin: https://localhost` → `403 forbidden`, no `LockSession`, no read; + `Origin: https://pc.tail1234.ts.net` → `202 lock_requested`, one `LockSession(c0ffee42)`; + `Origin: https://PC.Tail1234.TS.NET:443` → `202` | `assertion left == right failed: "{\"result\":\"misdirected_request\"}"` — `left: 421`, `right: 200` (server_tests.rs:2215) |
| `server_tests.rs::test_rmc_serve_head_with_http_proto_is_misdirected` | §13.2 "Transport" end to end; RMC4 (R4-1) — captured head with `X-Forwarded-Proto: http` → `421 misdirected_request` on `/api/status` and `/api/lock`, mandatory headers, no logind read, no `LockSession`; the same head with `https` → `200` (pins the `421` to the scheme, not to `Host: localhost`) | `assertion left == right failed: "{\"result\":\"misdirected_request\"}"` — `left: 421`, `right: 200` (server_tests.rs:2313; the `421` assertions pass on the current code for the wrong reason, the final `200` fails) |
| `server_tests.rs::test_rmc_serve_head_without_proto_or_with_foreign_host_is_misdirected` | §13.2 end to end; RMC4 (R4-1, R4-2, R4-3) — captured head without `X-Forwarded-Proto`, with `X-Forwarded-Proto` twice, with `X-Forwarded-Host: evil.com`, `…:8443`, repeated XFH, XFH `100.64.0.1` → `421 misdirected_request` with mandatory headers and no logind read; the unmodified head → `200` | `assertion left == right failed` — `left: 421`, `right: 200` (server_tests.rs:2363, the accepted-head assertion) |
| `server_tests.rs::test_rmc_allowed_hosts_apply_to_the_forwarded_host` | §13.3 e2e row (`allowed_hosts = ["mypc.tail1234.ts.net"]`); RMC4, RC-2 — captured head (XFH `pc…`) → `421`; XFH `MYPC.tail1234.ts.net:443` + `Host: localhost` → `200`, one read | `assertion left == right failed: "{\"result\":\"misdirected_request\"}"` — `left: 421`, `right: 200` (server_tests.rs:2384) |
| `tests/invariants/src/remote_companion_contract.rs::test_rmc_s12_installer_scripts_are_executable` | §8 RMC-S12 (R4-5); RMC16 — `scripts/install_remote.sh` and `scripts/install.sh`: `metadata.permissions().mode() & 0o111 != 0`, `std::fs` only, no `git` subprocess | G on this working tree (the `100755` mode change is already staged). Teeth proven in a scratch run with `chmod 644 scripts/install_remote.sh` (mode restored to `755` afterwards, `git status` unchanged): `scripts/install_remote.sh must be executable (git mode 100755, RMC-S12); found 644` |
| `remote_companion_contract.rs::test_rmc_s7b_documentation_describes_the_effective_host` | §8 RMC-S7b (R4-4); RMC19 — `Docs/REMOTE_COMPANION.md` contains `X-Forwarded-Host`, `X-Forwarded-Proto`, `effective host`; no longer contains `pending owner verification` or `(pending)` | `Docs/REMOTE_COMPANION.md must mention \`X-Forwarded-Host\` (RMC-S7b)` |

### Migrated existing tests

none — `test_rmc_check_host_requires_exactly_one_host_header`,
`test_rmc_check_host_table_without_allowlist`, `test_rmc_check_host_table_with_allowlist`,
`test_rmc_host_is_checked_before_identity_and_routing`,
`test_rmc_allowed_hosts_config_is_enforced_end_to_end`,
`test_rmc_s7_operator_documentation_exists_and_covers_the_requirements` and
`test_rmc_identity_errors_never_echo_values` are untouched and pass (they send no
`X-Forwarded-*` header; `HostError` variants, `Display` texts and the `421` mapping are
unchanged by D5a′).

### Red tally (2026-10-06)

| Suite | Result |
|---|---|
| `identity_tests` | 10 passed (all pre-existing + `test_rmc_forwarded_header_constants`), 4 failed (new) |
| `server_tests` | 28 passed (all pre-existing), 4 failed (new) |
| invariants `remote_companion_contract` | 467 passed in the crate (RMC-S12 among them), 1 failed (`RMC-S7b`) |
| rest of the workspace (`--no-fail-fast`) | no failure |

### Flakiness check

`cargo test --locked -p soos-remote --all-features --test server_tests -q serve_head` run
10 times in a row: `0 passed; 3 failed` every time, identical assertions and line numbers
(frozen-clock harness, real I/O, no wall-clock dependency);
`test_rmc_allowed_hosts_apply_to_the_forwarded_host` fails deterministically at the same
assertion. No `*.proptest-regressions` file was produced (no new property test).

### Quality gates at hand-off

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`: red only
  in the 9 new tests listed above; every pre-existing test green.
- `cargo deny --locked check`: `advisories ok, bans ok, licenses ok, sources ok`.
- `./scripts/candid_review.sh`: `Candid Review PASSED`.

### Notes for Phase 3 (auditor) and Phase 4 (developer)

- `check_host` keeps its signature; the e2e `202`/`403` rows rely on `server.rs` passing
  the value `check_host` returns to `check_lock_csrf` (already the case): the fix is
  confined to `identity.rs` (plus the doc comments listed in spec §13.3).
- The "Transport" rows pin the counting semantics: `X-Forwarded-Proto` twice with identical
  `https` values is `NotAllowed`, so a first-header-wins or last-header-wins lookup fails;
  reuse `single_header` with `NotAllowed` for both the missing and the repeated case when
  XFH is present, and treat `Missing` as acceptable only when XFH is absent.
- Evaluation order is pinned by "XFH twice + proto `http` → `Repeated`" and "`Host` twice
  without XFH → `Repeated`": count XFH first, then decide the branch.
- `test_rmc_serve_head_status_and_lock_end_to_end` asserts `reads() == 1` after the `403`
  (CSRF before any logind read) and `== 2` after the first `202` (exactly one fresh
  snapshot per accepted lock) — the current dispatch order already satisfies both.
- RMC-S7b and the §13.3 `Docs/REMOTE_COMPANION.md` rows (including the Shortcuts hedge,
  R4-6/R4-7) are the developer's and traceability agent's documentation work; the needles
  are the only thing the test checks, the content rules are in spec §13.3.
