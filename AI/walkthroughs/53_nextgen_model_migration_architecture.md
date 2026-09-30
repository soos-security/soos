# Walkthrough 53 — Next-Generation ONNX Model Migration Architecture

> **Historical record - class order superseded.** The MiniFASNetV2 live class index stated below is obsolete. The current contract is `[PrintPhoto, Live, ScreenReplay]`, live class index 1 (`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX`), per the 2026-09-29 and 2026-09-30 ADR entries in `AI/DECISIONS.md` (GitHub #146, #171; walkthroughs 79 and 85).


> **Date**: 2026-09-20
> **Scope**: AI model modernization — replacing 4 obsolete ONNX models with 3 modern ones
> **Status**: Architecture & reasoning documented — implementation pending via Issues #36–#45

---

## 1. Context and Motivation

The current soos biometric pipeline relies on 4 ONNX models that are increasingly difficult to source and lack modern accuracy benchmarks:

| Model | Role | Input | Output | Size | Problem |
|---|---|---|---|---|---|
| UltraFace Slim 320 | Face detection only | `[1,3,240,320]` | `[1,4420,2]` + `[1,4420,4]` | ~1 MB | No landmarks, low resolution, obsolete repo |
| landmark_5point | Separate landmark detector | `[1,3,112,112]` | `[1,10]` | ~0.5 MB | Extra inference pass, adds ~20ms latency |
| MobileFaceNet ArcFace | 128D embeddings | `[1,3,112,112]` | `[1,128]` | ~3.6 MB | Low-dim embeddings, old training data |
| MiniFASNet | Anti-spoofing | `[1,3,112,112]` | `[1,3]` | ~1.8 MB | Source repo archived, no updates |

**Key architectural problem**: Detection and landmarks are separate models, requiring two sequential inference passes. Modern detectors like SCRFD unify both in a single forward pass.

---

## 2. Selected Models and Rationale

### 2.1 SCRFD 500M KPS — Unified Face Detection + 5-Point Landmarks

**Why SCRFD over alternatives:**
- **RetinaFace-light**: Heavier (~10MB), no official ONNX export with landmarks integrated
- **YOLOFace**: Newer but less battle-tested in the InsightFace ecosystem; landmark ordering may differ from ArcFace canonical coordinates
- **SCRFD 500M**: Only ~2.4MB, outputs both boxes AND 5-point landmarks in a single pass, well-documented in InsightFace ecosystem, MIT license

**Technical specifications:**

| Property | Value |
|---|---|
| Download | `https://huggingface.co/RuteNL/SCRFD-face-detection-ONNX/resolve/main/500m.onnx` |
| File size | ~2.4 MB |
| Input | `[1, 3, 640, 640]` NCHW, BGR, normalized `(pixel - 127.5) / 128.0` |
| Output | 9 tensors across 3 strides (8, 16, 32): scores, bboxes, keypoints |
| Latency | ~3.6 ms CPU at VGA |
| Accuracy | ~77.8% mAP WIDER FACE Hard |

**Output tensor structure (for 640×640 input):**

```
Stride 8:  score_8  [1, 12800, 1]  bbox_8  [1, 12800, 4]  kps_8  [1, 12800, 10]
Stride 16: score_16 [1, 3200, 1]   bbox_16 [1, 3200, 4]   kps_16 [1, 3200, 10]
Stride 32: score_32 [1, 800, 1]    bbox_32 [1, 800, 4]    kps_32 [1, 800, 10]
```

Each stride has `(H/stride × W/stride × 2_anchors)` entries. Anchors per cell = 2.

**Decoding algorithm (distance-to-border):**
```
For each stride s ∈ {8, 16, 32}:
  grid_h = 640 / s, grid_w = 640 / s
  For row in 0..grid_h, col in 0..grid_w, anchor in 0..2:
    idx = (row * grid_w + col) * 2 + anchor
    confidence = sigmoid(score[idx])
    if confidence > threshold:
      x1 = (col - bbox[idx][0]) * s
      y1 = (row - bbox[idx][1]) * s
      x2 = (col + bbox[idx][2]) * s
      y2 = (row + bbox[idx][3]) * s
      for i in 0..5:
        lm_x = (col + kps[idx][i*2]) * s
        lm_y = (row + kps[idx][i*2+1]) * s
```

**Critical difference from UltraFace:** UltraFace uses center-offset + exponential scale with pre-computed anchor priors (4,420 boxes). SCRFD uses grid-based distance-to-border decoding with NO explicit prior generation. The entire `generate_priors()` function and its associated decode logic must be replaced.

**Input preprocessing difference:** UltraFace accepts RGB input. SCRFD requires BGR channel ordering. Additionally, SCRFD expects letterbox-padded input (aspect-ratio-preserving resize with border padding) rather than naive stretch-resize.

### 2.2 ArcFace MobileFaceNet w600k — 512D Biometric Embeddings

**Why w600k over alternatives:**
- **AdaFace**: Better accuracy but significantly heavier (~25MB), exceeds weight budget
- **GhostFaceNets**: Experimental, limited ONNX availability, uncertain landmark compatibility
- **w600k_mbf**: Same MobileFaceNet architecture (lightweight), but trained on WebFace600K (much larger dataset than original), 512D output provides 4× the discriminative capacity of current 128D, fully compatible with existing InsightFace alignment pipeline

**Technical specifications:**

| Property | Value |
|---|---|
| Download | `https://huggingface.co/garavv/arcface-onnx/resolve/main/arc.onnx` |
| File size | ~3.6 MB |
| Input | `[1, 3, 112, 112]` NCHW, RGB, normalized `(pixel - 127.5) / 127.5` |
| Output | `[1, 512]` float32 |
| Latency | ~20-24 ms CPU |
| Accuracy | >99.0% LFW, ~99% CFP-FP |

**Normalization difference analysis:**
```
Current:  (pixel - 127.5) / 128.0  → range [-0.996, +1.0]
New:      (pixel - 127.5) / 127.5  → range [-1.0, +1.0]
```
The difference is in the denominator: 128.0 vs 127.5. This is a subtle but mathematically significant change. Using the wrong denominator will produce valid-looking but slightly skewed embeddings, causing reduced match accuracy without obvious errors. The 127.5 denominator produces exact symmetric [-1.0, +1.0] range which is what the w600k model was trained with.

**Embedding dimensionality impact:**
Since the project is in development state with no enrolled users, the 128D→512D change has zero migration impact. The `BiometricEmbedding` struct is already dimension-agnostic (uses `Vec<f32>`). The `cosine_similarity` function works with any matching dimension. Only the `MockEmbeddingExtractor` default and docstrings need updating.

### 2.3 MiniFASNetV2 — Presentation Attack Detection

**Why MiniFASNetV2 over alternatives:**
- **Active liveness (blink/head-turn)**: Adds user friction, incompatible with passive PAM flow
- **FAS-SGTD or other deep PAD**: Too heavy for CPU budget
- **MiniFASNetV2**: Same family as current MiniFASNet (code compatibility), lighter (80×80 vs 112×112 input), faster inference, maintained in onnx-ready fork

**Technical specifications:**

| Property | Value |
|---|---|
| Download | `https://github.com/QingHeYang/Silent-Face-Anti-Spoofing-onnx/raw/main/models/anti_spoof_models/2.7_80x80_MiniFASNetV2.onnx` |
| File size | ~1.8 MB |
| Input | `[1, 3, 80, 80]` NCHW, BGR, normalized `pixel / 255.0` |
| Output | `[1, 3]` softmax probabilities |
| Latency | ~5-10 ms CPU |
| Accuracy | ~98% CelebA Spoof |

**Three critical preprocessing changes:**

1. **Input resolution**: 112×112 → 80×80 (fewer pixels = faster inference)
2. **Normalization**: `(pixel - 127.5) / 128.0` → `pixel / 255.0` (simpler, [0,1] range instead of [-1,1])
3. **Crop strategy**: The current pipeline feeds the aligned 112×112 face crop to PAD. MiniFASNetV2 instead requires a **2.7× expanded bounding box crop** centered on the detected face. This wider crop captures surrounding context (screen bezels, paper edges, lighting gradients) which is essential for anti-spoofing analysis.

**Class ordering decision:**
The original Silent-Face-Anti-Spoofing repository uses: `Class 0 = Live, Class 1 = Print, Class 2 = Replay`. The QingHeYang ONNX fork preserves this ordering. However, the CURRENT soos code assumes `Class 0 = Print, Class 1 = Live, Class 2 = Replay` (from the original MiniFASNet paper). 

**Resolution:** After analysis of the QingHeYang fork's inference code, the class ordering is `[Live, Print, Replay]` (Class 0 = Live). The `evaluate_liveness()` function must be updated to read `p_live` from index 0 instead of index 1. This is a critical correctness fix — getting it wrong would invert live/spoof classifications.

The safest implementation strategy is to make the class index configurable via the `OrtPadDetector` constructor with a default of `live_class_index = 0`, and add a startup validation test that runs a known test fixture through the model to verify the class ordering matches expectations.

---

## 3. Architectural Decisions

### 3.1 Pipeline Simplification: 4 Models → 3 Models

```
BEFORE (4 models, 4 inference passes):
  Frame → RGB → UltraFace(320×240) → Landmarks(112×112) → PAD(112×112) → Embedding(112×112)
                [detect]               [landmarks]          [anti-spoof]   [recognition]

AFTER (3 models, 3 inference passes):
  Frame → RGB → SCRFD(640×640) → PAD(80×80, 2.7× crop) → Embedding(112×112, aligned crop)
                [detect+landmarks]  [anti-spoof]            [recognition]
```

The `LandmarkDetector` trait and `FaceLandmarks` struct remain as domain types, but `OrtLandmarkDetector` is removed. `FaceDetection` gains an `Option<FaceLandmarks>` field. The `VisionPipeline` extracts landmarks from the detection result instead of calling a separate detector.

### 3.2 SCRFD Output Tensor Resolution Strategy

SCRFD ONNX exports vary in tensor naming conventions. Some use named outputs (`score_8`, `bbox_8`, `kps_8`), others use ordinal indices. 

**Decision:** Use ordinal iteration with stride-aware grouping. The 9 output tensors always follow the pattern: `[score_s8, bbox_s8, kps_s8, score_s16, bbox_s16, kps_s16, score_s32, bbox_s32, kps_s32]`. Parse them in groups of 3 per stride. Add a startup validation that checks `session.outputs.len() == 9` and verifies the shape patterns match expected `[1, N, 1]`, `[1, N, 4]`, `[1, N, 10]` for each group.

### 3.3 SCRFD Input Resolution: 640×640

**Decision:** Use 640×640, not a smaller resolution. Rationale:
- The total pipeline latency at 640×640 (~78ms estimated) is well within the 150ms budget
- Face detection is security-critical — higher resolution means better detection of small faces at distance
- PAM authentication has a firm accuracy requirement — 320×320 would sacrifice detection range
- The extra ~2ms of SCRFD inference at 640×640 vs 320×320 is negligible compared to the ~20ms saved by eliminating the separate landmark model

### 3.4 Letterbox Padding vs Stretch Resize

**Decision:** Use letterbox padding for SCRFD input. Stretch resize distorts aspect ratio, which degrades detection accuracy for non-square camera frames (typical 640×480 or 1280×720). The letterbox function must return `(scale, pad_x, pad_y)` for coordinate un-projection after detection.

### 3.5 PAD Crop Strategy

**Decision:** MiniFASNetV2 receives a 2.7× expanded bounding box crop, NOT the aligned 112×112 crop. This is a fundamental change in the pipeline flow:

```
BEFORE: detect → landmarks → align(112×112) → PAD(aligned_crop) → embed(aligned_crop)
AFTER:  detect+landmarks → expand_bbox(2.7×) → PAD(expanded_crop_80×80)
                         → align(112×112) → embed(aligned_crop)
```

PAD and embedding now use DIFFERENT crops from the same detection. PAD uses a wide context crop; embedding uses a tightly aligned crop.

### 3.6 No Migration Needed

Since the project is in active development with no production-enrolled users, there is zero biometric template migration concern. The 128D→512D embedding change and model ID changes are clean replacements with no backward compatibility requirements.

---

## 4. Revised Latency Budget

| Segment | Old Budget (p95) | New Budget (p95) | Delta |
|---|---:|---:|---|
| IPC dispatch + RAM snapshot | 5 ms | 5 ms | — |
| Color conversion (YUYV→RGB) | 5 ms | 5 ms | — |
| Face detection (UltraFace/SCRFD) | 30 ms | 5 ms | -25 ms |
| Landmarks (separate model) | 20 ms | 0 ms | -20 ms (absorbed by SCRFD) |
| Letterbox + coordinate unproject | — | 2 ms | +2 ms (new) |
| Affine alignment (112×112) | 5 ms | 5 ms | — |
| PAD liveness (MiniFASNet/V2) | 30 ms | 8 ms | -22 ms |
| Embedding extraction + cosine | 30 ms | 25 ms | -5 ms |
| OS scheduler margin | 25 ms | 25 ms | — |
| **Total** | **150 ms** | **~80 ms** | **~47% reduction** |

---

## 5. Impact Analysis by File

### Files Requiring Major Rewrite
- `crates/inference-ort/src/detector.rs` — Complete rewrite for SCRFD architecture
- `crates/inference-ort/src/pad.rs` — Input shape, normalization, channel order, class ordering
- `crates/vision/src/pipeline.rs` — 3-model flow, different PAD crop strategy

### Files Requiring Moderate Changes  
- `crates/inference-ort/src/embedding.rs` — Normalization denominator fix (127.5)
- `crates/inference-ort/src/landmarks.rs` — Remove `OrtLandmarkDetector`
- `crates/inference-ort/src/lib.rs` — Update exports
- `crates/inference-ort/src/mock.rs` — Update mocks for new architecture
- `models/manifest.toml` — Complete model entry replacement

### Files Requiring Minor Changes
- `crates/inference-ort/src/registry.rs` — Model ID string updates
- `crates/daemon/src/pipeline.rs` — 3-session initialization
- `crates/enrollment-cli/src/service.rs` — Model ID updates
- `scripts/download_models.sh` — New download URLs

### Files Requiring Documentation Updates
- `AI/ARCHITECTURE.md` §1, §7 — Model list, latency budget
- `AI/DECISIONS.md` — New ADR entries
- `AI/VERIFICATION_MATRIX.md` — Updated model attestation criteria
- `AI/BACKLOG.md` — New Phase 18 issues
- `Docs/INFERENCE_ORT_CRATE.md` — Updated model specifications

---

## 6. Risk Assessment

| Risk | Severity | Mitigation |
|---|---|---|
| SCRFD output tensor names vary across exports | Medium | Use ordinal indexing with shape validation at startup |
| MiniFASNetV2 class ordering wrong | High | Add startup test with known fixture; configurable `live_class_index` |
| Normalization denominator error (128.0 vs 127.5) | Medium | Golden test comparing embeddings against reference implementation |
| Letterbox coordinate un-projection error | Medium | Property test: project → unproject round-trip |
| BGR/RGB channel confusion | High | Explicit channel swap in `prepare_input()`; golden test |

---

## 7. Implementation Order (Dependency Graph)

```
#36 (manifest)  ────────────────────────────────────────→ #45 (docs)
       │                                                     ↑
       ├─→ #37 (SCRFD detector) ──→ #41 (pipeline) ─────────┤
       │         │                        ↑                  │
       │         └─→ #38 (remove landmarks)──┘               │
       │                                                     │
       ├─→ #39 (embedding 512D) ──→ #41 (pipeline) ─────────┤
       │                                                     │
       ├─→ #40 (PAD v2) ─→ #42 (bbox expand) → #41 ────────┤
       │                                                     │
       ├─→ #43 (daemon/CLI IDs) ─────────────────────────────┤
       │                                                     │
       └─→ #44 (mock backends) ──→ #41 (pipeline) ──────────┘
```

Critical path: #36 → #37 → #38 → #41 → #45
```
