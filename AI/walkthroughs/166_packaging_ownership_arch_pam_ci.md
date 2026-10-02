# Walkthrough 166 — Root-Owned Packages, Docker Target Overlays, Exact Arch `system-auth` Edit

- **Date**: 2026-10-02
- **Issue**: GitHub #301 (ONB-NEW-1, CRITICAL), #308 (TCI-NEW-1, MAJOR), #309 (ONB-NEW-2, MAJOR),
  #316 (ONB-NEW-3, ONB-NEW-4, TCI-NEW-2, TCI-NEW-3) — review of 2026-10-02
  (`AI/reviews/FULL_PROJECT_REVIEW_2026-10-02.md`, base `095eae9`); no backlog issue —
  **Branch**: `fix/packaging-ownership-and-arch-pam`
- **Matrix criteria**: POA1–POA12 (component `packaging-ownership-and-arch-pam`, after the
  `systemd-unit-acceptance` section); PK2, PK7 and DV3 evidence corrected

## 1. Context & Objectives

1. **#301** `scripts/build_arch.sh` archived its staging tree with plain `tar`: every entry,
   including `usr/libexec/soos/soos-daemon`, `usr/lib/security/pam_soos.so` and
   `usr/libexec/soos/provision-master-key`, recorded the builder's uid. pacman extracts owners, so a
   package built by a normal user let that user replace the root daemon or the PAM module after
   `pacman -U` (authentication bypass). CI builds as root and never saw it.
2. **#308** `run_tests.sh` and the CI job `pam-integration` bind-mounted the checkout read-write
   without a `/workspace/target` overlay and built as root, leaving root-owned `target/release` and
   `target/fault-injection` in the host checkout. `install.sh --build` only handled `SUDO_USER` and
   built as root under doas, pkexec or su.
3. **#309** The documented Arch edit (`auth required pam_unix.so` followed by the event line, stock
   jumps) sent a correct password into `[default=die] pam_faillock.so authfail`; §2, the snippet and
   the full `packaging/pam/arch/system-auth` (without faillock, `pam_systemd_home`, `pam_env`)
   disagreed; the Arch harness only tested a synthetic stack.
4. **#316** ONB-NEW-3: two divergent Arch paths (the PKGBUILD was never run), packages shipped the
   unit under `/etc/systemd/system` and shipped `/run/soos`. ONB-NEW-4: `install.sh --distro debian`
   installed `Default: yes` profiles. TCI-NEW-2: the priority assertion could not fail for 2600/1200.
   TCI-NEW-3: unpinned base images and an unretried `ort-sys` download.

## 2. Architect Design

- `scripts/build_arch.sh`: `TAR_OWNER_FLAGS` = `--owner=root:0 --group=root:0` (GNU tar, detected
  with `--version`) or `--uid 0 --gid 0 --uname root --gname root` (bsdtar) on both archive commands;
  new selectors `--tar <bsdtar|tar>` and `--compress <auto|zstd|gzip>` (defaults unchanged: bsdtar
  if present, zstd if present) so both paths are testable on any host.
- `scripts/install.sh`: `--unitdir <DIR>` (default `SYSCONFDIR/systemd/system`); the runtime
  directory is created on a live install only; `invoking_user()` resolves `SUDO_USER`, `DOAS_USER`,
  then `PKEXEC_UID` through `getent passwd`; `run_release_build` builds through `runuser -u` as that
  user, or, as root with no invoking user, refuses when the checkout owner (`stat -c %u`) is not 0.
- Packages: `build_deb.sh`, `build_arch.sh` and `packaging/debian/rules` pass
  `--unitdir /usr/lib/systemd/system`.
- Arch PAM: one four-line edit of the stock pambase `system-auth` (primary rule after
  `pam_faillock.so preauth`, event rule right after `pam_unix.so`, `pam_systemd_home` jump 2 → 3,
  `pam_unix` jump 1 → 2); `packaging/pam/arch/system-auth` = stock pambase `20260616-1` + that edit,
  the snippet carries the two lines and the jump instructions, the guide shows the file verbatim.
  The primary control stays `[success=done default=ignore]` (pinned by the existing invariant and
  shared with Debian, Fedora and GDM); consequence documented in the ADR: a face match skips
  `pam_env` and `pam_faillock authsucc`.
- Debian: both pam-configs `Default: no`; the `.deb` postinst keeps
  `pam-auth-update --package --enable soos soos-notify` (orchestrator decision).
- Docker: named volume `soos-sandbox-target` over `/workspace/target` in `run_tests.sh` and the
  `pam-integration` job; base images pinned by index digest (`Dockerfile`,
  `tests/docker/Dockerfile.{ubuntu,fedora,arch}`, harness default images).
- Arch paths: both kept and proven equivalent (ADR 2026-10-02 "Arch Packaging Paths Proven
  Equivalent"): the package harness builds `packaging/arch/PKGBUILD` with `makepkg --repackage` as an
  unprivileged user and compares paths, modes and owners with the `build_arch.sh` package.
- Invariants touched: PAM fallback and ordering (Arch stack), package integrity (ownership),
  host isolation of Docker harnesses. No Rust production code changed.

## 3. Plan Evaluation

Scope and decisions fixed by the orchestrator prompt (no separate plan-evaluator run for this review
batch): Red first, Docker only, no change to existing assertions without approval. The owner approved
the TCI-NEW-2 change of `test_pam_config_ordering_matches_spec` during the batch (2026-10-02).
Single-path consolidation of the Arch packaging was preferred but would change pinned invariants, so
it is reported instead (§9).

## 4. Tester Contract

| Test (path::name) | Matrix ID | Red evidence (on `main` @ `095eae9`) |
|---|---|---|
| `tests/invariants/src/packaging_ownership_contract.rs::test_arch_package_archive_entries_are_root_owned_on_every_archive_path` | POA1 | `bsdtar/gzip: archive entries not owned by root:root (pacman extracts these owners): .PKGINFO 1000:1000 (<builder>/<builder>) ... usr/libexec/soos/soos-daemon 1000:1000 ... usr/lib/security/pam_soos.so 1000:1000` (the `--tar`/`--compress` selectors were added first, without the owner fix) |
| `...::test_package_builders_force_root_ownership_statically` | POA2 | `build_arch.sh must pass --owner=root:0 to its tar implementation` |
| `...::test_read_write_workspace_mounts_overlay_target_with_a_volume` | POA3 | lists `run_tests.sh: docker run --rm --name "${CONTAINER_NAME}" -v "$(pwd)":/workspace ...` and the `pam-integration` step of `.github/workflows/ci.yml` |
| `...::test_install_build_as_root_drops_to_invoking_user_or_refuses` | POA4 | `doas: install.sh --build as root must build as 'doasuser' through runuser (calls: "cargo build --release --locked --workspace\n")` |
| `...::test_arch_system_auth_is_stock_pambase_with_exact_soos_edit` | POA5 | `packaging/pam/arch/system-auth auth rules must be stock pambase plus the two soos lines` (3 auth rules found) |
| `...::test_arch_harness_exercises_the_edited_stock_system_auth` | POA6 | `Dockerfile.arch must keep the stock pambase system-auth before overwriting it` |
| `...::test_packages_stage_unit_in_unitdir_and_never_ship_run_dir` | POA7 | `install.sh` rejected the unknown option `--unitdir` (usage printed) |
| `...::test_debian_pam_configs_are_opt_in_and_deb_enables_explicitly` | POA8 | `packaging/pam/debian/soos must declare 'Default: no'` |
| `...::test_docker_base_images_are_pinned_by_digest` | POA9 | `Dockerfile: base image not pinned by digest: FROM ubuntu:24.04` |
| `...::test_package_harness_builds_as_non_root_and_checks_ownership` | POA10 | `tests/docker/test_packages.sh must contain "runuser -u soosbuild --"` (needle later aligned with the harness variable, see below) |
| `tests/invariants/src/lib.rs::tests::test_pam_config_ordering_matches_spec` (strengthened) | POA11 | mutation `Priority: 1200` in `soos-notify`: new assertion fails with `soos-notify Priority 1200 must be below pam_unix (256)`; the former `contains("Priority: 12")` accepted it |

Docker Red evidence (same stock images):

- **#301** (`soos-distro-val-arch`, main's `build_arch.sh` run by `soosbuild`, uid 1001):
  `-rwxr-xr-x 0 1001 1001 ... usr/libexec/soos/soos-daemon`, `-rw-r--r-- 0 1001 1001 ...
  usr/lib/security/pam_soos.so`; after `pacman -U`: `1001:1001 /usr/libexec/soos/soos-daemon`,
  `1001:1001 /usr/lib/security/pam_soos.so`, `1001:1001 /usr/libexec/soos/provision-master-key`.
- **#309** (former §5.2 edit applied to the real stock pambase `system-auth`, real `pam_soos.so`,
  recording mock daemon in `deny` mode, locker `auth include system-auth`):
  `[pam_test_runner] authenticate result=6 (Permission denied)` for the **correct** password,
  `events after valid password: 1`, `faillock tally after valid password: 1`; the wrong password
  then brought the totals to 2 events and a tally of 2.
- **ONB-NEW-4** (`ubuntu:24.04`, D6 with `Default: yes` profiles and the `Default: no` precheck
  removed): `pam-auth-update --package changed /etc/pam.d/common-auth after install.sh: facial
  authentication was enabled implicitly` (generated stack showed `pam_soos.so`, `success=2`
  `pam_unix`, the event hook).

### Existing tests touched

- `tests/invariants/src/lib.rs::tests::test_pam_config_ordering_matches_spec` — **owner-approved
  (2026-10-02)** change of an existing assertion (TCI-NEW-2):
  - old: `assert!(deb_soos_content.contains("Priority: 260") || deb_soos_content.contains("Priority: 26"))`
    and `assert!(deb_notify_content.contains("Priority: 128") || deb_notify_content.contains("Priority: 12"))`;
  - new: `pam_config_priority()` parses the single `Priority:` line of each profile (exactly one,
    numeric) and asserts `soos > DEBIAN_UNIX_PROFILE_PRIORITY (256) > soos-notify`.
  Every other assertion of the test is unchanged.
- No other existing assertion changed. Existing invariants that keep pinning
  `packaging/pam/arch/system-auth` (`test_pam_config_ordering_matches_spec`: file exists, soos before
  `pam_unix`, event after `pam_unix`, snippet lines) pass unchanged because the file was rebased on
  stock pambase instead of being deleted.
- The new POA6 and POA10 needles were adjusted while developing them (this branch's own tests, never
  on `main`): POA6 looks for `cp -p system-auth /usr/local/share/soos-test/system-auth.stock`
  because the existing `distro_matrix::test_sandbox_dockerfiles_produce_multiline_pam_stacks`
  executes every `RUN` mentioning `/etc/pam.d/`, so the copy uses `cd /etc/pam.d`; POA10 checks
  `BUILD_USER="soosbuild"` and `runuser -u "${BUILD_USER}" -- bash scripts/build_{deb,arch}.sh`.

## 5. Auditor Constraints

1. No package entry may carry a non-root owner, on any archive path or tar implementation. Met
   (POA1/POA2; Docker: deb, Arch and makepkg archives `root:root`, RPM `root root`).
2. Installed package files are uid 0 / gid 0 (the RPM `%ghost /run/soos` is `root:soos` by design).
   Met (`verify_installed_files_root_owned`).
3. No container writes the host checkout's `target/`; read-only mounts are allowed. Met (POA3,
   POA12).
4. `install.sh --build` never runs cargo as root in a user-owned checkout; the refusal happens
   before any build or mutation. Met (POA4; the refusal returns from `run_release_build`, exit 40,
   before the artifact preflight).
5. Arch PAM: a correct password must never reach the event line or `pam_faillock authfail`; a wrong
   password must reach both; `preauth` stays first so a locked account fails even on a face match;
   no error is converted into success (the primary rule keeps `default=ignore`). Met (POA5, POA6).
6. Debian: installing the profiles never activates them; only the explicit `--enable` of the
   `.deb` postinst does. Met (POA8).
7. No host installer run, no sudo; every live harness ran in disposable non-privileged containers.
   Met.
8. English only; no secrets, keys or biometric data in logs or fixtures. Met.

## 6. Implementation

| File | Change |
|---|---|
| `scripts/build_arch.sh` | owner flags on both archive commands, `--tar`, `--compress`, `--unitdir /usr/lib/systemd/system`, `zstd -q` |
| `scripts/build_deb.sh`, `packaging/debian/rules` | `--unitdir /usr/lib/systemd/system` |
| `scripts/install.sh` | `--unitdir`, runtime directory on live installs only, `invoking_user()` (sudo/doas/pkexec), refusal of root builds in non-root checkouts |
| `packaging/pam/arch/system-auth` | rebased on stock pambase `20260616-1` with the exact edit |
| `packaging/pam/arch/system-auth.snippet` | the two lines plus the jump instructions |
| `packaging/pam/debian/soos`, `soos-notify` | `Default: no` |
| `run_tests.sh`, `.github/workflows/ci.yml` | `soos-sandbox-target` volume over `/workspace/target` |
| `Dockerfile`, `tests/docker/Dockerfile.{ubuntu,fedora,arch}` | base images pinned by digest; Arch image keeps the stock `system-auth` copy |
| `tests/docker/pam_rollback_test.sh`, `authselect_profile_test.sh` | pinned default images; D6 case |
| `tests/distro/arch_linux_test.sh` | Steps 4–6: mtree-verified stock stack, documented edit, byte equality, face/password/event/faillock assertions through `auth include system-auth` |
| `tests/docker/test_packages.sh` | non-root `.deb`/Arch builds, archive and installed ownership, unit/run layout, makepkg parity |
| `tests/invariants/src/packaging_ownership_contract.rs`, `tests/invariants/src/lib.rs` | POA1–POA10, strict priority (POA11) |
| `Docs/DISTRIBUTION_DEPLOYMENT.md` | §2 generic ordering, §3.2 `Default: no`, §5.1 both Arch paths, §5.2 exact edit and full file |
| `Docs/PACKAGING_AND_PROVISIONING.md` | `--build` user resolution, Arch template, `Default: no`, §7.2.2 ownership and layout, unit path |
| `Docs/CI_CD_AND_SECURITY.md` | job notes, workspace isolation and pinned images |
| `AI/DECISIONS.md` | four ADRs of 2026-10-02 |
| `AI/VERIFICATION_MATRIX.md` | POA section; PK2, PK7, DV3 evidence |

RPM and `.deb` were checked for the #301 defect class: `build_deb.sh` already used
`--root-owner-group`, and `rpmbuild` takes owners from `%files` (`%defattr`/`%attr`); both are now
asserted on the built archives by the Docker harness.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) run locally on this branch (§8). The Layer 2 sub-agent review
is run by the orchestrator before the push (not run here, as instructed).

## 8. Verification Results

```bash
export CARGO_BUILD_JOBS=5
cargo test --locked --all-features -p soos-invariants           # 355 passed, 0 failed
cargo fmt --all -- --check                                      # clean
cargo clippy --locked -p soos-invariants --all-targets --all-features -- -D warnings   # clean
bash -n <every changed script>                                  # clean
docker run --rm -v "$PWD:/mnt:ro" -w /mnt koalaman/shellcheck:stable -S warning <changed scripts>  # clean
bash tests/distro/run_distro_validation.sh arch                 # passed (Docker)
docker run ... soos-distro-val-arch bash /workspace/tests/docker/test_packages.sh   # passed
./run_tests.sh                                                  # T1–T15 passed; root-owned target/ entries: 0 before, 0 after
bash tests/docker/pam_rollback_test.sh                          # debian (D0–D6) and fedora passed
bash tests/docker/authselect_profile_test.sh                    # passed (pinned fedora:40 digest)
bash tests/distro/run_distro_validation.sh ubuntu               # passed (Docker)
docker run ... soos-distro-val-ubuntu bash /workspace/tests/docker/test_packages.sh # passed: .deb built by soosbuild, root:root entries, uid 0 installed paths, unit layout
./scripts/candid_review.sh                                      # Layer 1 PASSED
```

Arch harness highlights (2026-10-02): "Stock system-auth matches the pambase mtree digest
(/var/lib/pacman/local/pambase-20260616-1)", "Stock system-auth + documented edit ==
packaging/pam/arch/system-auth", face Allow on `test-swaylock` and `test-hyprlock`, "Valid password
with the daemon absent: faillock tally 0", "Valid password: accepted, no PasswordFailed event,
faillock tally 0", "Wrong password: rejected, exactly one PasswordFailed event, faillock tally 1".
Arch package harness: both archives `root:root`, "Unit under /usr/lib/systemd/system; no
/etc/systemd or /run entry", "PKGBUILD (makepkg) and scripts/build_arch.sh produce the same paths,
modes and owners", "Every installed soos path is owned by uid 0 (gid 0)".

The Fedora RPM branch of `test_packages.sh` and `run_distro_validation.sh fedora` were not run
locally (RPM recipe unchanged; its new ownership/layout assertions run in the CI job
`distro-deploy` on `main`).

## 9. Known Limitations / Follow-ups

- **One Arch packaging path** (ONB-NEW-3 preference): removing the PKGBUILD or building
  `build_arch.sh` through `makepkg` needs owner approval to change pinned invariants
  (`test_arch_packaging_specification`, `test_rpm_and_arch_recipes_ship_soos_gui`,
  `test_pkgbuild_builds_from_an_explicit_source_tree`, `test_package_license_matches_cargo_workspace`,
  `test_package_versions_derive_from_cargo_workspace`,
  `test_packaging_selects_one_distro_template_and_never_ships_snippet_in_pam_d`,
  `test_packaging_passes_explicit_distro_pam_dir`, `licence_contract::test_readme_and_packages_state_the_project_license`).
  Until then the two paths are proven equivalent in Docker.
- **Face match and faillock**: with `success=done` a face match does not run
  `pam_faillock.so authsucc`, so earlier password failures are not reset by a face login. Switching
  the Arch primary rule to `[success=4 default=ignore]` (lands on `pam_permit.so`) would change the
  primary-rule contract pinned by `test_pam_config_ordering_matches_spec`; owner decision.
- **TCI-NEW-3**: the `ort-sys` ONNX Runtime download is still neither retried nor cached across CI
  jobs; `tests/docker/Dockerfile.systemd` stays unpinned (its `FROM ubuntu:24.04` line is pinned by
  `systemd_unit_acceptance_contract`). Digests must be bumped deliberately.
- The RPM is still built by root in the harness (`rpmbuild` runs the full `%build`); its ownership
  comes from `%files` and is asserted on the archive.
