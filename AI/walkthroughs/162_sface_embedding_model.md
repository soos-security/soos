# Walkthrough 162 — SFace Embedding Model and `match_threshold = 0.50`

- **Date**: 2026-10-01
- **Issue**: GitHub #278 (owner decisions of 2026-10-01 on the ADR "Real-Face Embedding
  Evaluation and Recalibration Proposal", walkthrough 160). GitHub-only follow-up: the branch is
  not registered in `BRANCH_TO_ISSUE`; commits carry `Refs #278`.
- **Branch**: `feat/sface-embedding-model`
- **Base commit**: `11c967e`
- **Matrix criteria**: SFC1–SFC18 (new section `sface-embedding-model` right after the EVR
  section of walkthrough 160); EVR7 now `✅ Verified`; NGM7 and SFX2 `⏹ Superseded`; BIO1, NGM16,
  EMP4, EMR3, EMR5, EMR6 and VMX4 updated to the migrated test names
- **ADR**: 2026-10-01 "SFace Embedding Model Replaces ArcFace ResNet34" (new); "Real-Face
  Embedding Evaluation and Recalibration Proposal" marked Accepted; the alias part of
  2026-09-30 "Embedding Model Binding and Legacy Model Alias" superseded
- **Status**: phases 1, 1.5, 2, 3, 4 and 6 done. Section 4 was approved by the owner on
  2026-10-01 (Q1) together with the recommended answers Q2–Q9 and Q11 (section 8). One further
  pre-existing test that the switch breaks was found during Phase 4 and was **not** edited
  (section 11, F6): it awaits owner approval. Phases 5 (candid review) and 7 (release) are not
  run in this batch.

---

## 1. Context and Objectives

The owner decided on 2026-10-01 (GitHub #278 comment):

1. Adopt **SFace** (OpenCV Zoo `face_recognition_sface_2021dec.onnx`, Apache-2.0 model file,
   pinned Hugging Face revision) as the embedding model under a new manifest id, **replacing**
   the `NOASSERTION` ArcFace ResNet34 (`arcface_w600k_mbf`). Existing templates become
   `Foreign`: the daemon answers `Unavailable`, PAM returns `PAM_IGNORE` and the password module
   runs, until each user re-enrolls.
2. Default `match_threshold` = **0.50** (LFW extended protocol, SCRFD + soos alignment: SFace
   FAR 3.9e-6, TAR 0.957; target FAR <= 1e-3).

Objectives of the change:

- attest the SFace file in `models/manifest.toml` and remove the ArcFace entry;
- make the embedding extractor model-aware (one spec per model: id, dimension, layout,
  pre-processing) instead of the hard 512-D contract of GitHub #268;
- bind templates to the model id **and** the dimension, so no ArcFace vector is ever compared
  with an SFace probe (daemon, `soos-enroll verify`, `soos-gui` live verification);
- move the default `match_threshold` from 0.70 to 0.50 in its two constants and remove the
  duplicated literal in the GUI;
- keep `soos-enroll import` and `soos-enroll migrate` from producing or "upgrading" templates
  that the loaded model cannot use;
- accept the Proposed ADR and record the decision in a new ADR.

## 2. Verified Facts About the Model File

Every fact below was measured on the real file (cache of the evaluation,
`~/.cache/soos-eval/candidates/sface/sface_2021dec.onnx`) with `onnx` 1.17.0 and
`onnxruntime` 1.20.1, and on the pinned URL (HTTP headers), on 2026-10-01.

| Fact | Value | Evidence |
|---|---|---|
| Source URL (pinned revision) | `https://huggingface.co/opencv/face_recognition_sface/resolve/3d7082438a6e4551e840c9b2bb60b71e8da4b524/face_recognition_sface_2021dec.onnx` | HTTP 302 then 200; `x-linked-etag` equals the SHA-256 |
| SHA-256 | `0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79` | `sha256sum`, `x-linked-etag` |
| Size | 38,696,353 bytes | `ls`, `x-linked-size`, `content-length` |
| Licence | Apache-2.0 (OpenCV Zoo README and `LICENSE`, Hugging Face tag) | walkthrough 160 §4 |
| Graph | IR 6, opset 11, 88 nodes, 9,667,074 parameters | `onnx.load` |
| Input | `data`, float32 `[1, 3, 112, 112]` (**NCHW**, batch fixed to 1) | `graph.input`, `session.get_inputs()` |
| Output | `fc1`, float32 `[1, 128]` (raw, L2 norm about 5.1 on random input; **not** normalized in the graph) | `session.run` |
| In-graph normalization | first nodes `Sub(data, 127.5)` then `Mul(·, 0.0078125)`: the graph computes `(x - 127.5) / 128` itself | initializers `scalar_op1 = 127.5`, `scalar_op2 = 0.0078125` |
| Graph-input initializers | 175 `graph.input` entries, 174 of them initializers (IR 6 export); ORT reports only `data` as a session input and warns once per initializer at session creation | ORT warning `Initializer ... appears in graph inputs` |
| Batch 2 | rejected (`index: 0 Got: 2 Expected: 1`) | `session.run` |

OpenCV pre-processing (`modules/objdetect/src/face_recognize.cpp`, 4.x):
`alignCrop` warps to 112x112 with the destination landmarks
`{38.2946, 51.6963}, {73.5318, 51.5014}, {56.0252, 71.7366}, {41.5493, 92.3655}, {70.7299, 92.2041}`
(identical to `soos_vision::align::TARGET_LANDMARKS_112`), then
`blobFromImage(aligned, 1, Size(112, 112), Scalar(0, 0, 0), true, false)`: scale factor 1, no
mean, `swapRB = true` on an OpenCV BGR image, i.e. **RGB, raw 0..255, NCHW**. `match` L2-normalizes
both features before the cosine. This is the "SFace RGB raw" variant that the evaluation measured
best (10-fold accuracy 0.9848, against 0.9838 for BGR).

Consequences for soos: the aligned crop produced by `align_face_112` (RGB24, 112x112) is fed
unchanged as three planes R, G, B of raw `f32` values in `[0, 255]`; the extractor applies no
mean, no divisor; it L2-normalizes the 128-D output. Alignment needs no change.

## 3. Architect Spec

### 3.1 Scope and Blast Radius

| Area | Files | Change |
|---|---|---|
| Model manifest | `models/manifest.toml`, `models/README.md` | new `[models.sface_2021dec]`, ArcFace entry removed (see 3.9 for its retained lineage) |
| Download | `scripts/download_models.sh` | no code change (manifest-driven); comment on the largest model; optional orphan notice (owner question Q8) |
| Extractor | `crates/inference-ort/src/embedding.rs`, `src/lib.rs`, `src/mock.rs` | `EmbeddingModelSpec`, model-aware `OrtEmbeddingExtractor`, RGB raw NCHW input, `output_dimension()` |
| Vision | `crates/vision/src/pipeline.rs`, `src/lib.rs`, `src/align.rs` (docs) | `DEFAULT_MATCH_THRESHOLD = 0.50`, `VisionPipeline::embedding_dimension()` |
| Policy | `crates/policy/src/threshold.rs` | `ThresholdConfig::DEFAULT_MATCH_THRESHOLD = 0.50`, comment |
| Daemon | `crates/daemon/src/pipeline.rs`, `src/dispatcher.rs`, `src/main.rs`, `src/inference.rs` (comment) | `EMBEDDING_MODEL_ID` from the spec, legacy alias removed, dimension binding |
| Enrollment CLI | `crates/enrollment-cli/src/service.rs`, `src/args.rs`, `src/main.rs`, `src/guided_enrollment.rs` (comment) | ids from the spec, import bound to the loaded model, verify binding, mock dimension, re-enrollment messages |
| GUI | `crates/gui/src/app.rs`, `src/main.rs`, `src/worker.rs` | threshold literal removed, reference template binding, mock dimension, labels |
| Tests, fixtures, scripts | see section 4 | contract migration (owner approval) and new SFC tests |
| Docs | `Docs/INFERENCE_ORT_CRATE.md`, `Docs/VISION_CRATE.md`, `Docs/POLICY_CRATE.md`, `Docs/BIOMETRIC_STORE_CRATE.md`, `Docs/DAEMON.md`, `Docs/MEMORY_PROTECTION_AND_SWAP.md`, `AI/ARCHITECTURE.md`, `AI/MOCK_STRATEGY.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `.agents/skills/dev-workflow/references/project-facts.md`, `tests/physical/screensaver_test.md` | values and model facts |

No new crate, no new dependency, no new module (the spec lives in `embedding.rs`, so the
module-list invariant of `Docs/INFERENCE_ORT_CRATE.md` is unaffected). `crates/pam`,
`crates/protocol`, `crates/camera-v4l`, `crates/biometric-store` and `crates/evidence-store`
are not touched: the PAM module already maps `Verdict::Unavailable` to `PAM_IGNORE`.

### 3.2 Manifest Entry

```toml
[models.sface_2021dec]
id = "sface_2021dec"
filename = "sface_2021dec.onnx"
sha256 = "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79"
size_bytes = 38696353
# OpenCV Zoo SFace (Apache-2.0 model files); training data undocumented upstream
# (opencv_zoo issue #318), see AI/DECISIONS.md.
license = "Apache-2.0"
source_url = "https://huggingface.co/opencv/face_recognition_sface/resolve/3d7082438a6e4551e840c9b2bb60b71e8da4b524/face_recognition_sface_2021dec.onnx"
description = "OpenCV Zoo SFace 2021dec (MobileFaceNet backbone, SFace loss) 128D face embedding extractor; NCHW data [1,3,112,112], RGB raw 0..255, in-graph (x-127.5)/128"
input_shape = [1, 3, 112, 112]
input_layout = "NCHW"
output_shapes = [[1, 128]]
```

- The id and file name are new stable identifiers (`sface_2021dec`, the name the evaluation used);
  the upstream name stays in `source_url`.
- `[manifest] version` stays `"2.0.0"` (the binding is by model id, so no version bump is needed
  for safety; owner question Q6).
- `ModelRegistry::get_or_load_session` already validates the session against `[1, 3, 112, 112]`
  NCHW and `[[1, 128]]`; the concrete batch dim 1 matches the graph.
- `download_models.sh` takes URL, SHA-256 and size from the manifest; nothing else changes. The
  dry-run invariant (`huggingface.co` must not appear in the script) stays satisfied.

### 3.3 Embedding Model Spec and Model-Aware Extractor (`soos-inference-ort`)

```rust
/// Static contract of an attested embedding model: what the extractor feeds and expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingModelSpec {
    /// Manifest id recorded in every template produced with this model.
    pub model_id: &'static str,
    /// Exact length of the output vector (`[1, dimension]`).
    pub dimension: usize,
    /// Physical layout of the graph input; a session with another layout fails closed.
    pub input_layout: TensorLayout,
}

/// OpenCV Zoo SFace 2021dec: NCHW `data` [1, 3, 112, 112], RGB planes, raw 0..255
/// (the graph applies (x - 127.5) / 128 itself), output `fc1` [1, 128].
pub const SFACE_2021DEC: EmbeddingModelSpec = EmbeddingModelSpec {
    model_id: "sface_2021dec",
    dimension: 128,
    input_layout: TensorLayout::Nchw,
};

/// The embedding model shipped by soos (single source of truth for every binary).
pub const SHIPPED_EMBEDDING_MODEL: EmbeddingModelSpec = SFACE_2021DEC;

/// Output length of the shipped model (kept for existing call sites; equals
/// `SHIPPED_EMBEDDING_MODEL.dimension`, i.e. 128).
pub const EMBEDDING_DIMENSION: usize = SHIPPED_EMBEDDING_MODEL.dimension;
```

- `pub trait EmbeddingExtractor` gains a provided method
  `fn output_dimension(&self) -> Option<usize> { None }`. `OrtEmbeddingExtractor` returns
  `Some(spec.dimension)`, `MockEmbeddingExtractor` returns `Some(self.dim)`. Test doubles that do
  not override it keep compiling (no binding by dimension then).
- `OrtEmbeddingExtractor::new(session)` = `with_spec(session, SHIPPED_EMBEDDING_MODEL)`;
  `pub fn with_spec(session, spec: EmbeddingModelSpec) -> Self`; `pub fn spec(&self)`.
  The inferred layout is kept only if it equals `spec.input_layout`; otherwise `layout` is
  `None` and every extraction fails closed with `InferenceError::TensorError` (an NHWC session is
  no longer silently accepted). `input_layout()` and `is_nhwc()` stay as accessors.
- `pub fn prepare_input(crop_rgb, width, height) -> Result<Zeroizing<Vec<f32>>, InferenceError>`
  writes NCHW planes 0, 1, 2 = R, G, B as raw `f32` in `[0.0, 255.0]` (nearest-neighbour resize to
  112x112 kept; same buffer-size checks, same `InvalidBufferSize` error).
  `prepare_input_layout(.., is_nhwc)` (the BGR `/127.5` path, both layouts) is **removed**: no
  attested model uses it any more.
- `extract_embedding`: output length must equal `spec.dimension`, otherwise
  `InferenceError::DimensionMismatch { expected: spec.dimension, actual }`; the output is
  L2-normalized (existing zero-norm guard). Input tensor in `Zeroizing`, ORT outputs in
  `ZeroizingOutputs` (unchanged).
- `MockEmbeddingExtractor::DEFAULT_DIM` becomes `EMBEDDING_DIMENSION` (owner question Q7); every
  explicit `MockEmbeddingExtractor::new(512)` in tests stays valid.
- `lib.rs` re-exports `EmbeddingModelSpec`, `SFACE_2021DEC`, `SHIPPED_EMBEDDING_MODEL`,
  `EMBEDDING_DIMENSION`, and the module doc names SFace instead of ArcFace.

Edge semantics: `dimension` is never 0 for an attested spec (compile-time constant; a test pins
128); a crop of any size other than `width * height * 3` is refused; an all-zero crop is valid
input (raw zeros), a zero-norm output is refused (`EmbeddingFailed`).

### 3.4 Template Binding (daemon, `soos-enroll verify`, `soos-gui`)

Rule: a template is `Current` only when `template.model_id == loaded model id` **and** its
vector length equals the loaded extractor's `output_dimension()` (when the extractor reports
one). Anything else is `Foreign`. There is no alias any more.

- `crates/daemon/src/pipeline.rs`:
  `pub const EMBEDDING_MODEL_ID: &str = soos_inference_ort::SHIPPED_EMBEDDING_MODEL.model_id;`
  (value `"sface_2021dec"`). `LEGACY_EMBEDDING_MODEL_ALIAS_ID`,
  `LEGACY_EMBEDDING_MODEL_ALIAS_VERSION` and `TemplateModelBinding::LegacyAlias` are removed:
  `mobilefacenet` / `1.0.0` templates hold ArcFace vectors and must be `Foreign` once SFace is
  loaded (keeping the alias would compare them with SFace probes). ADR 2026-09-30 "Embedding Model
  Binding and Legacy Model Alias" is marked superseded for the alias part.
- `pub fn classify_template_model(loaded_model_id, template_model_id, template_model_version)`
  keeps its signature (call sites and tests compile) and returns `Current` iff the ids are equal,
  `Foreign` otherwise; the version is ignored (documented).
- New `pub fn classify_template(loaded_model_id: &str, loaded_dimension: Option<usize>,
  template_model_id: &str, template_dimension: usize) -> TemplateModelBinding`: `Foreign` when the
  ids differ or `loaded_dimension == Some(d)` and `template_dimension != d`.
- Dispatcher step 8d (already before the consensus loop and before any capture is consumed) uses
  `classify_template(expected, pipe.vision.embedding_dimension(), &t.model_id, t.embedding.len())`;
  `Foreign` → `Verdict::Unavailable` / `ReasonClass::ModelUnavailable` (unchanged reply and
  unchanged warning text without vector content). Today a dimension mismatch only reaches the
  cosine, which logs an error and scores `0.0` (a `Deny` that consumes a rate-limit attempt);
  after the change it is `Unavailable` before any inference.
- `VisionPipeline::embedding_dimension(&self) -> Option<usize>` delegates to the extractor.
- `soos-enroll verify`: before capturing, the stored template must be `Current` for
  `MODEL_ID_EMBEDDING` and the pipeline dimension; otherwise
  `EnrollmentCliError::TemplateModelMismatch { uid, template_model_id, loaded_model_id }` with the
  message "template of UID N was enrolled with model X; the loaded model is Y: re-enroll with
  `soos-enroll enroll`" (no camera start, no cosine). Owner question Q5.
- `soos-gui` live verification: the reference template is only installed in `match_reference`
  when it is `Current`; otherwise the panel shows "Re-enrollment required (template model X)"
  and no score.

One source of truth for the rule: the pure function
`soos_inference_ort::template_matches_model(loaded_id, loaded_dim, template_id, template_dim)`;
the daemon wraps it in `pipeline::classify_template` (its enum), `soos-enroll verify`, `list`,
`migrate` and the GUI call it directly (they do not depend on `soos-daemon`). The implemented
error variant is `TemplateModelMismatch { uid, template_model_id, template_dimension }`.

### 3.5 Threshold Default 0.50

| Location | Old | New |
|---|---|---|
| `crates/policy/src/threshold.rs` `ThresholdConfig::DEFAULT_MATCH_THRESHOLD` | 0.70 | **0.50** |
| `crates/vision/src/pipeline.rs` `DEFAULT_MATCH_THRESHOLD` | 0.70 | **0.50** |
| `crates/gui/src/app.rs:807` literal `let match_threshold = 0.70f32;` | 0.70 (duplicate) | `pipeline.config().match_threshold` (or `soos_vision::DEFAULT_MATCH_THRESHOLD`) |
| `crates/daemon/src/config.rs` | inherits the policy default unless `[pipeline.thresholds] match_threshold` is set | unchanged code |
| `soos-enroll verify` | `pipeline.config().match_threshold` (vision default) | unchanged code, value follows |
| Docs: `Docs/POLICY_CRATE.md:66` ("MobileFaceNet literature (FAR <= 0.1%)"), `Docs/VISION_CRATE.md:106,135`, `Docs/INFERENCE_ORT_CRATE.md`, `AI/ARCHITECTURE.md`, project facts §2, `tests/physical/screensaver_test.md` | 0.70 | 0.50 with the LFW SFace source (walkthrough 160) |

- `ThresholdConfig::MIN_MATCH_THRESHOLD` stays **0.40**: SFace LFW extended FAR at 0.40 is 2.8e-4,
  still under the 1e-3 target, so the floor keeps its meaning (owner question Q9). The default
  satisfies the floor (`threshold_floor_tests::test_defaults_satisfy_security_floor` unchanged).
- Comparison semantics unchanged: `score >= threshold`, non-finite scores refused.
- Operator note: a host whose `daemon.toml` sets `match_threshold = 0.70` explicitly keeps 0.70
  (SFace LFW TAR 0.47); the release note says to remove or lower the override.
- `MIN_SAMPLE_CONSISTENCY_COSINE = 0.5` (guided enrollment, pinned by
  `guided_enrollment_consistency_tests`) stays; its "ArcFace" comment becomes model-neutral. SFace
  LFW genuine p1 is 0.385 across pose and age; same-session samples within 25 degrees are expected
  well above 0.5 (to confirm on real captures, Q10).

### 3.6 Enrollment CLI (`soos-enroll`)

- `MODEL_ID_EMBEDDING = SHIPPED_EMBEDDING_MODEL.model_id` (`"sface_2021dec"`);
  `REQUIRED_MODEL_IDS[2]` follows. `EMBEDDING_MODEL_VERSION` stays `"2.0.0"` (= manifest version).
- `import`: `--model-id` default becomes `MODEL_ID_EMBEDDING` (today a literal
  `"arcface_w600k_mbf"`; the GUI pkexec path relies on this default, so without the change every
  GUI enrollment would be labelled ArcFace) and `--model-version` default `EMBEDDING_MODEL_VERSION`.
  `IMPORT_EMBEDDING_DIM = EMBEDDING_DIMENSION` (128). A JSON or CBOR template whose model id is not
  `MODEL_ID_EMBEDDING` is refused with `InvalidImport("template model X is not the loaded
  embedding model Y; re-enroll instead")` and nothing is stored (owner question Q4). The bounded
  64 KiB read, the never-growing buffer and the finite-value checks are unchanged.
- `migrate`: re-encrypts the envelope only and never rewrites `model_id` or the vector (already
  the case). An ArcFace template therefore stays `arcface_w600k_mbf` / 512-D after migration and
  stays `Foreign`. The migration summary adds one line per template whose model is not the loaded
  one: "UID N: template model X is not the loaded model Y; re-enroll" (metadata only; Q4).
- `list`: the table and JSON stay as they are (the GUI parses the JSON); `main.rs` prints a
  `[WARN]` line after the table for every `Foreign` template.
- `enroll`: unchanged (replaces an existing template after confirmation, which is the
  re-enrollment path); the summary already prints model id and dimension.
- Mock mode (`build_full_service_with_notes`): `MockEmbeddingExtractor::new(EMBEDDING_DIMENSION)`.

### 3.7 GUI (`soos-gui`)

- Threshold literal removed (3.5); reference binding (3.4); mock mode uses
  `EMBEDDING_DIMENSION`; the inspection label "ArcFace (112×112)" becomes "SFace (112×112)".
- `import_helper_args` is unchanged (`soos-enroll import --uid N --file - --yes`); the model id is
  the import default (3.6).

### 3.8 Error Taxonomy

| Condition | Component | Result | PAM |
|---|---|---|---|
| Template `model_id` != `sface_2021dec` (ArcFace, `mobilefacenet`, anything) | daemon 8d | `Unavailable` / `ModelUnavailable`, no inference | `PAM_IGNORE` → password |
| Template id current, vector length != 128 | daemon 8d | `Unavailable` / `ModelUnavailable` | `PAM_IGNORE` |
| Installed manifest without `sface_2021dec` (binary upgraded, models not redeployed) | daemon start | `get_or_load_session` error, daemon refuses to start | socket absent → `PAM_IGNORE` |
| SFace file absent / wrong SHA / wrong size | daemon start (`verify_integrity`) | refuses to start | `PAM_IGNORE` |
| Session layout != NCHW, output != 128 | extractor | `TensorError` / `DimensionMismatch` → `VisionError` → no `Allow` | `PAM_IGNORE` |
| `soos-enroll import` of a non-SFace template or wrong length | CLI | `InvalidImport`, nothing stored | n/a |
| `soos-enroll verify` with a `Foreign` template | CLI | `TemplateModelMismatch`, no capture | n/a |

No path turns an error into `Allow` / `PAM_SUCCESS`.

### 3.9 ArcFace Entry: Removal and Lineage

- `[models.arcface_w600k_mbf]` is removed from `models/manifest.toml`: no binary loads it,
  `download_models.sh` no longer fetches it, and it leaves the `NOASSERTION` licence question.
- `models/README.md` moves it to the lineage section (like the v1.0.0 models) with its SHA-256,
  size, source and measured results; `scripts/download_models.sh` comment updated.
- The ArcFace evaluation tests (`embedding_preprocessing_evaluation_tests`, the ArcFace arms of
  the LFW harness) need the old attestation to stay reproducible. Proposal (owner question Q2):
  `models/retired_models.toml`, same schema as `models/optional_models.toml`, read by no runtime
  crate and never by `download_models.sh`, holding the unchanged ArcFace entry; the evaluation
  tests load from it (setup-only change). Alternative: delete those tests (their results stay in
  walkthroughs 145 and 160).
- Installed hosts keep `/var/lib/soos/models/arcface_w600k_mbf.onnx` (136.6 MB, now unattested
  and unused) until the operator deletes it; Q8 asks whether `download_models.sh` should print a
  notice for unattested `*.onnx` files (no automatic deletion by a root script).

### 3.10 Latency Budget

The auth path changes only in the embedding stage: SFace p50 10.3 ms / p95 21.0 ms against
ArcFace p50 35.3 ms / p95 77.1 ms (walkthrough 160, four ORT threads, shared host). The budget
`PAM timeout_ms` (default 1000, clamped 10–5000) >= `DECISION_BUDGET_MS` (900) + camera wake + IPC
margin still holds with more headroom; `DEFAULT_INFERENCE_ESTIMATE_MS` (80) and the EMA seed are
unchanged (the start-up warm-up re-measures). The model is 3.5 times smaller to read and hash at
start-up.

### 3.11 Invariants Touched

- ARCHITECTURE §2 fail-closed: every new refusal ends in `Unavailable` / error, never `Allow`.
- Model attestation (Invariant 4): SHA-256, size and I/O shapes of the new entry, real-file test.
- Zero biometric leakage: no embedding values in logs or errors (binding messages carry ids and
  dimensions only); `Zeroizing` buffers unchanged.
- Single source of truth: model id, dimension and threshold each live in one constant; the GUI
  literal is removed and pinned by an invariant.
- No OpenCV: the OpenCV source is only a reference for the pre-processing; the runtime stays
  `ort` CPU.

### 3.12 Test Hooks for the Tester

- `OrtEmbeddingExtractor::with_spec` + the hand-encoded graphs of `tests/fixtures/embedding_onnx.rs`
  (`nchw_embedding_model(dim)`, `nhwc_embedding_model(dim)`) for layout and dimension refusals.
- `EmbeddingExtractor::output_dimension` on `MockEmbeddingExtractor` for the daemon dimension
  binding; `MockEmbeddingExtractor` call counting (or a recording extractor) to prove no
  inference runs after a `Foreign` refusal.
- `SOOS_MODELS_DIR` pointing at a scratch directory with the three attested files (SCRFD and
  MiniFASNet copied from `/var/lib/soos/models`, SFace from the evaluation cache) for the
  real-model tests on this host, without root.

### 3.13 Documentation Drift / ADR

- `AI/DECISIONS.md`: the Proposed ADR "Real-Face Embedding Evaluation and Recalibration Proposal"
  becomes **Accepted (2026-10-01, option C with a 0.50 threshold)**; new ADR "SFace Embedding Model
  Replaces ArcFace ResNet34" (manifest entry, RGB raw NCHW, in-graph normalization, 128-D,
  binding by id and dimension, alias removal, import refusal, threshold 0.50, accepted risk of
  undocumented training data, re-enrollment procedure). ADR 2026-09-30 "Embedding Model Binding and
  Legacy Model Alias": alias part marked superseded.
- Prose drift to fix in Phase 6: `models/README.md` §1, §4 and §6 ("MIT (as recorded in the
  manifest; not re-verified)" is already stale for ArcFace), `Docs/INFERENCE_ORT_CRATE.md`
  (ArcFace, 512D, NHWC, `(pixel - 127.5) / 127.5`), `Docs/VISION_CRATE.md` (ArcFace, 512D, 0.70),
  `Docs/POLICY_CRATE.md` (0.70 "MobileFaceNet literature"), `Docs/BIOMETRIC_STORE_CRATE.md`
  (example `vec![..; 512]`), `AI/ARCHITECTURE.md`, `AI/MOCK_STRATEGY.md`,
  `tests/fixtures/mod.rs` doc ("Face embeddings are 512D", reworded without a literal dimension
  so that `fixtures_contract` keeps passing), project facts §2 and §4.

## 4. Pre-existing Tests Migrated (approved by the owner on 2026-10-01)

**Approval**: the owner approved this 55-item list exactly as written (35 class a, 15 class b,
5 class c) on 2026-10-01 (Q1), with Q2 = retired manifest, Q5 = id check before capture
(items 48–50) and Q7 = mock default 128 (item 8). Every item below was executed as written and
nothing else was changed in these tests; the "After the switch" column is the new state
(old → new). Execution notes:

- #2 also keeps the `!extractor.is_nhwc()` assertion of the old test.
- #9 the SFace graph has a concrete batch of 1, so the shape pins are `[1, 3, 112, 112]` /
  `[1, 128]` (the old "batch dim is symbolic" checks do not apply to this file).
- #16, #18 changed only their manifest source (`retired_manifest_path()`); #17 was deleted and
  replaced by `embedding_real_model_tests::test_raw_opencv_recipe_matches_the_production_extractor`.
- #29 the `LEGACY_*` imports were dropped; #26 became `test_retired_cli_default_alias_is_foreign`.
- #33 also updates its comment `// match: 0.70` (class c); #34's comment "Never lower" was
  reworded to cite the owner decision; `pad_consensus_tests` doc comment `0.70 match` → `0.50
  match` (class c, §4.3).
- #43–#47 use `soos_enrollment_cli::service::IMPORT_EMBEDDING_DIM` /
  `MODEL_ID_EMBEDDING` instead of the literals; #48–#50 record `MODEL_ID_EMBEDDING`
  (version strings untouched).
- #51 updated; #52–#55 left as they are (their assertions still hold; `tests/fixtures/mod.rs`
  was reworded without a literal dimension).
- `cargo fmt` reformatted some of the migrated lines (layout only).

Classification: **(a)** the test pins the replaced model or default and must migrate (assertion
values change; the strength of each assertion is kept or increased); **(b)** setup-only (fixture
data or helper changes, assertions untouched); **(c)** unaffected but its message or rationale
becomes stale (optional cosmetic change). Matrix rows citing a migrated test are listed so that
Phase 6 can mark them `⏹ Superseded (SFCn)` or update them.

### 4.1 `soos-inference-ort`

| # | File::test | Assertion today | After the switch | Class | Matrix |
|---|---|---|---|---|---|
| 1 | `embedding_io_contract_tests::test_embedding_dimension_constant_is_512` | `EMBEDDING_DIMENSION == 512` | `== 128`, renamed `..._is_128` | a | VMX4 |
| 2 | `embedding_io_contract_tests::test_extract_embedding_accepts_512d_nhwc_output` | NHWC 512-D graph accepted, `is_nhwc()`, layout `Nhwc` | NHWC session fails closed (`TensorError`, `input_layout() == None`), renamed `test_extract_embedding_rejects_nhwc_session` | a | VMX4 |
| 3 | `embedding_io_contract_tests::test_extract_embedding_accepts_512d_nchw_output` | NCHW 512-D accepted, `len == 512` | NCHW 128-D accepted, `len == 128`, normalized; renamed `..._128d_...` | a | VMX4 |
| 4 | `embedding_io_contract_tests::test_extract_embedding_rejects_wrong_dimension` | NHWC dims `[3, 128, 511, 513]` rejected, `expected == 512` | NCHW dims `[3, 127, 129, 512]` rejected, `expected == 128` | a | VMX4 |
| 5 | `embedding_tests::test_embedding_normalization_symmetric_range` | BGR NCHW `(x - 127.5) / 127.5`: 255 → +1.0 on plane 0, 0 → -1.0 on plane 2, 127/128 anti-symmetric | RGB raw: plane 0 (R) = 0.0, plane 1 (G) = 127.0, plane 2 (B) = 255.0, last pixel 255/128/0; renamed `test_embedding_input_is_raw_0_255` | a | NGM7 |
| 6 | `embedding_tests::test_prepare_input_layout_nhwc_and_nchw` | `prepare_input_layout(.., false/true)` writes B, G, R in both layouts | function removed; replaced by `test_prepare_input_writes_rgb_nchw_planes` (NCHW only, exact raw values at pixel 0 and the last pixel) | a | — |
| 7 | `embedding_tests::test_arcface_input_bgr_ordering` | pure red: plane 0 = -1.0, plane 1 = -1.0, plane 2 = +1.0 | pure red: plane 0 = 255.0, planes 1 and 2 = 0.0; renamed `test_sface_input_rgb_ordering` | a | BIO1 |
| 8 | `embedding_tests::test_mock_embedding_default_512d` | `MockEmbeddingExtractor::default().dim() == 512` (and `new_default`) | `== EMBEDDING_DIMENSION` (128), renamed; only if Q7 = yes | a | NGM7, NGM16 |
| 9 | `embedding_real_model_tests::test_real_embedding_model_io_metadata_is_pinned` | file 136,619,444 B; `input_1` `[-1, 112, 112, 3]`; `embedding` `[-1, 512]` | file 38,696,353 B; `data` `[1, 3, 112, 112]`; `fc1` `[1, 128]` | a | EMR4 |
| 10 | `embedding_real_model_tests::test_real_embedding_committed_manifest_matches_session` | manifest layout `Nhwc` | `Nchw` | a | EMR4 |
| 11 | `embedding_real_model_tests::test_registry_rejects_real_embedding_model_under_nchw_manifest` | NCHW manifest rejects the NHWC ArcFace graph | NHWC manifest rejects the NCHW SFace graph (`ModelShapeMismatch`), renamed `..._under_nhwc_manifest` | a | EMR3 |
| 12 | `embedding_real_model_tests::test_real_models_all_pass_committed_manifest_shape_validation` | file list contains `arcface_w600k_mbf.onnx` | `sface_2021dec.onnx` | b | EMR3 |
| 13 | `embedding_real_model_tests::test_real_embedding_extractor_uses_nhwc_and_emits_normalized_512d` | `is_nhwc()`, 512-D | layout `Nchw`, 128-D, finite, deterministic, normalized; renamed `..._nchw_..._128d` | a | EMR6 |
| 14 | `embedding_real_model_tests::test_real_embedding_latency_report` | ArcFace id/file | SFace id/file | b | EMR6 |
| 15 | `embedding_real_model_tests::test_real_embedding_model_loads_under_manifest_without_input_layout` | `OrtEmbeddingExtractor::new(session).is_nhwc()` | `input_layout() == Some(Nchw)` | a | EMR7 |
| 16 | `embedding_preprocessing_evaluation_tests::test_real_embedding_graph_has_no_in_graph_normalization` | ArcFace graph from the committed manifest | same assertions, ArcFace loaded from `models/retired_models.toml` (Q2) — or deleted | b (or deleted) | SFX1 |
| 17 | `embedding_preprocessing_evaluation_tests::test_raw_production_arm_matches_the_production_extractor` | raw BGR/127.5 ArcFace arm equals `OrtEmbeddingExtractor` | the production extractor is no longer ArcFace: deleted, superseded by the new SFace test `test_raw_opencv_recipe_matches_the_production_extractor` (SFC) | a (delete + replace) | SFX2 |
| 18 | `embedding_preprocessing_evaluation_tests::test_real_embedding_preprocessing_sensitivity_report` | ArcFace from the committed manifest | ArcFace from the retired manifest (Q2) — or deleted | b (or deleted) | SFX3 |
| 19 | `manifest_shape_tests::test_workspace_manifest_declares_embedding_layout_nhwc` | `arcface_w600k_mbf` `Nhwc`, dims `[1, 112, 112, 3]` | `sface_2021dec` `Nchw`, dims `[1, 3, 112, 112]`, renamed `..._nchw` | a | EMR5 |
| 20 | `manifest_shape_tests::test_workspace_manifest_embedding_description_is_truthful` | needles `ResNet34`, `NHWC`, `tf2onnx`, `512`; must not contain `MobileFaceNet` | needles `SFace`, `NCHW`, `128`, `RGB`; must not contain `ArcFace` or `ResNet34` (SFace is MobileFaceNet-based, so the old negative needle no longer applies) | a | EMR5 |
| 21 | `manifest_shape_tests::test_workspace_manifest_declares_embedding_layout_explicitly` | `arcface_w600k_mbf` declares its layout | `sface_2021dec` declares it | a | EMR7 |
| 22 | `manifest_tests::test_parse_workspace_manifest_file` | ArcFace filename, `NOASSERTION`, `[1, 512]`, SHA `ffe014a4...` | SFace filename, `Apache-2.0`, `[1, 128]`, SHA `0ba9fbfa...`, plus `get_model("arcface_w600k_mbf").is_none()` | a | PAD1, NGM1, NGM17, SFX5 |
| 23 | `registry_tests::test_registry_initialization_with_workspace_manifest` | `get_model("arcface_w600k_mbf").is_some()` | `get_model("sface_2021dec").is_some()` | a | — |
| 24 | `registry_attestation_tests::test_max_model_file_bytes_bounds_the_attested_models` | `MAX_MODEL_FILE_BYTES >= 2 * 136_619_444` | still true; kept unchanged (stricter than the new largest model); doc comment stale | c | — |

Unaffected: the generic layout tests of `manifest_shape_tests` that build synthetic NHWC/512
metadata (`test_input_layout_nhwc_permutes_logical_shape`, `test_validate_*`, `test_absent_layout_*`,
`test_declared_layout_is_enforced_both_ways`, ...) test the registry mechanism, which stays;
`embedding_io_contract_tests::test_nhwc_detection_fails_closed_without_inputs` and
`test_embedding_layout_detection_fails_closed_on_ambiguous_input` keep their assertions;
`embedding_tests::test_mock_embedding_extractor_criterion_v2` uses explicit dimensions.

### 4.2 `soos-daemon`

| # | File::test | Assertion today | After the switch | Class | Matrix |
|---|---|---|---|---|---|
| 25 | `template_model_binding_tests::test_embedding_model_id_is_attested_by_manifest` | `EMBEDDING_MODEL_ID == "arcface_w600k_mbf"` | `== "sface_2021dec"` (and `== SHIPPED_EMBEDDING_MODEL.model_id`) | a | EMP3 |
| 26 | `template_model_binding_tests::test_legacy_alias_constants_are_exactly_the_retired_cli_default` | alias constants `mobilefacenet` / `1.0.0` | constants removed; replaced by an assertion that `mobilefacenet` / `1.0.0` classifies `Foreign` | a | EMP4 |
| 27 | `template_model_binding_tests::test_classify_template_model_accepts_only_current_or_exact_legacy_alias` | `mobilefacenet` / `1.0.0` → `LegacyAlias` | → `Foreign`, plus `arcface_w600k_mbf` / `2.0.0` → `Foreign`; renamed `..._accepts_only_the_loaded_model` | a | EMP4 |
| 28 | `template_model_binding_tests::test_legacy_alias_template_reaches_allow` | `Allow` / `FaceMatch` | `Unavailable` / `ModelUnavailable`; renamed `test_legacy_alias_template_is_refused` | a | EMP4 |
| 29 | `template_model_binding_tests` imports | `LEGACY_EMBEDDING_MODEL_ALIAS_ID`, `..._VERSION` | import line drops them | b | — |
| 30 | `model_deployment_tests::test_models_readme_complete_and_accurate` | README contains `arcface_w600k_mbf` | contains `sface_2021dec` (ArcFace stays in the lineage section, so the old needle would still pass but no longer proves the shipped model is documented) | a | D14, NGM15 |
| 31 | `pad_nonface_pipeline_real_model_tests` (`REQUIRED_FILES`, used by `test_pva12_real_pipeline_never_accepts_synthetic_nonface_frames`) | `arcface_w600k_mbf.onnx` | `sface_2021dec.onnx` | b | PVA12 |

Unaffected: the other binding tests (`test_template_with_foreign_model_id_is_refused`,
`test_template_with_matching_model_id_is_evaluated`, `test_production_wiring_*`,
`test_legacy_alias_with_other_version_is_refused`, `test_unrelated_foreign_model_id_is_refused`),
`vision_threshold_parity_tests` and `threshold_config_tests` (symbolic constants), every daemon
test that pairs `MockEmbeddingExtractor::new(512)` with 512-D templates (consistent dimensions),
`config_validation_tests::test_programmatic_unchecked_thresholds_fail_validate` (literal 0.70 is
data).

### 4.3 `soos-policy` and `soos-vision`

| # | File::test | Assertion today | After the switch | Class | Matrix |
|---|---|---|---|---|---|
| 32 | `policy/tests/threshold_tests::test_threshold_defaults_match_literature` | default match 0.70 | 0.50 | a | — |
| 33 | `policy/tests/decision_tests::test_decision_deny_score_below_threshold` | score 0.6999 with the default → `Deny` | 0.6999 would be `Allow`: score becomes 0.4999 (`Deny`, `ScoreBelowThreshold`) | a | — |
| 34 | `vision/tests/threshold_constants_tests::test_vision_threshold_constants_values` | `DEFAULT_MATCH_THRESHOLD == 0.70` | `== 0.50` | a | VTD4 |
| 35 | `vision/tests/pipeline_tests::test_pipeline_config_defaults_3_model` | `match_threshold == 0.70` | `== 0.50` | a | — |
| 36 | `vision/tests/pipeline_tests::test_vision_pipeline_default_thresholds_calibrated` | `match_threshold == 0.70` | `== 0.50` | a | BIO1 |
| 37 | `vision/tests/embedding_lfw_evaluation_tests::test_lfw_real_face_evaluation_report` (ignored) | production arm = ArcFace BGR; ArcFace from the committed manifest; `REPORTED_THRESHOLDS` ends with `DEFAULT_MATCH_THRESHOLD` (0.70) | production arm = SFace RGB raw from the committed manifest; ArcFace arms from the retired manifest (Q2); thresholds `[0.40, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70]` as literals (the default is now 0.50, already in the list) | a | EVR3–EVR5 |

Unaffected: `threshold_floor_tests` (symbolic, 0.50 >= 0.40), `pad_consensus_tests` and
`pad_final_reason_tests` (match scores 0.90 / 0.40 / 0.10 keep their class at 0.50; the doc
comment "0.70 match" becomes stale, c), `decision_tests` comment "match: 0.70" (c), the four
regular LFW harness tests (no model).

### 4.4 `soos-enrollment-cli`

| # | File::test | Assertion today | After the switch | Class | Matrix |
|---|---|---|---|---|---|
| 38 | `model_id_tests::test_enrollment_cli_model_ids_match_manifest` | `MODEL_ID_EMBEDDING` and `REQUIRED_MODEL_IDS[2] == "arcface_w600k_mbf"` | `== "sface_2021dec"` (manifest version `"2.0.0"` unchanged unless Q6) | a | EN7, NGM15, NGM17 |
| 39 | `enroll_model_provenance_tests::test_embedding_model_constants_match_attested_manifest` | `MODEL_ID_EMBEDDING == "arcface_w600k_mbf"` | `== "sface_2021dec"` | a | EMP1 |
| 40 | `import_tests::test_import_embedding_success` | 512 floats, model `arcface_w600k_mbf`; `embedding_dim == 512`, stored id ArcFace | 128 floats, `MODEL_ID_EMBEDDING`; `== 128`, stored id SFace | a | GEPU1, GEPU4 |
| 41 | `import_tests::test_import_embedding_dimension_mismatch_fails` | a **128**-value import is refused ("dimension != 512") | 128 is now valid: the refused vector becomes 512 values (and 127) | a | GEPU1 |
| 42 | `import_stdin_tests::test_import_reads_embedding_from_stdin` | 512 floats, `embedding_dim == 512`, `loaded.len() == 512` | 128 / `IMPORT_EMBEDDING_DIM` | a | GEPU4, ISE5 |
| 43 | `import_stdin_tests::test_import_rejects_malformed_or_non_finite_embeddings` | 512-value array with one `1e39`, and a 511-value array, are refused | still refused, but for the wrong reason (dimension), so the non-finite and short-array paths lose their coverage: lengths become `IMPORT_EMBEDDING_DIM` and `IMPORT_EMBEDDING_DIM - 1` (assertions untouched) | b (restores test power) | — |
| 44 | `import_stdin_tests::stdin_args` helper (used by the 4 import tests of the file) | model `arcface_w600k_mbf` | `MODEL_ID_EMBEDDING`; vectors `IMPORT_EMBEDDING_DIM` | b | — |
| 45 | `cli_hygiene_tests::test_import_onto_enrolled_uid_without_yes_is_refused`, `::test_import_onto_enrolled_uid_with_yes_replaces_and_reports_it`, `::test_stdin_import_reports_replacement` | helpers `write_embedding` (512) and `file_args` (ArcFace id) | 128 / `MODEL_ID_EMBEDDING` | b | — |
| 46 | `import_stdin_overwrite_tests` (4 tests through `stdin_args` / `embedding_json`) | ArcFace id, 512 floats | SFace id, 128 floats | b | — |
| 47 | `import_enroll_if_absent_tests::test_sgu_import_without_yes_never_replaces_a_concurrent_enrollment`, `::test_sgu_import_with_yes_still_replaces_and_reports_it` | ArcFace id, 512 floats | SFace id, 128 floats | b | — |
| 48 | `verify_tests` (`setup_verify_env`: template `mobilefacenet` / `1.0.0`; tests `test_verify_matching_user_reports_allow_and_metrics`, `test_verify_non_matching_user_reports_deny`) | verdicts on a `mobilefacenet` template | the template is recorded with `MODEL_ID_EMBEDDING` (assertions untouched); only if Q5 = id check before capture | b | — |
| 49 | `verify_pad_report_tests` (`setup_with_camera` and the inline template of `test_verify_no_face_reports_no_pad_score`; 6 tests) | `mobilefacenet` template | `MODEL_ID_EMBEDDING` (assertions untouched); only if Q5 | b | — |
| 50 | `quality_gate_report_tests::service` (`test_verify_face_too_small_reports_deny`, `test_verify_face_blurred_reports_deny`) | `mobilefacenet` template | `MODEL_ID_EMBEDDING`; only if Q5 | b | — |

Unaffected: `import_json_presize_tests` (symbolic `IMPORT_EMBEDDING_DIM`), `scaffold_tests`
(symbolic), `enroll_tests`, `enroll_model_provenance_tests` other tests and
`enroll_fresh_frames_tests` (enroll records whatever model id the arguments carry; no dimension
check at enrollment), `list_tests`, `list_bounded_tests`, `migrate_tests`, `delete_tests` (opaque
template data), `guided_enrollment_*` (explicit `DIM = 512` synthetic vectors; the guided session
is dimension-agnostic), `quality_tests` (`0.70` there is a quality floor, not the match threshold).

### 4.5 `soos-gui`, `soos-biometric-store`, invariants and shell tests

| # | File::test | Assertion today | After the switch | Class | Matrix |
|---|---|---|---|---|---|
| 51 | `tests/docker/systemd_unit_acceptance_test.sh` (host model copy loop, line 440) | copies `arcface_w600k_mbf.onnx` | copies `sface_2021dec.onnx` | b | — |
| 52 | `fixtures_contract::test_fil_fixtures_carry_no_stale_128d_embeddings` | `tests/fixtures/mod.rs` must not contain `128D` or `; 128]` ("the contract is 512D") | passes if the fixture doc is reworded without a literal dimension; rationale inverted. Optional: forbid a stale `512D` claim instead | c (optional a) | FIL |
| 53 | `maintainer_hygiene_contract::test_mock_and_memory_docs_describe_current_fixtures_and_512d` | `Docs/MEMORY_PROTECTION_AND_SWAP.md` must not contain `128D/512D` | still passes; name and message stale | c | — |
| 54 | `review_followups_contract` (line 130 message "embeddings are 512D") | message only | stale message | c | — |
| 55 | `vision_threshold_contract::test_vision_docs_state_code_thresholds_and_parsing` | message "(code: DEFAULT_MATCH_THRESHOLD 0.70)" | message only | c | — |

Unaffected (opaque `arcface_w600k_mbf` / 512-D data; the store is model-agnostic):
`biometric-store` tests (`aad_binding`, `aad_migration`, `bounded_read`, `bulk_migration`,
`cbor_zeroize`, `enroll_if_absent`, `erasure`, `store_lock`, `temp_sweep`) and the unit tests of
`store.rs` / `template.rs`; `gui` `store_task_tests`, `layout_tests` (JSON round trip of list
data), `import_privacy_tests` (argv unchanged, 512 floats sent to a fake helper),
`guided_*_tests`; `installer_templates_contract::test_download_models_dry_run_reports_only_manifest_urls`
(synthetic manifest data); `vision_attestation_contract` (legacy v1 ids).

Summary: **35 assertion migrations (a)** — of which #8 depends on Q7 and #17 is a delete plus
replacement — **15 setup-only items (b)** (#16 and #18 alternatively deleted under Q2; #48–#50
only if Q5 chooses the id check), and **5 cosmetic (c)**. No assertion is weakened: every value change replaces an ArcFace or 0.70 pin
with the equivalent SFace or 0.50 pin, and #2, #26–#28 become stricter (refusals instead of
acceptances).

## 5. Tester Contract (Phase 2) and Red Evidence

New tests (rows of the new matrix section `sface-embedding-model`):

| Row | Test (path::name) | Red evidence on the unchanged code (commit `c1717d7`) |
|---|---|---|
| SFC1 | `manifest_tests::test_workspace_manifest_attests_sface_from_pinned_revision` (+ migrated #19, #21–#23) | FAILED: `sface_2021dec` entry absent (TOML index panic) |
| SFC2 | `embedding_io_contract_tests::test_shipped_embedding_spec_is_sface` (+ #1, #25, #38, #39) | compile error: `SHIPPED_EMBEDDING_MODEL`, `SFACE_2021DEC`, `EmbeddingModelSpec` not found (the specified API); #38/#39 FAILED on the id |
| SFC3 | migrated #5–#7 | FAILED: plane 0 = `-1.0` / `+1.0` instead of the raw `0.0` / `255.0` |
| SFC4 | migrated #2–#4, #8 | #8 FAILED `left: 512, right: 128`; #2–#4 compile error with the missing spec items |
| SFC5 | `embedding_real_model_tests::test_real_sface_graph_normalizes_in_graph` (+ #9–#15) | red by construction: the SFace file is not attested by the old manifest |
| SFC6 | `embedding_real_model_tests::test_raw_opencv_recipe_matches_the_production_extractor` | red by construction like SFC5 (session load fails: the old manifest does not attest the file); against the old BGR `/127.5` extractor the recipe comparison could not hold either |
| SFC7 | `sface_template_binding_tests::test_arcface_template_is_refused_after_the_sface_switch`, `::test_current_sface_template_is_evaluated` | compile error: `classify_template`, `EmbeddingExtractor::output_dimension` (specified API) |
| SFC8 | migrated #26–#28 | compile error (`SHIPPED_EMBEDDING_MODEL`); on the old code the alias tests expected `LegacyAlias` / `Allow` |
| SFC9 | `sface_template_binding_tests::test_template_with_wrong_dimension_is_refused`, `::test_classify_template_binds_model_id_and_dimension` | compile error (specified API); behaviourally the old daemon answered `Deny` (score 0.0) |
| SFC10 | existing `ipc_tests::test_ipc_unavailable_returns_ignore` | — (cited, unchanged) |
| SFC11 | migrated #32–#36 | FAILED on 0.70 vs 0.50 (#32, #34–#36). #33 (0.4999 → `Deny`) passes on both: its power is that the old literal 0.6999 would be `Allow` under 0.50 |
| SFC12 | `vision_threshold_contract::test_no_match_threshold_literal_outside_the_constants` | FAILED: `crates/gui/src/app.rs: let match_threshold = 0.70f32;` |
| SFC13 | `import_tests::test_import_refuses_a_template_of_another_model` (+ #40–#42) | FAILED: the CLI default was `arcface_w600k_mbf`, a 512-value import succeeded |
| SFC14 | `migrate_tests::test_migrate_never_rebinds_a_template_to_the_loaded_model` | compile error: `MigrationSummary::reenrollment_required` (specified API) |
| SFC15 | `verify_tests::test_verify_refuses_a_template_of_another_model` | compile error: `EnrollmentCliError::TemplateModelMismatch` (specified API) |
| SFC16 | `embedding_model_docs_contract::test_docs_state_the_sface_model_and_the_050_default` | FAILED: docs name no `sface_2021dec`, ADR still Proposed |
| SFC17 | `embedding_model_docs_contract::test_retired_models_file_is_read_by_no_runtime_crate_or_script`, `::test_download_models_reports_unattested_model_files_without_deleting_them` | FAILED: `models/retired_models.toml` absent; no "not attested" notice |
| SFC18 | `embedding_real_model_tests::test_registry_silences_sface_initializer_warnings_only` (added in Phase 4 with the Q11 answer) | a default session of the file emits **174** "initializer appears in graph inputs" warnings (measured by the test's control arm); without the registry log level the registry session emits the same 174 |

Flakiness: the new daemon integration tests (`sface_template_binding_tests`) use the
5 s fixture connection timeout of `template_model_binding_tests` and passed in every run of the
batch (full workspace runs and package runs).

## 6. Risks and Re-enrollment Impact

### 6.1 Installed host (this machine)

`/var/lib/soos/models` holds the v2.0.0 ArcFace deployment (manifest, SCRFD, MiniFASNet,
`arcface_w600k_mbf.onnx`) and every template records `arcface_w600k_mbf` / `2.0.0` (512-D).

| Step | Effect |
|---|---|
| New binaries installed, models not redeployed | the installed manifest has no `sface_2021dec`: `soos-daemon` refuses to start, PAM falls back to the password (fail closed) |
| `sudo ./scripts/install.sh` (or `sudo ./scripts/download_models.sh`) | downloads `sface_2021dec.onnx` (38.7 MB, SHA-256 and size verified), overwrites the deployed manifest; the ArcFace file stays on disk, unattested and unused (and any operator-appended optional PAD entry is overwritten, as today) |
| Daemon restarted | every `Auth` → `Unavailable` / `ModelUnavailable` (warning "re-enrollment required") → `PAM_IGNORE` → password; like every request, each attempt still reserves one slot of the per-UID rate limit (8-pre runs before the template check) |
| Each user: `sudo soos-enroll enroll -u <user>` (confirms the replacement) or the GUI guided enrollment | new `sface_2021dec` / 128-D template; face unlock works again |
| Optional: `sudo rm /var/lib/soos/models/arcface_w600k_mbf.onnx` | frees 136.6 MB |
| Rollback to the ArcFace release | SFace templates become `Foreign` for the old daemon (id mismatch) → password fallback; users would re-enroll again |

`soos-enroll migrate` neither helps nor harms: it only re-encrypts envelopes; embeddings of two
models are not convertible.

### 6.2 Risks

1. **Training-data lineage** (accepted by the owner's adoption decision, to record in the ADR):
   the SFace file is Apache-2.0 but its training data is undocumented (possibly MS1MV2, derived
   from the withdrawn MS-Celeb-1M; opencv_zoo issue #318 unanswered).
2. **Real-camera calibration**: 0.50 comes from LFW web photos; the genuine scores of the owner's
   own camera (RGB and IR) are not measured. `soos-enroll verify` prints the score; IR (`Grey`)
   captures were not evaluated.
3. **ORT log noise**: the IR 6 export lists 174 initializers as graph inputs, and ONNX Runtime
   warns once per initializer when the session is created (seen with `onnxruntime` 1.20.1). If
   `ort` 2.0.0-rc.13 forwards them to `tracing`, `soos-daemon`, `soos-enroll` and `soos-gui` log
   about 174 warnings at each start. Phase 4 checks the journal; mitigation candidates: a session
   log level of `Error` for the registry's sessions, or accepting the noise (no security impact).
   The file cannot be rewritten (its SHA-256 is the attestation).
4. **Fixed batch 1**: the graph rejects batch 2; the extractor always runs batch 1 (no change).
5. **Partial upgrade states** are all fail closed (6.1), but the user sees password prompts until
   re-enrollment; the release note and the daemon warning must say so.
6. **Operator override**: an explicit `match_threshold = 0.70` in `daemon.toml` keeps 0.70.
7. **Guided enrollment consistency** (`MIN_SAMPLE_CONSISTENCY_COSINE = 0.5`) was chosen for
   ArcFace; with SFace the same-session pose samples are expected above it, unverified on real
   captures.
8. **Contract churn**: 35 assertion migrations and 15 setup-only items (section 4); every one is
   listed for approval, none relaxes an assertion.

## 7. Plan Evaluation (Phase 1.5)

`AI/plan_evaluator_report.md`: **VALIDATION_VERDICT: APPROVED**. Findings: F1 (critical) the
legacy alias would compare ArcFace vectors with SFace probes — removed; F2 (major) the `import`
model default literal would label every GUI enrollment ArcFace — derived from
`MODEL_ID_EMBEDDING`; F3 (minor) `store_imported` reported a hard-coded 512 — fixed; F4 (major)
GUI threshold literal 0.70 — removed and pinned; F5 (major) id-only binding scored a wrong-length
vector as `Deny` — dimension binding; F6 (minor) one more pre-existing test breaks (section 11).
Correction to §6.1: every request reserves a rate-limit slot before the template check, so a
refused ArcFace template still counts one attempt (unchanged behaviour).

## 8. Owner Decisions (2026-10-01)

Q1 approved (section 4). Q2 retired ArcFace attestation in `models/retired_models.toml`, never
read at runtime. Q3 legacy alias removed. Q4 `import` refuses any non-SFace / non-128-D template;
`migrate` and `list` print a re-enrollment notice. Q5 `verify` and the GUI check the template
model before capture. Q6 manifest version stays `2.0.0`. Q7 mock default dimension 128. Q8
`download_models.sh` reports unattested `.onnx` files, no deletion. Q9 `MIN_MATCH_THRESHOLD`
stays 0.40. Q10 deferred to the owner's face session (GitHub #296, section 11). Q11 the SFace
session logs at ORT `Error` only (that model only; errors stay visible). Training data: the
owner accepts that the SFace training data is undocumented (possibly MS1MV2 / MS-Celeb-1M
derived); recorded in the new ADR with the file licence (Apache-2.0) and the sources.

## 9. Auditor Constraints (Phase 3)

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No `unwrap` / `expect` / `panic!` / indexing in the new production code | `embedding.rs`, `registry.rs`, `pipeline.rs`, `dispatcher.rs`, enrollment `service.rs` / `main.rs`, GUI `app.rs` | clippy `-D warnings` (workspace lints deny them); `prepare_input` writes through `get_mut` |
| 2 | Every new refusal ends in `Unavailable` / an error, never `Allow` | dispatcher 8d, `verify`, import, GUI reference | SFC7, SFC9, SFC13, SFC15; GUI shows no score for a foreign template |
| 3 | No embedding value in logs, errors or JSON | dispatcher warning (ids and dimensions), `TemplateModelMismatch`, `InvalidImport`, migrate `reenrollment_required` (UIDs only), list warning | SFC14 asserts no vector value in the JSON; code review |
| 4 | Input and ORT output buffers stay wipe-on-drop | `OrtEmbeddingExtractor::prepare_input` (`Zeroizing`), `ZeroizingOutputs` | unchanged containers; `ort_output_zeroize_contract` invariant green |
| 5 | Single source of the model id, dimension and threshold | `SHIPPED_EMBEDDING_MODEL`, `DEFAULT_MATCH_THRESHOLD` | SFC2, SFC11, SFC12 |
| 6 | The retired attestation never reaches a runtime path | `models/retired_models.toml` | SFC17 |
| 7 | `download_models.sh` never deletes, only reports; stays manifest-driven and HTTPS / file only | `scripts/download_models.sh` | SFC17, existing `model_download_size_contract` and `installer_templates_contract` |
| 8 | ORT warnings are silenced only for the SFace session, errors stay visible | `registry::ERROR_ONLY_LOG_MODELS`, `ort_session_log_level` | SFC18 (other models keep `Warning`) |
| 9 | Fail closed on a partial upgrade (new binary, old manifest) | daemon start | `get_or_load_session` error → refuse to start (existing `model_deployment_tests`) |
| 10 | No new dependency | Cargo manifests | `Cargo.lock` unchanged; `ort::logging::LogLevel` is in the pinned `ort` |

Clearance: **CLEARED**.

## 10. Implementation (Phase 4)

| File | Change |
|---|---|
| `models/manifest.toml` | `[models.sface_2021dec]` replaces the ArcFace entry; header comment |
| `models/retired_models.toml` (new) | unchanged ArcFace attestation, evaluation only |
| `models/README.md` | SFace contract, lineage, retired section, licence notice |
| `crates/inference-ort/src/embedding.rs` | `EmbeddingModelSpec`, `SFACE_2021DEC`, `SHIPPED_EMBEDDING_MODEL`, `EMBEDDING_DIMENSION = 128`, `template_matches_model`, `EmbeddingExtractor::output_dimension`, `OrtEmbeddingExtractor::with_spec` / `spec`, RGB raw NCHW `prepare_input`; `prepare_input_layout` removed |
| `crates/inference-ort/src/registry.rs` | `ERROR_ONLY_LOG_MODELS`, `ort_session_log_level`, session `with_log_level` |
| `crates/inference-ort/src/mock.rs`, `src/lib.rs`, `Cargo.toml` | mock default and `output_dimension`; re-exports; description |
| `crates/vision/src/pipeline.rs` | `DEFAULT_MATCH_THRESHOLD = 0.50`, `VisionPipeline::embedding_dimension` |
| `crates/policy/src/threshold.rs` | `DEFAULT_MATCH_THRESHOLD = 0.50` with the LFW source |
| `crates/daemon/src/pipeline.rs`, `src/dispatcher.rs`, `src/inference.rs` | id from the spec, alias removed, `classify_template` by id and dimension in step 8d, comments |
| `crates/enrollment-cli/src/service.rs`, `args.rs`, `error.rs`, `main.rs`, `guided_enrollment.rs` | ids from the spec, import bound to the loaded model (and the 512 outcome fixed), `verify` refusal, `foreign_template_uids` / `reenrollment_required`, list and migrate notices, mock dimension, `TemplateModelMismatch` |
| `crates/gui/src/app.rs`, `src/main.rs`, `src/lib.rs` | threshold from the pipeline config, reference binding with a re-enrollment note, mock dimension, labels |
| `crates/biometric-store/src/template.rs` | doc example id |
| `scripts/download_models.sh` | unattested `.onnx` notice; size comment |
| Docs | `Docs/INFERENCE_ORT_CRATE.md`, `Docs/VISION_CRATE.md`, `Docs/POLICY_CRATE.md`, `Docs/DAEMON.md`, `Docs/ENROLLMENT_CLI.md`, `Docs/MEMORY_PROTECTION_AND_SWAP.md`, `AI/ARCHITECTURE.md`, `AI/MOCK_STRATEGY.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/plan_evaluator_report.md`, `.agents/skills/dev-workflow/references/project-facts.md`, `tests/fixtures/mod.rs`, `tests/physical/screensaver_test.md` |

Deviation from the spec: none in behaviour. `classify_template_model` keeps its three-argument
signature (version ignored) as specified; the GUI field `_pipeline` was renamed `pipeline` to read
its configuration.

## 11. Verification Results and Follow-ups

All on this host (2026-10-01, `CARGO_BUILD_JOBS=8`). The scratch models directory holds
SCRFD and MiniFASNet copied from `/var/lib/soos/models`, the retired ArcFace file (for the
evaluation tests) and SFace from the evaluation cache (SHA-256 verified).

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | pass |
| `SOOS_MODELS_DIR=<scratch> cargo test --locked --workspace --all-targets --all-features` | 2165 passed, **1 failed** (F6 below, not edited), 6 ignored |
| `SOOS_MODELS_DIR=<scratch> SOOS_REQUIRE_REAL_MODELS=1 cargo test -p soos-inference-ort -p soos-daemon -p soos-vision --all-targets --all-features -- --include-ignored` | 723 passed, 0 failed (SFace real-model suite incl. SFC5, SFC6, SFC18; retired ArcFace evaluation; PVA12 real pipeline) |
| `cargo test -p soos-invariants` (after the docs) | 344 passed |
| `ORT_SKIP_DOWNLOAD=1 cargo check --locked --workspace --all-targets --all-features --target i686-unknown-linux-gnu` | pass |
| `./scripts/candid_review.sh` (layer 1) | PASSED |
| `./tests/docker/systemd_unit_acceptance_test.sh --models download` | passed: `download_models.sh` fetched and verified `sface_2021dec.onnx` from the pinned revision, the release daemon reached READY=1 in 227 ms and `soos-admin status` reported `models_verified`; host sysctls unchanged |

LFW smoke run of the re-targeted harness (600 official pairs, 807 images, debug build,
`SOOS_EVAL_MAX_PAIRS=600`, four ORT threads), aggregates only:

| Variant | 10-fold acc. (600 pairs) | Extended TAR @ FAR 1e-4 | Extended FAR / TAR at 0.50 | Embed p50 |
|---|---|---|---|---|
| SFace RGB raw (production) | 0.9917 | 0.986 (thr 0.435) | 6.2e-6 / 0.965 | 10.9 ms |
| SFace BGR raw | 0.9917 | 0.979 (thr 0.430) | 1.2e-5 / 0.935 | 10.1 ms |
| Retired ArcFace BGR /127.5 | 0.9867 | 0.693 (thr 0.492) | — | 29.7 ms |

The subset agrees with the full run of walkthrough 160 (SFace RGB TAR 0.984 at FAR 1e-4, FAR
3.9e-6 / TAR 0.957 at 0.50) and confirms that the production arm is now SFace.


### Known limitations / follow-ups

- **F6 — additional pre-existing test, not edited (awaits owner approval)**:
  `crates/enrollment-cli/tests/import_json_presize_tests.rs::test_import_json_decodes_into_a_buffer_that_never_grows`
  iterates over `[0, 1, 3, 257, IMPORT_EMBEDDING_DIM - 1, IMPORT_EMBEDDING_DIM]` and asserts that
  every length is stored entirely; `257` assumed a 512-value import buffer. With
  `IMPORT_EMBEDDING_DIM = 128` the 257-value case is (correctly) truncated to 128 and the
  assertion `embedding == values` fails. Proposed setup-only migration: replace the literal 257
  with a length below the dimension (for example `IMPORT_EMBEDDING_DIM / 2 + 1`, i.e. 65); no
  assertion changes. Until approved, this is the only failing test of the workspace.
- **Host deployment**: `/var/lib/soos/models` on this host still holds the ArcFace deployment
  (root-owned). Without `SOOS_MODELS_DIR`, `scrfd_real_model_tests` (which runs
  `verify_integrity` over the whole committed manifest) fails on this host because
  `sface_2021dec.onnx` is not installed there; it passes with the scratch models directory and
  skips on CI runners. Fix: `sudo ./scripts/download_models.sh` (or `sudo ./scripts/install.sh`),
  then re-enroll every user (`sudo soos-enroll enroll -i <uid>` or the GUI). The old
  `arcface_w600k_mbf.onnx` is then reported as not attested and can be deleted.
- **Issue #296 checklist text (owner face session), Q10**: "- [ ] After deploying SFace
  (`sudo ./scripts/download_models.sh`) and re-enrolling, run `sudo soos-enroll verify -u <user>`
  at least 10 times per capture mode (RGB and, if used, IR) at the usual distance and lighting,
  record the genuine `match_score` values, and confirm they clear `match_threshold = 0.50` with
  margin (lowest score >= 0.55); also check that another person's face stays below 0.50.
  Report min / median / max only (no embedding or frame)."
- The ArcFace evaluation tests and the ArcFace arms of the LFW harness run only when the retired
  file is present next to the models (they skip otherwise).
- Phases 5 (candid review) and 7 (push, PR, merge) are not run in this batch.
