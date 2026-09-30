# IPC Protocol v1 Specification

> Crate: `crates/protocol` (`soos-protocol`)  
> Status: Implemented and Tested  

---

## 1. Overview

The IPC protocol provides synchronous, bounded, bidirectional communication between the unprivileged PAM module `pam_soos.so` (executing inside the caller PAM process: `sudo`, `login`, `gdm`, etc.) and the privileged root daemon `soos-daemon`.

```
┌─────────────────────────┐                     ┌─────────────────────────┐
│       pam_soos.so       │  Request (Postcard) │       soos-daemon       │
│  (unprivileged context) │ ──────────────────> │      (root process)     │
│                         │ <────────────────── │                         │
│  deadline: timeout_ms   │  Response (Postcard)│  V4L2 Camera + ONNX IA  │
└─────────────────────────┘                     └─────────────────────────┘
```

---

## 2. Wire Frame Format (Codec v1)

Every message transmitted over the Unix domain stream socket is framed by a 4-byte (`u32`) Big-Endian length prefix, followed by the payload serialized using the compact binary format **Postcard**:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                  Payload Length (u32, Big-Endian)             |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
|                  Serialized Payload (Postcard)                |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### Security Constants
- **Maximum Message Size (`MAX_MESSAGE_SIZE`)**: `4,096` bytes (4 KiB). Any message declaring a length exceeding 4 KiB is rejected immediately prior to buffer allocation (`CodecError::MessageTooLarge`), shielding daemon and PAM module from denial-of-service memory exhaustion.
- **Maximum Service Name Length (`MAX_SERVICE_LEN`)**: `64` bytes. Rejected with `CodecError::PayloadCorrupted` if exceeded.
- **Protocol Version (`PROTOCOL_VERSION`)**: `1`.

---

## 3. Message Schemas

### `Request`
Sent by the PAM module to the daemon to request facial verification:
- `kind: RequestKind`: Request operation (`Auth`, `Status`, or `PreviewFrame` — the latter is reserved for the diagnostic GUI and governed by §9).
- `request_id: RequestId`: 256-bit cryptographic random identifier (`[u8; 32]`) sourced via `getrandom`.
- `uid_hint: u32`: Declared UID from the PAM client (authoritatively cross-checked by the daemon using kernel `SO_PEERCRED`).
- `service: String`: PAM service name (`"sudo"`, `"su"`, `"gdm-password"`...). Bounded to 64 bytes.
- `deadline_monotonic_ns: u64`: Absolute monotonic deadline in nanoseconds. If exceeded, daemon immediately returns `Verdict::Unavailable`. `0` or `u64::MAX` means no client deadline (`DECISION_BUDGET_MS` applies). The daemon stops its decision `RESPONSE_WRITE_MARGIN_MS` (50 ms) before the earlier of this deadline and its own `connection_timeout` (measured from the start of request processing), and never starts an inference that would not finish in time; the response is then `Unavailable`/`Timeout` (or the consensus reached so far), never a silent overrun.

### `Response`
Returned by the daemon to the PAM module:
- `request_id: RequestId`: Must match the initial request's identifier bit-for-bit.
- `verdict: Verdict`:
  - `Allow`: Single face authenticated successfully (PAD validated, match score >= threshold).
  - `Deny`: Authentication failed (no face, multiple faces, low score, PAD anti-spoof rejected).
  - `Unavailable`: Hardware offline, model uninitialized, or deadline expired.
  - `ProtocolError`: Malformed message, mismatched UID, rate-limit reached.
- `reason_class: ReasonClass`: Internal telemetry diagnostic (must not alter PAM fallback semantics). It is **never** shown to the user: the PAM module maps every `Deny` to one neutral text and every other failure to one generic text, so a PAD rejection is indistinguishable from a non-match at the lock screen (review PAM-03, GitHub #174; see `Docs/PAM_MODULE.md` §8).
- `issued_monotonic_ns: u64`: Generation timestamp.
- `expires_monotonic_ns: u64`: Short expiration timestamp preventing replay.

### `Event`
Best-effort telemetry notification sent by PAM following password failures:
- `version: u8`: Protocol version (must be 1).
- `kind: EventKind`: `PasswordFailed`.
- `request_id: Option<RequestId>`: Related authentication request if applicable.
- `uid: Option<u32>`: Target POSIX user ID whose authentication failed, if known.
- `service: String`: PAM service name.
- `timestamp_monotonic_ns: u64`: Monotonic timestamp.

### Memory Zeroization (`Zeroize`)
The `Response` struct implements `Zeroize` and `ZeroizeOnDrop`: sensitive request identifiers and verdict metadata are overwritten in RAM when discarded.

---

## 4. Strict Security Invariants

1. **Zero Secrets on Wire**: Neither PAM passwords nor biometric embeddings ever travel over the IPC socket. Camera frames travel only on the authorized diagnostic preview stream described in §9, never towards the PAM module.
2. **Fail-Closed Fallback (`PAM_IGNORE`)**: Any verdict other than `Allow` (`Deny`, `ProtocolError`, `Unavailable`), network error, or timeout immediately returns `PAM_IGNORE`, seamlessly delegating to fallback modules (`pam_unix.so`).
3. **Single-Use Binding**: Responses are bound to unique 256-bit nonces and cannot be logically replayed.

---

## 5. Fuzzing and Property-Based Verification

To ensure that untrusted or malformed inputs can never trigger memory corruption or daemon/PAM crashes:
1. **Property-Based Testing (`proptest`)**:
   - `prop_request_roundtrip`, `prop_response_roundtrip`, and `prop_event_roundtrip` assert serialization/deserialization idempotency for arbitrary valid messages.
   - `prop_decode_request_never_panics`, `prop_decode_response_never_panics`, and `prop_decode_event_never_panics` feed arbitrary mutated byte streams ($0$ to $8{,}192$ bytes) asserting that `decode` never panics and always returns either `Ok` or a typed `CodecError`.
   - `prop_declared_size_bounds` and `prop_truncated_buffer_bounds` verify zero-allocation fast rejection of oversized ($> 4{,}096$ bytes) or truncated payloads.
   - Executed automatically via standard `cargo test` on every commit and CI run.
2. **LLVM libFuzzer Integration (`cargo-fuzz`)**:
   - Targets `decode_request`, `decode_response`, and `decode_event` in `crates/protocol/fuzz/`.
   - Supports continuous coverage-guided fuzzing over millions of iterations:
     ```bash
     cargo +nightly fuzz run decode_request -- -runs=1000000
     cargo +nightly fuzz run decode_response -- -runs=1000000
     ```

---

## 6. Socket Lifecycle, TOCTOU Protection & Permission Hardening

### Socket Path & Permissions
- **Socket Path**: `/run/soos/daemon.sock`
- **File Mode**: `0660` (`srw-rw----`)
- **Ownership**: `root:soos` (UID `0`, GID of `soos` system group)

### TOCTOU-Safe Binding Architecture
To prevent symlink substitution, race conditions, and privilege escalations, `soos-daemon` adheres to a strict descriptor-relative binding sequence:
1. **Directory Descriptor Verification**: Opens the parent runtime directory (`/run/soos`) with `O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW` and asserts via `fstat` that it is not a symlink, not world-writable, and root-owned.
2. **Critical Section Serialization**: Acquires an exclusive non-blocking file lock (`nix::fcntl::Flock`) on the directory descriptor, serializing socket creation and preventing concurrent daemon startup races.
3. **Descriptor-Relative Stale Check & Cleanup**: Inspects any existing socket node using `fstatat` with `AT_SYMLINK_NOFOLLOW`. Stale symlinks or non-socket files are immediately rejected fail-closed; verified stale sockets are unlinked via `unlinkat(..., NoRemoveDir)`.
4. **Post-Bind Invariant Assertion**: Immediately verifies via `fstatat` that the created filesystem entry is a genuine Unix socket.
5. **Symlink-Safe Permission & Ownership**: Applies mode `0660` using `fchmodat` with `NoFollowSymlink` and assigns `root:soos` ownership using `fchownat` with `AT_SYMLINK_NOFOLLOW`.
6. **Early Dispatcher Wire Validation**: The connection dispatcher invokes `Request::validate()` immediately after decoding, rejecting invalid protocol versions or oversized service names fail-closed with `Verdict::ProtocolError` and `ReasonClass::MalformedRequest`.

---

## 7. Async Cancellation Safety & Response Completeness Validation

### Decoupled Request Processing and Response Transmission
In `soos-daemon`, the connection lifecycle is explicitly decoupled into two non-overlapping phases:
1. **Request Reading & Verification Phase**: Runs under Tokio `timeout(connection_timeout, ...)`. The daemon reads and decodes the message, performs peer credential verification, and executes the vision verification pipeline. The resulting response is fully encoded into an in-memory byte buffer (`Vec<u8>`). Zero socket write syscalls are performed during this phase.
   - If the request processing times out, the future is cancelled *before* any response bytes are written to the socket. The stream is closed with 0 bytes sent, guaranteeing that partial or corrupted response fragments never reach the PAM client.
2. **Response Transmission Phase**: Runs *after* the request processing timeout has completed cleanly. The pre-encoded buffer is transmitted atomically via `tokio::io::AsyncWriteExt::write_all` and `flush` under a dedicated write timeout, protecting worker threads from slow-client stalls.

### Client-Side Response Completeness Validation
In `pam_soos.so`, the synchronous IPC client (`crates/pam/src/ipc.rs`) enforces strict byte-counted frame completeness:
- Both the 4-byte Big-Endian length header and the variable-length body payload are read using a byte-counted stream reader.
- If the socket connection is severed prematurely before the declared frame is fully received, the client detects the discrepancy and raises `IpcError::TruncatedResponse { expected, received }`.
- In accordance with fail-closed security invariants, any truncated response degrades safely to `PAM_IGNORE`.
- The whole exchange shares one cumulative deadline derived from the clamped `timeout_ms`: the socket timeout is re-armed with the remaining budget before every `read()` / `write()` syscall, and a response completed after the deadline is discarded (`IpcError::Timeout`). A peer sending one byte at a time cannot extend the wait (review PAM-02, GitHub #173).

---

## 8. Client Connection Timeout & Memory Zeroization

### Non-Blocking Connection with Timeout
To prevent `pam_soos.so` from blocking indefinitely if `soos-daemon` is unresponsive or its listen queue is full, the client does not use a blocking connect:
1. `connect_with_timeout` creates an `AF_UNIX` socket with `SOCK_NONBLOCK` and `SOCK_CLOEXEC`.
2. `libc::connect` is called; if it returns `EINPROGRESS`, `libc::poll` monitors the socket for writability within the remaining time budget.
3. Socket status is verified using `getsockopt` (`SO_ERROR`).
4. Upon successful connection, `stream.set_nonblocking(false)` restores blocking mode for read and write timeouts (`SO_RCVTIMEO` / `SO_SNDTIMEO`).

### Memory Zeroization
All sensitive payloads and buffers in the IPC pipeline are scrubbed on drop:
- `Request` implements `zeroize::Zeroize` and `Drop`, zeroing `request_id`, `service`, and metadata.
- `Response` implements `zeroize::Zeroize` and `Drop`, resetting `verdict` to `Deny` and zeroing `request_id` and timestamps.
- `PreviewResponse` implements `zeroize::Zeroize` and `Drop`, erasing the pixel buffer and metadata; the daemon wraps every encoded response (`ResponseOutput::encoded_response`) in `zeroize::Zeroizing`.
- Raw message buffers (`encoded`, `len_buf`, `full_buf`) are wrapped in `zeroize::Zeroizing` to ensure cryptographic hygiene.

---

## 9. Camera Preview Stream Authorization (`RequestKind::PreviewFrame`)

Camera frames are protected biometric data (`AI/ARCHITECTURE.md` §1). `RequestKind::PreviewFrame` lets the diagnostic GUI (`soos-gui`) display the daemon-owned camera without opening `/dev/video*` itself. Since GitHub #143 (review findings CAM-01 / DMN-02) the daemon treats it as a privileged operation.

### Daemon configuration (`/etc/soos/daemon.toml`)

```toml
[preview]
enabled = false            # default: only a root peer may request preview frames
allowed_uids = []          # unprivileged peer UIDs allowed when enabled = true (max 64 entries)
max_requests_per_sec = 40  # per peer UID sliding window (root included); 0 refuses every request
```

Constants live in `crates/daemon/src/preview.rs`: `MAX_PREVIEW_ALLOWED_UIDS = 64`, `DEFAULT_PREVIEW_MAX_REQUESTS_PER_SEC = 40`, `PREVIEW_RATE_WINDOW_NS = 1 s`, `PREVIEW_RATE_MAX_TRACKED_UIDS = 64`. A configuration with more than 64 allow-listed UIDs is rejected at startup (`DaemonError::Config`). The dispatcher created without `with_preview_config` uses `PreviewConfig::default()` (fail-closed).

### Dispatcher decision order (`ConnectionDispatcher::handle_preview_request`)

| Step | Check | Refusal (`Response`, zero pixel bytes) |
|---|---|---|
| 6 | Kernel `SO_PEERCRED` UID versus `uid_hint` (`verify_peer_credentials`) | `ProtocolError` / `UidMismatch` |
| 6c-1 | `authorize_preview`: `peer_uid == 0`, or `enabled` and `peer_uid ∈ allowed_uids` and `peer_uid == uid_hint` | `ProtocolError` / `UidMismatch` |
| 6c-2 | Unprivileged peer owns an active logind session not flagged `REMOTE=1` (`SessionValidator`) | `ProtocolError` / `UidMismatch` |
| 6c-3 | Per-peer-UID rate limit (`soos_policy::RateLimiter`, `check_and_record`) | `ProtocolError` / `RateLimited` |
| 6c-4 | Monotonic clock available | `Unavailable` / `InternalError` |

Only after these checks does the daemon call `camera.notify_activity()`, wait for readiness and copy the latest capture into a `PreviewResponse` (`format = 255` and empty `data` when no capture is available). The camera is therefore never woken, and the privacy LED never lit, by an unauthorized peer.

### Client contract (`soos-gui` `IpcCameraManager`)

- Each request carries a fresh 256-bit `request_id` (`getrandom`) and the caller's real UID as `uid_hint`; a refusal is recognised as a `Response` bounded by `MAX_MESSAGE_SIZE` whose `request_id` matches the nonce.
- `IpcPreviewError::Unauthorized` stops the polling worker (no reconnect storm); `RateLimited` backs off 250 ms; `Unavailable` backs off 100 ms; `Protocol` / `Io` reconnect after 200 ms.
- `IpcCameraManager::probe_preview` performs one round-trip at GUI start-up. On refusal the GUI does **not** fall back to direct V4L2 access (GitHub #150): the daemon owns the camera, so the GUI shows an actionable "Camera unavailable" notice (`soos_gui::camera_mode::CameraBlockReason`) instead of fighting the daemon for the device with `EBUSY`.
- Direct V4L2 access is selected only when the daemon is provably not running: `connect(2)` on the socket fails with `ENOENT`/`ECONNREFUSED` **and** `systemctl is-active soos-daemon.service` is false. `EACCES`/`EPERM` (user not in the `soos` group, `/run/soos` is `0750 root:soos`) is reported as "add the user to the `soos` group, then log out and back in".

---

## 10. Local Session Binding for `RequestKind::Auth` (GitHub #160)

After the `SO_PEERCRED` check (Step 6) the dispatcher runs `LocalSessionPolicy::authorize_auth` (Step 6b, `crates/daemon/src/session_policy.rs`) on every facial `Auth` request. The kernel peer PID captured at `accept` is passed through; it is never taken from the payload.

| Peer | Requirement | Typical caller |
|---|---|---|
| `peer_uid == 0`, caller in a session scope | The peer PID maps (`/proc/<pid>/cgroup`, first unit below the slices is `session-<id>.scope`, as `sd_pid_get_session` resolves it) to a logind session whose record in `/run/systemd/sessions/<id>` has `UID == uid_hint`, `ACTIVE=1` or `STATE=active`, `REMOTE=0`, a non-empty `SEAT=` and `CLASS=user` | `sudo` in a TTY or in a terminal started inside the session scope, lock-screen worker in the user's session |
| `peer_uid == 0`, caller under the user manager | The peer PID is in no session scope and its cgroup (unified `0::` line and/or v1 `name=systemd` line, all agreeing) is `/user.slice/user-<uid>.slice/user@<uid>.service/<child>...` with both `<uid>` strictly decimal (no sign, no leading zero, fits `u32`) and equal to `uid_hint`; the target owns at least one session with the record requirements above; the target owns **no** session with `REMOTE=1` or an unknown remote flag, in any state (a record without `UID=` counts as possibly the target's) | `sudo` in a GNOME ≥ 3.34 / KDE Plasma ≥ 5.25 terminal (`app.slice/.../vte-spawn-*.scope`, `app-org.kde.konsole-*.scope`), polkit agent of `org.gnome.Shell@wayland.service` |
| `peer_uid == uid_hint` (non-root) | The target owns an active session not flagged `REMOTE=1` | screen lockers running PAM as the user |

Refused (`ProtocolError` / `UidMismatch`, PAM falls back to the password): `sudo` or `su` from an SSH session, `su <victim>` from another user's local session or from another user's desktop terminal (`user@<other>.service`, `user_manager_uid_mismatch`), face `sudo` from the desktop while the same account also has an SSH session (`user_manager_caller_remote_session_active`: that session could reach the user manager through `systemd-run --user`), a malformed user-manager cgroup (`user_manager_cgroup_malformed`), a user-manager caller whose target has no local seat session (`user_manager_no_local_seat_session`), `sshd` logins (sshd is in no session), a remote-only user, a missing or non-positive peer PID, a session that vanished, an unreadable or oversized logind/procfs state. The daemon logs a stable reason code (`SessionDenial::as_str`, e.g. `caller_session_remote`) with the peer and target UIDs only; session contents such as `REMOTE_HOST` are never logged. Session IDs are validated as ASCII alphanumerics of at most 64 bytes before they are joined to the sessions directory, record files are read only if they are regular files (no symlink following) and are bounded to 4 KiB, cgroup files to 16 KiB, and the directory scan to 1024 entries (larger fails closed).

## 11. Per-Peer Connection Limits and Event Quota (GitHub #157, #175)

Since GitHub #157 (review finding DMN-04) connection admission is keyed on the kernel `SO_PEERCRED` UID instead of a single global semaphore, so one `soos`-group account can no longer hold every permit and deny face login to everyone. The PAM module runs inside root processes (`gdm-session-worker`, `sudo`, `su`, `login`, `polkit-agent-helper-1`), which is why root peers get a reserved share of the capacity.

### Daemon configuration (`/etc/soos/daemon.toml`)

```toml
[peer_limits]
max_connections_per_uid = 2          # concurrent connections of one unprivileged UID (root exempt)
reserved_root_connections = 2        # permits of [dispatcher] max_concurrent_connections usable only by root
max_requests_per_connection = 1024   # requests served on one connection before it is closed
max_connection_lifetime_ms = 30000   # a connection is not read again after this age
max_events_per_window = 5            # PasswordFailed events per peer UID (root included); 0 drops all
event_window_ms = 10000              # sliding window of the event quota
```

Constants live in `crates/daemon/src/limits.rs` (`DEFAULT_*`, `EVENT_RATE_MAX_TRACKED_UIDS = 256`). `PeerLimitsConfig::validate` rejects at startup (`DaemonError::Config`) a zero capacity, per-UID cap, request cap, lifetime or event window, and a `reserved_root_connections` that is not strictly below `[dispatcher] max_concurrent_connections` (e.g. `max_concurrent_connections = 2` now requires `reserved_root_connections <= 1`). A dispatcher built without `with_peer_limits` uses `PeerLimitsConfig::default()`; the library limiter additionally clamps the reservation so at least one unprivileged permit always remains.

### Admission and connection lifecycle (`ConnectionDispatcher::handle_connection`)

1. `SO_PEERCRED` is read once, before any byte of the stream; failure closes the connection.
2. `PeerConnectionLimiter::try_acquire(peer_uid)` refuses the peer with `GlobalCapacity` (every permit in use), `ReservedForPrivileged` (unprivileged peer and only root-reserved permits remain) or `PerUidCap` (the unprivileged UID already holds `max_connections_per_uid`). The refusal is logged with `peer_uid` and the reason, and the stream is closed immediately without reading or answering: the PAM client sees EOF and returns `PAM_IGNORE` (password fallback), and nothing ever waits on a permit. The limiter table holds only UIDs with a connection in use, so it is bounded by the capacity.
3. An admitted connection serves requests until the client closes it, it stays idle past `connection_timeout`, `max_requests_per_connection` requests were served, or `max_connection_lifetime` has elapsed (checked before each read, so the worst-case lifetime is the cap plus one request/response cycle).
4. An `Auth` request is one-shot: the daemon closes the connection right after writing its `Response` (the PAM client already drops it). `Status`, `PreviewFrame` and events may share one connection within the caps; `soos-gui` reconnects transparently (200 ms) when the daemon closes its preview connection.

### `PasswordFailed` event quota

Every `EventKind::PasswordFailed` event is first counted against a per-peer-UID sliding window (`soos_policy::RateLimiter`, root included, 256 tracked UIDs); an event beyond the quota, a full limiter table or an unavailable monotonic clock drops the event with a `warn` naming `peer_uid`, and no camera snapshot is taken. Before the quota, the target UID is authorized against the kernel peer UID (GitHub #175, ADR in `AI/DECISIONS.md`): a root peer (sudo, su, login, gdm-session-worker, polkit-agent-helper-1) may report for any UID; any other peer only for itself (`uid` absent or equal to its `SO_PEERCRED` UID). Any other event is dropped with a `warn` naming `peer_uid` and the claimed UID, and no snapshot is taken. Events carry no response, so nothing changes on the wire.
