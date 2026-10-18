# Architect Spec — Reproducible Source and Vendor Tarballs With a Release Dry Run That Never Publishes (GitHub #444, part of #361)

- **Date**: 2026-10-18
- **Issue**: GitHub #444 (sub-issue of epic #361, item 2 "signed tags, source and vendor tarballs,
  release workflow", minus every owner-only part). No backlog id, so the branch is not registered
  in `scripts/sync_issue.py`; the commit carries `Closes #444` and `Refs #361`.
- **Branch**: `chore/release-dry-run`
- **Base commit**: `3f76090` (`origin/main`)
- **Lane**: one new local script, one new Docker harness, two new steps in the existing CI job
  `system-onnxruntime`, one new invariant module, documentation. No runtime crate, no dependency,
  no packaging recipe change.

## 0. Owner decisions

Relied on (recorded, none new):
- 2026-10-12 on #361: maintainer identity `soos-security`; **nothing is published until the owner
  says go**.
- 2026-10-18 (standing rules of this run): pull request only, the owner merges; hardware and
  owner-key checks are recorded as pending matrix rows.

Open owner questions that this change deliberately does **not** answer (out of scope): the
OpenPGP release key and signed `v*` tags, the GitHub Release upload, the PKGBUILD `source=()`
URL, AUR/COPR/PPA/OBS, the soos-models package (#361 Q3), the package split (Q5), the app id,
desktop file and AppStream (Q2), the maintainer email, and 0.1.0 vs 1.0.0 (Q6: the dry run uses
whatever `[workspace.package] version` says). Nothing below needs one of them.

## 1. Facts (verified on `3f76090`)

| Fact | Source |
|---|---|
| `[workspace.package] version = "0.1.0"`, `rust-version = "1.92"` | `Cargo.toml` |
| No `.cargo/` directory and no `.gitattributes` exist (no `export-ignore`/`export-subst`) | `ls -a`, repository root |
| `Cargo.lock`: 489 `checksum = ` lines, no `source = "git+` entry | `grep -c` |
| `cargo vendor --locked --offline vendor` on a `git archive` of HEAD vendors 489 directories (538 MiB), each with a `.cargo-checksum.json` whose last key is `"package":"<64 hex>"`; it prints the `[source.crates-io] replace-with = "vendored-sources"` / `[source.vendored-sources] directory = "vendor"` snippet on stdout | local run, cargo 1.98.1 |
| zstd on the 526 MB vendor tar: level 3 in 2 s, level 12 in 13 s (48 MB), level 19 > 4 min; level 12 with `-T1` and `-T3` gives byte-identical output | local run, zstd 1.5.7 |
| `/target/` is git-ignored | `.gitignore` line 9 |
| Job `system-onnxruntime` ("System ONNX Runtime (Arch, offline, advisory on PRs)", `timeout-minutes: 75`) took 8.5 min on `main` run 39137537240 | `gh run view` |
| `ADVISORY_CHECKS` of `scripts/pr_checks_gate.sh` is pinned to **exactly** the macOS job and the system ONNX Runtime job by `portable_crates_contract::test_mac_pr_loop_treats_only_the_macos_job_as_advisory` ("no other check may be added"); `ci-success-main` is pinned to `needs: [ci-success, system-onnxruntime]` by `system_onnxruntime_contract` | tests on `origin/main` |
| `test_sort_s5_ci_job_runs_offline_arch_harness` checks the job with `contains` needles (`timeout-minutes: 75`, `run: ./tests/docker/system_onnxruntime_test.sh`, no `continue-on-error`, no `${{`, actions pinned by SHA): extra steps keep it green | `tests/invariants/src/system_onnxruntime_contract.rs` |
| `maintainer_hygiene_contract::test_docs_index_lists_every_docs_page` requires every `Docs/*.md` in `Docs/README.md` | test on `origin/main` |
| The workflow is `permissions: contents: read`; `ci.yml` has no `upload-artifact` step (only `android.yml` and `fuzz.yml` do) | `.github/workflows/` |
| Repository `soos-security/soos` is public | #361 triage |

### 1.1 Consequence for the CI placement (deviation from the computed task, recorded)

The computed task asked for a separate job listed in `ADVISORY_CHECKS`. Adding a third name to
`ADVISORY_CHECKS` would require changing the assertion of
`test_mac_pr_loop_treats_only_the_macos_job_as_advisory` (AGENTS.md "Zero Test Weakening"
forbids it), and a separate job **not** listed there would block `scripts/pr_loop.sh` on every
pull request (not advisory). The only design that is advisory on pull requests, required on
`main`, and leaves every existing assertion untouched is to run the harness as **additional steps
of the existing `system-onnxruntime` job**: same check name (already in `ADVISORY_CHECKS`), same
`ci-success-main` dependency, same Arch image. The job budget (75 min, measured 8.5 min) leaves
room; the new step carries its own `timeout-minutes: 40`.

## 2. Components

### 2.1 `scripts/release_dry_run.sh` (local only)

```
scripts/release_dry_run.sh [--dry-run] [--offline] [--out-dir DIR]
scripts/release_dry_run.sh --check-vendor VENDOR_DIR CARGO_LOCK
scripts/release_dry_run.sh --help
```

- Exit codes: `0` success, `1` failure (tool missing, check failed), `2` usage error.
- Options are bounded: each at most once, `--out-dir` takes exactly one non-empty value that is
  not `/` and does not start with `-`; `--check-vendor` takes exactly two existing paths and no
  other option. Unknown options exit 2.
- Constants: `ZSTD_LEVEL=12`, `GZIP_LEVEL=9`, `ARCHIVE_PREFIX=soos-<version>/`, registry source
  `registry+https://github.com/rust-lang/crates.io-index`.
- Environment hygiene: `set -euo pipefail`, `umask 022`, `LC_ALL=C`, `TZ=UTC`, and `TAR_OPTIONS`,
  `GZIP`, `ZSTD_CLEVEL`, `ZSTD_NBTHREADS` unset (each would change the bytes).
- Inputs are **HEAD only**: version from `git show HEAD:Cargo.toml`, the `version` key inside the
  `[workspace.package]` table (validated `^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$`);
  `SOURCE_DATE_EPOCH = git log -1 --format=%ct HEAD` (exported). A dirty work tree only prints a
  warning (uncommitted changes are never packed).
- Steps:
  1. Preflight: `git`, `cargo`, GNU `tar` with `--sort`, `gzip`, `zstd`, `sha256sum`.
  2. Staging directory `mktemp -d "<out>/.staging.XXXXXX"`, removed by an EXIT trap; the three
     outputs are moved into `<out>` only when everything passed (no partial set).
  3. Source tarball: `git -c tar.umask=0022 archive --format=tar --prefix=soos-<ver>/ HEAD |
     gzip -n -9` → `soos-<ver>.tar.gz` (git sets every mtime to the commit time; `-n` drops the
     gzip name and time). The listing must not contain `soos-<ver>/.cargo/` or `soos-<ver>/vendor/`.
  4. Unpack it into the staging tree; **fetch step** (skipped with `--offline`): `cargo fetch
     --locked` in that tree — the only network access.
  5. `CARGO_NET_OFFLINE=true cargo vendor --locked --offline vendor` in that tree; the stdout
     snippet becomes `soos-<ver>/.cargo/config.toml`; it must contain `replace-with =
     "vendored-sources"` and `directory = "vendor"`.
  6. `check_vendor <tree>/vendor <tree>/Cargo.lock` (§2.2).
  7. Vendor tarball: `tar --sort=name --format=gnu --owner=0 --group=0 --numeric-owner
     --mtime=@SOURCE_DATE_EPOCH --mode=u+rwX,go+rX,go-w --hard-dereference -cf -
     soos-<ver>/.cargo/config.toml soos-<ver>/vendor | zstd -12 -T1 -q -c` →
     `soos-<ver>-vendor.tar.zst`.
  8. `SHA256SUMS` = `sha256sum soos-<ver>.tar.gz soos-<ver>-vendor.tar.zst` (bare names).
- `--dry-run` prints the plan (commit, version, epoch, output directory, the three file names and
  the steps) and writes, fetches and builds nothing (the output directory is not created).
- Never: `gh`, `git tag`, `git push`, `gpg`, `--sign`, `cosign`, `minisign`, `upload-artifact`,
  `cargo publish`, `curl`, `wget`, `docker push` (pinned by RDR-S1).

### 2.2 Vendor check (`--check-vendor`, also step 6)

Fails closed (exit 1) unless all hold:
- every `source = ` value in the lock file is the crates.io registry (no git, path or other
  registry);
- the number of registry packages equals the number of `checksum = "<64 hex>"` lines;
- every top-level entry of the vendor directory is a real directory (no symlink, no file) with a
  `.cargo-checksum.json` holding exactly one `"package":"<64 hex>"`;
- the sorted list of vendored package checksums equals the sorted list of `Cargo.lock` checksums
  (same multiset: no missing, extra, duplicate or altered crate). The first differing checksum is
  printed (checksums are public crate hashes, not secrets).

### 2.3 `tests/docker/release_dry_run_test.sh`

Host mode (default): reads the MSRV like the system harness; work directory
`<root>/target/release-dry-run-harness-<run id>-<pid>` (removed on exit unless `SOOS_RDR_KEEP=1`);
run 1 `scripts/release_dry_run.sh --out-dir <w>/run1` (with the fetch), run 2 `--offline --out-dir
<w>/run2`; `cmp` of each of the three files and identical file lists; builds
`tests/docker/Dockerfile.system-ort` (bounded retry `SOOS_RDR_BUILD_ATTEMPTS` 1..3, default 2);
then `docker run --rm --network none` with **only** `<w>/run1` mounted read-only at `/release` and
the harness file mounted read-only (never the repository, never a cargo registry volume),
`ORT_SKIP_DOWNLOAD=1`, `CARGO_NET_OFFLINE=true`, `CARGO_HOME=/build/cargo-home`,
`RUSTUP_TOOLCHAIN=<MSRV>`, `CARGO_BUILD_JOBS` = `SOOS_RDR_JOBS` (1..64, default 4).

In-container mode (`--in-container`, refused outside the image as root): only `lo`; no
`/usr/local/cargo/registry` or `/usr/local/cargo/git`; empty `CARGO_HOME`; `sha256sum -c --strict
SHA256SUMS` with exactly two entries; unpack both tarballs into `/build`; no `.git` anywhere; the
vendored `.cargo/config.toml`; `cargo build --locked --offline --release --workspace
--no-default-features --features system-onnxruntime`; `soos-daemon` needs `libonnxruntime.so.1`,
`libpam_soos.so` does not; no `~/.cache/ort.pyke.io`.

### 2.4 CI

Two steps appended to job `system-onnxruntime`, after the existing harness step: `rustup toolchain
install --profile minimal` (the host cargo of `rust-toolchain.toml`) and `./tests/docker/
release_dry_run_test.sh` with `timeout-minutes: 40`. No `upload-artifact`, no cache save, no
`${{ }}`, no new permission. Every produced file stays on the runner and is discarded with it.

## 3. Invariants (new module `tests/invariants/src/release_dry_run_contract.rs`)

| Id | Kind | Content |
|---|---|---|
| RDR-S1 | static | Code lines (comments stripped) of the script and the harness, and the new CI steps, contain none of the forbidden publish/sign/upload/network commands of §2.1 |
| RDR-S2 | behavioral | `--check-vendor` accepts a matching fixture; rejects an altered checksum, a missing crate, an extra crate, a duplicate, a missing `.cargo-checksum.json`, a symlink entry, a git source and a registry package without checksum |
| RDR-S3 | behavioral + static | `--dry-run` names `soos-<version>` with the version of `[workspace.package]` (parsed independently by the test) and the epoch of `git log -1 --format=%ct`, creates nothing; the script reads `[workspace.package]` from `git show HEAD:Cargo.toml` |
| RDR-S4 | behavioral | Usage errors exit 2 (unknown option, repeated option, missing value, `/`, `--check-vendor` arity) |
| RDR-S5 | static | Determinism flags and order: fetch before vendor, `check_vendor` after vendor and before the vendor tar, `tar.umask`, `gzip -n`, the tar normalization flags, `ZSTD_LEVEL=12`, `-T1`, the unset environment variables |
| RDR-S6 | static | Harness: two runs, `cmp`, `Dockerfile.system-ort`, `--network none`, no repository or registry mount, `ORT_SKIP_DOWNLOAD=1`, the exact build command, `.git` and registry checks |
| RDR-S7 | static | CI: the `system-onnxruntime` job runs the dry-run harness after the system harness with its own timeout; no `upload-artifact` or release step in `ci.yml`; workflow permissions stay `contents: read` |
| RDR-S8 | static | ADR "Release Dry Run Publishes Nothing", `Docs/RELEASING.md` owner-only checklist, pointer from `Docs/PACKAGING_AND_PROVISIONING.md` §7.5 and `Docs/CI_CD_AND_SECURITY.md` |

## 4. Traceability

- ADR `[2026-10-18] Release Dry Run Publishes Nothing` (amends the CI item of ADR 2026-10-13
  "Distribution Builds: System ONNX Runtime": the job also runs the dry run).
- Matrix component `release-dry-run`: RDR1–RDR6 (criteria), RDR-S1–RDR-S8 (contracts), owner
  rows RDR-O1–RDR-O4 ⬜ Pending (OpenPGP key, signed tag, release upload, channels).
- Backlog section GitHub #444; `Docs/RELEASING.md` (new, indexed in `Docs/README.md`); walkthrough
  `AI/walkthroughs/239_release_dry_run.md`.
