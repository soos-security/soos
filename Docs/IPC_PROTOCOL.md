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
│  hard timeout: 250ms    │  Response (Postcard)│  V4L2 Camera + ONNX IA  │
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
- `kind: RequestKind`: Request operation (`Authenticate`).
- `request_id: RequestId`: 256-bit cryptographic random identifier (`[u8; 32]`) sourced via `getrandom`.
- `uid_hint: u32`: Declared UID from the PAM client (authoritatively cross-checked by the daemon using kernel `SO_PEERCRED`).
- `service: String`: PAM service name (`"sudo"`, `"su"`, `"gdm-password"`...). Bounded to 64 bytes.
- `deadline_monotonic_ns: u64`: Absolute monotonic deadline in nanoseconds. If exceeded, daemon immediately returns `Verdict::Unavailable`.

### `Response`
Returned by the daemon to the PAM module:
- `request_id: RequestId`: Must match the initial request's identifier bit-for-bit.
- `verdict: Verdict`:
  - `Allow`: Single face authenticated successfully (PAD validated, match score >= threshold).
  - `Deny`: Authentication failed (no face, multiple faces, low score, PAD anti-spoof rejected).
  - `Unavailable`: Hardware offline, model uninitialized, or deadline expired.
  - `ProtocolError`: Malformed message, mismatched UID, rate-limit reached.
- `reason_class: ReasonClass`: Internal telemetry diagnostic (must not alter PAM fallback semantics).
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

1. **Zero Secrets on Wire**: Neither PAM passwords, biometric embeddings, nor camera frames ever travel over the IPC socket.
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
