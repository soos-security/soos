# soos-daemon Reference

> Crate: `crates/daemon` (`soos-daemon`)
> Scope: configuration file, connection model, authorization per request kind, startup and swap protection, presence auto-unlock.
> Source of truth: `crates/daemon/src/config.rs` (keys and defaults), `crates/daemon/src/dispatcher.rs`
> (request handling), `crates/daemon/src/limits.rs`, `crates/daemon/src/preview.rs`. When this
> document and the code disagree, the code is right; the repository invariant
> `daemon_docs_contract::test_daemon_doc_documents_every_daemon_toml_key` fails the build when a
> parsed key is missing here (GitHub #206, review finding DMN-17).

---

## 1. Configuration File

`soos-daemon` reads its configuration from, in order:

1. `--config <FILE>` (`-c`): the file must exist and parse, otherwise the daemon refuses to start.
2. `/etc/soos/daemon.toml` (`DEFAULT_CONFIG_PATH`).
3. The built-in runtime defaults (`DaemonConfig::runtime_default()`), only when
   `/etc/soos/daemon.toml` does not exist.

Both files are read with the bounded reader shared with the daemon clients,
`soos_camera_v4l::daemon_config::read_daemon_config_text` (GitHub #315): the path is pinned with
`O_PATH` (symbolic links followed), must be a regular file of at most `MAX_DAEMON_CONFIG_BYTES`
(1 MiB), and the pinned inode is read through `/proc/self/fd/<n>`. There is no check-then-open:
a directory, FIFO, device node, unreadable, oversized or non-UTF-8 file at either path is a
startup error (a FIFO is refused at once, never read), not a fall back to the defaults.

No `daemon.toml` is packaged: a fresh install runs on the runtime defaults below. Every load path
starts from `DaemonConfig::runtime_default()`, so an absent file, an empty file and a file with an
empty `[pipeline]` table produce the same configuration (GitHub #205, see §1.4).

Parsing uses `toml` with `serde`: a malformed file or a value of the wrong type is a startup error
(`DaemonError::Config`). **Unknown keys are ignored silently**, so check spelling against this page.
Validation errors listed below are also startup errors; the daemon is fail-closed and never starts
with a half-applied configuration.

`--mock-camera` forces `[pipeline] use_mock_camera = true` (development only). It is applied
after the file is validated, so it does not make a file with `enforce_active_session = false`
acceptable (§1.3).

Non-fatal problems (an unknown `sensor_preference`, and `allow_virtual_camera = true`, which
weakens the camera trust boundary; GitHub #318) are collected in
`DaemonConfig::warnings` while the file is parsed and logged at `warn` level by `main.rs` once
logging is initialized; a warning names the key, never its value.

### 1.1 Root keys

| Key | Type | Default | Notes |
|---|---|---|---|
| `log_level` | string | `"info"` | `tracing_subscriber::EnvFilter` directive; the `RUST_LOG` environment variable wins when set. |

### 1.2 `[socket]`

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `socket_path` | path | `/run/soos/daemon.sock` | Must be a file name directly inside `socket_dir` (`SocketConfig::validate`, a startup error otherwise; GitHub #315): the stale-node checks resolve its file name relative to `socket_dir`; a stale symlink or non-socket node is refused. |
| `socket_dir` | path | `/run/soos` | Validated at bind time: not a symlink, not world-writable; when `enforce_root_owner`, owned by root and, when `socket_group` is set, by that group (`socket::validate_directory_group`, GitHub #315). Under systemd it is created by `RuntimeDirectory=soos` with the unit's `Group=soos`. |
| `socket_mode` | integer | `0o660` | Permission bits applied with `fchmodat`. Write it as a TOML octal literal (`0o660`). It must never grant world access (AGENTS.md prohibits `0666`); keep `0o660`. |
| `enforce_root_owner` | bool | `true` | Require `socket_dir` to be owned by UID 0. Only test harnesses disable it. |
| `socket_group` | string | `"soos"` | Group given to the socket node (`fchownat`); members of this group may connect. |

### 1.3 `[dispatcher]`

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `max_concurrent_connections` | integer | `8` | Global connection permits; must be at least 1 and strictly above `[peer_limits] reserved_root_connections`. |
| `connection_timeout_ms` | integer (ms) | `2500` (`DEFAULT_CONNECTION_TIMEOUT_MS`) | Budget of one request (read, verification, encoding) and idle timeout of a persistent connection (§2). Must lie in 100..=10000. The default matches the GDM line `timeout_ms=2500` (user decision 2026-09-30); console/sudo requests stay capped by their 1000 ms PAM client deadline. |
| `enforce_active_session` | bool | `true` | Local-session policy for `Auth` and the local seat session check of `PreviewFrame` (§3). `false` is accepted only together with `[pipeline] use_mock_camera = true` (test harnesses); otherwise it is a startup error (GitHub #315, orchestrator decision). |
| `logind_sessions_dir` | path | `/run/systemd/sessions` | logind runtime session records read by the session policy. |

### 1.4 `[pipeline]`

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `camera_device` | path | auto (`/dev/v4l/by-id/default-camera` sentinel) | `""`, `auto` and `default` keep auto-detection; any other value is used verbatim (ADR 2026-09-30 "Single Camera Resolver"). A value holding a NUL byte is a startup error naming the key, never the value (GitHub #318; no path can hold one and `v4l` would panic on it). |
| `allow_virtual_camera` | bool | `false` | Opt-in to open a virtual or output-capable V4L2 node (v4l2loopback, vivid, output or memory-to-memory capability), copied to `CameraConfig::allow_virtual_device` (GitHub #318, ADR 2026-10-02 "Virtual V4L2 Nodes Are Never Biometric Cameras"). Test rigs only: when `true` the daemon logs a configuration warning at startup, because any local writer of such a node can inject frames. A value that is not a boolean is a startup error. |
| `sensor_preference` | string | `prefer_ir` | `prefer_ir`/`ir`, `prefer_rgb`/`rgb`, `any` (case-insensitive); an unknown value keeps the default and is logged as a configuration warning (GitHub #315). |
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
| `key_path` | path | `/var/lib/soos/evidence.key` | Evidence encryption key, loaded or created only when `enabled` is set (GitHub #312): a disabled store never touches it, so a missing key means evidence was never enabled. |
| `retention_days` | integer | `7` | Snapshots older than this are purged. |
| `daily_cap_per_uid` | integer | `10` | Maximum snapshots per UID and day. |
| `daily_cap_total` | integer | `100` (`DEFAULT_DAILY_CAP_TOTAL`) | Maximum snapshots per day across all UIDs (GitHub #276); refused before any write without consuming the per-UID quota. The day's total is derived from the snapshot files already stored, so it survives a restart. |

#### `[pipeline.thresholds]`

| Key | Type | Default | Validation |
|---|---|---|---|
| `match_threshold` | float | `0.50` | Must lie in `[0, 1]` and be at least `ThresholdConfig::MIN_MATCH_THRESHOLD` (0.40). Default calibrated for SFace (GitHub #278); a host that still sets the retired `0.70` keeps it. |
| `pad_threshold` | float | `0.85` | Must lie in `[0, 1]` and be at least `ThresholdConfig::MIN_PAD_THRESHOLD` (0.50); PAD can never be disabled from the file (GitHub #170). |

#### `[pipeline.rate_limit]` (authentication attempts per target UID)

| Key | Type | Default | Notes |
|---|---|---|---|
| `max_attempts` | integer | `40` | Attempts accepted per window and per target UID, shared by every face request (`sudo`, GDM, lock screens) and by presence scans (GitHub #323; was `5`). Presence never consumes the last 5 attempts of a window (`PRESENCE_RESERVED_ATTEMPTS`), so PAM always keeps at least the former budget. A value of `5` or less with presence enabled is accepted with a startup warning (presence then never scans). |
| `window_duration_secs` | integer (s) | `60` | Sliding window. |
| `max_tracked_uids` | integer | `1024` | Bounded limiter table. |

### 1.5 `[preview]` (camera preview for `soos-gui` and `soos-remote`, GitHub #143, #345)

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `enabled` | bool | `false` | When `false`, only a root peer may request preview frames. |
| `allowed_uids` | list of integers | `[]` | Unprivileged UIDs allowed when `enabled`; at most `MAX_PREVIEW_ALLOWED_UIDS` (64). |
| `max_requests_per_sec` | integer | `40` | Per-peer-UID rate limit, root included (shared by every preview client of that UID). |
| `remote_view` | bool | `false` | Daemon-side opt-in for the live camera view of `soos-remote` (ADR 2026-10-07 "Live Camera View in `soos-remote` Through the Daemon Preview Channel"). A peer whose cgroup is the `soos-remote.service` user unit of its own UID is refused unless this is `true`. No effect unless `enabled = true` and the peer UID is in `allowed_uids`; `true` with `enabled = false` is accepted and inert. See below. |

Every unprivileged preview peer (GUI or companion) must also own an **active local seat session**:
the same record predicate as the root-peer `Auth` path (`UID` equal to the peer UID, `ACTIVE=1`
or `STATE=active`, `REMOTE=0` exactly — an absent or malformed `REMOTE` refuses —, a non-empty
`SEAT` and `CLASS=user`). A lingering user manager (`CLASS=manager`), an SSH session or a full
logout therefore gets no preview; a locked seat session still qualifies. Root peers skip this
check and the cgroup recognition below (ADR 2026-09-29 rule).

**Remote companion recognition.** On the first `PreviewFrame` of an unprivileged connection the
daemon reads the peer's `/proc/<pid>/cgroup` once (at most 16 KiB) and classifies it with
`classify_preview_peer_cgroup` (`crates/daemon/src/preview_peer.rs`): only the systemd
hierarchies are considered (`0::` and `name=systemd`); a path below
`/user.slice/user-<u>.slice/user@<u>.service/` whose first non-`.slice` component is exactly
`soos-remote.service` (`REMOTE_COMPANION_UNIT`), with `<u>` equal on both sides and equal to the
peer UID, is the remote companion; any other path (`soos-gui.service`, `session-4.scope`,
`soos-remote@x.service`, ...) is local. A missing PID, an unreadable or oversized file, a
malformed or UID-inconsistent path, or lines that disagree refuse the preview (fail closed). The
result is cached for the connection. The first non-empty frame served to a companion connection
logs one `info` line (`Remote camera view: first preview frame served to soos-remote on this
connection`) with the peer UID only, never a size, dimension, sequence or pixel. This
recognition is an administrative opt-in and an audit aid, **not a security boundary**: any
process of the owner can start a unit of that name, or run `soos-remote` outside it (it is then
classified local and bypasses `remote_view`), and such code can already read the GUI preview.
A live camera view holds one of the UID's `max_connections_per_uid` connections for its whole
duration (§1.6) and uses at most `camera_fps` (≤ 10) of the shared per-UID preview quota.

### 1.6 `[peer_limits]` (GitHub #157, #175)

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `max_connections_per_uid` | integer | `2` | Concurrent connections of one unprivileged UID; at least 1. Root is exempt. |
| `reserved_root_connections` | integer | `2` | Permits usable only by root peers; strictly below `max_concurrent_connections`. |
| `max_requests_per_connection` | integer | `1024` | At least 1. |
| `max_connection_lifetime_ms` | integer (ms) | `30000` | At least 1. |
| `max_events_per_window` | integer | `5` | `PasswordFailed` events per peer UID; `0` drops all events. |
| `event_window_ms` | integer (ms) | `10000` | At least 1. |

### 1.7 `[presence]` (presence auto-unlock of locked local sessions, GitHub #323)

| Key | Type | Default | Validation / notes |
|---|---|---|---|
| `enabled` | bool | `true` | Enabled by default (owner decision). `false` keeps the worker from starting. |
| `scan_interval_ms` | integer (ms) | `2000` | Minimum time between the starts of two scans of one session; `1000..=60000`. `0` is refused (never "immediately" nor "disabled"). |
| `lock_grace_ms` | integer (ms) | `3000` | Time a session must have been observed locked before any scan or unlock; `1000..=60000`. `0` is refused. |

Both intervals are validated even when `enabled = false`. When `ceil(window / scan_interval)`
exceeds `max_attempts - 5`, a startup warning says that presence scans will be throttled by the
rate limit. See §6 for the behaviour.

### 1.8 Example

```toml
log_level = "info"

[dispatcher]
connection_timeout_ms = 2500

[pipeline]
camera_device = "auto"
sensor_preference = "prefer_ir"
warmup_frames = 0

[pipeline.thresholds]
match_threshold = 0.50
pad_threshold = 0.85

[presence]
enabled = true
scan_interval_ms = 2000
lock_grace_ms = 3000
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
| `RequestKind::Auth` | root peer for any UID, or an unprivileged peer for its own UID | wire validation, `SO_PEERCRED` UID versus `uid_hint`, local-session policy (ADR 2026-09-30 "Local Session Binding"), deadline, template model binding (a `Foreign` template is answered `Unavailable`/`ModelUnavailable` here, without an attempt or a camera wake; GitHub #298), rate limit, missing template, camera wake + PAD consensus + match | `ProtocolError`/`UidMismatch`, `Unavailable`, or `Deny`; never `Allow` on an error path |
| `RequestKind::PreviewFrame` | root peer, or an allow-listed UID with `[preview] enabled = true` and an active local seat session (`soos-gui`, or `soos-remote` when `[preview] remote_view = true`) | wire validation, `SO_PEERCRED` UID versus `uid_hint`, `authorize_preview`, peer cgroup origin (once per connection, §1.5), `remote_view` for a `soos-remote.service` peer, local seat session check, per-UID rate limit | standard `Response` with `ProtocolError`/`UidMismatch`, zero pixel bytes |
| `Event` (`PasswordFailed`) | root peer for any UID, any other peer for itself | target UID versus peer UID, per-peer event quota | dropped with a `warn`; events never get a response |
| Presence auto-unlock (no message; daemon-internal, GitHub #323) | nobody: the daemon itself, for the owner of one locked local session | kill switch, logind snapshot (bound + `LockedHint`), lock grace and scan interval, template binding, account guard, lid/screen gates, PAM priority, rate-limit reserve, camera wake + PAD consensus + match, fresh logind re-check, fresh account check, kill-switch re-check, then `UnlockSession` (§6) | no unlock; the locker and its password path are untouched |

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
   Right after both stores are opened, `pipeline::sweep_orphaned_store_temp_files` removes the
   temporary files an interrupted write left behind (GitHub #291): an evidence write abandoned at
   the previous shutdown (see §5) or an interrupted `soos-enroll` template write. It runs
   `BiometricStore::sweep_orphaned_temp_files` and `EvidenceStore::sweep_orphaned_temp_files`,
   which remove only the stores' exact temporary names (regular single-link files owned by
   root, older than 60 s, at most 256 per store, never following a symlink; see
   `Docs/BIOMETRIC_STORE_CRATE.md` and `Docs/EVIDENCE_STORE_CRATE.md`). The sweep is
   housekeeping: a failure is logged at `warn` and never stops the startup; only counts are
   logged.
4. Inference warm-up (GitHub #276): `pipeline::warmed_inference_gate` runs every vision stage
   `WARMUP_PASSES` (2) times on blank inputs on the blocking pool and seeds the inference
   latency estimate with the last pass, so the first `Auth` request is admitted against a
   measured latency. A failing or panicking warm-up keeps `DEFAULT_INFERENCE_ESTIMATE_MS` and
   never stops the start-up. The estimate is clamped to `MAX_INFERENCE_ESTIMATE_MS` and, when
   a request is finalized by the estimate admission gate without evaluating any capture,
   decays toward `DEFAULT_INFERENCE_ESTIMATE_MS` (`InferenceEstimator::decay_toward_default`,
   GitHub #315), so a slow warm-up pass cannot disable face authentication for 1000 ms PAM
   stacks until a restart; that request itself still fails closed (`Unavailable` / `Timeout`).
5. The socket is bound (§1.2). When started by systemd (`Type=notify`, `NOTIFY_SOCKET` set) the
   daemon then sends `READY=1` through `soos_daemon::sd_notify`; only then does systemd start
   units ordered after it (`display-manager.service`). A sent notification is logged once as
   `Reported readiness to systemd (ready_sent_monotonic_us=<us>)`, where `<us>` is the
   `CLOCK_MONOTONIC` time in microseconds (ASCII digits) read immediately before the `READY=1`
   datagram was sent, or as `Reported readiness to systemd (ready_sent_monotonic_us=unknown)`
   when the clock could not be read (the send is never prevented by a clock error). The value
   is in the message text, not a structured field, so ANSI styling never alters it; the systemd
   acceptance harness requires it to be no later than the unit's
   `ActiveEnterTimestampMonotonic` (GitHub #333). A notification failure is logged at
   `warn`. The accept loop starts, and `STOPPING=1` is sent on SIGTERM / SIGINT (GitHub #203).
   Under `PrivateNetwork=yes` only a filesystem `NOTIFY_SOCKET` (systemd's default) is
   reachable; an abstract `@` address belongs to the host network namespace and cannot be
   reached from the unit's private one.

## 5. Shutdown, Panic Reporting and Log Anonymization (GitHub #257, #258, #259)

- **Graceful shutdown**: on SIGINT or SIGTERM `accept_until_shutdown` returns, the listener is
  dropped (no new connection is accepted), `socket_ready` is cleared and the socket file is
  unlinked (`SocketGuard`). The remaining handlers are then drained by
  `ConnectionTasks::drain(connection_timeout)`; handlers still running when that budget expires are
  aborted (the PAM client sees EOF and returns `PAM_IGNORE`). The drain logs its start
  (`in_flight`, `budget_ms`) and its outcome (`completed`, `panicked`, `aborted`).
- **Evidence writes at shutdown** (GitHub #287, #310): the opt-in spoof evidence snapshot and the
  `PasswordFailed` evidence snapshot are written on the blocking pool (never on a Tokio worker:
  encryption, `fsync` and the retention `flock` block) without delaying the response, but they
  are tracked in
  `ConnectionDispatcher::evidence_writes()` (`soos_daemon::shutdown::BlockingTasks`) instead of
  being detached. After the connection drain, `soos-daemon` waits for the tracked writes with what
  is left of the same one-`connection_timeout` budget (`drain_budget.saturating_sub(elapsed)`); a
  write still running at the deadline is reported as abandoned (a blocking write cannot be
  cancelled). Finished writes are reaped on every spawn, so the set stays bounded. The retention
  rotation that follows every write waits at most `EVIDENCE_LOCK_TIMEOUT` (5 s) for the evidence
  base-directory lock, so a running `soos-enroll migrate` can never pin a blocking thread
  indefinitely either (`password_failed_evidence_offload_tests.rs`, rows SKE2–SKE3).
- **Bounded process exit** (GitHub #289): `soos-daemon` builds its Tokio runtime explicitly
  (no `tokio::main` attribute, whose runtime drop waits without bound for every `spawn_blocking`
  job) and ends with `soos_daemon::shutdown::shutdown_runtime(runtime, remaining)`, i.e.
  `Runtime::shutdown_timeout`, where `remaining = remaining_budget(drain_started, drain_budget,
  now)` is what is left of the same one-`connection_timeout` budget. An abandoned evidence write,
  or an inference left running by an aborted handler, is therefore not awaited: process exit
  stays bounded by the drain budget (plus the 100 ms abort reap). A startup failure shuts the
  runtime down with a zero budget (nothing was served).
- **Accept errors** (GitHub #287): a failed `accept()` (`EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM`,
  ...) is logged at `error` level with `retry_in_ms` and retried after a bounded exponential
  `AcceptBackoff`: `ACCEPT_BACKOFF_INITIAL` (5 ms), doubled after each consecutive error, capped at
  `ACCEPT_BACKOFF_MAX` (1 s), reset by the next successful accept. The loop never stops; the
  shutdown signal interrupts a backoff sleep, and finished handlers keep being reaped during it.
- **Panics**: a panicking connection handler is reported at `error` level ("Connection handler
  panicked") when its task is joined; the `JoinError` is never formatted because its `Display`
  carries the panic payload. `install_panic_hook` replaces the default stderr hook with a `tracing`
  `error` event that records only the source file, line and thread name, never the panic message.
  `main.rs` selects the hook with `PanicMessagePolicy::for_build()` (GitHub #287, owner decision
  2026-10-01): a release build (no `debug_assertions`) installs exactly that payload-free hook; a
  debug build installs `install_panic_hook_with(PanicMessagePolicy::LogMessage)`, which also logs
  the panic message as `panic_message`, truncated to `MAX_DEBUG_PANIC_MESSAGE_CHARS` (512)
  characters. Debug builds are developer builds only; every packaging path builds release.
- **Request nonce in logs**: the 256-bit `request_id` is never logged. A log line that needs to
  correlate a request uses `request_id = %short_request_id(&req.request_id)`: the first 4 bytes of
  `SHA-256("soos.request-id.log.v1" || request_id)` as 8 hex digits, which reveals no nonce bit
  (enforced by `crates/daemon/tests/request_id_logging_tests.rs`).
- **Response timestamps**: `build_response` stamps every `Response` from the dispatcher clock
  through the pure function `soos_daemon::dispatcher::stamp_response` (GitHub #287):
  `issued_monotonic_ns > 0` and `expires_monotonic_ns = issued + RESPONSE_VALIDITY_NS` (2 s) on every
  verdict path. If the clock fails, the response carries `issued = expires = 0` (already expired)
  and an `Allow` verdict is downgraded to `Unavailable` / `InternalError`. Since GitHub #287 the
  PAM client enforces both fields against its own CLOCK_MONOTONIC reading and returns
  `PAM_IGNORE` for an unstamped, inverted, future-dated or expired response (see
  `Docs/IPC_PROTOCOL.md`, "Response Freshness").

## 6. Presence Auto-Unlock (GitHub #323)

`soos-daemon` unlocks a locked local session when the face of its owner is verified, without a
keypress and without PAM, through systemd-logind `Manager.UnlockSession` (ADR 2026-10-02
"Presence Auto-Unlock Through logind", ARCHITECTURE invariant 6). Source of truth:
`crates/daemon/src/presence/` and `crates/daemon/src/consensus.rs`.

**Start and stop.** The worker starts after `READY=1` (startup never waits for the system bus)
when `[presence] enabled = true` and `[dispatcher] enforce_active_session = true` (the harness
mode never starts it). At shutdown it is signalled right after the accept loop returns, joined
for at most 500 ms, then aborted; no attempt and no unlock starts after the stop signal. A
worker panic is logged once (`error`) and disables presence until the next daemon start.

**One tick per second** (`LOCK_POLL_INTERVAL_MS`), each step failing closed:

1. Kill switch: `/etc/soos/disabled` (the global flag, which also disables face PAM) or
   `/etc/soos/presence.disable` (presence only), as any entry kind, or a stat error other than
   "not found", stops presence and restarts every grace. `gdm.disable` and the other
   per-service PAM flags do **not** stop presence. No restart is needed in either direction.
2. Enrollment probe (GitHub #325): while the biometric store holds no template file (a bounded
   listing of `/var/lib/soos/biometrics`, at most 4096 entries, nothing opened or decrypted),
   the tick ends as `not_enrolled` before any D-Bus traffic (no connection, no snapshot) and
   every grace restarts; a store listing error ends it as `template_store_error`. The probe runs
   every tick, so a new enrollment is picked up within one second, without a restart.
3. Connection: when none is held, the worker opens one over the pinned system bus
   `unix:path=/run/dbus/system_bus_socket` (`zbus`) under its own 1000 ms bound
   (`DBUS_CONNECT_TIMEOUT_MS`), outside the 500 ms call bound; only this step opens a
   connection. A failure or timeout ends the tick as `logind_unavailable` with the backoff below.
4. logind snapshot. The 500 ms bound (`DBUS_CALL_TIMEOUT_MS`) covers the whole snapshot as one
   unit (one `ListSessions` plus one `GetAll` per seat session), and separately each later
   logind step (re-check, lid read, `UnlockSession`); every single D-Bus round trip is bounded
   by the same value. It never covers opening the connection. Failures back off 1 s, doubling
   to 30 s, logged at `warn` once per outage. A session is eligible only if it passes the `Auth` binding predicate (owner UID,
   active, explicit `REMOTE=0`, seat, `CLASS=user`) and `LockedHint` is `true`.
5. Grace: no scan until `lock_grace_ms` after the lock was first observed; a session seen
   unlocked, inactive or gone ends its lock period. Scans of one session are spaced by
   `scan_interval_ms`.
6. Exactly one eligible session must have a current-model template and pass the account guard;
   zero or several mean no scan (no attempt, no camera wake).
7. Gates: no scan while logind reports the lid closed or every connected DRM connector is
   DPMS-off; an undetectable state does not gate. Screen-off detection is best effort: on
   atomic-KMS drivers the sysfs `dpms` attribute may stay `On` while the compositor blanks the
   screen, and scans then continue. Gate transitions are logged at `info`.
8. PAM priority: no scan while an `Auth` request is in progress; a running scan yields before
   its next inference.
9. One attempt is recorded in the shared per-UID rate limiter, never consuming the last 5. It
   is stamped with a clock read taken under the policy lock right before recording (GitHub
   #325); a clock error or a reading older than the start of the tick skips the tick
   (`clock_unavailable`) with no attempt, no camera wake and no unlock.
10. The unchanged pipeline (camera wake, `k = 3` consecutive passing captures, any spoof vetoes,
   `[pipeline.thresholds]`) runs; a spoof veto is logged at `warn` and never sealed as evidence.
   **Wake settle (GitHub #329)**: when the camera was not streaming before the scan (`is_ready()`
   sampled before `notify_activity`), captures stamped earlier than
   `PRESENCE_WAKE_SETTLE_MS` (1000 ms, `crates/daemon/src/presence/mod.rs`, not configurable) after
   the wake are never evaluated, because the sensor's auto-exposure is still converging and
   MiniFASNet classifies such frames as spoof. The bound travels in the request window
   (`RequestDeadline::with_not_before`, read by `run_face_consensus` through `not_before_ns()`);
   a skipped capture is neither a pass nor a spoof (no admission, no inference, no estimate
   decay), and the 900 ms decision budget starts after the settle. **Stream start (GitHub #331)**:
   the bound is `max(wake ? scan start + settle : 0, stream start + settle)`
   (`presence_settle_window`), where the stream start is `CameraManager::stream_started_mono_ns()`
   (the V4L supervisor's CLOCK_MONOTONIC stamp of the stream set-up; `None` by default), so a
   camera woken by a PAM request less than 1 s before the scan is settled too. A scan of a camera
   streaming for longer than the settle applies no settle. Presence only: the PAM path computes its window with `RequestDeadline::compute`
   (bound 0, nothing skipped) and keeps the instant wake (`warmup_frames` = 0); PAD rules and
   thresholds are unchanged, so any settled spoof capture still vetoes the scan.
11. After an `Allow`, a fresh logind re-check of the same session ID (still bound, locked, same
   UID and same `Name`), a fresh account check, a lid re-check (a lid closed during the scan
   refuses; a read error does not) and a kill-switch re-check must pass within 1000 ms
   (`MAX_ALLOW_TO_UNLOCK_MS`), measured on both `CLOCK_MONOTONIC` and `CLOCK_BOOTTIME` (a
   suspend in between expires the `Allow`; a boot-clock error refuses); then `UnlockSession` is called once (never retried for the
   same `Allow`) and one `info` line names the session ID and the UID. A locker still locked 5 s
   later is not scanned again until its next lock period.

**Account guard.** Owner rule: a presence unlock respects `pam_faillock` and account / password
expiry; locked, expired **or undeterminable** means no unlock, and the password stays the only
path. Before every scan and again after the `Allow`, the guard (files re-read every time):

- refuses UID 0;
- applies the `/etc/pam.conf` rule (GitHub #325): libpam reads `/etc/pam.conf` only when none
  of `/etc/pam.d`, `/usr/lib/pam.d` and `/usr/etc/pam.d` is a directory. A present
  `/etc/pam.conf` (any type) without any of these directories is undeterminable (the guard does
  not model a `pam.conf`-only system); next to a PAM directory it is scanned like a stack file
  (a policy option, or a special, unreadable, oversized or non-UTF-8 file, is undeterminable).
  A comment-only `pam.conf`, as shipped by Debian and Ubuntu, stays usable;
- scans every file of `/etc/pam.d`, `/usr/lib/pam.d` and `/usr/etc/pam.d` (symbolic links
  followed like libpam, so authselect stacks are seen): a line holding `pam_faillock.so`
  followed anywhere later by a policy option (`deny=`, `dir=`, `fail_interval=`,
  `unlock_time=`, `root_unlock_time=`, `admin_group=`, `conf=`, `even_deny_root`) makes the
  state undeterminable. The scan is a plain substring search after the first `pam_faillock.so`
  of the line, independent of the tokenizer: it deliberately over-detects compared with libpam
  (any whitespace, Unicode included, any brackets, a `\` before a `#` comment joining the next
  line), so it can never miss an option libpam passes, and a false positive only refuses
  presence. A bracketed control field before the module such as `[success=1 default=bad]` is
  not matched. An `include`, `substack` or `@include` of a path (a target holding `/`:
  absolute, nested or `..`) is undeterminable, since the guard scans only the top level of the
  three directories; plain names (`include system-auth`) stay usable. The module is recognised
  by its name only: a renamed copy of `pam_faillock.so` is not seen. The guard assumes libpam's
  `VENDORDIR=/usr/etc`; a libpam built with another `VENDORDIR` reads `<VENDORDIR>/pam.d`
  stacks that the guard never scans. Keep faillock policy in `/etc/security/faillock.conf`,
  not on PAM lines;
- reads `faillock.conf` (`/etc/security/faillock.conf`; when absent, the built-in defaults
  3 / 900 s / 600 s **and** the vendor `/usr/etc/security/faillock.conf` are both evaluated,
  strictest wins); an unknown key or malformed value is undeterminable;
- reads the tally `/run/faillock/<user>` (or the configured `dir`) with `O_NOFOLLOW` under a
  shared non-blocking `flock` and applies the `pam_faillock` `check_tally` rule; a tally
  directory that is a symlink or group/other-writable is undeterminable; with `admin_group`
  set, the stricter unlock time applies;
- reads `/etc/shadow`: account expired or inactive, password expired, forced change
  (`lastchg = 0`) or locked password (a field starting with `!` or `*`, e.g. `*LK*`) refuse; a missing or duplicated line is
  undeterminable, so NSS-only (LDAP/SSSD) and systemd-homed users never get presence unlock.

There is no tally reset: a presence unlock never writes `/run/faillock` (it is not a PAM authentication;
the guard already refused while locked). `CAP_DAC_OVERRIDE` (kept in the unit's
`CapabilityBoundingSet`) is required to read the `0660 user:root` tally and, on distributions
that ship `/etc/shadow` as `0000 root:root` (Fedora, RHEL), the shadow file; without it the
guard refuses every unlock. Other account modules (`pam_access`, `pam_time`, `pam_nologin`,
`pam_tally2`) are not consulted.

**Logging.** Users are identified by UID and session ID above `debug`, never by name; no frame,
embedding, template, score, file content or D-Bus body is logged. Skip reasons are logged at
`debug` on change only.

**Known limits.** The session owner can set `LockedHint` itself: `false` under a running locker
only denies presence to that owner, `true` while unlocked only turns the camera on (logind
refuses `SetLockedHint` on another user's session). Any user able to hold more than 1024 logind
sessions (for example over SSH) makes every tick `TooManySessions` and disables presence for
everyone until they close (fail closed; PAM is unaffected). Locking while seated unlocks again
after the grace period, by design.
