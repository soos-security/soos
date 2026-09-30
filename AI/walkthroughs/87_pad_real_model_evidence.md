# Walkthrough 87 — PAD Real-Model Evidence (No More Tautological APCER / BPCER)

- **Date**: 2026-09-30
- **Issue**: Review finding PAD-06 (GitHub #172) from `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md` — **Branch**: `test/pad-real-model-evidence`
- **Matrix criteria**: PAD5 (reworded), PAD8 (new), PAD9 (new, pending), PH5 (downgraded to pending)

---

## 1. Context & Objectives

The review (PAD-06, MAJOR, verifier CONFIRMED) found that every PAD "FAR/FRR" and "APCER/BPCER"
claim in the project was tautological or fabricated:

1. `crates/vision/tests/pad_tests.rs::test_pad_far_frr_benchmark` scripted `MockPadDetector`
   verdicts (`PadResult::live(0.95)`, `spoof(0.04, …)`) and then "measured" 0 % FAR/FRR — it
   measured the mock.
2. `tests/physical/adversarial_test.sh --mock` hard-coded `BONA_FIDE_ACCEPTED=10` and
   `ATTACKS_REJECTED += 5` three times, then always printed
   `PAD SECURITY AUDIT PASSED: 100% of presentation attacks successfully rejected!`.
3. `AI/VERIFICATION_MATRIX.md` PAD5 and PH5 were `✅ Verified` on that basis, while BACKLOG PHY2
   ("PAD rejects printed photos and screen replays on real hardware") is still pending.
4. No test ever ran the real `minifasnet_v2_80x80.onnx` session, although the models are installed
   on the development host at `/var/lib/soos/models`.

Objectives: exercise the real model wherever it is installed, provide a corpus-driven APCER / BPCER
harness with hard ceilings, stop the mock script from reporting security metrics, rename the mock
benchmark to what it is, and correct the matrix. Per the verifier's correction, **no face crop is
committed** (biometric data at rest); the corpus lives in `SOOS_PAD_CORPUS_DIR` and only derived
numbers may enter the repository.

## 2. Architect Design

Scope is test, script and documentation only. No production code changes, so the PAD format work
for IR frames running in parallel (`crates/vision`, `crates/inference-ort/src`) is not touched.

New test target `crates/inference-ort/tests/pad_real_model_tests.rs`:

```rust
const PAD_MODEL_ID: &str = "minifasnet_v2_pad";
const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
const SHIPPED_PAD_THRESHOLD: f32 = 0.85;   // == ThresholdConfig::DEFAULT_PAD_THRESHOLD
const APCER_CEILING: f64 = 0.05;           // per attack species
const BPCER_CEILING: f64 = 0.10;
const MIN_SAMPLES_PER_CLASS: usize = 20;
const MAX_CROP_EDGE: usize = 1024;         // bounded corpus I/O
const GOLDEN_LOGIT_TOLERANCE: f32 = 1e-3;

fn load_real_pad_session(test: &str) -> Option<SharedSession>; // ModelRegistry + committed manifest
fn raw_logits(session, rgb, w, h) -> Vec<f32>;                  // OrtPadDetector::prepare_input + run
enum CorpusClass { BonaFide, Print, Screen }                    // dirs bona_fide/ print/ screen/
struct PadErrorCounts { … }  fn apcer() -> (print, screen); fn bpcer();
fn parse_ppm(bytes) -> Result<(Vec<u8>, u32, u32), String>;    // P6, maxval 255, bounded
```

Gating semantics:
- PAD model file absent from `SOOS_MODELS_DIR` (default `/var/lib/soos/models`) → print `SKIPPED`
  and pass, so CI runners without models stay green. `SOOS_REQUIRE_REAL_MODELS=1` → hard failure.
- Model present but SHA-256 differs from the committed `models/manifest.toml` → hard failure (the
  evidence must come from the attested artifact).
- `SOOS_PAD_CORPUS_DIR` unset → corpus test `SKIPPED`. Set without a model → failure. Set with fewer
  than 20 crops in any class → failure. An empty class yields a 100 % error rate, never 0 %.

A runtime skip was chosen over `#[ignore]` so that the model-only checks actually run inside the
standard `cargo test --workspace --all-targets --all-features` gate on any host with models installed.

`tests/physical/adversarial_test.sh --mock`: runs the vision plumbing tests and the real-model target,
prints `SIMULATION – no security metrics` and exits 0 before the report section. The physical
branch keeps its counters, and its success message no longer claims "100%".

## 3. Tests Written First (Red Evidence)

- `pad_real_model_tests::test_real_pad_model_golden_logits_on_synthetic_inputs` with `NaN` golden
  placeholders failed against the real model:
  `uniform_grey_80: logit[0] = -3.7092967 drifted from golden NaN (tolerance 0.001)`.
- `soos-invariants::tests::test_adversarial_mock_mode_never_fabricates_pad_metrics` failed on the
  unmodified script: `adversarial_test.sh --mock must print 'SIMULATION – no security metrics'`.

The golden values were then recorded from the real run; the invariant turned green only after the
script was changed.

## 4. Audit Constraints

1. No biometric data enters git: goldens are computed from synthetic patterns only; the corpus
   directory is external and its crops are never read into committed fixtures.
2. Bounded corpus I/O: each file is read through `Read::take(1024·1024·3 + 256 + 1)`, edges are
   capped at 1024 px, at most 10 000 files per class, 16-bit and non-P6 PPMs are rejected.
3. No silent pass on a real measurement: missing model with a corpus, too few samples, or an empty
   class all fail.
4. Attestation: the session is loaded through `ModelRegistry::get_or_load_session`, which verifies
   the SHA-256 against the committed manifest before instantiating ORT.
5. Test weakening: `test_pad_far_frr_benchmark` was renamed only; its body and every assertion are
   byte-identical (only the leading comment was rewritten).

## 5. Implementation

- `crates/inference-ort/tests/pad_real_model_tests.rs` (new, 8 tests).
- `crates/vision/tests/pad_tests.rs`: rename to
  `test_pad_pipeline_plumbing_routes_mock_verdicts_without_leaks`.
- `tests/physical/adversarial_test.sh`: simulation branch, help text, physical success message.
- `tests/invariants/src/lib.rs`: `test_adversarial_mock_mode_never_fabricates_pad_metrics`.
- `Docs/INFERENCE_ORT_CRATE.md`: "PAD real-model evidence" and corpus capture protocol.
- `AI/VERIFICATION_MATRIX.md`: PAD5 reworded to plumbing only, PAD8 and PAD9 added, PH5 pending.

## 6. Real-Model Output (local host, `/var/lib/soos/models`, 2026-09-30)

```
REAL PAD MODEL metadata: input 'input' [-1, 3, 80, 80], output 'output' [-1, 3]
REAL PAD MODEL golden uniform_grey_80: logits [-3.7092967, -0.748328, 4.4586043], argmax 2, softmax [0.00028198786, 0.005447069, 0.9942709]
REAL PAD MODEL golden gradient_80: logits [-3.6341434, -0.7635624, 4.398681], argmax 2, softmax [0.0003226767, 0.005694363, 0.993983]
REAL PAD MODEL golden checkerboard_160: logits [-3.6002579, -0.7372163, 4.3384256], argmax 2, softmax [0.0003543358, 0.0062060906, 0.9934396]
REAL PAD MODEL channel-swapped gradient: logits [-3.7463973, -0.77433896, 4.521746], max delta 0.123064995
REAL OrtPadDetector uniform_grey_80: is_live=false score=0.005447 attack=Some(ScreenReplay)
REAL OrtPadDetector gradient_80: is_live=false score=0.005694 attack=Some(ScreenReplay)
REAL OrtPadDetector checkerboard_160: is_live=false score=0.006206 attack=Some(ScreenReplay)
SKIPPED test_real_pad_corpus_apcer_bpcer: SOOS_PAD_CORPUS_DIR not set
test result: ok. 8 passed; 0 failed
```

Interpretation: the model's I/O contract matches the manifest; non-face synthetic inputs collapse to
class 2 (ScreenReplay) with `p_live` about 0.005, i.e. far below the 0.85 threshold. This confirms the
real session runs through the production detector, but it says **nothing** about APCER / BPCER on
faces, nor does it independently confirm that class 1 is "live" (that needs bona fide crops).

Corpus harness smoke check (not evidence): a throw-away corpus of 20 uniformly random 96×96 PPMs per
class, generated in the session scratch directory and deleted afterwards, produced
`APCER(print)=0.0000 APCER(screen)=0.0000 BPCER=1.0000` and failed with
`BPCER 1.0000 exceeds ceiling 0.1`, as expected for noise labelled "bona fide".

Skip path: `SOOS_MODELS_DIR=/nonexistent` → all model tests print `SKIPPED`, 8 passed;
with `SOOS_REQUIRE_REAL_MODELS=1` → 5 model tests fail, as designed.

## 7. Quality Gate

- `cargo fmt --all -- --check`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo test --locked --workspace --all-targets --all-features`
- `./scripts/candid_review.sh`
- `tests/physical/adversarial_test.sh --mock` → prints `SIMULATION – no security metrics`, exit 0.

## 8. Remaining Work (Needs the Owner)

- Capture a PAD corpus on the target laptop (>= 20 bona fide RGB + IR, >= 20 print, >= 20 screen
  replay) into a root-only directory outside the repository and run the corpus test; then record the
  resulting `GOLDEN …` lines and the measured APCER / BPCER, and move PAD9 / PH5 / BACKLOG PHY2 to
  verified only if the ceilings hold.
- A crop-export tool (for example a `soos-enroll` sub-command writing the 2.7× expanded PAD crop as
  PPM into a `0700` directory) does not exist yet; crops currently have to be produced out of band.
