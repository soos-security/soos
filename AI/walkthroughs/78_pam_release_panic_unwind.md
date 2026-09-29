# Walkthrough 78 — Release Panic Strategy: Make `catch_unwind` Effective in the Shipped `pam_soos.so`

- **Date**: 2026-09-29
- **Issue**: Full project review finding PAM-01 / TCI-01 (GitHub #148) — **Branch**: `fix/pam-panic-unwind`
- **Matrix criteria**: PRU1, PRU2, PRU3, PRU4 (complement PA3, PA12, PFT1)

---

## 1. Context & Objectives

`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md` (base `fbb99c4`) rated PAM-01 CRITICAL: the root
`Cargo.toml` `[profile.release]` set `panic = "abort"`, and every packaging path
(`scripts/build_{deb,rpm,arch,packages}.sh`, `packaging/debian/rules`, `packaging/rpm/soos.spec`,
`packaging/arch/PKGBUILD`) as well as `tests/docker/test_suite.sh` builds `pam_soos.so` with that
profile. Under `panic = "abort"` the runtime never unwinds, so all three `catch_unwind` sites in
`crates/pam/src/lib.rs` (`authenticate_with_config`, `PamHooks::sm_authenticate`, `catch_c_entry`)
were no-ops in the shipped artifact: a panic — including an arithmetic overflow trapped by
`overflow-checks = true`, which the same profile enables — killed the PAM host process (gdm,
sudo, login, screen locker) with SIGABRT instead of degrading to `PAM_IGNORE` and password
fallback. `AI/ARCHITECTURE.md` invariant 5, the ADR "Panic Safety" and
`Docs/SECURITY_AND_QUALITY_GUIDELINES.md` §2 all claimed the opposite. PA3/PFT1 were marked
Verified only by `cargo test`, which runs under `[profile.test]` (always unwinding) and therefore
can never observe the defect.

Re-verification before any change:
- `Cargo.toml:126` — `panic = "abort"`; no `.cargo/config.toml`, no per-package override (Cargo
  forbids `panic` in `[profile.*.package.*]`).
- Scratch repro (`catch_unwind` around an out-of-bounds index): `rustc -C panic=unwind` prints
  `caught -> PAM_IGNORE (25)` and exits 0; `rustc -C panic=abort` exits 134 (SIGABRT).
- Docker (see §4): the release-built `.so` armed to panic aborted `pam_test_runner`
  (`Aborted (core dumped)`, exit 134) while `Cargo.toml` still said `abort`.

Objectives: (1) make `catch_unwind` effective in the artifact that packaging ships, (2) prove it
on that artifact — not under the test profile — and pin the profile so the defect cannot return,
(3) record the decision as an ADR and correct the documentation.

## 2. Architect Design

### Scope & blast radius
- `Cargo.toml` — `[profile.release] panic = "unwind"`.
- `crates/pam` — opt-in Cargo feature `fault-injection`; new module `src/fault_injection.rs`;
  `config::FaultInject` enum and `PamConfig::fault_inject` field; one `cfg`-gated call inside the
  `catch_unwind` region of `authenticate_with_config`.
- `tests/docker/test_suite.sh` — new matrix case T10.
- `tests/invariants` — three new invariant tests.
- Docs: `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `Docs/PAM_MODULE.md`,
  `Docs/PAM_DOCKER_TEST_MATRIX.md`, `Docs/CI_CD_AND_SECURITY.md`, `AI/DECISIONS.md`,
  `AI/VERIFICATION_MATRIX.md`, `.agents/skills/dev-workflow/references/project-facts.md`,
  comments in `.github/workflows/ci.yml` and `run_tests.sh`.
- No consumer of a changed public API: `PamConfig` literals in tests use `..Default::default()`.

### Decision: workspace-wide `unwind`, no separate PAM profile
The review offered two options: a dedicated `[profile.release-pam]` or a workspace-wide switch.
The workspace-wide switch was chosen because:
1. the panic strategy cannot be set per package, so a dedicated profile would have to be threaded
   through nine build scripts, specs and `install.sh` (`find_artifact` looks in `target/release`),
   and any plain `cargo build --release` would silently produce an aborting artifact with the same
   file name — a drift trap with a CRITICAL outcome;
2. `abort` bought nothing at the FFI boundary: Rust already aborts when a panic escapes an
   `extern "C"` function, so the defense-in-depth stated in walkthrough 11 is retained for free;
3. the Tokio daemon benefits from task-level panic isolation instead of a whole-process abort.
`overflow-checks = true`, `lto`, `codegen-units = 1` and `strip` are unchanged.

### Types & signatures
```rust
// crates/pam/src/config.rs (always compiled; same struct shape in every build)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultInject { Panic, Overflow }
pub struct PamConfig { /* existing fields */ pub fault_inject: Option<FaultInject> }
// `fault_inject=<panic|overflow>` is parsed only under #[cfg(feature = "fault-injection")];
// the production variant of `apply_fault_inject_arg` is a no-op.

// crates/pam/src/fault_injection.rs (compiled only with --features fault-injection)
pub const PANIC_PAYLOAD: &str = "fault injection: explicit panic requested by fault_inject=panic";
pub fn trigger(mode: Option<FaultInject>);   // None => returns; Panic => panic_any(String); Overflow => u64::MAX + 1
```
Sentinels: `fault_inject=`, `fault_inject=bogus`, `fault_inject` (no `=`) leave the field `None`.

### Invariants touched
ARCHITECTURE.md invariant 5 (panic degrades to password fallback), ADR Panic Safety, matrix
PA3/PA12/PFT1 (now also proven on the release artifact), packaging invariants (build commands must
use `[profile.release]`).

### Documentation drift / ADR
Walkthrough 11 recorded `panic = "abort"` as deliberate hardening; superseded by the new ADR
`[2026-09-29] Release Panic Strategy = unwind` in `AI/DECISIONS.md`.

## 3. Plan Evaluation

Condensed self-evaluation (review-issue branch, no `plan_evaluator_report.md`):
- Fail-closed preserved: the hook runs inside `catch_unwind` before any socket activity, so an
  armed panic can only reach `PAM_IGNORE`; T10 also asserts the wrong password is still rejected.
- Zero production exposure: the hook is compiled only under the opt-in feature; production builds
  ignore `fault_inject=` entirely (`without_feature::test_fault_inject_argument_without_feature_is_ignored`),
  and an invariant test forbids the feature in every packaging, install, Dockerfile and CI build path.
- `cargo test --all-features` (CI) enables the feature for the test binaries; that is harmless
  because the hook only fires on an explicit `fault_inject=` argument and is never shipped.
- Verdict: APPROVED.

## 4. Tester Contract

| Test (path::name) | Criterion | Red evidence (before the fix) |
|---|---|---|
| `tests/invariants::test_release_profile_unwinds_so_pam_catch_unwind_is_effective` | PRU1 | `SECURITY VIOLATION (PAM-01): [profile.release] must set panic = "unwind" ... (found: panic = "abort")` |
| `tests/invariants::test_pam_fault_injection_feature_is_opt_in_and_never_packaged` | PRU2 | `crates/pam/Cargo.toml must declare a [features] table for the fault-injection hook` |
| `tests/invariants::test_pam_docker_suite_proves_release_panic_returns_pam_ignore` | PRU4 (static pin) | `test_suite.sh must contain 'fault_inject=panic' for the release panic-safety case (T10)` |
| `crates/pam/tests/fault_injection_tests.rs::test_default_config_has_no_fault_injection`, `::test_fault_inject_unknown_mode_is_ignored`, `::without_feature::test_fault_inject_argument_without_feature_is_ignored` | PRU3 | `error[E0609]: no field 'fault_inject' on type 'PamConfig'` (exactly the specified API) |
| `crates/pam/tests/fault_injection_tests.rs::with_feature::{test_parse_cstrs_fault_inject_modes, test_fault_inject_panic_returns_pam_ignore_via_c_abi, test_fault_inject_overflow_returns_pam_ignore_via_c_abi, test_fault_inject_via_pam_hooks_returns_pam_ignore, test_fault_inject_never_yields_success, test_fault_inject_is_repeatable_in_same_process}` | PRU3 | same compile error (missing `fault_inject` / `FaultInject`) |
| `tests/docker/test_suite.sh` T10 (`fault_inject=panic`, `fault_inject=overflow`; valid and invalid password) | PRU4 | With the hook implemented but `panic = "abort"` still set: `test_suite.sh: line 344: 1242 Aborted (core dumped) /usr/local/bin/pam_test_runner "${FAULT_SERVICE}" testuser password123` → `T10 (panic) failed: PAM host process was killed by a signal (exit 134)` |

Migrated existing tests: none. Existing tests `panic_safety_returns_pam_ignore`,
`authenticate_catches_parse_argv_panics` and `test_c_abi_all_entry_points_panic_safe` are
unchanged; they remain valid for the test profile and are now complemented by T10.

Refinement during Green: the first version of the PRU1 build-line filter matched the usage text
`--skip-build  Skip cargo build step` in `scripts/build_packages.sh`; the filter was narrowed to
command lines (`trim_start().starts_with("cargo build")`) with the same assertions.

Flakiness: the new tests are static or in-process; T10 has no timing dependency.

## 5. Auditor Constraints

| # | Constraint | Applies to | Verified by | Status |
|---|---|---|---|---|
| 1 | No `unwrap`/`expect`/`panic!` added to PAM production code; the only deliberate panic is `panic_any` inside the feature-gated hook with a scoped `#[allow(clippy::panic, reason = ...)]` | `crates/pam/src/fault_injection.rs::trigger` | clippy `-D warnings` (with and without the feature), `test_pam_crate_has_no_unwraps_or_expects_in_production_code`, `scripts/candid_review.sh` | met |
| 2 | The hook must execute inside a `catch_unwind` region and before any socket activity | `crates/pam/src/lib.rs::authenticate_with_config` | `test_fault_inject_never_yields_success`, T10 | met |
| 3 | Feature never default, never referenced by packaging, install, Dockerfiles or CI build commands | `crates/pam/Cargo.toml`, `scripts/*`, `packaging/**`, `.github/workflows/ci.yml` | `test_pam_fault_injection_feature_is_opt_in_and_never_packaged` | met |
| 4 | Production builds must not parse `fault_inject=` (no way to arm the hook from `/etc/pam.d`) | `crates/pam/src/config.rs::apply_fault_inject_arg` (cfg(not(feature))) | `without_feature::test_fault_inject_argument_without_feature_is_ignored` | met |
| 5 | Panic payload is fixed and secret-free; no user data in syslog | `PANIC_PAYLOAD` | code review, existing `test_pam_crate_has_syslog_panic_logging_without_secrets` | met |
| 6 | The overflow mode must not trip the compiler's `arithmetic_overflow` deny lint and must be explicitly allowed for `clippy::arithmetic_side_effects` with a reason | `fault_injection.rs::overflow` (`black_box`) | clippy | met |
| 7 | `[profile.release]` keeps `overflow-checks = true`; `panic` declared exactly once as `"unwind"`; no `--profile` in packaging | `Cargo.toml`, build scripts | `test_workspace_cargo_toml_enforces_overflow_checks`, `test_release_profile_unwinds_so_pam_catch_unwind_is_effective` | met |
| 8 | T10 must load a distinct module file (`pam_soos_fault.so`) and its own services, and clean them up on every exit path so T1–T9 artifacts are untouched | `tests/docker/test_suite.sh` (`cleanup_fault_injection`, EXIT trap) | Docker run | met |
| 9 | CI/workflow change limited to comments; no new actions, permissions or interpolations | `.github/workflows/ci.yml` | diff review | met |

Pre-existing findings (not introduced here): `test_suite.sh` reuses a stale
`target/release/libpam_soos.so` when one exists locally (CI always starts from a clean checkout);
noted in §9.

Clearance: CLEARED.

## 6. Implementation

- `Cargo.toml`: `[profile.release] panic = "unwind"` with a comment pointing to the ADR, the
  review finding, the invariant test and T10.
- `crates/pam/Cargo.toml`: `[features] fault-injection = []`.
- `crates/pam/src/config.rs`: `FaultInject`, `PamConfig::fault_inject` (default `None`),
  `apply_fault_inject_arg` in two `cfg` variants.
- `crates/pam/src/fault_injection.rs`: `trigger` (`panic_any` / `u64::MAX + 1` under
  `black_box`), module-level rationale.
- `crates/pam/src/lib.rs`: `#[cfg(feature = "fault-injection")] pub mod fault_injection;` and
  the gated call at the top of the `catch_unwind` closure.
- `tests/docker/test_suite.sh`: T10 builds `cargo build --release -p soos-pam --features
  fault-injection --target-dir target/fault-injection`, installs it as `pam_soos_fault.so`, writes
  `/etc/pam.d/test-soos-fault-{panic,overflow}`, and for each mode asserts: valid password → exit 0
  and never ≥ 128; wrong password → non-zero and never ≥ 128. Cleanup is part of the EXIT trap.
- `tests/invariants/src/lib.rs`: helpers `toml_section`, `is_code_line`; tests PRU1/PRU2/PRU4.
- Docs and traceability as listed in §2.

## 7. Candid Review

Pending (to be run by the release loop: `./scripts/candid_subagent.sh --prepare` then the
fingerprint-bound reviewer). Layer 1 (`./scripts/candid_review.sh`) result is recorded in §8.

## 8. Verification Results

```bash
cargo fmt --all                                                        # clean
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings   # Finished, 0 warnings
cargo test   --locked --workspace --all-targets --all-features         # all suites green (see report)
./scripts/candid_review.sh                                             # Layer 1 green
./run_tests.sh                                                         # Docker Ubuntu matrix T1–T10 green
```

Docker (Ubuntu 24.04 sandbox, release profile, rustc stable in-container):
- Before the fix (profile `abort`): `T10 (panic) failed: PAM host process was killed by a signal
  (exit 134); catch_unwind is not effective in the release build.`
- After the fix (profile `unwind`): `T10 (panic) passed: in-module panic degraded to PAM_IGNORE,
  password fallback succeeded.` / `invalid password still rejected after in-module panic (exit 7)`,
  identical for `overflow`; `ALL IN-CONTAINER PAM MATRIX TESTS PASSED SUCCESSFULLY!`

## 9. Known Limitations / Follow-ups

- `tests/docker/test_suite.sh` still reuses an existing `target/release/libpam_soos.so` for T1–T9
  when one is present locally (pre-existing behavior; CI always builds from a clean checkout). T10
  always rebuilds its own variant, so the panic-safety proof cannot use a stale artifact.
- T10 asserts the observable PAM contract (no abort, fallback works); it does not read the
  container's syslog to check the `soos-pam: authentication panic caught` line, because the sandbox
  runs no syslog daemon. PA12 covers the logging path in-process.
- The multi-distribution matrix (`./run_tests.sh --matrix`, Fedora and Arch) was not executed
  locally for this change; the Ubuntu sandbox used by CI was. T10 is distribution-agnostic
  (it only depends on `pam_unix.so` and the detected modules directory).
- `AI/ARCHITECTURE.md` and `AI/DECISIONS.md` still quote the historical "200–250ms" figure
  (known drift, unrelated to this change).
