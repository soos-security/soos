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
- `service: String`: PAM service name.
- `timestamp_monotonic_ns: u64`: Monotonic timestamp.

### Memory Zeroization (`Zeroize`)
The `Response` struct implements `Zeroize` and `ZeroizeOnDrop`: sensitive request identifiers and verdict metadata are overwritten in RAM when discarded.

---

## 4. Strict Security Invariants

1. **Zero Secrets on Wire**: Neither PAM passwords, biometric embeddings, nor camera frames ever travel over the IPC socket.
2. **Fail-Closed Fallback (`PAM_IGNORE`)**: Any verdict other than `Allow` (`Deny`, `ProtocolError`, `Unavailable`), network error, or timeout immediately returns `PAM_IGNORE`, seamlessly delegating to fallback modules (`pam_unix.so`).
3. **Single-Use Binding**: Responses are bound to unique 256-bit nonces and cannot be logically replayed.
