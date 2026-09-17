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
# Interactive enrollment for current root user or specified UID
sudo soos-enroll enroll --uid 1000 --frames 5

# Enrollment by username with automated non-interactive confirmation
sudo soos-enroll enroll --username alice --yes
```

**Options**:
- `-i, --uid <UID>`: Target Linux User ID. Defaults to caller UID if unspecified.
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

---

## 3. Global Options

- `--biometrics-dir <PATH>`: Override biometric template storage directory (default: `/var/lib/soos/biometrics`).
- `--key-file <PATH>`: Override cryptographic master key path (default: `/var/lib/soos/master.key`).
- `--models-dir <PATH>`: Override ONNX neural models directory (default: `/var/lib/soos/models`).
- `--camera-device <PATH>`: Override V4L2 camera device node (default: `/dev/v4l/by-id/default-camera` or first detected entry in `/dev/v4l/by-id/`).

---

## 4. Lazy Initialization & Model Attestation

- **Lazy Service Construction**: The CLI separates store-only initialization (`build_store_only`) from full biometric pipeline initialization (`build_full_service`). Non-biometric operations (`list`, `delete`) initialize solely the cryptographic `BiometricStore`, avoiding camera device allocation and neural model loading. This allows headless or unprovisioned machines to inspect and clean up templates without camera or model files.
- **Model Registry Attestation**: Biometric capture operations (`enroll`, `verify`) attest against official neural models defined in `models/manifest.toml`:
  - Face Detection: `ultraface_slim_320` (`version-slim-320.onnx`)
  - 5-Point Landmarks: `landmark_5point` (`landmark_5point.onnx`)
  - Presentation Attack Detection: `minifasnet_pad` (`minifasnet_pad.onnx`)
  - Feature Embedding: `mobilefacenet_arcface` (`mobilefacenet_arcface.onnx`)
- **Deterministic Camera Addressing**: Satisfies Criterion C4 by resolving camera device paths via `/dev/v4l/by-id/`, eliminating enumeration races across kernel restarts.

