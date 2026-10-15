# Plan Evaluation Report
- **Date**: 2026-10-13
- **Issue**: GitHub-only #372 — `scripts/setup.sh` core (PR 1 of #362, planned split item 1); no `AI/BACKLOG.md` id
- **Branch**: `feat/one-command-install`
- **Base commit**: `37d03c4`
- **Plan under evaluation**: `AI/architect_spec_one_command_install.md` with the normative §11 "Revision 1" (R-1..R-7, m1..m11)
- **Round**: 2 (round 1: REVISION_REQUIRED, 7 MAJOR, 11 MINOR)
- **Note**: the untracked draft `tests/invariants/src/setup_contract.rs` predates Revision 1. It is revised in Phase 2 and is not assessed here as a plan defect.

## 0. Round 1 findings and their resolution

| Round 1 finding | Revision | Verified against code | Status |
|---|---|---|---|
| F1 (MAJOR) enrollment vs the daemon's camera | R-1: bounded `systemctl stop` → `soos-enroll --biometrics-dir/--key-file/--models-dir ... enroll --username` → `systemctl start` → readiness; `DAEMON_STOPPED` + EXIT trap restarts on every path; failed restart/readiness = 70; busy stub test hook | Global flags exist (`crates/enrollment-cli/src/args.rs:28-45`, `global = true`, so placing them before `enroll` is valid); the stop/enroll/start sequence is the documented remedy (`error.rs:52-58`, `Docs/ENROLLMENT_CLI.md:227`) | RESOLVED (residual m-B, m-C) |
| F2 (MAJOR) local seat lost across sudo | R-2: decided in the user half and passed as `--local-seat yes\|no`; SSH vars → no; `XDG_SESSION_ID`/`self`, then `show-user --property=Display` fallback; root-started: no Display fallback, failure → not local | Logic is fail-safe in every root-started case | RESOLVED (residual m-A, m-D) |
| F3 (MAJOR) sudo stub keeps the environment | R-3: `env -i` allowlist stub; argv assertions; an SSH variable set for the user half must still skip enrollment | — | RESOLVED |
| F4 (MAJOR) build user vs target user | R-4: build user = current user, or invoking user, or non-root checkout owner, else exit 2; target user separate; `find -H ! -uid` preflight; explicit `PATH`/`HOME` instead of a login shell (deliberate, justified by ADR 2026-10-01 `--no-modify-path`); `runuser` preflight | `install.sh:521-583` checks owner **and** read/write permissions; R-4 checks owner only | RESOLVED (residual m-E, m-F) |
| F5 (MAJOR) exit 2 after changes | R-5: root-started preflight validates root-only items (exit 2); user-started root half refuses with new **exit 4**; 130/143 documented | — | RESOLVED |
| F6 (MAJOR) incomplete "already done" | R-6: provision helper equal, `master.key` present, `soos` group, family PAM template, `download_models.sh --check-only`; `--reinstall`; hint after a 70 on a skipped system phase | Paths match `install.sh`: Debian `${DESTDIR}/usr/share/pam-configs` (`install.sh:899`), Fedora `${SYSCONFDIR}/authselect/custom/soos` (`:907`), Arch `${PREFIX}/share/soos/pam` (`:936`); `--check-only` is read-only (`download_models.sh:355,373,421`: no `mkdir`/`chmod`, no `curl`) | RESOLVED (residual m-G) |
| F7 (MAJOR) unmapped #362 lines | R-7: display manager from the `display-manager.service` link, bounded `tailscale version`, static ban narrowed to "no subcommand but `version`", SET3 as the defaults invariant, write-free snapshot instead of `strace` (the acceptance line allows "a write-free stage"), CI scanners plus an exact state-content assertion, explicit deferral list | — | RESOLVED (residual m-H) |
| m1 internal options undocumented | `--artifact-dir`, `--local-seat`, `--system-phase` internal in `--help`/§2; absolute re-exec | — | RESOLVED |
| m2 family / pam dir divergence | always `--distro <family>`; `--pam-dir` resolved with `install.sh`'s rule and passed | candidate list `install.sh:287-303` | RESOLVED |
| m3 INT and state-write semantics | INT/TERM while armed rolls back; state-write failure keeps changes, exit 1 | — | RESOLVED (see m-B for SIGHUP) |
| m4 state-file owner / user change | `-O` owner check; a different stored user is refused with "remove it to start over" | — | RESOLVED (residual m-I) |
| m5 presence lstat, global flag | `-e \|\| -L`; `${SYSCONFDIR}/soos/disabled` reported | matches `presence/switch.rs:20-33` | RESOLVED |
| m6 distro cargo without rustup | warning + recommendation | — | RESOLVED |
| m7 apt env, refresh hint, globbing | `sudo -- env DEBIAN_FRONTEND=...`; hints printed, never run; `read -r -a` | — | RESOLVED |
| m8 dry run | no TTY needed; provisional build/system lines; snapshot covers fake root, HOME, checkout, TMPDIR | — | RESOLVED |
| m9 bounds, `$EUID` | `systemctl`/`loginctl` bounded; `$EUID`/`$UID` banned | `getent` still unbounded (low risk, local NSS) | RESOLVED |
| m10 exit 70 continuation | access, presence, verify, enrollment skipped; state written | — | RESOLVED |
| m11 draft contract / SET2 | revised in Phase 2; SET2 bans every package name as a whole shell word | — | RESOLVED |

## 1. Coverage Matrix

| Acceptance line / TDD test | Spec element | Status |
|---|---|---|
| `--help` documents every flag (internal ones marked) | §2 + §11 m1 | COVERED |
| `--dry-run` no write, full plan, exit 0 | §4.11, §11 m8 (write-free whole-tree snapshot) | COVERED |
| os-release parsed, never sourced | §4.0, §11 m7 | COVERED |
| Package lists only from `check_build_deps.sh` | §4.0, §11 m11 | COVERED |
| Reports distribution, package manager, display manager, camera summary, Tailscale version | §4.0 + R-7 | COVERED |
| Defaults: core only (owner OD-3 amendment through ADR), no PAM/Tailscale write/remote/`[preview]`/network-lock action | §7, R-7 (SET3) | COVERED |
| Typed confirmations, inconsistent opt-in combinations, user phase never root | R-7 deferral list (PR 4) | DEFERRED (recorded) |
| Idempotence (second run `already done`) | §4.x + R-6 | COVERED locally; Docker matrix DEFERRED (recorded) |
| Rollback | §6 + m3 + R-1 trap | COVERED locally; fault-injection matrix and `pam_snapshot.sh verify` DEFERRED (recorded) |
| PAM activation, Arch `soos-admin pam`, `--check-config`, TOML editor, `--tailscale-serve`, `--from-release` | R-7 deferral list | DEFERRED (recorded) |
| No secret printed or stored | §5 + R-7 (exact state-content assertion, CI scanners) | COVERED |
| README leads with the one command | §1 | COVERED |
| ADR, matrix rows (incl. owner hardware run), invariants, walkthrough | §1, §10 | COVERED |
| OD-2 local-seat-only enrollment | R-1, R-2 | COVERED |
| ADR 2026-10-02 #308 never build as root | R-4 | COVERED |

## 2. Facts Verified Against Code (round 2 additions; round 1 facts unchanged)

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `soos-enroll` path flags are global | `crates/enrollment-cli/src/args.rs:28-45` | `--biometrics-dir`, `--key-file`, `--models-dir`, `global = true` | Yes |
| `soos-enroll` confirmation | `crates/enrollment-cli/src/main.rs:24-31,131`, `service.rs:871` | stdin `[y/N]`; EOF → declined; `--yes` skips the prompt | See m-C |
| `download_models.sh --check-only` read-only | `scripts/download_models.sh:355,373,421` | no target-dir creation, no curl requirement, missing file → exit 1 | Yes |
| PAM template locations | `scripts/install.sh:897-940` | Debian `${DESTDIR}/usr/share/pam-configs/{soos,soos-notify}`; Fedora `${SYSCONFDIR}/authselect/custom/soos/*`; Arch `${PREFIX}/share/soos/pam/system-auth.snippet` | Yes |
| Provision helper install path | `scripts/install.sh:858` | `${PREFIX}/libexec/soos/provision-master-key` from `scripts/provision_master_key.sh` | Yes |
| `install.sh` target-dir check | `scripts/install.sh:521-583` | owner, then read, then write checks | R-4 keeps the owner check only (m-E) |
| "installer never runs `tailscale`" wording | `AI/DECISIONS.md:130` (and agent rule at `:125`) | about `install_remote.sh`; agents never run `tailscale` | Read-only `tailscale version` needs an explicit carve-out in the new ADR (m-H) |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Scenario: the enrollment hand-off stops the daemon. During that window `pam_soos.so` cannot connect → `PAM_IGNORE` → password; presence unlock is off. Exclusive camera ownership is respected because the daemon is stopped before `soos-enroll` opens the device. PASS.
- Scenario (false local seat): the owner attaches over SSH to a tmux server started at the desk. The existing pane has no `SSH_*`, and `XDG_SESSION_ID` is the desk session (`Remote=no`, `seat0`). Setup decides "local", stops the daemon and runs enrollment while the owner is remote. Whoever sits at the PC is captured, and the owner confirms a summary without seeing the face. The scenario is contrived and needs the owner's cooperation, but the summary should make the remote risk explicit. MINOR m-A.
- Scenario: `tailscale version` runs from `PATH` (as root when root-started); it is read-only and does not touch Tailscale state. PASS, ADR wording m-H.

### Pillar 2 — PAM deadline & concurrency
- No PAM code or file change. The daemon-down window is bounded by the enrollment duration plus a bounded stop/start and the 30 s readiness wait. PASS.
- Scenario: the terminal closes during enrollment (SIGHUP to the root half through sudo). R-1 and m3 name EXIT, INT and TERM; HUP is not named. If HUP is not trapped explicitly, the restart guarantee rests on bash behaviour that is not stated in the plan. The unit stays enabled, so the next boot recovers, but the session would run without the daemon until then. MINOR m-B.

### Pillar 3 — Panic safety & fail-closed
- Every new path fails toward "skip / keep / not local": seat detection failure → not local; restart failure → 70; root-half refusal → 4; build-as-root → 2. No path turns an error into 0. PASS.
- Scenario: `sudo ./scripts/setup.sh` from a GNOME terminal (a likely habit). sudo strips `XDG_SESSION_ID`/`SSH_*`, and `self` fails in an app scope, so enrollment is always skipped. This is fail-safe, but the summary should say "run setup.sh without sudo to enroll from this seat". MINOR m-D.

### Pillar 4 — Dependencies
- No new crate or package list. Scenario: root-started run for a user whose rustup lives in a custom `CARGO_HOME`. The probe only knows `~/.cargo/bin` (sudo reset the environment), so `install_rustup.sh` installs a second toolchain into `~/.cargo`. The result is visible and recoverable, but it is a surprise write into the user's home. MINOR m-F.

### Pillar 5 — Data confidentiality
- State-file content is asserted exactly and holds no secret field. It must be owned by the effective user. A user change is refused. PASS.
- Scenario: a second household member must delete `setup.json` to be set up, which drops the sticky provenance of the first user's run (later `--uninstall` PR). Acceptable for PR 1 if the ADR says so. MINOR m-I.

### Pillar 6 — Test integrity & power
- R-3 makes environment reliance detectable. The busy `soos-enroll` stub makes a wrong enrollment ordering fail. Argv assertions cover the build user, artifact dir and seat verdict. PASS.
- Scenario: an implementation that skips the restart on the exit-3 path must fail a test. R-1 lists the hook but not the assertion that the stub unit is active again after exit 3, INT and a failed state write. The tester must cover each exit path. MINOR m-C (test plan precision).
- Scenario: an implementation that checks only the presence of the family PAM template (not its content) passes R-6 tests; R-6 says "equal" only for the provision helper. MINOR m-G.

## 4. Findings (remaining)

- **[MINOR] m-A** — The display-session fallback and inherited `XDG_SESSION_ID` can classify a remotely attached tmux/screen pane or a VNC-mirrored desk session as local. Required: state this limitation in the ADR/Docs. Recommended: show "a face will be captured at this PC's camera now" in the plan and require the TTY confirmation for enrollment.
- **[MINOR] m-B** — Name `HUP` (and `QUIT`) with INT/TERM in the trap set that guarantees the daemon restart and the journal replay. Do not rely on unstated bash EXIT-trap behaviour for signals.
- **[MINOR] m-C** — Spell out how setup answers `soos-enroll`'s `[y/N]` prompt. With a TTY it stays interactive; without one, `--yes` is passed to `soos-enroll`, or enrollment is skipped with the printed command (EOF currently aborts as exit 3). The tester must assert the unit restart after exit 0, exit 3, INT and a state-write failure.
- **[MINOR] m-D** — When setup is root-started and the seat is not local, the summary should recommend re-running without `sudo` from the desk session.
- **[MINOR] m-E** — The R-4 target-directory preflight checks ownership only. `install.sh` also checks read and write permissions (`install.sh:556-572`). Mirror all three checks, or record the narrower rule (cargo then fails late with exit 40).
- **[MINOR] m-F** — In the root-started toolchain probe, document the custom `CARGO_HOME`/`RUSTUP_HOME` limitation, or probe `runuser -u <build user> -- sh -c 'command -v cargo'` with the user's `HOME` before installing a second toolchain.
- **[MINOR] m-G** — R-6: compare the family PAM template files by content (`cmp -s` against `packaging/pam/...`), not only by presence, so a template change after `git pull` triggers the system phase.
- **[MINOR] m-H** — The new ADR must carve out the read-only `tailscale version` call explicitly, next to the remote ADRs' "installer never runs `tailscale`" wording (`AI/DECISIONS.md:130`) and the agent rule (`:125`). The tests must assert that it is the only `tailscale` argv.
- **[MINOR] m-I** — The ADR must record that `setup.json` is single-user in format 1, and that a second user requires removing it (provenance of the first run is lost).

## 5. Verdict
VALIDATION_VERDICT: APPROVED

All seven round 1 MAJOR findings and all eleven MINOR ones are resolved by §11 Revision 1 and verified against the code. Nine MINOR refinements remain (m-A..m-I). They do not block Phase 2; the tester and the developer must fold them in (m-B, m-C, m-E, m-G in the script and contract; m-A, m-D, m-F, m-H, m-I in the ADR and Docs).
