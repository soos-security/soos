# Architect Spec — GitHub #400: `scripts/setup.sh` user phase (phone companion profile, PR 2 of #362)

- **Date**: 2026-10-15
- **Issue**: GitHub-only #400 (part of #362; follows #372 = PR 1); no `AI/BACKLOG.md` id.
- **Branch**: `feat/setup-user-phase` (not registered in `scripts/sync_issue.py`; the commit carries `Closes #400`).
- **Base**: `origin/main` 2025738 plus the owner-approved contract migration 40b0fb1 (SET3).
- **Owner decisions** (#362 comments of 2026-10-12/13, #400 comment of 2026-10-15):
  - OD-U1 SET3 contract migration approved: `install_remote.sh`, `remote.toml` and `[preview]` may appear in `setup.sh`; only `tailscale version` may run; `serve`/`funnel` are printed inside quoted messages only.
  - OD-U2 No VPN on the phone (Funnel): setup writes by default `allow_funnel`, `allow_unlock`, `camera_view`, `camera_view_funnel`, `password_alerts`, `push_notifications` and `offline_alerts` = `true` (`offline_alerts` only together with `push_notifications`). Each can be turned off in the GUI. An explicit `false` already in a setup-managed `remote.toml` (for example written by the GUI tour) is kept unless the matching `--<feature> on` flag is given.
  - OD-U3 The Tailscale login and node name come from `--tailscale-login` / `--rp-id`, else an interactive prompt (never with `--yes`, which then prints the steps). `tailscale status` is never read.
  - OD-U4 (not decided, conservative, "Owner to confirm"): the journal group needed by the password alerts is reported, never added; `loginctl enable-linger` is printed, never run.
  - OD-U5 The `install_remote.sh` `CARGO_TARGET_DIR` fix is in this PR.
- **Assumptions (Owner to confirm)**: AS-U1 started as root, the user phase is skipped with the exact command (re-run as the user without `sudo`), because it must run as the session owner with that owner's user manager; AS-U2 an existing `daemon.toml` is never edited (only created when absent) until the bounded TOML editor and `soos-daemon --check-config` of a later PR exist; AS-U3 a `remote.toml` that setup did not write (other than the untouched `install_remote.sh` template) is never edited; AS-U4 the user-phase failure exit code is `5`.

## 1. Scope & Blast Radius

| Area | Change |
|---|---|
| `scripts/setup.sh` | user phase (companion install, `remote.toml`, user units, verify, guided Tailscale), root-half `[preview]` creation, new flags, exit code 5, plan/summary lines |
| `scripts/install_remote.sh` | binaries copied from `${CARGO_TARGET_DIR:-<repo>/target}/release` (relative values resolved against the checkout), same rule as `setup.sh resolve_target_dir`; nothing else changes |
| `tests/invariants/src/setup_user_phase_contract.rs` + `lib.rs` | **new** contract file with its own fake-root harness (the PR 1 file `setup_contract.rs` is not edited) |
| `Docs/PACKAGING_AND_PROVISIONING.md` §3.0, `Docs/REMOTE_COMPANION.md` §4/§5, `README.md` | the one command now also sets up the phone companion |
| `AI/DECISIONS.md` | new ADR "Setup User Phase…" amending the #372 ADR items (7)–(9), plus a pointer at the end of the #372 ADR |
| `AI/VERIFICATION_MATRIX.md` | SET3 row text migrated (OD-U1); new rows SET20–SET3x; owner hardware row |
| `AI/walkthroughs/213_setup_user_phase.md` | walkthrough |

No Rust production code changes. No crate, unit file, daemon or `soos-remote` default changes.

## 2. Command Line (new options, user half)

| Option | Values | Meaning |
|---|---|---|
| `--no-remote` | — | skip the user phase (nothing of the companion is touched) |
| `--tailscale-login <LOGIN>` | `^[A-Za-z0-9._%+@-]{1,254}$` | the one `allowed_logins` entry |
| `--rp-id <HOST>` | lowercased; one trailing `.` dropped; `^([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.){2,}ts\.net$`, at most 253 bytes | `rp_id` (the node name `<pc>.<tailnet>.ts.net`) |
| `--funnel on\|off` | | `allow_funnel` |
| `--remote-unlock on\|off` | | `allow_unlock` |
| `--password-alerts on\|off` | | `password_alerts` |
| `--push on\|off` | | `push_notifications` |
| `--offline-alerts on\|off` | | `offline_alerts` |
| `--camera-view on\|off` | | `camera_view` (and, internally for `--system-phase`, the daemon side) |
| `--camera-view-funnel on\|off` | | `camera_view_funnel` |

Prerequisites (`soos-remote` refuses otherwise): `push_notifications` ⇐ `password_alerts`; `offline_alerts` ⇐ `push_notifications`; `camera_view_funnel` ⇐ `camera_view` ∧ `allow_funnel`. Every key needs `rp_id`, which setup always writes.

**Usage errors, exit 2, nothing changed** (checked after parsing, before detection effects):
- an explicit `on` whose prerequisite flag is an explicit `off` (`--push on --password-alerts off`, `--offline-alerts on --push off`, `--camera-view-funnel on --camera-view off`, `--camera-view-funnel on --funnel off`);
- `--no-remote` together with any companion option;
- an invalid login or node name.

**Preflight errors, exit 2, nothing changed**: a companion option (login, node name, any feature flag) given while the user phase cannot run (started as root, another `--user`, `XDG_CONFIG_HOME` relative, `HOME` not absolute, `XDG_RUNTIME_DIR` unset/not an owned directory, a user-managed `remote.toml`, a symbolic-link or foreign-owned `remote.toml` or configuration directory); an explicit `on` whose prerequisite resolves `off` from the existing managed file (message names the flag to add); with `--yes`, a feature flag or only one of login/node name while neither the flags nor the managed file provide both.

`--system-phase` additionally accepts `--camera-view on|off` (internal meaning: create the daemon `[preview]` section, §4.3). Companion options other than `--camera-view` are refused with `--system-phase` (exit 2).

## 3. Constants (`readonly` in `scripts/setup.sh`)

| Name | Value | Use |
|---|---|---|
| `EXIT_COMPANION` | 5 | face install kept, companion changes of this run rolled back |
| `MIN_TAILSCALE_VERSION` | `1.102.4` | guidance only (ADR 2026-10-05 item 8, header stripping verified) |
| `MAX_REMOTE_CONFIG_BYTES` | 16384 | bounded read of `remote.toml` (= `soos_remote::MAX_CONFIG_BYTES`) |
| `MAX_DAEMON_TOML_SCAN_BYTES` | 65536 | bounded read of an existing `daemon.toml` for the read-only `[preview]` check |
| `MAX_LOGIN_LEN` | 254 | `= soos_remote::MAX_LOGIN_LEN` |
| `MAX_HOST_LEN` | 253 | DNS name bound |
| `REMOTE_CONFIG_MARKER` | `# Managed by scripts/setup.sh (soos-setup): re-running setup rewrites this file.` | first line of a setup-managed `remote.toml` |
| `DAEMON_PREVIEW_MARKER` | `# Written by scripts/setup.sh (soos-setup): live camera view of the phone companion.` | first line of a setup-created `daemon.toml` |

All `systemctl --user` calls are bounded by `QUERY_TIMEOUT_S` (10 s) through `run_bounded`.

## 4. Phases

### 4.0 Companion resolution (user half, read-only, also in `--dry-run`)

`resolve_companion` sets `COMPANION` = `run` or `skip:<reason>`:

1. `--no-remote` → skip ("--no-remote").
2. Started as root → skip ("started as root: run ./scripts/setup.sh as <user> without sudo") (AS-U1).
3. Target user ≠ current user → skip ("run setup as <user>").
4. `HOME` not absolute, `XDG_CONFIG_HOME` set and not absolute, `XDG_RUNTIME_DIR` unset / not absolute / not a directory owned by the user → skip with the reason.
5. Paths: `CONFIG_DIR=${XDG_CONFIG_HOME:-$HOME/.config}/soos`, `REMOTE_CONFIG=$CONFIG_DIR/remote.toml`, `BIN_DIR=$HOME/.local/bin`, `USER_UNIT_DIR=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user`, `REMOTE_SOCKET=$XDG_RUNTIME_DIR/soos-remote/remote.sock`. `CONFIG_DIR` or `REMOTE_CONFIG` a symbolic link, `REMOTE_CONFIG` not a regular file or not owned by the user → preflight error (exit 2).
6. Classify `REMOTE_CONFIG` (bounded read; larger than `MAX_REMOTE_CONFIG_BYTES` → user-managed):
   - **absent** → `CONFIG_STATE=absent`;
   - **template**: every line blank or starting with `#`, except exactly one line `allowed_logins = []` (the untouched `install_remote.sh` template) → `template`;
   - **managed**: line 1 equals `REMOTE_CONFIG_MARKER`; every other line is blank, starts with `#`, or is `<key> = <value>` with `<key>` one of `allowed_logins` (`["<login>"]`, one login matching the login rule), `rp_id` (`"<host>"` matching the node-name rule), or one of the seven boolean keys (`true`/`false`); no key twice; `allowed_logins` and `rp_id` present → `managed`, with `FILE_LOGIN`, `FILE_RP_ID` and `FILE_BOOL[key]`;
   - otherwise → `user`.
7. Login and node name: the flag, else `FILE_LOGIN`/`FILE_RP_ID` of a managed file, else (interactive run: not `--yes`, not `--dry-run`, stdin and stdout a TTY) one prompt each (`read -r -p`; empty answer = skip the companion; invalid answer → exit 2). Still missing → skip ("needs the Tailscale login and node name"), unless a companion option was given (preflight error).
8. Feature values in the order `allow_funnel`, `allow_unlock`, `password_alerts`, `push_notifications`, `offline_alerts`, `camera_view`, `camera_view_funnel`: explicit flag; else `false` when the managed file has `false` (reported "kept off"); else `true` (owner default). Then, for a key without an explicit flag, a prerequisite resolved `false` forces `false` ("off: needs <prerequisite>"); for a key with an explicit `on`, a prerequisite resolved `false` is a preflight error (exit 2).
9. `CONFIG_ACTION`: `absent` → `create`; `template` → `replace`; `managed` → `already done` when the rendered file (§5) is byte-equal, else `rewrite`; `user` → `kept` (no write, no unit enable; the keys setup would set are printed).
10. `CAMERA_DAEMON` = `on` when the companion runs and `camera_view` resolves `true` (also for a user-managed file whose `camera_view` setup cannot know: `off`).
11. Companion install current: `cmp` of `<target>/release/soos-remote` and `soos-push-sender` with `BIN_DIR`, and of `packaging/soos-remote.service` / `soos-push-sender.service` with `USER_UNIT_DIR` → `already done`, else `will run`.

### 4.1 Plan lines (always printed; `--dry-run` stops after them)

`companion`, `companion config`, `camera view (daemon)`, `companion units`, `tailscale`. The login is never printed (`login: given`), the node name is printed (it is the URL the phone opens). Example: `[PLAN] companion config: will run - create <path> (mode 0600): Funnel on, remote unlock on, camera view on (Funnel on), password alerts on, push on, offline alerts on`. The `tailscale` line always reads `printed only - setup runs no tailscale serve or funnel`.

### 4.2 Interactive confirmation

The single existing `Proceed with this plan? [y/N]` covers the companion (owner decision: everything on by default; Funnel is published only by the owner's own `tailscale funnel` command).

### 4.3 Root half: camera view, daemon side (`--camera-view on` with `--system-phase`, or inline never — root starts skip the companion)

After `presence`, before `verify`. `DAEMON_TOML=${SYSCONFDIR}/soos/daemon.toml`:
- symbolic link → phase failure (rollback, exit 1);
- absent → `ensure_soos_conf_dir`; render (§5) into `mktemp` in `${SYSCONFDIR}/soos` (umask 022), `chmod 0644`, `ln` (no-clobber, atomic) to `DAEMON_TOML`, remove the temporary file; journal `daemon_toml_created`; `systemctl restart soos-daemon.service` (bounded); `wait_daemon_ready.sh`; failure → remove the file, `systemctl restart` again (best effort), phase failure (exit 1, journal rolled back);
- present and byte-equal to the render → `already done`;
- present, readable, at most `MAX_DAEMON_TOML_SCAN_BYTES`, and a line scan of the `[preview]` table finds exactly `enabled = true`, `remote_view = true` and `allowed_uids = [...]` containing the uid → `already done (daemon.toml allows it)`;
- otherwise → `[SKIP]` with the exact three lines and `sudo systemctl restart soos-daemon` (never edited, AS-U2).
Rollback of `daemon_toml_created`: `rm -f` then `systemctl try-restart soos-daemon.service`.

### 4.4 User phase (after the root half returned 0 or 3; as the user; `EXIT_COMPANION` on failure)

1. Re-check that `REMOTE_CONFIG` is unchanged since the plan (`cksum` or absence); changed → failure.
2. Config (`create`/`replace`/`rewrite`): `mkdir -p` the directory under `umask 077`; for `replace`/`rewrite` a backup `remote.toml.soos-setup.bak` (temporary file + `mv`, 0600); the render goes to `mktemp` in the directory, `chmod 0600`, `mv -f` over `REMOTE_CONFIG`. Journal `remote_config_created` or `remote_config_replaced`.
3. Install: `bash scripts/install_remote.sh` with `CARGO_TARGET_DIR=<resolved target>` and the cargo `PATH` (its stdout discarded, stderr kept), unless already done. It never overwrites the configuration.
4. Validate: `~/.local/bin/soos-remote --config <REMOTE_CONFIG> passkeys list` (stdout discarded; it loads and validates the whole configuration, exit 78 on any refusal, and prints no login) must exit 0 — for `create`/`replace`/`rewrite`/`already done`; skipped for `kept`.
5. Units (not for `kept`): `systemctl --user daemon-reload` is done by `install_remote.sh`; `soos-remote.service`: not enabled → `enable --now` (journal `unit_enabled`); enabled but inactive → `start`; active and (config changed or binaries installed) → `restart`. `soos-push-sender.service` when `push_notifications` is `true`: same rule; when `false`: untouched. `kept`: `try-restart` both units only when the binaries were installed.
6. Verify: `systemctl --user is-active --quiet soos-remote.service` (and the sender when enabled) → `[DONE] companion: running`; otherwise failure.
7. Failure (any step) → replay the user journal backwards (`unit_enabled` → `disable --now`; `remote_config_replaced` → restore the backup; `remote_config_created` → `rm -f`; then `try-restart soos-remote.service` when a configuration was restored), print `[FAIL] companion`, exit 5. Binaries installed by `install_remote.sh` are kept (`scripts/install_remote.sh --uninstall` printed). HUP/INT/QUIT/TERM during the user phase replay the journal (exit 129/130/131/143).

### 4.5 Guided steps (summary; printed, never run)

- Tailscale not installed → install it from the official instructions; version below `MIN_TAILSCALE_VERSION` → upgrade first.
- `tailscale funnel --bg unix:<socket>` when `allow_funnel`, else `tailscale serve --bg unix:<socket>`; Funnel needs the `funnel` node attribute in the tailnet policy (admin console).
- Open `https://<rp_id>` on the phone, add it to the home screen; register the passkey from the tailnet: `soos-remote enroll-code` at the PC (code shown only there, never by setup).
- Push: tap *Enable notifications*; a `/etc/resolv.conf` resolving into `/run` gets the sandbox warning (same read-only check as `install_remote.sh`).
- Journal access (OD-U4): the user is in `wheel`, `adm` or `systemd-journal` → reported; otherwise `sudo usermod -aG systemd-journal <user>` is printed, never run.
- Optional `loginctl enable-linger <user>` printed, never run.
- How to turn each feature off later (GUI feature tour) and `scripts/install_remote.sh --uninstall`.

## 5. Renders

`remote.toml` (managed):

```toml
# Managed by scripts/setup.sh (soos-setup): re-running setup rewrites this file.
# A feature turned off later (for example in the soos-gui feature tour) stays off until
# setup runs with its --<feature> on flag. Reference: Docs/REMOTE_COMPANION.md section 5.
allowed_logins = ["<login>"]
rp_id = "<host>"
allow_funnel = true
allow_unlock = true
password_alerts = true
push_notifications = true
offline_alerts = true
camera_view = true
camera_view_funnel = true
```

`daemon.toml` (created only when absent):

```toml
# Written by scripts/setup.sh (soos-setup): live camera view of the phone companion.
# Docs/DAEMON.md section 1.5; edit or remove it as an administrator.
[preview]
enabled = true
allowed_uids = [<uid>]
remote_view = true
```

## 6. Defaults table (what the installer path now applies)

| Feature | Key | Default written | Off later |
|---|---|---|---|
| Phone companion | unit `soos-remote` | enabled and started | GUI switch |
| Internet access | `allow_funnel` | true | GUI |
| Remote unlock | `allow_unlock` | true (passkey per unlock) | GUI |
| Camera view | `camera_view` + daemon `[preview]` | true (daemon side only when `daemon.toml` absent) | GUI |
| Camera view over Funnel | `camera_view_funnel` | true | GUI (cascade) |
| Password alerts | `password_alerts` | true | (edit) |
| Push | `push_notifications` + unit `soos-push-sender` | true | GUI |
| Offline alerts | `offline_alerts` | true, only with push | GUI |

Code defaults of `soos-remote` and `[preview]` stay `false` (manual path unchanged).

## 7. Invariants Touched

- User phase never as root; `install_remote.sh` root refusal unchanged.
- Only `tailscale version` executed (SET3, behavioural `assert_no_forbidden_calls`).
- No secret printed or stored: no enroll code, VAPID key, passkey data; login never printed.
- `remote.toml` 0600 in a 0700 directory, atomic, symbolic links refused; `daemon.toml` 0644 root, created no-clobber; never world-writable.
- No password capture (alerts carry classes and counts only; ADR 2026-10-06).
- Existing explicit `false` kept (GUI tour compatibility, `key = false` lines).
- `offline_alerts = true` never written without `push_notifications = true`.

## 8. Test Hooks for Tester

New file `tests/invariants/src/setup_user_phase_contract.rs`: own harness (fake root, fake HOME with `XDG_RUNTIME_DIR` owned by the test uid, stub `systemctl` handling `--user` with per-unit state, stub `install_remote.sh` that installs fake binaries/units and logs, fake `soos-remote` that validates the config with a few checks (exit 78 when `offline_alerts = true` without `push_notifications = true`, or `STUB_REMOTE_RC`), stub `tailscale`, `id -nG` controllable). Static checks on `setup.sh`, `install_remote.sh`, docs, ADR, matrix, walkthrough.

## 9. Documentation Drift / ADR

New ADR "[2026-10-15] Setup User Phase: Phone Companion Profile With Default-On Features (GitHub #400 = PR 2 of #362)" amending the #372 ADR: item (7) (`allow_unlock` and `camera_view_funnel` now default-on in the installer path by the 2026-10-15 owner decision; the code defaults stay), item (8) (unchanged: only `tailscale version`; serve/funnel printed), item (9) (user phase delivered; still deferred: PAM, TOML editor, `--check-config`, `--uninstall`, `--profile`, Docker matrix, `--from-release`), the SET3 migration, OD-U4 and AS-U1..AS-U4. `Docs/REMOTE_COMPANION.md` §4 wording "never enables the unit" stays true for `install_remote.sh`.

## 10. Revision 1 (answers to `AI/plan_evaluator_report.md` round 1) — normative, overrides §1–§9 where they differ

| Finding | Revision |
|---|---|
| F1 install decision | **R-U1** `install_remote.sh` runs on every companion run (incremental; it builds `-p soos-remote -p soos-push-sender`, whose feature set differs from `--workspace`, so the workspace artifacts are never compared). The decision is taken on the installed files: `cksum` of `~/.local/bin/soos-remote`, `soos-push-sender` and the two user units before and after; equal → `[DONE] companion: already up to date (installed files unchanged)` and no restart; different → installed, running units restarted. The plan line is always `will run`. Tests: unchanged files (second run, SET25) and new files (first run, SET24); owner hardware row SET39 for the real second run. |
| F2 disabled unit | **R-U2** Setup enables the units only for a configuration it writes for the first time (absent or template). When a setup-managed `remote.toml` already exists and `soos-remote.service` (or, with push, `soos-push-sender.service`) is not enabled, it stays disabled: plan `companion units: kept off`, run `[SKIP] ... stays disabled; enable it with: systemctl --user enable --now <unit>`. No new flag (*Owner to confirm*, AS-U5). Test SET37. |
| F3 text constraints | **R-U3** Text rules for `setup.sh`: the word `tailscale` followed by a space appears only as `tailscale version` or inside a quoted string (plan key written `plan "tailscale"`); no heredoc for guidance; Tailscale install guidance is the download page URL only (no `curl`); no package name of `check_build_deps.sh` as a word (`make`, `pam`, `clang`, `curl`, …); uid only from `id -u` / `TARGET_UID`, never `$UID`; never the strings `tailscale status`, `tailscale up`, `tailscale set`, `--http` (SET20). |
| F-m1 settle | **R-U4** After any start, enable or restart of a user unit in this run, setup waits `COMPANION_SETTLE_S` = 2 s, then each unit it manages must be `is-active` (a `Type=exec` unit refused with exit 78 is then `failed`; a crash loop is `activating`). |
| F-m2 validation | **R-U5** `soos-remote --config <file> passkeys list` (bounded by `run_bounded`) exits 0 when the whole configuration is accepted, 78 when refused and 1 when an existing passkey store is refused; any non-zero status is a companion failure ("soos-remote refused …"). Its stdout (no login) is discarded. |
| F-m3 | **R-U6** `unit_enabled:<unit>` is journaled before `enable --now`, so a partial enable is rolled back (SET33). |
| F-m4 exit codes | **R-U7** Root half exit 0 → companion runs, final exit 0 or 5; exit 3 → companion runs, final exit 3 (a companion failure is printed); any other exit (1, 4, 70, signals) → `[SKIP] companion: the system phase did not complete (exit N)`, the companion does not run, the root half's exit code is kept. |
| F-m5 | **R-U8** `usage()` documents every companion option and exit code 5; the PR 1 texts are updated: `[DETECT] tailscale: <version> (read only; setup never runs serve or funnel)` and `[PLAN] later: not in this version - PAM activation`. |
| F-m6 | **R-U9** Documented: a key forced off by a prerequisite is written `false` and is then indistinguishable from a key turned off in the GUI; turning the prerequisite back on later keeps it off until its own `--<feature> on` (ADR, Docs). |
| F-m7 | **R-U10** The `[preview]` uid is the root half's own `TARGET_UID` (getent of `--user`); only `--camera-view on` crosses the escalation. |
| F-m8 | **R-U11** Documented: a setup-created `daemon.toml` is kept by `--camera-view off` and by `scripts/uninstall.sh` (administrator file; removed by hand). |
| F-m9 | **R-U12** On a host whose `/etc/resolv.conf` resolves into `/run`, the guide warns that push notifications are NOT delivered until the resolver drop-in of `Docs/REMOTE_COMPANION.md` §2d is added (setup never adds it). |
| F-m10 | **R-U13** A new configuration directory is created `0700`; an existing one must be owned by the user and not group- or world-writable, else the companion is skipped (exit 2 with companion options) with the `chmod 0700` hint (SET38). |
| F-m11 | **R-U14** The render is pinned to `tests/fixtures/setup/remote.toml` (SET38) and that fixture plus its cascade variants are parsed by the real `soos_remote::config::parse_config` in `crates/remote/tests/setup_render_tests.rs`; variants setup never writes are refused there. |
| F-m12 | Draft contract realigned with this revision before the Red evidence is recorded. |
| F-m13 | AS-U1 stays under "Owner to confirm" and in the ADR. |
| F-m14 | **R-U15** When the daemon is not ready after `daemon.toml` was created, setup removes the file and restarts the daemon: ready again → the root half is rolled back (exit 1); still not ready → `[FAIL]`, state written, exit 70, nothing else rolled back (SET34). |
| — | **R-U16** Uniform rule for every reason that prevents the companion (root start, other user, `HOME`/`XDG_CONFIG_HOME`/`XDG_RUNTIME_DIR`, symbolic link, foreign or writable path): without companion options it is skipped with the reason; with companion options it is a preflight error (exit 2). A symbolic-link `daemon.toml` is never followed (`[SKIP]`, hint printed). |

Rows: SET20–SET36 as in §8, SET37 (R-U2), SET38 (R-U13, R-U14), SET39 owner hardware run.

## 11. Revision 2 (plan evaluation round 2) — normative

- **R2-F1** The sender is kept off (R-U2) only when the existing setup-managed file already had `push_notifications = true` and `soos-push-sender.service` is not enabled; when push goes from `false` to `true` (or the file is written for the first time) the sender is enabled and started, journaled per R-U6. Test SET37 (`test_set_u21_push_switched_on_later_enables_the_sender`), next to `test_set_u19_disabled_unit_stays_disabled`.
- **R2-m1** The plan line of `companion` says that `install_remote.sh` builds its own `soos-remote`/`soos-push-sender` variant (a second compile on the first run); Docs §3.0 says so too.
- **R2-m2** Neutral wording: "`<unit>` is not enabled; setup only enables it for a configuration it writes for the first time (or, for the sender, when push is switched on)". After `install_remote.sh --uninstall` and a re-run with a setup-managed file, the units therefore stay disabled until enabled by hand (AS-U5, ADR item (9)).

## 12. Revision 3 (lead review of PR #402) — normative

- **R3-1** soos-daemon reads only `/etc/soos/daemon.toml` (`crates/daemon/src/config.rs` `DEFAULT_CONFIG_PATH`; `packaging/soos-daemon.service` passes no `--config`). `setup.sh` declares `DAEMON_READ_CONFIG="/etc/soos/daemon.toml"`; the camera-view daemon side is read, written and reported done only when `<sysconfdir>/soos/daemon.toml` equals it. Otherwise the state is `elsewhere`: the user half does not pass `--camera-view on`, the root half (defence in depth) prints `[SKIP] camera view (daemon): not applied` with the exact change for `/etc/soos/daemon.toml`, the plan line is `manual (not applied)`, and nothing is written or restarted. The `/etc` creation path is unchanged and checked on real hardware (SET39). PR 1's `presence.disable` has the same `--sysconfdir` mismatch (the daemon reads `/etc/soos`); reported, not changed.
- **R3-2** A `remote.toml.soos-setup.bak` that exists and is not a regular file owned by the user refuses the companion (exit 2 with companion options).
