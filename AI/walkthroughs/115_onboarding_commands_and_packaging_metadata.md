# Walkthrough 115 — Onboarding Commands and Packaging Metadata

- **Date**: 2026-09-30
- **Issues**: GitHub #207 (ONB-11, onboarding docs give wrong commands; README has no install
  section), GitHub #210 (ONB-14, inconsistent packaging metadata)
- **Branch**: `chore/p2-onboarding-packaging-metadata`
- **Matrix criteria**: OPM1–OPM10 (new component `onboarding-packaging-metadata`)
- **ADR**: 2026-09-30 "Package Metadata Derived From `[workspace.package]`"
- **Scope**: README, installer epilogue, packaging recipes and build scripts, one new shell helper,
  one new invariant module. No Rust production code and no pre-existing test changed.

---

## 1. Findings Still Open on `origin/main`

Earlier passes had already fixed most of ONB-11 (`Docs/DISTRIBUTION_DEPLOYMENT.md` uses
`soos-enroll enroll --username` and `<uid>.cbor.enc`; `models/README.md` states BGR and live
class 1; README has an installation section). What remained:

| Finding | Location | State before |
|---|---|---|
| ONB-11 | `scripts/install.sh` epilogue | `sudo soos-enroll <username>` (no subcommand, rejected by clap); `soos-admin add-user` without `sudo` (it edits `/etc/group`); no pointer to the explicit PAM activation step |
| ONB-11 | `README.md` | install only: no enrollment, verification, PAM activation, rescue or uninstall steps |
| ONB-11 | tests | no doc-consistency invariant |
| ONB-14 | `soos.spec`, `PKGBUILD`, `build_arch.sh` | license `Apache-2.0 OR MIT` / `('Apache-2.0' 'MIT')` although `Cargo.toml` says `AGPL-3.0-or-later` |
| ONB-14 | `build_deb.sh`, `build_rpm.sh`, `build_arch.sh` | `VERSION="0.1.0"` hardcoded |
| ONB-14 | `soos.spec`, `PKGBUILD` | `soos-gui` not installed (the `.deb` ships it) |
| ONB-14 | `debian/rules`, `soos.spec`, `PKGBUILD`, `build_*.sh` | `cargo build` / `cargo test` without `--locked` |
| ONB-14 | `PKGBUILD` | `source=()` and `build()` ran `cargo` in the empty `$srcdir` |

## 2. Red Evidence

`tests/invariants/src/onboarding_packaging_contract.rs` was added first. On the unchanged tree,
`cargo test --locked -p soos-invariants --all-features onboarding_packaging` failed 10/10:

| Test | Failure before the change |
|---|---|
| `test_onboarding_cli_invocations_name_real_subcommands` | `scripts/install.sh:702: <username> is not a subcommand (expected one of {"debug-vision", "delete", "enroll", "import", "list", "verify"})` |
| `test_onboarding_docs_have_no_placeholder_enroll_or_bin_templates` | `scripts/install.sh:702: soos-enroll <...> without a subcommand` |
| `test_install_epilogue_prints_real_enrollment_command` | epilogue lacks `sudo soos-enroll enroll --username <username>` |
| `test_readme_has_quick_start_through_uninstall` | no `## Quick Start` section |
| `test_pkg_meta_reads_workspace_package_metadata` | `scripts/lib/pkg_meta.sh` does not exist |
| `test_package_license_matches_cargo_workspace` | spec `Apache-2.0 OR MIT` != `AGPL-3.0-or-later` |
| `test_package_versions_derive_from_cargo_workspace` | build scripts do not source `pkg_meta.sh` |
| `test_rpm_and_arch_recipes_ship_soos_gui` | spec `%install` has no `soos-gui` |
| `test_packaging_cargo_invocations_are_locked` | 6 unlocked lines (rules x2, spec, PKGBUILD, build_deb/rpm/arch/packages) |
| `test_pkgbuild_builds_from_an_explicit_source_tree` | `build()` has no `cd` into a source tree |

The CLI scan derives the subcommand set from `pub enum Commands` of
`crates/enrollment-cli/src/args.rs` and `crates/admin-cli/src/args.rs` (kebab-case variants) and
skips global flags, treating `bool` fields of `Cli` as switches and every other flag as taking a
value. It guards against a vacuous pass (at least 10 invocations must be found).

## 3. Change

- `scripts/install.sh`: epilogue now prints `sudo soos-admin add-user <username>`,
  `sudo soos-enroll enroll --username <username>`, start, `soos-admin status && soos-admin test-pam`
  and the explicit PAM activation pointer (`Docs/DISTRIBUTION_DEPLOYMENT.md`).
- `README.md`: `## Quick Start` with five steps (install, start and enroll, verify, explicit PAM
  activation per distribution, rescue with `/etc/soos/disabled` / `gdm.disable` and uninstall).
- `scripts/lib/pkg_meta.sh` (new): static parser of `[workspace.package]`; `soos_pkg_version`,
  `soos_pkg_license`; fails closed; executable form `pkg_meta.sh [--manifest <path>] <version|license>`.
- `scripts/build_{deb,rpm,arch}.sh`: source the helper, print `Version:` and `License:`, `--locked`
  build; `.PKGINFO` license from `${PKG_LICENSE}`; `build_rpm.sh` refuses a spec whose
  `Version:` / `License:` drift from `Cargo.toml`. `scripts/build_packages.sh`: `--locked`.
- `packaging/rpm/soos.spec`: `License: AGPL-3.0-or-later`, `Recommends:` GUI runtime libraries,
  `--locked` build, `soos-gui` in `%install` and `%files`.
- `packaging/arch/PKGBUILD`: `license=('AGPL-3.0-or-later')`, `optdepends` GUI libraries,
  `_soos_srcdir` (`SOOS_SRC_DIR`, default `${startdir}/../..`) entered by `build()` / `package()`,
  `--locked` build, `soos-gui` installed.
- `packaging/debian/rules`: `--locked` for `cargo build` and `cargo test --no-run`.
- Package descriptions (`soos.spec`, `debian/control`, `build_deb.sh`) no longer claim
  "sub-250ms" latency (the daemon budget is `DECISION_BUDGET_MS` = 900 ms; ADR 2026-09-30 on the
  PAM deadline).
- `Docs/PACKAGING_AND_PROVISIONING.md` §7.2.1: metadata source of truth.

## 4. Audit

Shell only: no Rust production code, no PAM, FFI, IPC or socket change. `pkg_meta.sh` reads one
local file, rejects values outside `[A-Za-z0-9.+_ -]` before they reach package metadata, and
never executes manifest content. No secret, frame or embedding is touched.

## 5. Gate

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast`, `./scripts/candid_review.sh`
and `bash -n` on every touched script (see the branch report for results).

## 6. Follow-ups

- Publish a release tarball (tag `v<version>`) and give the PKGBUILD a real `source=()` with a
  checksum so `makepkg` works outside a checkout.
- The Debian source package (`packaging/debian/`) has no `changelog` or `copyright` file; the
  AGPL license is therefore not declared in Debian metadata yet.
- Docker validation of the RPM / Arch packages with `soos-gui` runs in the `distro-deploy` CI job
  after merge (not run locally).
