# Audit Constraints — GitHub #444: release dry run (reproducible source and vendor tarballs)

Inputs: `AI/architect_spec_release_dry_run.md`, the new contract
`tests/invariants/src/release_dry_run_contract.rs` (RDR1 and RDR-S1–RDR-S8; red on `3f76090`: 11
of 12 fail because `scripts/release_dry_run.sh`, the harness, the CI steps and the documents do not
exist; the scanner self-test passes), `tests/docker/system_onnxruntime_test.sh`,
`tests/docker/Dockerfile.system-ort`, `.github/workflows/ci.yml`, `scripts/pr_checks_gate.sh`.

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| C1 | Nothing is published: no `gh`, `git tag`, `git push`, `gpg`, `--sign`, `cosign`, `minisign`, `curl`, `wget`, `cargo publish`, `docker push`, `upload-artifact` in the script, the harness or the CI job | script, harness, `ci.yml` | RDR-S1 |
| C2 | No network after the fetch step: `cargo vendor` runs with `--offline` and `CARGO_NET_OFFLINE=true`; `--offline` skips the fetch; the offline container runs with `--network none` and asserts only `lo` | script, harness | RDR-S5, RDR-S6 |
| C3 | Fail closed: `set -euo pipefail`; every check failure exits non-zero; the three outputs appear in the output directory only after every step passed (staging + move); the staging directory is removed on every exit path (EXIT trap) | script | RDR1, review |
| C4 | Bounded input: every option at most once; `--out-dir` exactly one non-empty value, never `/`, never an option; `--check-vendor` exactly two existing paths and nothing else; unknown arguments exit 2 | script | RDR-S4 |
| C5 | No destructive operation outside the staging directory: never `rm -rf` of the output directory or a user path; outputs replaced by `mv -f` of the three known names only | script | review |
| C6 | Only committed content is packed (`git archive HEAD`); version and epoch from HEAD; a dirty work tree only warns | script | RDR-S3, RDR1 |
| C7 | The vendor check trusts checksums, not directory names; it rejects git or other-registry sources, a registry package without checksum, malformed checksums, symlinks, plain files, missing or ambiguous `.cargo-checksum.json`, and any multiset difference | `--check-vendor` | RDR-S2 |
| C8 | Byte determinism: `LC_ALL=C`, `TZ=UTC`, `umask 022`, `TAR_OPTIONS`/`GZIP`/`ZSTD_CLEVEL`/`ZSTD_NBTHREADS` unset, `tar.umask=0022`, `gzip -n`, GNU tar normalization flags, fixed zstd level and `-T1` | script | RDR-S5, RDR1, harness two-run `cmp` |
| C9 | Quoting: zsh is the login shell of the owner but the scripts are `#!/usr/bin/env bash`; every expansion quoted, arrays for argument lists, no word splitting of unquoted variables | script, harness | `shellcheck --severity=error`, `bash -n` |
| C10 | The in-container mode refuses to run on a host (`SOOS_SORT_CONTAINER=1`, `/.dockerenv`, EUID 0); the offline container mounts only the run-1 output directory and the harness file, both read-only; never the repository, never a cargo registry | harness | RDR-S6 |
| C11 | No secret or personal data: no key material, no maintainer email, no `/home/<user>` path; the only printed values are commit id, version, epoch, file names and public crate checksums | all | `maintainer_hygiene_contract`, `scripts/secret_scan.sh` |
| C12 | CI: the new steps live in the existing `system-onnxruntime` job (advisory on pull requests through the existing `ADVISORY_CHECKS` entry, required on `main` through `ci-success-main`); no `${{ }}` in the job, no `continue-on-error`, no new permission, no cache save, no artifact upload; the existing `timeout-minutes: 75` stays and the new step has its own `timeout-minutes: 40` | `ci.yml` | RDR-S7, SORT-S5, MAC5 |
| C13 | No test assertion of `origin/main` is modified, weakened or deleted; only a new module and its `mod` line are added | `tests/invariants` | `git diff origin/main -- tests/` |
| C14 | Disk and memory: outputs and the harness work directory live under the git-ignored `target/`; the harness removes its work directory on exit unless `SOOS_RDR_KEEP=1`; build jobs bounded (`SOOS_RDR_JOBS` 1..64, image attempts 1..3) | harness | review |

### Pre-existing violations found (not introduced by this change)

None.
