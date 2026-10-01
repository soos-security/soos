# Walkthrough 161 — Second PAD Model Attestation (MiniFASNetV1SE), Disabled Until Calibrated

- **Date**: 2026-10-01
- **Issue**: GitHub #212 (review finding PAD-07), continues walkthrough 141 (groundwork)
- **Branch**: `fix/pad-v1se-attestation`
- **Matrix criteria**: PVA1–PVA11 (new component `pad-second-model-attestation`), VMX8 superseded
- **ADR**: 2026-10-01 "Second PAD Model Attested, Disabled Until Calibrated"

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

**Normalization mismatch (found, not fixed).** Upstream `ToTensor` does not divide by 255
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
`pad_real_model_tests` (contract tests are never edited), so it is left as PVA11 for the owner.

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

No existing test was modified.

## 6. Fused latency (PVA7)

`SOOS_PAD_V1SE_DIR=<dir> cargo test --release --locked --all-features -p soos-daemon --test pad_v1se_real_model_tests -- --ignored --nocapture`,
i7-13620H (16 threads, shared machine, load about 8), default registry threads, 640x480 synthetic
frame, mocked detector and extractor (PAD rejects the frame, so the time is crop + PAD):

| Build | Single V2 median / p95 | Fused V2 + V1SE median / p95 | Added (median) |
|---|---|---|---|
| release, run 1 | 2.615 / 2.859 ms | 6.002 / 6.494 ms | +3.39 ms |
| release, run 2 | 2.776 / 3.078 ms | 5.827 / 6.530 ms | +3.05 ms |
| debug (`cargo test`) | 13.878 / 15.430 ms | 28.242 / 31.065 ms | +14.36 ms |

Real member scores on the synthetic frame (soos `/ 255` convention): 2.7x V2 `p_live` 0.0056,
4.0x V1SE `p_live` 0.0273; the fused score is their mean. About +3 ms per frame fits the 900 ms
decision budget easily.

## 7. Remaining work for the owner

1. **Normalization decision (PVA11)**, before any calibration: keep `/ 255` or feed raw `[0, 255]`
   as upstream trained. Changing it means re-recording the `pad_real_model_tests` golden logits
   through an ADR, and recalibrating the single-model threshold too.
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
  `crates/daemon/tests/pad_v1se_real_model_tests.rs`, this walkthrough.
- Changed: `scripts/download_models.sh` (`--with-optional`), `tests/invariants/src/lib.rs`,
  `.gitignore`, `models/README.md`, `Docs/VISION_CRATE.md`, `Docs/INFERENCE_ORT_CRATE.md`,
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`.

## 9. Validation

`cargo fmt --all -- --check`; `cargo clippy --locked --all-targets --all-features -p soos-daemon -p soos-invariants -- -D warnings`;
`cargo test --locked --all-features -p soos-inference-ort -p soos-vision -p soos-daemon -p soos-invariants`;
`bash -n scripts/download_models.sh`; `python3 -m py_compile` on both scripts; `./scripts/candid_review.sh`.
