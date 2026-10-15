# Candid Review Report

- **Date**: 2026-10-15 (round 4, after the rebase onto `origin/main` `0be59f6`; round 3 on `94a7cb3` the same day; rounds 1–2 on 2026-10-13)
- **Target Branch**: `feat/system-onnxruntime`
- **Base (merge-base)**: `0be59f6` (previously `94a7cb3`, before that `6f32ef8`)
- **Reviewed-Diff-Fingerprint**: `a286fbe242f54c9105fa9c15364684e14f6545f56b55d2f9f05970fc19695a17`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `.github/workflows/ci.yml`,
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_system_onnxruntime.md`,
  `AI/auditor_constraints_system_onnxruntime.md`, `AI/walkthroughs/201_system_onnxruntime.md`,
  `Docs/MACOS_PORT.md`, `scripts/pr_checks_gate.sh`, `scripts/pr_loop.sh`,
  `tests/invariants/src/portable_crates_contract.rs`,
  `Cargo.lock`, `Cargo.toml`, `Docs/CI_CD_AND_SECURITY.md`, `Docs/DAEMON.md`,
  `Docs/INFERENCE_ORT_CRATE.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, every member `Cargo.toml`
  (18), `crates/daemon/src/pipeline.rs`, `crates/daemon/tests/system_onnxruntime_startup_tests.rs`,
  `crates/enrollment-cli/src/service.rs`, `crates/gui/src/main.rs`,
  `crates/inference-ort/src/{error.rs,lib.rs,runtime.rs}`,
  `crates/inference-ort/tests/system_onnxruntime_tests.rs`, `save.sh`,
  `tests/docker/Dockerfile.system-ort`, `tests/docker/system_onnxruntime_test.sh`,
  `tests/invariants/src/{dependency_tooling_contract.rs,lib.rs,system_onnxruntime_contract.rs}`

## 1. Executive Summary

GitHub #375 (PR 1 of #361) adds an opt-in `system-onnxruntime` feature that links the distribution
`libonnxruntime` through pkg-config at C API 17, keeps `download-binaries` (the SHA-256-verified
pyke archive) as the default, adds a fail-closed CPU execution provider start-up check, declares
MSRV 1.92 and a new offline Arch CI job wired into CI Success.

Independently verified: the default feature graph of every third-party crate is byte-identical to
`origin/main` (`cargo tree --workspace -e normal,build,dev -f '{p}|{f}'` diff shows only the new
soos feature labels); under `--all-features` the only third-party delta is `pkg-config` on
`ort`/`ort-sys`. `LIBONNXRUNTIME_NO_PKG_CONFIG` is honoured by `pkg-config` 0.3.34 for any value
(`env_var_os(..).is_some()` → `EnvNoPkgConfig`), after which `ort-sys` falls through to the
unchanged, hash-verified download path; it cannot skip verification. The start-up check asks the
library through `GetAvailableProviders` (verified in `ort` 2.0.0-rc.13 `ep/mod.rs`), runs under
`catch_unwind` with `panic = "unwind"` in release, and only `Ok(true)` passes; the daemon calls it
before camera/keys/stores/models and before `bind_socket`. `soos-pam` has no ORT in its graph.
`fmt`, workspace `clippy --all-targets --all-features -D warnings` (which now applies
`incompatible_msrv`), the new tests and the full invariants suite (619) pass locally. No CRITICAL
or MAJOR finding. Round 2: both round-1 MINOR CI findings are resolved (§3b); only suggestions
remain.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Test files touched: `crates/daemon/tests/system_onnxruntime_startup_tests.rs` (new),
`crates/inference-ort/tests/system_onnxruntime_tests.rs` (new), `tests/docker/Dockerfile.system-ort`
(new), `tests/docker/system_onnxruntime_test.sh` (new), `tests/fixtures/Cargo.toml` and
`tests/invariants/Cargo.toml` (`rust-version.workspace = true` only), `tests/invariants/src/lib.rs`
(module registration), `tests/invariants/src/system_onnxruntime_contract.rs` (new),
`tests/invariants/src/dependency_tooling_contract.rs` (one amendment).

- Removed/changed assertions (`^-` with assert/test markers): none matched by the grep. The one
  semantic change is in `dependency_tooling_contract.rs::test_prerelease_workspace_dependencies_are_exact_pinned`:
  `section.contains("ort = \"=2.0.0-rc.")` became "the `ort = ` line contains
  `version = \"=2.0.0-rc.`". Justified by spec §11 contract migration (the entry is now an inline
  table with `default-features = false`). Strength is preserved: a caret/range version, a missing
  `=` or a GA version still fails; the exact entry is additionally pinned by
  `test_sort_s1_workspace_ort_entry_and_feature_tables`. Not a weakening.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance/epsilon): none.
- Inline `mod tests` changes: none.
- SORT7 skips without `manifest.toml` unless `SOOS_REQUIRE_REAL_MODELS=1`, same gating as the
  existing real-model suites; the CI harness sets the variable, and the test ran (passed) locally
  against `/var/lib/soos/models`.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: does the default build still download and verify the pinned archive? `download-binaries
  = ["ort/default"]` reproduces `ort`'s defaults exactly (`std, ndarray, tracing,
  download-binaries, tls-native, copy-dylibs, api-27`); tree diff vs `origin/main` identical → PASS.
- Scenario: can a crate silently re-enable the download in the system build? Workspace `ort` is
  `default-features = false`; `soos-inference-ort`, `soos-vision`, `soos-enrollment-cli` workspace
  edges are `default-features = false`; SORT-S3 asserts per-crate and workspace `ort-sys` features
  `api-17, pkg-config, std`; the harness re-asserts no `download-binaries` in the resolved graph →
  PASS.
- Scenario: `--all-features` on a host with an older distro `.pc` (links both libraries). Mitigated
  by workflow-level `LIBONNXRUNTIME_NO_PKG_CONFIG: "1"` (covers clippy, i686, test, portable-macos;
  `fuzz.yml` and `pr-title.yml` do not use `--all-features`) and `save.sh` → PASS.
- Scenario: system build without a `.pc` → `ort-sys` without `download-binaries` sets
  `should_skip = true` and defers to a link error; no download code compiled → PASS.
- ADR 2026-09-12 (ort CPU only, no OpenCV) preserved: no provider registration, no `load-dynamic`,
  probe never registered; model attestation and PAD self-test unchanged → PASS.

### PAM Concurrency & Deadlines
- `crates/pam` only gains `rust-version.workspace = true`; SORT-S6 checks its graph has no `ort`,
  `ort-sys`, `soos-inference-ort` or Tokio under `--all-features` and `--no-default-features`; the
  harness checks `libpam_soos.so` has no `libonnxruntime` NEEDED entry. Daemon refusing to start →
  socket absent → existing `test_ipc_offline_daemon_returns_ignore` path → PASS.

### Panic Safety & Fail-Closed
- `check_cpu_execution_provider`: exhaustive match, `Ok(true)` is the single pass arm; `Ok(false)`,
  `Ok(Err)`, `Err(panic)` all map to `CpuExecutionProviderUnavailable` → PASS.
- Release profile is `panic = "unwind"`, so `catch_unwind` is effective; daemon panic hook only
  logs (no abort) → PASS.
- Daemon: `initialize_pipeline` step 0 `?` → `DaemonError::Inference` → `main.rs` logs and returns
  before `bind_socket` (line 138) → PASS. Enroll: before `MasterKey::load_or_create` (skipped only
  under `--mock`, which never runs ORT). GUI: before `ModelRegistry::new` → PASS.
- No `unwrap/expect/panic!/indexing` in the new production code (`prefix_within` uses
  `get(..end).unwrap_or_default()` and `saturating_sub`) → PASS.

### Test Integrity & Anti-Weakening
- SORT9 asserts `Ok(false)` for an unknown and a wrong-case name, so a constant-`true` probe
  (`ort::ep::CPU::is_available`) fails it → PASS.
- SORT3/SORT4 exercise the 256-byte bound on multi-byte UTF-8 and every payload type; a missing
  `catch_unwind` would abort the test binary → PASS.
- Contract amendment reviewed in §2 → PASS.

### Memory, Bounds & Secrets
- Error detail bounded to 256 bytes before allocation; probe text originates in ORT only (provider
  list / handshake), never frames, embeddings or credentials → PASS (see SUGGESTION 1).

### Supply Chain & Automation
- `Cargo.lock` delta: one `pkg-config` edge on `ort-sys` (already a locked package); no new crate.
- New job: `actions/checkout` pinned by SHA with `persist-credentials: false`; workflow
  `permissions: contents: read`; no secrets; no `${{ github.event.* }}` in `run:`; image base pinned
  by the same digest as `Dockerfile.arch`; Rust installed through the verified `install_rustup.sh`;
  phase 3 runs `--network none` with the repository mounted `:ro`, asserts only `lo`, `--locked
  --offline`, `ORT_SKIP_DOWNLOAD=1`, unset `ORT_LIB_*`/`PKG_CONFIG_*`, no RPATH, `ldd` resolution
  to `/usr/lib/libonnxruntime.so.1` owned by `onnxruntime-cpu`; destructive `pacman -Rdd` guarded
  by the image marker, `/.dockerenv` and EUID 0 → PASS.
- MSRV 1.92: verified `soos-system-ort` image ships `rustc 1.92.0` and `onnxruntime-cpu 1.29.0-3`;
  the harness log shows the MSRV `cargo check --all-targets --all-features` and release build
  passing; `incompatible_msrv` is clean in the normal clippy job (run locally) → PASS.
- Required status of a job on rolling Arch → FINDING (MINOR 2); timeout margin → FINDING (MINOR 1).

### English-Only Policy
- All code, comments, docs, scripts and messages are English; the only non-ASCII content beyond
  typography is the intentional `'é'` UTF-8 test data → PASS.

## 3b. Re-Review of the Fix Delta (round 2)

The two round-1 MINOR findings were addressed; this report covers the whole new diff.

- **Timeout**: `timeout-minutes: 75` (as `package-deploy`) → round-1 MINOR 1 resolved.
- **Advisory on PRs, required on main**: `system-onnxruntime` was removed from `ci-success.needs`,
  renamed "System ONNX Runtime (Arch, offline, advisory on PRs)" and added to `ADVISORY_CHECKS` →
  round-1 MINOR 2 resolved.
  - **Gate stays fail-closed.** I ran the real `scripts/pr_checks_gate.sh` against a fake `gh`.
    The advisory job failing while CI Success passes and `ci-success-main` is `skipping` → 0
    (proceed). CI Success pending → 3. A check named like the old job ("…(Arch, offline)") failing
    → 1, because the match is exact and literal. Only the advisory check present → 1 (no
    non-advisory check). Every non-advisory fail/cancel/pending still blocks, and `pr_loop.sh`
    still requires the `CI Success` check run to be `success` and a clean final gate snapshot.
  - **`pr_loop.sh` watch.** `--fail-fast` may stop early on the advisory job; the loop then
    decides on non-advisory checks only, within the same 45-minute budget. A 75-minute advisory
    job therefore never blocks and never fakes a pass.
- **`ci-success-main`**: runs only on `push`/`workflow_dispatch` with `if: always()`. It needs
  `[ci-success, system-onnxruntime]` and fails on any result other than `success`, including
  skipped or cancelled. It cannot be bypassed by a failing dependency.
  - **Limits.** `main` has no branch protection (stated in `pr_checks_gate.sh`), so it is a
    post-merge red/green signal, not a merge gate. That is sensible for a job that tracks live Arch
    state. It is skipped on pull requests, and the gate counts `skipping` as non-blocking, so it
    cannot stall a PR.
  - **Owner note.** If branch protection is ever enabled, do not list this check as a required PR
    status. It never runs on PRs (a skipped job satisfies a required check vacuously); require
    "CI Success" instead.
- **MAC5 amendment** (`portable_crates_contract.rs::test_mac_pr_loop_treats_only_the_macos_job_as_advisory`):
  - **Assertion.** It is still an exact `assert_eq!` on the single `ADVISORY_CHECKS` line, and
    still exactly one `ADVISORY_CHECKS=(` definition. The allowlist grows by exactly one named job.
  - **Rationale.** It is a direct consequence of the requested advisory status, recorded in
    walkthrough 201 with Red observed and in ADR item (6). The other MAC5 assertions are untouched.
    Not a weakening.
  - **Companion change.** `test_sort_s5_ci_job_runs_offline_arch_harness` (own test of this PR) now
    asserts the stricter new shape: 75 min, no `continue-on-error`, not in `ci-success.needs`,
    present in `ADVISORY_CHECKS`, and the `ci-success-main` wiring.
- **Retry**: the image build loop is bounded by `SOOS_SORT_BUILD_ATTEMPTS`, an integer validated in
  1..3 (default 2), with 30 s/60 s sleeps. The last failure calls `fail`, so a persistent failure
  still fails the job, and no step after the build is retried. `cargo fetch` keeps cargo's built-in
  `net.retry`, and `download_models.sh` keeps `curl --retry 3`.
- **Docs**: ADR item (6), `Docs/CI_CD_AND_SECURITY.md`, `Docs/MACOS_PORT.md`, project facts and
  matrix SORT8/SORT-S5/MAC5 are consistent with the workflow. English only.
- **Re-run**: `bash -n` on both scripts, `cargo fmt --check`, full `soos-invariants` (619 passed).
  No Rust production code changed in the delta.
- **`runtime.rs` suggestion** not applied. Accepted: SORT4 requires the bounded payload, and the
  payload is `ort`'s fixed handshake text.

## 3c. Re-Review After the Rebase onto `origin/main` `94a7cb3` (round 3)

The branch was rebased from `6f32ef8` onto `94a7cb3`. Main gained only #396 (`rustls` 0.23.45 →
0.23.46 for RUSTSEC-2026-0346/-0347, a four-line `Cargo.lock` change) and walkthrough 211. The
diff fingerprint changed because this branch also edits `Cargo.lock`. I re-checked the whole new
patch (`target/candid_diff.patch`, 49 files) and the rebase itself.

- **The rebase changed nothing else.** `git range-diff origin/feat/system-onnxruntime...HEAD` shows
  the new upstream commit and, for the branch commit, hunks in `AI/candid_review_report.md` only
  (context shift, because main's report text changed). A tree diff of
  `origin/feat/system-onnxruntime` against `HEAD` lists exactly `AI/walkthroughs/211_rustls_advisories.md`
  (from main) and the four `rustls` version/checksum lines in `Cargo.lock`. Every other file in the
  branch diff is byte-identical to the round-2 version.
- **The lockfile is consistent.** Against the new merge base, the `Cargo.lock` delta is still the
  single `pkg-config` edge under `ort-sys`. There is exactly one `rustls` entry, at `0.23.46` with
  main's checksum. `ureq` 3.4.2 (pulled by `ort-sys` through `download-binaries` = `ort/default`)
  keeps its unchanged dependency list, including `rustls`, so the default download path uses the
  patched version. The `ort`/`ureq` feature edits on this branch do not add or drop a TLS crate in
  the lockfile (the lockfile is feature-independent, and the round-1 `cargo tree` comparison showed
  an identical default third-party graph). `cargo metadata --locked --format-version 1` succeeds,
  so the lockfile needs no update.
- **The patch has no rebase leftovers.** It has no conflict markers, and no hunk mentions `rustls`
  (main's bump is in the base).
- **Test changes are unchanged.** The step-3 listing returns the same 10 test files and no removed
  assertion, new escape hatch or inline `mod tests` change (§2 still holds).
- **Production code was re-read.** `crates/inference-ort/src/runtime.rs` is unchanged: only
  `Ok(true)` passes, the probe runs under `catch_unwind`, and the detail is bounded before
  allocation.
- No Rust build was run in this round, at the lead's request (shared RAM). The source tree is
  identical to round 2 apart from the upstream lockfile bump, and upstream CI validated that bump.

Previous findings carry forward unchanged: no CRITICAL, MAJOR or MINOR finding remains open, and
the four SUGGESTION items in section 4 still apply.

## 3d. Re-Review After the Rebase onto `origin/main` `0be59f6` (round 4)

The branch was rebased from `94a7cb3` onto `0be59f6`. Main gained #383 (push delivery test
deflake), #394 (actions bump), #384 (motion specification), #388 (read-only lock-screen state
request), #386 (remote offline alerts) and #385 (`scripts/setup.sh` one-command setup core). The
rebase had append-only conflicts in `AI/DECISIONS.md`, `tests/invariants/src/lib.rs` and
`AI/VERIFICATION_MATRIX.md`. I re-checked the whole new patch (`target/candid_diff.patch`, 49
files) and the rebase itself.

- **The rebase changed nothing except placement.** `git range-diff origin/feat/system-onnxruntime...HEAD`
  pairs the old and new branch commits; the review-report commit is identical (`=`), and the
  feature commit differs only by context. For each of the 51 files of the feature commit, I
  compared the sorted added and removed lines of the old commit (`7e7e6af`) and the new one
  (`6b73f12`). 48 files are identical. The three that differ:
  - `AI/VERIFICATION_MATRIX.md`: the only extra lines are a blank line and a `---` separator, so
    the `system-onnxruntime` section follows main's new sections (`rwp-delivery-deflake`,
    `ui-motion-spec`, setup SET rows). The content of the section and the MAC5 amendment are
    unchanged.
  - `AI/plan_evaluator_report.md` and `AI/candid_review_report.md`: whole-file rewrites whose
    removed lines differ because main changed the old text. The resulting file contents are
    byte-identical to the pre-rebase versions. Both files are excluded from the review patch.
- **The conflict resolutions are correct.** `AI/DECISIONS.md`: main's three new ADRs (#388, #386,
  #385) come first, then this branch's "Distribution Builds: System ONNX Runtime" ADR, unchanged.
  `tests/invariants/src/lib.rs`: 78 modules (77 on main plus `system_onnxruntime_contract`,
  registered after `setup_contract` under `#[cfg(all(test, unix))]`). No conflict markers in the
  patch.
- **Interaction with #385 `scripts/setup.sh`.** setup.sh builds with
  `cargo build --release --locked --workspace` as the build user, then runs `scripts/install.sh
  --start`. That command uses default features, so it gets `download-binaries` (= `ort/default`),
  the same SHA-256-verified download as on main. No crate enables `system-onnxruntime` by default,
  so `pkg-config` is never consulted for libonnxruntime there, and `LIBONNXRUNTIME_NO_PKG_CONFIG`
  is not needed. When setup.sh is started as root, it builds under `env -i` with an allow-list,
  so stray `ORT_*`/`PKG_CONFIG_*` variables from root never reach the build. setup.sh, install.sh
  and the packaging builders (`build_deb.sh`, `build_rpm.sh`, `build_arch.sh`,
  `build_packages.sh`, `PKGBUILD`, `debian/rules`) do not use `--all-features`. Moving packages to
  the system library stays a later PR of #361, as the ADR says. The pinned toolchain
  (`rust-toolchain.toml` 1.98.1, installed by `install_rustup.sh`) is above the new MSRV 1.92. →
  PASS, no gap.
- **Interaction with the other main changes.**
  - #394 bumped `taiki-e/install-action` and `upload-artifact`. This branch's new job uses only
    `actions/checkout@3d3c42e…` (v7.0.1), the same SHA as all 16 other checkout pins.
  - #388 and #386 add no crate manifest, so `rust-version.workspace` coverage is unchanged.
  - `crates/daemon/src/pipeline.rs` was not touched on main. `crates/daemon/src/main.rs` changed
    on main (lock-state board), but the start-up check still runs inside
    `initialize_pipeline`, before `bind_socket`.
  - Residual: whether main's new daemon, remote and protocol code compiles under the MSRV 1.92 was
    not built locally (RAM). It is gated before merge: `incompatible_msrv` in the required clippy
    job now applies `rust-version = "1.92"` to all of that code, and the advisory system job builds
    the workspace with 1.92.0.
- **Test changes are unchanged.** The step-3 listing returns the same 10 test files and no removed
  assertion, new escape hatch or inline `mod tests` change (§2 still holds).
- **Tests run in this round** (lead-restricted scope):
  `cargo test --locked -p soos-invariants -p soos-inference-ort -- --test-threads=4`, with
  `CARGO_INCREMENTAL=0` and no debuginfo. Exit 0, 851 passed, 0 failed, 0 ignored. That includes
  668 `soos-invariants` unit tests: every SORT-S contract, the amended MAC5 and dependency-tooling
  contracts, and main's new `setup_contract`, `remote_offline_contract`, lock-state and
  `ui_motion_spec_contract`, run against this branch's `Cargo.toml`, `ci.yml` and `save.sh`.

Previous findings carry forward unchanged. No CRITICAL, MAJOR or MINOR finding remains open, and
the four SUGGESTION items in section 4 still apply.

## 4. Detailed Findings & Action Items

- **[RESOLVED, round-1 MINOR]** `.github/workflows/ci.yml` — job timeout raised to 75 minutes.
- **[RESOLVED, round-1 MINOR]** `.github/workflows/ci.yml` / `scripts/pr_checks_gate.sh` — the job
  is advisory on pull requests and required on `main` through `ci-success-main`; the image build has
  a bounded retry.
- **[SUGGESTION]** `scripts/pr_loop.sh:169` — the comment "45 min ceiling, above the longest job
  timeout of 40 min" is stale (`package-deploy` and the advisory system job have 75). Behaviour is
  correct; only the wording is outdated.
- **[SUGGESTION]** `.github/workflows/ci.yml` (`ci-success-main`) — owner note: it is a post-merge
  signal, never a PR-required check (see §3b).
- **[SUGGESTION]** `crates/inference-ort/src/runtime.rs:81` — the panic payload reaches the daemon
  error log. It is bounded and `ort`-only. Kept as is, as SORT4 requires.
- **[SUGGESTION]** `.agents/skills/dev-workflow/references/project-facts.md` — running
  `--all-features` locally without `save.sh` on a host with a distro ONNX Runtime 1.27 or newer
  links that library; documented.

## 5. Final Verdict

**VERDICT: APPROVED**
