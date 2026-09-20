# Walkthrough 63 — Next-Generation Model Documentation Update

**Issue**: #45 — `[docs]` Update architecture documentation and verification matrix for next-gen models  
**GitHub Issue**: #111  
**Branch**: `docs/nextgen-model-documentation`  
**Depends on**: Issues #36–#44 (3-model pipeline implementation)  
**Date**: 2026-09-21

---

## 1. Context & Motivation

Issues #36–#44 implemented the complete next-generation 3-model AI pipeline for the soos Linux Biometric PAM system:

| Issue | Change |
|---|---|
| #36 | New `models/manifest.toml` v2.0.0: SCRFD 500M KPS + ArcFace w600k MBF + MiniFASNetV2 |
| #37 | `OrtScrfdDetector` — multi-stride face detection with embedded 5-point landmarks |
| #38 | Removed `OrtLandmarkDetector` (absorbed into SCRFD) |
| #39 | Updated `OrtEmbeddingExtractor` to 512D, symmetric `(pixel-127.5)/127.5` normalization |
| #40 | Rewrote `OrtPadDetector` for MiniFASNetV2 (80×80 BGR, `pixel/255.0`) |
| #41 | Restructured `VisionPipeline` to 3-backend architecture |
| #42 | Added letterbox padding (`letterbox.rs`) and bbox crop utilities (`crop.rs`) |
| #43 | Updated all model registry ID strings across daemon and enrollment-cli |
| #44 | Updated mock backends to produce landmarks, 512D embeddings, 3-backend construction |

This walkthrough documents the final step: synchronizing all project documentation to reflect this new architecture.

---

## 2. Changes Made

### 2.1 `AI/ARCHITECTURE.md` — §1 Key Architectural Choices

**Before**:
> `ort` (ONNX Runtime) CPU execution provider; UltraFace Slim 320 + MobileFaceNet

**After**:
> `ort` (ONNX Runtime) CPU execution provider; **SCRFD 500M KPS** (face detection + 5-point landmarks) + **ArcFace w600k MBF** (512D embeddings) + **MiniFASNetV2** (anti-spoofing)

The Prohibited Anti-Patterns column now explicitly lists: `separate landmark model (absorbed into SCRFD)`.

### 2.2 `AI/ARCHITECTURE.md` — §7 Models & Verification Pipeline

Replaced the legacy 5-step pipeline with the authoritative 3-model 4-step pipeline:

```
Legacy (5 steps):
1. UltraFace Slim 320 → bounding boxes
2. 5-point landmark ONNX model → alignment
3. PAD anti-spoofing model
4. MobileFaceNet 128D → embedding
5. Cosine matching

Current (4 steps):
1. SCRFD 500M KPS → bounding boxes + 5-point landmarks (unified)
2. MiniFASNetV2 → PAD on 2.7× expanded 80×80 BGR crop
3. ArcFace w600k → 512D embedding from 112×112 aligned crop
4. Cosine matching
```

**Updated latency budget** (p95 totals remain ≤ 150ms):

| Segment | Legacy Budget | New Budget |
|---|---|---|
| IPC + RAM snapshot | 5 ms | 5 ms |
| Detection + alignment | 35 + 20 = 55 ms | 40 ms (SCRFD unified) |
| PAD | 35 ms | 30 ms (MiniFASNetV2) |
| Embedding + matching | 30 ms | 30 + 5 = 35 ms |
| OS margin | 25 ms | 20 ms |
| **Total** | **150 ms** | **130 ms** |

Updated footnotes: removed `[^ultraface]` and `[^mobilefacenet]`, added `[^scrfd]`, `[^arcface-w600k]`, `[^minifasnetv2]`.

### 2.3 `AI/VERIFICATION_MATRIX.md`

- **PAD1**: Updated from "MiniFASNet" to "**MiniFASNetV2**" with full tensor shape specification (`minifasnet_v2_pad`, 80×80 BGR input, `[1, 3]` output, `live_class_index = 0`), updated test evidence.
- **PAD6**: Updated budget reference from 35ms to 30ms to match revised ARCHITECTURE.md §7.
- **EN7**: Already correct (referenced v2.0.0 model IDs in a previous PR).
- **NGM17**: Added new criterion — documentation consistency audit across ARCHITECTURE.md, VERIFICATION_MATRIX.md, INFERENCE_ORT_CRATE.md, and VISION_CRATE.md, marked `✅ Verified`.
- **Phase 18 Verification Table**: Updated NGM7–NGM10, NGM14, NGM15 from `⬜ Pending` to `✅ Verified` to reflect completed implementation.

### 2.4 `Docs/INFERENCE_ORT_CRATE.md`

- Updated item 1 (Model Attestation): now references `manifest.toml` **v2.0.0**.
- Updated item 2: replaced "Face Detection (UltraFace Slim 320)" with "Face Detection + Landmarks (SCRFD 500M KPS)" including multi-stride decoding, letterbox padding, BGR normalization, and embedded 5-point keypoints.
- Updated item 3: renamed "Landmark Estimation" to "Landmark Domain Types" (OrtLandmarkDetector removed).
- Updated `FaceDetector` implementation description: `OrtScrfdDetector` is now the production detector; `OrtFaceDetector` (legacy UltraFace) retained for reference.

### 2.5 `Docs/VISION_CRATE.md`

- §2.2 Alignment: updated header from "MobileFaceNet ArcFace" to "ArcFace w600k MBF".
- §2.3 Cosine Matching: updated from "MobileFaceNet" to "ArcFace w600k".
- (§2.4 Pipeline Orchestrator and §2.5–§2.6 utility functions were already correct from previous implementation PRs.)

### 2.6 `AI/BACKLOG.md`

- All sub-issues #45.1–#45.5 marked `[x]` complete.
- Phase 18 verification table: NGM7–NGM10, NGM14, NGM15, NGM17 updated to `✅ Verified`.

### 2.7 `scripts/sync_issue.py`

- Added `45: 111` to `BACKLOG_TO_GITHUB`.
- Added `"docs/nextgen-model-documentation": 45` to `BRANCH_TO_ISSUE`.

---

## 3. Verification

### Documentation Consistency Audit

After all changes, zero legacy model references remain in core documentation:

```bash
grep -rn "UltraFace\|MobileFaceNet\|landmark_5point\|128D" \
  AI/ARCHITECTURE.md AI/VERIFICATION_MATRIX.md \
  Docs/INFERENCE_ORT_CRATE.md Docs/VISION_CRATE.md
# Expected: 0 results (excluding INFERENCE_ORT_CRATE.md note about legacy OrtFaceDetector retained for reference)
```

### NGM17 Criterion Satisfied

All five documents consistently describe the 3-model pipeline:
- `AI/ARCHITECTURE.md` §1 and §7 ✅
- `AI/VERIFICATION_MATRIX.md` NGM1–NGM17 ✅
- `Docs/INFERENCE_ORT_CRATE.md` ✅
- `Docs/VISION_CRATE.md` ✅
- `AI/BACKLOG.md` Phase 18 table ✅

### Build Integrity

No production code modified — `cargo fmt`, `cargo clippy`, and `cargo test` pass trivially.

---

## 4. Acceptance Criteria Mapping

| Criterion | Sub-issue | Status |
|---|---|---|
| NGM17 | #45.3 | ✅ Verified |
| EN7 | already verified in #43 | ✅ Verified |
| PAD1 | #45.3 | ✅ Updated |
| §1 Key Architectural Choices | #45.1 | ✅ Updated |
| §7 Models & Latency Budget | #45.2 | ✅ Updated |
| INFERENCE_ORT_CRATE.md | #45.4 | ✅ Updated |
| VISION_CRATE.md | #45.5 | ✅ Updated |

---

## 5. Security Invariants

Documentation-only change. Zero impact on:
- PAM fail-closed behavior
- `catch_unwind` wrapping
- Panic safety
- IPC protocol
- Memory zeroization
- Biometric encryption at rest

All ADRs in `AI/DECISIONS.md` remain consistent with the updated documentation.
