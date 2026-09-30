# Walkthrough 130 — Dependency Skip Reasons, Sandbox Toolchain Sync and `save.sh` Commit Hygiene

- **Date**: 2026-09-30
- **Issues**: GitHub #243 (TCI-12), #244 (TCI-13), #245 (TCI-15)
- **Branch**: `chore/p2-deps-docker-save`
- **Matrix criteria**: DDS1–DDS8 (new)
- **ADR**: 2026-09-30 "Dependency Pins, Sandbox Toolchain Sync and `save.sh` Commit Hygiene"
- **Scope**: `Cargo.toml`, `Cargo.lock`, `deny.toml`, `tests/docker/test_suite.sh`,
  `.github/workflows/ci.yml`, `save.sh`, new `scripts/commit_message.sh`, new invariant module
  `tests/invariants/src/dependency_tooling_contract.rs`, docs. No production Rust code change
  beyond the `getrandom` major version (same `getrandom::fill` API). No existing test changed.

---

## 1. Findings

| Finding | What was wrong |
|---|---|
| TCI-12 (#243) | `deny.toml` said `getrandom@0.3.4` was a "transitive dependency of rand_core 0.9 via proptest" although six workspace crates used it directly; many other skip reasons named crates that do not depend on the skipped version (e.g. `security-framework`, `flate2`, `native-tls`, `smithay-clipboard`); `getrandom@0.4.3` (third version) was undocumented; `ort` used a caret requirement on a release candidate. |
| TCI-13 (#244) | The Docker sandbox built its toolchain as `stable` at image build time; the CI layer cache can keep that compiler while the other jobs install the current stable from `rust-toolchain.toml`. The stale-`.so` half was already fixed on `origin/main` (unconditional `cargo build --locked --release -p soos-pam`). |
| TCI-15 (#245) | `save.sh` produced `feat(<crate>): update component implementation` for any change (fixes included), pseudo-subject body bullets and a `[N file(s) modified]` footer, and staged everything with `git add .`. |

## 2. Specification

- **Skip reasons** are derived from `cargo tree --locked -i <crate>@<version> --target all
  -e normal,build,dev --depth 1`. The contract is checked from `Cargo.lock` (no network, no
  cargo invocation): a reason must name at least one direct dependent of the skipped version, and
  if a workspace member depends on it directly the reason must say "direct", not "transitive".
- **Three-version crates**: every locked version of a crate locked three or more times is named
  in `deny.toml` (skip entry or comment).
- **Pre-release pins**: any `[workspace.dependencies]` requirement containing `-rc`, `-alpha`,
  `-beta` or `-pre` starts with `=`.
- **Sandbox**: `test_suite.sh` runs `rustup toolchain install --profile minimal` (reads
  `/workspace/rust-toolchain.toml`, updates the channel) before its first cargo call, logs
  `rustc --version`, and exits 1 on a failed sync when `SOOS_REQUIRE_TOOLCHAIN_SYNC=1` (set by
  the CI `pam-integration` job). Offline local runs warn and continue.
- **`save.sh`**: staging is `git add -u` plus `git add ./crates ./tests ./Docs ./AI ./scripts
  ./packaging`; remaining untracked files are listed as not staged. Without `-m`,
  `scripts/commit_message.sh` (`soos_infer_commit_subject BRANCH < staged-files`) prints one
  subject: type = branch prefix (`feat`, `fix`, `test`, `chore` only), scope = the crate
  directory when all staged crate files belong to one crate, description = branch suffix
  lowercased with every character outside `[a-z0-9]` collapsed to one space. It refuses other
  prefixes, empty descriptions and subjects above 72 characters with a message asking for `-m`;
  `save.sh` performs that check before running the pipeline.

## 3. Red Evidence

`cargo test --locked -p soos-invariants --all-features dependency_tooling_contract` on the
original `Cargo.toml`/`Cargo.lock`/`deny.toml`/`save.sh`/`test_suite.sh`: **11 failed, 3 passed**.

- `test_deny_skip_reasons_name_a_real_direct_dependent`: 13 stale reasons, among them
  `getrandom@0.2.17` (reason names `crypto-common`, dependent is `rand_core`), `rustix@0.38.44`
  (names `bindgen`, dependents `which`, `calloop`, `winit`, ...), `core-foundation@0.9.4`
  (names `security-framework`), `miniz_oxide@0.8.9` (names `flate2`, dependent `png`),
  `thiserror@1.0.69` (names `image/glutin`, dependents `calloop`, `ndk`, `smithay-client-toolkit`).
- `test_deny_skip_reasons_never_call_a_direct_workspace_dependency_transitive`:
  `getrandom@0.3.4` is a direct dependency of soos-admin-cli, soos-biometric-store,
  soos-enrollment-cli, soos-evidence-store, soos-gui, soos-pam.
- `test_deny_documents_every_version_of_crates_locked_three_times`: `getrandom@0.4.3` never named.
- `test_prerelease_workspace_dependencies_are_exact_pinned`: `ort` uses `"2.0.0-rc.13"`.
- `test_docker_suite_syncs_toolchain_with_rust_toolchain_toml`,
  `test_ci_pam_integration_requires_toolchain_sync`: no `rustup toolchain install`, no
  `SOOS_REQUIRE_TOOLCHAIN_SYNC`.
- `test_save_sh_never_stages_every_untracked_file`: `save.sh` line `git add .`.
- `test_save_sh_never_generates_generic_subjects`: `update component implementation`.
- The three `test_commit_helper_*` tests: `scripts/commit_message.sh` did not exist.

Already green (regression guards): `test_parse_lockfile_resolves_direct_dependents` (parser
self-test), `test_deny_skip_entries_are_locked_versions`, `test_docker_suite_always_rebuilds_the_pam_module`.

## 4. Implementation

1. `Cargo.toml`: `getrandom = "0.4"` (0.4.3 was already locked through `tempfile`, so
   `Cargo.lock` only re-points six workspace crates; no new package) and
   `ort = "=2.0.0-rc.13"` with a comment. `cargo deny --offline --locked check bans licenses
   sources`: `bans ok, licenses ok, sources ok`.
2. `deny.toml`: every skip reason rewritten from `cargo tree -i`; a header comment documents all
   versions of `getrandom`, `redox_syscall` and `windows-sys` and how to refresh the reasons.
3. `tests/docker/test_suite.sh` section 1c and `.github/workflows/ci.yml`
   (`-e SOOS_REQUIRE_TOOLCHAIN_SYNC=1`).
4. `scripts/commit_message.sh` and `save.sh` (early refusal, staging, one-line subject).

## 5. Test Integrity

No pre-existing test was modified. `sync_issue_contract::test_save_sh_runs_sync_check_before_staging`
still locates the first line starting with `git add .` (now `git add ./crates ...`), which still
follows the `sync_issue.py --check` step, so the ordering contract it guards (SIT5) holds.

## 6. Not Done / Follow-ups

- No Docker run was made for this change: the first CI `pam-integration` run exercises the
  toolchain sync (DDS6 marks the Docker evidence as pending).
- Pinning `rust-toolchain.toml` to a numbered release (instead of `stable`) would make every job
  reproducible but freezes clippy/test too; left to a user decision.
- Dependabot for cargo stays disabled (existing decision); no automation proposes cargo
  updates for advisories, which remain handled by hand through the daily `cargo deny` job.
- Replace `=2.0.0-rc.13` with `2.0` once `ort` 2.0.0 GA is released and validated against the
  real models.
