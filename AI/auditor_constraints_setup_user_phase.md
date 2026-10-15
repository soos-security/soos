# Audit Constraints — GitHub #400: `scripts/setup.sh` user phase (PR 2 of #362)

- **Date**: 2026-10-15 — **Branch**: `feat/setup-user-phase`
- Inputs: spec §1–§11, tester contract, current `scripts/setup.sh`, `scripts/install_remote.sh`, `packaging/soos-remote.service`, `packaging/soos-push-sender.service`, `crates/remote/src/config.rs`, `crates/remote/src/main.rs`.

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| U-1 | The companion phase never runs as root: a root start skips it; with companion options it is exit 2 | `resolve_companion` | SET22, SET29 |
| U-2 | Only `--camera-view on` crosses the escalation; the root half resolves the `[preview]` uid itself (`TARGET_UID` from `--user`) and never reads or writes under the user's HOME | `SYSTEM_ARGS`, `--system-phase` checks, `phase_preview` | SET22 (internal option refusal), SET24 (escalation line without login), SET29 |
| U-3 | Only `tailscale version` is executed; `serve`/`funnel` appear only inside quoted messages; never `status`, `up`, `set`, `--http` | whole script | SET3 (migrated), SET20, `assert_no_forbidden_calls` in SET24 |
| U-4 | No secret printed or stored: the login is never printed (plan says "login given"), no enroll code, no VAPID key, no passkey data; `soos-remote passkeys list` output is discarded | `print_companion_plan`, `run_companion_phase`, `print_companion_guide` | SET23, SET24 (`!t.contains(LOGIN)`); review |
| U-5 | `remote.toml` is written 0600 through `mktemp` under `umask 077` and `mv -f` in the same directory; a new directory is 0700; symbolic links (config home, directory, file, backup) refused; an existing directory must be owned and not group/world-writable | `resolve_companion`, `write_remote_config` | SET24, SET31, SET38 |
| U-6 | A user-written `remote.toml` is never edited; the template is replaced only when it is exactly the untouched `install_remote.sh` template; a managed file is parsed with a strict grammar and a 16 KiB bound | `classify_remote_config` | SET30, SET26 |
| U-7 | The login and node name are validated against strict character sets before they reach TOML text (no quote, backslash or newline) | `valid_login`, `valid_rp_id` | SET22 |
| U-8 | `offline_alerts = true` is never written without `push_notifications = true`; no key is written on whose prerequisite is off | `resolve_features` | SET27, `setup_render_tests` |
| U-9 | An explicit `false` in a managed file is kept unless its `on` option is given; a unit not enabled on a re-run stays disabled (sender: unless push is switched on now) | `resolve_features`, `REMOTE_UNIT_KEPT_OFF`, `SENDER_UNIT_KEPT_OFF` | SET26, SET37 |
| U-10 | `daemon.toml` is created only when absent, 0644, through a hard link (never replacing a file), never through a symbolic link; an existing file is never edited; a creation is journaled and rolled back (file removed, daemon restarted) | `phase_preview`, `rollback_journal` | SET24, SET32, SET34 |
| U-11 | Every `systemctl` and `soos-remote` call is bounded (`run_bounded`, 10 s); the post-start settle is a fixed 2 s | `user_unit_query`, `companion_unit`, validation | SET20 (constants); review |
| U-12 | A companion failure rolls back exactly the run's companion changes (journal: created file, replaced file from its 0600 backup, units it enabled — journaled before `enable --now` — units it started from inactive, units it restarted — restarted again —, and a configuration directory it created) and exits 5; HUP/INT/QUIT/TERM roll back too | `rollback_companion`, `companion_failed`, `on_companion_signal` | SET33 |
| U-13 | No `eval`, os-release never sourced, no package-name word, no `$UID`/`$EUID` | whole script | SET1, SET2, SET3 |
| U-14 | `install_remote.sh` keeps its root refusal, never enables a unit, never runs `sudo`/`tailscale`; only the copy source changes | `scripts/install_remote.sh` | RMC-S6, RSH13, SET35 |
| U-15 | The journal group and linger are never changed: `usermod` and `enable-linger` only printed | `print_companion_guide` | SET20, SET24 |

### Pre-existing violations found

- `scripts/install_remote.sh` copied from a hard-coded `<checkout>/target/release` while cargo honours `CARGO_TARGET_DIR` (fixed here, OD-U5).
- Note (not a violation): `run_companion_phase` is called in a `||` context like the PR 1 phases, so `set -e` does not apply inside it; every step checks its status explicitly and returns through `companion_failed`.

### Clearance: CLEARED
