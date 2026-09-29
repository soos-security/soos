# Walkthrough 79 — PAD Live Class Index: Single Source of Truth

- **Date**: 2026-09-29
- **Issue**: Review finding PAD-01 (GitHub #146; also CAM-09, DMN-01, STO-01, VIS-01) — **Branch**: `fix/pad-live-class-index`
- **Matrix criteria**: PLC1, PLC2, PLC3 (new); ASG1, PAD1, NGM9 (wording and test-name corrections)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, base `fbb99c4`) confirmed a
CRITICAL defect on the only security-relevant path (PAM → daemon):

- `crates/daemon/src/pipeline.rs:189-193` built the anti-spoofing detector with
  `OrtPadDetector::new_with_class_index(pad_session, config.vision.pad_threshold, 2)`;
- `crates/enrollment-cli/src/service.rs:773` did the same with `new_with_class_index(pad_session, 0.80, 2)`;
- the crate contract `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` (`crates/inference-ort/src/pad.rs`) is `1`
  (class 0 = PrintPhoto, class 1 = Live, class 2 = ScreenReplay), and `soos-gui` used `OrtPadDetector::new`
  (default 1).

`interpret_probabilities` reads `p_live = probs[live_class_index]`, so the daemon and `soos-enroll`
scored the **ScreenReplay** column as liveness: a phone-screen replay could pass PAD at the lock screen,
a genuine face was reported as spoof, and the GUI (what the user calibrates with) disagreed with the daemon
(what PAM enforces). History: commit `084ce5c` (walkthrough 66) set the literal `2` in all three binaries;
`72469ba` (walkthrough 72) flipped the constant back to `1` but never touched the call sites; `94d9ebe`
(walkthrough 75) fixed only the GUI. No test bound the production wiring to the constant, and the ADR,
architecture, crate docs and matrix stated the class order four different ways (PAD-05).

Objectives:
1. Remove every production override of the live class index; `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` is the
   single source of truth consumed by the daemon, the enrollment CLI and the GUI.
2. Bind the production construction sites to the constant with tests that run in CI (no model file needed).
3. Add a repository invariant rejecting `new_with_class_index(` / `with_live_class_index(` outside tests.
4. Record the decision in a superseding ADR and align `AI/ARCHITECTURE.md`, `Docs/`, the matrix and the
   shared project facts, without rewriting history.

## 2. Architect Design

- **`soos-inference-ort`**: new public alias `SharedSession = Arc<Mutex<ort::session::Session>>`
  (`registry.rs`, re-exported from `lib.rs`) so downstream factories can name the session type without a
  direct `ort` dependency. `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` unchanged (`1`); its doc and the docs of
  `new_with_class_index` / `with_live_class_index` now state that the latter are test-only.
- **`soos-daemon`**: `pub fn build_pad_detector(pad_session: SharedSession, pad_threshold: f32) -> OrtPadDetector`
  in `pipeline.rs`, the sole PAD construction site, implemented as `OrtPadDetector::new(pad_session, pad_threshold)`;
  `initialize_pipeline` calls it with `config.vision.pad_threshold`.
- **`soos-enrollment-cli`**: `pub fn build_pad_detector(pad_session: SharedSession, liveness_threshold: f32) -> OrtPadDetector`
  in `service.rs`, used by `build_full_service` (threshold `0.80` unchanged, out of scope).
- **Test fixture**: `tests/fixtures/mod.rs::onnx::minimal_identity_model()` hand-encodes a valid ONNX
  `ModelProto` (one `Identity` node, `x[1,3] -> y[1,3]`, IR 7, opset 13) as protobuf bytes, so an ORT session
  can be created with `Session::builder().commit_from_memory(..)` in CI without any file under `/var/lib/soos`.
  `ort` is added as a **dev-dependency** of `soos-daemon` and `soos-enrollment-cli` (workspace version, no new
  runtime dependency, no duplicate version).
- No new configuration knob: the index is a model contract, not a tunable.
- Invariants touched: ASG1 (matrix), PAD1/NGM9 wording; fail-closed behavior of the vision pipeline is unchanged.
- Drift recorded: ADR 2026-09-20 ("Class 0 = Live, default 0"), `AI/ARCHITECTURE.md:188`,
  `Docs/INFERENCE_ORT_CRATE.md:12/114/147`, matrix PAD1/NGM9 (0) and ASG1 (cited non-existent tests),
  `models/README.md:37`.

## 3. Plan Evaluation

Condensed (review-issue branch, harness-driven): the plan was checked against `AI/ARCHITECTURE.md` §2
(fail-closed PAD), the `architect-agent` "one source of truth" rule and the model contract in
`project-facts.md` §4. Verdict: APPROVED. Key finding: the recommendation to expose the detector through
`VisionPipeline` (`Arc<dyn PadDetector>`) would require a downcast or a trait change; a per-crate factory
seam (`build_pad_detector`) plus the repository invariant gives the same guarantee with a smaller surface.

## 4. Tester Contract

| Test (path::name) | Acceptance line / matrix ID | Red evidence (failure message) |
|---|---|---|
| `crates/daemon/tests/pad_wiring_tests.rs::test_pipeline_pad_detector_uses_default_live_class_index` | PLC1 — daemon uses the crate default and `vision.pad_threshold` | `assertion 'left == right' failed: Daemon PAD detector must use DEFAULT_MINIFASNET_LIVE_CLASS_INDEX (single source of truth)  left: 2  right: 1` (Red obtained while the test still lived in `pipeline_init_tests.rs`, see flakiness check) |
| `crates/daemon/tests/pad_wiring_tests.rs::test_pipeline_pad_detector_classifies_replay_as_spoof` | PLC1 — class 2 dominant is a spoof, class 1 dominant is live through the daemon-built detector | Added with the move to a dedicated test binary; fails with `Screen replay column (class 2) must never be read as liveness by the daemon` when the seam is stubbed with the literal `2` (same stub as the enrollment counterpart) |
| `crates/enrollment-cli/tests/pad_wiring_tests.rs::test_enrollment_pad_detector_uses_default_live_class_index` | PLC2 — `soos-enroll` uses the crate default | `assertion 'left == right' failed: soos-enroll PAD detector must use DEFAULT_MINIFASNET_LIVE_CLASS_INDEX  left: 2  right: 1` |
| `crates/enrollment-cli/tests/pad_wiring_tests.rs::test_enrollment_pad_detector_classifies_replay_as_spoof` | PLC2 — class 2 dominant is a spoof, class 1 dominant is live through the CLI-built detector | `Screen replay column (class 2) must never be read as liveness by soos-enroll` |
| `tests/invariants/src/lib.rs::tests::test_no_pad_live_class_index_override_outside_tests` | PLC3 — no override in production code, constant defined once as 1, ≥ 3 `OrtPadDetector::new` sites | `Production code must not override the MiniFASNet live class index ...: crates/enrollment-cli/src/service.rs:794: OrtPadDetector::new_with_class_index(pad_session, liveness_threshold, 2)` / `crates/daemon/src/pipeline.rs:114: soos_inference_ort::OrtPadDetector::new_with_class_index(pad_session, pad_threshold, 2)` |

Red was obtained with the factory seams stubbed on the *current* behavior (literal `2` moved into
`build_pad_detector`), so every test failed on its assertion rather than on a missing symbol.

### Migrated existing tests
none — no existing test encoded the defect; `pad_tests::test_pad_class_ordering_live_index_0` still exercises
`interpret_probabilities` with an explicit index and is untouched.

### Flakiness check
The daemon wiring test was first written inside `pipeline_init_tests.rs`. During the gate runs the
pre-existing `test_daemon_startup_initializes_all_pipeline_components` (dispatcher `connection_timeout`
500ms) intermittently failed with `Read resp len failed: early eof`. To rule out an interaction with the
in-process ONNX Runtime session, the wiring tests were moved to their own test binary,
`crates/daemon/tests/pad_wiring_tests.rs` (same layout as the enrollment CLI); `pipeline_init_tests.rs`
is byte-identical to `origin/main`. Measured afterwards on a host with load average ≈ 35 on 16 cores
(other sessions compiling): `pipeline_init_tests` still failed 5/10 without any change to it, and passed
3/3 once the load dropped; `pad_wiring_tests` (daemon) passed 10/10. The flake is therefore
pre-existing and load-induced (500ms dispatcher timeout), not introduced by this change; it is listed
in §9 as a follow-up.

## 5. Auditor Constraints

| # | Constraint | How it was met |
|---|---|---|
| 1 | No `unwrap`/`expect`/`panic`/indexing in new production code | Both factories are a single infallible call to `OrtPadDetector::new`; `cargo clippy -D warnings` green |
| 2 | No `unsafe` (business crates forbid it; daemon lib denies undocumented unsafe) | No `unsafe` added; `scripts/candid_review.sh` audit 1 passes |
| 3 | No literal class index in production; only the constant via `OrtPadDetector::new` | Invariant `test_no_pad_live_class_index_override_outside_tests` scans `crates/*/src` with `#[cfg(test)]` blocks stripped |
| 4 | `ort` only as a dev-dependency, workspace version, no duplicate | `Cargo.lock` diff adds `ort` to the two packages' dependency lists only; `cargo deny` policy unaffected |
| 5 | Fixture ONNX bytes deterministic, no file I/O, no `/var/lib/soos` access | `minimal_identity_model()` is pure byte construction |
| 6 | No frames, embeddings or secrets logged; no new log lines | No logging added |
| 7 | Existing tests untouched (no weakening) | Only additions under `crates/*/tests`, `tests/fixtures`, `tests/invariants` |
| 8 | Fail-closed unchanged: PAD failure → `PadFailed` → Deny | `crates/vision` untouched; `vision::pad_tests` still pass |

Pre-existing violation noted, not fixed here: `soos-enroll` hardcodes the PAD threshold `0.80` while the
daemon uses `config.vision.pad_threshold` (default `0.85`); tracked with the PAD-04 threshold findings.
Clearance: CLEARED.

## 6. Implementation

Files changed:
- `crates/daemon/src/pipeline.rs` — `build_pad_detector` (sole PAD construction site, `OrtPadDetector::new`);
  `initialize_pipeline` uses it. The literal `2` is gone.
- `crates/enrollment-cli/src/service.rs` — `build_pad_detector`; `build_full_service` uses it. The literal `2` is gone.
- `crates/inference-ort/src/registry.rs`, `src/lib.rs` — `SharedSession` alias and re-export.
- `crates/inference-ort/src/pad.rs` — documentation only: single-source-of-truth note on the constant,
  test-only note on `new_with_class_index` / `with_live_class_index`.
- `crates/daemon/Cargo.toml`, `crates/enrollment-cli/Cargo.toml`, `Cargo.lock` — `ort` dev-dependency.
- `tests/fixtures/mod.rs` — `onnx::minimal_identity_model()`.
- `crates/daemon/tests/pad_wiring_tests.rs` (new), `crates/enrollment-cli/tests/pad_wiring_tests.rs` (new),
  `tests/invariants/src/lib.rs` — contract tests listed in §4. `pipeline_init_tests.rs` is unchanged.
- Docs: `AI/DECISIONS.md` (ADR 2026-09-29 appended; the 2026-09-20 entry is marked superseded, not deleted),
  `AI/ARCHITECTURE.md`, `AI/VERIFICATION_MATRIX.md` (PAD1, NGM9, ASG1 corrected; new `PLC` section),
  `Docs/INFERENCE_ORT_CRATE.md`, `Docs/ENROLLMENT_CLI.md`, `models/README.md`,
  `.agents/skills/dev-workflow/references/project-facts.md`.

Notable decisions:
- The explicit-index constructors are kept (tests use `interpret_probabilities` with explicit indices and
  the constructors remain useful for diagnostics) but are documented as test-only and rejected in
  production by the invariant. Removing them was not required by the issue and would widen the diff.
- The GUI site (`crates/gui/src/main.rs`) already used `OrtPadDetector::new`; it is unchanged and counted by
  the invariant's "at least three construction sites" check.
- `AI/BACKLOG.md:1482` (issue #26 specification text "Class 0 = Live") is historical specification and is
  left as written; the superseding ADR is the authoritative statement.

## 7. Candid Review

pending — the fingerprint-bound layer 2 review (`scripts/candid_subagent.sh`) is run by the release step,
outside this worktree. Layer 1 (`./scripts/candid_review.sh`) passed on the working tree (see §8).

## 8. Verification Results

```bash
cargo fmt --all                                                               # clean
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings  # exit 0
cargo test   --locked --workspace --all-targets --all-features                 # see counts below
./scripts/candid_review.sh                                                    # PASSED
```

Results on the final working tree:
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: `Finished`, zero warnings.
- `cargo test --locked --workspace --all-targets --all-features`: 105 test suites `ok`, 0 failed, 484 tests
  passed (including the 5 new contract tests: 2 in `crates/daemon/tests/pad_wiring_tests.rs`, 2 in
  `crates/enrollment-cli/tests/pad_wiring_tests.rs`, 1 in `tests/invariants/src/lib.rs`).
- `./scripts/candid_review.sh`: `Candid Review PASSED` (audits 1–7).
- `./run_tests.sh` (Docker PAM matrix): not run, `crates/pam` untouched.

## 9. Known Limitations / Follow-ups

- The empirical class order of the shipped `minifasnet_v2_80x80.onnx` on real captures (RGB and IR) is still
  not measured by any automated test; this change makes the whole product consistent with the crate
  contract (index 1, upstream convention, walkthrough 72 analysis) and is tracked by PAD-06 (GitHub #172).
  If that measurement ever contradicts index 1, the fix is one constant plus a new ADR entry, and the
  invariant guarantees it propagates to every binary.
- PAD-05 (GitHub #171): the class-order statements in the ADR, architecture, crate docs, matrix and model
  README are now aligned; the proposed invariant that greps the documents for agreement with the constant
  is not part of this change.
- `soos-enroll` still hardcodes its PAD threshold (`0.80`) independently of the daemon configuration.
- The Docker PAM matrix (`./run_tests.sh`) was not run: `crates/pam` is untouched.
- Pre-existing, load-induced flake: `pipeline_init_tests::test_daemon_startup_initializes_all_pipeline_components`
  (`crates/daemon`, unchanged here) fails with `early eof` when the host is heavily loaded, because its
  dispatcher `connection_timeout` is 500ms and the mock pipeline must answer within it. It should get a
  CI-safe budget (tester determinism rule: never assert wall-clock tighter than the product budget) in a
  dedicated `test/` branch.
