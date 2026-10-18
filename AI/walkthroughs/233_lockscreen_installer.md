# Walkthrough 233 — Installer Opt-In of the Lock-Screen Adapters

- **Date**: 2026-10-18
- **Issue**: GitHub #432 = PR 6 of epic #352 (GitHub-only, no backlog id) — **Branch**: `feat/lockscreen-installer`
- **Decision**: ADR 2026-10-18 "Installer Opt-In of the Lock-Screen Adapters and the Shipped Widget Build" (applies the owner decisions of 2026-10-12 on #352 and of 2026-10-12/13 on #362; amends the default-on profile of ADR 2026-10-13 "One-Command Setup `scripts/setup.sh` and Default-On Installer Profile")
- **Spec**: `AI/architect_spec_lockscreen_installer.md`
- **Matrix criteria**: LKI1–LKI10 (new, verified), LKI11 and LKI12 (owner hardware, pending)

## 1. Context & Objectives

PRs 2 and 4 of #352 delivered `soos-lockstatus`, `soos-lock-widget`, the hyprlock snippet and the GNOME Shell system extension `soos-lockscreen@soos`, but `install.sh`, `setup.sh`, `uninstall.sh` and the packages never mentioned them: every user had to copy them by hand. The epic asks for installer detection and opt-in, and #362 for an installer that turns everything on by default with a way to turn each feature off. KDE stays deferred (#421).

## 2. Architect Design

- **install.sh** installs the two programs as optional artifacts (present → 0755 in `<prefix>/bin`, absent → skipped with a notice, never a failure), copies the extension's three files (0644, directory 0755) and the snippet (0644), all through the existing journal (`tracked_install`, `ensure_dir`), so a late failure restores everything. A symbolic link in place of a target directory fails the run. Nothing is enabled.
- **uninstall.sh** removes exactly those files, never through a symbolic link, and keeps a directory that holds foreign files.
- **setup.sh** gains `--lock-widget on|off` (default on: the kill switch file is absent), a root phase `lock widget` mirroring `presence`, read-only locker detection, per-locker plan lines and a user step that runs `gnome-extensions enable soos-lockscreen@soos` only as the session owner from a local GNOME seat. swaylock-plugin and hyprlock get the exact line printed; no locker configuration is ever edited.
- **State file** stays format 1: the two lock-widget keys are written only away from the default.
- **Shipped widget build**: the workspace build every installer path already runs (`libwayland-client.so.0` loaded at run time); verified with `cargo tree -e features -i wayland-backend` (`client_system` and `dlopen` in the workspace, only `default` standalone) and `ldd` of swaylock-plugin on the owner's Arch host (links `libwayland-client.so.0`).

## 3. Tester Contract (Red)

`tests/invariants/src/lockscreen_installer_contract.rs` (registered in `tests/invariants/src/lib.rs`):

| Row | Test |
|---|---|
| LKI1 | `test_lki1_install_stages_the_lock_screen_adapters` |
| LKI2 | `test_lki2_adapters_are_optional_artifacts` |
| LKI3 | `test_lki3_late_failure_rolls_back_the_adapters` |
| LKI4 | `test_lki4_uninstall_removes_the_adapters_without_following_links` |
| LKI5 | `test_lki5_lock_widget_option_is_parsed_and_documented` |
| LKI6 | `test_lki6_lock_widget_flag_default_on_toggle_and_state` |
| LKI7 | `test_lki7_dry_run_detects_lockers_and_prints_exact_lines` |
| LKI8 | `test_lki8_gnome_enable_runs_as_the_user_after_the_system_phase` |
| LKI9 | `test_lki9_no_enable_off_seat_or_when_off_and_no_user_config_write` |
| LKI10 | `test_lki10_packages_variant_and_documentation` |

The setup tests run the real `setup.sh` in a fake checkout against a fake root with stub commands (`sudo` with `env -i`, `runuser`, `loginctl`, `systemctl`, `pacman`, `cargo`, `gnome-extensions`, and locker stubs that log if they are ever run) and a restricted `PATH`. Run against the scripts of origin/main, 9 of the 10 failed for the expected reasons (no adapter staged, unknown option, no detection lines, no packaging entries); LKI3 passed there vacuously (nothing touched the adapters) and stays as a regression guard of the rollback.

No existing test was modified.

## 4. Auditor Constraints

No `eval`; new functions errexit-safe; every user-step command bounded by `QUERY_TIMEOUT_S`; flags created with `set -C`, 0644, journaled, symbolic links refused; no write to `$HOME`, no `gsettings` or `dconf`; no locker executed; installed files root-owned and not group/other writable on a live install; no change to the PAM, daemon, protocol, lockstatus or lock-widget crates or to `packaging/gnome-shell/` (LKG7), no Tailscale command (SET3).

## 5. Implementation (Green)

- `scripts/install.sh`: `OPTIONAL_ARTIFACTS`, step 4b, plan lines, prefix warning, epilogue line 6.
- `scripts/uninstall.sh`: step 3b.
- `scripts/setup.sh`: option, paths, `lock_widget_report`, `detect_lockers`, `gnome_desktop`, `gnome_enable_blocker`, `print_lockscreen_plan`, `print_lockscreen_lines`, `run_gnome_enable_step`, `run_lockscreen_step`, `phase_lock_widget`, state keys, rollback entries, `system_up_to_date` comparing the adapter files when the build and checkout have them.
- `packaging/arch/PKGBUILD`, `packaging/rpm/soos.spec`: the same files (`build_deb.sh`, `debian/rules` and `build_arch.sh` stage through `install.sh`).

## 6. Documentation

`Docs/LOCKSCREEN_WIDGET.md` §3 (installer table, setup opt-in, shipped build, by-hand variant), §4, §5, §10, §11; `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 (phase table, state keys, rollback, lock screen paragraph), §3.2 and §5; ADR in `AI/DECISIONS.md` with cross-reference notes on the 2026-10-13, 2026-10-15 and 2026-10-17 entries; matrix component `lockscreen-installer`.

## 7. Verification

- `cargo test --locked -p soos-invariants -- --test-threads=3`: the whole invariant suite, the new contract included, passes (scoped target directory, `LIBONNXRUNTIME_NO_PKG_CONFIG=1`).
- `cargo fmt --check -p soos-invariants` and `cargo clippy --locked -p soos-invariants --all-targets -- -D warnings`: clean.
- `bash -n` of the three scripts and the PKGBUILD. `shellcheck` is not installed on this host: the CI lint job runs `shellcheck --severity=error`.
- Not run here: the Docker package matrix and the real hardware. LKI11 (Arch, driftwm, swaylock-plugin) and LKI12 (Ubuntu GNOME) are the owner's checks.
