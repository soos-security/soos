# Architect Spec — GitHub #379: first-run onboarding tour with per-feature toggles in `soos-gui`

- **Date**: 2026-10-13
- **Issue**: GitHub-only #379 (PR 1 of epic #363); no `AI/BACKLOG.md` id.
- **Branch**: `feat/onboarding-tour` (not registered in `scripts/sync_issue.py`; the commit carries `Closes #379`, `Refs #363`).
- **Base commit**: `6f32ef8` (`origin/main`).
- **Walkthrough**: 209 (number assigned by the orchestrator for this run; other parallel branches hold 200–208).
- **Owner decisions** (comments on #362, #358 and #363):
  - OD-1 (#362, 2026-10-12) The install enables everything by default (presence auto-unlock, phone companion, push alerts, Funnel, camera view). Each feature can be turned off later **in the GUI** and is explained by the onboarding tour.
  - OD-2 (#358, 2026-10-12) The tour is not childish; it explains every feature and lets the user turn each one off. Presence auto-unlock must be toggleable in the GUI at any time.
  - OD-3 (#358, 2026-10-13) Motion is a separate stream (#377 spec, `feat/gui-motion`); GUI reduced motion comes from the XDG portal plus `--reduce-motion` / `SOOS_REDUCE_MOTION`. This PR adds no animation, only a hook.
  - OD-4 (#363) The off switch applies through the existing privileged paths (`pkexec` / admin CLI), never by editing root files from the GUI process.
- **Assumed — owner to confirm** (open points this PR has to settle; conservative option taken, restated in the ADR):
  - AS-1 `soos-remote` keys (`allow_unlock`, `allow_funnel`, `push_notifications`, `camera_view`) can be turned **off** from the GUI (the safe direction). Turning one back **on** shows the exact key and restart command instead of a switch, because each widens remote exposure and has prerequisites (`rp_id`, a passkey, Tailscale policy) that only `soos-remote` validates at start-up. A GUI "on" switch can follow once the #362 bounded TOML editor and a `soos-remote --check-config` exist.
  - AS-2 Root-owned flags (`/etc/soos/disabled`, `/etc/soos/presence.disable`) are toggled through `pkexec /usr/bin/touch <flag>` and `pkexec /usr/bin/rm -f -- <flag>` with a closed set of two paths, until a dedicated `soos-admin` subcommand exists. The Wi-Fi lock uses `pkexec <bindir>/soos-admin network-lock install|uninstall` (existing admin CLI, never `--rfkill`).
  - AS-3 "Offline alerts" (#376) is not merged: its step explains the feature and shows "Not available in this version"; no control.
  - AS-4 Tour state file `$XDG_CONFIG_HOME/soos/gui-tour.json` (else `$HOME/.config/soos/gui-tour.json`), next to `remote.toml`.

## 1. Scope & Blast Radius

| Area | Change |
|---|---|
| `crates/gui/src/onboarding.rs` | **new**, pure: tour page state machine, feature catalogue and wording, `FeatureSnapshot` → `FeatureState`, toggle routing, per-user tour state store |
| `crates/gui/src/feature_control.rs` | **new**: state probes, fixed argument vectors, `remote.toml` reader and off-only editor, `FeatureExecutor` trait, `SystemFeatureExecutor`, `FeatureRunner` (one action at a time, background thread) |
| `crates/gui/src/onboarding_view.rs` | **new**: egui rendering of the tour modal; returns `TourEvent`s, never runs I/O |
| `crates/gui/src/privileged.rs` | **add** `SOOS_ADMIN_PROGRAM` (same `cfg(soos_custom_bindir)` scheme as `SOOS_ENROLL_PROGRAM`), `TOUCH_PROGRAM`, `RM_PROGRAM`; existing items untouched |
| `crates/gui/src/header.rs` | **add** `features_button` (pale outline pill in the band) |
| `crates/gui/src/app.rs` | wire the tour, the runner and the header button; daemon switch busy also while a feature action runs |
| `crates/gui/src/lib.rs` | `pub mod onboarding; pub mod feature_control; pub mod onboarding_view;` |
| `crates/gui/Cargo.toml` | add `toml = { workspace = true }` (0.8, already in `Cargo.lock` through the daemon and remote; no new crate) |
| `crates/gui/tests/onboarding_*_tests.rs` | **new** tests (§7) |
| Docs | `Docs/GUI_APPLICATION.md` new §; `Docs/REMOTE_COMPANION.md` and `Docs/NETWORK_LOCK.md` one pointer each |
| Traceability | ADR, matrix section `OBT*`, walkthrough 209 |

Untouched: `PrivilegedAction` / `PrivilegedOutcome` / `PrivilegedExecutor` (the exhaustive matches in `responsiveness_tests.rs` and `camera_source_tests.rs` are immutable contracts, so feature toggles go through a separate runner, the precedent of `StoreTaskRunner`), the daemon, PAM, IPC, `soos-remote`, `soos-admin`, installers, packaging.

## 2. Types & Signatures

### 2.1 `onboarding.rs` (pure except `TourStore`)

```rust
pub const TOUR_VERSION: u32 = 1;                       // bump to show the tour again after new features
pub const MAX_TOUR_STATE_BYTES: u64 = 4096;            // larger file = treated as not seen (overwritten)
pub const TOUR_STATE_FILE_NAME: &str = "gui-tour.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature { FaceUnlock, PresenceUnlock, PhoneCompanion, RemoteUnlock,
                   InternetAccess, PushAlerts, CameraView, WifiLock, OfflineAlerts }
pub const FEATURES: [Feature; 9];                      // tour order = this order
impl Feature {
    pub fn title(self) -> &'static str;                // e.g. "Presence auto-unlock"
    pub fn explanation(self) -> &'static str;          // 2–4 plain sentences: what, when, risk
    pub fn when_off(self) -> &'static str;             // one sentence: what turning it off changes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureState { On, Off, NotInstalled, NotAvailable, Unknown }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitState { Unknown, NotInstalled, Active, Inactive }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemoteKeys { pub allow_unlock: bool, pub allow_funnel: bool, pub password_alerts: bool,
                        pub push_notifications: bool, pub camera_view: bool, pub camera_view_funnel: bool }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteConfigState { Unknown, Missing, Invalid, Keys(RemoteKeys) }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureSnapshot {
    pub global_disabled: Option<bool>,     // `/etc/soos/disabled` exists (None = stat error)
    pub presence_disabled: Option<bool>,   // `/etc/soos/presence.disable` exists
    pub network_lock_installed: Option<bool>,
    pub remote_unit: UnitState,            // `soos-remote.service` in the user manager
    pub remote_config: RemoteConfigState,
    pub running_as_root: bool,             // euid 0: user-level controls become commands
}
impl FeatureSnapshot {
    pub const fn unknown() -> Self;        // every field Unknown/None, running_as_root false
    pub fn state(&self, feature: Feature) -> FeatureState;
}
```

`state` rules (table-tested):

| Feature | On | Off | NotInstalled | Unknown |
|---|---|---|---|---|
| FaceUnlock | `global_disabled == Some(false)` | `Some(true)` | — | `None` |
| PresenceUnlock | both flags `Some(false)` | either flag `Some(true)` | — | otherwise |
| PhoneCompanion | `Active` | `Inactive` | `NotInstalled` | `Unknown` |
| RemoteUnlock / InternetAccess / PushAlerts / CameraView | key `true` | key `false` | unit `NotInstalled` or config `Missing` | config `Unknown`/`Invalid` (unit installed or unknown) |
| WifiLock | `Some(true)` | `Some(false)` | — | `None` |
| OfflineAlerts | — | — | — | always `NotAvailable` |

Keys: RemoteUnlock → `allow_unlock`, InternetAccess → `allow_funnel`, PushAlerts → `push_notifications`, CameraView → `camera_view`.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToggleRoute {
    Action(FeatureAction),        // applied in the GUI through FeatureRunner
    Command(String),              // exact command / key edit shown with a Copy button
    Unavailable(&'static str),    // reason shown, no control
}
pub fn toggle_route(snapshot: &FeatureSnapshot, feature: Feature, turn_on: bool) -> ToggleRoute;
```

Routing rules:

| Feature | turn_on = false | turn_on = true |
|---|---|---|
| FaceUnlock | `Action(SetSystemFlag { Global, present: true })` | `Action(SetSystemFlag { Global, present: false })` |
| PresenceUnlock | `Action(SetSystemFlag { Presence, present: true })` | `Action(SetSystemFlag { Presence, present: false })` |
| PhoneCompanion | root: `Command("systemctl --user disable --now soos-remote.service")`; `NotInstalled`: `Unavailable`; else `Action(SetCompanion { enabled: false })` | same with `enable --now` / `enabled: true` |
| Remote keys | root or config not `Keys`: `Command(<key = false edit + restart>)`; else `Action(DisableRemote(<switch>))` | always `Command(<key = true edit + restart>)` (AS-1) |
| WifiLock | `Action(SetNetworkLock { installed: false })` | `Action(SetNetworkLock { installed: true })` |
| OfflineAlerts | `Unavailable` | `Unavailable` |

Command texts name the file as `~/.config/soos/remote.toml` and end with `systemctl --user restart soos-remote.service`. They contain no user data.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourPage { Intro, Feature(Feature), Overview }
pub const TOUR_PAGE_COUNT: usize = FEATURES.len() + 2;  // Intro, 9 features, Overview

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tour { open: bool, index: usize, generation: u64 }
impl Tour {
    pub fn closed() -> Self;              // not shown
    pub fn first_run() -> Self;           // open on Intro
    pub fn is_open(&self) -> bool;
    pub fn index(&self) -> usize;         // 0..TOUR_PAGE_COUNT
    pub fn page(&self) -> TourPage;
    pub fn next(&mut self);               // saturates at Overview
    pub fn back(&mut self);               // saturates at Intro
    pub fn restart(&mut self);            // open on Intro
    pub fn open_overview(&mut self);      // open on Overview (header "Features" button)
    pub fn close(&mut self);              // skip / finish / Escape
    pub fn generation(&self) -> u64;      // motion hook: +1 (wrapping) on every page or open change
}
```

Per-user state:

```rust
pub fn tour_state_path(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf>;
// Absolute non-empty XDG first, else absolute HOME + ".config"; relative or empty values ignored;
// None = no persistence: the app uses TourLoad::Unavailable (never auto-shown).
pub fn parse_tour_state(bytes: &[u8]) -> Option<u32>;  // {"version":N,"seen":true} -> Some(N); else None
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourLoad { NotSeen, Seen { version: u32 }, Unavailable }
pub fn should_show_tour(load: TourLoad) -> bool;       // NotSeen -> true; Seen{v} -> v < TOUR_VERSION; Unavailable -> false
pub struct TourStore { path: PathBuf }
impl TourStore {
    pub fn new(path: PathBuf) -> Self;
    pub fn load(&self) -> TourLoad;
    pub fn mark_seen(&self) -> Result<(), TourStoreError>;
}
#[derive(Debug, thiserror::Error)]
pub enum TourStoreError { #[error("refusing to write through a symbolic link")] Symlink,
                          #[error("{0}")] Io(#[from] std::io::Error) }
```

- `load`: `symlink_metadata`: `NotFound` → `NotSeen`; symlink or not a regular file → `Unavailable`; other error → `Unavailable`; size > `MAX_TOUR_STATE_BYTES` → `NotSeen`; read bounded by `take(MAX + 1)`; `parse_tour_state` `None` → `NotSeen`.
- No path (`tour_state_path` = `None`) → the app uses `TourLoad::Unavailable` (never shown automatically, still reachable from the header).
- `mark_seen`: create the parent with mode 0700 if missing (`DirBuilder::recursive`); refuse when the target is a symlink; write `{"version":TOUR_VERSION,"seen":true}\n` to `<file>.tmp-<pid>-<16 hex random>` opened `create_new` + mode 0600, `sync_all`, `rename` over the target; remove the temp file on any error. Runs on a short-lived `soos-gui-tour-state` thread (fire and forget, failure logged at `warn` with the error kind only).

### 2.2 `feature_control.rs`

```rust
pub const SOOS_ETC_DIR: &str = "/etc/soos";
pub const GLOBAL_DISABLE_FLAG: &str = "disabled";            // mirrors soos-daemon presence::GLOBAL_DISABLE_FLAG
pub const PRESENCE_DISABLE_FLAG: &str = "presence.disable";  // mirrors soos-daemon presence::PRESENCE_DISABLE_FLAG
pub const NETWORK_LOCK_RULE_PATH: &str = "/etc/polkit-1/rules.d/49-soos-network-lock.rules"; // mirrors soos-admin constants
pub const REMOTE_UNIT: &str = "soos-remote.service";
pub const MAX_REMOTE_CONFIG_BYTES: u64 = 16_384;             // mirrors soos-remote MAX_CONFIG_BYTES
pub const MAX_UNIT_SHOW_BYTES: usize = 1024;
pub const USER_SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemFlag { Global, Presence }            // closed set: the only root paths ever touched
impl SystemFlag { pub fn path(self) -> PathBuf; }   // SOOS_ETC_DIR joined with the flag name

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSwitch { Unlock, Funnel, Push, CameraView }
impl RemoteSwitch {
    pub fn key(self) -> &'static str;                 // allow_unlock / allow_funnel / push_notifications / camera_view
    pub fn keys_to_clear(self) -> &'static [&'static str];
    // Unlock: [allow_unlock]; Funnel: [allow_funnel, camera_view_funnel];
    // Push: [push_notifications]; CameraView: [camera_view, camera_view_funnel]
    // (soos-remote refuses camera_view_funnel without camera_view and allow_funnel)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureAction {
    Refresh,
    SetSystemFlag { flag: SystemFlag, present: bool },
    SetCompanion { enabled: bool },
    DisableRemote(RemoteSwitch),
    SetNetworkLock { installed: bool },
}
impl FeatureAction { pub fn is_privileged(self) -> bool; } // SetSystemFlag | SetNetworkLock

#[derive(Debug)]
pub enum FeatureOutcome {
    Refreshed(FeatureSnapshot),
    Applied { action: FeatureAction, result: Result<(), String> },
}

// Fixed argument vectors (no shell; program by absolute path):
pub fn system_flag_command(flag: SystemFlag, present: bool) -> (&'static str, Vec<String>);
//  present: (PKEXEC_PROGRAM, [TOUCH_PROGRAM, <path>]); absent: (PKEXEC_PROGRAM, [RM_PROGRAM, "-f", "--", <path>])
pub fn network_lock_command(installed: bool) -> (&'static str, Vec<String>);
//  (PKEXEC_PROGRAM, [SOOS_ADMIN_PROGRAM, "network-lock", "install"|"uninstall"]) — never "--rfkill", never "--root"
pub fn companion_command(enabled: bool) -> (&'static str, Vec<String>);
//  (SYSTEMCTL_PROGRAM, ["--user", "enable"|"disable", "--now", REMOTE_UNIT])
pub fn companion_restart_command() -> (&'static str, Vec<String>);
//  (SYSTEMCTL_PROGRAM, ["--user", "try-restart", REMOTE_UNIT])
pub fn unit_show_command() -> (&'static str, Vec<String>);
//  (SYSTEMCTL_PROGRAM, ["--user", "show", "--property=LoadState", "--property=ActiveState", REMOTE_UNIT])
pub fn parse_unit_show(text: &str) -> UnitState;
//  LoadState=not-found -> NotInstalled; ActiveState in {inactive, failed} -> Inactive;
//  ActiveState in {active, activating, reloading, refreshing, deactivating} -> Active; else Unknown

pub fn remote_config_path(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf>; // same rule as soos-remote
pub fn parse_remote_keys(text: &str) -> RemoteConfigState;   // toml::Table; missing key = false; non-bool = Invalid; parse error = Invalid
pub fn read_remote_config(path: &Path) -> RemoteConfigState; // NotFound -> Missing; symlink/non-file/oversize/io -> Invalid; bounded read

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoteEditError {
    #[error("remote.toml uses a layout the GUI does not edit (tables or inline values)")] UnsupportedLayout,
    #[error("remote.toml sets `{0}` to a value that is not true or false")] UnsupportedValue(&'static str),
    #[error("remote.toml is not valid TOML")] Invalid,
    #[error("the edited remote.toml did not verify")] VerifyFailed,
}
pub fn disable_remote_keys(text: &str, keys: &[&'static str]) -> Result<Option<String>, RemoteEditError>;
```

`disable_remote_keys` (pure): the original must parse as a TOML table; any line whose first non-blank char is `[` → `UnsupportedLayout`; a line `<ws>key<ws>=<ws>value<ws>[# comment]` for a target key: value `true` → line replaced by `<ws>key = false` (+ the original comment), value `false` → kept, any other value → `UnsupportedValue`. A key set twice is invalid TOML, so the initial parse already refuses it (`Invalid`). Commented-out lines (`# key = ...`) are ignored. Absent keys stay absent (`soos-remote` defaults are `false`). `Ok(None)` when nothing changes. Verification: the new text parses, every target key reads `false` or absent, and every other key of the table equals the original (`VerifyFailed` otherwise). Never adds a key, never touches another line.

```rust
pub fn write_user_file_atomically(path: &Path, contents: &str) -> Result<(), String>;
// Refuses a symlink target and a missing parent; temp file in the same directory, create_new + 0600,
// write_all + sync_all, rename; temp removed on error. Messages carry the error kind, never contents.

pub struct ProgramOutput { pub success: bool, pub code: Option<i32>, pub stdout: Vec<u8> }
pub trait ProgramRunner: Send + Sync {
    /// Runs `program args` (stdin null, stderr null). `deadline = None` waits for completion
    /// (pkexec: the user may take long in the Polkit dialog); `Some(d)` kills and reaps the child
    /// after `d`. stdout bounded by `max_stdout` bytes.
    fn run(&self, program: &str, args: &[String], deadline: Option<Duration>, max_stdout: usize)
        -> Result<ProgramOutput, String>;
}
pub struct SystemProgramRunner;   // std::process::Command, polls try_wait every 10 ms under a deadline

pub struct FeaturePaths { pub etc_dir: PathBuf, pub network_lock_rule: PathBuf,
                          pub remote_config: Option<PathBuf>, pub running_as_root: bool }
impl FeaturePaths { pub fn system() -> Self; }   // constants + env + geteuid

pub trait FeatureExecutor: Send + Sync { fn execute(&self, action: FeatureAction) -> FeatureOutcome; }
pub struct SystemFeatureExecutor { paths: FeaturePaths, runner: Arc<dyn ProgramRunner> }
impl SystemFeatureExecutor { pub fn new(paths: FeaturePaths, runner: Arc<dyn ProgramRunner>) -> Self; }
pub fn probe_snapshot(paths: &FeaturePaths, runner: &dyn ProgramRunner) -> FeatureSnapshot;
```

Executor behaviour:
- `Refresh`: flags by `symlink_metadata` on `etc_dir.join(flag)` (exists as any entry = `true`, `NotFound` = `false`, other error = `None`; the daemon's rule); network lock idem on `network_lock_rule` (an `EACCES` on a `0750` rules directory gives `None` → Unknown); unit by `unit_show_command` with `USER_SYSTEMCTL_TIMEOUT` (skipped → `Unknown` when `running_as_root`); remote config by `read_remote_config`.
- `SetSystemFlag` / `SetNetworkLock`: one `runner.run(pkexec…, None, 0)`; non-success → `Err("… (authorization denied or error, exit code N)")`.
- `SetCompanion`: refused (`Err`) when `running_as_root`; else `runner.run(systemctl --user …, Some(USER_SYSTEMCTL_TIMEOUT), 0)`.
- `DisableRemote(s)`: refused when `running_as_root` or no config path; read (bounded, symlink refused), `disable_remote_keys`, `Ok(None)` → `Ok(())` without writing; else `write_user_file_atomically` then `companion_restart_command` (deadline); a restart failure → `Err("Saved, but soos-remote did not restart: …")`.

```rust
pub type Notify = crate::privileged::Notify;
#[derive(Debug, thiserror::Error)]
pub enum FeatureRunnerError { #[error("another feature change is already in progress")] Busy,
                              #[error("failed to start the feature worker: {0}")] Spawn(#[from] std::io::Error) }
pub struct FeatureRunner { /* executor, notify, mpsc tx/rx, in_flight: Option<(JoinHandle<()>, FeatureAction)> */ }
impl FeatureRunner {
    pub fn new(executor: Arc<dyn FeatureExecutor>, notify: Notify) -> Self;
    pub fn submit(&mut self, action: FeatureAction) -> Result<(), FeatureRunnerError>;
    pub fn is_busy(&self) -> bool;
    pub fn privileged_in_flight(&self) -> bool;   // a SetSystemFlag / SetNetworkLock is running
    pub fn poll(&mut self) -> Vec<FeatureOutcome>; // never blocks; a worker that died without an outcome yields Applied/Refreshed error logged and clears in_flight
}
```

### 2.3 `onboarding_view.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TourEvent { Next, Back, Close, Restart, Apply(FeatureAction) }
pub struct TourView<'a> {
    pub snapshot: &'a FeatureSnapshot,
    pub loaded: bool,                    // a Refresh has completed at least once
    pub busy: bool,                      // any privileged or feature action in flight
    pub message: Option<(&'a str, bool)>,// last result, (text, is_error)
}
pub fn show_tour(ctx: &egui::Context, tour: &Tour, view: &TourView<'_>) -> Vec<TourEvent>;
```

- `egui::Modal` with the brand modal frame of the delete dialog (pale fill, 2 px blue border, `R_CARD`), width 520 pt clamped to the screen, id `soos_onboarding_tour`; page content inside a scope whose id is salted with `tour.generation()` (motion hook).
- Header: `n / TOUR_PAGE_COUNT` step text and a thin progress track (`widgets::progress_bar` is not reused; a static track painted from theme tokens).
- Feature page: title, explanation, `when_off`, a state chip ("On", "Off", "Not installed", "Not available in this version", "Checking…" before the first refresh, "Unknown"), then the control: a `widgets::toggle_switch` interactive only when `toggle_route(…, !current)` is an `Action` and `!busy`; otherwise the switch is inert and the `Command` text is shown in a monospace, selectable label with a "Copy" button (`ctx.copy_text`), or the `Unavailable` reason. For `Unknown` with both directions `Action`, two buttons "Turn on" / "Turn off" replace the switch.
- Footer: "Skip tour" (Intro and feature pages), "Back", "Next" / "Done" (Overview). Close buttons and Escape are disabled while `busy` (the modal keeps the rest of the window blocked until the pending action returns). Escape / backdrop click = `Close`.
- Overview: every feature with its state chip and the same control in a compact row, plus "Replay the tour" (`Restart`).
- Wording rules (not childish): sentence case, no exclamation marks, no emoji, no "awesome/magic/simply", the risk stated plainly (for example presence: "unlocks a locked session when your face is verified, without a keypress").

### 2.4 `app.rs` wiring

- Fields: `tour: Tour`, `tour_store: Option<Arc<TourStore>>`, `features: FeatureSnapshot`, `features_loaded: bool`, `feature_runner: FeatureRunner`, `feature_message: Option<(String, bool)>`.
- `new`: `tour_state_path(env)` → `TourStore`; `should_show_tour(load)` → `Tour::first_run()` else `Tour::closed()`; submit `Refresh`.
- Each frame: drain `feature_runner.poll()`; `Refreshed` → snapshot; `Applied` → message (success text names the feature), then submit `Refresh`; daemon monitor `request_refresh()` after a flag change.
- Header: `header::features_button` left of the frame statistics; click → `tour.open_overview()` + `Refresh`.
- `TourEvent::Close` → `tour.close()`, and when the tour was opened by first run or the user went through it, persist `mark_seen` on the background thread (always on close: closing the overview also marks it seen, harmless).
- `TourEvent::Apply(a)`: refused with a message while `tasks.is_busy()` or `feature_runner.is_busy()`; else submitted.
- Header daemon switch busy = `tasks.is_busy() || feature_runner.privileged_in_flight()`; `submit_privileged` refuses while `feature_runner.privileged_in_flight()` (no stacked Polkit dialogs, either direction).
- `app.rs` still contains no `Command::new` (existing `test_app_ui_code_spawns_no_process`).

## 3. Constants & Config

| Name | Location | Value | Sentinel semantics |
|---|---|---|---|
| `TOUR_VERSION` | `onboarding.rs` | 1 | stored version `< 1` or garbage → shown; `>=` current → not shown |
| `MAX_TOUR_STATE_BYTES` | `onboarding.rs` | 4096 | larger → `NotSeen` (rewritten on close) |
| `TOUR_STATE_FILE_NAME` | `onboarding.rs` | `gui-tour.json` | — |
| `MAX_REMOTE_CONFIG_BYTES` | `feature_control.rs` | 16 384 (= `soos-remote` `MAX_CONFIG_BYTES`, cross-checked by a source test) | larger → `Invalid` (no edit) |
| `MAX_UNIT_SHOW_BYTES` | `feature_control.rs` | 1024 | larger → output truncated to the bound before parsing → `Unknown` if no state line |
| `USER_SYSTEMCTL_TIMEOUT` | `feature_control.rs` | 3 s | child killed and reaped → `Unknown` / `Err` |
| `GLOBAL_DISABLE_FLAG`, `PRESENCE_DISABLE_FLAG` | `feature_control.rs` | `disabled`, `presence.disable` | equal to the daemon constants (source cross-check test) |
| `NETWORK_LOCK_RULE_PATH` | `feature_control.rs` | `/etc/polkit-1/rules.d/49-soos-network-lock.rules` | equal to `soos-admin` `NETWORK_LOCK_RULES_DIR` + `FILE` (source cross-check) |
| `SOOS_ADMIN_PROGRAM` | `privileged.rs` | `/usr/bin/soos-admin` or `concat!(env!("SOOS_BINDIR"), "/soos-admin")` | same build-time validation as `SOOS_ENROLL_PROGRAM` |
| `TOUCH_PROGRAM`, `RM_PROGRAM` | `privileged.rs` | `/usr/bin/touch`, `/usr/bin/rm` | absolute, never `PATH` |

No configuration key is added. Environment read once at start-up: `XDG_CONFIG_HOME`, `HOME` (absolute values only).

## 4. Error Taxonomy

Nothing here is on the authentication path. Every failure is a GUI message; no failure changes a feature state by itself:

| Failure | Result |
|---|---|
| Polkit denied / helper exit ≠ 0 | `Applied { result: Err }` banner in the tour; snapshot refreshed (shows the real state) |
| Probe error | `Unknown` state, no inferred value |
| `remote.toml` symlink / oversize / invalid / table layout / non-bool | no write, error message, state `Unknown` |
| Restart failure after a successful edit | `Err("Saved, but soos-remote did not restart: …")` |
| Tour state unreadable | tour not auto-shown; still reachable from the header |
| Tour state write failure | logged (`warn`, error kind), tour closed for this session |
| Worker thread panics / dies | `poll` clears `in_flight`, logs `error`, the UI stays usable |

Fail-safe direction: only explicit user clicks produce a change; a refresh never writes.

## 5. Latency Budget

Not on the auth path. UI thread: no process and no blocking I/O (the state file write runs on its own thread; `TourStore::load` is one bounded 4 KiB read at start-up, before the first frame).

## 6. Invariants Touched

- GUI threading model (`Docs/GUI_APPLICATION.md` §1): every process runs on a worker thread; `app.rs` has no `Command::new` (existing test unchanged).
- Programs by absolute path (GitHub #314 CAM-NEW-7, `gcv_review_contract`): `pkexec`, `systemctl`, `touch`, `rm`, `soos-admin`.
- One privileged dialog at a time (GitHub #154): cross-runner exclusion in §2.4.
- GUI never writes a root-owned file (OD-4); the only file it writes is the user's own `gui-tour.json` and (off only) `remote.toml`, both mode 0600 through temp + rename, symlinks refused.
- No secret, biometric data or configuration value is logged; `Debug` of every new type prints no file contents.
- `#![forbid(unsafe_code)]` in every new module (crate already forbids).
- Network lock ADR 2026-10-13: "installed only by an explicit administrator command" — the GUI switch runs that same command under Polkit admin authentication (ADR amendment, §8).
- Presence ADR 2026-10-02: flag semantics unchanged; the GUI only creates/removes the documented flag.

## 7. Test Hooks for Tester (contract, `crates/gui/tests/`)

- `onboarding_state_tests.rs`: `Tour` navigation (saturation, `generation` increments, `open_overview`, `restart`, `close`), `TOUR_PAGE_COUNT`, `tour_state_path` (XDG, HOME, relative/empty ignored, none), `parse_tour_state` (valid, other keys, `seen:false`, garbage, wrong types), `should_show_tour` (all three loads, older version), `TourStore` round trip in a tempdir (0600 mode, parent created 0700, symlink refused for load and write, oversize → `NotSeen`, malformed → `NotSeen` then overwritten).
- `onboarding_feature_tests.rs`: `FeatureSnapshot::state` table; `toggle_route` table (root, not installed, remote keys on/off, offline alerts); catalogue wording rules (9 features, unique titles, no `!`, no emoji/non-ASCII, no "awesome"/"magic"/"simply", every explanation ≥ 80 chars).
- `feature_control_tests.rs`: argv functions (absolute programs, exact vectors, never `--rfkill`/`--root`, only the two flag paths); `parse_unit_show`; `parse_remote_keys`; `disable_remote_keys` (true→false, comments and other lines byte-identical, absent unchanged, `Ok(None)` when already off, cascade keys, tables refused, inline/string values refused, duplicates refused, commented lines ignored, verification); `read_remote_config` bounds and symlink; `write_user_file_atomically` (0600, symlink refused, no temp left); `SystemFeatureExecutor` with a recording fake `ProgramRunner` and tempdir `FeaturePaths` (each action's exact call, root refusals, no call on `Ok(None)`, restart after edit, Refresh never runs pkexec); `FeatureRunner` (submit returns while executor blocks, `Busy`, `privileged_in_flight`, poll drains); source cross-checks with the daemon, `soos-admin` and `soos-remote` constants; `feature_control.rs` runs no shell (`"sh"`, `"-c"` absent).
- `onboarding_view_tests.rs`: headless `egui::Context::run_ui` renders every page with unknown, loaded, busy and message views without panic and emits no event without input; Escape while not busy yields `Close`, while busy yields nothing; `onboarding_view.rs` contains no `Command::new`, `std::fs` or `std::process`.

## 8. Documentation Drift / ADR

New ADR "[2026-10-13] First-Run Onboarding Tour and GUI Feature Toggles in `soos-gui`" recording: OD-1..OD-4, AS-1..AS-4, the toggle/privilege model (§2.2 routing table), the separate runner and cross-runner exclusion, the amendment of the network-lock ADR wording ("explicit administrator command" now includes the GUI switch under Polkit `auth_admin`), and that motion (#377) and the phone tour are later PRs. `Docs/GUI_APPLICATION.md` gains a section "First-run tour and feature switches"; the threading table gains `soos-gui-features` and `soos-gui-tour-state`.

## 9. #363 Acceptance Coverage (this PR)

| #363 acceptance line | This PR |
|---|---|
| Tour on first launch only, per-user state, replayable, skippable | §2.1 `Tour`, `TourStore`; §2.4 header button; `onboarding_state_tests.rs` |
| Each step shows the current state (daemon / soos-remote / config) and offers an off switch through existing privileged paths | §2.1 state table (flags, `daemon.toml`, user unit, `remote.toml`, polkit + udev rules), §2.2 executor; `onboarding_feature_tests.rs`, `feature_control_tests.rs` |
| Phone tour (web app, native app) | **Deferred** to the next PR of #363 (owner scope for this run) |
| soos brand and #358 motion; reduced motion | Brand theme tokens and widgets only; no animation is added, only the `Tour::generation` hook, so reduced motion is respected trivially; motion lands with `feat/gui-motion` (#377) |
| English UI text; docs, ADR, matrix, walkthrough | wording test; ADR; `OBT*` rows; walkthrough 209 |

## 10. Revision 1 (plan evaluator round 1, findings 1–11)

R-1 **Wi-Fi lock and the rfkill rule** (finding 1). `FeatureSnapshot` gains `network_lock_rfkill_installed: Option<bool>` probed on `/etc/udev/rules.d/72-soos-network-lock-rfkill.rules` (constant `NETWORK_LOCK_RFKILL_PATH`, cross-checked with `soos-admin`). Routing of WifiLock: *off* = `Action(SetNetworkLock { installed: false })` (removes both rules; `when_off` says so); *on* = `Action(SetNetworkLock { installed: true })` **only** when `network_lock_rfkill_installed == Some(false)`, otherwise `Command("sudo soos-admin network-lock install --rfkill")` (keeps the rfkill rule; the text says to drop `--rfkill` to remove it). With the polkit-rule state Unknown (Arch `0750` rules directory), only the off action is offered as a button; on is the command.

R-2 **Presence when face unlock is off** (finding 2). `toggle_route(PresenceUnlock, on)` returns `Unavailable("Face unlock is off; turn it on first.")` when `global_disabled == Some(true)`. `FaceUnlock::when_off` states that presence auto-unlock stops too.

R-3 **Daemon configuration** (finding 3). The probe reads `/etc/soos/daemon.toml` with the shared bounded reader `soos_camera_v4l::daemon_config::read_daemon_config_text` (the daemon's own reader: `O_PATH` pin, regular file, 1 MiB bound) and parses with `toml`. New snapshot fields: `daemon_presence_enabled: Option<bool>` (`[presence] enabled`, default `true`; missing file = `Some(true)`; unreadable or invalid = `None`) and `daemon_remote_view: Option<bool>` (`[preview] enabled && remote_view && allowed_uids` contains the GUI's real uid; defaults `false`; missing file = `Some(false)`; unreadable = `None`). State rules: PresenceUnlock is Off when `daemon_presence_enabled == Some(false)`, On only when both flags are `Some(false)` and the key is `Some(true)`, otherwise Unknown. CameraView: key `false` → Off; key `true` and `daemon_remote_view == Some(true)` → On; `Some(false)` → Off; `None` → Unknown. Routing: when the daemon side is what keeps the feature off, *on* is a `Command` naming the key in `/etc/soos/daemon.toml` and `sudo systemctl restart soos-daemon.service` (the GUI never edits `daemon.toml`; #362 PR 2 brings the checked editor). CameraView *off* stays `DisableRemote(CameraView)` whenever the `camera_view` key is `true`, even if the state is Off because of the daemon side (the view shows the off button in that case).

R-4 **Cross-runner exclusion at every submit site** (finding 4). All `TaskRunner::submit` calls in `app.rs` go through one wrapper `submit_task(action) -> Result<(), String>` that refuses while `feature_runner.privileged_in_flight()`; feature actions are refused while `tasks.is_busy()`.

R-5 **Non-blocking opens** (finding 5). User files (`gui-tour.json`, `remote.toml`) are read through one helper `read_user_file_bounded(path, max) -> Result<Option<Vec<u8>>, UserFileError>`: `open(O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)`, `fstat` on the handle (regular file, size ≤ max), bounded read. `ELOOP` (symlink) and a non-regular file are refused; a FIFO never blocks. `TourStore::load` keeps running once in `SoosApp::new` (before the first frame; non-blocking open, 4 KiB bound).

R-6 **Turn-on commands name the prerequisites** (finding 6). Unlock: "needs `rp_id` and a registered passkey"; Funnel: "needs `rp_id` and the Tailscale Funnel policy"; Push: "needs `password_alerts = true`, `rp_id` and the soos-push-sender user unit"; CameraView: "needs `rp_id`, and `[preview]` of /etc/soos/daemon.toml must allow your uid and `remote_view`". Tested.

R-7 **Runner lifecycle** (finding 7). A `Refresh` refused as `Busy` sets `refresh_pending`; the app submits it once the runner is idle; no message. `FeatureRunner` has no `Drop` and never joins (dropping the `JoinHandle` detaches a worker that may sit in a Polkit dialog).

R-8 Header facts updated (base `6f32ef8`); the walkthrough number 209 is the orchestrator's assignment (finding 8).

R-9 §9 added (finding 9).

R-10 **Face unlock wording** (finding 10). The FaceUnlock explanation says per-service switches (`/etc/soos/<service>.disable`, `gdm.disable`) can still turn it off for one service, and the Overview footnote repeats it. The probe keeps `symlink_metadata` (the daemon's rule); PAM's `exists()` differs only for a dangling link, which `soos-admin status` reports.

R-11 **Masked unit** (finding 11). `UnitState::Masked` (from `LoadState=masked`); PhoneCompanion state Off; routing *on* = `Command("systemctl --user unmask soos-remote.service && systemctl --user enable --now soos-remote.service")`, *off* = `Unavailable("The unit is masked, so it cannot start.")`.
