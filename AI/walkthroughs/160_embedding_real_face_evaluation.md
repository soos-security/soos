# Walkthrough 160 — Real-Face Embedding Evaluation on LFW

- **Date**: 2026-10-01
- **Issue**: GitHub #278 ([STO-FU] fourth item: evaluate a licence-compatible lighter embedding
  model and recalibrate `match_threshold` / BGR order / normalization for the shipped ArcFace
  ResNet34; #191 ADR follow-up)
- **Branch**: `test/embedding-real-face-evaluation`
- **Matrix criteria**: EVR1–EVR6 (new, ✅ Verified), EVR7 (new, ⏳ Pending: owner decision)
- **ADR**: 2026-10-01 "Real-Face Embedding Evaluation and Recalibration Proposal"
  (**Proposed — awaiting owner decision**) in `AI/DECISIONS.md`

---

## 1. Context

Issue #278 has four items. The first three are on `main`: the GDM gate de-duplication with
jumps and the non-blocking `O_NONBLOCK` include reads (#284, walkthrough 127, matrix
SFU1–SFU3) and the `.opaque.enc` evidence suffix (#284/#286, matrix SFU4,
`OPAQUE_SNAPSHOT_EXTENSION` in `crates/evidence-store/src/store.rs`). Walkthrough 145 covered
the synthetic part of the fourth item (SFX1–SFX5): the attested `arcface_w600k_mbf.onnx` has no
in-graph normalization, its model card documents RGB and `(x - 127.5) / 128`, the divisor is
template-neutral and the channel order is not. The owner then decided to record the licence as
`NOASSERTION` and to **keep BGR until real faces are measured; no re-enrollment** (SFX4 stayed
pending). This walkthrough supplies the real-face measurement, the threshold calibration and the
licence survey. No production default changes.

## 2. Harness

`crates/vision/tests/embedding_lfw_evaluation_tests.rs`, ignored test
`test_lfw_real_face_evaluation_report`. It runs the production path image by image:

1. bounded read (1 MiB) and the production MJPEG decoder (`convert_to_rgb`, `PixelFormat::Mjpeg`);
2. `OrtScrfdDetector` with `DEFAULT_MIN_FACE_CONFIDENCE = 0.70` and `DEFAULT_NMS_IOU_THRESHOLD = 0.45`;
3. LFW face selection: the confident detection closest to the image centre (production rejects a
   multi-face frame; those images are counted, see §3);
4. `align_face_112` (5-point similarity warp to the ArcFace template);
5. one embedding per variant: the production `OrtEmbeddingExtractor` (BGR, `/127.5`); the same
   extractor on an R/B-swapped crop (RGB, `/127.5`, the InsightFace `arcface_onnx.py`
   convention: `input_mean = input_std = 127.5`, `swapRB=True`); raw session runs for `/128`
   (model card) in both orders; and the OpenCV Zoo SFace candidate (RGB and BGR, raw 0..255, NCHW,
   the `FaceRecognizerSF::feature` pre-processing).

Every model goes through `ModelRegistry` and `verify_integrity` (SHA-256 and I/O shapes). The
production models come from `/var/lib/soos/models` against the committed manifest (SHA-256
verified: `scrfd 500m` `a3562ef6...`, ArcFace `ffe014a4...`, MiniFASNet `0cbe5cae...`). The
candidate comes from a manifest outside the repository.

Two protocols: **official** (the 6000 `pairs.txt` pairs, 10-fold accuracy with the threshold
chosen on the other nine folds; a pair with a failed image scores `-1`, i.e. never matches) and
**extended** (every pair of the 7646 embedded images, labelled by LFW identity: 7961 genuine and
29,218,874 impostor pairs, which resolves FAR down to about 1e-7; the official 3000 impostor
pairs cannot resolve 1e-4).

Data: `scripts/fetch_lfw_eval.sh` downloads `lfw.tgz` and `pairs.txt` from the figshare mirror
pinned by scikit-learn (`sklearn/datasets/_lfw.py`), checks the SHA-256 digests
(`055f7d9c...d536c0`, `ea42330c...dc1592`), bounds the transfer and refuses a cache inside the
repository. Images, crops, embeddings and per-pair scores live only in memory (wipe-on-drop
containers) and only aggregates are printed. Reproduce:

```text
scripts/fetch_lfw_eval.sh
SOOS_EVAL_LFW_DIR=$HOME/.cache/soos-eval/lfw SOOS_EVAL_LFW_PAIRS=$HOME/.cache/soos-eval/pairs.txt \
SOOS_MODELS_DIR=/var/lib/soos/models SOOS_EVAL_CANDIDATE_DIR=$HOME/.cache/soos-eval/candidates/sface \
SOOS_EVAL_ORT_THREADS=4 cargo test --release --locked -p soos-vision \
  --test embedding_lfw_evaluation_tests -- --ignored --nocapture test_lfw_real_face_evaluation_report
```

The candidate directory holds `sface_2021dec.onnx` (Hugging Face `opencv/face_recognition_sface`
revision `3d70824`, 38,696,353 bytes, SHA-256 `0ba9fbfa...4c34e79`, verified) and a
`manifest.toml` entry `sface_2021dec` (`input_shape = [1, 3, 112, 112]`, `NCHW`,
`output_shapes = [[1, 128]]`).

## 3. Results (LFW, this host, 2026-10-01)

Detection over the 7701 distinct images of `pairs.txt`: 0 decode failures, **55 failures to
detect** (0.71 %), 0 missing landmarks, 0 alignment failures, 670 images (8.7 %) with more than
one confident face (production would reject those frames with `MultipleFacesDetected`), 0 faces
below `DEFAULT_MIN_FACE_WIDTH_PX = 48`.

| Variant | 10-fold acc. | TAR@FAR 1e-2 (thr) | TAR@FAR 1e-3 (thr) | TAR@FAR 1e-4 (thr) | TAR@FAR 1e-5 (thr) | TAR / FAR at 0.70 |
|---|---|---|---|---|---|---|
| ArcFace BGR /127.5 **(production)** | 0.9668 ± 0.0114 | 0.945 (0.303) | 0.836 (0.402) | 0.666 (0.485) | 0.446 (0.565) | 0.117 / 1.0e-6 |
| ArcFace RGB /127.5 (InsightFace convention) | **0.9738 ± 0.0100** | 0.962 (0.295) | 0.886 (0.394) | 0.746 (0.476) | 0.573 (0.543) | 0.152 / 1.0e-7 |
| ArcFace RGB /128 (model card) | 0.9730 ± 0.0101 | 0.962 (0.295) | 0.886 (0.394) | 0.745 (0.476) | 0.573 (0.543) | 0.152 / 6.8e-8 |
| ArcFace BGR /128 | 0.9680 ± 0.0123 | 0.945 (0.303) | 0.836 (0.402) | 0.666 (0.485) | 0.446 (0.565) | 0.117 / 9.9e-7 |
| SFace RGB raw (candidate, OpenCV recipe) | **0.9848 ± 0.0070** | 0.995 (0.302) | 0.991 (0.369) | 0.984 (0.424) | 0.968 (0.476) | 0.470 / 3.8e-7 |
| SFace BGR raw (candidate) | 0.9838 ± 0.0057 | 0.993 (0.297) | 0.987 (0.367) | 0.973 (0.425) | 0.943 (0.477) | 0.323 / 2.7e-7 |

TAR / thresholds in the FAR columns are from the extended protocol. The official protocol
agrees where it can resolve: production BGR TAR@1e-3 = 0.878 (threshold 0.362), RGB 0.907
(0.359), SFace RGB 0.971 (0.371); at 0.70 no official impostor pair matches for any variant.

Score distributions (extended protocol, mean ± std):

| Variant | Genuine | Impostor |
|---|---|---|
| ArcFace BGR /127.5 (production) | 0.538 ± 0.141 (p1 0.164, p50 0.545, p99 0.831) | 0.053 ± 0.098 (p99 0.303) |
| ArcFace RGB /127.5 | 0.560 ± 0.139 (p1 0.187, p50 0.570, p99 0.843) | 0.045 ± 0.098 (p99 0.295) |
| SFace RGB raw | 0.684 ± 0.104 (p1 0.385, p50 0.692, p99 0.883) | 0.101 ± 0.087 (p99 0.302) |

Fixed thresholds (extended protocol):

| Threshold | ArcFace BGR (production) FAR / TAR | ArcFace RGB /127.5 FAR / TAR | SFace RGB FAR / TAR |
|---|---|---|---|
| 0.40 (policy floor) | 1.06e-3 / 0.839 | 8.6e-4 / 0.876 | 2.8e-4 / 0.988 |
| 0.45 | 2.8e-4 / 0.749 | 2.2e-4 / 0.800 | 3.1e-5 / 0.979 |
| 0.50 | 6.4e-5 / 0.629 | 4.6e-5 / 0.688 | 3.9e-6 / 0.957 |
| 0.55 | 1.5e-5 / 0.487 | 7.6e-6 / 0.553 | 1.1e-6 / 0.910 |
| 0.60 | 4.4e-6 / 0.344 | 1.1e-6 / 0.411 | 8.6e-7 / 0.821 |
| 0.65 | 2.0e-6 / 0.217 | 2.1e-7 / 0.269 | 5.8e-7 / 0.671 |
| 0.70 (production default) | 1.0e-6 / 0.117 | 1.0e-7 / 0.152 | 3.8e-7 / 0.470 |

The `/128` variants match their `/127.5` counterparts to within 0.001. The run is deterministic:
a second full run gave identical figures, and its wall time was 1495 s against 2095 s for the
first (shared host). The few impostor pairs above 0.6 for SFace are consistent with LFW's known
duplicate and mislabelled images.

Latency per image on this host (16 logical CPUs shared with other builds, ORT 4 intra-op
threads, release build): SCRFD detection p50 25.1 ms / p95 41.4 ms; alignment p50 0.5 ms;
ArcFace embedding p50 35.3 ms / p95 77.1 ms (identical for every pre-processing variant);
SFace embedding p50 10.3 ms / p95 21.0 ms.

### Reading

1. **Channel order.** RGB is better than BGR on real faces on every metric (+0.7 points of
   accuracy, +5.0 points of TAR at FAR 1e-3, +8.0 points at 1e-4). This confirms the model card
   and the InsightFace convention: the network was trained on RGB and the production extractor
   feeds it swapped channels (walkthrough 71 assumed an InsightFace model).
2. **Divisor.** `/127.5` and `/128` are indistinguishable (≤ 0.001 on every metric), as the
   synthetic evidence of walkthrough 145 predicted. No change is needed.
3. **Threshold.** `match_threshold = 0.70` sits at an impostor rate of about 1e-6 to 1e-7 and
   accepts only 12 % (BGR) to 15 % (RGB) of LFW genuine pairs. The documented target is
   FAR ≤ 0.1 % per comparison (`Docs/POLICY_CRATE.md`, "Sourced from MobileFaceNet literature
   (FAR <= 0.1%)"); for this network that point is at a cosine of about **0.40** (BGR 0.402,
   RGB 0.394), i.e. at the policy floor `ThresholdConfig::MIN_MATCH_THRESHOLD = 0.40`. The 0.70
   default was never calibrated for this ResNet34; it does not weaken security, it costs
   usability (LFW pairs are harder than same-camera captures, so the real FRR is lower than 85 %
   but unmeasured).
4. **Candidate.** SFace is better than the shipped ArcFace on every metric (TAR 0.984 vs 0.746 at
   FAR 1e-4 against the best ArcFace variant), 3.5 times smaller (38.7 MB vs 136.6 MB) and about
   3.4 times faster, with the same alignment template. Its training data is undocumented (§4).

### Limits

LFW is web photographs of public figures, not same-camera captures; impostors are random
celebrities, not look-alikes or presentation attacks. Thresholds derived here bound the
zero-effort FAR of one comparison; the daemon additionally needs `DEFAULT_PAD_CONSENSUS_REQUIRED
= 3` consecutive matching live captures of the same session, whose errors are correlated, so no
multiplication of FARs is claimed. LFW labels contain a few known errors. IR (`Grey`) captures
are not covered. The decision still needs a check on the owner's real enrollment captures
(genuine scores of the camera in use), which is listed in §6.

## 4. Licence survey (weights, not code)

| Candidate | Weight licence | Training data | ONNX | Size | Reported accuracy | Verdict |
|---|---|---|---|---|---|---|
| OpenCV Zoo SFace `face_recognition_sface_2021dec.onnx` | Apache-2.0 ("All files are licensed under Apache 2.0 License", zoo README and LICENSE; HF mirror tagged Apache-2.0) | **Undocumented** for this file; the SFace paper trains on CASIA-WebFace, VGGFace2 and MS1MV2; questions in opencv_zoo issues #124, #313, #318 and zhongyy/SFace #9 are unanswered | yes (direct, pinned) | 38.7 MB fp32 (9.9 MB int8) | 0.9940 (zoo eval); **0.9848 measured here** with SCRFD + soos alignment | Best available; data lineage risk to record |
| OpenVINO `face-reidentification-retail-0095` | Apache-2.0 (Open Model Zoo licence via `model.yml`) | Undocumented | no (OpenVINO IR; would need a conversion) | 4.4 MB | LFW 0.9947 | Fallback, conversion step |
| dlib `dlib_face_recognition_resnet_model_v1` | Public domain (Davis King, dlib-models README) | ~3 M faces incl. FaceScrub and VGG (research terms) | no (dlib `.dat`, no maintained ONNX) | ~22 MB | LFW 0.9938 | Conversion and data risk |
| EdgeFace (Idiap) | CC-BY-NC-SA-4.0 | — | no (`.pt`) | 14.7 MB | — | Excluded (non-commercial) |
| AdaFace, cvlface AdaFace IR50 | MIT code, no weight licence | MS1M / WebFace4M (research only) | no | — | — | Excluded |
| GhostFaceNets | MIT | MS1MV2 / MS1MV3 (MS-Celeb-1M withdrawn 2019) | no (Keras) | — | — | Excluded |
| facenet-pytorch (VGGFace2 / CASIA weights) | MIT | VGGFace2 / CASIA-WebFace (non-commercial) | no | — | — | Excluded |
| InsightFace `buffalo_*` / `w600k_*` | non-commercial research only (InsightFace README) | WebFace600K | yes | — | — | Excluded |
| Shipped `garavv/arcface-onnx` `arc.onnx` | none declared (`NOASSERTION`) | undeclared | yes | 136.6 MB | 0.9738 measured (RGB) | Current |

Sources: https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface,
https://raw.githubusercontent.com/opencv/opencv_zoo/main/models/face_recognition_sface/LICENSE,
https://huggingface.co/opencv/face_recognition_sface (revision `3d7082438a6e4551e840c9b2bb60b71e8da4b524`),
https://arxiv.org/abs/2205.12010, https://github.com/opencv/opencv_zoo/issues/318,
https://github.com/zhongyy/SFace/issues/9,
https://raw.githubusercontent.com/opencv/opencv/4.x/modules/objdetect/src/face_recognize.cpp
(`blobFromImage(..., 1, Size(112, 112), Scalar(0, 0, 0), true, false)`),
https://raw.githubusercontent.com/openvinotoolkit/open_model_zoo/master/models/intel/face-reidentification-retail-0095/model.yml,
https://github.com/davisking/dlib-models, https://huggingface.co/Idiap/EdgeFace-S-GAMMA,
https://github.com/mk-minchul/AdaFace, https://github.com/HamadYA/GhostFaceNets,
https://github.com/timesler/facenet-pytorch,
https://github.com/deepinsight/insightface/blob/master/python-package/README.md (model licence),
https://github.com/deepinsight/insightface/blob/master/python-package/insightface/model_zoo/arcface_onnx.py
(`input_mean = input_std = 127.5`, `swapRB=True`),
https://github.com/scikit-learn/scikit-learn/blob/main/sklearn/datasets/_lfw.py (LFW mirror and digests).

No candidate is clean on both the weight licence and the training data. SFace is the only one
with a permissive licence on the ONNX file itself and a direct download; its data lineage is no
better documented than the shipped model's, and its licence is strictly better (Apache-2.0 vs
none). Apache-2.0 is compatible with AGPL-3.0-or-later.

## 5. Why no production change

The owner decision of 2026-10-01 keeps BGR and forbids a silent re-enrollment. The BGR order and
the `/127.5` divisor are pinned by the pre-existing contract tests
`embedding_tests::test_arcface_input_bgr_ordering`,
`embedding_tests::test_prepare_input_layout_nhwc_and_nchw` and
`embedding_tests::test_embedding_normalization_symmetric_range`, and `match_threshold = 0.70` by
the threshold parity tests; changing any of them needs approval. The exact proposal (code,
configuration, `model_id` and re-enrollment impact) is the ADR "Real-Face Embedding Evaluation and
Recalibration Proposal".

## 6. Changes

| File | Change |
|---|---|
| `crates/vision/tests/embedding_lfw_evaluation_tests.rs` | New harness: ignored LFW evaluation plus four regular tests (env refusal, in-repo data refusal, `pairs.txt` parser bounds, metric arithmetic) |
| `crates/vision/Cargo.toml`, `Cargo.lock` | Dev-dependencies `ort` and `tempfile` (workspace versions; no new package) |
| `scripts/fetch_lfw_eval.sh` | New bounded, SHA-256 verified LFW downloader that refuses a cache inside the repository |
| `tests/invariants/src/embedding_evaluation_contract.rs`, `tests/invariants/src/lib.rs` | New invariants: the evaluation stays ignored and env-gated, the downloader refuses an in-repo cache without creating it, no LFW data in the source tree |
| `AI/DECISIONS.md` | New ADR (Proposed — awaiting owner decision) |
| `AI/VERIFICATION_MATRIX.md` | New component `embedding-real-face-evaluation`, rows EVR1–EVR7, after the SFX rows |
| `Docs/VISION_CRATE.md`, `Docs/INFERENCE_ORT_CRATE.md` | Harness and results sections |

## 7. Red → Green

This is an evaluation: no production behaviour changes, nothing turns red. The four regular
harness tests and the three invariants pass in normal CI without data or network. The ignored
evaluation passed on this host in 2095 s (`1 passed`).

## 8. Security Notes

- No image, crop, embedding, per-image or per-pair score is written or printed; only aggregates.
  Crops and embeddings are held in `Zeroizing` containers; raw ORT outputs go through
  `ZeroizingOutputs`.
- The dataset and the candidate model live in `~/.cache/soos-eval`, outside the repository;
  the harness and the downloader refuse an in-repository location and an invariant scans the
  source tree for LFW artefacts.
- Every model is attested (SHA-256 and shapes) before use; downloads are HTTPS-only, size- and
  time-bounded and digest-verified.

## 9. Open questions for the owner

1. Switch the ArcFace input to RGB (approved change of the pinned BGR contract tests) with a new
   template binding and re-enrollment of every user?
2. Lower `match_threshold` from 0.70 to the proposed value after checking genuine scores on your
   own enrollment captures?
3. Replace the model with SFace (Apache-2.0 file, undocumented training data) under a new
   manifest id, or keep the `NOASSERTION` ArcFace?

## Integration note (batch merge)

On the combined batch, `cargo test -- --include-ignored` (without the LFW variables) failed on
`test_lfw_real_face_evaluation_report`, which panicked on missing variables while every other
env-gated real-hardware test skips. The report now prints `SKIPPED ...: <VAR> not set` and returns
when a variable is unset, and still refuses (panics) when a variable is set but unusable (for
example data inside the repository); `test_lfw_harness_refuses_to_run_without_env_vars` and
`test_lfw_harness_refuses_data_inside_the_repository` keep pinning the configuration refusal.
