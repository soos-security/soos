# Walkthrough 119 — Verify PAD Report and GDM Comment Parsing

- **Date**: 2026-09-30
- **Issues**: Review findings PAD-11 (GitHub #216) and STO-20 (GitHub #236) — **Branch**: `fix/p2-pad-verify-report`
- **Matrix criteria**: PVR1–PVR5 (new, ✅ Verified), component `enroll-verify-pad-report`
- **ADR**: 2026-09-30 "Diagnostic PAD Report Carries Score and Threshold" in `AI/DECISIONS.md`

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) confirmed three
defects in the operator diagnostics:

- **PAD-11**: `EnrollmentService::verify` matched `NoFaceDetected`, `MultipleFacesDetected` and
  `FaceBelowConfidence` but let `VisionError::PadFailed` fall through `Err(e) => Err(e.into())`.
  A spoof therefore surfaced as a generic error: the report (PAD score, threshold, latency) was
  never printed, although `Docs/ENROLLMENT_CLI.md` promised a PAD status. The same applied to
  `VisionError::IrLivenessGateFailed` on monochrome frames.
- **STO-20 (a)**: a live presentation reported the literal `pad_result: "PASSED"` although
  `PipelineOutput::pad_result` carries the real score, so operators could not see the liveness
  score that governs the daemon verdict.
- **STO-20 (b)**: `soos-admin gdm status` used `content.contains("pam_soos.so")`, so a file
  holding only `# auth sufficient pam_soos.so` reported `installed: true`.

## 2. Architect Design

- `DiagnosticVerificationReport` gains `pad_score: Option<f32>` (PAD model score, when it ran)
  and `pad_threshold: Option<f32>` (effective threshold for the frame modality, from
  `VisionPipelineConfig::effective_pad_threshold(PadInputModality::for_frame(&frame))`).
- `pad_result` stays the outcome class. The existing contract
  `verify_tests::test_verify_matching_user_reports_allow_and_metrics` pins `"PASSED"`, so the
  score is added as structured fields (extend, never weaken). `pad_status()` renders the operator
  line: `PASSED (score=0.930, threshold=0.85)`; a class that already carries `score=` is
  returned unchanged.
- New `verify` arms: `PadFailed { score, threshold }` → `Deny`, face count 1, match score 0,
  `SPOOF(score=<s:.3>, threshold=<t:.2>)`; `IrLivenessGateFailed { reason }` → `Deny`,
  `IR_GATE_REJECTED(<reason>)`, no score, effective IR threshold. Every other error still
  propagates.
- `soos-enroll verify` prints `report.pad_status()`; it still exits 1 on every non-`Allow`
  verdict, so `adversarial_test.sh` still counts a spoof as rejected.
- `soos_admin_cli::gdm::has_active_pam_soos_rule(content)` reuses the existing `PamLine::parse`
  (comments, blank and malformed lines are `None`; `module_name` takes the basename and excludes
  delegations). `get_gdm_status` reads the file with `read_bounded_utf8` (64 KiB bound, was an
  unbounded `read_to_string`) and uses it; `plan_gdm_enable` uses the same helper for its
  "administrator rule already present" check, so status and enable agree.

## 3. Tester Contract (Red Phase)

New files only; no existing test line changed.

- `crates/enrollment-cli/tests/verify_pad_report_tests.rs` (6 tests): spoof and low live score
  yield `Deny` with score; live reports the mock score (0.93) and `pad_status()`; no-face reports
  no score; Grey mock frame reports the IR threshold; a black Grey frame served by a
  `FixedFrameCamera` double is rejected by the IR gate and reported as `IR_GATE_REJECTED(..)`.
- `crates/admin-cli/tests/gdm_commented_rule_tests.rs` (5 tests): commented lines, a
  comment-only file and a mention in module arguments are not installed; a module path is; enable
  inserts `GDM_PAM_LINE` next to a commented line and preserves the comment.

Red evidence on `origin/main` (`e2f602c`):

- `cargo test -p soos-admin-cli --test gdm_commented_rule_tests`: 3 failed
  (`test_gdm_status_ignores_commented_pam_soos_line`,
  `test_gdm_status_only_comment_line_reports_not_installed`,
  `test_gdm_status_ignores_pam_soos_in_module_arguments`); the two enable/path tests passed
  (enable already parsed rules, they are regression guards).
- `cargo test -p soos-enrollment-cli --test verify_pad_report_tests`: does not compile
  (`no field pad_score`, `no field pad_threshold`, `no method pad_status`); on the old code a
  spoof returned an `Err` wrapping `VisionError::PadFailed` (review verifier probe).

## 4. Audit

- `#![forbid(unsafe_code)]` unchanged in `enrollment-cli` and `admin-cli`; no `unwrap`/`expect`
  added in production code; PAM module untouched.
- Bounded I/O: `gdm status` now reads at most `MAX_PAM_FILE_BYTES`; an oversized or non-UTF-8
  file reports `installed: false` (fail-safe for a status command).
- No sensitive data: only the PAD score/threshold (scalars) are printed; no frame, crop or
  embedding. A spoof never becomes `Allow`.

## 5. Green Phase & Gate

Minimal implementation in `crates/enrollment-cli/src/service.rs`, `crates/enrollment-cli/src/main.rs`
and `crates/admin-cli/src/gdm.rs`. Gate commands: `cargo fmt --all -- --check`,
`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast`,
`./scripts/candid_review.sh`.

## 6. Documentation

- `Docs/ENROLLMENT_CLI.md`: PAD status classes, score and threshold, exit status.
- `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1: `gdm status` counts active rules only.

## 7. Follow-ups

- Calibrating the PAD threshold on real RGB/IR hardware with the new score output needs a real
  camera and the production MiniFASNet model.
