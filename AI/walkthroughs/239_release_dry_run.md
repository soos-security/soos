# Walkthrough 239 — Reproducible Source and Vendor Tarballs With a Release Dry Run That Never Publishes

- **Date**: 2026-10-18
- **Issue**: GitHub #444, part of epic #361 (GitHub-only, no backlog id in `scripts/sync_issue.py`) — **Branch**: `chore/release-dry-run`
- **Decision**: ADR 2026-10-18 "Release Dry Run Publishes Nothing" (amends item (6) "CI" of ADR 2026-10-13 "Distribution Builds: System ONNX Runtime")
- **Matrix criteria**: RDR1–RDR6, RDR-S1–RDR-S8 (new); owner rows RDR-O1–RDR-O4 ⬜ Pending

## 1. Context & Objectives

Epic #361 item 2 asks for signed tags, source and vendor tarballs and a release workflow. Its first step (#375) made the distribution build work offline against the system ONNX Runtime, but nothing built the files a distribution would download: a source tarball, a vendor tarball and their checksums. Signing, tagging and uploading need the owner's OpenPGP key and decisions, and the repository is public, so this step had to build and check everything **without publishing anything**.

Objectives:

- one local script that builds the source tarball, the vendor tarball and `SHA256SUMS` from `HEAD`, reproducibly;
- a fail-closed check that the vendored crates are exactly the ones of `Cargo.lock`;
- proof that the two tarballs alone build soos offline in a clean distribution container;
- invariants that keep every publishing step out of the script, the harness and CI;
- an owner-only checklist for the steps that do publish.

## 2. Architect Design

### `scripts/release_dry_run.sh`

- Inputs come from the committed `HEAD` only: the version is the `version` key of `[workspace.package]` in `git show HEAD:Cargo.toml` (decoy `version` keys in other tables are ignored, and an uncommitted edit only prints a warning); `SOURCE_DATE_EPOCH` is `git log -1 --format=%ct HEAD`.
- Steps: `git archive` → unpack into a staging tree → `cargo fetch --locked` (the only network access; skipped with `--offline`) → `cargo vendor --locked --offline` in the archived tree → vendor check → deterministic vendor tar → `SHA256SUMS` → move the three files into the output directory. The staging directory lives inside the output directory and is removed on every exit (EXIT trap); a failed run leaves no partial set.
- Determinism: `LC_ALL=C`, `TZ=UTC`, `umask 022`; `TAR_OPTIONS`, `GZIP`, `ZSTD_CLEVEL`, `ZSTD_NBTHREADS` unset; `git -c tar.umask=0022 archive`; `gzip -n -9` (no name, MTIME 0); GNU tar `--sort=name --format=gnu --owner=0 --group=0 --numeric-owner --mtime=@SOURCE_DATE_EPOCH --mode=u+rwX,go+rX,go-w --hard-dereference`; `zstd -12 -T1`.
- The `.cargo/config.toml` of the vendor tarball is written by the script from a fixed text, and the `cargo vendor` output must equal it. A future git or second-registry dependency therefore fails the run instead of silently producing a different configuration.
- Options are bounded: each at most once, `--out-dir` never empty, `/` or an option, `--check-vendor` takes exactly two existing paths and nothing else; misuse exits 2.

### Vendor check

`check_vendor` (also `--check-vendor VENDOR_DIR CARGO_LOCK`) compares sorted lists: the `"package"` checksum of every `vendor/*/.cargo-checksum.json` against every `checksum = ` of `Cargo.lock`. Directory names are not trusted. It fails closed on any other source than crates.io, a crates.io package without a 64-hex checksum, a malformed checksum, a symlink or plain file in `vendor/`, a missing or ambiguous `.cargo-checksum.json`, or any missing, extra, duplicated or altered crate. On the real repository: 489 of 489.

### CI placement

The computed task asked for a separate advisory job listed in `ADVISORY_CHECKS`. That list is pinned to exactly two names by the existing contract MAC5 (`portable_crates_contract::test_mac_pr_loop_treats_only_the_macos_job_as_advisory`), which may not be changed, and a job outside the list would be required on pull requests. The harness therefore runs as two extra steps of the existing `system-onnxruntime` job (host toolchain, then the harness with its own `timeout-minutes: 40`): same check name, advisory on pull requests, required on `main` through `ci-success-main`, same Arch image. The job uploads no artifact, saves no cache and creates no tag or release.

## 3. Tester Contract (TDD Red)

New module `tests/invariants/src/release_dry_run_contract.rs`, written before the script. On `3f76090`, 11 of its 12 tests failed because the script, the harness, the CI steps and the documents did not exist (only the scanner self-test passed).

| Test | Proves |
|---|---|
| `test_rdr_s1_forbidden_command_scanner_self_test` | The scanner finds `gh`, `git tag`/`push` (also after `-c`/`-C`), `gpg`, `cosign`, `cargo publish`, `curl`, `wget`, `--sign`, `upload-artifact`, `docker push`, and ignores comments and prose |
| `test_rdr_s1_script_and_harness_never_publish_sign_or_upload` | None of them in the script, the harness or the `system-onnxruntime` job |
| `test_rdr_s2_check_vendor_accepts_matching_set` | A matching fixture passes, with renamed directories too |
| `test_rdr_s2_check_vendor_rejects_mismatches` | 14 mismatch cases fail with exit 1 |
| `test_rdr_s3_dry_run_plan_uses_workspace_package_version_and_commit_time` | The plan of the real repository names the version (parsed independently by the test) and the commit time, and creates nothing |
| `test_rdr_s3_version_comes_from_head_workspace_package` | A temporary repository with decoy `version` keys and an uncommitted edit still gives `1.2.3`; no `[workspace.package]` version fails |
| `test_rdr_s4_usage_errors_exit_2` | 14 misuse cases exit 2; `--help` lists every option |
| `test_rdr1_two_runs_are_byte_identical_and_normalized` | End to end on a dependency-free temporary repository: two runs byte-identical, exactly three outputs, `sha256sum -c` passes, gzip header without name or time, every entry `0/0` at the commit time without group/other write, no untracked file, exact vendor tar entries and `.cargo/config.toml` text |
| `test_rdr_s5_script_pins_determinism_and_step_order` | Flags, environment hygiene and the step order in `main()` |
| `test_rdr_s6_harness_builds_offline_from_tarballs_only` | Two runs and `cmp`, `--network none`, only the run-1 directory mounted, `ORT_SKIP_DOWNLOAD=1`, empty `CARGO_HOME`, the exact build command, `.git` and registry checks |
| `test_rdr_s7_ci_runs_dry_run_in_advisory_system_job_without_upload` | Step order and timeout in the job; no `upload-artifact`, `gh release` or release action in `ci.yml`; read-only permissions |
| `test_rdr_s8_adr_and_docs` | ADR, `Docs/RELEASING.md`, packaging and CI pointers, docs index |

No existing test was modified; `tests/invariants/src/lib.rs` only gains the `mod` line.

## 4. Auditor Constraints

`AI/auditor_constraints_release_dry_run.md` C1–C14: nothing published (C1), no network after the fetch (C2), fail closed with no partial outputs (C3), bounded options (C4), no deletion outside the staging directory (C5), HEAD only (C6), checksum-based vendor check (C7), byte determinism (C8), quoting (C9), in-container guard and read-only mounts (C10), no secret or personal path (C11), CI placement without new permission or upload (C12), no test weakening (C13), bounded disk and jobs (C14).

## 5. Developer Implementation

- `scripts/release_dry_run.sh` (new, executable).
- `tests/docker/release_dry_run_test.sh` (new, executable): host phases 1–3 and `--in-container`, which refuses to run outside the image as root, asserts only `lo`, no `/usr/local/cargo/registry` or `git`, a fresh `CARGO_HOME`, no mounted repository and the MSRV `rustc`; verifies `SHA256SUMS`; unpacks; rejects any `.git`; checks the vendored-sources configuration; builds; checks `readelf` of `soos-daemon` and `libpam_soos.so`; and rejects any ort-sys download cache or downloaded `.crate`.
- `.github/workflows/ci.yml`: header and job comments, two steps in `system-onnxruntime`.
- Documentation: ADR, `Docs/RELEASING.md` (new, indexed in `Docs/README.md`), `Docs/PACKAGING_AND_PROVISIONING.md` §7.5, `Docs/CI_CD_AND_SECURITY.md`, backlog section, matrix component.

## 6. Verification

- `cargo test -p soos-invariants --lib release_dry_run_contract` (scoped, `--test-threads=3`): 12 passed.
- Real repository, on the base commit `3f76090`: run 1 (with the fetch) took 80 s and run 2 (`--offline`) 32 s. The three files were byte-identical (`cmp`): the source tarball was 5.8 MB, the vendor tarball 48 MB with 489 crates, and `soos-0.1.0.tar.gz` had SHA-256 `f09176fb…`.
- `tests/docker/release_dry_run_test.sh` locally on the branch commit (`SOOS_RDR_JOBS=3`), exit 0. Phase 1: two runs, 489 vendored crates matched `Cargo.lock`, and `SHA256SUMS` and both tarballs were byte-identical. Phase 3 (`--network none`, only the run-1 directory mounted): `SHA256SUMS` verified, no `.git`, no registry, the release workspace built from the tarballs in 4 min 17 s, `soos-daemon` linked `libonnxruntime.so.1` and `libpam_soos.so` did not, and no download happened. The work directory was removed at exit.
- An earlier local run passed every functional check but exited 1. Two causes: the harness file was edited while it was running, and the EXIT trap referenced a variable local to `host()` (`work: unbound variable`). The trap now uses the global `WORK_DIR`; the final run above used the fixed harness.
- Formatting, scoped clippy and the scoped test runs are listed in the PR body; the full gate runs through `./save.sh` and CI.

## 7. Owner Follow-ups (not done here)

`Docs/RELEASING.md` §2: OpenPGP release key, version choice (#361 Q6), signed tag, signed `SHA256SUMS`, GitHub Release upload, packaging source URLs, channels (AUR, COPR, PPA, OBS), soos-models package (Q3), package split (Q5), app id / AppStream (Q2), maintainer email. The optional SBOM (from `cargo metadata --locked`) was not added.
