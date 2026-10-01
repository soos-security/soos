# soos-daemon Reference

> Crate: `crates/daemon` (`soos-daemon`)
> Scope: configuration file, connection model, authorization per request kind, startup and swap protection.
> Source of truth: `crates/daemon/src/config.rs` (keys and defaults), `crates/daemon/src/dispatcher.rs`
> (request handling), `crates/daemon/src/limits.rs`, `crates/daemon/src/preview.rs`. When this
> document and the code disagree, the code is right; the repository invariant
> `daemon_docs_contract::test_daemon_doc_documents_every_daemon_toml_key` fails the build when a
> parsed key is missing here (GitHub #206, review finding DMN-17).

---

## 1. Configuration File

`soos-daemon` reads its configuration from, in order:

1. `--config <FILE>` (`-c`): the file must exist and parse, otherwise the daemon refuses to start.
2. `/etc/soos/daemon.toml` (`DEFAULT_CONFIG_PATH`) when it is a regular file.
3. Otherwise the built-in runtime defaults (`DaemonConfig::runtime_default()`).

No `daemon.toml` is packaged: a fresh install runs on the runtime defaults below. Every load path
starts from `DaemonConfig::runtime_default()`, so an absent file, an empty file and a file with an
empty `[pipeline]` table produce the same configuration (GitHub #205, see §1.4).

Parsing uses `toml` with `serde`: a malformed file or a value of the wrong type is a startup error
(`DaemonError::Config`). **Unknown keys are ignored silently**, so check spelling against this page.
Validation errors listed below are also startup errors; the daemon is fail-closed and never starts
with a half-applied configuration.

`--mock-camera` forces `[pipeline] use_mock_camera = true` (development only).

### 1.1 Root keys

| Key | Type | Default | Notes |
|---|---|---|---|
| `log_level` | string | `"info"` | `tracing_subscriber::EnvFilter` directive; the `RUST_LOG` environment variable wins when set. |

### 1.2 `[socket]`

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `socket_path` | path | `/run/soos/daemon.sock` | Keep it inside `socket_dir`: the stale-node checks resolve its file name relative to `socket_dir`; a stale symlink or non-socket node is refused. |
| `socket_dir` | path | `/run/soos` | Validated at bind time: not a symlink, not world-writable, root-owned when `enforce_root_owner`. |
| `socket_mode` | integer | `0o660` | Permission bits applied with `fchmodat`. Write it as a TOML octal literal (`0o660`). It must never grant world access (AGENTS.md prohibits `0666`); keep `0o660`. |
| `enforce_root_owner` | bool | `true` | Require `socket_dir` to be owned by UID 0. Only test harnesses disable it. |
| `socket_group` | string | `"soos"` | Group given to the socket node (`fchownat`); members of this group may connect. |

### 1.3 `[dispatcher]`

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `max_concurrent_connections` | integer | `8` | Global connection permits; must be at least 1 and strictly above `[peer_limits] reserved_root_connections`. |
| `connection_timeout_ms` | integer (ms) | `2500` (`DEFAULT_CONNECTION_TIMEOUT_MS`) | Budget of one request (read, verification, encoding) and idle timeout of a persistent connection (§2). Must lie in 100..=10000. The default matches the GDM line `timeout_ms=2500` (user decision 2026-09-30); console/sudo requests stay capped by their 1000 ms PAM client deadline. |
| `enforce_active_session` | bool | `true` | Local-session policy for `Auth` and the session check of `PreviewFrame` (§3). `false` is for test harnesses only. |
| `logind_sessions_dir` | path | `/run/systemd/sessions` | logind runtime session records read by the session policy. |

### 1.4 `[pipeline]`

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `camera_device` | path | auto (`/dev/v4l/by-id/default-camera` sentinel) | `""`, `auto` and `default` keep auto-detection; any other value is used verbatim (ADR 2026-09-30 "Single Camera Resolver"). |
| `sensor_preference` | string | `prefer_ir` | `prefer_ir`/`ir`, `prefer_rgb`/`rgb`, `any` (case-insensitive); an unknown value keeps the default. |
| `idle_timeout_secs` | integer (s) | `10` | Inactivity delay before the capture thread drops to its idle rate / standby. |
| `warmup_frames` | integer | `0` (`DAEMON_DEFAULT_WARMUP_FRAMES`) | Frames discarded after each camera (re)start. The daemon default is the same with and without a config file (GitHub #205); the camera crate's library default of 20 does not apply to the daemon. |
| `use_mock_camera` | bool | `false` | Simulated camera (development only). |
| `models_dir` | path | `/var/lib/soos/models` | Directory holding `manifest.toml` and the attested ONNX models; checksums are verified before the socket opens (fail-closed). |
| `inference_intra_threads` | integer | ONNX Runtime default (`soos_inference_ort::default_intra_threads()`) | ONNX Runtime intra-op threads per model session (GitHub #252); spin-waiting stays disabled. |
| `biometrics_dir` | path | `/var/lib/soos/biometrics` | Encrypted templates (`0700 root:root`). |
| `master_key_path` | path | `/var/lib/soos/master.key` | Template master key (`0600 root:root`). |

#### `[pipeline.evidence]` (opt-in intrusion snapshots)

| Key | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `false` | Strictly opt-in. When set, the daemon seals a snapshot for a `PasswordFailed` event (reason `PasswordFailed`) and for every `Auth` request vetoed as a presentation attack (reason `PadFailed`, the capture that triggered the veto, one per request, written on the blocking pool after the `Deny` / `PadFailed` response is rendered; GitHub #261). |
| `base_dir` | path | `/var/lib/soos/evidence` | `0700 root:root`. |
| `key_path` | path | `/var/lib/soos/evidence.key` | Evidence encryption key. |
| `retention_days` | integer | `7` | Snapshots older than this are purged. |
| `daily_cap_per_uid` | integer | `10` | Maximum snapshots per UID and day. |
| `daily_cap_total` | integer | `100` (`DEFAULT_DAILY_CAP_TOTAL`) | Maximum snapshots per day across all UIDs (GitHub #276); refused before any write without consuming the per-UID quota. The day's total is derived from the snapshot files already stored, so it survives a restart. |

#### `[pipeline.thresholds]`

| Key | Type | Default | Validation |
|---|---|---|---|
| `match_threshold` | float | `0.70` | Must lie in `[0, 1]` and be at least `ThresholdConfig::MIN_MATCH_THRESHOLD` (0.40). |
| `pad_threshold` | float | `0.85` | Must lie in `[0, 1]` and be at least `ThresholdConfig::MIN_PAD_THRESHOLD` (0.50); PAD can never be disabled from the file (GitHub #170). |

#### `[pipeline.rate_limit]` (authentication attempts per target UID)

| Key | Type | Default | Notes |
|---|---|---|---|
| `max_attempts` | integer | `5` | Attempts accepted per window. |
| `window_duration_secs` | integer (s) | `60` | Sliding window. |
| `max_tracked_uids` | integer | `1024` | Bounded limiter table. |

### 1.5 `[preview]` (GUI camera preview, GitHub #143)

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `enabled` | bool | `false` | When `false`, only a root peer may request preview frames. |
| `allowed_uids` | list of integers | `[]` | Unprivileged UIDs allowed when `enabled`; at most `MAX_PREVIEW_ALLOWED_UIDS` (64). |
| `max_requests_per_sec` | integer | `40` | Per-peer-UID rate limit, root included. |

### 1.6 `[peer_limits]` (GitHub #157, #175)

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `max_connections_per_uid` | integer | `2` | Concurrent connections of one unprivileged UID; at least 1. Root is exempt. |
| `reserved_root_connections` | integer | `2` | Permits usable only by root peers; strictly below `max_concurrent_connections`. |
| `max_requests_per_connection` | integer | `1024` | At least 1. |
| `max_connection_lifetime_ms` | integer (ms) | `30000` | At least 1. |
| `max_events_per_window` | integer | `5` | `PasswordFailed` events per peer UID; `0` drops all events. |
| `event_window_ms` | integer (ms) | `10000` | At least 1. |

### 1.7 Example

```toml
log_level = "info"

[dispatcher]
connection_timeout_ms = 2500

[pipeline]
camera_device = "auto"
sensor_preference = "prefer_ir"
warmup_frames = 0

[pipeline.thresholds]
match_threshold = 0.70
pad_threshold = 0.85
```

---

## 2. Connection Model

- One Tokio task per accepted connection, tracked in `soos_daemon::shutdown::ConnectionTasks`
  (never detached; finished handlers are reaped while the accept loop runs, see §5). `SO_PEERCRED` is read once, before any byte, and the
  connection is admitted by `PeerConnectionLimiter` (global permits, root reservation, per-UID cap);
  a refused peer is closed without being read (the PAM client sees EOF and returns `PAM_IGNORE`).
- **Persistent loop**: an admitted connection serves framed messages until the client closes it, it
  stays idle past `connection_timeout_ms`, `max_requests_per_connection` requests were served, or
  `max_connection_lifetime_ms` elapsed. A connection that times out before its first request is
  closed as an error; later idle timeouts close it gracefully.
- Each request runs in two phases: read + process + encode under `connection_timeout_ms` (no byte is
  written in this phase, so a cancelled request never leaves a partial frame), then a separate write
  with its own timeout.
- `Auth` is one-shot: the daemon closes the connection after its `Response`. `Status`,
  `PreviewFrame` and `Event` messages may share one connection.
- Inbound frames are bounded by `MAX_MESSAGE_SIZE` (4096 bytes); only outbound `PreviewResponse`
  frames may reach `MAX_PREVIEW_MESSAGE_SIZE` (2 MiB). The wire format is specified in
  `Docs/IPC_PROTOCOL.md`.

## 3. Authorization per Request Kind

| Message | Who may send it | Checks, in order | Refusal |
|---|---|---|---|
| `RequestKind::Status` | any peer admitted to the socket | wire validation only; returns `StatusResponse` (readiness booleans, PID, uptime; no biometric data) | `ProtocolError` on malformed request |
| `RequestKind::Auth` | root peer for any UID, or an unprivileged peer for its own UID | wire validation, `SO_PEERCRED` UID versus `uid_hint`, local-session policy (ADR 2026-09-30 "Local Session Binding"), deadline, rate limit, camera + PAD consensus + match | `ProtocolError`/`UidMismatch`, `Unavailable`, or `Deny`; never `Allow` on an error path |
| `RequestKind::PreviewFrame` | root peer, or an allow-listed UID with `[preview] enabled = true` and an active local session | wire validation, `SO_PEERCRED` UID versus `uid_hint`, `authorize_preview`, session check, per-UID rate limit | standard `Response` with `ProtocolError`, zero pixel bytes |
| `Event` (`PasswordFailed`) | root peer for any UID, any other peer for itself | target UID versus peer UID, per-peer event quota | dropped with a `warn`; events never get a response |

## 4. Startup and Swap Protection

1. Configuration is loaded (§1) and logging initialized.
2. Swap protection: `soos_daemon::mlock::enable_swap_protection` calls
   `mlockall(MCL_CURRENT | MCL_FUTURE)`. Success is logged at `info`; a refusal is logged at `warn`
   and recorded as `HealthState::memory_locked() == false` (GitHub #201). `mlockall` is the only
   page-locking layer; see `Docs/MEMORY_PROTECTION_AND_SWAP.md`. The flag is informational and does
   not change `is_healthy`.
3. The pipeline (models, camera supervisor, stores) is initialized fail-closed; any error stops the
   daemon before the socket exists.
   The PAD model `minifasnet_v2_pad` is always loaded and self-tested. The optional second PAD
   ensemble member `minifasnet_v1se_pad` (`SECONDARY_PAD_MODEL_ID`, crop scale 4.0, GitHub #212)
   is wired by `attach_optional_pad_members` only when the deployed `manifest.toml` attests it;
   the repository manifest does not, so the daemon is single-model by default. When the entry is
   present, a load, shape or self-test failure of the member stops the daemon (fail closed).
4. The socket is bound (§1.2) and the accept loop starts.

## 5. Shutdown, Panic Reporting and Log Anonymization (GitHub #257, #258, #259)

- **Graceful shutdown**: on SIGINT or SIGTERM `accept_until_shutdown` returns, the listener is
  dropped (no new connection is accepted), `socket_ready` is cleared and the socket file is
  unlinked (`SocketGuard`). The remaining handlers are then drained by
  `ConnectionTasks::drain(connection_timeout)`; handlers still running when that budget expires are
  aborted (the PAM client sees EOF and returns `PAM_IGNORE`). The drain logs its start
  (`in_flight`, `budget_ms`) and its outcome (`completed`, `panicked`, `aborted`).
- **Panics**: a panicking connection handler is reported at `error` level ("Connection handler
  panicked") when its task is joined; the `JoinError` is never formatted because its `Display`
  carries the panic payload. `install_panic_hook` replaces the default stderr hook with a `tracing`
  `error` event that records only the source file, line and thread name, never the panic message.
- **Request nonce in logs**: the 256-bit `request_id` is never logged. A log line that needs to
  correlate a request uses `request_id = %short_request_id(&req.request_id)`: the first 4 bytes of
  `SHA-256("soos.request-id.log.v1" || request_id)` as 8 hex digits, which reveals no nonce bit
  (enforced by `crates/daemon/tests/request_id_logging_tests.rs`).
- **Response timestamps**: `build_response` stamps every `Response` from the dispatcher clock:
  `issued_monotonic_ns > 0` and `expires_monotonic_ns = issued + RESPONSE_VALIDITY_NS` (2 s) on every
  verdict path. If the clock fails, the response carries `issued = expires = 0` (already expired)
  and an `Allow` verdict is downgraded to `Unavailable` / `InternalError`. Since GitHub #287 the
  PAM client enforces both fields against its own CLOCK_MONOTONIC reading and returns
  `PAM_IGNORE` for an unstamped, inverted, future-dated or expired response (see
  `Docs/IPC_PROTOCOL.md`, "Response Freshness").
4. The socket is bound (§1.2). When started by systemd (`Type=notify`, `NOTIFY_SOCKET` set) the daemon
   then sends `READY=1` through `soos_daemon::sd_notify`; only then does systemd start units ordered
   after it (`display-manager.service`). A notification failure is logged at `warn`. The accept loop
   starts, and `STOPPING=1` is sent on SIGTERM / SIGINT (GitHub #203).
