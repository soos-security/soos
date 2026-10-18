# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/release-dry-run`
- **Base (merge-base)**: `54d69f0` (origin/main after #445, #440, #434, #435 and #442)
- **Reviewed-Diff-Fingerprint**: `0d5740dced03567e87512506dee5b5916e2ecf0745959163e32085cd14fdd62f`
- **Audited Files**: `.github/workflows/ci.yml`, `AI/BACKLOG.md`, `AI/DECISIONS.md`,
  `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_release_dry_run.md`,
  `AI/auditor_constraints_release_dry_run.md`, `AI/walkthroughs/239_release_dry_run.md`,
  `Docs/CI_CD_AND_SECURITY.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/README.md`,
  `Docs/RELEASING.md`, `scripts/release_dry_run.sh`, `tests/docker/release_dry_run_test.sh`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/release_dry_run_contract.rs`

## 1. Executive Summary

Independent re-review after the rebase of `chore/release-dry-run` (GitHub #444, part of #361) from
`3f76090` onto `54d69f0`. The frozen patch (`target/candid_diff.patch`, 2295 lines) was compared,
file by file, with the pre-rebase branch (`origin/chore/release-dry-run`, commit `325dbac` against
`3f76090`): the added and removed lines of 12 of the 15 files are identical, and
`AI/plan_evaluator_report.md` (excluded from the fingerprint) is byte-identical. The only
differences are in the three append-conflict files:

- `AI/BACKLOG.md`: the same 28 added lines; one blank separator line moved from before the entry
  to after it. The `### GitHub #444` entry sits at line 2043 inside
  `## Full Project Review Remediation (2026-09-29)` (line 1979), directly after the #415/#416
  model-deploy entry and before main's new `## Active-IR Anti-Spoofing (epic #353)` section
  (line 2072). It is not filed under the IR epic.
- `AI/VERIFICATION_MATRIX.md`: the same component section, preceded by a blank line and a `---`
  separator after main's last table (LKI12), matching the separator style used between the
  other components.
- `AI/DECISIONS.md`: the same single appended ADR line, now after main's #439 ADR.

No semantic conflict with main's changes was found and the soos-invariants suite passes
(822 tests). No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing from the frozen patch:

- Test files touched: `tests/docker/release_dry_run_test.sh` (new),
  `tests/invariants/src/release_dry_run_contract.rs` (new), `tests/invariants/src/lib.rs`
  (one added `#[cfg(all(test, unix))] mod release_dry_run_contract;` with its doc comment).
- Removed or changed assertions (`^-` lines with `assert`, `#[test]`, `proptest!`,
  `#[should_panic]`): none.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

All test changes are additions. No pre-existing assertion was modified by the rebase.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Rebase fidelity: per-file comparison of `+`/`-` lines between the old and new branch diffs
  → identical for `ci.yml`, both scripts, both invariant files, all `Docs/` files, the spec, the
  auditor constraints and walkthrough 239 → PASS.
- Interaction with main's #434 (lock-screen adapters): `PKGBUILD`, `soos.spec` and `install.sh`
  now install `soos-lockstatus`, `soos-lock-widget`, `packaging/gnome-shell/soos-lockscreen@soos/*`
  and `packaging/lockscreen/hyprlock-soos.conf`. The two crates were already workspace members on
  the old base and have no cargo features, so the harness's
  `cargo build --release --workspace --no-default-features --features system-onnxruntime` still
  builds both binaries. The packaging files are tracked and there is no `.gitattributes`
  (`export-ignore`), so `git archive HEAD` puts them in the source tarball → PASS.
- Interaction with #435 (`crates/vision/src/ir_active.rs`) and #445 (`crates/daemon/src/check_config.rs`):
  plain workspace sources with no new dependency, `Cargo.toml`/`Cargo.lock` unchanged between
  `3f76090` and `54d69f0`, so the vendor set and the `--check-vendor` comparison are unaffected → PASS.
- Interaction with #440 (`setup.sh` start bounds) and #442 (Android): no CI, build or vendor
  impact; `android/` is part of the git archive as before → PASS.
- `ci.yml`: main changed nothing in `.github/` between the two bases; the branch's hunk (header
  comment, job comment, `rustup toolchain install`, release dry-run step with
  `timeout-minutes: 40`) applies unchanged in the `system-onnxruntime` job. The last CI run of
  PR #447 took 12m34s for that job, inside its 75-minute job timeout → PASS.
- Doc coherence: the `Docs/PACKAGING_AND_PROVISIONING.md` §7.5 edit merged cleanly next to
  main's lock-screen text; `Docs/RELEASING.md` does not enumerate shipped binaries, so #434 does
  not make it stale → PASS.

### PAM Concurrency & Deadlines

No change under `crates/pam` or any runtime crate → N/A, PASS.

### Panic Safety & Fail-Closed

No Rust production code changed. The scripts keep their fail-closed `--check-vendor` behaviour
(content unchanged; RDR-S2 tests pass) → PASS.

### Test Integrity & Anti-Weakening

The section 2 listing is additions only; the new contract module is registered under
`#[cfg(all(test, unix))]` like its neighbours; the whole `soos-invariants` suite, including the
pre-existing CI invariants (SHA-pinned actions, workflow `permissions: contents: read`,
`ci-success` needs, `ADVISORY_CHECKS` in `scripts/pr_checks_gate.sh`), passes on the rebased
tree → PASS.

### Memory, Bounds & Secrets

No secret, token or credential is read or written; no `${{ secrets.* }}` added → PASS.

### Supply Chain & Automation

- Every `uses:` in `ci.yml` is pinned by a 40-hex SHA (a grep for unpinned `uses:` returns nothing).
- Workflow-level `permissions: contents: read` unchanged; no job-level escalation added.
- `ci-success-main` still needs `[ci-success, system-onnxruntime]`; the job stays advisory on pull
  requests through the existing `ADVISORY_CHECKS` entry.
- No `${{ github.event.* }}` interpolation in `run:`; no `upload-artifact`, cache save, `gh`,
  `git tag`/`push`, `gpg` or `curl` command in either script (comments only) → PASS.

### English-Only Policy

All added text is English → PASS.

## 4. Detailed Findings & Action Items

None. Commands run on the rebased tree:

- `bash -n scripts/release_dry_run.sh` and `bash -n tests/docker/release_dry_run_test.sh`: OK.
- `LIBONNXRUNTIME_NO_PKG_CONFIG=1 ... cargo test --locked -p soos-invariants -- --test-threads=3`:
  822 passed, 0 failed, 0 ignored.

The Docker harness itself (`tests/docker/release_dry_run_test.sh`) was not run locally; it runs in
the `system-onnxruntime` CI job.

## 5. Final Verdict

**VERDICT: APPROVED**
