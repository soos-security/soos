# `soos-enrollment-cli` — Root Enrollment and Diagnostics Tool

`soos-enrollment-cli` (`soos-enroll`) is the administrative command-line interface for the `soos` local biometric PAM subsystem. It provides privileged user enrollment, anti-forensic secure template erasure, diagnostic verification, and template inventory enumeration.

---

## 1. Architectural Role & Security Invariants

In accordance with `AI/ARCHITECTURE.md` (§8 Monorepo Structure, §9 Privacy, Persistence, and Anti-Intrusion):
- **Root-Only Modification**: State-modifying operations (`enroll`, `delete`) mandate root execution (effective UID 0), preventing unauthorized tampering with biometric credentials.
- **Single-Face Security Invariant**: The tool strictly enforces that exactly one human face is visible in any frame evaluated for enrollment or verification. Multi-face or zero-face candidate frames are rejected.
- **Encrypted Persistence**: All biometric templates are serialized to canonical CBOR and encrypted using authenticated AES-256-GCM via `soos-biometric-store` under `/var/lib/soos/biometrics/<uid>.cbor.enc` with POSIX permissions `0600` owned by `root:root`.
- **Anti-Forensic Secure Erasure**: When deleting an enrolled user template, file blocks on disk are overwritten with cryptographically secure random bytes from the kernel CSPRNG, followed by a zeroization pass and `fsync()`, prior to filesystem unlink.
- **Zeroization**: Sensitive biometric vector buffers (`Vec<f32>`) implement `Zeroize` via `zeroize::Zeroizing` and are automatically scrubbed from memory when dropped.

---

## 2. Command Reference

### `soos-enroll enroll`
Enrolls a user with multi-frame quality selection and interactive confirmation:

```bash
# Interactive enrollment of the user who invoked sudo (SUDO_UID)
sudo soos-enroll enroll

# Interactive enrollment of a specified UID
sudo soos-enroll enroll --uid 1000 --frames 5

# Enrolling root is only possible explicitly
sudo soos-enroll enroll --uid 0

# Enrollment by username with automated non-interactive confirmation
sudo soos-enroll enroll --username alice --yes
```

**Options**:
- `-i, --uid <UID>`: Target Linux User ID. Without `--uid` and `--username`, the target is the invoking user: the real UID when it is not root, otherwise `SUDO_UID` (then `PKEXEC_UID`) when it names a non-root user. Root is never an implicit target: with no such variable the command fails before any capture and asks for `--username`/`--uid`; `--uid 0` (or `--username root`) enrolls root explicitly. A malformed `SUDO_UID`/`PKEXEC_UID` fails closed. The resolved target UID is printed before capture (GitHub #184 / STO-11).
- `-u, --username <NAME>`: Target username (resolved via system user database).
- `--frames <N>`: Number of candidate frames to capture and evaluate (default: 5, bounded 1–30).
- `-y, --yes`: Automatically confirm enrollment without interactive confirmation prompt.
- `--model-id <ID>`: Model identifier stored in template metadata (default: `mobilefacenet`).
- `--model-version <VER>`: Model version stored in template metadata (default: `1.0.0`).

### `soos-enroll verify`
Performs a one-shot diagnostic verification against an enrolled biometric template:

```bash
sudo soos-enroll verify --uid 1000
```

**Report Details**:
- Match verdict: `Allow` (similarity >= threshold) or `Deny`.
- Match score: Cosine similarity value `[-1.0, 1.0]`.
- Face count and detection confidence.
- Presentation Attack Detection (PAD) status.
- Latency breakdown: camera frame capture, neural vision pipeline, cosine matching, total roundtrip.

### `soos-enroll delete`
Deletes an enrolled biometric template with anti-forensic secure erasure:

```bash
sudo soos-enroll delete --uid 1000
sudo soos-enroll delete --username alice --yes
```

### `soos-enroll list`
Lists all enrolled biometric templates with identity and model metadata:

```bash
# Human-readable table output
sudo soos-enroll list

# Machine-readable JSON output
sudo soos-enroll list --format json
```

### `soos-enroll debug-vision`
Captures one camera frame, runs the face detector and writes a standalone HTML report drawing the bounding boxes, confidence scores and 5-point landmarks on an HTML5 canvas (GitHub #149 / STO-02 hardening):

```bash
# Geometry-only report in the root-only default directory (/var/lib/soos/debug/soos-debug-<unix_secs>-<pid>.html)
sudo soos-enroll debug-vision

# Explicit output path (absolute, no '..', under an allowed FHS prefix; the file must not exist)
sudo soos-enroll debug-vision --output /var/lib/soos/debug/lighting-check.html

# Opt in to embedding the raw camera frame (biometric data) in the report
sudo soos-enroll debug-vision --embed-frame

# Mock camera and synthetic detector, no hardware required
sudo soos-enroll --mock debug-vision
```

Filesystem and privacy contract (`crates/enrollment-cli/src/service.rs`):
- **Atomic root-only creation**: the report is opened with `O_CREAT | O_EXCL | O_NOFOLLOW` and mode `DEBUG_REPORT_FILE_MODE` (`0600`) in a single `open(2)` call; the mode is never fixed up with `chmod` after the write.
- **Never overwrite, never follow**: a pre-existing file or symbolic link at the output path, or a symbolic link in place of the parent directory, aborts the command with `EnrollmentCliError::DebugReportRefused` and leaves the existing target byte-identical. This closes the root truncate-through-symlink primitive of the former `std::fs::write` into the current working directory.
- **Safe default location**: without `--output`, reports go to `DEFAULT_DEBUG_REPORT_DIR` (`/var/lib/soos/debug`), created with `DEBUG_REPORT_DIR_MODE` (`0700`) when missing; an existing directory is accepted as is and never `chmod`-ed. An explicit `--output` passes `validate_fhs_path` (absolute, no `..`, allowed FHS prefix) and is validated before the camera is touched.
- **Frame embedding is opt-in**: by default the report contains only detection geometry and resolution. `--embed-frame` embeds the raw RGB24 frame as base64; the report then carries a visible privacy warning and the CLI prints a reminder to delete it after use. The raw frame is biometric data (`AI/ARCHITECTURE.md` lists storing raw frames on disk as an anti-pattern), so embedding is a deliberate, per-invocation administrator decision.
- **Detector errors are propagated**: a failing detector aborts the command (`EnrollmentCliError::Inference`) instead of silently producing a report with zero detections, and no file is created.

Verification: `crates/enrollment-cli/tests/debug_vision_tests.rs` (matrix rows EN12–EN14).

---

### Guided multi-angle enrollment (`GuidedEnrollmentSession`, used by `soos-gui`)

`soos_enrollment_cli::guided_enrollment` collects samples over four steps and fuses them (GitHub #183 / STO-10). A sample is recorded only when:

| Check | Rule | Feedback on failure |
|---|---|---|
| Liveness / centering | PAD live and face centered | `SpoofDetected` / `PromptCenterFace` |
| Pose finiteness | yaw, pitch, roll finite | `PromptHoldStill` |
| Roll (all steps) | `abs(roll) <= 10` degrees | `PromptHoldStill` |
| Frontal | `abs(yaw) <= 8`, `abs(pitch) <= 10` | `PromptCenterFace` / `PromptHoldStill` |
| Turn left | yaw in `[-25, -10]`, `abs(pitch) <= 15` | `PromptTurnLeft` (not enough) / `PoseOutOfRange` (too far) |
| Turn right | yaw in `[10, 25]`, `abs(pitch) <= 15` | `PromptTurnRight` / `PoseOutOfRange` |
| Tilt up | pitch in `[-25, -8]`, `abs(yaw) <= 15` | `PromptTiltUp` / `PoseOutOfRange` |
| Embedding validity | non-empty, at most `MAX_GUIDED_EMBEDDING_DIM` (2048) values, same dimension as the first sample, all values finite, L2 norm > 1e-6 | `InvalidEmbedding` |
| Identity consistency | cosine >= `MIN_SAMPLE_CONSISTENCY_COSINE` (0.5) with every accepted frontal sample and, for off-axis steps, with the frontal mean direction | `IdentityMismatch` |

Each step records at most `MAX_SAMPLES_PER_STEP` (20) samples (`GuidedEnrollmentSession::new` clamps the target to `1..=20`). The composite template is the mean direction of the L2-normalized samples, `t = m / ||m||` with `m = sum_i s_i / ||s_i||`; `compute_composite_embedding` re-validates every sample (dimension, finiteness, consistency with the frontal anchor) and fails closed instead of returning a non-finite or contaminated vector.

---

## 3. Global Options

- `--biometrics-dir <PATH>`: Override biometric template storage directory (default: `/var/lib/soos/biometrics`).
- `--key-file <PATH>`: Override cryptographic master key path (default: `/var/lib/soos/master.key`).
- `--models-dir <PATH>`: Override ONNX neural models directory (default: `/var/lib/soos/models`).
- `--camera-device <PATH>`: Override V4L2 camera device node. Without it (or with `auto`/`default`), the device is resolved exactly like `soos-daemon` through the shared `soos_camera_v4l::resolve_camera_device`: `[pipeline] camera_device` from `/etc/soos/daemon.toml` if explicit, otherwise the capture node matching `[pipeline] sensor_preference` (default IR first), reported through its stable `/dev/v4l/by-id/` alias (GitHub #152).

---

## 4. Lazy Initialization & Model Attestation

- **Lazy Service Construction**: The CLI separates store-only initialization (`build_store_only`) from full biometric pipeline initialization (`build_full_service`). Non-biometric operations (`list`, `delete`) initialize solely the cryptographic `BiometricStore`, avoiding camera device allocation and neural model loading. This allows headless or unprovisioned machines to inspect and clean up templates without camera or model files.
- **Model Registry Attestation**: Biometric capture operations (`enroll`, `verify`) attest against official neural models defined in `models/manifest.toml`:
  - Face Detection & 5-Point Landmarks: `scrfd_500m_kps` (`scrfd_500m_kps.onnx`)
  - Presentation Attack Detection: `minifasnet_v2_pad` (`minifasnet_v2_80x80.onnx`), constructed by `service::build_pad_detector` with `OrtPadDetector::new`, i.e. the crate default live class index `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX = 1` shared with `soos-daemon` and `soos-gui` (never overridden; matrix PLC2, GitHub #146)
  - Feature Embedding (512D): `arcface_w600k_mbf` (`arcface_w600k_mbf.onnx`)
- **Deterministic Camera Addressing**: Satisfies Criterion C4 by resolving camera device paths via `/dev/v4l/by-id/`, eliminating enumeration races across kernel restarts.

