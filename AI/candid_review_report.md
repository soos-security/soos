# Candid Review Report

- **Date**: 2026-10-02
- **Target Branch**: `fix/fu1002-followups`
- **Base (merge-base)**: `ac47d3e3`
- **Reviewed-Diff-Fingerprint**: `e6ed5ba568c125b8890e6bda0ba0c75d89dcb218ec153eecad5065cacb30dd59`
- **Audited Files**: `.github/workflows/ci.yml`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`,
  `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/170_arch_faillock_ci_cache_prefix.md`,
  `AI/walkthroughs/171_virtual_camera_optin_and_cli_races.md`, `Docs/CAMERA_V4L_CRATE.md`,
  `Docs/CI_CD_AND_SECURITY.md`, `Docs/DAEMON.md`, `Docs/DEVELOPMENT_WORKFLOW.md`,
  `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/ENROLLMENT_CLI.md`, `Docs/GUI_APPLICATION.md`,
  `Docs/PACKAGING_AND_PROVISIONING.md`, `crates/admin-cli/src/gdm.rs`,
  `crates/admin-cli/src/logs.rs`, `crates/daemon/src/config.rs`,
  `crates/daemon/tests/virtual_camera_optin_tests.rs`, `crates/enrollment-cli/src/lib.rs`,
  `crates/enrollment-cli/src/service.rs`, `crates/enrollment-cli/tests/virtual_camera_optin_tests.rs`,
  `crates/gui/build.rs`, `crates/gui/build_support/bindir.rs`, `crates/gui/src/privileged.rs`,
  `crates/gui/tests/program_path_tests.rs`, `packaging/arch/PKGBUILD`, `packaging/arch/soos.install`,
  `packaging/debian/rules`, `packaging/pam/arch/system-auth`, `packaging/pam/arch/system-auth.snippet`,
  `packaging/rpm/soos.spec`, `scripts/build_{arch,deb,packages,rpm}.sh`, `scripts/install.sh`,
  `scripts/prefetch_onnxruntime.sh`, `tests/distro/{arch_linux,debian_ubuntu,fedora_rhel}_test.sh`,
  `tests/distro/run_distro_validation.sh`, `tests/docker/Dockerfile.systemd`,
  `tests/docker/systemd_unit_acceptance_test.sh`, `tests/invariants/src/arch_faillock_ci_contract.rs`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/packaging_ownership_contract.rs`,
  `tests/invariants/src/systemd_unit_acceptance_contract.rs`

## 1. Executive Summary

The diff resolves GitHub #318: the Arch primary rule becomes `[success=4 default=ignore]` so a
face match runs `pam_permit`, `pam_env` and `pam_faillock authsucc` (stock success path),
`Dockerfile.systemd` is digest-pinned, the ort-sys ONNX Runtime download is cached (main-only
save) and prefetched with a bounded retry, `soos-gui` derives its `soos-enroll` path from a
validated build-time `SOOS_BINDIR`, the daemon and `soos-enroll` honour `allow_virtual_camera`
(default false), the daemon refuses a NUL `camera_device`, `gdm restore` re-checks the PAM file
before the rename, and `logs --file` tails backwards in bounded chunks. No PAM-module code is
touched. No CRITICAL or MAJOR defect found. Every modified pre-existing assertion is listed as
owner-approved in walkthrough 170 §4. Local runs: `soos-admin-cli --lib`, `soos-gui`,
`soos-invariants` (378 + module tests), daemon and enrollment-cli `virtual_camera_optin_tests`
all pass.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Removed/changed assertion lines (`grep '^-.*assert'`): one, patch line 3315,
  `tests/invariants/src/lib.rs::test_pam_config_ordering_matches_spec` (Arch snippet primary
  rule now checked with `[success=4 default=ignore]`). Listed in walkthrough 170 §4 "Migrated
  existing tests (owner-approved 2026-10-02)". Debian/Fedora still require `success=done`.
- Other changed expectations (not caught by the grep because they are data, not `assert!`):
  - `packaging_ownership_contract::test_arch_system_auth_is_stock_pambase_with_exact_soos_edit`:
    `ARCH_PRIMARY_LINE` and the rule-1 control changed to `success=4`. Listed as owner-approved.
    The existing "every success=N lands on pam_permit.so" loop now also covers the soos rule
    (stricter).
  - `packaging_ownership_contract::test_docker_base_images_are_pinned_by_digest`:
    `Dockerfile.systemd` added to the pinned list (stricter). Listed.
  - `systemd_unit_acceptance_contract::test_systemd_acceptance_harness_exists_and_isolates_the_host`:
    `FROM ubuntu:24.04` replaced with `FROM ubuntu:24.04@sha256:<64 hex>` (stricter). Listed.
  - `packaging_ownership_contract`: helpers became `pub(crate)` (visibility only, no assertion).
- New escape hatches (`#[ignore]`, `should_panic`, tolerance): none.
- Inline test modules added: `gdm.rs::restore_race_tests` (5 tests), `logs.rs::tail_tests`
  (2 tests); `read_bounded_line` became `#[cfg(test)]` and is used as the oracle.
- New test files: daemon and enrollment-cli `virtual_camera_optin_tests.rs`,
  `crates/gui/tests/program_path_tests.rs`, `tests/invariants/src/arch_faillock_ci_contract.rs`.

Verdict on test integrity: PASS, no unapproved weakening.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Arch jump count: from line 6 of `packaging/pam/arch/system-auth`, `success=4` skips
  `pam_systemd_home` (7), `pam_unix` (8), the event line (9), `faillock authfail` (10) and lands
  on `pam_permit` (11), then `pam_env` (12) and `faillock authsucc` (13). A `-auth` line whose
  module is missing still occupies a stack slot in Linux-PAM, so the count holds with or
  without `pam_systemd_home.so` (pambase's own `success=3` relies on the same thing). With the
  module missing, the `default=ignore` branch applies. PASS.
- Locked account plus face Allow: `faillock preauth` is `required`, so its failure persists
  past the jump, and `authsucc` re-checks the tally. The stack still fails. The harness
  scenario 6h covers this. PASS.
- `tail_lines`: checked by hand against `""`, `"\n"`, `"a"`, a final line without `\n`, a
  break inside the first chunk (no extra `(0, line_end)` push because `pos` is not
  advanced), and an exact `capacity` stop. The randomized oracle test compares against the
  previous forward reader with lines over `MAX_LOG_LINE_BYTES` and lines that cross chunk
  boundaries. PASS.
- `gdm restore`: one bounded `O_NOFOLLOW` snapshot feeds both the stale-backup check and the
  pre-rename re-check (inode + bytes, or still-absent). `--force` skips only the stale check.
  On a refusal the temporary file is removed and the target is untouched. Tests cover an
  edit, `--force`, a same-bytes inode swap and a file appearing. PASS. One residual window
  between the final re-check and `rename(2)` is inherent and documented.
- Daemon `allow_virtual_camera`: an `Option<bool>` via serde, so a mistyped value is a config
  error (the daemon fails to start, PAM falls back). Enabling it queues a startup warning. Enroll
  reads the same key through the shared reader in the single resolution call (false when
  mistyped, with a note). Both sides fail closed. PASS.
- `SOOS_BINDIR`: the build script validates the value (absolute, normalized,
  `[A-Za-z0-9._+-]`, ≤256 bytes) and fails the build with `cargo::error` otherwise. The cfg is
  set only for a non-default value, so the default constant literal (GCV19) stays.
  `rerun-if-env-changed` keeps `env!` consistent. All package builders pin `/usr/bin`. PASS.

### PAM Concurrency & Deadlines
- No file under `crates/pam` changed. The only PAM-facing change is the Arch stack control
  word. No new blocking path in the module. PASS.

### Panic Safety & Fail-Closed
- New production code (`gdm.rs`, `logs.rs`, `config.rs`, `service.rs`, `build.rs`,
  `bindir.rs`) has no `unwrap`/`expect`/indexing. It uses `get_mut`, `saturating_*` and
  `try_from(..).unwrap_or(..)`.
- No path turns an error into `Allow`/`PAM_SUCCESS`. A face match on Arch still runs the
  `required` faillock modules, and their failure is preserved. PASS.

### Test Integrity & Anti-Weakening
- See §2. The new tests fail against plausible wrong implementations: the old `success=done`
  stack (red evidence recorded), a missing re-check, a hard-coded `false` opt-in (source scan
  in VCO3), and a whole-file read in `logs` (byte-served counter on a 64 MiB virtual log). PASS.

### Memory, Bounds & Secrets
- `read_pam_file_snapshot` is bounded by `MAX_PAM_FILE_BYTES` with `take(+1)`. The `logs`
  tail memory is bounded by `MAX_LOG_TAIL_LINES` × `MAX_LOG_LINE_BYTES` plus one 64 KiB chunk.
  A file that shrinks while it is read is an error, not a hang.
- The NUL `camera_device` error names the key and never the value (asserted). The
  virtual-camera warning holds no secret. PASS.
- MINOR (see §4): an oversized PAM file is now refused even with `--force`. This fails closed.

### Supply Chain & Automation
- `actions/cache/restore` and `actions/cache/save` are pinned by full SHA with a version
  comment. No new `permissions:` and no `${{ github.event.* }}` inside `run:`. Saving is gated
  to `refs/heads/main` on a cache miss, so a PR cannot poison the main-scoped cache. The key is
  `Cargo.lock`-hashed, so an ort bump invalidates it. ort-sys verifies the SHA-256 of each
  archive on download.
- `prefetch_onnxruntime.sh` validates its inputs (1..5 attempts, 0..60 s delay), uses
  `set -euo pipefail` and makes a bounded number of attempts. `cargo check -p ort` uses the
  same default features as every workspace consumer (`ort = "=2.0.0-rc.13"`, no feature
  overrides), so it fetches the same archive.
- `SOOS_ORT_CACHE_DIR` is validated (absolute, existing) before it is mounted.
  `Dockerfile.systemd` digest equals `Dockerfile.ubuntu`. PASS.

### English-Only Policy
- Scanned every added line for non-ASCII and French tokens. Only typographic symbols (—, →,
  ≤, ✅) were found. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/admin-cli/src/gdm.rs:545`: `gdm restore --force` now reads the PAM file
  through `read_pam_file_snapshot` and refuses an oversized (> `MAX_PAM_FILE_BYTES`),
  symlinked or special file. Previously `--force` skipped reading the file. This is
  fail-closed and arguably correct, but it is a behavior change of `--force`. Mention it in
  `Docs/` (the error tells the operator to fix the file by hand).
- **[SUGGESTION]** `crates/admin-cli/src/logs.rs:159`: memory is bounded, but a file with no
  `\n` over a huge region is still scanned end to end (backwards). This matches the previous
  forward reader, so it is not a regression. A byte budget for the backward scan (for example
  `MAX_LOG_TAIL_LINES × MAX_LOG_LINE_BYTES`, plus a fallback line) would make the cost follow
  the tail in every case.
- **[SUGGESTION]** `scripts/install.sh:417`: `SOOS_BUILD_BINDIR="${PREFIX}/bin"` is not
  normalized. A `--prefix /usr/local/` (trailing slash) yields `/usr/local//bin`, which
  `crates/gui/build.rs` rejects, so `--build` fails late with a cargo error. This fails closed.
  Stripping trailing slashes from `PREFIX`, or validating it upfront, would improve the error
  message.
- **[SUGGESTION]** `.github/workflows/ci.yml` (systemd-unit, package-deploy): the host ORT
  cache is populated by root inside the container and saved by the runner user. The archive
  modes (0644/0755) make it readable, but if `actions/cache/save` ever reports permission
  errors on main, add a `chown`/`chmod` step before saving.

## 5. Final Verdict

**VERDICT: APPROVED**
