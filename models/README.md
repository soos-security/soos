# Neural Models Attestation & Acquisition Guide — soos

This directory contains the cryptographic manifest (`manifest.toml`) and documentation for the machine learning models utilized by the `soos` local facial verification PAM daemon and enrollment tools.

In accordance with `AI/ARCHITECTURE.md` §7, `soos` executes all neural inference locally on the host CPU using ONNX Runtime (`ort`). No biometric frames, embeddings, or parameters are ever transmitted off the machine.

---

## 1. Attested Models Overview

| Model Identifier | File Name | Architecture | License | Input Tensor | Output Tensor(s) | Primary Purpose |
|---|---|---|---|---|---|---|
| `ultraface_slim_320` | `version-slim-320.onnx` | UltraFace Slim 320 | MIT | `[1, 3, 240, 320]` | `[1, 4420, 2]`, `[1, 4420, 4]` | Lightweight face bounding box & confidence detection |
| `landmark_5point` | `landmark_5point.onnx` | InsightFace 5-Point | MIT | `[1, 3, 112, 112]` | `[1, 10]` | 5-point facial landmark regression (eyes, nose, mouth) |
| `mobilefacenet_arcface` | `mobilefacenet_arcface.onnx` | MobileFaceNet ArcFace | MIT | `[1, 3, 112, 112]` | `[1, 128]` | 128D L2-normalized biometric face feature extractor |
| `minifasnet_pad` | `minifasnet_pad.onnx` | MiniFASNet | Apache-2.0 | `[1, 3, 112, 112]` | `[1, 3]` | Presentation Attack Detection (anti-spoofing) |

---

## 2. Cryptographic Attestation & Invariant 4

To protect against model tampering, unauthorized replacement, and supply chain attacks, `soos-daemon` and `soos-inference-ort` enforce **strict cryptographic attestation**:
- Every model must be attested in `manifest.toml` with its exact SHA-256 digest.
- During daemon initialization, `ModelRegistry::verify_integrity()` verifies the SHA-256 digest of each model on disk against `manifest.toml`.
- If any model is missing, modified, corrupted, or tampered with, `soos-daemon` immediately refuses to start (fail-closed), preventing unverified or compromised models from handling PAM authentication.

### Expected SHA-256 Checksums

```text
ultraface_slim_320:     b7a44f4340d0fb9b071e6be94deaa9bcfceeb9f1c7dcf14b1b365bca7f79ff0b
landmark_5point:        94f0e08c843f51a43a0e1b6fef65cbe5b1dbdc3e4bc3929424687771746a5be4
mobilefacenet_arcface:  66fbe536c4eb827a5e828d11c8cb5f98bb2d7ebec44ec2f35952fdfa3ceee473
minifasnet_pad:         65b8e9076c8c4a4a6873523f858203cba21efae9d13e314ad4b87e2dbf77c867
```

---

## 3. Automated Model Acquisition & Deployment

The deployment script `scripts/download_models.sh` automates downloading, checksum verification, and installation into the production system directory:

```bash
# Automated deployment to /var/lib/soos/models/ (requires root)
sudo ./scripts/download_models.sh

# Or preview actions with dry-run
./scripts/download_models.sh --dry-run

# Or verify existing installed models
./scripts/download_models.sh --check-only
```

### Destination Directory & Permissions

Production models and the attestation manifest are deployed to `/var/lib/soos/models/`:
- **Directory**: `/var/lib/soos/models/` (owner `root:root`, permissions `0755`)
- **Model Files**: `*.onnx` (owner `root:root`, permissions `0644`)
- **Manifest**: `/var/lib/soos/models/manifest.toml` (owner `root:root`, permissions `0644`)

---

## 4. Upstream Sources & Lineage

1. **UltraFace Slim 320**:
   - Upstream: [Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB](https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB)
   - Author: Linzaer
   - Format: ONNX FP32, 320x240 RGB input

2. **InsightFace 5-Point Landmark Detector**:
   - Upstream: [deepinsight/insightface](https://github.com/deepinsight/insightface)
   - Author: DeepInsight (Jiankang Deng et al.)
   - Format: ONNX FP32, 112x112 RGB input

3. **MobileFaceNet ArcFace**:
   - Upstream: [sirius-ai/MobileFaceNet_TF](https://github.com/sirius-ai/MobileFaceNet_TF)
   - Authors: Sheng Chen, Yang Liu, Xiang Gao, Zhen Han
   - Format: ONNX FP32, 112x112 RGB input, 128D embedding vector

4. **MiniFASNet Presentation Attack Detection**:
   - Upstream: [minivision-ai/Silent-Face-Anti-Spoofing](https://github.com/minivision-ai/Silent-Face-Anti-Spoofing)
   - Author: Minivision AI
   - Format: ONNX FP32, 112x112 RGB input, 3-class softmax output

---

## 5. Legal Notice & Redistribution Restrictions

- **Open Source Licensing**: The model architectures and pre-trained weights referenced in `manifest.toml` are authored by their respective upstream creators and licensed under the permissive licenses indicated above (MIT and Apache License 2.0).
- **Redistribution Policy**: In strict compliance with zero-trust principles and source repository hygiene, compiled binary weights (`*.onnx`) are **NOT** bundled or tracked in git version control. They are downloaded directly from authenticated upstream sources or local installation packages during setup.
- **Third-Party Rights**: Users and distribution packagers must comply with the upstream license agreements when acquiring, caching, or distributing model weights for end-user deployments.
