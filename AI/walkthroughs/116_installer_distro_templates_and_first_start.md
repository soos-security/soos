# Walkthrough 116 — One PAM Template per Distribution, Hardened Model Download, Guarded First Start

- **Date**: 2026-09-30
- **Issues**: GitHub #208 (ONB-12), #209 (ONB-13), #211 (ONB-15)
- **Branch**: `fix/p2-installer-distro-templates-unit`
- **Matrix criteria**: IDT1–IDT8 (new component `installer-distro-templates-and-first-start`)
- **ADR**: `AI/DECISIONS.md` 2026-09-30 "One Distribution PAM Template per Install, Guarded First Start"

---

## 1. Findings

| Finding | Defect |
|---|---|
| ONB-12 (#208) | `resolve_download_url` in `scripts/download_models.sh` returned early for every manifest URL (all end in `.onnx`), so its per-model table, which included four v1.0.0 models that are no longer in the manifest, could never run. curl had no `--max-filesize`, and the temporary file name `${dest_path}.tmp.$$` was predictable. |
| ONB-13 (#209) | Steps 6a–6c of `scripts/install.sh` installed the Debian profiles, the Fedora authselect profile and the Arch snippet on every distribution. As a result a `.deb` shipped a Fedora profile, and `/etc/pam.d/soos.snippet` was installed, which Linux-PAM reads as a service named `soos.snippet`. The PKGBUILD installed the same file. |
| ONB-15 (#211) | `soos-daemon.service` had no precondition, so a fresh install without models crash-looped until the start limit. A missing `soos` group (`Group=soos`) was only a warning, and nothing gave a readiness signal. |

`StartLimitIntervalSec=60` / `StartLimitBurst=5`, `--proto '=https' --proto-redir '=https'`,
`--tlsv1.2` and `--max-time 900` were already on `main` (GitHub #167). This change keeps them and
tests them.

## 2. Design

- `install.sh --distro auto|debian|fedora|arch|none`, with `auto` as the default. `detect_pam_family`
  reads `ID` and `ID_LIKE` line by line. It never sources the file, strips the quotes, and uses the
  first token that matches a known family. With `--destdir`, only `<stage>/etc/os-release` and
  `<stage>/usr/lib/os-release` are read. If neither exists, or the distribution is unsupported, no
  template is installed and a `--distro` hint is printed. The build host is never used, which is the
  same rule as for the PAM module directory. An unknown value is a preflight error (exit 2) and
  nothing is written.
- The Arch snippet is installed at `${PREFIX}/share/soos/pam/system-auth.snippet`. It is reference
  material that the administrator merges into `system-auth` by hand, so nothing is placed in
  `/etc/pam.d`. `uninstall.sh` removes the new path and the legacy `/etc/pam.d/soos.snippet`.
- Packaging selects its family explicitly: `build_deb.sh` and `packaging/debian/rules` pass
  `--distro debian`, and `build_arch.sh` passes `--distro arch`. `soos.spec` was already correct
  (authselect only). `tests/docker/test_packages.sh` gains `verify_distro_templates <family>
  <listing cmd>` for each package branch. It requires the family's own files and rejects any foreign
  template or `etc/pam.d/` entry.
- `download_models.sh`:
  - The download URL is `source_url` as written in the manifest.
  - `MAX_MODEL_BYTES=268435456` (256 MiB). The issue suggested 50 MB, which would reject the attested
    136,619,444-byte embedding model.
  - The cap is enforced by curl (`--max-filesize`) and by a `wc -c` check before hashing.
  - `SOOS_MODEL_MAX_BYTES` accepts 1 to 10 digits, 1..cap, and can only lower the cap.
  - The temporary file is `mktemp "${TARGET_DIR}/.<file>.XXXXXXXX"` created under `umask 077`. An
    EXIT/INT/TERM trap removes it.
- `soos-daemon.service` gains `ConditionPathExists=/var/lib/soos/models/manifest.toml` in `[Unit]`.
  This is the file `download_models.sh` writes last. There is no condition on `master.key`, because
  `initialize_pipeline` calls `MasterKey::load_or_create`.
- On a live install, the preflight fails with `Cannot create the 'soos' system group` when the group
  is missing and neither `groupadd` nor `addgroup` exists. The old warning branch in step 1 is now an
  error.
- `scripts/wait_daemon_ready.sh` checks readiness in three steps:
  1. The manifest must exist. If it is missing, the script fails at once, because the unit condition
     would skip the start.
  2. It polls `-S <socket>` twice a second for at most `--timeout` seconds (1..300, default 30).
  3. It runs `soos-admin --format json --socket-path <sock> status`.

  The exit codes are 0 (ready), 1 (not ready) and 2 (usage). `install.sh --start` (live install, not
  with `--skip-systemd`) runs `systemctl start` after the unit is enabled, then runs the helper. The
  install is already committed at that point, so a failure exits 70 and nothing is rolled back.

## 3. Tests (red → green)

New module `tests/invariants/src/installer_templates_contract.rs` (12 tests). No existing test was
modified.

Red on `origin/main` (`cargo test -p soos-invariants --all-features installer_templates_contract`):
11 failed, 1 passed. `test_install_rejects_unknown_distro_value` already passed, because `--distro`
was an unknown option and was rejected before any write. It stays as the contract for the new option.

| Test | Red reason on `main` |
|---|---|
| `test_install_stages_only_the_selected_distro_template` | `--distro` unknown option (exit 1) |
| `test_install_detects_distro_from_os_release_id_and_id_like` | all three families staged on every stage |
| `test_packaging_selects_one_distro_template_and_never_ships_snippet_in_pam_d` | no `--distro debian` in `build_deb.sh` |
| `test_uninstall_removes_relocated_and_legacy_arch_snippet` | `/usr/share/soos/pam/system-auth.snippet` left behind |
| `test_download_models_dry_run_reports_only_manifest_urls` | the legacy table rewrote the `scrfd_500m_kps` / `arcface_w600k_mbf` / `minifasnet_v2_pad` / `ultraface_slim_320` URLs |
| `test_download_models_enforces_size_cap_before_hashing` | a 64-byte model was deployed with a 16-byte cap |
| `test_download_models_hardens_curl_and_temporary_files` | no `--max-filesize` |
| `test_daemon_unit_requires_deployed_models_and_bounds_restarts` | no `ConditionPathExists=` |
| `test_install_requires_group_and_offers_bounded_readiness` | warning downgrade still present |
| `test_wait_daemon_ready_succeeds_when_socket_and_status_answer` | helper missing |
| `test_wait_daemon_ready_fails_closed_and_bounded` | helper missing |

Green: all 12 pass. The existing installer, rollback and PAM-snapshot contracts in
`installer_contract.rs` and `lib.rs` still pass unchanged. For example,
`test_pam_rollback_is_wired_and_documented` still finds "Installing distribution PAM configuration
templates" after `pam_snapshot.sh`. `verify_distro_templates` was also exercised locally against
simulated `dpkg-deb -c`, `rpm -qlp` and `bsdtar -tf` listings: it accepts each family's own content
and rejects a `.deb` with `etc/pam.d/soos.snippet` and a Fedora check against a Debian listing.

## 4. Audit notes

- Shell only. No Rust production code, no PAM or FFI path, and no new logging of secrets.
- `os-release` is parsed as data and never sourced. The tokens are split with `read -ra`, so glob
  characters cannot expand.
- Every loop and wait is bounded: the socket poll is at most 600 iterations, and both the model size
  and the curl time are capped.
- The temporary model file is created 0600 under `umask 077` in the root-owned 0755 models directory
  and is renamed atomically after the SHA-256 match.

## 5. Not covered hermetically (follow-ups)

- The actual systemd `condition failed` state after `systemctl start` on a system without models.
  The issue asked for a Docker test with systemd as PID 1, and the current Docker matrices do not
  boot systemd.
- The `verify_distro_templates` checks run only in the Docker package jobs (`package-deploy` in
  ubuntu on every PR, `distro-deploy` in fedora and arch on `main`).
