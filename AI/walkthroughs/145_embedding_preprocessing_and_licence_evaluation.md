# Walkthrough 145 — Embedding Pre-processing and Licence Evaluation

- **Date**: 2026-09-30
- **Issue**: GitHub #278 ([STO-FU] fourth item: evaluate a licence-compatible lighter embedding
  model and recalibrate `match_threshold` / BGR order / normalization for the shipped ArcFace ResNet34)
- **Branch**: `fix/p2-embedding-preproc-eval`
- **Matrix criteria**: SFX1, SFX2, SFX3 (new, ✅ Verified); SFX4 (new, ⏳ Pending)
- **ADR**: 2026-09-30 "Embedding Pre-processing Evaluation" in `AI/DECISIONS.md`

---

## 1. Context

Issue #278 has four items. The first three (GDM gate de-duplication with jumps, FIFO include
targets, the `.opaque.enc` evidence suffix) were merged in #284 (walkthrough 127, matrix
SFU1–SFU4). This walkthrough covers the fourth item, which walkthrough 127 left as SFU5
(⏳ Pending). SFU5 stays as it is (the matrix is append-only); the new SFX rows record the
evidence gathered here.

ADR 2026-09-30 "Face Embedding Model Identity & Retention" (#191) said three things about the
attested ResNet34 were never checked: the channel order it was trained with, its input
normalization, and the `match_threshold = 0.70` default. It also said a lighter model with a
compatible licence still had to be evaluated.

## 2. Evidence

1. **Provenance.** The Hugging Face API for `garavv/arcface-onnx` (revision `224c23c`, last
   modified 2025-06-04) lists `arc.onnx` with LFS SHA-256
   `ffe014a45c9488506719d37fd578ece6661bb385535b36e8039975fa5d4683db` and 136,619,444 bytes, the
   same values as the manifest. The model card belongs to the attested file.
2. **Upstream pre-processing.** The model card's quick start does `cv2.cvtColor(img,
   cv2.COLOR_BGR2RGB)` and then `(img.astype(np.float32) - 127.5) / 128.0` on a
   `(1, 112, 112, 3)` tensor. So it expects **RGB** input divided by **128**. The extractor feeds
   **B, G, R** divided by **127.5**.
3. **No in-graph normalization.** A bounded protobuf walk of the attested file (162 nodes) shows
   that `input_1` goes only to a `Transpose`, and that goes only to the first `Conv` (BatchNorm
   folded in). No `Sub`/`Mul`/`Div`/`Add` sits in front of it, so the extractor's arithmetic is
   the only normalization.
4. **Sensitivity on the real network** (synthetic, non-biometric patterns):

   | Pattern | cos(BGR/127.5, BGR/128) | cos(BGR/127.5, RGB/127.5) | cos(BGR/127.5, RGB/128) |
   |---|---|---|---|
   | gradient | 1.00000 | 0.97930 | 0.97941 |
   | warm_disc | 1.00000 | 0.97024 | 0.97028 |
   | stripes | 0.99998 | 0.99816 | 0.99802 |

   The divisor makes no difference to the template. The channel order does move the embedding.
   Synthetic patterns cannot show which order the network was trained on, or how far a real face
   embedding moves.
5. **Licence.** The upstream repository has no licence tag, no licence file and no licence in
   its card. The manifest's `license = "MIT"` has no support in the source.
6. **Lighter candidate.** OpenCV Zoo SFace (`face_recognition_sface_2021dec.onnx`) is a
   MobileFaceNet trained with the SFace loss. Its model files are Apache-2.0 and the zoo's own
   evaluation reports 0.9940 accuracy. The terms of its training data still need review. We
   would use only the ONNX file, never the OpenCV library, which the project prohibits.
   InsightFace `buffalo_sc` remains excluded because of its non-commercial terms. No model was
   downloaded, attested or swapped.

## 3. Why No Production Change

- The BGR order and the 127.5 divisor are pinned by tests that already exist on `main`
  (`embedding_tests::test_arcface_input_bgr_ordering`,
  `embedding_tests::test_prepare_input_layout_nhwc_and_nchw` and
  `embedding_tests::test_embedding_normalization_symmetric_range` in
  `crates/inference-ort/tests/embedding_tests.rs`). The licence string is pinned by
  `crates/inference-ort/tests/manifest_tests.rs`. Under the test-integrity rule, changing any of
  them needs approval. The exact proposals are in the branch report.
- Switching the order changes every enrolled template, so re-enrollment and a threshold
  recalibration have to go with it. That is an owner decision.
- One alternative was rejected: a pre-processing switch driven by the manifest, which would
  leave the tested helper unchanged while production takes another path. That would get around
  the pinned contract instead of changing it with approval.

## 4. Changes

| File | Change |
|---|---|
| `crates/inference-ort/tests/embedding_preprocessing_evaluation_tests.rs` | New real-model evaluation target (SFX1–SFX3). Gated like `embedding_real_model_tests`; prints only cosines between synthetic-pattern embeddings |
| `crates/inference-ort/src/embedding.rs` | The `OrtEmbeddingExtractor` doc comment now states the upstream contract and the result of the evaluation (no behaviour change) |
| `models/manifest.toml` | A comment above the embedding `license` line says the value is not backed by the source (the comment is skipped by the manifest parsers) |
| `Docs/INFERENCE_ORT_CRATE.md` | New "Embedding pre-processing evaluation" section |
| `.agents/skills/dev-workflow/references/project-facts.md` | §4 embedding row updated with the upstream contract and the licence finding |
| `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md` | New ADR; new component with rows SFX1–SFX4 |

## 5. Red → Green

Nothing turns red here: this is an evaluation, and no production behaviour changes. The three
new tests are evidence tests and pass on the current code (`cargo test -p soos-inference-ort
--test embedding_preprocessing_evaluation_tests -- --nocapture`: 3 passed, report lines as in §2).
Without `/var/lib/soos/models` they print `SKIPPED`. With `SOOS_REQUIRE_REAL_MODELS=1` a missing
model makes them fail.

## 6. Security Notes

- The tests use synthetic patterns only. No embedding value is printed or committed; only
  cosine similarities are.
- The protobuf walk reads only the attested file, after checking its exact size, and asserts
  that every field length stays within the buffer.
- No production path, PAM code, IPC or permission changed.

## 7. Follow-ups (owner decisions)

1. Switch the extractor to the documented RGB order (the divisor can follow as `128.0` at no
   template cost). This needs approved updates to the pinned contract tests and re-enrollment of
   every user.
2. Settle the licence status of the current model: change the manifest to `NOASSERTION` (needs
   an approved `manifest_tests.rs` update) or replace the model.
3. Evaluate SFace: review the training-data terms, attest it under a new manifest id, add its
   pre-processing contract, and migrate or re-enroll templates.
4. Recalibrate `match_threshold` with FAR/FRR measured on labelled real captures (needs
   hardware and data).
