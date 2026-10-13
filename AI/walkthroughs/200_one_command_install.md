# Walkthrough 200 — One-Command Setup `scripts/setup.sh` (Core, Face Profile)

- **Date**: 2026-10-13
- **Issue**: GitHub-only #372 (PR 1 of #362), no backlog id — **Branch**: `feat/one-command-install` (not registered in `scripts/sync_issue.py`; the commit carries `Closes #372`)
- **Matrix criteria**: SET1–SET18 (verified), SET19 (owner hardware, pending)

## 1. Context & Objectives

Installing soos from source took about ten manual steps across several pages: package lists, the verified rustup bootstrap, the release build, `sudo ./scripts/install.sh`, starting the daemon, `soos-admin add-user`, the camera check, enrollment and the verify commands. #362 asks for one entry point that detects the host, plans, installs what is missing, is safe to re-run and can be rolled back. This PR is item 1 of its planned split: the core and the face profile.

Owner decisions applied: the name `scripts/setup.sh` (later `/usr/bin/soos-setup`); enrollment from a local seat only (otherwise skip and print the command); the default install enables everything (presence, phone companion, push, Funnel, camera view), each toggleable later in the GUI. That reverses the installer-path opt-in defaults and is recorded in an ADR. This PR applies only the face profile. Tailscale-account steps are guided, never silent. PAM activation keeps its safety checks.

## 2. Architect Design

Spec: `AI/architect_spec_one_command_install.md` (§1–§10, Revision 1 §11, Revision 2 §12).

- **Phases**:
  - detect: distribution (os-release parsed, never sourced), package manager, systemd, display manager, `tailscale version`, local seat, camera nodes, target and build user.
  - plan, then confirm (`--yes` without a terminal).
  - packages: `check_build_deps.sh --print-packages build|models|gui`, missing ones only.
  - toolchain: `install_rustup.sh`.
  - build: as the user, never root.
  - system: `install.sh --start --artifact-dir … --distro … --pam-dir …`.
  - access: `soos-admin add-user`.
  - presence: `--presence on|off`, otherwise the current state is kept.
  - verify: `wait_daemon_ready.sh` and `soos-admin camera list`.
  - enrollment: local seat, camera and no template. The daemon is stopped, `soos-enroll` runs, the daemon is started and checked again.
  - state: `/var/lib/soos/state/setup.json`.
  - summary.
- **Privilege flow**: at most two escalations (package manager, then `bash scripts/setup.sh --system-phase …`). Everything the root half needs is passed as an argument, because `sudo` resets the environment. A root start needs the invoking user or `--user`, and builds through `runuser`.
- **Constants**: `MAX_STATE_BYTES` 4096, `SETUP_STATE_FORMAT` 1, `READY_TIMEOUT_S` 30, `CAMERA_TIMEOUT_S` 15, `LOGINCTL_TIMEOUT_S` 5, `QUERY_TIMEOUT_S` 10, `MAX_USER_LEN` 32.
- **Exit codes**: 0, 1, 2, 3, 4, 40, 70, 129/130/131/143.
- **Invariants touched**: exit-70 semantics and the guarded first start (ADR 2026-09-30); verified rustup (ONB-16); never build as root (#308); presence default (ADR 2026-10-02); the installer never configures Tailscale (only the read-only `tailscale version` is carved out); PAM never touched.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`, independent sub-agent.

**Round 1: REVISION_REQUIRED.** 7 MAJOR and 11 MINOR findings:
- F1: enrollment would hit `CameraBusy`, because the daemon owns the camera.
- F2: the local-seat verdict was lost across `sudo`.
- F3: the sudo stub kept the environment, so it could not catch that.
- F4: the build user and the target user were conflated.
- F5: an "exit 2, nothing changed" refusal could happen after packages and the build had already run.
- F6: the "already done" check missed damage (models, key, group, helper, PAM template).
- F7: some #362 acceptance lines were neither covered nor deferred.

**Round 2: APPROVED** after Revision 1, with 9 MINOR refinements (m-A..m-I). They were applied in Revision 2:
- HUP/QUIT traps.
- `--yes` passed to `soos-enroll` only without a terminal.
- A root-start hint.
- Read/write checks on the target directory.
- Content comparison of the PAM templates.
- ADR text for the Tailscale carve-out, one user per state file, and the tmux/VNC residual.

## 4. Tester Contract

`AI/tester_contract_one_command_install.md`: 26 tests in `tests/invariants/src/setup_contract.rs` (28 after the candid review follow-up, §7). Red before the script existed: 0 passed, 26 failed (exit 127, or the read error / missing README text).

The harness:
- builds a fake checkout: the real `setup.sh`, `check_build_deps.sh`, provision helper, unit, Arch PAM snippet and manifest, plus stub `install.sh`, `install_rustup.sh`, `wait_daemon_ready.sh` and `download_models.sh`;
- uses a fake root via the path options;
- puts a restricted `PATH` in front: stubs, then symlinks to harmless executables only;
- logs every stub call with the emulated uid;
- uses a `sudo` stub that resets the environment with `env -i`;
- uses a `soos-enroll` stub that fails with "Camera is busy" while the stub unit is active.

No existing test was changed. Harness corrections made before Green are listed in the contract; none weakens an assertion.

## 5. Auditor Constraints

`AI/auditor_constraints_one_command_install.md` C-1..C-14, CLEARED. Main points:
- strict bash without `eval`;
- options validated first;
- the dry run writes nothing;
- never build as root;
- inputs reach the root half only as arguments;
- the daemon is restarted on every exit path;
- no PAM, Tailscale-write, remote or network-lock action;
- package names from one source only;
- bounded, strict, atomic state file;
- rollback limited to setup's own journal;
- no symlink is followed;
- `%s`/`%q` printing;
- bounded queries;
- ShellCheck clean.

Pre-existing and accepted: the root half runs a script of a user-owned checkout, the same trust model as `sudo ./scripts/install.sh`.

**Process note:** the developer draft was prepared in a scratch directory in parallel with the audit and placed in `scripts/` only after the round 2 approval. Every constraint was then verified on the placed script.

## 6. Implementation

Files:
- `scripts/setup.sh` (new, 0755);
- `tests/invariants/src/setup_contract.rs` (new) and its registration in `tests/invariants/src/lib.rs`;
- `README.md` (one command first, manual procedure kept);
- `Docs/PACKAGING_AND_PROVISIONING.md` §3.0;
- `AI/DECISIONS.md` (ADR);
- `AI/VERIFICATION_MATRIX.md` (component `one-command-setup`, SET1–SET19);
- spec, plan report, tester contract and auditor constraints in `AI/`;
- this walkthrough.

Notable decisions:
- The build uses an explicit `PATH=~/.cargo/bin:$PATH` instead of the login shell that `install.sh --build` uses: `install_rustup.sh` never edits profiles, so a login shell would not see a fresh toolchain.
- `setup.json` has no timestamp, so an identical run writes nothing.
- The family table that maps os-release tokens to package managers mirrors `check_build_deps.sh`. Package names stay single-sourced.
- An `install.sh` exit 70 stops setup before access (state written, exit 70).
- The Wi-Fi network lock is not installed; an opt-in hook is left to a later PR.

Not changed: every crate, `install.sh`, `install_remote.sh`, `uninstall.sh` (`--purge-data` already removes the state file; `--keep-data` keeps it), packaging, CI.

## 7. Candid Review

The first independent review of this diff (layer 2, run by the orchestrator) returned **CHANGES_REQUESTED** with three findings. Each was fixed tester-first; see spec §13 and the follow-up section of the tester contract.

1. **MAJOR: enrollment could run from SSH.** The fallback to the user's desk session ran whenever the shell's own session was not local, including when logind positively answered `Remote=yes` or reported no seat. Example: an operator over SSH runs `su - alice` while Alice sits at the desk.
   - Fix: `session_is_local` now returns local (0), not local (1) or unknown (2). Only an unknown own session may fall back to the `Display` session.
   - Tests:
     - New `test_set_remote_own_session_never_borrows_the_desk_session`.
     - The `loginctl` stub now answers per session id. This also turned the existing `test_set_remote_session_skips_enrollment_with_command` Red against the old script, without any change to its assertions.
2. **MINOR: the daemon stop marker was cleared before `systemctl start`.**
   - Fix: it is now cleared only after a successful start, so a signal in between still leaves the restart to the EXIT trap.
   - Test: new `test_set_daemon_stop_marker_cleared_only_after_restart`.
3. **MINOR: root's environment leaked into the user build.**
   - Fix: when setup is started as root, the build and `install_rustup.sh` run under `runuser … env -i` with an allow-list: `HOME`, `USER`, `LOGNAME`, `PATH` (with `~/.cargo/bin`), `LANG`, `LC_ALL`, `TERM`, and the proxy variables.
   - Test: `test_set_full_run_as_root_builds_as_invoking_user` was strengthened. It now sets root's `CARGO_HOME`, `RUSTUP_HOME` and `RUSTFLAGS` and asserts that none of them reach cargo.

Red evidence: 24 passed, 4 failed. After the fixes: 28 passed, 0 failed. A new layer-2 review is due on the new fingerprint (orchestrator).

## 8. Verification Results

- `cargo fmt --all -- --check`: pass.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: pass (the first run flagged `clippy::type_complexity` in the new test; fixed with a `StatePrep` type alias).
- `cargo test --locked --workspace --all-targets --all-features`: pass, 3538 passed, 0 failed, 6 ignored. `soos-invariants` alone: 617 passed, including the 26 `setup_contract` tests.
- `./scripts/candid_review.sh` (layer 1): PASSED. It was also run with the new files marked intent-to-add, so that `scripts/setup.sh` gets the syntax and English checks; the index was restored afterwards.
- ShellCheck 0.11.0 on `scripts/setup.sh`, in a network-less `koalaman/shellcheck:stable` container: clean at every severity (CI runs `--severity=error`). `bash -n` also passes.
- Flakiness of `cargo test --locked -p soos-invariants --all-features setup_contract`:
  - Repeated runs found 1 failure in 30. `tailscale version | head -n 1` under `pipefail` lost the version whenever `head` closed the pipe before the writer finished (SIGPIPE).
  - Fixed in the script: the whole output is read, then cut to its first line.
  - After the fix: 40 consecutive green runs.
- `./scripts/setup.sh --dry-run` on the development host (read-only): exit 0. It printed the detection (Arch, pacman, systemd, gdm, Tailscale 1.104.1, local seat, 2 V4L2 nodes) and the plan.

After the candid review follow-up, all gates were re-run on `origin/main` `6f32ef8`. The build used a private cargo target directory with `CARGO_BUILD_JOBS=4`, and tests ran with `--test-threads=4`.

- **Red recheck** of the reviewed script against the new tests: 24 passed, 4 failed.
- `cargo fmt --all -- --check`: pass.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: pass.
- **Stability loop:** `setup_contract` 30 times in a row; every run had 28 passed and 0 failed.
- `cargo test --locked --workspace --all-targets --all-features`: 3577 passed, 0 failed, 6 ignored. `soos-invariants` alone: 632 passed.
- **ShellCheck 0.11.0:** clean.
- `./scripts/candid_review.sh` (layer 1): PASSED, and its syntax check covered `scripts/setup.sh`.

`setup.sh` was never run for real; nothing was installed and `sudo` was never used.

## 9. Known Limitations / Follow-ups

- **Owner hardware row SET19**, on the Arch host from the desktop session:
  1. `./scripts/setup.sh --dry-run`, then `./scripts/setup.sh`: install, add-user, the daemon stops, enrollment, the daemon restarts and is healthy.
  2. Run it again: every phase is already done and nothing changes.
  3. Over SSH, enrollment is skipped and the command is printed.
  4. `sudo stat -c '%a %U' /var/lib/soos/state/setup.json` prints `600 root`.
- Residuals:
  - A remotely attached tmux pane that inherited a desk session, or a VNC-mirrored desk session, can look local.
  - Started as root, a custom `CARGO_HOME` is not seen.
  - The root half runs the checkout's own script; signed releases (`--from-release`) will close this gap.
- Later PRs of #362: the Docker idempotence and fault-injection matrix with `pam_snapshot.sh verify`; the user phase (companion, push, Funnel, camera view, guided Tailscale) with typed confirmations and consistency checks between options; PAM activation and `soos-admin pam`; `soos-daemon --check-config` and the TOML editor; `--uninstall`, `--profile`, `--format json` and `--network-lock`; `--from-release` and any signed bootstrap.
