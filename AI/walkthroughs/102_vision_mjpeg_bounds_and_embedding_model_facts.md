# Walkthrough 102 — Bounded MJPEG Decoding and the Real Embedding Model

- **Date**: 2026-09-30
- **Issues**: Review findings VIS-02 (GitHub #190) and VIS-03 (GitHub #191)
- **Branch**: `fix/vision-mjpeg-bounds-model-facts`
- **Matrix criteria**: MJB1–MJB6, EMR1–EMR6 (new)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) confirmed two defects in
the vision and inference core:

1. **VIS-02** — `convert_to_rgb` decoded MJPEG frames with `jpeg_decoder::Decoder::decode()` and no
   limit (`decoding_buffer_size_limit` is `usize::MAX` by default in jpeg-decoder 0.3.2). The SOF
   header of a device-supplied frame decides the allocation, so a 65535×65535 header could make the
   root daemon try to allocate about 12.9 GB and abort. Only the decoded byte count was compared, so
   a 16×8 frame was accepted for an 8×16 request, and CMYK / 16-bit frames returned buffers of any
   size.
2. **VIS-03** — the manifest, `models/README.md`, `AI/DECISIONS.md` and `AI/ARCHITECTURE.md`
   described the embedding model as an "ArcFace MobileFaceNet w600k, ~3.6 MB, NCHW, RGB". The
   attested file is a 136 MB tf2onnx export of a Keras ArcFace ResNet34 with an NHWC input. The
   manifest shapes were never checked against the session.

## 2. Real-Model Metadata (observed, not assumed)

The installed files in `/var/lib/soos/models` match the committed checksums. Their ONNX protobuf
was read directly (a raw protobuf walk of `ModelProto` / `GraphProto`, no third-party tool) and
through ONNX Runtime (`session.inputs()` / `outputs()`):

| Model id | Producer | Params (float32) | Input | Output(s) |
|---|---|---:|---|---|
| `arcface_w600k_mbf` | `tf2onnx` 1.16.1, opset 15, nodes `StatefulPartitionedCall/ResNet34/...` | 34,138,432 (164 initializers, 136,619,444 bytes) | `input_1` `[unk__556, 112, 112, 3]` (NHWC) | `embedding` `[unk__557, 512]` |
| `scrfd_500m_kps` | PyTorch 1.8 | 626,354 | `input.1` `[?, 3, ?, ?]` | 9 × `[?, ?, 1 / 4 / 10]` |
| `minifasnet_v2_pad` | PyTorch 2.8.0 | 429,760 | `input` `[batch_size, 3, 80, 80]` | `output` `[batch_size, 3]` |

ORT reports the symbolic dims as `-1`. The attested embedding network uses 34.1 M parameters,
about ten times more than the MobileFaceNet it was described as. PReLU is exported as
`Relu`/`Neg`/`Mul`.

## 3. Architect Design

### 3.1 Vision (`crates/vision/src/color.rs`, `error.rs`)

- New constants: `MAX_MJPEG_COMPRESSED_BYTES = 16 MiB` and `MAX_MJPEG_DIMENSION = 4096` (covers 4K
  UHD and keeps the decoder output at most about 48 MiB).
- New typed errors: `VisionError::MjpegInputTooLarge { max, actual }` and
  `VisionError::MjpegFrameMismatch { expected_width, expected_height, actual_width, actual_height }`.
- The MJPEG branch now calls `decode_mjpeg`, which: bounds the compressed size → bounds the
  negotiated dims → `read_info()` (headers only) → requires SOF dims equal to the negotiated dims →
  accepts only `RGB24` / `L8` (CMYK32 and L16 fail closed with `ColorConversionFailed`; UVC MJPEG is
  YCbCr) → `set_max_decoding_buffer_size(exact expected size)` → `decode()` → checks the exact
  length → replicates greyscale.

jpeg-decoder checks `decoding_buffer_size_limit` only once the scans are decoded. The per-component
planes are sized from the header while scans are decoded. That is why the header check in step 3 is
the guard that matters, and the buffer cap is a second layer.

### 3.2 Inference (`crates/inference-ort/src/manifest.rs`, `registry.rs`, `error.rs`, `embedding.rs`)

- `TensorLayout { Nchw (default, "NCHW"), Nhwc ("NHWC") }` and the optional manifest field
  `input_layout`. `input_shape` keeps its meaning as the logical `[N, C, H, W]` shape.
- `ModelMetadata::expected_input_dims()` gives the physical dims. For NHWC it permutes to
  `[N, H, W, C]` and requires rank 4.
- `ModelMetadata::validate_session_shapes(inputs, outputs)` checks: exactly one input; the declared
  number of outputs when `output_shapes` is set; the same rank everywhere; every concrete dim equal
  (zero included). Negative session dims are wildcards. This follows the verifier caveat that the
  shipped graphs declare symbolic batch dims.
- `ModelRegistry::get_or_load_session` runs the validation after the SHA-256 check and the session
  build, and before caching. Any mismatch fails closed with
  `InferenceError::ModelShapeMismatch { id, detail }`.
- `OrtEmbeddingExtractor::is_nhwc()` accessor, used by the real-model tests.

### 3.3 Why `input_layout` instead of `input_shape = [1, 112, 112, 3]`

The issue proposed writing the physical NHWC shape into `input_shape`. The existing contract test
`manifest_tests::test_parse_workspace_manifest_file` asserts
`arcface.input_shape == [1, 3, 112, 112]`, and tests are immutable. So `input_shape` stays the
logical NCHW shape, and the new `input_layout = "NHWC"` states the physical layout that the registry
enforces. The manifest therefore states both facts and neither is dropped. Changing `input_shape`
itself to the physical shape would need an explicit user decision to amend that existing test.

## 4. Tester Contract (Red Evidence)

New targets, written before the implementation and run against API stubs (constants, error
variants, `TensorLayout`, no-op validation):

- `crates/vision/tests/mjpeg_bounds_tests.rs`: 11 tests and 3 proptest properties on synthetic 16×8
  ImageMagick fixtures (YCbCr, greyscale, Adobe CMYK), with an SOF-patching helper.
  **Red: 8 of 14 failed.** Examples: `test_mjpeg_oversized_header_rejected` got
  `ColorConversionFailed("... failed to decode huffman code")` after trying to decode;
  `test_mjpeg_dimension_mismatch_fails_closed` got `Ok` for the transposed frame;
  `test_mjpeg_cmyk_frame_rejected` got `Ok`; `prop_mjpeg_header_geometry_must_match` and
  `prop_mjpeg_mutated_frames_fail_closed` both failed.
- `crates/inference-ort/tests/manifest_shape_tests.rs`: 15 tests. **Red: 9 of 15 failed**, including
  the layout permutation, every mismatch case, and the committed-manifest truth checks
  (`test_workspace_manifest_embedding_description_is_truthful`,
  `test_workspace_manifest_declares_embedding_layout_nhwc`).
- `crates/inference-ort/tests/embedding_real_model_tests.rs`: 6 real-model tests, gated like
  walkthrough 87 (`SOOS_MODELS_DIR`, `SOOS_REQUIRE_REAL_MODELS=1`, `SKIPPED` otherwise). **Red: 2 of
  6 failed**: `test_real_embedding_committed_manifest_matches_session` and
  `test_registry_rejects_real_embedding_model_under_nchw_manifest`, because the registry accepted
  the NHWC graph under an NCHW manifest. The metadata pin, extractor and latency tests passed on the
  stub, since they describe what exists today.

No existing test was modified, weakened or deleted.

## 5. Audit

- `#![forbid(unsafe_code)]` still holds in `soos-vision` and `soos-inference-ort`.
- Allocations: the compressed input is bounded, and so are the decoded output (exact size) and the
  header-derived dims (checked before `decode`). The only new allocations in the registry are
  shape vectors sized by the graph's rank.
- No panics: there is no `unwrap` / `expect` / indexing in the new production code
  (`let [x] = inputs else`, `usize::try_from`, slice patterns).
- No sensitive data: error strings carry dims, byte counts and jpeg-decoder messages, never pixel
  data or embeddings. The real-model tests use synthetic, non-biometric inputs and commit no
  embedding values.
- Deployment: a `manifest.toml` from an older release (no `input_layout`) now makes the daemon
  refuse the NHWC embedding model at startup, which fails closed (PAM falls back to the password).
  `scripts/download_models.sh` / `install.sh` always install the manifest shipped with the release.
  Its bash parser ignores unknown keys (`--preflight` passes).

## 6. Decision on the Embedding Model (ADR 2026-09-30) and Follow-up

**Keep** the attested ResNet34 for now:

- It is the only attested embedding model, and every enrolled template is bound to it. A swap means
  re-attesting, re-enrolling every user and recalibrating the thresholds.
- The obvious replacement, InsightFace `buffalo_sc` `w600k_mbf.onnx` (MobileFaceNet, NCHW, about
  13 MB), is published under InsightFace's non-commercial-research terms. Those terms must be
  reviewed before adoption.
- Latency, measured by `test_real_embedding_latency_report` (one ORT intra-op thread, 3 warm-ups +
  20 runs, development host under concurrent build load): **p50 127.5 ms, p95 170.9 ms** for one
  embedding. The 30 ms row and the 150 ms per-capture target in `AI/ARCHITECTURE.md` §7 are not met.
  The bounds actually enforced are unchanged: `DECISION_BUDGET_MS = 900` and the EMA admission
  estimate (`MAX_INFERENCE_ESTIMATE_MS = 1000`). An `Allow` needs three consensus captures,
  so at least three embeddings must fit in that budget.

**Follow-up (scheduled, not done here; no model was downloaded or swapped):**

1. Evaluate a licence-compatible lightweight embedding model (MobileFaceNet class, NCHW):
   re-attestation, a new manifest id, template migration or re-enrollment, and FAR/FRR calibration.
2. Verify, for the current network, the channel order it was trained with. The extractor feeds
   B, G, R, a choice walkthrough 71 made assuming an InsightFace model. Also verify its input
   normalization: the extractor uses `(x - 127.5) / 127.5`, while the upstream Keras pipeline may
   differ.
3. Recalibrate `match_threshold = 0.70`, which comes from MobileFaceNet literature
   (`crates/policy/src/threshold.rs`, `Docs/POLICY_CRATE.md`), on real captures.
4. Out of this branch's scope, for the owners of those crates: the doc comment of `MODEL_ID_EMBEDDING`
   in `crates/enrollment-cli/src/service.rs` still says "ArcFace MobileFaceNet w600k", and the
   `soos-enroll --model-id` default is `mobilefacenet` (GitHub #182–#184).

## 7. Documentation Updated

`models/manifest.toml` (header comment, embedding `description`, `input_layout`; ids, file names and
checksums unchanged), `models/README.md`, `AI/DECISIONS.md` (new ADR, and a pointer on the
2026-09-20 entry), `AI/ARCHITECTURE.md` §1 / §7 and footnote, `AI/MOCK_STRATEGY.md`,
`Docs/VISION_CRATE.md` (§2.1.1 bounded MJPEG, §3 latency), `Docs/INFERENCE_ORT_CRATE.md` (shape
attestation, embedding real-model evidence), `.agents/skills/dev-workflow/references/project-facts.md`
§4, the `soos-inference-ort` crate docs and the `Cargo.toml` description, and
`AI/VERIFICATION_MATRIX.md` (MJB1–MJB6, EMR1–EMR6).

## 8. Verification

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`: all green (see the
  final report for the run).
- `./scripts/candid_review.sh`: passed.
