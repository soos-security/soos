# Research: failed-password alerts in soos-remote

Status: research note for GitHub #339 (branch `feat/remote-auth-alerts`), 2026-10-06.
Scope: every external fact the "failed-password alerts" design (owner decisions O-1 to O-5,
feature level 1) depends on. Nothing on the host was changed: only read-only commands were run
(`journalctl` queries, `getfacl`, `ls -l`, `pacman -Q`, `systemctl [--user] show/list-units`,
`/proc/<pid>/status`, `man`). No journal entry was written, no unit was started or restarted.
Account names below are written `<owner>` (UID 1000); the counts and field layouts are real.

## 0. Versions and sources

| Component | Installed (Arch, `pacman -Q`) | Source read |
|---|---|---|
| systemd / journalctl | `systemd 262 (262-1-arch)`, `+PCRE2 +ZSTD +LZ4 +XZ` | `man journalctl`, `man systemd.journal-fields` (262) |
| Linux-PAM | `pam 1.7.3-1` | tag `v1.7.3` (commit `28fce1fd55b0…`): `modules/pam_unix/support.c`, `modules/pam_unix/unix_chkpwd.c`, `modules/pam_unix/passverify.c`, `modules/pam_faillock/{pam_faillock.c,faillock.c,faillock.h}`, `libpam/pam_syslog.c` |
| sudo | `1.9.17.p2-6` | journal evidence only |
| gdm | `50.3-1` | journal evidence only |
| polkit | `127-3` | journal evidence only |
| Lock screen | `swaylock-plugin` (not a pacman package: `~/.local/bin/swaylock-plugin`), PAM service `swaylock` | journal evidence only |
| tokio (workspace) | `1.53.1`, features `rt-multi-thread net sync time macros signal io-util` (no `process`) | `Cargo.toml:41`, `Cargo.lock` |
| nix (workspace) | `0.29`, features `socket fs user` | `Cargo.toml:42` |

Permalink form for PAM: `https://github.com/linux-pam/linux-pam/blob/v1.7.3/<path>#L<n>`
(abbreviated `pam@v1.7.3:<path>:<line>`).

## 1. Who can read the journal, and does soos-remote qualify

- `man journalctl`: "Members of the groups `systemd-journal`, `adm`, and `wheel` can read all
  journal files." Without one of them a user only sees its own user journal.
- Host: `/var/log/journal` is `root:systemd-journal` with ACL `group:wheel:r-x`,
  `group:adm:r-x` (default ACL identical); `system.journal` is `-rw-r-----+` with
  `group:wheel:r--`. Storage is persistent (`/var/log/journal` exists, `journald.conf` has no
  override and no `journald.conf.d`), 404 MiB in use, entries from 60+ days back are readable.
- `id`: the owner is in `wheel` (998).
- The running `soos-remote.service` (user unit, PID from `systemctl --user show -p MainPID`)
  has `Groups: 944 957 989 992 998 1000` in `/proc/<pid>/status`, so the user manager passes
  the supplementary group `wheel` to the service: it can read the system journal without root.
- Consequence for fail-closed design: on a host where the owner is NOT in
  `wheel`/`adm`/`systemd-journal`, `journalctl` still runs and exits 0 but only shows the user
  journal (root-side services such as `gdm-password`, `sudo`'s `unix_chkpwd` with `_UID=0`,
  `polkit-agent-helper-1`, `soos-daemon` would be invisible). A startup probe is needed, e.g.
  `journalctl -q -n 1 -o json _UID=0` must return one entry (checked: it returns one here).

## 2. What a wrong password writes to the journal (PAM source + host evidence)

### 2.1 Log producers (source)

1. `pam_unix` in the PAM-using process, first failure per PAM handle only:
   `pam_unix(<service>:auth): authentication failure; logname=… uid=… euid=… tty=… ruser=…
   rhost=…  user=<name>` (`pam@v1.7.3:modules/pam_unix/support.c:803-836`). A second failure
   on the same handle only increments a counter (`new->count = old->count + 1`, no log). The
   summary `N more authentication failures` is logged by `_cleanup_failures`
   (`support.c:297-316`) only when the handle is torn down without `PAM_DATA_REPLACE`; a later
   success resets the data with `pam_set_data(..., NULL, ...)` (`support.c:767-768`), i.e. a
   replace, so the summary is not logged. **Hence `pam_unix` lines undercount attempts in
   long-lived PAM clients (lock screen, sudo's 3 tries).** No `more authentication failure`
   line exists in 60 days of this journal.
2. `unix_chkpwd` helper, once per password check, whenever `pam_unix` cannot read the shadow
   hash itself (`get_pwd_hash` returns `PAM_UNIX_RUN_HELPER`, `passverify.c:249`,
   `support.c:740-742`): `password check failed for user (<name>)`
   (`unix_chkpwd.c:171-174`), via `openlog("unix_chkpwd", LOG_CONS|LOG_PID, LOG_AUTHPRIV)`
   (`passverify.c:1074`). It is logged for every failed check except a blank password with
   `nullok`. Before exec, `pam_unix` calls `setuid(0)` in the child when it is running with
   `euid == 0` (`support.c:562-569`), so the helper's real UID is 0 for sudo / polkit / gdm and
   the caller's UID (1000) for the lock screen. `/usr/bin/unix_chkpwd` is `-rwsr-sr-x root`.
3. `pam_faillock` (`pam_faillock.c:167-170`, `408`, `509`): `User unknown: <name>`,
   `Consecutive login failures for user <name> account temporarily locked`,
   `User <name> is temporarily locked out due to N consecutive failed login attempts`. An
   attempt made while locked out is refused at `preauth`; no password check runs, so no
   `unix_chkpwd` and no `pam_unix` failure line is written.
4. `pam_syslog` always uses facility `LOG_AUTHPRIV` (`libpam/pam_syslog.c:99`), i.e.
   `SYSLOG_FACILITY=10` in the journal (checked on sudo and lock-screen entries).
5. soos itself (already installed on this host): `/etc/pam.d/system-auth` contains
   `auth optional pam_soos.so event=password-failed timeout_ms=20` right after
   `pam_unix.so` (`[success=2 default=bad]`) and before `pam_faillock.so authfail`. On each
   `pam_unix` failure `pam_soos.so` sends an `Event { kind: PasswordFailed, uid, service, … }`
   to `soos-daemon` (`crates/pam/src/lib.rs:277-287`, `Docs/IPC_PROTOCOL.md` "Event"), which,
   after the per-peer quota (5 events / 10 s / peer UID, `crates/daemon/src/limits.rs:37-41`)
   and the peer/target UID check, logs `INFO Processing telemetry auth failure event
   peer_uid=<p> target_uid=<t>` (`crates/daemon/src/dispatcher.rs:576-580`). The service name
   is NOT in that log line. The PAM module refuses to emit an event when the PAM user does not
   resolve to a UID (`lib.rs:267-274`).

PAM stacks on the host (read-only): `sudo` → `system-auth`; `swaylock` → `login` →
`system-local-login` → `system-login` → `system-auth`; `gdm-password` → `system-local-login`
→ … → `system-auth`. So every observed local password service goes through the same
`pam_unix` + `pam_soos event=password-failed` + `pam_faillock` sequence.

### 2.2 Host evidence (2026-10-05, one day, all failures correlated)

| Time | `unix_chkpwd` | `pam_unix(...:auth)` failure | `soos-daemon` event | faillock |
|---|---|---|---|---|
| 16:06:10 | yes | `swaylock` | `peer_uid=1000` | – |
| 16:22:46 | yes | `swaylock` | `peer_uid=1000` | – |
| 16:24:43 | yes | – (same handle) | `peer_uid=1000` | – |
| 16:25:38 | yes | – | `peer_uid=1000` | "Consecutive login failures … temporarily locked" |
| 16:25:59 | – | – | – | "temporarily locked out" (attempt while locked) |
| 16:26:00 | yes | – | `peer_uid=1000` | – |
| 16:34:07, :16, :23 | – | – | – | "temporarily locked out" ×3 |
| 16:34:17 | yes | `swaylock` | `peer_uid=1000` | – |
| 16:34:38, :51, :56 | – | – | – | `gdm-password` "temporarily locked out" ×3 |
| 18:05:21 | yes | `sudo` | – (daemon restarting, PID changed) | – |
| 18:05:28 | yes | – (same sudo handle) | – (daemon restarting) | – |
| 18:09:05 | yes | `sudo` | `peer_uid=0` | – |
| 18:14:27 | yes | `polkit-1` | `peer_uid=0` | – |
| 21:04:46 | yes | `polkit-1` | `peer_uid=0` | – |

Facts drawn from it:

- `unix_chkpwd` lines = one per real password check (11 that day); `pam_unix` failure lines =
  6 (undercount, §2.1.1); daemon events = 9 (missed 2 while `soos-daemon` was restarting).
- Attempts during a faillock lockout leave only a `pam_faillock` "temporarily locked out"
  line (7 that day, lock screen and GDM). Whether those count as "failed password attempts"
  is a design decision; they are attempts, but the typed text was never checked.
- 60-day totals of `pam_unix(<service>:auth): authentication failure` by service:
  `swaylock` 53, `sudo` 9, `gdm-password` 8, `polkit-1` 2. No `login` (tty) failure exists in
  the journal, so its format is known from source only (§2.1.1), not observed.
- GDM (root `gdm-session-worker`, `_UID=0`) also logs `pam_unix(gdm-password:auth):
  conversation failed` / `auth could not identify password for [<name>]` when the greeter
  aborts; these are not wrong passwords.
- No `sudo: N incorrect password attempts` line exists in the 60-day journal (sudo's own
  summary was never emitted on this host; not relied upon).

### 2.3 Noise that must not become an alert

- Developer test binaries on this host write real PAM failure lines for service `swaylock`:
  `~/.config/driftwm/rust/target/release/deps/drift_verrou-*` (64 `pam_faillock(swaylock:auth):
  User unknown` and 27 `pam_unix(swaylock:auth): authentication failure` lines in 60 days,
  cgroup `kitty-*.scope`), and `~/.local/bin/drift-verrou` (4). They are indistinguishable by
  message from the real lock screen; only `_EXE` / `_SYSTEMD_USER_UNIT` differ (real lock
  screen: `_EXE=/home/<owner>/.local/bin/swaylock-plugin`, `_SYSTEMD_USER_UNIT=driftwm.service`).
- `cargo test` of `crates/pam` on the host writes `soos-pam:` lines on `authpriv` (hundreds per
  run: "could not resolve the PAM user", "authentication panic caught", configuration errors).
- `pam_unix(sudo:session): session opened/closed` and sudo command lines share the facility.
- Volume: `SYSLOG_FACILITY=10 + _SYSTEMD_UNIT=soos-daemon.service` = 47 174 entries over 7
  days (~6 700/day, mostly `soos-daemon` chatter); JSON lines with the 8 selected fields: max
  4 001 bytes, mean 1 134 bytes.

## 3. Trust of journal fields (O-3)

- `man systemd.journal-fields`, "TRUSTED JOURNAL FIELDS": fields prefixed with `_` "are
  implicitly added by the journal and cannot be altered by client code". `SYSLOG_IDENTIFIER`
  and `MESSAGE` are client-controlled: any local process can log
  `SYSLOG_IDENTIFIER=unix_chkpwd MESSAGE="password check failed for user (x)"` (syslog,
  `logger -t`, `systemd-cat -t`, native protocol).
- `_UID`/`_GID`/`_PID` come from the socket credentials. Linux lets a sender pass explicit
  `SCM_CREDENTIALS` only with a UID equal to its real, effective or saved UID, unless it has
  `CAP_SETUID` (`net/core/scm.c`, `__scm_send` → `SCM_CREDENTIALS` check). Default credentials
  are the real UID: observed `sudo` (real 1000, effective 0 per its own `uid=1000 euid=0`) is
  stored with `_UID=1000`. **An unprivileged process can never produce `_UID=0`.**
- `_SYSTEMD_UNIT` / `_SYSTEMD_CGROUP` come from the sender's cgroup. A UID-1000 process can
  move itself only inside its delegated `user@1000.service` subtree, never into
  `system.slice/gdm.service`, `soos-daemon.service` or `polkit-agent-helper@*.service`.
- `_COMM` is the kernel task name, settable by the process itself (`prctl(PR_SET_NAME)`,
  or by naming its executable `unix_chkpwd`). `_EXE` comes from `/proc/<pid>/exe` and is only
  present when journald reads it before the process exits.
- `_TRANSPORT` (`syslog`, `journal`, `stdout`, …) is trusted: PAM lines are `syslog`,
  `soos-daemon` lines are `stdout`.
- Observed per producer (60 days):

| Producer | `_UID` | `_COMM` | `_EXE` | `_SYSTEMD_UNIT` / user unit | Forgeable by a UID-1000 process? |
|---|---|---|---|---|---|
| `unix_chkpwd` (lock screen) | 1000 | `unix_chkpwd` or absent | **absent in 54/54 entries** | `user@1000.service` or absent | yes (same UID, `_EXE` never present to compare) |
| `unix_chkpwd` (sudo, polkit, gdm) | 0 | `unix_chkpwd` or absent | absent | `user@1000.service` / `gdm.service` / absent | no (`_UID=0`) |
| `pam_unix(swaylock:auth)` | 1000 | `swaylock-plugin` | `/home/<owner>/.local/bin/swaylock-plugin` (owner-writable) | `user@1000.service` / `driftwm.service` | yes |
| `pam_unix(sudo:auth)` | 1000 | `sudo` | `/usr/bin/sudo` | `user@1000.service` / terminal scope | only by actually running sudo with a wrong password (a real attempt) |
| `pam_unix(gdm-password:auth)` | 0 | `gdm-session-wor` | `/usr/lib/gdm-session-worker` | `gdm.service` | no |
| `pam_unix(polkit-1:auth)` | 0 | `polkit-agent-he` | `/usr/lib/polkit-1/polkit-agent-helper-1` | `polkit-agent-helper@….service` | no |
| `soos-daemon` event | 0 | `soos-daemon` | `/usr/libexec/soos/soos-daemon` | `soos-daemon.service`, `_TRANSPORT=stdout` | no (but a UID-1000 process can make the daemon log a genuine-looking event for its own UID by speaking the IPC protocol: `PasswordFailed` is accepted from any peer for itself, quota 5/10 s) |

- Residual risk (to document in the ADR): every lock-screen signal is produced by UID-1000
  code, so any process running as the owner can forge lock-screen alerts (false positives
  only: it cannot read, suppress or alter other entries through this path, and it already
  runs as the owner). Root-side signals (`_UID=0` + system unit) cannot be forged without
  root. Processes of other local UIDs can forge entries with their own `_UID`, so restricting
  accepted `_UID` to `{0, owner}` removes them.

## 4. O-2: what the journal can and cannot reveal

- No producer above logs the typed password, its length or a hash: `pam_unix` logs the
  service, `logname`, `uid/euid`, `tty`, `ruser`, `rhost`, `user`; `unix_chkpwd` reads the
  password from a pipe and overwrites it (`unix_chkpwd.c:163`, `pam_overwrite_array`).
- **But the `user=` / `(name)` / `User unknown: <name>` field is whatever was typed in the user
  name prompt.** `pam_unix` itself warns: "this might be a typo and the user has given a
  password instead of a username" (`support.c:747-748`, logged only with the `audit` option,
  otherwise it logs `check pass; user unknown` without the name). `pam_faillock` logs
  `User unknown: <name>` with the raw name (`pam_faillock.c:167`). At a GDM or tty login, a
  password typed into the user field therefore reaches the journal. To honour O-2 the
  account name must only be shown when it resolves to an existing local account (e.g.
  `nix::unistd::User::from_name`, feature `user` already enabled) or matches the configured
  `allowed_logins`; otherwise it must be reported as an unnamed "unknown account" and the raw
  field dropped. `soos-daemon` events carry only numeric UIDs (resolved by `pam_soos` before
  sending), so they never carry typed text.
- The raw `MESSAGE` of `pam_unix` also carries `rhost`/`ruser`/`tty`; O-2 forbids forwarding
  raw lines, so only the parsed class, account and time leave the parser.

## 5. Following the journal without a C binding

Options considered:

| Mechanism | Fact |
|---|---|
| `sd-journal` via `libsystemd` (FFI crates) | C library binding: excluded by the task; FFI is also confined to adapter crates by the workspace rules. |
| Pure-Rust journal file reader | Journal files are mmap'ed, rotated (`system@….journal`, `user-1000@….journal~`), compressed (zstd/xz/lz4) and FSS-sealable; no maintained pure-Rust reader is used in this workspace. Not pursued. |
| journald Varlink (`io.systemd.Journal`) | Exposes control calls (synchronize, rotate, flush), not a subscription stream (systemd 262 `journald` varlink). Not usable. |
| `systemd-journal-gatewayd` | Separate HTTP service, needs root to install/enable (O-5 forbids touching `/etc`). Not usable. |
| `/run/faillock/<owner>` tally (`-rw-rw---- <owner> root`, created `0660` and `fchown`'d to the user, `faillock.c:74-102`) | Binary records `{source, status, time}`; reset on success; owner-writable (forgeable); does not see attempts outside faillock. Not suitable as primary source. |
| **Spawn `journalctl --follow -o json`** | Works under the current unit (below). Recommended. |

Facts for the `journalctl` child (all checked on systemd 262):

- `journalctl -f -n 0 -q -o json …` prints only entries appended after start (checked: ran 3 s
  under `timeout`, printed nothing, exit 124). `-n 0` is accepted with `--follow`.
- Matches combine as `FIELD=VALUE [FIELD=VALUE …] [+ FIELD=VALUE …]` (`+` = OR); checked:
  `SYSLOG_FACILITY=10 + _SYSTEMD_UNIT=soos-daemon.service` selects both PAM lines and daemon
  events. Further narrowing per term is possible (e.g. `SYSLOG_FACILITY=10 _UID=0`).
- `--output-fields=A,B,…` restricts the JSON object; `__CURSOR`, `__REALTIME_TIMESTAMP`,
  `__MONOTONIC_TIMESTAMP`, `_BOOT_ID` are always printed (man), and on 262 also `__SEQNUM`
  and `__SEQNUM_ID` (checked).
- JSON encoding (man, "json"): one object per line; fields larger than 4096 bytes are `null`
  (unless `--all`); duplicate fields become a JSON array of values; **fields containing
  non-printable or non-UTF-8 bytes become an array of byte numbers**. Checked: every
  `soos-daemon` `MESSAGE` is a byte array because it contains ANSI colour escapes
  (`ESC[2m…ESC[32m INFO ESC[0m Processing telemetry auth failure event ESC[3mpeer_uid…`). The
  parser must accept string and byte-array forms and strip SGR sequences, and must treat
  ambiguous arrays (duplicate field) as untrusted / reject.
- `-g PATTERN` uses PCRE2, loaded with `dlopen` (`libsystemd-shared-262-1.so` references
  `pcre2_compile_8`/`pcre2_match_8`, no `pcre2_jit_*` symbol), so it does not need W+X memory.
  Filtering can also be done in Rust instead of `-g`.
- `--cursor-file=FILE` / `--after-cursor=` allow resuming after a restart without gaps; the
  persistent journal (60+ days here) also allows rebuilding a recent history at startup with
  `--since` instead of persisting a history file (a file would still be needed for an
  "acknowledged up to" marker if acknowledgements must survive restarts).
- `--follow` mode: "journalctl will send an sd_notify(3) READY=1 message once it initialized"
  (man). `soos-remote.service` is `Type=exec`, so `NOTIFY_SOCKET` is not set; the child's
  environment should still be cleared to keep it that way.
- Default reads "all messages that the user can see" (system + user journals); `--user` needs
  persistent storage (present) but must not be used here (it would hide root-side entries).

Sandbox compatibility with `packaging/soos-remote.service` (read via `systemctl --user show`):
`NoNewPrivileges=yes`, `RestrictAddressFamilies=AF_UNIX`, `MemoryDenyWriteExecute=yes`,
`LockPersonality`, `RestrictRealtime`, `RestrictSUIDSGID`, `SystemCallArchitectures=native`,
`UMask=0077`, no `ProtectSystem`, no `SystemCallFilter`. `journalctl` is not setuid
(`-rwxr-xr-x root /usr/bin/journalctl`), needs only file reads and inotify (no socket family
other than possibly `AF_UNIX`), no JIT, and inherits the `wheel` group, so none of these
directives blocks it; `RestrictAddressFamilies=AF_UNIX` can stay. The child is killed with the
service (default `KillMode=control-group`); in-process, `tokio::process::Command::kill_on_drop`
covers restarts of the follower task. `PR_SET_PDEATHSIG` would need `CommandExt::pre_exec`, a
raw-memory API that the crate-level `forbid` lint on this business crate excludes. Not verified by running `journalctl` inside a
transient unit with these properties (O-5: no unit started); the developer should confirm with
the installed service on the owner's go.

Rust-side facts:

- `tokio::process` needs the tokio feature `process`, not enabled in the workspace today
  (`Cargo.toml:41`). On Linux it reuses `signal-hook-registry` (already in `Cargo.lock`,
  1.4.8, pulled by `signal`), so `cargo deny` should see no new crate; to confirm when the
  feature is added. Alternative without the feature: `std::process::Command` read on a
  dedicated blocking thread feeding a bounded channel.
- soos-remote logs to stderr → journal as user unit `soos-remote.service`, `_UID=1000`,
  `_TRANSPORT=stderr`, not on `authpriv`, so the chosen matches cannot feed its own lines back.

## 6. Time display

- Host timezone `Europe/Paris` (`timedatectl show -p Timezone`). `__REALTIME_TIMESTAMP` is
  microseconds since the Unix epoch (UTC), so the server can send epoch milliseconds and let
  the phone render "HH:MM" in its own locale/timezone.

## 7. Open points for the architect / owner

1. **User request vs O-2.** The relayed request asks to "see on the phone the passwords that
   were tested". No journal source contains them (§4), and obtaining them would require a new
   PAM component capturing `PAM_AUTHTOK`, which O-2 and AGENTS.md ("never transmit passwords
   over the IPC socket", "never log credentials") forbid; the owner's own typos would also be
   near-copies of the real password. This research assumes the binding O-2 reading (attempts,
   not their text) and the owner should confirm that wording.
2. Counting source: `unix_chkpwd` (complete, but lock-screen lines are owner-UID and
   forgeable, no `_EXE`), `pam_unix` (trusted for root services but undercounts), or the
   `soos-daemon` event (trusted, root, numeric UID only, no service name, misses attempts
   while the daemon is down, quota 5/10 s). Correlating them within the same second gives a
   complete count plus a source class; adding the service class to the daemon log line would
   be a change outside `crates/remote`.
3. Whether faillock "temporarily locked out" attempts count, and whether `conversation failed`
   (GDM abort) is excluded.
4. Excluding developer test binaries (`_EXE` under `~/.config/driftwm/…/target/`) and the
   `soos-pam:` test chatter: an allow-list of trusted `(_COMM, _EXE, _UID, unit)` tuples per
   source class is implied by §2.3 and §3.
