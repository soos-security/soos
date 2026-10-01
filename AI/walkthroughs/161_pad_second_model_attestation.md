# Walkthrough 161 — Second PAD Model Attestation (MiniFASNetV1SE), Disabled Until Calibrated

- **Date**: 2026-10-01
- **Issue**: GitHub #212 (review finding PAD-07), continues walkthrough 141 (groundwork)
- **Branch**: `fix/pad-v1se-attestation`
- **Matrix criteria**: PVA1–PVA13 (new component `pad-second-model-attestation`), VMX8 superseded, NGM8 reworded
- **ADR**: 2026-10-01 "Second PAD Model Attested, Disabled Until Calibrated" and 2026-10-01 "PAD Input Range Matches Upstream (0-255)"

---

## 1. Context and scope

Upstream Silent-Face-Anti-Spoofing decides on the mean of two softmax vectors: MiniFASNetV2 on a
2.7x context crop and MiniFASNetV1SE on a 4.0x crop. soos ships only the V2. Walkthroughs 117 and
141 already added the fusion code (`pad_fusion.rs`, `with_additional_pad_model`) and the daemon
wiring (`attach_optional_pad_members`, enabled when the deployed manifest declares
`minifasnet_v1se_pad`). Missing: a sourced, attested V1SE file, and evidence that the shipped V2
really is the upstream model (its source is a third-party ONNX fork).

No spoof corpus exists, so this branch delivers provenance, attestation, download, wiring
evidence and latency. Calibration is left to the owner and the model stays **disabled**.

## 2. Provenance

| Item | Value |
|---|---|
| Upstream | https://github.com/minivision-ai/Silent-Face-Anti-Spoofing, licence **Apache-2.0** (GitHub licence API and `LICENSE`) |
| Upstream commit | `b6d5f04ad78778917853b25c778acef6d5626d15` (HEAD of `master`, 2020-08-05) |
| `resources/anti_spoof_models/2.7_80x80_MiniFASNetV2.pth` | 1,849,453 bytes, SHA-256 `a5eb02e1843f19b5386b953cc4c9f011c3f985d0ee2bb9819eea9a142099bec0` |
| `resources/anti_spoof_models/4_0_0_80x80_MiniFASNetV1SE.pth` | 1,856,130 bytes, SHA-256 `84ee1d37d96894d5e82de5a57df044ef80a58be2b218b5ed7cdfd875ec2f5990` |
| `src/model_lib/MiniFASNet.py` | SHA-256 `e498c4ec5e1ddfaba62b941a126c19d65aa564999f3309661fe43ee8bf38acd7` |
| ONNX fork | https://github.com/QingHeYang/Silent-Face-Anti-Spoofing-onnx, licence **Apache-2.0**, commit `584d4421d7ac42c59e640796f46e886b0095367a` (2025-10-13) |
| Fork `onnx/2.7_80x80_MiniFASNetV2.onnx` | 1,744,126 bytes, SHA-256 `0cbe5cae...bfbccb8` = the shipped `minifasnet_v2_80x80.onnx` |
| Fork `onnx/4_0_0_80x80_MiniFASNetV1SE.onnx` | 1,743,294 bytes, SHA-256 `a25886a85cdcfa2c4ea23edb71de35f250c17827b4cadd253a972b28c80fdf1e` |

The weights were downloaded to `~/.cache/soos-eval/pad/` (outside the repository). Nothing
downloaded is committed; `.gitignore` now also covers `models/*.pth`.

## 3. Reproducible conversion and equivalence

`scripts/convert_pad_models.py` (new) pins the upstream commit and the SHA-256 of the three upstream
files, imports the upstream `MiniFASNet.py`, strips the DataParallel `module.` prefix like
`anti_spoof_predict.py`, and exports opset 11 with `input` `[batch, 3, 80, 80]` and `output`
`[batch, 3]` logits (no in-graph softmax), the same contract as the fork files and as
`OrtPadDetector` (NCHW, BGR, softmax computed in Rust, class order `[print, live, replay]`). It
refuses a work directory inside the repository. Venv: Python 3.12, `torch==2.5.1+cpu`,
`onnx==1.17.0`, `onnxruntime==1.20.1`, `numpy==2.1.3` (header documents the rerun).

```
2.7_80x80_MiniFASNetV2.onnx:     sha256 49f0072a...ca7156a size_bytes 1744126 max|torch-ort| softmax 1.192e-07
4_0_0_80x80_MiniFASNetV1SE.onnx: sha256 335ddec8...cbc072  size_bytes 1743294 max|torch-ort| softmax 1.192e-07
```

Two runs gave the same bytes. The SHA-256 differs from the fork files (exporter metadata), the size
does not.

`scripts/compare_pad_models.py` (new) runs a reference and a candidate graph with onnxruntime on
60 synthetic inputs (constants, gradients, checkerboards, seeded uniform and blocky noise) and 96
crops of public-domain / CC0 images (scikit-image v0.24.0 `astronaut`, `coffee`, `chelsea`,
`rocket`), in both input conventions, and compares all initializers:

| Pair | Weights | Softmax max abs diff | Argmax agreement |
|---|---|---|---|
| shipped V2 vs conversion of upstream 2.7 V2 | 137 initializers, 429,760 parameters, max delta **0** | **0** (raw and `/ 255`) | 156/156 in both conventions |
| fork V1SE vs conversion of upstream 4.0 V1SE | 149 initializers, 428,486 parameters, max delta **0** | **0** (raw and `/ 255`) | 156/156 in both conventions |

**The shipped fork model is numerically identical to upstream**, and so is the fork V1SE file.
The fork V1SE ONNX is therefore attested directly (stable, commit-pinned URL).

**Class order.** Because the graphs are identical, the index semantics are upstream's:
upstream `test.py` and the fork demo both treat `label == 1` as a real face, which matches
`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX = 1`. A purely empirical confirmation needs a bona fide
live capture, which was not used (no face data). The public-domain astronaut portrait (a photo,
not a live presentation) fuses to replay (index 2) at 2.7x / 4.0x, which is consistent but proves
nothing about index 1.

**Normalization mismatch (found here, fixed in §10 after owner approval).** Upstream `ToTensor` does not divide by 255
(`src/data_io/functional.py`: `# return img.float().div(255)  modify by zkx`), and the fork demo
documents `[0, 255]` inputs. `OrtPadDetector::prepare_input` feeds `pixel / 255.0`. Reference
argmax histograms `[print, live, replay]`:

| Model, inputs | raw `[0, 255]` | `/ 255` (soos today) |
|---|---|---|
| V2, synthetic (60) | 2 / 35 / 23 | 0 / 0 / 60, max `p_live` 0.0112 |
| V2, natural crops (96) | 2 / 52 / 42 | 0 / 0 / 96, max `p_live` 0.0078 |
| V1SE, synthetic (60) | 2 / 37 / 21 | 1 / 0 / 59, max `p_live` 0.2290 |
| V1SE, natural crops (96) | 0 / 67 / 29 | 0 / 0 / 96, max `p_live` 0.0310 |

With `/ 255` the inputs are close to black and the models answer "replay" whatever the content.
Non-face inputs say nothing about accuracy, but they show that the production input range is not
the training range. Changing it moves the PAD operating point and the golden logits recorded in
`pad_real_model_tests`, so it was escalated to the owner; it was approved and fixed (§10).

## 4. Attestation, disabled by default

`models/manifest.toml` is the enable switch (ADR 2026-09-30): `ModelRegistry::verify_integrity`
hashes every entry and `attach_optional_pad_members` wires any declared member. Adding the entry
there would enable the member, and a host without the file would refuse to start. The contracts
`test_optional_pad_members_disabled_by_default_in_shipped_manifest` and
`test_vmx_repository_manifest_declares_every_model_size` also pin the shipped manifest.

The entry therefore lives in the new `models/optional_models.toml`, in the manifest schema, with
the commit-pinned fork URL, the SHA-256, `size_bytes`, `input_layout = "NCHW"`, shapes, licence,
and informational provenance keys (upstream commit, weights URL and SHA-256, conversion script,
converted SHA-256). Both manifest parsers ignore unknown keys.

`scripts/download_models.sh --with-optional` (`--optional-manifest <path>`,
`SOOS_OPTIONAL_MANIFEST_PATH`) parses the optional file after the main one, with the same checks
(bare file name, 64-hex digest, https/file source, size bounds, ids unique across files), downloads
and verifies its files, and still deploys the main `manifest.toml`, so nothing is enabled. A run on
this host fetched the pinned URL and the SHA-256 matched (`4/4 models verified`).

Enabling (operator, after calibration): append the `[models.minifasnet_v1se_pad]` table (not the
`[manifest]` header) to `/var/lib/soos/models/manifest.toml` and restart `soos-daemon`.

## 5. Tests

- `tests/invariants/src/pad_second_model_attestation_contract.rs` (PVA1–PVA4). Red evidence: with the
  `main` download script and `.gitignore`, all 4 failed (no `--with-optional`, no `models/*.pth`
  rule); green after the change.
- `crates/daemon/tests/pad_v1se_real_model_tests.rs` (own binary):
  - PVA5: optional entry well-formed; appended to the shipped manifest it yields exactly
    `minifasnet_v1se_pad` at 4.0; the shipped manifest alone yields none.
  - PVA6: `pad_crop_window(…, 4.0, …)` equals upstream `CropImage._get_new_box` for 8 boxes. The
    table was generated by running the upstream `src/generate_patches.py` (commit b6d5f04). It
    passed at once, as expected after walkthrough 117's transcription; it is independent evidence
    for the 4.0x scale.
  - PVA7 (`#[ignore]`, `SOOS_PAD_V1SE_DIR`): real V2 + V1SE through `build_pad_detector`,
    `validate_pad_detector` and `attach_optional_pad_members` on a registry built from the shipped
    entry plus the optional table; fused score = mean of real member scores; latency.

No existing test was modified by the attestation commit (the input range fix of §10 changed tests under explicit owner approval).

## 6. Fused latency (PVA7)

`SOOS_PAD_V1SE_DIR=<dir> cargo test --release --locked --all-features -p soos-daemon --test pad_v1se_real_model_tests -- --ignored --nocapture`,
i7-13620H (16 threads, shared machine, load about 8), default registry threads, 640x480 synthetic
frame, mocked detector and extractor (PAD rejects the frame, so the time is crop + PAD):

| Build | Single V2 median / p95 | Fused V2 + V1SE median / p95 | Added (median) |
|---|---|---|---|
| release, run 1 | 2.615 / 2.859 ms | 6.002 / 6.494 ms | +3.39 ms |
| release, run 2 | 2.776 / 3.078 ms | 5.827 / 6.530 ms | +3.05 ms |
| debug (`cargo test`) | 13.878 / 15.430 ms | 28.242 / 31.065 ms | +14.36 ms |

| release, after the §10 fix (load about 3) | 1.493 / 1.686 ms | 3.140 / 3.403 ms | +1.65 ms |

Real member scores on the synthetic frame: with the former `/ 255` input 2.7x V2 `p_live` 0.0056
and 4.0x V1SE 0.0273; with the raw input (§10) 0.5395 and 0.0352. The fused score is their mean in
both cases. The input range does not change the cost; the run-to-run spread is machine load. About
+2-3 ms per frame fits the 900 ms decision budget easily.

## 7. Remaining work for the owner

1. ~~Normalization decision (PVA11)~~: decided and fixed (§10). The single-model threshold also needs
   the calibration below, because every earlier PAD number was measured on the `/ 255` input.
2. **Calibration (PVA9)** with the real camera, separately for the RGB and the IR sensor paths:
   - bona fide: at least 20 live captures per enrolled user (more is better), varied lighting and
     pose; also confirms index 1 = live on real faces;
   - attacks: at least 20 per species, printed photo (matte and glossy) and screen replay (phone and
     laptop, several brightness levels), at the distances the PAM flow sees;
   - run `pad_real_model_tests::test_real_pad_corpus_apcer_bpcer_under_ceilings` (single V2) and the same corpus
     through the fused pipeline; report APCER per species and BPCER at 0.85 and over a threshold
     sweep, pick the fused operating point, and check it against the `PadAggregator` consensus
     (3 of 5 frames).
3. **Enable** only if fusion is better at the chosen point: append the table to the deployed
   manifest, then wire `soos-enroll` / `soos-gui` the same way (PVA10).

## 8. Files

- New: `models/optional_models.toml`, `scripts/convert_pad_models.py`, `scripts/compare_pad_models.py`,
  `tests/invariants/src/pad_second_model_attestation_contract.rs`,
  `crates/daemon/tests/pad_v1se_real_model_tests.rs`, this walkthrough; §10 adds
  `crates/inference-ort/tests/pad_input_range_tests.rs`,
  `crates/daemon/tests/pad_nonface_pipeline_real_model_tests.rs` and
  `crates/daemon/tests/pad_live_camera_check_tests.rs`.
- §10 changes `crates/inference-ort/src/pad.rs`, `crates/inference-ort/tests/pad_tests.rs`,
  `pad_resample_tests.rs`, `pad_real_model_tests.rs` (owner-approved), `AI/ARCHITECTURE.md` and
  `.agents/skills/dev-workflow/references/project-facts.md`.
- Changed: `scripts/download_models.sh` (`--with-optional`), `tests/invariants/src/lib.rs`,
  `.gitignore`, `models/README.md`, `Docs/VISION_CRATE.md`, `Docs/INFERENCE_ORT_CRATE.md`,
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`.

## 9. Validation

`cargo fmt --all -- --check`; `cargo clippy --locked --all-targets --all-features -p soos-daemon -p soos-invariants -- -D warnings`;
`cargo test --locked --all-features -p soos-inference-ort -p soos-vision -p soos-daemon -p soos-invariants`;
`bash -n scripts/download_models.sh`; `python3 -m py_compile` on both scripts; `./scripts/candid_review.sh`.

## 10. PAD input range fixed to upstream `[0, 255]` (owner decision 2026-10-01)

**Approvals (owner, 2026-10-01, relayed by the coordinator):** (1) fix the PAD input to the raw
`[0, 255]` BGR range for every PAD member; (2) re-record only the golden constant values (and
their doc comment) of `pad_real_model_tests`; (3) migrate exactly five `prepare_input` range
assertions by dropping the `/ 255.0` divisor, tolerances and structure unchanged; (4) option A for
the checkerboard check, with a compensating pipeline-level test.

**Red first.** `crates/inference-ort/tests/pad_input_range_tests.rs`: before the fix
`test_pva11_pad_input_keeps_raw_0_255_values_in_bgr_order` failed (channel 0 was `50 / 255`) and
`test_pva11_real_pad_logits_reproduce_upstream_preprocessing` failed (soos grey logits
`[-3.709, -0.748, 4.459]` against upstream `[-1.893, 0.407, 1.486]`). The upstream references were
computed in Python with onnxruntime on the attested V2 and the upstream preprocessing (BGR, NCHW,
no division). The same script reproduces the old goldens exactly with `/ 255`, so the Python path
mirrors the Rust one. Both tests pass after the fix (within 1e-3).

**Fix.** `OrtPadDetector::prepare_input` (the only PAD preprocessing in the workspace; `grep` found
no copy in vision, daemon, enroll or gui) writes the bilinear sample itself instead of
`sample / 255.0`. V2 and V1SE share it.

**Golden logits (`pad_real_model_tests`, `[print, live, replay]`).**

| Input | Old (`/ 255`, 2026-09-30) | New (raw, 2026-10-01) | New argmax, `p_live` |
|---|---|---|---|
| `uniform_grey_80` | `[-3.709_296_7, -0.748_328, 4.458_604_3]` | `[-1.892_888_1, 0.407_292_2, 1.485_569_5]` | 2, 0.2475 |
| `gradient_80` | `[-3.634_143_4, -0.763_562_4, 4.398_681]` | `[-4.305_515, 0.249_781_67, 4.055_547]` | 2, 0.0218 |
| `checkerboard_160` | `[-3.600_257_9, -0.737_216_3, 4.338_425_6]` | `[-2.284_479_6, 4.028_476, -1.746_661_3]` | 1, 0.9951 |

**Migrated range assertions (only the `/ 255.0` divisor dropped).**

| Test | Old expected | New expected |
|---|---|---|
| `pad_tests::test_pad_prepare_input_80x80_bgr` | B `64.0 / 255.0`, G `128.0 / 255.0`, R `1.0` (tol 1e-4) | B `64.0`, G `128.0`, R `255.0` (tol 1e-4) |
| `pad_tests::test_pad_normalization_0_1_range` | every value in `0.0..=1.0`; white `1.0` (tol 1e-4); black `0.0` | every value in `0.0..=255.0`; white `255.0` (tol 1e-4); black `0.0` (unchanged) |
| `pad_resample_tests::test_pad_prepare_input_downscale_uses_half_pixel_centres` | R `(2x + 0.5) / 255`, G `(2y + 0.5) / 255` (1e-5), B `200 / 255` (1e-6) | R `2x + 0.5`, G `2y + 0.5` (1e-5), B `200.0` (1e-6) |
| `pad_resample_tests::test_pad_prepare_input_upscale_clamps_to_border` | `clamp(...) / 255.0` (1e-5) | `clamp(...)` (1e-5) |
| `pad_resample_tests::test_pad_prepare_input_80x80_is_identity` | `x / 255`, `y / 255` (exact) | `x`, `y` (exact) |

The assertion messages were updated to say "raw [0, 255] range". Test names, structure and
assertion counts are unchanged. Absolute tolerances are unchanged, so they are stricter relative to
the new values.

**Narrowed check (option A).** In
`pad_real_model_tests::test_real_ort_pad_detector_score_matches_live_class_softmax`, the old check
`assert!(!result.is_live, "... must never be accepted as live")` stays for `uniform_grey_80` and
`gradient_80`. For `checkerboard_160` only, it is now the existing threshold rule
`assert_eq!(result.is_live, p_live >= SHIPPED_PAD_THRESHOLD)`, with a comment. The old pass
relied on the `/ 255` bug; MiniFASNet alone scores that checkerboard live (0.995).

**Compensating test (PVA12).** `crates/daemon/tests/pad_nonface_pipeline_real_model_tests.rs`, gated
on `SOOS_MODELS_DIR` like the other real-model tests, runs 640x480 synthetic frames (uniform grey,
tiled gradient, checkerboards of 10/20/25/30/40 px and one aligned on the 2.7x PAD window) through
real SCRFD, real V2 and real ArcFace: every frame ends in `NoFaceDetected`. Red check: the same
frames with detection bypassed (mock face box) give a live PAD verdict on the window-aligned
checkerboard (`p_live` 0.9951, the `checkerboard_160` fixture after crop and resize). The test
asserts that control, so it would fail if the detector stopped being the gate.

**Thresholds on non-personal inputs after the fix** (`scripts/compare_pad_models.py --threshold
0.85`, V2 as reference, V1SE as candidate, same crop for both, so "mean" approximates fusion):

| Inputs | V2 `p_live >= 0.85` | V1SE `>= 0.85` | Mean `>= 0.85` | V2 argmax `[print, live, replay]` |
|---|---|---|---|---|
| synthetic (60) | 32 | 32 | 32 | 2 / 35 / 23 |
| public-domain crops (96) | 37 | 44 | 37 | 2 / 52 / 42 |

The public-domain astronaut portrait (a photograph) at the real 2.7x / 4.0x geometry fuses to replay
(`p_live` 0.15-0.38, below 0.85). On a static input every frame is the same, so the 3-of-5
`PadAggregator` consensus gives the single-frame decision. Conclusion: MiniFASNet output is only
meaningful on detected faces. Without a face detection the pipeline never reaches PAD (PVA12).
The 0.85 threshold and the consensus are unchanged, but **real-face calibration is still pending**
(PVA9): every number above comes from non-face or photographed inputs.

**Owner live-camera check (PVA13).** `crates/daemon/tests/pad_live_camera_check_tests.rs` (ignored)
captures frames from the camera, runs real SCRFD and prints aggregate statistics only (counts,
`p_live` min / median / max, fraction at or above 0.85, argmax counts) for V2 alone at 2.7x and,
when the V1SE file is given, for the fused mean of V2 2.7x and V1SE 4.0x. Frames, RGB buffers and
crops stay in memory (`Zeroizing`), nothing is written, no embedding is computed. No root is needed.
`soos-enroll verify` was not used: it needs root and an enrolled template, captures one frame and
runs single-model PAD only. Exact command (the V1SE file is the attested fork file, copied as
`minifasnet_v1se_80x80.onnx` into `~/.cache/soos-eval/pad/v1se/`, SHA-256 `a25886a8...`):

```bash
cd <repository> && SOOS_PAD_LIVE_CAMERA=/dev/video0 SOOS_PAD_V1SE_DIR=$HOME/.cache/soos-eval/pad/v1se \
  cargo test --release --locked --all-features -p soos-daemon --test pad_live_camera_check_tests \
  -- --ignored --nocapture
```

Optional: `SOOS_PAD_LIVE_FRAMES=<1..600>` (default 60), `SOOS_MODELS_DIR` (default
`/var/lib/soos/models`). The tool was run once on this host to check that it works. It detected
one face in all 60 frames, so the scene was not empty. Aggregates only: V2 `p_live` min 0.908,
median 0.999, max 1.000, 60/60 at or above 0.85; fused min 0.561, median 0.996, 57/60 at or above
0.85; argmax live 60/60 for both. The agent cannot tell whether that face was a live person or a
picture, so these numbers are not calibration evidence. The owner should run it under a controlled
protocol (live face, then a print and a screen replay).

**Not done.** `soos-enroll` and `soos-gui` wiring of the second member (PVA10). Shipping
`optional_models.toml` through install / packaging (owner: not now, follow-up). The V2 `source_url` in
`models/manifest.toml` is now pinned to fork commit `584d442` (no test pins the URL string; the
file at that commit has the attested SHA-256, verified by download in §4).

**Validation (after §10).** `cargo fmt --all -- --check`; clippy `-D warnings` on the affected
packages; `cargo test --locked --all-features -p soos-inference-ort -p soos-vision -p soos-daemon
-p soos-enrollment-cli -p soos-gui -p soos-invariants`; the real-model tests with
`SOOS_MODELS_DIR=/var/lib/soos/models` including the ignored PVA7; `ORT_SKIP_DOWNLOAD=1 cargo check
--locked -p soos-inference-ort --all-targets --all-features --target i686-unknown-linux-gnu`;
`./scripts/candid_review.sh`.
