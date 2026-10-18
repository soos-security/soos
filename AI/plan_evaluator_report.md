# Plan Evaluation Report
- **Date**: 2026-10-18
- **Issue**: GitHub #444 (part of epic #361, no backlog id) — reproducible source and vendor tarballs with a release dry run that never publishes
- **Branch**: `chore/release-dry-run`
- **Base commit**: `3f76090`
- **Plan evaluated**: `AI/architect_spec_release_dry_run.md` §0–§4
- **Mode**: self-check by the implementing agent (tooling, CI and documentation change; no runtime crate). The independent candid review still runs separately.

## 1. Coverage Matrix

| #444 acceptance line | Spec element | Status |
|---|---|---|
| `scripts/release_dry_run.sh`, local only, bounded options, `--dry-run` plan | §2.1, RDR-S3, RDR-S4 | Covered |
| `soos-<workspace version>.tar.gz` from HEAD with `git archive`, mtime = commit time | §2.1 step 3, RDR-S3, RDR-S5 | Covered |
| `soos-<ver>-vendor.tar.zst` from `cargo vendor --locked` + `.cargo/config.toml` snippet, sorted, owner 0, fixed mtime, fixed zstd level | §2.1 steps 5 and 7, RDR-S5 | Covered |
| `SHA256SUMS` over both files in `target/release-dry-run/` | §2.1 step 8 | Covered |
| No gpg, no gh release, no network after the fetch step | §2.1 "Never", RDR-S1, RDR-S5 | Covered |
| Invariant: no publish/sign/upload command | RDR-S1 | Covered |
| Invariant: vendored set equals Cargo.lock by `.cargo-checksum.json` package checksum | §2.2, RDR-S2 | Covered (behavioural) |
| Invariant: version from `[workspace.package]` | RDR-S3 | Covered (behavioural + static) |
| Two runs byte-identical; offline build from the tarballs only in the clean Arch system-ORT image, `--network none`, `ORT_SKIP_DOWNLOAD=1`, no `.git`, no registry | §2.3, RDR-S6 | Covered |
| Advisory on PRs like `system-onnxruntime`, no artifact upload | §1.1, §2.4, RDR-S7 | Covered with a recorded deviation (steps in the existing job, see §3) |
| ADR, matrix rows, backlog entry, `Docs/RELEASING.md` owner-only checklist, packaging pointer, walkthrough | §4, RDR-S8 | Covered |
| Optional SBOM | — | Not done (optional; no new tool) |

## 2. Facts Verified

| Fact | Check | Actual | Match |
|---|---|---|---|
| Workspace version | `Cargo.toml` `[workspace.package]` | `0.1.0` | Yes |
| Registry-only lock | `grep -c 'checksum = ' Cargo.lock`, `grep -c 'source = "git' Cargo.lock` | 489, 0 | Yes |
| Vendored set size | local `cargo vendor --locked --offline` | 489 directories, 538 MiB | Yes |
| Advisory list pinned | `portable_crates_contract` MAC5 `assert_eq!` on the `ADVISORY_CHECKS` line | exactly two names | Yes |
| Job assertions are `contains` | `test_sort_s5_ci_job_runs_offline_arch_harness` | needles only | Yes |
| Job duration | `gh run view 39137537240` | 8 min 27 s of 75 | Yes |
| zstd determinism | level 12, `-T1` vs `-T3` | byte-identical | Yes |

## 3. Findings

| # | Severity | Finding | Resolution |
|---|---|---|---|
| X-1 | Major | A separate job in `ADVISORY_CHECKS` breaks MAC5 (immutable test); a separate job outside it blocks `pr_loop.sh` | Steps in the existing advisory `system-onnxruntime` job (§1.1); reported to the owner in the PR body |
| X-2 | Minor | zstd level 19 takes > 4 min per run (two runs per CI job) | Level 12 pinned (13 s, 48 MB) |
| X-3 | Minor | `TAR_OPTIONS`, `GZIP`, `ZSTD_CLEVEL`, `ZSTD_NBTHREADS` and the umask change the bytes | Unset / fixed in the script, pinned by RDR-S5 |
| X-4 | Minor | Reading the version from the work tree could mismatch the archived HEAD | Version read from `git show HEAD:Cargo.toml` |
| X-5 | Minor | Writing the cargo snippet into `.cargo/config.toml` while `cargo vendor` runs | Snippet captured to a staging file first, then moved |

VALIDATION_VERDICT: APPROVED
