# Neural Models Attestation & Acquisition Guide — soos

This directory contains the cryptographic manifest (`manifest.toml`) and technical documentation for the machine learning models utilized by the `soos` local facial verification PAM daemon and enrollment tools.

In accordance with `AI/ARCHITECTURE.md` §7 and `AI/DECISIONS.md`, `soos` executes all neural inference locally on the host CPU using ONNX Runtime (`ort`). No biometric frames, embeddings, or parameters are ever transmitted off the machine.

---

## 1. Next-Generation Attested Models Overview (Manifest v2.0.0)

Under the modernized 3-model vision architecture (ADR [2026-09-20]), the pipeline unifies face detection and landmark regression into a single forward pass, extracts 128D SFace biometric embeddings (since the owner decision of 2026-10-01, GitHub #278; 512D ArcFace before), and deploys high-accuracy presentation attack detection.

> **Model ids are stable identifiers, not descriptions.** The embedding model is OpenCV Zoo SFace
> 2021dec (`sface_2021dec`), which replaced the ArcFace ResNet34 `arcface_w600k_mbf` on 2026-10-01
> (GitHub #278, walkthrough 162). The retired entry is kept, unchanged, in
> `models/retired_models.toml` for the evaluation tests only; no binary reads it. Templates
> enrolled with the retired model are refused by `soos-daemon` (password fallback) until the
> user re-enrolls.

Shapes below are the ONNX graph metadata of the attested files (`N` is a symbolic batch dim):

| Model Identifier | File Name | Architecture | License | Input Tensor | Output Tensor(s) | Primary Purpose |
|---|---|---|---|---|---|---|
| `scrfd_500m_kps` | `scrfd_500m_kps.onnx` | SCRFD 500M KPS | MIT | `[1, 3, 640, 640]` BGR | 9 tensors (scores, bboxes, kps across strides 8, 16, 32) | Unified face bounding box detection + 5-point facial landmark regression |
| `sface_2021dec` | `sface_2021dec.onnx` | OpenCV Zoo SFace 2021dec (MobileFaceNet backbone, SFace loss; 9.7 M params, 38.7 MB) | Apache-2.0 (training data undocumented, owner-accepted, see §6) | `data` `[1, 3, 112, 112]` **NCHW**, RGB, raw 0..255 | `fc1` `[1, 128]` | 128D biometric feature extractor (L2-normalized by the extractor) |
| `minifasnet_v2_pad` | `minifasnet_v2_80x80.onnx` | MiniFASNetV2 | Apache-2.0 | `[1, 3, 80, 80]` BGR | `[1, 3]` | Presentation Attack Detection (anti-spoofing; live vs print vs replay) |

### Preprocessing & Tensor Normalization Rules

1. **`scrfd_500m_kps`**:
   - **Resolution & Aspect Ratio**: 640×640 with letterbox padding (black border preservation of aspect ratio).
   - **Color Format**: BGR channel ordering.
   - **Normalization**: `(pixel - 127.5) / 128.0` mapping `[0, 255]` to `[-0.996, +1.0]`.
   - **Output Parsing**: 9 tensors parsed across 3 strides (stride 8: 12,800 anchors; stride 16: 3,200 anchors; stride 32: 800 anchors). Distance-to-border box decoding and grid-offset keypoint regression.

2. **`sface_2021dec`** (OpenCV Zoo SFace 2021dec):
   - **Resolution & Crop**: 112×112 aligned face crop via the 5-point similarity transformation; the soos template (`TARGET_LANDMARKS_112`) is identical to OpenCV `FaceRecognizerSF::alignCrop`.
   - **Tensor Layout**: **NCHW** — graph input `data` is `[1, 3, 112, 112]` (batch fixed to 1). `OrtEmbeddingExtractor` refuses any session whose layout is not the spec's (`SFACE_2021DEC.input_layout`), and `ModelRegistry` rejects the session if it disagrees with the manifest `input_layout`.
   - **Color Format**: R, G, B planes, exactly like OpenCV `blobFromImage(aligned, 1, Size(112, 112), Scalar(0, 0, 0), swapRB = true)` on its BGR image.
   - **Normalization**: none in soos: the raw `[0, 255]` values are fed and the graph applies `(x - 127.5) / 128` itself (first nodes `Sub(127.5)` then `Mul(0.0078125)`, pinned by `embedding_real_model_tests::test_real_sface_graph_normalizes_in_graph`).
   - **Output**: graph output `fc1` `[1, 128]`, raw; the extractor L2-normalizes it.
   - **Cost**: 9,667,074 float32 parameters; one embedding measured at p50 9.8 ms / p95 11.0 ms on one ORT intra-op thread (`embedding_real_model_tests`).
   - **Logging**: the IR 6 export lists 174 initializers as graph inputs; the registry creates this session at ORT log level `Error` (`ERROR_ONLY_LOG_MODELS`), so the per-initializer warnings do not flood the journal.

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
- `input_shape` is always the logical `[N, C, H, W]` shape; `input_layout` (default `"NCHW"`) is the physical layout of the graph input. An entry without `input_layout` (a manifest installed by an earlier release) has its layout *unspecified*: the input may be the logical shape in NCHW or NHWC order, rank and every dim are still enforced, the SHA-256 already binds the exact file, and `ModelRegistry` logs a one-time warning that the manifest predates layout attestation. An explicit `input_layout` is always enforced and a wrong value fails closed. The committed manifest declares `input_layout` explicitly for the embedding model (`"NCHW"` for SFace; the retired ArcFace entry declares `"NHWC"`).

### Expected SHA-256 Checksums (v2.0.0)

```text
scrfd_500m_kps:     a3562ef62592bf387f6ef19151282ac127518e51c77696e62e0661bee95ba1ad
sface_2021dec:      0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79  (38,696,353 bytes)
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

2. **SFace 2021dec (`sface_2021dec`)**:
   - Upstream: [OpenCV Zoo `face_recognition_sface`](https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface), Hugging Face mirror [`opencv/face_recognition_sface`](https://huggingface.co/opencv/face_recognition_sface) at the pinned revision `3d7082438a6e4551e840c9b2bb60b71e8da4b524` (`face_recognition_sface_2021dec.onnx`; the LFS etag equals the SHA-256)
   - Architecture: MobileFaceNet backbone trained with the SFace loss ([arXiv:2205.12010](https://arxiv.org/abs/2205.12010)); ONNX IR 6, opset 11, 88 nodes, 9,667,074 float32 parameters; 38,696,353 bytes; 128D output
   - License: Apache-2.0 ("All files are licensed under Apache 2.0 License", opencv_zoo README and LICENSE)
   - Training data: undocumented for this file (possibly MS1MV2, derived from the withdrawn MS-Celeb-1M; opencv_zoo issue #318 unanswered); the owner accepted this risk on 2026-10-01 (AI/DECISIONS.md)
   - Measured on LFW with the soos pipeline: 10-fold accuracy 0.9848, TAR 0.984 at FAR 1e-4, FAR 3.9e-6 / TAR 0.957 at the 0.50 default (walkthrough 160)

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

Retired from manifest v2.0.0 on 2026-10-01 (GitHub #278, walkthrough 162), attested only in `models/retired_models.toml` (read by no binary):
- `arcface_w600k_mbf` (`arcface_w600k_mbf.onnx`, `NOASSERTION`, 136,619,444 bytes, SHA-256 `ffe014a45c9488506719d37fd578ece6661bb385535b36e8039975fa5d4683db`): a Keras ArcFace ResNet34 exported with `tf2onnx` 1.16.1 from [garavv/arcface-onnx](https://huggingface.co/garavv/arcface-onnx) (`arc.onnx`, revision `224c23c`), NHWC `input_1` `[N, 112, 112, 3]`, output `embedding` `[N, 512]`, fed BGR `(x - 127.5) / 127.5`; not an InsightFace `w600k_mbf` despite its historical id. Replaced because its source declares no licence and SFace measured better on LFW (walkthrough 160). An installed `/var/lib/soos/models/arcface_w600k_mbf.onnx` is unused; `scripts/download_models.sh` reports it as not attested and never deletes it.

Manifest v2.0.0 completely supersedes the v1.0.0 models. These ids are historical only: `scripts/download_models.sh` no longer resolves any download URL for them (GitHub #249), and no production code loads them. The unified `scrfd_500m_kps` model eliminates the separate `landmark_5point` inference stage, reducing total verification latency by ~20ms.

---

## 6. Legal Notice & Redistribution Restrictions

- **Open Source Licensing**: The model architectures and pre-trained weights referenced in `manifest.toml` are authored by their respective upstream creators and licensed under permissive open-source licenses (MIT and Apache License 2.0). The shipped embedding model `sface_2021dec` is Apache-2.0; its training data is not documented upstream, a risk the owner accepted on 2026-10-01 (GitHub #278). The retired `arcface_w600k_mbf` (`models/retired_models.toml`, evaluation only) declares no licence (`NOASSERTION`) and is not deployed.
- **Redistribution Policy**: In strict compliance with zero-trust principles and source repository hygiene, compiled binary weights (`*.onnx`) are **NOT** bundled or tracked in git version control. They are downloaded directly from authenticated upstream sources or local installation packages during setup.
- **Third-Party Rights**: Users and distribution packagers must comply with the upstream license agreements when acquiring, caching, or distributing model weights for end-user deployments.
