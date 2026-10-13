# Walkthrough 201 — Distribution Builds Against the System ONNX Runtime

- **Date**: 2026-10-13
- **Issue**: GitHub #375 (PR 1 of #361, no backlog id) — **Branch**: `feat/system-onnxruntime` (not registered in `scripts/sync_issue.py`; the commit carries `Closes #375`)
- **Matrix criteria**: SORT1–SORT10, SORT-S1–SORT-S7
- **Owner decisions** (#361, 2026-10-12): maintainer identity `soos-security`; nothing is published until the owner says go. This change publishes nothing.

## 1. Context & Objectives

Official repositories and offline build farms (Arch `devtools`, Fedora `mock`, Launchpad, OBS,
Nix) build without network and forbid bundled prebuilt binaries, but `ort` downloaded a prebuilt
ONNX Runtime archive in its build script. #375 asks for: an opt-in feature linking the distribution
`libonnxruntime` through pkg-config at the lowest needed C API level, the default dev/CI build
unchanged, a fail-closed CPU execution provider check at start, a declared MSRV, an offline CI job
in a clean Arch container, and an ADR with the rejected alternatives and the trust change. Signed
releases, vendor tarballs, packages and channels are later PRs of #361.

## 2. Architect Design

Spec `AI/architect_spec_system_onnxruntime.md` (§10–§12 are the normative revisions).

- **Evidence first** (spec §0): `ort-sys` rc.13 links through pkg-config first when the feature is
  on, then `ORT_LIB_*`, then the download; without `download-binaries` no download code exists and a
  missing library is a link error. The workspace compiles at `api-17` (the floor of `ort` rc.13), so
  the system build needs ONNX Runtime ≥ 1.17: Arch `[extra]` ships 1.29.0 (queried in the pinned
  image), Fedora 43/44 1.20.1/1.22.2, Debian 1.19.2. `ort`'s handshake panics (`expect`) on an
  older library; `ort::ep::CPU::is_available` answers `true` without asking the library.
- **Features**: `download-binaries` (default, `ort/default`, unchanged) and `system-onnxruntime`
  (`ort/pkg-config`, `ort/api-17`) in the five ORT crates (`soos-inference-ort`, `soos-vision`,
  `soos-daemon`, `soos-enrollment-cli`, `soos-gui`); the workspace `ort` entry is
  `default-features = false` with `std`, `ndarray`, `tracing`, and the edges disable defaults.
  The default feature graph is byte-identical (`cargo tree` diff empty).
- **Build line**: `ORT_SKIP_DOWNLOAD=1 cargo build --locked --offline --release --workspace
  --no-default-features --features system-onnxruntime` (the literal #361 line would still compile
  the download code, raise the gate to API 27 and download silently without `ORT_SKIP_DOWNLOAD`).
- **Start-up check** (`crates/inference-ort/src/runtime.rs`): `verify_cpu_execution_provider()`
  → `check_cpu_execution_provider(|| execution_provider_listed(CPU_EXECUTION_PROVIDER))`;
  `execution_provider_listed` uses a never-registered `ListedProviderProbe` that keeps the default
  `is_available` (`GetAvailableProviders`, exact name); the pure check runs the probe inside
  `catch_unwind`; only `Ok(true)` passes, everything else is
  `InferenceError::CpuExecutionProviderUnavailable` (detail ≤ 256 bytes). Callers: daemon
  `initialize_pipeline` step 0, `soos-enroll` before key/store/camera/models, `soos-gui` before its
  registry.
- **MSRV**: `rust-version = "1.92"` (highest `rust-version` of the 516 locked packages, egui 0.35;
  1.92.0 builds, 1.88.0 fails in `epaint` even with `--ignore-rust-version`).
- **`--all-features`**: both features on; the workflow `env:` and `save.sh` set
  `LIBONNXRUNTIME_NO_PKG_CONFIG=1` so those gates keep the verified download only.
- ADR "[2026-10-13] Distribution Builds: System ONNX Runtime" appended to `AI/DECISIONS.md`.

## 3. Plan Evaluation

Round 1: `REVISION_REQUIRED` (M-1 an existing invariant pinned the plain `ort = "=2.0.0-rc."`
line; M-2 the guard could become a no-op without a failing test; M-3 stale base; minors m-4 mixed
link under `--all-features`, m-5 ADR reasons, m-6 the REQUIRE test list, m-7 partial MSRV coverage,
m-8 rolling release in the PR gate, m-9 `soos-enroll`/`soos-gui`, m-10 CPU only in source). The
branch was rebased on `6f32ef8` and the spec gained §10/§11. Round 2: `APPROVED` with minors n-1
(enroll check before key/store), n-2 (`save.sh` parity), n-3 (token-aware scan), n-4 (stale text),
all applied (§12). Report: `AI/plan_evaluator_report.md`.

## 4. Tester Contract

| Test (path::name) | Criterion | Red evidence |
|---|---|---|
| `crates/inference-ort/tests/system_onnxruntime_tests.rs::test_sort_check_accepts_listed_cpu_provider` | SORT1 | E0432 unresolved imports `check_cpu_execution_provider`, `verify_cpu_execution_provider`, `CPU_EXECUTION_PROVIDER`, `MAX_RUNTIME_ERROR_DETAIL_BYTES`, `soos_inference_ort::runtime` |
| `…::test_sort_check_rejects_unlisted_cpu_provider` | SORT2 | same + E0599 no variant `CpuExecutionProviderUnavailable` |
| `…::test_sort_check_rejects_probe_error_with_bounded_detail` | SORT3 | same |
| `…::test_sort_check_catches_handshake_panic` | SORT4 | same |
| `…::test_sort_linked_runtime_lists_cpu_provider` | SORT5 | same |
| `…::test_sort_constants` | SORT6 | same |
| `…::test_sort_provider_probe_asks_the_linked_library` | SORT9 | same |
| `crates/daemon/tests/system_onnxruntime_startup_tests.rs::test_sort_daemon_pipeline_starts_on_linked_runtime_with_real_models` | SORT7 | evidence test (green before the change; its Red counterpart is SORT-S4) |
| `tests/invariants/src/system_onnxruntime_contract.rs` (15 tests) | SORT-S1–S7 | 11 of 14 failed before implementation (`test_sort_s1_ort_stays_cpu_only`, `test_sort_s3_default_feature_graph_unchanged`, `test_sort_s6_pam_graph_has_no_onnx_runtime` are regression guards that already held); the token-scan self-test and the `save.sh` clause were added in revision 2 |

Migrated existing test (spec §11):

| Test | Old assertion | New assertion | Mandating acceptance line |
|---|---|---|---|
| `tests/invariants/src/dependency_tooling_contract.rs::test_prerelease_workspace_dependencies_are_exact_pinned` | `section.contains("ort = \"=2.0.0-rc.")` | the `ort = ` line of `[workspace.dependencies]` contains `version = "=2.0.0-rc.` | #375/#361 "build `ort` with `default-features = false`" (inline table); the exact rc pin is kept |

Flakiness: the new tests are deterministic (no timing); none is timing-sensitive.

## 5. Auditor Constraints

`AI/auditor_constraints_system_onnxruntime.md`, 27 constraints, `Clearance: CLEARED`. Notable and
how they were met: C2 probe by name (`ListedProviderProbe`, `execution_provider_listed`); C3/C4/C5
fail-closed `catch_unwind`, payload truncated before the prefix is joined (`prefix_within`), only
`&str`/`String` payloads rendered; C7/C9/C10 call sites (daemon step 0, enroll before
`MasterKey::load_or_create` under `!cli.mock`, GUI before `ModelRegistry::new`); C15 one-line
`Cargo.lock` delta (`pkg-config` edge on `ort-sys`), `deny.toml` untouched; C16 MSRV in all 18
members; C17 workflow env + `save.sh` export, pinned by SORT-S5; C19 explicit `-e` list, asserts
`LIBONNXRUNTIME_NO_PKG_CONFIG`, `PKG_CONFIG_PATH/LIBDIR/SYSROOT_DIR`, `ORT_LIB_*` unset; C21 no
RPATH/RUNPATH on `soos-daemon`; C22 `--in-container` guard (image marker `SOOS_SORT_CONTAINER=1`,
`/.dockerenv`, root) as the first statement; C23 per-run volume names; C24 image hygiene; C25
rolling-release risk and triage documented, job `timeout-minutes: 45` (within `pr_loop.sh`'s
watch); C26 accurate trust wording (the default archive is pyke-built ONNX Runtime 1.28.0 from
`cdn.pyke.io`, SHA-256 pinned in `ort-sys` `dist.tsv`).

## 6. Implementation

- `Cargo.toml` (workspace `ort` entry, `default-features = false` edges, `rust-version`), every
  member `Cargo.toml` (`rust-version.workspace = true`), the five ORT crates' `[features]`,
  `Cargo.lock` (one line).
- `crates/inference-ort/src/{runtime.rs (new), error.rs, lib.rs}`.
- `crates/daemon/src/pipeline.rs`, `crates/enrollment-cli/src/service.rs`, `crates/gui/src/main.rs`.
- `.github/workflows/ci.yml` (workflow env, job `system-onnxruntime`, CI Success), `save.sh`.
- `tests/docker/Dockerfile.system-ort`, `tests/docker/system_onnxruntime_test.sh` (new).
- Docs: `Docs/INFERENCE_ORT_CRATE.md`, `Docs/PACKAGING_AND_PROVISIONING.md` §7.5,
  `Docs/CI_CD_AND_SECURITY.md`, `Docs/DAEMON.md` §4, `.claude/skills/dev-workflow/references/project-facts.md`.
- Traceability: ADR, matrix section `system-onnxruntime`, this walkthrough.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) passed. Layer 2 (fresh reviewer, `AI/candid_review_report.md`)
returned APPROVED with two MINOR CI findings and two suggestions; the orchestrator asked for both
MINOR findings to be fixed before the PR (a new layer-2 review is needed for the new fingerprint):

- **MINOR 1** — `timeout-minutes: 45` left little headroom: raised to 75.
- **MINOR 2** — this would have been the first required job depending on live Arch `[extra]` state.
  The job ("System ONNX Runtime (Arch, offline, advisory on PRs)") is now advisory on pull requests:
  removed from `ci-success.needs` and added to `ADVISORY_CHECKS` of `scripts/pr_checks_gate.sh`; it
  still runs on every pull request for signal. On push to `main` (and manual dispatch) the new
  aggregate `ci-success-main` ("CI Success (main, system ONNX Runtime)", `needs: [ci-success,
  system-onnxruntime]`, skipped on pull requests) fails unless both succeed. The image build is
  retried at most `SOOS_SORT_BUILD_ATTEMPTS` times (default 2, bound 3). ADR (6) amended,
  `Docs/CI_CD_AND_SECURITY.md`, `Docs/MACOS_PORT.md`, project facts and matrix SORT8/SORT-S5/MAC5
  updated.
- **SUGGESTION** (`runtime.rs`, log a fixed message instead of the caught panic text) not applied:
  SORT4 (an approved contract) requires the bounded payload in the error detail; the payload is
  `ort`'s own fixed handshake text, never user data, and the daemon panic hook already withholds
  payloads in release builds (#259).

Contract changes for MINOR 2 (tester-first, Red observed before the change: both tests failed):

| Test | Old assertion | New assertion | Justification |
|---|---|---|---|
| `tests/invariants/src/system_onnxruntime_contract.rs::test_sort_s5_ci_job_runs_offline_arch_harness` (own, this PR) | job in `ci-success.needs`, any timeout | job named "…advisory on PRs", `timeout-minutes: 75`, no `continue-on-error`, **not** in `ci-success.needs`, listed in `ADVISORY_CHECKS`, `ci-success-main` push/dispatch-only aggregate needing `[ci-success, system-onnxruntime]` and failing on any non-success, unique job names | candid review MINOR 1/2 |
| `tests/invariants/src/portable_crates_contract.rs::test_mac_pr_loop_treats_only_the_macos_job_as_advisory` (pre-existing, #367) | `ADVISORY_CHECKS` is exactly the macOS job | `ADVISORY_CHECKS` is exactly the macOS job **and** the system ONNX Runtime job (still an exact line; every other MAC5 assertion unchanged) | candid review MINOR 2 (orchestrator-mandated); the allowlist stays exact, so no other check can become advisory silently |

## 8. Verification Results

All with the private target dir, `CARGO_BUILD_JOBS=4`, `LIBONNXRUNTIME_NO_PKG_CONFIG=1` (as in CI):

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --workspace --all-targets --all-features -- --test-threads=4` | pass: 3572 passed, 0 failed, 6 ignored (400 suites); SORT7 ran against `/var/lib/soos/models` |
| `cargo deny --locked check` (0.20.2) | advisories ok, bans ok, licenses ok, sources ok |
| `./scripts/candid_review.sh` (layer 1) | PASSED |
| `cargo test --locked -p soos-invariants --all-features` (after the last harness and docs edits) | pass: 619 passed, 0 failed |
| `SOOS_SORT_MODELS=host SOOS_SORT_JOBS=4 bash tests/docker/system_onnxruntime_test.sh` | pass: image with `onnxruntime-cpu 1.29.0-3` and Rust 1.92.0; offline (`lo` only); feature graph `api-17, pkg-config, std`; MSRV `cargo check --all-targets --all-features`; offline release build; `soos-daemon` → `/usr/lib/libonnxruntime.so.1` (`onnxruntime-cpu`, no RPATH), `libpam_soos.so` without ONNX Runtime; 41 test suites, 234 tests passed (real models required on the shipped-model suites); no `ort.pyke.io` cache; without the library the daemon exits 127 |
| `shellcheck --severity=style tests/docker/system_onnxruntime_test.sh` (koalaman/shellcheck container) | clean |

The models were the host's attested `/var/lib/soos/models` (mounted read-only); CI downloads them
with `scripts/download_models.sh` in the fetch phase.

## 9. Known Limitations / Follow-ups

- The packaging recipes and `scripts/build_*.sh` still build the default `download-binaries`
  variant; switching them (with `Requires: onnxruntime >= 1.17` style dependencies), signed
  releases, the vendor tarball and the channels are later PRs of #361.
- The `system-onnxruntime` job follows rolling `[extra]`: it is advisory on pull requests and
  required on `main` (`ci-success-main`); an Arch change turns `main` red until fixed forward
  (triage in `Docs/CI_CD_AND_SECURITY.md`). `scripts/pr_loop.sh` may wait up to its 45-minute watch
  for this advisory job before deciding on the non-advisory checks.
- Only Arch is exercised offline; Fedora (1.20/1.22) and Debian (1.19) ship pkg-config files of
  their own and are covered by the API 17 floor, but are first built in their PRs of #361.
- `ort-sys` gates only on the minor version of the `.pc` file: a future ONNX Runtime 2.x would be
  rejected at build time (fail closed).
- Owner hand checks: none required for this PR (no installed file, unit or PAM change).
