# Neural Models Attestation & Acquisition Guide — soos

This directory contains the cryptographic manifest (`manifest.toml`) and technical documentation for the machine learning models utilized by the `soos` local facial verification PAM daemon and enrollment tools.

In accordance with `AI/ARCHITECTURE.md` §7 and `AI/DECISIONS.md`, `soos` executes all neural inference locally on the host CPU using ONNX Runtime (`ort`). No biometric frames, embeddings, or parameters are ever transmitted off the machine.

---

## 1. Next-Generation Attested Models Overview (Manifest v2.0.0)

Under the modernized 3-model vision architecture (ADR [2026-09-20]), the pipeline unifies face detection and landmark regression into a single forward pass, upgrades biometric embeddings to 512D, and deploys high-accuracy presentation attack detection.

> **Model ids are stable identifiers, not descriptions.** `arcface_w600k_mbf` is a historical name:
> the attested file is a Keras ArcFace ResNet34 exported with tf2onnx, **not** an InsightFace
> w600k MobileFaceNet (ADR 2026-09-30, GitHub #191). The id and file name are kept because
> `soos-daemon` and `soos-enroll` reference them.

Shapes below are the ONNX graph metadata of the attested files (`N` is a symbolic batch dim):

| Model Identifier | File Name | Architecture | License | Input Tensor | Output Tensor(s) | Primary Purpose |
|---|---|---|---|---|---|---|
| `scrfd_500m_kps` | `scrfd_500m_kps.onnx` | SCRFD 500M KPS | MIT | `[1, 3, 640, 640]` BGR | 9 tensors (scores, bboxes, kps across strides 8, 16, 32) | Unified face bounding box detection + 5-point facial landmark regression |
| `arcface_w600k_mbf` | `arcface_w600k_mbf.onnx` | ArcFace ResNet34 (Keras, tf2onnx export; ~34.1 M params, 136.6 MB) | NOASSERTION (the source declares no licence, see §6) | `input_1` `[N, 112, 112, 3]` **NHWC** (manifest: logical `[1, 3, 112, 112]` + `input_layout = "NHWC"`), fed BGR | `embedding` `[N, 512]` | 512D biometric feature extractor (L2-normalized by the extractor) |
| `minifasnet_v2_pad` | `minifasnet_v2_80x80.onnx` | MiniFASNetV2 | Apache-2.0 | `[1, 3, 80, 80]` BGR | `[1, 3]` | Presentation Attack Detection (anti-spoofing; live vs print vs replay) |

### Preprocessing & Tensor Normalization Rules

1. **`scrfd_500m_kps`**:
   - **Resolution & Aspect Ratio**: 640×640 with letterbox padding (black border preservation of aspect ratio).
   - **Color Format**: BGR channel ordering.
   - **Normalization**: `(pixel - 127.5) / 128.0` mapping `[0, 255]` to `[-0.996, +1.0]`.
   - **Output Parsing**: 9 tensors parsed across 3 strides (stride 8: 12,800 anchors; stride 16: 3,200 anchors; stride 32: 800 anchors). Distance-to-border box decoding and grid-offset keypoint regression.

2. **`arcface_w600k_mbf`** (ArcFace ResNet34, tf2onnx):
   - **Resolution & Crop**: 112×112 tightly aligned face crop via similarity transformation based on 5-point landmarks.
   - **Tensor Layout**: **NHWC** — graph input `input_1` is `[N, 112, 112, 3]` (channels last). `OrtEmbeddingExtractor` detects the layout from the session; `ModelRegistry` rejects the session if it disagrees with the manifest `input_layout`.
   - **Color Format**: fed in B, G, R channel order. The order the network was trained with is **not verified** (the BGR choice of walkthrough 71 assumed an InsightFace model); tracked as a follow-up in ADR 2026-09-30.
   - **Normalization**: symmetric `(pixel - 127.5) / 127.5` mapping `[0, 255]` to `[-1.0, +1.0]` (also not verified against the upstream training pipeline).
   - **Output**: graph output `embedding` `[N, 512]`, raw; the extractor L2-normalizes it.
   - **Cost**: ~34.1 M float32 parameters; one embedding measured at p50 127.5 ms / p95 170.9 ms on one ORT intra-op thread (`embedding_real_model_tests`).

3. **`minifasnet_v2_pad`**:
   - **Resolution & Crop**: 80×80 context crop generated from a 2.7× expanded face bounding box (captures facial margins, bezels, and printed paper boundaries).
   - **Color Format**: BGR channel ordering.
   - **Value range**: raw pixel values as floats in `[0, 255]`, **not** divided by 255, exactly like
     upstream Silent-Face-Anti-Spoofing (`to_tensor` returns `img.float()`; both MiniFASNet checkpoints
     were trained on that range; ADR 2026-10-01 "PAD Input Range Matches Upstream (0-255)", walkthrough 161). Until 2026-10-01 soos fed
     `pixel / 255.0`, on which every input tested scored replay (`p_live < 0.012`).
   - **Class Ordering**: Softmax logits with Class 0 = Print Spoof, Class 1 = Live, Class 2 = Replay Spoof (`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX = 1` in `crates/inference-ort/src/pad.rs`, ADR 2026-09-29; never overridden in production).

---

## 2. Cryptographic Attestation & Invariant 4

To protect against model tampering, unauthorized substitution, and supply chain poisoning, `soos-daemon` and `soos-inference-ort` enforce **strict cryptographic attestation**:
- Every model is attested in `manifest.toml` with its exact SHA-256 digest.
- During daemon startup, `ModelRegistry::verify_integrity()` verifies the SHA-256 digest of each model file on disk against `manifest.toml`.
- If any model file is missing, modified, corrupted, or tampered with, `soos-daemon` immediately refuses to start (fail-closed), preventing unverified or compromised weights from handling PAM authentication.
- After each ONNX Runtime session is built, `ModelRegistry::get_or_load_session` compares its input and output tensor shapes with `input_shape` / `input_layout` / `output_shapes` (symbolic graph dims such as `batch_size` or `unk__556` are wildcards; rank and every concrete dim must match). A mismatch fails closed with `InferenceError::ModelShapeMismatch` (GitHub #191), so a model with the right hash but unexpected I/O is never handed to a detector.
- `input_shape` is always the logical `[N, C, H, W]` shape; `input_layout` (default `"NCHW"`) is the physical layout of the graph input. An entry without `input_layout` (a manifest installed by an earlier release) has its layout *unspecified*: the input may be the logical shape in NCHW or NHWC order, rank and every dim are still enforced, the SHA-256 already binds the exact file, and `ModelRegistry` logs a one-time warning that the manifest predates layout attestation. An explicit `input_layout` is always enforced and a wrong value fails closed. The committed manifest declares `input_layout = "NHWC"` for the embedding model.

### Expected SHA-256 Checksums (v2.0.0)

```text
scrfd_500m_kps:     a3562ef62592bf387f6ef19151282ac127518e51c77696e62e0661bee95ba1ad
arcface_w600k_mbf:  ffe014a45c9488506719d37fd578ece6661bb385535b36e8039975fa5d4683db
minifasnet_v2_pad:  0cbe5caec95c31de9d2ef845cb85407d76aecd1b6a2c0e343f7d35306bfbccb8
```

### Optional, Disabled Models (`models/optional_models.toml`)

`models/optional_models.toml` uses the same schema but is read by no runtime crate. It attests the
second upstream PAD model, which stays **disabled** until the fused PAD threshold is calibrated on a
print / screen-replay corpus (GitHub #212, walkthrough 161):

```text
minifasnet_v1se_pad: a25886a85cdcfa2c4ea23edb71de35f250c17827b4cadd253a972b28c80fdf1e  (1,743,294 bytes)
```

- `./scripts/download_models.sh --with-optional` fetches and verifies it next to the attested models;
  the deployed `manifest.toml` is still the main manifest, so nothing is enabled.
- Enabling (operator decision): append its `[models.minifasnet_v1se_pad]` table (not the `[manifest]`
  header) to `/var/lib/soos/models/manifest.toml` and restart `soos-daemon`, which then fuses it at
  the 4.0x scale (`attach_optional_pad_members`) or refuses to start if the file does not match.

---

## 3. Automated Model Acquisition & Deployment

The deployment script `scripts/download_models.sh` automates downloading, cryptographic checksum verification, and installation into the production system directory:

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

1. **SCRFD 500M KPS (`scrfd_500m_kps`)**:
   - Upstream: InsightFace / [ykk648/face_lib](https://huggingface.co/ykk648/face_lib)
   - Architecture: Sample and Computation Redistribution for Efficient Face Detection (SCRFD) with 5-point keypoint regression
   - License: MIT

2. **ArcFace ResNet34 (`arcface_w600k_mbf`, historical id)**:
   - Upstream: [garavv/arcface-onnx](https://huggingface.co/garavv/arcface-onnx) (`arc.onnx`)
   - Architecture: ArcFace-trained ResNet34 (Keras), converted with `tf2onnx` 1.16.1 (opset 15); node names `StatefulPartitionedCall/ResNet34/...`; 164 initializers, 34,138,432 float32 parameters; 136,619,444 bytes; 512D output
   - Not an InsightFace `w600k_mbf` (MobileFaceNet, WebFace600K); the training dataset of this network is not documented upstream
   - License: MIT (as recorded in the manifest; not re-verified in GitHub #191)

3. **MiniFASNetV2 (`minifasnet_v2_pad`)** and **MiniFASNetV1SE (`minifasnet_v1se_pad`, optional, disabled)**:
   - Upstream weights: [minivision-ai/Silent-Face-Anti-Spoofing](https://github.com/minivision-ai/Silent-Face-Anti-Spoofing)
     (Apache-2.0) at commit `b6d5f04ad78778917853b25c778acef6d5626d15`:
     `resources/anti_spoof_models/2.7_80x80_MiniFASNetV2.pth`
     (`a5eb02e1843f19b5386b953cc4c9f011c3f985d0ee2bb9819eea9a142099bec0`) and
     `4_0_0_80x80_MiniFASNetV1SE.pth` (`84ee1d37d96894d5e82de5a57df044ef80a58be2b218b5ed7cdfd875ec2f5990`).
   - ONNX files: [QingHeYang/Silent-Face-Anti-Spoofing-onnx](https://github.com/QingHeYang/Silent-Face-Anti-Spoofing-onnx)
     (Apache-2.0) at commit `584d4421d7ac42c59e640796f46e886b0095367a`, `onnx/2.7_80x80_MiniFASNetV2.onnx`
     (the shipped file, same SHA-256) and `onnx/4_0_0_80x80_MiniFASNetV1SE.onnx`.
   - Equivalence: `scripts/convert_pad_models.py` re-exports both checkpoints with the upstream
     `src/model_lib/MiniFASNet.py` (pinned torch / onnx versions); `scripts/compare_pad_models.py` shows
     every initializer bit-identical to the fork files and a softmax difference of exactly 0 on 156
     inputs per convention (walkthrough 161). The fork ONNX files are therefore the upstream models.
   - Architecture: MiniFASNet 80×80, input `input` `[N, 3, 80, 80]` NCHW BGR, output `output` `[N, 3]`
     logits; classes `[print, live, replay]` (upstream `test.py`: `label == 1` is a real face).

---

## 5. Model Migration & Legacy Pipeline Lineage

In earlier revisions (Manifest v1.0.0), `soos` utilized a 4-model pipeline requiring two separate sequential passes for detection and landmark localization:
- `ultraface_slim_320` (`version-slim-320.onnx`, MIT, `[1, 3, 240, 320]`)
- `landmark_5point` (`landmark_5point.onnx`, MIT, `[1, 3, 112, 112]`)
- `mobilefacenet_arcface` (`mobilefacenet_arcface.onnx`, MIT, `[1, 3, 112, 112]`, 128D)
- `minifasnet_pad` (`minifasnet_pad.onnx`, Apache-2.0, `[1, 3, 112, 112]`, 3-class)

Legacy checksums (v1.0.0):
```text
ultraface_slim_320:     b7a44f4340d0fb9b071e6be94deaa9bcfceeb9f1c7dcf14b1b365bca7f79ff0b
landmark_5point:        94f0e08c843f51a43a0e1b6fef65cbe5b1dbdc3e4bc3929424687771746a5be4
mobilefacenet_arcface:  66fbe536c4eb827a5e828d11c8cb5f98bb2d7ebec44ec2f35952fdfa3ceee473
minifasnet_pad:         65b8e9076c8c4a4a6873523f858203cba21efae9d13e314ad4b87e2dbf77c867
```

Manifest v2.0.0 completely supersedes the v1.0.0 models. These ids are historical only: `scripts/download_models.sh` no longer resolves any download URL for them (GitHub #249), and no production code loads them. The unified `scrfd_500m_kps` model eliminates the separate `landmark_5point` inference stage, reducing total verification latency by ~20ms.

---

## 6. Legal Notice & Redistribution Restrictions

- **Open Source Licensing**: The model architectures and pre-trained weights referenced in `manifest.toml` are authored by their respective upstream creators and licensed under permissive open-source licenses (MIT and Apache License 2.0), except `arcface_w600k_mbf`: its source repository (`garavv/arcface-onnx`) declares no licence, so the manifest records the SPDX value `NOASSERTION` (GitHub #278). Packagers must clear its redistribution terms with the upstream author or replace the model.
- **Redistribution Policy**: In strict compliance with zero-trust principles and source repository hygiene, compiled binary weights (`*.onnx`) are **NOT** bundled or tracked in git version control. They are downloaded directly from authenticated upstream sources or local installation packages during setup.
- **Third-Party Rights**: Users and distribution packagers must comply with the upstream license agreements when acquiring, caching, or distributing model weights for end-user deployments.
