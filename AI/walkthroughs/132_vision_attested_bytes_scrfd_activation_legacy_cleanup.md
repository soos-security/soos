# Walkthrough 132 — Attested Model Bytes, Explicit SCRFD Score Activation, Legacy Cleanup

- **Date**: 2026-09-30
- **Issues**: GitHub #246 (VIS-04, checksum-then-load TOCTOU and double hashing), #247 (VIS-05,
  non-monotonic SCRFD score activation), #249 (VIS-07, legacy 4-model code, unused dependency,
  legacy download URLs)
- **Branch**: `fix/p2-vision-toctou-scrfd-legacy`
- **Matrix criteria**: VTS1–VTS8 (new)
- **ADR**: 2026-09-30 "Attested Model Bytes and Explicit SCRFD Score Activation"

---

## 1. Context

1. **VIS-04.** `ModelRegistry::get_or_load_session` hashed a model by path
   (`verify_model_checksum`) and then re-opened the same path with `commit_from_file`, so the
   bytes ONNX Runtime loaded were not provably the bytes that were hashed. The daemon also called
   `verify_integrity` and then `get_or_load_session`, which hashed every model twice.
2. **VIS-05.** `OrtScrfdDetector::decode_stride` passed a raw score through when it lay in
   `[0, 1]` and applied a sigmoid otherwise, per element. The mapping is non-monotonic across
   the boundary (`1.0 -> 1.0` but `1.01 -> 0.733`; `0.0 -> 0.0` but `-0.01 -> 0.4975`).
3. **VIS-07.** The UltraFace detector, landmark detector types, the `OrtScrfdDetector`
   `unproject` / `letterbox_pad` wrappers and `validate_output_shapes` had no production caller;
   `ndarray` was an unused dependency; `scripts/download_models.sh` still resolved four legacy
   model URLs; `OrtScrfdDetector::new` validated only the output count although the docs claimed
   shape validation.

## 2. Change

### 2.1 Hashed bytes are loaded bytes (#246)

- `crates/inference-ort/src/manifest.rs`: `MAX_MODEL_FILE_BYTES` (512 MiB), `sha256_hex`,
  `read_verified_model` and `read_verified_model_with_limit`. The file is opened once; type and
  size are checked on the opened handle; at most `max_bytes + 1` bytes are read (a file growing
  after `metadata()` is still rejected); the digest is computed over the returned buffer.
  Missing or non-regular file -> `ModelNotFound`; oversized -> `ModelIo`; wrong digest ->
  `ChecksumMismatch`.
- `crates/inference-ort/src/registry.rs`: `verify_integrity` reads and hashes every manifest
  model (sorted ids) and retains the verified bytes; `get_or_load_session` takes the retained
  bytes (or does one verified read) and calls `commit_from_memory(&bytes)`, then drops the buffer.
  A failed integrity check retains nothing. `pending_verified_models()` exposes the retained
  count. `commit_from_file` no longer appears in the crate.
- Caching only the digest (the issue's second suggestion) was rejected: skipping the hash at
  load while re-reading the file would reopen the window the fix closes.

### 2.2 Explicit SCRFD score activation (#247)

- `crates/inference-ort/src/detector.rs`: `ScoreActivation { Probability (default), Logit }`
  with `infer` (one decision per tensor), `validate` (fail closed) and `apply`.
  `OrtScrfdDetector` gains `score_activation` and `with_score_activation`. `detect` now calls
  `decode_stride_checked`, which rejects the frame with `TensorError` on any non-finite or
  out-of-range probability (or non-finite logit).
- The public `decode_stride` keeps its signature because pre-existing contract tests
  (`scrfd_tests::test_scrfd_decode_stride8_known_output`, `test_scrfd_decode_all_strides`) feed
  logits and `test_scrfd_supports_preactivated_probabilities` feeds probabilities. It now infers
  one activation for the whole tensor, which is monotonic. Production never calls it
  (`vision_attestation_contract::test_scrfd_detect_uses_explicit_fail_closed_activation`).

### 2.3 Startup shape validation and legacy cleanup (#249)

- `OrtScrfdDetector::validate_output_dims` validates the session output metadata in `new`.
  The real `scrfd_500m_kps.onnx` reports `[-1, -1, 1] x3, [-1, -1, 4] x3, [-1, -1, 10] x3`
  (printed by `scrfd_real_model_tests::test_real_scrfd_session_passes_startup_shape_validation`):
  the anchor dims are symbolic, so a strict per-stride check at startup would have rejected the
  shipped model. The check therefore enforces what the metadata can prove (9 rank-3 outputs,
  batch 1 or symbolic, 3 heads each of 1/4/10 channels, concrete anchor dims in
  {12800, 3200, 800} without duplicates); exact per-stride anchor counts remain enforced on every
  frame by `detect`.
- Removed: the UltraFace inference path of `OrtFaceDetector` (`new`, `generate_priors`, the
  `FaceDetector` impl), `OrtScrfdDetector::unproject` / `letterbox_pad` wrappers, the `ndarray`
  dependency (crate and workspace), and the four legacy cases of `resolve_download_url` in
  `scripts/download_models.sh`. `ndarray` remains in `Cargo.lock` as a transitive dependency of
  `ort`.
- Kept (pre-existing tests use them, see §4): `OrtFaceDetector::prepare_input` (now on a unit
  struct documented as legacy preprocessing), `LandmarkDetector`, `MockLandmarkDetector`.

## 3. Red evidence

1. Before any production change, the three new test targets failed to compile: missing
   `ScoreActivation`, `MAX_MODEL_FILE_BYTES`, `sha256_hex`, `read_verified_model(_with_limit)`,
   `pending_verified_models`, `decode_stride_checked`, `validate_output_dims`,
   `with_score_activation`, field `score_activation`.
2. Behavioural red, by reverting each fix alone on the finished code:
   - registry forced to ignore the retained bytes (re-read and re-hash from disk):
     `test_registry_loads_bytes_verified_by_integrity_check_without_rehash` and
     `test_registry_verified_bytes_are_single_use` FAIL;
   - per-element activation restored in the decoder:
     `test_scrfd_decode_stride_is_monotonic_across_unit_boundary` and
     `test_scrfd_logit_activation_is_monotonic_and_rejects_non_finite` FAIL;
   - shape validation removed from `new`: `test_scrfd_new_rejects_nine_outputs_with_wrong_shapes`
     FAILS.
3. A first version of `validate_output_dims` that required concrete anchor dims failed both
   real-model tests on the attested SCRFD file; the check was corrected (§2.3) and the new test
   updated before commit.

## 4. Not changed: pre-existing tests (proposals awaiting user approval)

Complete removal of the legacy types requires changing tests that exist on `origin/main`, which
the zero-test-weakening rule forbids without approval:

1. `crates/inference-ort/tests/zeroize_tests.rs::test_inference_input_buffers_zeroized`, block
   `// 1. OrtFaceDetector input buffer` (lines 34-37 onward, calling
   `OrtFaceDetector::prepare_input(&dummy_rgb, 320, 240)`): re-point it to
   `OrtScrfdDetector::prepare_input(&dummy_rgb, 320, 240)` (the SCRFD block of the same test
   already covers that path, so the block could also be deleted), and drop `OrtFaceDetector`
   from the `use` list. Matrix NGM8 cites this test, so it must be re-pointed, not deleted.
   Then delete `OrtFaceDetector`.
2. `crates/inference-ort/tests/landmark_tests.rs::test_mock_landmark_detector_scaling` and
   `crates/inference-ort/tests/landmarks_tests.rs::test_landmark_detector_trait_mock_dispatch`
   exercise only `MockLandmarkDetector` / `LandmarkDetector`; delete those two tests (the
   `FaceLandmarks` / `Point2f` geometry tests in both files stay), then delete
   `LandmarkDetector` and `MockLandmarkDetector`.

## 5. Verification

- `cargo fmt --all -- --check`: pass.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: pass.
- `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`: pass (152 test
  binaries, 0 failures), including both SCRFD real-model tests against `/var/lib/soos/models`.
- `./scripts/candid_review.sh`: PASSED. `bash -n scripts/download_models.sh`: pass.

## 6. Documentation

- `Docs/INFERENCE_ORT_CRATE.md`: invariant 3a (hashed bytes are loaded bytes, memory cost),
  API listing, `OrtFaceDetector` status, SCRFD startup validation and score activation.
- `models/README.md` §5: legacy ids are historical only and no longer resolvable.
- `AI/DECISIONS.md`: ADR 2026-09-30; `AI/VERIFICATION_MATRIX.md`: VTS1–VTS8.
