# soos — Master Architecture and Context Document

## 1. Architectural Decisions

### Phase 1 Objective

**soos** adds local facial verification to Linux PAM stacks without modifying the user interface of session managers, lock screens, or `sudo`. A successful facial match authorizes authentication; any unavailability, uncertainty, or anomaly **falls back cleanly to the existing password prompt**. Following a password authentication failure, an anti-intrusion event can notify the daemon to preserve cryptographic evidence under an explicit privacy policy.

The system is not considered a high-assurance biometric factor until robust Presentation Attack Detection (PAD/liveness) and false-accept evaluations are integrated. Photos, digital screens, and 3D masks represent known presentation attacks documented by NIST; NIST SP 800-63B guidelines mandate PAD and measured false-match rates for facial verification.[^nist-63b] Phase 1 is strictly positioned as **convenience + local threat detection** with mandatory password fallback, not as a universal replacement for hardware security keys or master secrets.

### Key Architectural Choices

| Domain | Canonical Choice | Rationale | Prohibited Anti-Patterns |
|---|---|---|---|
| PAM/Daemon Boundary | Local Unix Domain Socket (UDS), `SOCK_SEQPACKET` if available, otherwise framed `SOCK_STREAM` | Zero network exposure, ultra-low latency, kernel-enforced peer credentials (`SO_PEERCRED`) | HTTP, loopback TCP, world-writable socket without peer verification |
| PAM Module Execution | Pure synchronous blocking Rust, `std::os::unix::net::UnixStream`, hard 200–250ms deadline | Critical authentication path must never spawn a persistent async runtime | AI inference, direct camera access, or network downloads inside `.so` |
| Privileged Daemon | Rust + Tokio root process, sole owner of `/dev/video*` and ONNX sessions | Keeps camera warm and models in memory; central arbitration | Re-opening `/dev/video0` inside PAM on every authentication attempt |
| Linux Camera Capture | `v4l` 0.14, MMAP buffers on dedicated worker thread; `nokhwa` only as prototype | Deterministic V4L2 control and predictable zero-copy buffer rotation | Depending on OpenCV or allowing competing camera consumers |
| Local AI Inference | `ort` (ONNX Runtime) CPU execution provider; UltraFace Slim 320 + MobileFaceNet | Fast, battle-tested, no OpenCV required in application code | Claiming "pure Rust" (ORT is native C/C++); unverified weight downloads |
| Biometric Storage | AES-GCM encrypted embeddings at rest; intrusion snapshots opt-in and isolated | Minimizes attack surface and persistent biometric risk | Storing raw frames or passwords on disk or sending over socket |

Verified crate versions as of September 2026: `pam-bindings` 0.3.0, `tokio` 1.53.1, `v4l` 0.14.0, `nokhwa` 0.10.11, `ort` 2.0.0-rc.13, `zeroize` 1.9.0. Versions are pinned in `Cargo.lock` and audited via `cargo-deny`.[^pam-bindings][^tokio][^v4l][^nokhwa][^ort][^zeroize]

---

## 2. Threat Model and Security Invariants

### Protected Assets
- Authorization to authenticate a local UID.
- Biometric templates (embeddings) and encryption keys.
- Camera frames and anti-intrusion evidence snapshots.
- Socket integrity, daemon binary, ONNX model weights, and PAM configuration files.
- System availability of lock screens, console logins, and `sudo`.

### Threat Actors Considered
Local unprivileged users, processes running under an attacker UID, rogue IPC socket clients, tampered ONNX model files, disconnected or captured webcams, and presentation attacks (printed photos, smartphone screens, recorded videos). Root attackers, kernel exploits, hardware camera replacement, and physical bus extraction are out of scope for Phase 1.

### Mandatory Security Invariants
1. The PAM module returns `PAM_SUCCESS` **only** upon receiving a fresh `Allow` response matched to the kernel-verified socket UID (`SO_PEERCRED`); all other scenarios return `PAM_IGNORE`.
2. Zero passwords ever transit to `soos`, are parsed by its module, or are logged.
3. The daemon never trusts the username, PID, PAM service, or UID declared in payload messages: it strictly cross-references `SO_PEERCRED`, `/etc/passwd`, and active `logind` sessions.
4. An `Allow` verdict is single-use, cryptographically bound to a 256-bit random nonce (`request_id`), target UID, service name, and monotonic deadline; it is never cached inside PAM.
5. Any timeout, panic, disconnected socket, missing camera, ambiguous face, invalid model, or internal error degrades silently to password fallback, never to authorization.

---

## 3. System Architecture

```text
PAM Caller (gdm, swaylock, hyprlock, sudo, login)
  │ pam_soos.so — synchronous path, 250ms deadline, zero camera access
  │ connect + AuthAttempt request (UID authoritatively checked by SO_PEERCRED)
  ▼
/run/soos/daemon.sock ── Local UDS ── soos-daemon (root, Tokio)
                                       │
                         CameraManager (exclusive V4L2 owner)
                                       │ latest fresh frame (RAM)
                                       ▼
                         VisionEngine: detect → align → PAD → embedding → match
                                       │
                   Allow / Deny / Unavailable, cryptographically bound to request_id
                                       │
                         EvidenceStore (only following PasswordFailed, opt-in)
```

The daemon starts as a systemd service before login prompts, loads and validates model checksums, opens the camera, and stabilizes auto-exposure. The PAM module contains only the lightweight IPC client, response interpreter, and C ABI bindings.

### Request State Matrix

| Daemon Internal State | PAM Module Return | System Effect |
|---|---|---|
| Single face, PAD passed, match score >= threshold, valid context | `Allow` | `PAM_SUCCESS`; PAM stack short-circuits to grant access |
| No face, multiple faces, low score, failed PAD anti-spoof | `Deny` | `PAM_IGNORE`; PAM proceeds silently to password prompt |
| Camera/model/socket offline, timeout exceeded, internal error | `Unavailable` | `PAM_IGNORE`; seamless password fallback |
| Malformed payload, mismatched UID, rate-limit reached | `ProtocolError` | `PAM_IGNORE`; daemon logs security warning |

`Deny` and `Unavailable` are intentionally indistinguishable to the PAM caller, preventing timing or enumeration attacks.

---

## 4. IPC Architecture (PAM ↔ Daemon)

### Socket Path and Permissions
The runtime directory is provisioned via systemd, not opportunistic daemon `mkdir`:

```ini
# /etc/systemd/system/soos-daemon.service.d/runtime.conf
[Service]
RuntimeDirectory=soos
RuntimeDirectoryMode=0750
UMask=0077
```

The daemon verifies `/run/soos` is owned by `root:soos`, is not world-writable, and is not a symlink; it unlinks its own socket node following `lstat` verification, then binds `/run/soos/daemon.sock` with mode `0660`, owner `root:soos`. Users permitted to use facial verification are added to the system `soos` group.

On every incoming connection, the daemon queries `getsockopt(..., SO_PEERCRED)`:[^unix7]
- `peer.uid == uid` of target identity.
- Target UID is an authorized local user and owns the active local graphical session.
- Bounded payload size and protocol version verified before deserialization.
- Strict per-UID rate limits and global concurrent connection caps.

The socket must never be configured as `0666` or use abstract UDS namespace.

### Wire Protocol Framing
The protocol is binary, versioned, and strictly bounded. Postcard serialization is used over `SOCK_STREAM` with a 4-byte big-endian length prefix (max size 4096 bytes).

```text
Request v1:  version | kind=AUTH | request_id[32] | uid_hint:u32 |
             service_len:u8 | service[<=64] | deadline_monotonic_ns:u64
Response v1: version | request_id[32] | verdict:u8 | reason_class:u8 |
             issued_monotonic_ns:u64 | expires_monotonic_ns:u64
Event v1:    version | kind=PASSWORD_FAILED | request_id[32] |
             peer_uid | service[<=64] | timestamp_monotonic_ns:u64
```

### Async Boundaries (Tokio vs. PAM)
- **Privileged Daemon**: Runs Tokio for IPC connection dispatching. CPU-heavy capture and inference run on dedicated worker threads with semaphore limit 1 to avoid thread exhaustion.
- **PAM Module**: **Strictly forbidden from starting Tokio**. Uses `std::os::unix::net::UnixStream` with synchronous read/write timeouts totaling 200–250ms. Immediately closes socket after response.

---

## 5. PAM Module Implementation & Universal Stack

### Crate and ABI
Built with `pam-bindings` 0.3.0 (`cdylib`), exporting `pam_sm_authenticate` and `pam_sm_setcred`.[^pam-bindings] All entry points are wrapped with `std::panic::catch_unwind(AssertUnwindSafe(...))`. Panics are caught, logged to syslog, and mapped to `PAM_IGNORE`.

```rust
// sm_authenticate conceptual flow
match ipc_auth(uid, service, deadline) {
    Ok(Allow { request_id, .. }) => PAM_SUCCESS,
    Ok(Deny | Unavailable | ProtocolError) | Err(_) => PAM_IGNORE,
}
```

### Proposed PAM Stack (`/etc/pam.d/soos-auth`)

```pam
# Placed AFTER mandatory faillock preauth, BEFORE pam_unix.
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250

# Standard password verification.
auth  [success=done default=bad]     pam_unix.so try_first_pass

# Reached only if pam_unix fails. Zero secrets accessed.
auth  optional                       pam_soos.so event=password-failed timeout_ms=20
```

---

## 6. Warm Camera Streaming & Low Latency

The daemon exclusively controls the camera by stable hardware ID (`/dev/v4l/by-id/...`), using `v4l` 0.14 MMAP streaming.[^v4l] It continuously captures frames at 640x480 (10–15 FPS) into memory, maintaining the latest frame in an `ArcSwap<LatestFrame>` buffer with monotonic timestamp.

When PAM requests authentication, the daemon grabs the snapshot from RAM (age <= 150ms) rather than waiting for camera hardware wake-up.

---

## 7. Local Vision Pipeline & Latency Budget (150ms Target)

### Models
1. **Face Detection**: UltraFace Slim 320 ONNX (~1.04MB) -> bounding boxes and confidence scores with deterministic Rust NMS.[^ultraface]
2. **Landmarks & Alignment**: 5-point landmark ONNX model -> affine transform to 112x112 aligned face crop.
3. **Presentation Attack Detection (PAD)**: Challenge/response or dedicated ONNX anti-spoofing model.
4. **Feature Extraction**: MobileFaceNet ArcFace-compatible ONNX FP32/int8 -> 128D/512D L2-normalized embedding.[^mobilefacenet]
5. **Matching**: Cosine similarity (`cosine = dot(a, b)`). Authorized only if score >= calibrated threshold and single face verified.

### 150ms Latency Budget (p95 Target)

| Segment | Budget (p95) |
|---|---:|
| IPC dispatch and RAM snapshot | 5 ms |
| Color conversion & face detection | 35 ms |
| 5-point landmarks & affine alignment | 20 ms |
| PAD liveness verification | 35 ms |
| MobileFaceNet embedding & cosine distance | 30 ms |
| OS scheduler margin | 25 ms |
| **Total Decision Budget** | **<= 150 ms** |

---

## 8. Cargo Monorepo Structure

```text
soos/
├── Cargo.toml                    # workspace resolver="2"
├── Cargo.lock                    # audited and committed
├── rust-toolchain.toml
├── deny.toml                     # cargo-deny rules (licenses, bans, advisories)
├── crates/
│   ├── protocol/                 # bounded schemas, codec v1, fuzz tests
│   ├── policy/                   # authorization, rate-limiting, pure logic
│   ├── pam/                      # cdylib pam_soos.so, synchronous std-only IPC
│   ├── daemon/                   # root binary, Tokio, supervisor
│   ├── camera-v4l/               # V4L2 MMAP capture, mock-camera feature
│   ├── vision/                   # preprocessing, alignment, cosine similarity
│   ├── inference-ort/            # isolated ONNX Runtime CPU bindings
│   ├── biometric-store/          # encrypted embeddings at rest
│   ├── evidence-store/           # intrusion evidence storage
│   ├── enrollment-cli/           # root enrollment CLI
│   └── admin-cli/                # non-biometric status diagnostic CLI
├── models/
│   ├── manifest.toml             # model IDs, licenses, SHA-256 checksums
│   └── README.md
├── packaging/
├── tests/
│   ├── invariants/               # architectural security invariants
│   ├── integration-pam/          # ephemeral Docker pamtester harness
│   └── fixtures/
└── AI/                           # AI development guidelines and walkthroughs
```

`#![forbid(unsafe_code)]` is strictly enforced in all business crates (`protocol`, `policy`, `vision`).

---

## 9. Privacy, Persistence, and Anti-Intrusion

- **Biometric Templates**: Stored encrypted at `/var/lib/soos/biometrics/<uid>.cbor.enc` (mode `0600`, owned by `root:root`). Raw enrollment frames are securely deleted immediately after vector extraction.
- **Evidence Snapshots**: Stored encrypted under `/var/lib/soos/evidence/YYYY-MM-DD/<uuid>.webp.enc` (mode `0600`, owned by `root:root`) with automatic 7-day retention rotation. Strictly opt-in.

---

## 10. Daemon Hardening & Operational Security

- **Systemd Sandboxing**: `NoNewPrivileges=yes`, `ProtectHome=yes`, `ProtectSystem=strict`, `RestrictAddressFamilies=AF_UNIX`, `MemoryDenyWriteExecute=yes`, `DevicePolicy=closed`.
- **Memory Scrubbing**: Sensitive keys, nonces, and buffers implement `zeroize::Zeroize` and `ZeroizeOnDrop`.[^zeroize]
- **Fail-Closed Principle**: Any runtime failure in `soos-daemon` or `pam_soos.so` falls back silently to password authentication.

---

## 11. Acceptance Criteria & Implementation Phases

1. **Foundations**: Workspace, protocol v1, skeleton daemon, fail-closed PAM returning `PAM_IGNORE`, invariant tests.
2. **Hardened IPC**: Systemd socket, `SO_PEERCRED`, bounded codec, timeouts, fuzzing.
3. **Camera Pipeline**: Warm V4L2 MMAP streaming, latest frame atomic buffer, hotplug resilience.
4. **Vision Engine**: Verified model manifests, pre-processing golden tests, calibrated cosine threshold.
5. **PAM Validation**: Dockerized `pamtester` validation across failure/success matrix.
6. **Evidence & Storage**: Encryption at rest, retention rotation, atomic writes.
7. **PAD & Production**: Presentation attack testing, external security review.

---

## References

[^pam-bindings]: `pam-bindings`, [pam crate 0.3.0 documentation](https://docs.rs/pam-bindings/latest/pam/).
[^pam-conf]: Linux-PAM, [pam.conf(5) manual](https://man7.org/linux/man-pages/man5/pam.conf.5.html).
[^pam-exec]: Linux-PAM, [pam_exec(8) manual](https://www.man7.org/linux/man-pages/man8/pam_exec.8.html).
[^unix7]: Linux man-pages, [unix(7) — SO_PEERCRED](https://man7.org/linux/man-pages/man7/unix.7.html).
[^nix]: `nix`, [PeerCredentials socket API](https://docs.rs/nix/latest/nix/sys/socket/).
[^tokio]: Tokio, [UnixListener documentation](https://docs.rs/tokio/latest/tokio/net/struct.UnixListener.html).
[^tokio-stream]: Tokio, [UnixStream documentation](https://docs.rs/tokio/latest/tokio/net/struct.UnixStream.html).
[^v4l]: `v4l`, [v4l 0.14.0 documentation](https://docs.rs/v4l/latest/v4l/).
[^nokhwa]: `nokhwa`, [nokhwa 0.10.11 documentation](https://docs.rs/crate/nokhwa/latest/source/README.md).
[^nokhwa-v4l]: `nokhwa`, [V4LCaptureDevice documentation](https://docs.rs/nokhwa/latest/nokhwa/backends/capture/struct.V4LCaptureDevice.html).
[^ort]: `ort`, [ort 2.0.0-rc.13 documentation](https://docs.rs/ort/latest/ort/).
[^zeroize]: RustCrypto, [zeroize 1.9.0 documentation](https://docs.rs/zeroize/latest/zeroize/).
[^ultraface]: Linzaer, [UltraFace Slim 320 ONNX](https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB).
[^mobilefacenet]: Chen et al., [*MobileFaceNets: Efficient CNNs for Accurate Real-Time Face Verification on Mobile Devices*](https://arxiv.org/abs/1804.07573), 2018.
[^nist-63b]: NIST, [SP 800-63B Digital Identity Guidelines](https://pages.nist.gov/800-63-4/sp800-63b.html).
[^nist-blog]: NIST, [Facing the Facts to Keep Our Biometrics Secure](https://www.nist.gov/blogs/taking-measure/facing-facts-keep-our-biometrics-secure), 2024.
[^nist-pad]: NIST, [IR 8491 — Face Analysis Technology Evaluation, Part 10](https://nvlpubs.nist.gov/nistpubs/ir/2023/NIST.IR.8491.pdf), 2023.
