# Walkthrough 85 — PAD Threshold Security Floor and MiniFASNetV2 Class-Order Contract

- **Date**: 2026-09-30
- **Issues**: Review findings PAD-04 (GitHub #170) and PAD-05 (GitHub #171) — **Branch**: `fix/pad-threshold-and-class-contract`
- **Matrix criteria**: PTF1–PTF5 (new component `pad-threshold-floor-and-class-contract`)
- **ADRs**: `[2026-09-30] MiniFASNetV2 Class-Order Contract — One Canonical Statement`, `[2026-09-30] Operator Threshold Security Floor`

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) confirmed two MAJOR findings
in the anti-spoofing area:

- **PAD-04 (#170)** — `DaemonConfig::from_toml_str` built `[pipeline.thresholds]` with
  `ThresholdConfig::new_raw`, documented "without validation". `pad_threshold = 0` (or any negative
  value) made `OrtPadDetector` (`p_live >= threshold`) and `VisionPipeline::process_frame`
  (`score < pad_threshold`) accept every frame as live: anti-spoofing was disabled by a one-character
  configuration edit, with no warning. The same applied to `match_threshold`. The review verifier noted
  that routing through `ThresholdConfigBuilder::build` alone would not fix it, since `0.0` is inside
  `[0.0, 1.0]`: a security floor is needed.
- **PAD-05 (#171)** — the MiniFASNetV2 class order was stated four different ways across ADRs, docs,
  matrix, walkthroughs and code. Walkthrough 79 (#146) already aligned the code, `AI/ARCHITECTURE.md`,
  `Docs/INFERENCE_ORT_CRATE.md` and matrix rows PAD1/NGM9/ASG1 on index 1, but the drift remained in
  `AI/BACKLOG.md` (problem statement of #40, #40.2, NGM9 row "index 0 = Live"), in a quoted literal of
  the 2026-09-29 ADR, in `Docs/VISION_CRATE.md` §2.6 (`expand_bbox_for_pad` described as clamping,
  whereas `crop.rs` implements the Minivision inward-shift algorithm), and in walkthroughs 53/58/63/66.
  Nothing prevented the next agent from re-introducing a wrong index from any of these sources.

Objectives: fail closed on unsafe thresholds; state the class contract one way only and make drift a
build failure.

## 2. Architect Design

Code constants are the truth; prose follows them (`project-facts.md`).

`soos-policy` (`crates/policy/src/threshold.rs`, zero I/O, `#![forbid(unsafe_code)]`):

| Item | Kind | Contract |
|---|---|---|
| `ThresholdConfig::MIN_MATCH_THRESHOLD` | `pub const f32 = 0.40` | floor for operator match threshold |
| `ThresholdConfig::MIN_PAD_THRESHOLD` | `pub const f32 = 0.50` | floor for operator PAD threshold |
| `ThresholdConfigBuilder::build_with_security_floor(self)` | `-> Result<ThresholdConfig, PolicyError>` | `build()` (NaN / inf / `[0,1]`) then floors; error `PolicyError::InvalidThreshold { name, .. }` naming the setting |

`build()` keeps its domain-only contract (the existing test
`threshold_tests::test_threshold_builder_boundary_extremes` accepts `0.0` and is not touched).

`soos-daemon` (`crates/daemon/src/config.rs`): `[pipeline.thresholds]` goes through
`build_with_security_floor()`; the error maps to `DaemonError::Config("Invalid [pipeline.thresholds]: ...")`
so `load_from_path` / `load_or_default` fail and the daemon does not start. `pipeline.vision.*` is set
from the validated config. No `unsafe_thresholds` override: disabling biometrics is done with
`/etc/soos/disabled` (PAM returns `PAM_IGNORE`, password fallback), never by weakening the check.

Class contract: `[PrintPhoto, Live, ScreenReplay]`, live class index 1, owned by
`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX`. Historical lines kept for the record carry the marker
`*(Superseded ...)*`; numbered walkthroughs and dated reviews are immutable history (not scanned) but
get a banner.

Invariants affected: fail-closed configuration, "never convert an error into success", PAD integrity.
No change to the PAM module, the IPC protocol, or `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` itself.

## 3. Plan Evaluation

- Floors match the review recommendation (0.5 / 0.4) and are below the defaults (0.85 / 0.70), so no
  shipped configuration is refused (`test_defaults_satisfy_security_floor`, compile-time `const` assert).
- The only existing TOML thresholds in the repository (`config_tests.rs`: 0.75 / 0.90) satisfy the floors.
- `new_raw` stays available to `soos-policy` internals and tests; production use elsewhere is forbidden
  by invariant rather than by removing a public API other crates' tests might rely on.

## 4. Tester Contract (Red Evidence)

New test files (no existing test modified):

- `crates/policy/tests/threshold_floor_tests.rs` — 7 tests (PTF1). Red: compilation failure,
  `error[E0599]: no associated function or constant named MIN_MATCH_THRESHOLD / MIN_PAD_THRESHOLD`,
  `no method named build_with_security_floor found for struct ThresholdConfigBuilder` (11 errors).
- `crates/daemon/tests/threshold_config_tests.rs` — 13 tests (PTF2). Red: 11 failed, 2 passed
  (the two acceptance tests). Failing: `test_daemon_toml_rejects_pad_threshold_zero`,
  `..._pad_threshold_integer_zero_typo` (TOML integer `0` was coerced and accepted),
  `..._negative_pad_threshold`, `..._nan_pad_threshold`, `..._infinite_pad_threshold`,
  `..._pad_threshold_above_one`, `..._pad_threshold_below_floor`, `..._match_threshold_zero`,
  `..._match_threshold_below_floor`, `..._nan_match_threshold`,
  `test_daemon_toml_rejected_thresholds_fail_load_from_path` — each with
  "must be refused, but was accepted".
- `tests/invariants/src/pad_contract.rs` (new module of `soos-invariants`) — 4 tests. Red:
  - `test_thresholds_never_built_unvalidated_outside_policy` (PTF3):
    `crates/daemon/src/config.rs:311: soos_policy::ThresholdConfig::new_raw(match_thresh, pad_thresh);`
  - `test_minifasnet_class_contract_prose_matches_code_constant` (PTF4):
    `AI/DECISIONS.md:22: states live class index 0`, `AI/BACKLOG.md:1482: states live class index 0`,
    `AI/BACKLOG.md:1701: states live class index 0`.
  - `test_pad_matrix_rows_cite_existing_tests` (PTF5): `AI/VERIFICATION_MATRIX.md must contain row PTF1`
    (also guards the ASG1 regression where two non-existent tests were cited as evidence).
  - `test_class_contract_claim_extractor_detects_every_phrasing`: green from the start by design; it
    self-tests the extractor on every historical phrasing (`[Live, Print, Replay]`, `Class 0 = Live`,
    `index 0 = Live`, `live class index **1**`, `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX = 2`, `... (1, `)
    so the documentation scan cannot pass vacuously.

## 5. Auditor Constraints

1. No `unwrap`/`expect`/`panic` in production paths — the daemon uses `?` with `map_err`; the policy
   method returns `Result`. PAM crate untouched.
2. Error messages carry only the setting name, the numeric threshold and the floor — no biometric data,
   frames or credentials.
3. Fail closed: an invalid threshold is a start-up `DaemonError::Config`, never a silent fallback to
   defaults (a silent fallback would hide the operator's mistake).
4. `soos-policy` stays zero I/O and `#![forbid(unsafe_code)]`; the new invariant module inherits the
   crate-level `#![forbid(unsafe_code)]` and has zero external dependencies.
5. Bounded work: the invariant scans a fixed file list plus `Docs/` and `.agents/skills/`, skipping `target/`.

## 6. Implementation

- `crates/policy/src/threshold.rs`: `MIN_MATCH_THRESHOLD`, `MIN_PAD_THRESHOLD`,
  `ThresholdConfigBuilder::build_with_security_floor`; `new_raw` doc now points to the invariant.
- `crates/daemon/src/config.rs`: `[pipeline.thresholds]` validated with `build_with_security_floor()`,
  mapped to `DaemonError::Config`; vision thresholds copied from the validated config.
- `tests/invariants/src/lib.rs`: registers `mod pad_contract;`.
- Documentation: `AI/DECISIONS.md` (two ADRs; the quoted obsolete literal in the 2026-09-29 entry
  reworded), `AI/BACKLOG.md` (NGM9 row corrected, #40 / #40.2 lines marked `*(Superseded ...)*`),
  `AI/VERIFICATION_MATRIX.md` (PTF1–PTF5), `Docs/POLICY_CRATE.md` (security floor section),
  `Docs/VISION_CRATE.md` (§2.6 and pipeline step 6 describe the Minivision inward shift),
  walkthroughs 53, 58, 63, 66 (supersession banner only).

## 7. Candid Review

`./scripts/candid_review.sh` (Layer 1) PASSED: no `unsafe` additions, no PAM panics, no async runtime in
PAM, no forbidden dependencies, no PAM prints, English only. The fingerprint-bound Layer 2 review is run
by the orchestrator when the branches are integrated.

## 8. Verification Results

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --locked --workspace --all-targets --all-features` | all new and existing tests green; see note |
| `./scripts/candid_review.sh` | PASSED |

Green: `threshold_floor_tests` 7/7, `threshold_config_tests` 13/13, `threshold_tests` 7/7,
`config_tests` 7/7, `soos-invariants` 37/37 (4 new).

Note: while other agents were building in parallel (load average 34–48), the timing-based mock tests of
`soos-camera-v4l` (`error_recovery_tests`, `shutdown_tests`, 50 ms sleeps) failed intermittently; that
crate is untouched by this change and the tests pass when the machine is idle.

## 9. Known Limitations / Follow-ups

- The empirical re-verification of the live class index on real captures with the real
  `minifasnet_v2_80x80.onnx` stays open under PAD-06 (GitHub #172); the invariant only guarantees that
  prose and code agree.
- `crates/inference-ort/tests/pad_tests.rs::test_pad_class_ordering_live_index_0` still carries the comment
  "MiniFASNetV2 class ordering: [Class 0 = Live, ...]". Its assertions exercise the test-only explicit
  index constructor and remain valid; the comment was left untouched under the test-immutability rule
  and test files are outside the scan.
- Outside the PAD rows, `AI/VERIFICATION_MATRIX.md` cites about twenty `module::test_name` references that
  do not exist in the workspace (for example `gdm_tests::test_gdm_enable_idempotent`,
  `dual_sensor_tests::test_sensor_preference_defaults_to_prefer_ir` in ASG3). They are out of scope
  here; widening `test_pad_matrix_rows_cite_existing_tests` to the whole matrix would catch them.
