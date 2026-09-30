# Walkthrough 93 — Fail-Closed Transactional Installer, Dependency Lists and Python-Free Model Download

- **Date**: 2026-09-30
- **Issues**: Review findings ONB-06 (GitHub #164), ONB-07 (GitHub #165), ONB-09 (GitHub #167) — **Branch**: `fix/installer-preflight-and-deps`
- **Matrix criteria**: INS1–INS9 (new); PK1 staging tests migrated (assertions unchanged)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, §3.3 and findings ONB-06/07/09)
established, and this change re-verified, that the source installation path was not reliable:

1. `scripts/install.sh` searched `target/release` then `target/debug`, only warned when an artifact was
   missing ("not built yet. Skipped."), printed "completed successfully" and exited 0. Packages built with
   `build_deb.sh --skip-build` could therefore ship without `pam_soos.so` or with debug binaries.
2. No root check for a live install, `chmod 0755` applied to `/usr/bin` and the PAM module directory on
   every run, the unit enabled before models were fetched, and no cleanup on failure (half-installed host).
3. The build needs OpenSSL headers (`openssl-sys` ← `native-tls` ← `ureq`, build-dependency of `ort-sys`),
   libclang (`bindgen` for `v4l2-sys-mit`) and pkg-config; none of this was documented, and the GUI's
   dlopen'ed libraries were undeclared.
4. `scripts/download_models.sh` required Python >= 3.11 (`tomllib`, then a nonexistent `tomllib_fallback`
   module), never preflighted `curl`, and — found while writing the tests — accepted a manifest `filename`
   such as `../escape.onnx`, writing the model **outside** the target directory.

Objectives (the parts of the §3.3 bootstrap plan these issues require, not the full `bootstrap.sh`):
strict read-only preflight, explicit release build or a clear error, non-zero exit on missing artifacts,
transactional install with rollback, complete and single-sourced dependency lists, and a model downloader
that needs neither Python nor an unchecked tool.

## 2. Architect Design

No Rust production code is touched; the change is shell scripts, packaging metadata, tests and docs.

- **`scripts/install.sh` phases**:
  - *Phase A — read-only preflight*: root required when `--destdir` is empty (unless `--dry-run`);
    required tools; optional `--build` (runs `scripts/check_build_deps.sh`, then
    `cargo build --release --locked --workspace`, as `$SUDO_USER` under sudo; exit `40` on failure);
    artifact set resolved from `--artifact-dir` or `${CARGO_TARGET_DIR:-target}/release` — never
    `target/debug`; a cargo `debug` profile directory is refused unless `--allow-debug-artifacts`; all five
    artifacts required unless `--allow-missing`; `download_models.sh --preflight` validates the manifest and
    tools. Any error exits `2` before the first write. `--dry-run` stops after this phase.
  - *Phase B — journal*: arrays of created files, created directories, and overwritten files with their
    saved copy (private `mktemp -d` directory); flags for group creation, unit installation/enablement.
    An `EXIT` trap replays the journal backwards unless the run committed; `INT`/`TERM` exit through it.
  - *Phase C — steps*: group, directories (`ensure_dir`: creates missing levels and journals them; applies
    the mode only to created directories, or always for soos-owned ones), live-only master key (journaled
    only when it did not exist), artifacts and templates through `tracked_install` (temp file + rename in
    the target directory), models (files added by `download_models.sh` are journaled; failure exits `60`),
    and finally `systemctl daemon-reload` + `enable` (live only). A partial `--allow-missing` run ends
    with an `INCOMPLETE` banner.
- **`scripts/download_models.sh`**: pure-bash parser for the fixed manifest schema (`[models.<id>]` tables;
  `filename`, `sha256`, `source_url`, `license` as single-line double-quoted strings; other tables, keys and
  multi-line arrays skipped; ambiguous values for the four keys fail closed); validation (bare file name,
  64-hex digest, `https://`/`file://` only, >= 1 model); tool preflight (`sha256sum`/`shasum`, `curl` when a
  remote source exists) before any write; new `--preflight` mode; `curl --proto '=https' --proto-redir
  '=https' --tlsv1.2 --max-time 900`.
- **`scripts/check_build_deps.sh`**: the single source of the per-distribution `build`, `gui` and `models`
  package lists (`--print-packages`), plus a read-only build host preflight (cargo, rustc, cc, pkg-config,
  `pkg-config openssl`, `security/pam_appl.h`, libclang) that prints the install command.
- **Packaging metadata**: `debian/control` Build-Depends `+ libclang-dev libssl-dev`, `Recommends:` GUI
  libraries; `build_deb.sh` control `Depends` gains the linked libraries (`libc6 libgcc-s1 libstdc++6
  libpam0g`, since it does not run `dpkg-shlibdeps`) and the same `Recommends:`; `soos.spec`
  `BuildRequires: gcc-c++ openssl-devel pkgconf-pkg-config`; `PKGBUILD` makedepends `+ openssl pkgconf`.
- **Unit**: `StartLimitIntervalSec=60` / `StartLimitBurst=5` in `[Unit]` so a daemon without models does
  not crash-loop forever. `ConditionPathExists=/var/lib/soos/models/manifest.toml` was **not** added:
  `models_dir` is configurable in `daemon.toml`, so a hard-coded condition would silently disable custom
  layouts.

## 3. Plan Evaluation

Checked against `AI/ARCHITECTURE.md` §5/§7/§9 and the project facts §3 modes: modes unchanged
(0700 biometrics/evidence, 0755 models, 0600 key, 0750 `root:soos` run dir, 0755 binaries, 0644 module);
the master key is still never generated under `--destdir` (GitHub #144) and install.sh keeps delegating to
`scripts/provision_master_key.sh` with no inline generator; PAM activation profiles and uninstall logic are
untouched (owned by GitHub #161/#166). The review critique's note that `--allow-missing` must not become
the default is honoured. The `tls-rustls` alternative for `ort` (removing OpenSSL) was not taken: it changes
`Cargo.lock` and the supply-chain review scope; documenting the requirement is the minimal fix.

## 4. Tester Contract

New module `tests/invariants/src/installer_contract.rs` (committed first, red):

| Test | Contract |
|---|---|
| `test_install_fails_closed_when_artifact_dir_is_empty` | non-zero, every artifact named, destination untouched, no success banner |
| `test_install_fails_closed_when_one_artifact_is_missing` | missing `libpam_soos.so` fatal; `--allow-missing` explicit and reported `INCOMPLETE` |
| `test_install_refuses_debug_profile_artifacts` | `target/debug` refused, `--allow-debug-artifacts` override, no implicit `target/debug` search |
| `test_install_stages_every_artifact_with_expected_modes` | names, contents, 0755/0644, no temp file left |
| `test_install_never_chmods_preexisting_system_dirs` | pre-existing 0775 `usr/bin` and PAM dir kept; `biometrics` tightened to 0700 |
| `test_install_rolls_back_every_change_on_failure` | checksum failure → exact pre-run tree, overwritten file content/mode restored, rollback reported |
| `test_install_live_mode_requires_root` | non-root live install refused before provisioning (skipped when run as root) |
| `test_install_dry_run_runs_preflight_without_mutation` | dry run fails on missing artifacts, never writes |
| `test_install_orders_models_before_unit_enable_and_builds_release` | model step before `systemctl enable`, locked release build, deps preflight, bounded restarts |
| `test_download_models_deploys_and_verifies_without_python` | restricted PATH (no python3, no curl): deploy, 0644/0755, `--check-only`, tamper detection |
| `test_download_models_parses_repository_manifest_without_python` | all 3 filenames and digests of `models/manifest.toml` reported |
| `test_download_models_rejects_malformed_or_unsafe_entries` | traversal, absolute, hidden, short/non-hex digest, empty/http URL, missing digest, no models |
| `test_download_models_preflight_requires_curl_for_remote_sources` | missing curl named, nothing written, no python/tomllib in the script |
| `test_check_build_deps_lists_required_packages` | per-distro lists contain the graph-derived packages, models list has curl and no python, unknown distro rejected, missing cargo named |
| `test_packaging_and_docs_declare_complete_dependencies` | control/spec/PKGBUILD/build_deb.sh declarations, every listed package documented, `ORT_LIB_LOCATION`, README section |

**Contract migration (explicit spec change, no assertion weakened)**: `test_install_script_creates_required_directories`
and `test_install_script_destdir_stages_no_key_material` invoked `install.sh --destdir` with no artifacts and
asserted success — precisely the behaviour ONB-06 classifies as a defect. Both now pass `--artifact-dir` with a
complete fixture artifact set (`stage_fixture_artifacts`); every existing assertion is kept unchanged (same
precedent as the #144 migration of the first test). Recorded in `AI/BACKLOG.md` (#164.1).

Red evidence (before implementation): 17 failures — the 15 new tests plus the 2 migrated tests
(`Unknown option: --artifact-dir`); notably `path traversal: malformed entry must be rejected` (the old
script wrote `../escape.onnx` outside the target) and `line 132: python3: command not found`.

## 5. Auditor Constraints

1. No key material under `--destdir`; no inline `openssl rand` or `/dev/urandom` in install.sh (invariant PMK4).
2. Rollback only touches journaled paths: `rm -f` on created files, `rmdir` (empty only) on created
   directories, restores through temp + rename; never recursive deletion of a system path.
3. Backups live in a `mktemp -d` directory (mode 0700) and are kept only when a restore failed.
4. Manifest values are never evaluated; file names are bare names (traversal closed); only https/file sources.
5. The optional build never uses `--all-features` (the `fault-injection` feature stays opt-in) and never runs
   as root inside a user's checkout when `$SUDO_USER` is known.
6. `--dry-run` and preflight failures perform zero writes.
7. File modes exactly as `.agents/skills/dev-workflow/references/project-facts.md` §3.

## 6. Implementation & Verification Results

- Green: `cargo test -p soos-invariants` 48/48; full gate below.
- **Real builds with only the documented lists** (scratch clone outside the worktree, bare images + rustup):
  `cargo build --release --locked --workspace` succeeded in `ubuntu:24.04`, `fedora:40` and `archlinux`
  (all five artifacts produced). `ldd` on the Ubuntu build: binaries link only `libc`, `libm`, `libgcc_s`,
  `libstdc++` (daemon, enroll, gui) and `libpam` (module) — OpenSSL is build-time only.
- **Live root install in containers** (`ubuntu:24.04` and `archlinux`, no python in either image, artifacts
  from the real builds): `download_models.sh --dry-run` on the real manifest passes; a partial artifact set
  exits non-zero with `/var/lib/soos` absent; a bad model checksum exits `60` and leaves nothing named
  `*soos*` on the filesystem, the freshly created `soos` group and master key removed; a valid install gives
  `755 /var/lib/soos`, `700` biometrics/evidence, `755` models, `600 root:root master.key`,
  `750 root:soos /run/soos`, `755` binaries, `644 pam_soos.so`, `644` model; a re-install keeps the same key.
- Gate: `cargo fmt --all -- --check` clean; `cargo clippy --locked --workspace --all-targets --all-features
  -- -D warnings` clean; `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`
  576 passed, 0 failed; `./scripts/candid_review.sh` PASSED; `bash -n` and `shellcheck --severity=warning`
  (koalaman/shellcheck:stable, only the pre-existing unused `BOLD` colour warning excluded) clean on
  `install.sh`, `download_models.sh`, `check_build_deps.sh`, `build_deb.sh`.

## 7. Known Limitations / Follow-ups

- The full `scripts/bootstrap.sh` (package installation, rustup, PAM activation canary) from review §3.3
  remains a separate delivery; this change provides its building blocks (`check_build_deps.sh`,
  `install.sh --build`, fail-closed/transactional install).
- The Docker images (`Dockerfile`, `tests/docker/Dockerfile.*`) do not yet install `libssl-dev` /
  `openssl-devel`; they belong to the Docker matrix fix (GitHub #162/#168) and should source
  `scripts/check_build_deps.sh --print-packages build`.
- Fedora/RHEL live install and RHEL 9 package names (`clang-devel` in AppStream) were verified by build only,
  not by a live `install.sh` run; Debian 12 was not built separately from Ubuntu 24.04.
- A mismatched pre-existing model file replaced by a verified download is not restored on rollback (the old
  file was invalid by definition).
- `build_deb.sh --skip-build`, `build_arch.sh --skip-build` and `debian/rules` now fail closed on a stale or
  incomplete `target/release`, which is the intended behaviour.
