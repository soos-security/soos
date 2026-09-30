# Walkthrough 141 — Embedding I/O Contract, Manifest Model Sizes and Optional Second PAD Member

- **Date**: 2026-09-30
- **Issues**: GitHub #269 (VIS-15), #268 (VIS-14), #212 (PAD-07, groundwork only)
- **Branch**: `fix/p3-vision-models`
- **Matrix criteria**: VMX1–VMX8 (new component `vision-model-contracts-and-sizes`)
- **ADR**: 2026-09-30 "Optional Manifest-Gated Second PAD Member, Embedding I/O Contract and Manifest Model Sizes"

---

## 1. Findings and their state on `origin/main`

| Issue | Finding | State before this branch |
|---|---|---|
| #269 | `download_models.sh` imported a phantom `tomllib_fallback` module and ran `curl` without `--max-time`; recommendation: also print the expected size from the manifest | Python parser and phantom import already removed (#167), `--max-time 900` already present (#208). **Open**: no expected size anywhere |
| #268 | `extract_embedding` accepted an output of any length; `OrtEmbeddingExtractor::new` silently fell back to NCHW when the lock failed or the session had no input | **Open**. The other items of the finding were already covered: MJPEG bounds (`mjpeg_bounds_tests`), YUYV odd width (`frame_robustness_tests::test_yuyv_odd_width_rejected`), letterbox parity (`letterbox_parity_tests`), PAD class index / threshold wiring (`pad_wiring_tests`, `vision_threshold_parity_tests`), real-model I/O shapes (`embedding_real_model_tests`, `pad_real_model_tests`, `scrfd_real_model_tests`, gated on `SOOS_MODELS_DIR` / `/var/lib/soos/models` and run by `cargo test` when the models are present) |
| #212 | Only the 2.7-scale MiniFASNetV2 is used; upstream fuses it with the 4.0-scale MiniFASNetV1SE | Vision-side fusion already implemented (`pad_fusion.rs`, `with_additional_pad_model`, PMC9/PMC10). **Open**: model not attested, no daemon wiring |

## 2. Specification

- `soos_inference_ort::embedding::EMBEDDING_DIMENSION = 512`. `extract_embedding` returns
  `DimensionMismatch { expected: 512, actual }` for any other output length, before the copy is
  normalized.
- `OrtEmbeddingExtractor` stores `layout: Option<TensorLayout>` inferred from the first session
  input: `[_, _, _, 3]` → NHWC, `[_, 3, _, _]` → NCHW, anything else (or a poisoned lock, no input,
  non-tensor input) → `None`. New accessor `input_layout()`; `is_nhwc()` keeps its signature.
  `extract_embedding` on `None` → `TensorError` (fail closed).
- `models/manifest.toml`: optional `size_bytes = <integer>` per entry (real sizes of the three
  attested files, checked against their SHA-256 on the installed models). `download_models.sh`
  parses it as a bare decimal integer (1..size cap, no duplicates), prints it (dry run and
  download), uses it as the per-model `curl --max-filesize`, and discards a staged file of any
  other size before hashing. The Rust `ModelManifest` ignores the key (serde does not deny
  unknown fields), so `ModelMetadata` and older installed manifests are unchanged.
- Daemon (`crates/daemon/src/pipeline.rs`): `SECONDARY_PAD_MODEL_ID = "minifasnet_v1se_pad"`,
  `SECONDARY_PAD_BBOX_SCALE = 4.0`, `optional_pad_members(&ModelManifest)`,
  `validate_pad_detector_for(pad, manifest, model_id)` (the old `validate_pad_detector` delegates
  with `PAD_MODEL_ID`), and `attach_optional_pad_members(vision, &mut registry, pad_threshold)`,
  called by `initialize_pipeline`. The manifest is the only switch; the shipped manifest does not
  declare the member, so production stays single-model.

## 3. Tests first (red evidence)

- `crates/inference-ort/tests/embedding_io_contract_tests.rs` with hand-encoded ONNX graphs in
  `tests/fixtures/embedding_onnx.rs` (Flatten + opset-9 Slice to fix the output length, a
  rank-2 input, and an input-less `Constant` graph). With only the API stubbed (constant and an
  `input_layout()` mirroring the old fallback), 3 of 6 failed:
  `test_extract_embedding_rejects_wrong_dimension`, `test_nhwc_detection_fails_closed_without_inputs`,
  `test_embedding_layout_detection_fails_closed_on_ambiguous_input`.
- `tests/invariants/src/model_download_size_contract.rs`: all 3 VMX tests failed (no size
  printed, mis-sized file deployed, invalid `size_bytes` ignored, manifest without sizes).
- `crates/daemon/tests/pad_ensemble_wiring_tests.rs` (own binary, ORT sessions): did not compile
  (`attach_optional_pad_members`, `optional_pad_members`, `SECONDARY_PAD_*` missing).

## 4. Audit

- No `unwrap` / `expect` in production code; every new path returns a typed error.
- The layout fallback and the dimension check both fail closed; no error becomes a pass.
- The optional PAD member is loaded through the attested registry (SHA-256 + shape checks),
  uses `build_pad_detector` (live class index never overridden, invariant
  `test_no_pad_live_class_index_override_outside_tests` unaffected) and the PAD self-test; an
  attested member that fails refuses to start.
- `download_models.sh`: `size_bytes` is validated before any write, bounded by the size cap,
  and a mismatch removes the staged file (the EXIT trap also covers interruption). The
  `--max-time 900` bound and the SHA-256 flow are unchanged.
- No new logging of frames, embeddings or credentials (only model id and crop scale).

## 5. Implementation

- `crates/inference-ort/src/embedding.rs`: `EMBEDDING_DIMENSION`, `infer_input_layout`, the
  `layout` field, `input_layout()`, fail-closed extraction and the output length check.
- `scripts/download_models.sh`, `models/manifest.toml`: `size_bytes` parsing, printing, per-model
  curl bound and exact-size check.
- `crates/daemon/src/pipeline.rs`: optional member constants and wiring.

## 6. Out of scope / follow-ups

- #212 stays partial: the 4.0x MiniFASNetV1SE ONNX export must be sourced, downloaded, hashed and
  shape-checked on a host with network access, then added to `models/manifest.toml` together with
  a coordinated update of `manifest_tests::test_manifest_v2_model_count_and_checksum_attestation`
  (tests are never weakened), the fused threshold re-measured on the PAD-06 corpus, and
  `soos-enroll` / `soos-gui` given the same `attach_optional_pad_members` wiring (row VMX8).
- A per-model download time limit scaled by `size_bytes` was not added: `--max-time 900` stays
  the single bound.

## 7. Validation

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast`,
`./scripts/candid_review.sh`, `bash -n scripts/download_models.sh`.
