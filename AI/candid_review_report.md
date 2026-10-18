# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/release-dry-run`
- **Base (merge-base)**: `3f76090` (origin/main after #430)
- **Reviewed-Diff-Fingerprint**: `52ff21b8554390ecebdb880f90c05d75efc9c76134a16c1dd5d0f39f1d79f99b`
- **Audited Files**: `.github/workflows/ci.yml`, `AI/BACKLOG.md`, `AI/DECISIONS.md`,
  `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_release_dry_run.md`,
  `AI/auditor_constraints_release_dry_run.md`, `AI/walkthroughs/239_release_dry_run.md`,
  `Docs/CI_CD_AND_SECURITY.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/README.md`,
  `Docs/RELEASING.md`, `scripts/release_dry_run.sh`, `tests/docker/release_dry_run_test.sh`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/release_dry_run_contract.rs`

## 1. Executive Summary

The branch adds a local-only release dry run (`scripts/release_dry_run.sh`) that builds a
`git archive` source tarball, a deterministic `cargo vendor` tarball and `SHA256SUMS` from the
committed HEAD, a Docker harness that checks reproducibility and an offline build from the two
tarballs, two extra steps in the existing advisory `system-onnxruntime` CI job, an ADR, docs and
an invariant contract (12 tests). No Rust production crate is touched. The reviewer independently
re-ran the scoped invariants tests and clippy, ran the script twice on the real repository
(byte-identical outputs, 489 vendored crates matching `Cargo.lock`), unpacked the tarballs and
resolved the workspace offline with an empty `CARGO_HOME`, and exercised the vendor-check failure
and usage-error paths. No CRITICAL or MAJOR finding.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `tests/docker/release_dry_run_test.sh` (new), `tests/invariants/src/lib.rs`
  (one new `mod release_dry_run_contract;` declaration, add-only),
  `tests/invariants/src/release_dry_run_contract.rs` (new).
- `grep '^-[^-].*(assert|#[test]|#[tokio::test|proptest!|#[should_panic)'`: **no output**. No
  pre-existing assertion was removed or changed.
- `grep '^\+.*(#[ignore|#[cfg(any())]|should_panic|tolerance|epsilon)'`: **no output**.
- `git diff 3f76090 HEAD -- tests 'crates/*/tests' android/app/src/test`: only the three files
  above; the `lib.rs` hunk is a pure addition. No inline `mod tests` hunk in any crate.
- The ADR records that a separate CI job was rejected precisely because it would require changing
  the immutable MAC5 assertion (`portable_crates_contract::test_mac_pr_loop_treats_only_the_macos_job_as_advisory`,
  which pins exactly two `ADVISORY_CHECKS`); the reviewer confirmed that assertion and
  `scripts/pr_checks_gate.sh` are unchanged.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Reproducibility: ran `scripts/release_dry_run.sh --offline --out-dir` twice into two scratch
  directories; `SHA256SUMS` and both tarballs `cmp`-identical; the vendor tarball decompresses to
  525,568,000 bytes, matching the ADR's "526 MB". PASS.
- Version source: `[workspace.package] version = "0.1.0"` in `Cargo.toml`; the outputs are named
  `soos-0.1.0*`. The awk parser stops at the first `version` inside `[workspace.package]` only;
  decoys in other tables are covered by `test_rdr_s3_version_comes_from_head_workspace_package`. PASS.
- `SOURCE_DATE_EPOCH` = committer time (`%ct`), the same time `git archive` uses for a commit
  argument, so both tarballs carry one mtime. PASS.
- Source tarball guard: the repository tracks no `.cargo/` or `vendor/` (`git ls-files`), and no
  `.gitattributes` exists, so `export-ignore`/`export-subst` cannot alter the archive. PASS.
- `vendor_crates`: the `grep ... && fail` list does not trip `set -e` when the lock has no
  registry source (zero-dependency fixture path), verified by the RDR1 fixture run. PASS.
- Outputs are staged in `mktemp -d` inside the output directory and moved with `mv -f` on the same
  file system; the `EXIT` trap removes the staging directory; `--out-dir /` and option-like values
  are refused. PASS.
- Docs claims checked against code: PKGBUILD `source=()` is empty (line 29), RPM `Source0` exists,
  `ADVISORY_CHECKS` already lists the `System ONNX Runtime (Arch, offline, advisory on PRs)` job,
  the Dockerfile sets `SOOS_SORT_CONTAINER=1`, `CARGO_HOME=/usr/local/cargo` with no baked registry
  (the system harness mounts its registry as a volume), `WORKDIR /workspace`, root user; the
  harness's in-container guards therefore hold. 489 crates confirmed. PASS.
- CI time budget: the latest `main` run of the `system-onnxruntime` job took about 8.5 min against
  `timeout-minutes: 75`; the new step has its own 40-minute limit. PASS.

### PAM Concurrency & Deadlines
- Not affected: no change under `crates/pam` or any Rust production crate. The harness asserts
  `libpam_soos.so` does not link ONNX Runtime. PASS (N/A).

### Panic Safety & Fail-Closed
- No Rust production code changed. The vendor check fails closed: removing one vendored crate gave
  `vendored crates differ from Cargo.lock (488 vendored, 489 locked ...)`, exit 1; an unknown
  argument exits 2; non-crates.io sources, malformed or missing lock checksums, symlinks and
  ambiguous `.cargo-checksum.json` files fail (RDR-S2 tests). PASS.

### Test Integrity & Anti-Weakening
- No pre-existing assertion touched (section 2). The new tests run the real script against a
  temporary fixture git repository and assert concrete bytes (gzip header flags and MTIME, tar
  owner/mtime/mode, exact output set, `sha256sum -c`), so a non-deterministic or wrongly named
  implementation fails them. `cargo test -p soos-invariants -- --test-threads=3`: 799 passed,
  0 failed (12 in `release_dry_run_contract`); `cargo clippy -p soos-invariants --all-targets
  -- -D warnings`: clean. PASS.

### Memory, Bounds & Secrets
- No secret, key or credential is read or written; nothing signs. The harness bounds
  `SOOS_RDR_JOBS` (1..64) and `SOOS_RDR_BUILD_ATTEMPTS` (1..3) and builds under
  `--network none` with the release directory mounted read-only. PASS.

### Supply Chain & Automation
- `ci.yml`: no new action, no `upload-artifact`, no cache save, no `${{ github.event.* }}` in
  `run:`; the two steps are plain `rustup toolchain install --profile minimal` (reads
  `rust-toolchain.toml`) and the harness. No `Cargo.toml`, `Cargo.lock` or `deny.toml` change. The
  only network step is `cargo fetch --locked`, plus the existing image build. PASS.

### English-Only Policy
- All added files, comments and script output are English; a scan for French accented characters
  in the changed files returned nothing. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `AI/VERIFICATION_MATRIX.md` RDR4/RDR5 — the "local Docker run on 2026-10-18" evidence
  (including "4 min 17 s") could not be reproduced by this reviewer (Docker phase 3 not re-run);
  the reviewer only reproduced phase 1 reproducibility and the offline workspace resolution. The
  rows already state that CI evidence follows on the pull request; confirm with the PR run of the
  `system-onnxruntime` job.
- **[SUGGESTION]** `scripts/release_dry_run.sh:128` — the `"package":"<hex>"` grep would also
  match a vendored file whose crate-relative path is literally `package` in the `files` map; this
  fails closed (exit 1, "exactly one package checksum"), so it can only cause a false failure,
  never a false pass. Parsing the trailing `"package"` key only would remove the ambiguity.
- **[SUGGESTION]** `AI/walkthroughs/239_release_dry_run.md` — the number jumps from 231 to 239
  (presumably reserved by parallel branches); make sure no other branch lands a different 239.

## 5. Final Verdict

**VERDICT: APPROVED**
