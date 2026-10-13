# Architect Spec — GitHub #365: opt-in polkit Wi-Fi lock and `soos-admin network-lock status`

- **Date**: 2026-10-13
- **Issue**: GitHub-only #365 (Part A of #359, planned split item 1); no `AI/BACKLOG.md` id.
- **Branch**: `feat/network-lock` (not registered in `scripts/sync_issue.py`; the commit carries `Closes #365`).
- **Base commit**: `1b6d211` (`origin/main`).
- **Owner decisions** (#359 comments of 2026-10-12 and 2026-10-13, restated by the owner for this run):
  - OD-1 Scope: changing and disconnecting Wi-Fi only (not DNS, hostname or every NetworkManager action).
  - OD-2 `polkit.Result.AUTH_ADMIN` on every request (never `AUTH_ADMIN_KEEP`, `YES` or `NO`); every other action falls through.
  - OD-3 The rfkill udev rule is a separate option, **off by default**; the status diagnostic reports the `rfkill_uaccess` gap.
  - OD-4 No separate admin account; the docs explain that the lock prompts for the wheel user's own password like `sudo`, and how to add a separate admin account.
  - OD-5 Face verification for polkit prompts is not provided (documented, `Docs/PAM_MODULE.md` polkit-agent-helper note).
  - OD-6 The one-command installer (#362) does not exist yet: the install/uninstall step ships in a form #362 can call later, with a temp-root mode for tests. Agents never install anything on the host's real `/etc` and never run `sudo`.

## 1. Scope & Blast Radius

| Area | Change |
|---|---|
| `packaging/polkit/49-soos-network-lock.rules` | **new**: polkit JavaScript rule (D-1, D-2) |
| `packaging/udev/72-soos-network-lock-rfkill.rules` | **new**: udev rule removing the `uaccess` tag from `/dev/rfkill` (D-4) |
| `crates/admin-cli/src/network_lock.rs` | **new** module: embedded rule contents, constants, status, install, uninstall (§2) |
| `crates/admin-cli/src/args.rs` | new `Commands::NetworkLock(NetworkLockArgs)` and `NetworkLockAction` |
| `crates/admin-cli/src/main.rs` | dispatch of `network-lock status|install|uninstall` |
| `crates/admin-cli/src/lib.rs` | `pub mod network_lock;` and re-exports |
| `crates/admin-cli/src/error.rs` | new variant `AdminCliError::NetworkLock(String)` |
| `crates/admin-cli/Cargo.toml` | add `sha2 = { workspace = true }` (already a workspace dependency, no new crate in `Cargo.lock`) |
| `scripts/uninstall.sh` | removes both network-lock files under `DESTDIR` + `SYSCONFDIR` (D-5) |
| `tests/docker/network_lock_pkcheck_test.sh` | **new**: real polkitd + `pkcheck` in the pinned `ubuntu:24.04` image (D-8) |
| `.github/workflows/ci.yml` | **new** job `network-lock-polkit` (after `lint`), added to `ci-success.needs` |
| `tests/invariants/src/network_lock_contract.rs` + `lib.rs` | **new** static contracts (§7) |
| `crates/admin-cli/tests/network_lock_*` | **new** unit/integration and shell tests (§7) |
| Docs | **new** `Docs/NETWORK_LOCK.md`; pointers in `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/README.md`, `Docs/PAM_MODULE.md` (polkit note) |
| Traceability | ADR, matrix section `NLK*`, walkthrough 196 |

Untouched: `crates/pam`, the daemon, the IPC protocol, `soos-remote`, `scripts/install.sh` (the lock is opt-in and is **not** installed by the current installer), the distribution packages (they ship `soos-admin`, which embeds both rule files; nothing is active by default).

Consumers of changed public items: `Commands` gains a variant. Every existing test that matches `Commands` uses a `_ => panic!` arm (`grep -rn "Commands::" crates/admin-cli/tests`), `main.rs` is the only exhaustive match. No other crate depends on `soos-admin-cli`.

## 2. Types & Signatures (`crates/admin-cli/src/network_lock.rs`, `#![forbid(unsafe_code)]` inherited)

```rust
/// Shipped polkit rule, single source: `packaging/polkit/49-soos-network-lock.rules`.
pub const NETWORK_LOCK_RULES: &str =
    include_str!("../../../packaging/polkit/49-soos-network-lock.rules");
/// Shipped udev rule of the rfkill option: `packaging/udev/72-soos-network-lock-rfkill.rules`.
pub const NETWORK_LOCK_RFKILL_RULE: &str =
    include_str!("../../../packaging/udev/72-soos-network-lock-rfkill.rules");

/// Install locations relative to the root directory (`/` in production).
pub const NETWORK_LOCK_RULES_DIR: &str = "etc/polkit-1/rules.d";
pub const NETWORK_LOCK_RULES_FILE: &str = "49-soos-network-lock.rules";
pub const NETWORK_LOCK_RFKILL_DIR: &str = "etc/udev/rules.d";
pub const NETWORK_LOCK_RFKILL_FILE: &str = "72-soos-network-lock-rfkill.rules";

/// The NetworkManager polkit actions the rule answers with AUTH_ADMIN (D-1), in file order.
pub const NETWORK_LOCK_ACTION_IDS: [&str; 5] = [ // 6 before R-1
    "org.freedesktop.NetworkManager.network-control",
    "org.freedesktop.NetworkManager.enable-disable-wifi",
    "org.freedesktop.NetworkManager.enable-disable-network",
    "org.freedesktop.NetworkManager.settings.modify.system",
    "org.freedesktop.NetworkManager.settings.modify.own",
    // "org.freedesktop.NetworkManager.sleep-wake" removed by R-1 (§11)
];

pub const MAX_RULE_FILE_BYTES: u64 = 16_384;   // read cap of an installed rule file
pub const MAX_PROC_ENTRIES: usize = 65_536;    // /proc directory entries examined
pub const MAX_COMM_BYTES: u64 = 64;            // /proc/<pid>/comm read cap
pub const MAX_SYSFS_DEV_BYTES: u64 = 32;       // /sys/class/misc/rfkill/dev read cap
pub const MAX_UDEV_DB_BYTES: u64 = 65_536;     // /run/udev/data/c<maj>:<min> read cap

/// Network stack detected from running processes (D-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkBackend { NetworkManager, Networkd, Iwd, Unknown }

/// Which network services run (seam output).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunningServices { pub network_manager: bool, pub networkd: bool, pub iwd: bool }

/// Injectable service-detection seam (replaces a D-Bus name-owner query, see D-6).
pub trait NetworkServiceProbe { fn running(&self) -> RunningServices; }

/// Production probe: scans `<root>/proc/<pid>/comm` (bounded, read-only).
#[derive(Debug, Clone)]
pub struct ProcServiceProbe { pub root: PathBuf }
impl NetworkServiceProbe for ProcServiceProbe { /* ... */ }

/// State of one installed rule file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleFileState { Installed, Missing, Modified, InsecurePermissions, Unreadable }

/// Seat-user access to /dev/rfkill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RfkillAccess { Uaccess, Restricted, Absent, Unknown }

/// Overall verdict of `network-lock status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkLockState {
    Installed, Missing, Modified, InsecurePermissions, Unreadable,
    UnsupportedBackend,   // rendered `unsupported_backend(<backend>)` in the table
}

/// Filesystem seam: everything is read below `root`; `required_owner_uid` is 0 in production.
#[derive(Debug, Clone)]
pub struct NetworkLockEnv { pub root: PathBuf, pub required_owner_uid: u32 }
impl NetworkLockEnv { pub fn system() -> Self /* root "/", owner 0 */; }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkLockReport {
    pub state: NetworkLockState,
    pub backend: NetworkBackend,
    pub rules: RuleFileState,
    pub rfkill_rule: RuleFileState,
    pub rfkill: RfkillAccess,
    pub warnings: Vec<&'static str>,   // only "rfkill_uaccess" today, fixed vocabulary
}
impl NetworkLockReport {
    pub fn exit_code(&self) -> i32;        // §4
    pub fn to_json(&self) -> String;       // serde_json, never fails on these types
    pub fn format_table(&self) -> String;  // fixed vocabulary + the two fixed paths
}

pub fn network_lock_status(env: &NetworkLockEnv, probe: &dyn NetworkServiceProbe) -> NetworkLockReport;
pub fn rule_file_state(env: &NetworkLockEnv, dir: &str, file: &str, expected: &str) -> RuleFileState;
pub fn rfkill_access(root: &Path) -> RfkillAccess;
pub fn detect_backend(services: RunningServices) -> NetworkBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallOptions { pub rfkill: bool }

/// What install/uninstall did to each file (printed by `main`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction { Written, Removed, Absent }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport { pub rules: FileAction, pub rfkill_rule: FileAction }

pub fn install_network_lock(root: &Path, options: InstallOptions, chown_root: bool)
    -> Result<InstallReport, AdminCliError>;
pub fn uninstall_network_lock(root: &Path) -> Result<InstallReport, AdminCliError>;

/// `true` when the call may proceed: a non-`/` root (tests, image builds) or euid 0.
pub fn may_modify_root(root: &Path, euid: u32) -> bool;
```

CLI (`args.rs`):

```rust
/// Opt-in polkit Wi-Fi lock (GitHub #365).
NetworkLock(NetworkLockArgs),

pub struct NetworkLockArgs { #[command(subcommand)] pub action: NetworkLockAction }
pub enum NetworkLockAction {
    /// Read-only diagnostic.
    Status { #[arg(long, default_value = "/")] root: PathBuf },
    /// Write the polkit rule (and the rfkill rule with --rfkill; without it an installed soos rfkill rule is removed).
    Install { #[arg(long)] rfkill: bool, #[arg(long, default_value = "/")] root: PathBuf },
    /// Remove both files.
    Uninstall { #[arg(long, default_value = "/")] root: PathBuf },
}
```

`--format json` (global) selects `to_json()` for `status`.

## 3. Rule files (D-1 … D-4)

### D-1 Action ids (OD-1: Wi-Fi change and disconnect, minimal set)

| Action id | Kept because |
|---|---|
| `org.freedesktop.NetworkManager.network-control` | activating, deactivating and switching connections (`nmcli con up/down`, `nmcli dev disconnect`, picking another Wi-Fi network); also required for hotspot activation |
| `org.freedesktop.NetworkManager.enable-disable-wifi` | the Wi-Fi radio switch (`nmcli radio wifi off`, the desktop Wi-Fi toggle) |
| `org.freedesktop.NetworkManager.enable-disable-network` | `nmcli networking off`, which disconnects Wi-Fi with every other interface |
| `org.freedesktop.NetworkManager.settings.modify.system` | creating or editing a system profile (a new Wi-Fi network from the desktop, a changed PSK or autoconnect flag); an autoconnect profile would otherwise let NetworkManager switch networks by itself without `network-control` |
| `org.freedesktop.NetworkManager.settings.modify.own` | the same through a user-owned profile ("only for this user") |
| ~~`org.freedesktop.NetworkManager.sleep-wake`~~ | *(removed by R-1, §11: its default is already `no` for every subject)* |

Excluded, with reason: `wifi.share.open` / `wifi.share.protected` (a hotspot activation is also checked against `network-control`, which is locked); `enable-disable-wwan`, `enable-disable-wimax` (not Wi-Fi); `settings.modify.hostname`, `settings.modify.global-dns`, `enable-disable-statistics`, `enable-disable-connectivity-check`, `wifi.scan` (no Wi-Fi change; DNS and hostname are out of scope per OD-1); `checkpoint-rollback`, `reload` (default already `auth_admin_keep`; widening is out of the minimal set); all `org.freedesktop.network1.*` (systemd-networkd does not associate with Wi-Fi networks — wpa_supplicant or iwd does — and its actions already default to `auth_admin_keep`); all `org.freedesktop.resolve1.*` (DNS, out of scope).

Accepted granularity: `network-control` and `settings.modify.*` are not per device type; Ethernet and VPN profile changes also prompt (polkit receives no device type for these actions). Documented.

### D-2 Rule semantics (OD-2)

`polkit.addRule(function (action, subject) { var soosNetworkLockActions = [ ...5 ids (R-1)... ]; if (soosNetworkLockActions.indexOf(action.id) >= 0) { return polkit.Result.AUTH_ADMIN; } });`

- For any subject (the rule never reads `subject`): an active local session gets an administrator prompt on every request (no 5-minute retention). Root callers are never affected: NetworkManager authorizes uid 0 without polkit, and polkitd authorizes uid 0 subjects.
- Every other action: the function returns `undefined` (falls through to the next rule or the shipped default).
- Never `YES`, `NO`, `AUTH_SELF*`, `AUTH_ADMIN_KEEP`, `NOT_HANDLED`; exactly one `return`; no `addAdminRule`.
- **Equivalent-privilege note (records a deviation from the issue's "only tightens")**: for a subject whose shipped default is `no` (a remote or inactive subject for `enable-disable-wifi` on Ubuntu), the result becomes an administrator challenge. Answering it requires a password of an administrator identity (`wheel`/`sudo`), which already grants the same action through `sudo nmcli`; the rule therefore never authorizes anything without administrator authentication and never grants more than the administrator already holds. The invariant is phrased "never authorizes without administrator authentication, never denies outright".
- File name `49-…`: polkit evaluates the rules of `/etc/polkit-1/rules.d` and `/usr/share/polkit-1/rules.d` in lexical file-name order, first result wins. `49-soos-network-lock.rules` runs before `50-default.rules` and before Ubuntu's `org.freedesktop.NetworkManager.rules` (which returns `YES` for `settings.modify.system` to local active `sudo`/`netdev` members). A distribution or admin rule that sorts **before** `49-soos` and returns a result for the same action still wins; documented as a limitation (the status does not parse foreign rules).
- A malformed rules file makes polkitd log an error and ignore it (fall back to defaults); `status` reports `modified` for any byte difference.

### D-3 Placement

`/etc/polkit-1/rules.d/49-soos-network-lock.rules` (administrator configuration, survives package upgrades, removed by uninstall). Distribution packages ship nothing active: the content is embedded in `soos-admin` (`include_str!`), so packaging needs no change.

### D-4 rfkill option (OD-3)

`packaging/udev/72-soos-network-lock-rfkill.rules`, single rule `SUBSYSTEM=="misc", KERNEL=="rfkill", TAG-="uaccess"` (`-=` on `TAG` since systemd 217). Name `72-` sorts after `70-uaccess.rules` and GNOME's `61-gnome-settings-daemon-rfkill.rules` (both add the tag) and before `73-seat-late.rules` (which applies the ACL for tagged devices). Installed only with `install --rfkill`; takes effect after a reboot (an existing ACL is not revoked by a reload). Disables the desktop airplane-mode toggle for the seat user. Gaps that remain even with it: membership of the group that owns `/dev/rfkill` (Arch `60-rfkill.rules`: `GROUP="rfkill", MODE="0664"`), hardware kill switches, cable removal, root, power-off.

## 4. Status semantics (D-6)

Order of evaluation (no step writes anything):

1. `rules = rule_file_state(env, NETWORK_LOCK_RULES_DIR, NETWORK_LOCK_RULES_FILE, NETWORK_LOCK_RULES)`; `rfkill_rule` likewise for the udev file.
2. `rule_file_state`:
   - `symlink_metadata(<root>/<dir>)`: `NotFound` → `Missing`; `PermissionDenied` → `Unreadable`; a symlink or non-directory → `InsecurePermissions`; owner ≠ `required_owner_uid` or `mode & 0o022 != 0` → `InsecurePermissions`.
   - `symlink_metadata(<file>)`: `NotFound` → `Missing`; `PermissionDenied` → `Unreadable` (Arch: `/etc/polkit-1/rules.d` is `0750 root:polkitd`, so a non-root caller always gets `unreadable`; the table prints "run as root"); symlink or non-regular → `InsecurePermissions`; owner ≠ `required_owner_uid` or `mode & 0o022 != 0` → `InsecurePermissions`.
   - Content: open with `O_NOFOLLOW | O_CLOEXEC`, read at most `MAX_RULE_FILE_BYTES + 1` bytes; open/read `PermissionDenied` → `Unreadable`, other I/O error → `Unreadable`; longer than the cap → `Modified`; SHA-256 of the bytes ≠ SHA-256 of the embedded content → `Modified`; else `Installed`.
3. `backend = detect_backend(probe.running())`: NetworkManager if running (it may use iwd or wpa_supplicant underneath; polkit applies) → else `Iwd` if iwd runs → else `Networkd` → else `Unknown`.
4. `rfkill = rfkill_access(root)`:
   - `<root>/sys/class/misc/rfkill/dev` missing → `Absent`; unreadable, longer than `MAX_SYSFS_DEV_BYTES`, or not `<u32>:<u32>` → `Unknown`.
   - `<root>/dev/rfkill`: if `symlink_metadata` succeeds and `mode & 0o002 != 0` (world-writable) → `Uaccess`.
   - `<root>/run/udev/data/c<major>:<minor>` (≤ `MAX_UDEV_DB_BYTES`, else `Unknown`; missing → `Unknown`): tag set = the `Q:` lines (current tags, systemd ≥ 247) when any exists, else the `G:` lines; contains `uaccess` → `Uaccess`, else `Restricted`.
   - `Uaccess` adds the warning `rfkill_uaccess`.
5. `state` precedence: `rules` `InsecurePermissions` → `Modified` → `Unreadable` → backend `Iwd`/`Networkd` → `UnsupportedBackend` → `rules` `Installed` → `Installed`, else `Missing`.
6. `exit_code()`: `1` when `rules` or `rfkill_rule` is `Modified`, `InsecurePermissions` or `Unreadable`; else `0` (`missing`, `installed`, `unsupported_backend` and the rfkill warning exit 0: the lock is opt-in and the gap is documented).

Sentinels: empty rule file → `Modified`; `--root` empty string → clap rejects nothing, `Path::new("")` joins relative; `main` refuses an empty root with an error. `required_owner_uid` is never configurable from the CLI (always 0).

Output vocabulary (table and JSON): the enum names above, `rfkill_uaccess`, the two fixed install paths, and fixed explanatory sentences. **No** SSID, BSSID, PSK, connection name or UUID, IP address, interface name or process id is ever read or printed (the probe reads only `comm`; nothing under `/etc/NetworkManager` is opened).

## 5. Install / uninstall semantics (D-5, OD-6)

`main`: refuses (`AdminCliError::NetworkLock`, exit 1) when `!may_modify_root(root, euid)` ("run as root, or pass --root for a staging directory") and when `root` is empty. `chown_root = euid == 0`.

`install_network_lock(root, options, chown_root)`:
1. Writes the polkit file with `write_rule_file`; then, if `options.rfkill`, writes the udev file, else removes an existing soos rfkill file (declarative: the files on disk match the options; #362 re-runs it when the option changes).
2. `write_rule_file(root, dir, file, content, chown_root)`:
   - every component of `<root>/<dir>` below `root` must exist as a real directory (`symlink_metadata`, not a symlink); a missing directory is an error ("polkit is not installed" / "udev rules directory missing") — the installer never creates `/etc/polkit-1/rules.d` with guessed permissions;
   - the final directory is opened with `O_DIRECTORY | O_NOFOLLOW` and its `(dev, ino)` must equal the `symlink_metadata` of the path;
   - an existing target that is a symlink or not a regular file is refused (nothing written, the link target untouched);
   - temp file `<dir>/.<file>.<pid>.tmp` created with `create_new(true)`, `O_NOFOLLOW | O_CLOEXEC`, mode `0600`; content written; `fchmod 0644`; `fchown(0, 0)` when `chown_root`; `sync_all`; `rename` onto the target (rename never follows a symlink at the destination); directory `fsync`; on any error the temp file is removed. The temp name does not end in `.rules`, so neither polkitd nor udev loads it.
3. `uninstall_network_lock(root)`: for each file, refuses a symlinked directory component (same check as install); `NotFound` → `Absent`; a directory at the name → error; else `remove_file` (unlinks a symlink itself, never its target) → `Removed`.
4. `scripts/uninstall.sh` removes both files under `${DESTDIR}${SYSCONFDIR}` (only when the parent directory is not a symlink), so a full uninstall leaves no lock behind even when `soos-admin` is already gone.

Root-owned: the file is root:root `0644` when run as root (the production path). In a staging root run by a non-root user the owner is the caller; `status` then reports `insecure_permissions`, which the shell test asserts.

## 6. Error Taxonomy

| Condition | Result |
|---|---|
| install/uninstall: unsafe path, missing dir, symlink, non-regular target, I/O error | `AdminCliError::NetworkLock(<fixed message + path>)`, exit 1, nothing half-written (temp removed) |
| install on `/` without euid 0 | `AdminCliError::NetworkLock("must run as root ...")`, exit 1, nothing touched |
| status: any read error | mapped to a state (`Unreadable`, `Unknown`, `Missing`), never an `Err`; exit code per §4 |

No PAM path, no daemon, no IPC: the PAM fail-closed invariants are untouched.

## 7. Test Hooks for Tester

- `NetworkLockEnv { root, required_owner_uid }` (temp root fixture; owner = `geteuid()` of the test).
- `NetworkServiceProbe` fake returning any `RunningServices`; `ProcServiceProbe { root }` over a fixture `proc/<pid>/comm` tree.
- `install_network_lock(root, opts, chown_root = false)` / `uninstall_network_lock(root)` on a temp root.
- `may_modify_root(root, euid)` pure function.
- The `soos-admin` binary (`CARGO_BIN_EXE_soos-admin`) driven by a bash test script against a temp root, plus `scripts/uninstall.sh --destdir`.
- Docker: `tests/docker/network_lock_pkcheck_test.sh` (D-8).

Planned tests (names are the contract; prefix `test_nlk_`):

| Area | Tests |
|---|---|
| Static (`soos-invariants::network_lock_contract`) | rule lists exactly the documented ids (rule file = `NETWORK_LOCK_ACTION_IDS` source = `Docs/NETWORK_LOCK.md` table); never grants/denies/keeps; single `return` inside the `indexOf` guard, no `else`, no `subject.` read; rfkill rule is the single `TAG-="uaccess"` line and sorts between 70 and 73; contents embedded by `include_str!` from `packaging/`; `scripts/install.sh` does not install the lock (opt-in); `scripts/uninstall.sh` removes both files; CI job wired into `ci-success`; Docker script pinned image; docs/ADR/walkthrough |
| Status (`crates/admin-cli/tests/network_lock_status_tests.rs`) | installed; missing; modified (one byte, empty, over cap); insecure (group-writable, world-writable, wrong owner, symlink file, symlinked dir, writable dir); unreadable (dir `0000`, skipped as root); unsupported backend iwd and networkd; backend precedence; proc probe (comm match, truncated `systemd-network`, non-numeric entries, oversize comm); rfkill (Q tag, G-only legacy, Q without uaccess but G with it, world-writable node, absent, malformed dev, missing udev db); exit codes; JSON shape; output has no network names |
| Install (`crates/admin-cli/tests/network_lock_install_tests.rs`) | writes 0644 identical content, no temp left; replaces a modified file; refuses symlink target (target untouched); refuses symlinked parent; refuses missing dir; refuses directory target; `--rfkill` writes the udev rule; install without `--rfkill` removes it; uninstall removes both and is idempotent; uninstall refuses symlinked parent and unlinks a symlink target without touching its destination; `may_modify_root`; CLI parsing |
| Shell (`crates/admin-cli/tests/network_lock_install_test.sh` via `network_lock_shell_tests.rs`) | binary install/status/uninstall in a temp root, symlink refusal, `scripts/uninstall.sh --destdir` removal |
| Docker (`tests/docker/network_lock_pkcheck_test.sh`, CI job `network-lock-polkit`) | real polkitd: with the rule every listed action is an admin challenge with no retention, controls identical to the baseline, root authorized, the rule changes at least one baseline result |

## 8. D-8 Docker pkcheck test: feasibility and limits

Prototyped on 2026-10-13 in the pinned `ubuntu:24.04@sha256:0081…` image (`polkitd 124`, `network-manager 1.46`): `dbus-daemon --system --fork`, `/usr/lib/polkit-1/polkitd --no-debug &`, `pkcheck --process $$` as an unprivileged user. Baseline: `network-control` rc 2 (challenge), `enable-disable-wifi` rc 1, `settings.modify.system` rc 2 with `polkit.retains_authorization_after_challenge=1`; with the rule all six rc 2 without retention; `settings.modify.hostname` unchanged; root rc 0. The test runs as a new CI job (same image digest as `pam_rollback_test.sh`, packages installed at run time, about 1–2 minutes).

Limit: a container has no seat, so the subject is never "active local" (polkit classifies seatless sessions as non-local and uses `allow_any`). The acceptance wording "for an active local subject" is covered by (a) the rule never reading `subject` (static invariant), so the result is the same for every subject class, and (b) the owner hardware check NLK row on the Arch host with NetworkManager.

## 9. Invariants Touched

- AGENTS: no `unwrap`/`expect` in production code; `#![forbid(unsafe_code)]` in admin-cli (unchanged); English only; no secrets logged (none read).
- Packaging ownership (`packaging_ownership_contract`): files under `/etc` written only by the explicit opt-in command; uninstall removes them.
- RC-1 and the remote companion: untouched.
- PAM: untouched; face is never used for polkit prompts (ADR "Local Session Binding" hardware note).

## 10. Documentation Drift / ADR

- New ADR "[2026-10-13] Opt-In polkit Wi-Fi Lock and `soos-admin network-lock`" recording OD-1…OD-6, D-1…D-8, the equivalent-privilege deviation (D-2), the D-Bus → `/proc` probe deviation (D-6: `soos-admin` has no D-Bus dependency; reading `comm` is read-only, bounded and dependency-free; an injectable trait keeps the seam), the `unreadable` state added to the issue's state list, and the Docker subject-class limit (D-8).
- `Docs/NETWORK_LOCK.md` (new): what is locked (table of D-1), install/uninstall/status commands, AUTH_ADMIN prompts for the wheel user's own password like `sudo`, how to add a separate admin account (create an account in `wheel`, remove the daily user from `wheel`, keep `sudo` working through that account; polkit's `50-default.rules` `addAdminRule` defines admins as `wheel` on Arch and `sudo` on Ubuntu), face not used and why, gaps (rfkill and its group, iwd-only, networkd + wpa_supplicant, hardware switch, cable, root, power-off, distribution rules sorting before `49-`), rfkill option and reboot.

## 11. Revision 1 (plan evaluation round 1, `AI/plan_evaluator_report.md` F-1 … F-10)

This section supersedes the earlier sections wherever they differ.

- **R-1 (F-1, MAJOR): `sleep-wake` dropped.** Its shipped default is `no` for every subject (verified in the host's `/usr/share/polkit-1/actions/org.freedesktop.NetworkManager.policy`, NetworkManager 1.58), so `AUTH_ADMIN` would make it weaker. The D-1 rationale for it was wrong. `NETWORK_LOCK_ACTION_IDS` becomes `[&str; 5]`: `network-control`, `enable-disable-wifi`, `enable-disable-network`, `settings.modify.system`, `settings.modify.own`. Excluded list gains "`sleep-wake`: default already `no` for every subject". Selection criterion (ADR): an action is listed only if its shipped default for the **active local** subject is weaker than `auth_admin` (or a distribution rule makes it `YES`) and it changes or disconnects Wi-Fi. Arch defaults for the active subject: `network-control` yes, `enable-disable-wifi` yes, `enable-disable-network` yes, `settings.modify.own` yes, `settings.modify.system` `auth_admin_keep` plus Arch's `org.freedesktop.NetworkManager.rules` `YES` for local `wheel` members (Ubuntu: `YES` for local active `sudo`/`netdev`). The widening for inactive or remote subjects whose default is `no` (`enable-disable-wifi`, `enable-disable-network`) stays the documented equivalent-privilege deviation of D-2.
- **R-2 (F-2, MAJOR): Docker sentinel differential.** Inside the container only, a test rule `/etc/polkit-1/rules.d/90-soos-test-sentinel.rules` returns `polkit.Result.YES` for every `org.freedesktop.NetworkManager.*` action. With the sentinel and without the soos rule, every NetworkManager action listed by `pkaction` is authorized (rc 0). With both, each locked id is an unretained challenge (`2 plain`) and every other NetworkManager action id stays rc 0 (exact list, fall-through, guard index 0 covered); root rc 0. The real-default comparison (sentinel removed) stays as the secondary check (P-checks of the first version). The static invariant pins the exact guard `if (soosNetworkLockActions.indexOf(action.id) >= 0) {`, exactly one `return`, and that it is `return polkit.Result.AUTH_ADMIN;` (pkcheck cannot tell `AUTH_ADMIN` from `AUTH_SELF`; the invariant does).
- **R-3 (F-3): warning `iwd_dbus_bypass`.** When NetworkManager and iwd both run, `backend` stays `network_manager` and the fixed warning `iwd_dbus_bypass` is added (exit code unchanged): iwd's D-Bus policy admits root and the `netdev` group, which can drive iwd directly. Docs also name Debian's wpa_supplicant D-Bus policy (`netdev`); Arch's admits root only.
- **R-4 (F-4): warning `rfkill_reboot_pending`.** When `rfkill_rule` is `installed` and the file's `ctime` is later than boot time (`now - uptime`, uptime = first field of `<root>/proc/uptime`, read ≤ `MAX_UPTIME_BYTES` = 64, a finite non-negative number), the warning `rfkill_reboot_pending` is added: a pre-reboot ACL may still grant access although the udev tag is gone. An unreadable or malformed uptime adds no warning (informational only). The rfkill group membership count is not implemented (documented gap).
- **R-5 (F-5): gap list** extended in `Docs/NETWORK_LOCK.md`: root-equivalent groups (`docker`, `lxd`, `libvirt`), the `sudo` timestamp cache, `run0` / `systemctl stop NetworkManager` within a retained `auth_admin_keep` authorization, suspend through logind, `auth-polkit=false` in `NetworkManager.conf`, distribution rules sorting before `49-` (`20-gdm.rules` answers `settings.modify.system` with `YES` for the `gdm` greeter user only), `empower.rules` (sorts after `49-`, not an override for the listed ids).
- **R-6 (F-6): install enforces what status checks, directory-relative.** Every directory component below `root` (`etc`, `etc/polkit-1`, `etc/polkit-1/rules.d`; likewise `etc/udev/rules.d`) must be a real directory owned by the required owner (uid 0 when `chown_root`, else the caller's euid) and not group- or world-writable; otherwise install and uninstall refuse. The final directory is opened `O_DIRECTORY | O_NOFOLLOW` and matched by `(dev, ino)`; then the target type check (`fstatat(dirfd, name, AT_SYMLINK_NOFOLLOW)`), the rename (`renameat(dirfd, tmp, dirfd, name)`), temp cleanup and uninstall removal (`unlinkat(dirfd, …)`) are relative to that descriptor. The temp file is created by path with `create_new` + `O_NOFOLLOW` (`nix::fcntl::openat` returns a raw descriptor that cannot be wrapped without `unsafe`, forbidden in this crate) and is then proven to live in the verified directory: `fstatat(dirfd, tmp, AT_SYMLINK_NOFOLLOW)` must have the same `(dev, ino)` as `fstat` of the open temp file, else install aborts and unlinks it.
- **R-7 (F-7): atomic replace tested.** New install tests: a target that is a hard link to a file outside the directory is replaced by a new inode and the outside file keeps its content; a stale temp file with the exact temp name makes install fail cleanly with the target untouched.
- **R-8 (F-8): removal paths.** (a) `scripts/uninstall.sh` removes both files right after the systemd teardown (step 1) and before `# 2. Roll Back PAM`, so no later `exit 1` or `set -e` abort can skip it; the invariant asserts the order. (b) Package removal: the packages never install the lock, and their removal hooks are not changed in this PR; `Docs/NETWORK_LOCK.md` and the ADR say to run `sudo soos-admin network-lock uninstall` before removing the package. Reason: the lock is installed only by an explicit administrator command; a leftover file keeps the lock active (fail-safe direction) and can be deleted by hand; hooking the three package formats belongs with #362 (one-command installer) and the packaging follow-up, where the package harnesses run.
- **R-9 (F-9): harness conventions.** The workspace is mounted read-only (`:ro`), so the `/workspace/target` volume overlay (POA3) does not apply; the default image is `SOOS_DEBIAN_IMAGE:-ubuntu:24.04@sha256:0081…` (digest-pinned, asserted by the new invariant instead of extending the existing POA9 list, which stays unchanged); the CI job uses no `${{ github.event` value, `persist-credentials: false`, a SHA-pinned checkout and is appended to `ci-success.needs`.
- **R-10 (F-10):** non-root `status` on Arch reports `unreadable` (exit 1) with "run as root"; recorded in the ADR together with the `unreadable` state added to the issue's list.

Updated report: `warnings` vocabulary is `rfkill_uaccess`, `rfkill_reboot_pending`, `iwd_dbus_bypass` (fixed order). New constant `MAX_UPTIME_BYTES: u64 = 64`.
