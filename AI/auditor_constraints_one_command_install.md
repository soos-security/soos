# Audit Constraints — GitHub #372: `scripts/setup.sh` core (PR 1 of #362)

- **Date**: 2026-10-13 — **Branch**: `feat/one-command-install`
- **Inputs**: spec `AI/architect_spec_one_command_install.md` (§11 Revision 1, §12 Revision 2), plan evaluation round 2 APPROVED (`AI/plan_evaluator_report.md`), tester contract `AI/tester_contract_one_command_install.md`, `scripts/install.sh`, `scripts/check_build_deps.sh`, `scripts/wait_daemon_ready.sh`, `scripts/download_models.sh`, `scripts/install_rustup.sh`, `crates/enrollment-cli` (camera ownership, global path flags, `--yes`).
- **Scope**: one new Bash script and one invariant test module. No Rust production code, no crate, no dependency, no unit, no PAM file, no CI workflow change.

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| C-1 | `set -euo pipefail`, `umask 022`, `LC_ALL=C`; no `eval`; os-release parsed line by line, never sourced | `scripts/setup.sh` | SET1 (`test_set_s1_script_is_strict_bash_without_eval`), SET8 injection fixture |
| C-2 | Every option is validated before anything runs (absolute paths, user-name regex and length 32, enumerations); internal options refused outside `--system-phase`; usage errors exit 2 | argument parser | SET6 |
| C-3 | `--dry-run` performs no write, no escalation, no build, no package install, no temp file; only read-only queries (`pacman -Q`, `dpkg-query`, `rpm -q`, `systemctl is-*`, `loginctl show-*`, `tailscale version`, `download_models.sh --check-only`) | user half | SET7 (whole-tree snapshot of fake root, HOME, checkout and TMPDIR; log allow-list) |
| C-4 | The build never runs as root: current user, or `runuser -u <invoking user or checkout owner>`; a root build user is a preflight error; the target directory owner/read/write preflight mirrors `install.sh` | `resolve_build_user`, `check_target_dir_owner`, `as_build_user` | SET10/SET12 (`uid=1000 cargo`, no `uid=0 cargo`) |
| C-5 | Exactly one escalation for packages and one for the root half; nothing reaches the root half through the environment (artifact dir, user, seat verdict are arguments) | `as_root`, `SYSTEM_ARGS` | SET10 (two `sudo` lines, `--local-seat yes`), SET13 (SSH variable dropped by the `env -i` sudo stub, enrollment still skipped) |
| C-6 | Enrollment only with a local-seat verdict, a camera, no template; the daemon is stopped for it and started again on every exit path (EXIT trap + HUP/INT/QUIT/TERM traps); paths derive from `--localstatedir` | `phase_enrollment`, `on_exit_system`, `on_signal` | SET10 order stop→enroll→start→ready, SET16 restart after exit 3, SET13 |
| C-7 | No PAM change, no Tailscale write (only `tailscale version`), no remote companion, no `[preview]`, no network lock, no download by setup itself, no world/group-writable mode | whole script | SET3 static, `assert_no_forbidden_calls` in behavioral tests |
| C-8 | Package names come only from `check_build_deps.sh --print-packages`; installs only the missing ones; no repository change, no partial upgrade (`-Sy`) | `detect_packages`, packages phase | SET2, SET10 |
| C-9 | State file: bounded read (4096 bytes), regular, not a symlink, owned by the effective user, not group/world-writable, strict grammar, one user; atomic write (`mktemp` under `umask 077`, `chmod 0600`, `mv -f`), skipped when identical; directory `0700`; no secret, no timestamp | `read_state`, `write_state` | SET18, SET10 exact content, SET11 unchanged tree |
| C-10 | Rollback replays only setup's own journal (group, presence flag, `/etc/soos` dir); `install.sh` stays the authority for its files; committed installs are kept (exit 3/70); packages and toolchain never removed | `rollback_journal`, `fail_and_rollback`, `finish_with_state` | SET15, SET16 |
| C-11 | Presence flag created with `noclobber` (no symlink follow), `0644`; symlinked `/etc/soos` or flag refused; an existing flag is removed only with `--presence on` | `phase_presence`, `root_preflight` | SET18 |
| C-12 | Messages print variable content with `%s` (no `echo -e` interpretation); paths in preflight errors quoted with `%q`; os-release values stripped of non-printable characters and bounded to 80 characters | `info`/`warn`/`error`, `clean_value` | grep (`echo -e` absent), SET8 |
| C-13 | Every external query that can hang is bounded (`timeout --kill-after=2`): `loginctl` 5 s, `systemctl`, `getent`, `tailscale` 10 s, camera report 15 s, readiness 30 s | `run_bounded` callers | review; SET4 constants |
| C-14 | ShellCheck clean (CI runs `--severity=error`; the script is clean at every level) and `bash -n` | `scripts/setup.sh` | `shellcheck` 0.11 (`koalaman/shellcheck:stable`, no network), candid layer 1 |

## Pre-existing violations found (not introduced by this change)

- The root half executes a script of a user-owned checkout as root (`sudo bash scripts/setup.sh --system-phase`). This is the same trust model as the documented `sudo ./scripts/install.sh` and is unchanged; the signed-release path (`--from-release`, later PR) addresses it.
- `install.sh` prints with `echo -e` (escape interpretation of paths); not touched here.

## Clearance: CLEARED
