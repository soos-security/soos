# Walkthrough 100 — Guided Enrollment Fusion Safety and Sudo Enrollment Target

- **Date**: 2026-09-30
- **Issues**: Review findings STO-10 (GitHub #183) and STO-11 (GitHub #184)
- **Branch**: `fix/enroll-model-fusion-target`
- **Matrix criteria**: GEF1–GEF3, ETU1 (✅ Verified)
- **Deferred**: STO-09 (GitHub #182), see §7

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area Storage & CLIs)
confirmed two MAJOR findings in `soos-enrollment-cli` that this change resolves:

- **STO-10**: `GuidedEnrollmentSession` fused every sample that passed a one-sided pose gate:
  no identity-consistency check (a second person in front of the camera during
  TurnLeft/TurnRight/TiltUp contaminated the template), no finiteness check (a NaN sample produced
  a NaN template), and the documented upper pose bounds (yaw 25, pitch -25) and the TiltUp lower
  bound (-8; the code used -5) were not enforced.
- **STO-11**: without `--uid`/`--username` the target was `getuid()`, which is 0 under `sudo`,
  so `sudo soos-enroll enroll` silently enrolled the administrator's face as **root**.

Objectives: admit only finite, bounded, identity-consistent, in-range samples and fail closed
during fusion; never target root implicitly.

## 2. Architect Design

- `guided_enrollment`: public constants `MIN_SAMPLE_CONSISTENCY_COSINE = 0.5`,
  `MAX_SAMPLES_PER_STEP = 20`, `MAX_GUIDED_EMBEDDING_DIM = 2048`; private pose-range constants;
  new feedback variants `InvalidEmbedding`, `IdentityMismatch`, `PoseOutOfRange`. Admission order:
  liveness, centering, pose finiteness, pose range, embedding validity, identity consistency.
  Identity anchor: every accepted frontal sample (pairwise within the frontal set) plus, off-axis,
  the frontal mean direction. Comparing off-axis samples to the frontal anchor rather than to each
  other avoids rejecting a genuine left-vs-right pair (up to 50 degrees apart). If frontal samples
  are pairwise at least `c` apart in cosine, each is at least `c` from their mean, so the fusion
  re-check is consistent with admission. Fusion = mean direction of normalized samples
  (`t = m / ||m||`, `m = sum_i s_i / ||s_i||`); `compute_composite_embedding` re-validates every
  sample and fails closed.
- `args::resolve_default_target_uid(real_uid, sudo_uid, pkexec_uid)` (pure, testable without
  mutating the process environment): a non-root real UID is its own target; otherwise `SUDO_UID`,
  then `PKEXEC_UID`, when non-root; otherwise `EnrollmentCliError::TargetUserRequired`. Malformed
  values (empty, non-digit, longer than 10 characters, over `u32::MAX`, non-UTF-8) fail with
  `InvalidInvokerUid`. Explicit `--uid 0` / `--username root` remain the only way to enroll root.
  `verify`, `delete` and `import` share `resolve_target_uid` and inherit the same rule (their code
  is unchanged). `soos-enroll enroll` prints the resolved target UID before any capture.

## 3. Tester Contract (Red Phase)

Two new files; no existing test was modified:

- `crates/enrollment-cli/tests/guided_enrollment_consistency_tests.rs` (13 tests):
  NaN/Inf/zero/oversized/mismatched embeddings, orthogonal identity during the frontal step and
  every off-axis step (the composite must equal the frontal direction), yaw -80/+80/26,
  pitch -60/-6/-8, secondary rotation and roll limits, bounded sample count, non-finite pose.
- `crates/enrollment-cli/tests/enroll_target_tests.rs` (8 tests): `SUDO_UID`, `PKEXEC_UID`,
  precedence, no implicit root (absent or `0` invoker), malformed invoker UID, unprivileged caller,
  explicit root.

Red evidence (`cargo test` against the unmodified sources): unresolved imports
`resolve_default_target_uid`, `MIN_SAMPLE_CONSISTENCY_COSINE`, `MAX_SAMPLES_PER_STEP`,
`MAX_GUIDED_EMBEDDING_DIM`; missing variants `InvalidEmbedding` (x4), `IdentityMismatch` (x5),
`PoseOutOfRange` (x5), `TargetUserRequired` (x3), `InvalidInvokerUid`.

## 4. Auditor Constraints

1. No `unwrap`/`expect`/`panic` in production code (`unwrap_or` for the non-UTF-8 fallback only).
2. No embedding value is logged or printed; fusion errors are static strings.
3. All loops bounded: at most 20 samples per step, at most 2048 components, at most 20 pairwise
   cosine evaluations plus one anchor mean per sample.
4. Environment variables are only read (no `set_var`); parsing is strict and length-bounded.
5. Fail closed: fusion returns an error (never a non-finite or contaminated vector); the implicit
   target returns an error (never root).
6. `#![forbid(unsafe_code)]` untouched in `soos-enrollment-cli`.

## 5. Implementation (Green Phase)

- `crates/enrollment-cli/src/guided_enrollment.rs`: admission rules, bounded storage,
  mean-direction fusion with re-validation, documented constants.
- `crates/enrollment-cli/src/args.rs`: `resolve_default_target_uid`, strict `parse_invoker_uid`;
  `resolve_target_uid` delegates to them when no explicit target is given.
- `crates/enrollment-cli/src/error.rs`: `TargetUserRequired`, `InvalidInvokerUid`.
- `crates/enrollment-cli/src/main.rs`: target UID printed before capture.
- `crates/enrollment-cli/src/lib.rs`: re-export of `resolve_default_target_uid`.
- `crates/gui/src/app.rs`: guidance messages for the three new feedback variants (the banner
  match is exhaustive).
- `Docs/ENROLLMENT_CLI.md`: target resolution rules and the guided admission table.

## 6. Verification

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features --no-fail-fast
./scripts/candid_review.sh
```

All four pass. Targeted: `guided_enrollment_consistency_tests` 13/13, `enroll_target_tests` 8/8,
and the unchanged `guided_enrollment_tests` 1/1, `enroll_tests` 3/3, `scaffold_tests` 6/6.

## 7. Deferred: STO-09 (GitHub #182) — Blocked on a Test-Contract Decision

Fixing the recorded model provenance requires `soos-enroll enroll` to default to
`arcface_w600k_mbf` / `2.0.0`. Two existing contract tests pin the retired behaviour:

- `crates/enrollment-cli/tests/scaffold_tests.rs::test_cli_parse_enroll_subcommand_with_uid`
  asserts `enroll.model_id == "mobilefacenet"` and `enroll.model_version == "1.0.0"` for the
  parsed defaults (lines 34-35).
- `crates/enrollment-cli/tests/enroll_tests.rs` passes `"mobilefacenet"` explicitly and asserts it
  is recorded verbatim (compatible with an override-only design).

Under the zero test weakening rule the first assertion cannot be changed without an explicit
decision. The daemon-side refusal of foreign `model_id` templates is also held back: shipping it
while the CLI still records `mobilefacenet` would lock out every CLI-enrolled user. The complete
STO-09 implementation (manifest-derived defaults, `EnrollmentSummary::model_overridden` with a
warning, `pipeline::EMBEDDING_MODEL_ID` and `ConnectionDispatcher::with_expected_embedding_model`
answering `Unavailable` / `ModelUnavailable`, plus its tests) is kept on the local branch
`fix/enroll-model-provenance`, where the only failing test is the `scaffold_tests` assertion above.

## 8. Residual Risks & Follow-ups

- `MIN_SAMPLE_CONSISTENCY_COSINE = 0.5` is a conservative bound chosen from typical ArcFace
  behaviour; it should be confirmed on real captures across the documented pose range.
- The GUI still turns a fusion error into a silent `None` when saving a guided template; surfacing
  it is left to the GUI work stream.
- `SUDO_UID` is trusted as provided by `sudo`; only root can run `soos-enroll`, so the variable
  cannot widen privileges, but a root shell with a stale `SUDO_UID` will target that user
  (the target UID is printed before capture).
