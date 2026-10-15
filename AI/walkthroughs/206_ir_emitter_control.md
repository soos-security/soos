# Walkthrough 206 — UVC Extension-Unit IR Emitter Control and Strobe Classification

- **Date**: 2026-10-13
- **Issue**: GitHub-only #378 (PR 1 of epic #353, active-IR anti-spoofing) — **Branch**: `feat/ir-antispoofing-mock`
- **Matrix criteria**: IRA1–IRA28 (IRA25–IRA28 pending)

## 1. Context & Objectives

soos read IR frames but never controlled the near-infrared emitter, and nothing characterized how an
IR stream is lit. Epic #353 turns the IR path into active presentation attack detection; this
first PR delivers the hardware-facing building blocks, fully testable without IR hardware (owner
decision 2026-10-12: the owner's PC has an RGB camera only; IR validation runs on another project
member's PC):

- an `IrEmitter` trait, a UVC extension-unit implementation (`UvcXuEmitter` over
  `UVCIOC_CTRL_QUERY`) and a `MockIrEmitter` behind the existing `mock-camera` feature;
- strobe classification of IR streams (`Steady`, `NativeAlternating`, `Controllable`, `Absent`)
  with synthetic fixtures;
- a bounded, fail-closed `[camera.ir_emitter]` daemon section (default off);
- `soos-admin camera ir-emitter probe|configure`.

Liveness cues, template modality, strict IR mode and the physical protocol are later PRs.

## 2. Architect Design

Spec `AI/architect_spec_ir_emitter_control.md` (three revisions). New modules of
`soos-camera-v4l`: `uvc_xu` (request codes, `XuControl` seam with a blanket `&mut T` impl,
`XuReadOnly`, bounded `probe_xu_controls`, `UvcXuDevice` with the only new `unsafe` block),
`ir_emitter` (`EmitterState`, `IrEmitter` with `capability`/`set`/`get`/`commanded`,
`UvcXuEmitter` with restore-on-drop), `ir_emitter_config` (types, parser, text entry points for
clients, bounded sysfs USB id reader), `strobe` (mean luma, classifier, `StrobeMonitor`) and
`ir_mock` (`mock-camera` only). `MockCameraManager` gained a private frame-source hook,
`new_ir` and `step_frame` (both `mock-camera` only). `soos-daemon` parses `[camera]` strictly into
`PipelineConfig::ir_emitter` and warns that `auto` is not acted upon. `soos-admin` gained
`camera ir-emitter probe|configure` (`crates/admin-cli/src/ir_emitter.rs`). Nothing was added to
`soos-frame`, so no `ir` feature was needed (macOS ADR 2026-10-13).

Assumed owner decisions (to confirm): AD-1 no blind writes and no known-device database yet;
AD-2 `mode` defaults to `off`, `auto` only after a successful `--test-on-device`, no runtime effect
yet; AD-3 the daemon-side VID:PID-vs-resolved-node start-up check lands with strict IR mode.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: round 1 REVISION_REQUIRED (2 major: missing `IrEmitter::get`;
`configure` defaulting to `auto` would enable untested controls; 14 minor incl. signal-mask scope,
TOML parsing in soos-admin, classifier boundaries, period-4 ambiguity, writer hardening, sysfs and
fstat races); round 2 REVISION_REQUIRED (1 major: the frozen mock produced no frame, fixed with
`step_frame`); round 3 **APPROVED**.

## 4. Tester Contract

`AI/tester_contract_ir_emitter_control.md`: 89 new tests: 82 in seven test targets, 5 invariants
(`ir_emitter_contract`) and 2 unit tests in `uvc_xu.rs`. Red: compile errors on exactly the specified
modules (`E0432` unresolved imports). No existing test was changed.

## 5. Auditor Constraints

`AI/auditor_constraints_ir_emitter_control.md` A1–A14, CLEARED: one documented ioctl `unsafe`
block; checked lengths and `u16::try_from`; open-time and post-open V4L2 checks; bounded scans;
read-only probe; device writes only under `--test-on-device` with restore on every path and
blocked termination signals; `auto` only after a successful test; fail-closed parsing without
echoing values; hardened atomic writer; no payload in logs; bounded sysfs reads; mocks gated;
auth path untouched.

## 6. Implementation

Files: `crates/camera-v4l/src/{uvc_xu,ir_emitter,ir_emitter_config,strobe,ir_mock}.rs` (new),
`crates/camera-v4l/src/{lib,mock,diagnostics}.rs`, `crates/camera-v4l/Cargo.toml` (dev-deps
`proptest`, `soos-test-fixtures`), `crates/daemon/src/config.rs`,
`crates/admin-cli/src/{ir_emitter.rs (new),args,main,lib,error,camera}.rs`,
`crates/admin-cli/Cargo.toml` (dev-dep feature `mock-camera`), `tests/fixtures/mod.rs`
(`ir_strobe`), `tests/invariants/src/{ir_emitter_contract.rs (new),lib.rs}`, `Cargo.lock` (dev
edges only, no new package), `Docs/CAMERA_V4L_CRATE.md`, `Docs/DAEMON.md`, `AI/DECISIONS.md`,
`AI/VERIFICATION_MATRIX.md`.

Notable decisions: the request code is computed locally (`_IOWR` arithmetic) because `v4l` does not
export its macro; `soos-admin` never parses TOML (camera-crate text entry points); the managed block
is replaced in place and the whole result must load like the daemon loads it; the signal guard is
created before the emitter so the original value is restored before a pending Ctrl-C is delivered.

## 7. Candid Review

Layer 2 (independent reviewer, `AI/candid_review_report.md`, fingerprint `202312bc…`) returned
**CHANGES_REQUESTED**:

1. MAJOR — any key under `[camera]` other than `ir_emitter` refused daemon start-up, while every
   other table ignores unknown keys; a config following `tests/physical/screensaver_test.md`
   (`[camera] idle_timeout_secs`) would have stopped face unlock after an upgrade. Fixed (spec
   R21): only `[camera.ir_emitter]` is strict; other `[camera]` keys are ignored with a start-up
   warning naming only the key (`ignored_camera_keys`, at most 8); the physical doc now says
   `[pipeline] idle_timeout_secs`; `Docs/DAEMON.md` and the ADR updated.
2. MINOR — `MAX_DEVICE_TEST_QUERIES` was documented, not enforced. Fixed (R22): `XuBudget`
   enforces it; the bound is the real maximum, 11 (2 pre-checks, 3 to open, 4 for on/off and
   read-backs, restore, drop-time retry).
3. MINOR — the opened descriptor was not proven to be the classified node. Fixed (R23): the
   classified node's `st_rdev` (read before and after classification) must equal the opened
   descriptor's (`ensure_same_node`).
4. SUGGESTION — `vetoes_allow` is false for `Stable(Absent)`: recorded in the ADR as an open
   decision for split item 4 (no behaviour change).

Tests were written first (five new tests, Red on the missing API); two of this PR's own new tests
were adjusted for the new contract (listed in the tester contract). A new layer 2 review must run
on the updated diff (the fingerprint changed); it was not run by this agent.

## 8. Verification Results

Private target directory, `CARGO_BUILD_JOBS=4`, `--test-threads=4`:

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --workspace --all-targets --all-features --no-fail-fast -- --test-threads=4` | 3638 passed, 0 failed, 6 ignored (after the candid-review fixes) |
| `ir_mock_camera_tests` 10 runs in a row | 10/10 |
| `cargo deny --locked check` (cargo-deny 0.20.2) | advisories, bans, licenses, sources ok |
| `./scripts/candid_review.sh` (layer 1) | PASSED |

## 9. Known Limitations / Follow-ups

- IRA25–IRA27 need a real Windows Hello IR module (project member's PC): threshold calibration,
  probe/configure on hardware, classification of real streams.
- IRA28 / epic line "VID:PID differs from the resolved node ⇒ refuse at start-up": split item 4.
- `--test-on-device` proves the control accepts the payloads, not that the emitter lights (no frame
  is captured by `soos-admin`); check visually.
- No known-device database until entries are verified on hardware (#353 split item 6).
- SIGKILL/SIGSTOP during `--test-on-device` can leave the last written value (replug or reboot).
