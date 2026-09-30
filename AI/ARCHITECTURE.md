# soos — Master Architecture and Context Document

## 1. Architectural Decisions

### Phase 1 Objective

**soos** adds local facial verification to Linux PAM stacks without modifying the user interface of session managers, lock screens, or `sudo`. A successful facial match authorizes authentication; any unavailability, uncertainty, or anomaly **falls back cleanly to the existing password prompt**. Following a password authentication failure, an anti-intrusion event can notify the daemon to preserve cryptographic evidence under an explicit privacy policy.

The system is not considered a high-assurance biometric factor until robust Presentation Attack Detection (PAD/liveness) and false-accept evaluations are integrated. Photos, digital screens, and 3D masks represent known presentation attacks documented by NIST; NIST SP 800-63B guidelines mandate PAD and measured false-match rates for facial verification.[^nist-63b] Phase 1 is strictly positioned as **convenience + local threat detection** with mandatory password fallback, not as a universal replacement for hardware security keys or master secrets.

### Key Architectural Choices

| Domain | Canonical Choice | Rationale | Prohibited Anti-Patterns |
|---|---|---|---|
| PAM/Daemon Boundary | Local Unix Domain Socket (UDS), `SOCK_SEQPACKET` if available, otherwise framed `SOCK_STREAM` | Zero network exposure, ultra-low latency, kernel-enforced peer credentials (`SO_PEERCRED`) | HTTP, loopback TCP, world-writable socket without peer verification |
| PAM Module Execution | Pure synchronous blocking Rust, `std::os::unix::net::UnixStream`; every blocking PAM operation has an explicit deadline derived from the clamped `timeout_ms` (default 1000 ms, range 10–5000 ms) | Critical authentication path must never spawn a persistent async runtime | AI inference, direct camera access, or network downloads inside `.so` |
| Privileged Daemon | Rust + Tokio root process, sole owner of `/dev/video*` and ONNX sessions | Keeps camera warm and models in memory; central arbitration | Re-opening `/dev/video0` inside PAM on every authentication attempt |
| Linux Camera Capture | `v4l` 0.14, MMAP buffers on dedicated worker thread; `nokhwa` only as prototype | Deterministic V4L2 control and predictable zero-copy buffer rotation | Depending on OpenCV or allowing competing camera consumers |
| Local AI Inference | `ort` (ONNX Runtime) CPU execution provider; **SCRFD 500M KPS** (face detection + 5-point landmarks) + **ArcFace ResNet34** (512D embeddings, manifest id `arcface_w600k_mbf`) + **MiniFASNetV2** (anti-spoofing) | Fast 3-model pipeline with unified detection+landmarks, battle-tested ORT runtime, no OpenCV required | Claiming "pure Rust" (ORT is native C/C++); unverified weight downloads; separate landmark model (absorbed into SCRFD) |
| Biometric Storage | AES-GCM encrypted embeddings at rest; intrusion snapshots opt-in and isolated | Minimizes attack surface and persistent biometric risk | Storing raw frames or passwords on disk or sending over socket |

Verified crate versions: `pam-bindings` 0.3.0 (target crate), `tokio` 1.53.1, `v4l` 0.14.0, `nokhwa` 0.10.11, `ort` 2.0.0-rc.13, `zeroize` 1.9.0. Versions are pinned in `Cargo.lock` and audited via `cargo-deny`.[^pam-bindings][^tokio][^v4l][^nokhwa][^ort][^zeroize]

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
  │ pam_soos.so — synchronous path, deadline from clamped timeout_ms, zero camera access
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

The daemon starts as a systemd service before login prompts, loads and validates model checksums, opens the camera, and stabilizes auto-exposure. The PAM module contains only the lightweight IPC client, response interpreter, and C ABI bindings. This separation guarantees that AI model loading, camera reinitialization, or video processing delays cannot block PAM authentication calls beyond the strictly enforced latency budget.

### Request State Matrix

| Daemon Internal State | PAM Module Return | System Effect |
|---|---|---|
| Single face, PAD passed, match score >= threshold, valid context | `Allow` | `PAM_SUCCESS`; PAM stack short-circuits to grant access |
| No face, multiple faces, low score, failed PAD anti-spoof | `Deny` | `PAM_IGNORE`; PAM proceeds silently to password prompt |
| Camera/model/socket offline, timeout exceeded, internal error | `Unavailable` | `PAM_IGNORE`; seamless password fallback |
| Malformed payload, mismatched UID, rate-limit reached | `ProtocolError` | `PAM_IGNORE`; daemon logs security warning |

`Deny` and `Unavailable` are intentionally indistinguishable to the PAM caller, preventing timing or enumeration attacks.

**Multi-frame consensus (GitHub #147).** The `Allow` row is never satisfied by a single capture. The daemon evaluates successive distinct camera captures within the decision budget and feeds them to the zero-I/O `soos_policy::PadAggregator`: `Allow` requires `k = 3` consecutive captures (window `n = 5`) that are live at or above the PAD threshold and match at or above the cosine threshold, and any capture classified as a spoof vetoes `Allow` for the whole request. A request whose budget expires before consensus returns `Unavailable`/`Timeout`.

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
- `peer.uid == uid` of target identity (or documented rule for root PAM caller).
- Target UID is an authorized local user and owns the active local graphical session. For a root PAM caller (`su`, `sudo`, `sshd`, display manager) the request is tied to the caller's own logind session through the `SO_PEERCRED` PID; that session must belong to the target UID, be active, local (`REMOTE=0`) and seat-attached (`CLASS=user`). Any lookup failure denies face verification (ADR 2026-09-30 "Local Session Binding", GitHub #160).
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
- **Privileged Daemon**: Runs Tokio for IPC connection dispatching. Capture runs on the dedicated `soos-v4l-capture` thread; vision inference runs on the Tokio blocking pool (`spawn_blocking`) behind the `InferenceGate` semaphore (`MAX_CONCURRENT_INFERENCES` = 1, `crates/daemon/src/inference.rs`), so Tokio workers, the accept loop and Status requests never block on inference (GitHub #158). Each authentication request computes one `RequestDeadline` (client deadline and outer `connection_timeout`, each minus the 50ms `RESPONSE_WRITE_MARGIN_MS`) and never starts an inference whose measured estimate exceeds the remaining budget (GitHub #159).
- **PAM Module**: **Strictly forbidden from starting Tokio**. Uses `std::os::unix::net::UnixStream`; every blocking PAM operation has an explicit deadline derived from the clamped `timeout_ms` (`DEFAULT_TIMEOUT_MS` = 1000, clamped to 10–5000 ms): connect, request write and verdict read share one cumulative deadline, and the `event=password-failed` notification uses its own `timeout_ms=20`. Immediately closes socket after response. Packaged console/sudo stacks rely on the module default (ADR 2026-09-30 "PAM Deadline Derived From Clamped `timeout_ms`"); GDM uses `timeout_ms=2500`.

---

## 5. PAM Module Implementation & Universal Stack

### Crate and ABI
The module compiles to a `cdylib` (`pam_soos.so`).
- **Current Skeleton Phase**: Direct C ABI exports (`pam_sm_authenticate`, `pam_sm_setcred`) wrapped in `catch_unwind`, systematically returning `PAM_IGNORE` to test PAM ABI compatibility and non-interference without external dependencies.
- **Planned Target Integration**: Adoption of `pam-bindings` 0.3.0 (`PamHandle`, `PamHooks`), syslog logging on caught panics, and IPC client integration.

```rust
// Conceptual authentication flow
match ipc_auth(uid, service, deadline) {
    Ok(Allow { request_id, .. }) => PAM_SUCCESS,
    Ok(Deny | Unavailable | ProtocolError) | Err(_) => PAM_IGNORE,
}
```

### Universal PAM Stack Ordering

```pam
# Placed AFTER mandatory faillock preauth, BEFORE pam_unix.
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250

# Standard password verification. On failure: marks stack failed but continues.
auth  [success=done default=bad]     pam_unix.so try_first_pass

# Reached only if pam_unix fails. Zero secrets accessed.
auth  optional                       pam_soos.so event=password-failed timeout_ms=20
```

### Distribution Adaptation Guidelines

| Family | Primary Auth File | Integration Strategy |
|---|---|---|
| Debian / Ubuntu | `/etc/pam.d/common-auth` | Managed via `pam-auth-update` profile; preserves `pam_unix` and `pam_faillock`. |
| RHEL / Fedora | `/etc/pam.d/system-auth` | Managed via custom `authselect` profile; avoid direct manual edits. |
| Arch Linux | `/etc/pam.d/system-auth` | Inserted into include chain; preserve `.pacnew` files during system updates. |
| openSUSE | `/etc/pam.d/common-auth` | Managed via `pam-config`; inspect resulting stack before deployment. |

**GDM (`/etc/pam.d/gdm-password`)**: managed by `soos-admin gdm enable` with
`auth  [success=done default=ignore]  pam_soos.so timeout_ms=2500` inside a marked block placed
before the first credential or shared-stack rule, after every in-file `pam_nologin`,
`pam_succeed_if`, `pam_shells` and `pam_faillock preauth` rule; the gates of a delegated stack
that run before its credential module are copied in front of it (ADR 2026-09-30 "GDM PAM Stack
Placement", `Docs/DISTRIBUTION_DEPLOYMENT.md` section 2.1).

Before deployment, always maintain an active root rescue shell, verify fallback to password in a VM, and test screensavers (`swaylock`, `hyprlock`), TTY, SSH, and `sudo`.

---

## 6. Warm Camera Streaming & Low Latency

The daemon exclusively controls the camera by stable hardware ID (`/dev/v4l/by-id/...`), using `v4l` 0.14 MMAP streaming.[^v4l]

```text
CameraManager thread (blocking): dequeue MMAP -> timestamp CLOCK_MONOTONIC
 -> convert/scale -> ArcSwap<LatestFrame> -> requeue MMAP buffer immediately
                                            |
                              PAM request reads RAM snapshot
                              (age <= 100-150ms), never touches camera
```

- Discard first 15–30 frames upon camera initialization for auto-exposure stabilization.
- Fall back to 5 FPS after 60s of inactivity; close camera only upon explicit user policy.
- Handle `ENODEV`, `EIO`, `EBUSY` with bounded exponential backoff and seamless `Unavailable` response; never hang the IPC listener.

---

## 7. Local Vision Pipeline & Latency Budget (150ms Target)

### Models & Verification Pipeline

The soos vision pipeline uses a **3-model architecture** (manifest version 2.0.0), with SCRFD unifying face detection and landmark regression into a single model:

1. **Face Detection + Landmarks**: SCRFD 500M KPS ONNX (~2.4 MB, MIT) → bounding boxes with confidence scores AND 5-point facial keypoints directly, via multi-stride (8/16/32) distance-to-border box decoding. BGR 640×640 input with letterbox padding and `(pixel - 127.5) / 128.0` normalization. Eliminates the separate landmark model of the legacy pipeline.
2. **Presentation Attack Detection (PAD)**: MiniFASNetV2 ONNX (~1.8 MB, Apache-2.0) → `[PrintPhoto, Live, ScreenReplay]` 3-class liveness scores. Receives an **80×80 BGR** crop of the 2.7× expanded bounding box (wider context than the aligned face), normalized with `pixel / 255.0`. Live class index 1 = `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` (single source of truth in `crates/inference-ort/src/pad.rs`; never overridden in production, see ADR 2026-09-29 and GitHub #146). The model is RGB-trained: `PixelFormat::Grey` (IR) frames never take the colour path; they pass a fail-closed IR exposure/contrast/texture gate and a stricter, uncalibrated liveness threshold (`DEFAULT_IR_PAD_THRESHOLD = 0.95`, never looser than `pad_threshold`), see ADR 2026-09-30 and GitHub #169.
3. **Feature Extraction**: ArcFace **ResNet34** ONNX (Keras model exported with tf2onnx, ~34.1 M parameters, 136.6 MB, licence recorded as MIT; manifest id `arcface_w600k_mbf` is a historical name — it is **not** an InsightFace w600k MobileFaceNet, see ADR 2026-09-30 / GitHub #191) → **512D** embedding vector, L2-normalized by the extractor. Graph input `input_1` is **NHWC** `[N, 112, 112, 3]` (manifest `input_layout = "NHWC"`); it receives the standard **112×112** aligned face crop produced by affine alignment from the 5-point landmarks, fed in B, G, R order with `(pixel - 127.5) / 127.5` normalization (symmetric `[-1.0, +1.0]`). The channel order and normalization the network was trained with are not verified (follow-up).
4. **Matching**: Cosine similarity (`cosine = dot(a, b)` for L2-normalized vectors). Authorized only if score ≥ calibrated threshold and a single face is verified with PAD passed.

Every model file is tracked in `models/manifest.toml` v2.0.0 with license, source URL, SHA-256 checksum, and tensor shape specifications. `ModelRegistry` enforces both: the SHA-256 before the ONNX Runtime session is built, and the declared input/output shapes (`input_shape`, `input_layout`, `output_shapes`; symbolic graph dims are wildcards) before the session is used.

### 150ms Latency Budget (p95 Target)

| Segment | Budget (p95) |
|---|---:|
| IPC dispatch and RAM snapshot | 5 ms |
| Color conversion & SCRFD face detection + landmarks (640×640 BGR, letterbox) | 40 ms |
| 2.7× bbox expansion + crop-resize to 80×80 + MiniFASNetV2 PAD | 30 ms |
| Affine alignment (112×112) + ArcFace ResNet34 embedding (512D) | 30 ms (target, **not met**: embedding alone measured p50 127.5 ms / p95 170.9 ms, see below) |
| OS scheduler margin | 20 ms |
| Cosine matching + policy verdict | 5 ms |
| **Total Decision Budget** | **<= 150 ms** |

Measured on the attested embedding model (GitHub #191,
`embedding_real_model_tests::test_real_embedding_latency_report`, one ORT intra-op thread,
development host under concurrent build load): one 112×112 embedding takes p50 127.5 ms,
p95 170.9 ms. The 150 ms per-capture target is therefore not met by the shipped ResNet34; the
bounds actually enforced are the daemon's `DECISION_BUDGET_MS = 900` per request and its EMA
inference admission estimate (`MAX_INFERENCE_ESTIMATE_MS = 1000`). Keeping this model and the
scheduled evaluation of a lighter one are recorded in ADR 2026-09-30 (Face Embedding Model
Identity & Retention).

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

### Systemd Sandboxing

```ini
[Service]
User=root
Group=soos
ExecStart=/usr/libexec/soos/soos-daemon
Restart=on-failure
RestartSec=2
UMask=0077
RuntimeDirectory=soos
RuntimeDirectoryMode=0750
StateDirectory=soos
StateDirectoryMode=0755
NoNewPrivileges=yes
PrivateTmp=yes
ProtectHome=yes
ProtectSystem=strict
ReadWritePaths=/var/lib/soos /run/soos
DevicePolicy=closed
DeviceAllow=/dev/video* rw
RestrictAddressFamilies=AF_UNIX
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictSUIDSGID=yes
SystemCallArchitectures=native
```

### Memory Hygiene
- **Implemented**: The IPC `Response` struct implements manual `zeroize::Zeroize` and `Drop` to clear nonces and reset verdicts to `Deny` / `InternalError` upon deallocation.
- **Planned Target**: Key material, decrypted biometric vectors, and raw camera frames in daemon memory will implement zeroization wrappers (`Zeroizing<T>`) and undergo bounds checking to prevent residual copies in heap or swap.

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

## 12. Frequent Pitfalls to Avoid

- Directly modifying `/etc/pam.d/system-auth` on an `authselect`-managed distribution.
- Placing facial authentication before `pam_faillock` preauth, bypassing account lockout.
- Using `sufficient` without checking previous stack failures.
- Transmitting biometric embeddings across the IPC socket, or writing frames or embeddings to logs. Camera frames leave the daemon only through the authorized diagnostic preview stream (`RequestKind::PreviewFrame`: root peer or explicit `[preview]` opt-in, active session, rate limited — see ADR 2026-09-29 and walkthrough 81); never through PAM.
- Assuming `/dev/video0` index is static or shareable across processes.
- Downloading unverified ONNX weights at runtime without manifest hash checks.
- Promising presentation attack security without active or dedicated PAD validation.

---

## References

[^pam-bindings]: `pam-bindings`, [pam crate 0.3.0 documentation](https://docs.rs/pam-bindings/latest/pam/).
[^pam-conf]: Linux-PAM, [pam.conf(5) manual](https://man7.org/linux/man-pages/man5/pam.conf.5.html).
[^pam-exec]: Linux-PAM, [pam_exec(8) manual](https://man7.org/linux/man-pages/man8/pam_exec.8.html).
[^unix7]: Linux man-pages, [unix(7) — SO_PEERCRED](https://man7.org/linux/man-pages/man7/unix.7.html).
[^nix]: `nix`, [PeerCredentials socket API](https://docs.rs/nix/latest/nix/sys/socket/).
[^tokio]: Tokio, [UnixListener documentation](https://docs.rs/tokio/latest/tokio/net/struct.UnixListener.html).
[^tokio-stream]: Tokio, [UnixStream documentation](https://docs.rs/tokio/latest/tokio/net/struct.UnixStream.html).
[^v4l]: `v4l`, [v4l 0.14.0 documentation](https://docs.rs/v4l/latest/v4l/).
[^nokhwa]: `nokhwa`, [nokhwa 0.10.11 documentation](https://docs.rs/crate/nokhwa/latest/source/README.md).
[^nokhwa-v4l]: `nokhwa`, [V4LCaptureDevice documentation](https://docs.rs/nokhwa/latest/nokhwa/backends/capture/struct.V4LCaptureDevice.html).
[^ort]: `ort`, [ort 2.0.0-rc.13 documentation](https://docs.rs/ort/latest/ort/).
[^zeroize]: RustCrypto, [zeroize 1.9.0 documentation](https://docs.rs/zeroize/latest/zeroize/).
[^scrfd]: Guo et al., [*Sample and Computation Redistribution for Efficient Face Detection*](https://arxiv.org/abs/2105.04714), ICLR 2022. Model: SCRFD 500M KPS ONNX (RuteNL fork, MIT license).
[^arcface-w600k]: Deng et al., [*ArcFace: Additive Angular Margin Loss for Deep Face Recognition*](https://arxiv.org/abs/1801.07698), CVPR 2019. Model shipped: Keras ArcFace ResNet34 exported to ONNX with tf2onnx (`garavv/arcface-onnx`), 512D embeddings.
[^minifasnetv2]: Zhang et al., [*A Dataset and Benchmark for Large-Scale Multi-Modal Face Anti-Spoofing*](https://arxiv.org/abs/1812.00408), CVPR 2019. Model: MiniFASNetV2 ONNX fork by QingHeYang (Apache-2.0).
[^nist-63b]: NIST, [SP 800-63B Digital Identity Guidelines](https://pages.nist.gov/800-63-4/sp800-63b.html).
[^nist-blog]: NIST, [Facing the Facts to Keep Our Biometrics Secure](https://www.nist.gov/blogs/taking-measure/facing-facts-keep-our-biometrics-secure), 2024.
[^nist-pad]: NIST, [IR 8491 — Face Analysis Technology Evaluation, Part 10](https://nvlpubs.nist.gov/nistpubs/ir/2023/NIST.IR.8491.pdf), 2023.
