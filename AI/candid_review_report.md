# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/lockscreen-installer`
- **Base (merge-base)**: `3f76090`
- **Reviewed-Diff-Fingerprint**: `031d1be4746799b46e61ea636ab96a5f1c22a8b35250e9f9617aeba7a164c214`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_lockscreen_installer.md`, `AI/walkthroughs/233_lockscreen_installer.md`, `Docs/LOCKSCREEN_WIDGET.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `packaging/arch/PKGBUILD`, `packaging/rpm/soos.spec`, `scripts/install.sh`, `scripts/setup.sh`, `scripts/uninstall.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/lockscreen_installer_contract.rs`

## 1. Executive Summary

The change (GitHub #432, PR 6 of #352) makes `install.sh` stage `soos-lockstatus` and `soos-lock-widget` as optional artifacts plus the GNOME Shell system extension and the hyprlock snippet, all through the existing journal (`tracked_install`, `ensure_dir`); `uninstall.sh` removes exactly those files without following a symbolic link; `setup.sh` gains `--lock-widget on|off` (kill switch `<sysconfdir>/soos/lockwidget.disable`, journaled and rolled back like `presence.disable`), read-only locker detection (`command -v` only), plan lines, and a user-half step that runs `gnome-extensions list --enabled` / `enable` bounded by `QUERY_TIMEOUT_S`, only when not root, from a local seat with a GNOME desktop. The PKGBUILD and RPM spec install and list the same files; Debian stages through `install.sh`. No change to PAM, the daemon, the protocol, `crates/lockstatus`, `crates/lock-widget` or `packaging/gnome-shell/`. No CRITICAL or MAJOR finding; three MINOR wording/robustness items and two suggestions.

Re-run evidence (this review, scoped, with the mandated environment): `cargo test -p soos-invariants -- --test-threads=3` → 797 passed, 0 failed (LKI1–LKI10 all ok); `cargo clippy -p soos-invariants --all-targets -- -D warnings` → clean; `bash -n` on the three scripts → ok (shellcheck is not installed on this host).

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `tests/invariants/src/lib.rs` (12 added lines: doc comment plus `#[cfg(all(test, unix))] mod lockscreen_installer_contract;`), `tests/invariants/src/lockscreen_installer_contract.rs` (new file, 1088 lines).
- Removed/changed checks (`^-` lines with `assert`, `#[test]`, `proptest!`, `should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` changes: **none**.
- `git diff 3f76090 HEAD -- tests 'crates/*/tests' android/app/src/test`: only the two files above, additions only. No pre-existing assertion removed or changed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Optional artifacts never enter `MISSING_ARTIFACTS`, so `--allow-missing` and the INCOMPLETE banner are untouched; lookup uses the same `-f && -s` rule as required artifacts and the already declared `ARTIFACT_PATHS` associative array (`install.sh:623`). PASS.
- Step 4b runs in Phase C after the journal arrays exist and before models, so the model-failure rollback covers it (LKI3 exercises it). The dry run exits at `install.sh:686`, before any write. PASS.
- `ensure_dir ... 0755 owned` re-modes only the extension leaf; parents are system directories. PASS.
- `setup.sh`: `lock_widget_will_be_off` honours explicit on/off before the flag state; plan for `on` with an existing flag shows "will run remove"; user half reads the flag through a 0755 `/etc/soos` (`ensure_soos_conf_dir`), so a freshly added soos-group member still sees it. `print_summary` prints locker lines only for a root start (`SYSTEM_PHASE != true`), and the user half prints them via `run_lockscreen_step`, so lines are printed exactly once per flow. PASS.
- State file: new keys only away from default, placed before `"enrolled"`; `read_state` accepts exactly `"off"` / `true` and rejects anything else; SET10 default content unchanged (existing contract green). PASS.
- `system_up_to_date` compares adapter files only when their sources exist, so an upgrade from a pre-adapter install triggers a reinstall. PASS.
- Verified doc claim: `cargo tree --offline -e features -i wayland-backend --workspace` shows `client_system`, `default`, `dlopen`; `-p soos-lock-widget` shows only `default`. Verified: every installer path runs `cargo build --release --locked --workspace` (`install.sh:454`, `setup.sh:2656`, PKGBUILD, debian/rules, rpm spec, `build_*.sh`), and `build_deb.sh`/`build_arch.sh`/`debian/rules` stage through `install.sh`. Verified: a Wayland connect failure maps to `Exit::Setup` → one stderr line (`crates/lock-widget/src/wayland.rs:190`, `main.rs` `fail`). Verified on this host: `swaylock-plugin` links `libwayland-client.so.0`. Verified: the extension trusts only `/usr/bin` and `/usr/local/bin` and reads `/etc/soos/lockwidget.disable` (`extension.js:27-28`).
- FINDING (MINOR): kill-switch path mismatch with a non-default `--sysconfdir` (see §4).

### PAM Concurrency & Deadlines
- No file under `crates/pam` changed; no PAM stack edit; no new blocking call in any PAM path. PASS (not applicable).

### Panic Safety & Fail-Closed
- No Rust production code changed. Shell paths: new setup functions are called in `if`/`!` contexts or end with an explicit `return 0`; the user step never changes `rc`; a failing `gnome-extensions` only prints the command. `phase_lock_widget` failure → `fail_and_rollback "lock widget"`. No path turns an error into authentication success (PAM is not touched). PASS.

### Test Integrity & Anti-Weakening
- Tests run the real `install.sh`, `uninstall.sh` and `setup.sh` against stages and stub commands; locker stubs log if executed, and LKI9 asserts no write to the fake home and no `gsettings`/`dconf` in `setup.sh`. A plausible wrong implementation (enabling as root, running a locker, writing `hyprlock.conf`, making adapters mandatory, not journaling) would fail at least one of LKI1–LKI9. LKI3 is documented as a regression guard that passes vacuously on origin/main. No existing test modified. PASS.

### Memory, Bounds & Secrets
- `gnome-extensions` calls go through `run_bounded "${QUERY_TIMEOUT_S}"`; its captured output is truncated to 64 KiB before matching (the capture itself is not byte-bounded, acceptable for a user-owned, small-output tool). Flags created with `set -C` (no clobber, no symlink follow), 0644, symlinked flags refused in `root_preflight` and `phase_lock_widget`. `install.sh` refuses a symlinked target directory; `uninstall.sh` never follows a symlinked extension directory and keeps foreign files. No secret, frame or embedding involved; messages print paths only. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or hook change. Packaging changes only add `install` lines and `%files` entries for files produced by the existing build. PASS.

### English-Only Policy
- All added code comments, messages, docs, walkthrough and commit subject are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/setup.sh:660` / `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 — `LOCK_WIDGET_FLAG` follows `--sysconfdir`, but `packaging/gnome-shell/soos-lockscreen@soos/extension.js:28` hard-codes `/etc/soos/lockwidget.disable`; with `--sysconfdir` other than `/etc`, `--lock-widget off` creates a file the extension never reads while the help says the GNOME card is off. Suggested correction: warn when `SYSCONFDIR != /etc` (as install.sh already does for `--prefix`), or document the restriction.
- **[MINOR]** `Docs/LOCKSCREEN_WIDGET.md` §3 ("nothing enabled or printed") and `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 ("nothing is enabled or printed") — with the widget off setup still prints the off notice (`print_lockscreen_lines`, `setup.sh:797`) and the plan prints a skipped line. The ADR wording ("prints no locker line") is the accurate one; align the two docs.
- **[MINOR]** `scripts/setup.sh:831-833` — if `gnome-extensions enable` succeeds by updating the user's `enabled-extensions` setting before GNOME Shell has scanned the new system extension, setup reports "the card shows from the next lock" although a log-out may still be required. Covered by the pending owner row LKI12; consider adding "(log out and back in if it does not)" to the DONE line.
- **[SUGGESTION]** `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 — the sentence "so a later `--uninstall` can reverse exactly what setup did" (pre-existing for the two older keys, now extended to `lock_widget_flag_created_by_setup`) refers to a `setup.sh --uninstall` that does not exist yet; mark it as future work.
- **[SUGGESTION]** `scripts/uninstall.sh:1088` — the snippet removal checks only that `share/soos/lockscreen` is not a link; `share/soos` itself is not checked (rm -f of one named file, low impact).

## 5. Final Verdict

No CRITICAL or MAJOR finding; no pre-existing test assertion removed or changed; scoped tests and clippy green.

**VERDICT: APPROVED**
