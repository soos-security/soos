//! First-run onboarding tour of `soos-gui` (GitHub #379, PR 1 of #363).
//!
//! This module is pure except [`TourStore`]: it holds the tour page machine, the feature
//! catalogue and its wording, the mapping from probed facts ([`FeatureSnapshot`]) to the state
//! shown for each feature, and the route of every on/off request ([`toggle_route`]). Probing and
//! applying live in [`crate::feature_control`]; rendering lives in [`crate::onboarding_view`].
//!
//! The tour opens on the first launch only (per-user state file `gui-tour.json` next to
//! `remote.toml`), can be skipped, and can be reopened at any time from the header.

#![forbid(unsafe_code)]

use std::ffi::OsStr;
use std::fs::{DirBuilder, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::feature_control::{FeatureAction, RemoteSwitch, SystemFlag};

/// Version of the tour content. Raising it shows the tour again to users who saw an older one.
pub const TOUR_VERSION: u32 = 1;

/// Largest state file read; a larger file is treated as "not seen" and rewritten on close.
pub const MAX_TOUR_STATE_BYTES: u64 = 4096;

/// File name of the per-user tour state, in `$XDG_CONFIG_HOME/soos` (or `~/.config/soos`).
pub const TOUR_STATE_FILE_NAME: &str = "gui-tour.json";

// ---------------------------------------------------------------------------------------------
// Feature catalogue
// ---------------------------------------------------------------------------------------------

/// A feature explained by the tour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    /// Face verification in PAM (login, lock screen, `sudo`).
    FaceUnlock,
    /// Presence auto-unlock of a locked session by `soos-daemon`.
    PresenceUnlock,
    /// The `soos-remote` user service (phone web app).
    PhoneCompanion,
    /// Lock from the phone, and unlock with a passkey (`allow_unlock`).
    RemoteUnlock,
    /// Access over the Internet through Tailscale Funnel (`allow_funnel`).
    InternetAccess,
    /// Web Push alerts of failed passwords (`push_notifications`).
    PushAlerts,
    /// Live camera view on the phone (`camera_view`).
    CameraView,
    /// Administrator authentication before Wi-Fi changes (polkit rule).
    WifiLock,
    /// Alerts when the PC goes offline and comes back (`offline_alerts`).
    OfflineAlerts,
}

/// Every feature, in tour order.
pub const FEATURES: [Feature; 9] = [
    Feature::FaceUnlock,
    Feature::PresenceUnlock,
    Feature::PhoneCompanion,
    Feature::RemoteUnlock,
    Feature::InternetAccess,
    Feature::PushAlerts,
    Feature::CameraView,
    Feature::WifiLock,
    Feature::OfflineAlerts,
];

impl Feature {
    /// Short title of the step.
    pub fn title(self) -> &'static str {
        match self {
            Self::FaceUnlock => "Face unlock",
            Self::PresenceUnlock => "Presence auto-unlock",
            Self::PhoneCompanion => "Phone companion",
            Self::RemoteUnlock => "Remote lock and unlock",
            Self::InternetAccess => "Internet access",
            Self::PushAlerts => "Push alerts",
            Self::CameraView => "Live camera view",
            Self::WifiLock => "Wi-Fi lock",
            Self::OfflineAlerts => "Offline alerts",
        }
    }

    /// What the feature does, when it acts and what it exposes.
    pub fn explanation(self) -> &'static str {
        match self {
            Self::FaceUnlock => {
                "soos checks your face with the camera when you log in, unlock the screen or \
                 run sudo. Your password always remains accepted as a fallback, and a failed or \
                 unavailable check never blocks it. A per-service switch \
                 (/etc/soos/<service>.disable) can still turn it off for one service."
            }
            Self::PresenceUnlock => {
                "While your session is locked, soos-daemon looks for your face every few \
                 seconds and unlocks the session when it is verified, without a keypress. The \
                 PC then opens whenever you are in front of it, even if you did not mean to \
                 use it."
            }
            Self::PhoneCompanion => {
                "A small service in your user session shows the lock state of this PC on your \
                 phone and lets you lock it remotely. It listens only on a local socket that \
                 Tailscale publishes to devices of your tailnet."
            }
            Self::RemoteUnlock => {
                "Locking from the phone is always available while the companion runs. \
                 Unlocking from the phone additionally needs a passkey check (Face ID, \
                 fingerprint or screen lock) every time."
            }
            Self::InternetAccess => {
                "Tailscale Funnel makes the phone page reachable from the Internet without the \
                 VPN app on the phone. Every action still needs your passkey, but the page is \
                 then exposed beyond your tailnet."
            }
            Self::PushAlerts => {
                "Failed password attempts on this PC are pushed to your phone as \
                 notifications. Alerts carry the time, source, account and count, never the \
                 typed text."
            }
            Self::CameraView => {
                "You can watch the PC camera live from the phone, for a limited time and after \
                 a passkey check per view. Nothing is recorded or stored."
            }
            Self::WifiLock => {
                "Changing or disconnecting the Wi-Fi of this PC asks for an administrator \
                 password, so whoever holds the PC cannot cut it off the network to silence the \
                 phone companion. Connecting to known networks keeps working."
            }
            Self::OfflineAlerts => {
                "Your phone is told when this PC comes back online after being offline, with \
                 how long it was away. It uses the push alerts, so those must stay on."
            }
        }
    }

    /// One sentence on what turning the feature off changes.
    pub fn when_off(self) -> &'static str {
        match self {
            Self::FaceUnlock => {
                "Off: soos stops checking faces everywhere, presence auto-unlock stops too, \
                 and only passwords are used."
            }
            Self::PresenceUnlock => {
                "Off: the session stays locked until you unlock it yourself; face unlock at a \
                 prompt keeps working."
            }
            Self::PhoneCompanion => {
                "Off: the service is stopped and disabled; the phone page is unreachable."
            }
            Self::RemoteUnlock => "Off: the phone can still lock this PC, never unlock it.",
            Self::InternetAccess => {
                "Off: the page is reachable only from devices signed in to your tailnet."
            }
            Self::PushAlerts => {
                "Off: no notification is sent; alerts remain visible on the phone page."
            }
            Self::CameraView => "Off: the phone can no longer open the camera of this PC.",
            Self::WifiLock => {
                "Off: any local user can change the Wi-Fi again without a password; the \
                 optional airplane-mode lock is removed as well."
            }
            Self::OfflineAlerts => {
                "Off: no offline history is recorded and no back-online notification is sent; \
                 push alerts keep working."
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Probed facts and the state shown
// ---------------------------------------------------------------------------------------------

/// State shown for a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureState {
    /// The feature is enabled.
    On,
    /// The feature is disabled.
    Off,
    /// The component providing the feature is not installed.
    NotInstalled,
    /// The feature does not exist in this version.
    NotAvailable,
    /// The state could not be determined.
    Unknown,
}

/// State of a systemd unit in the user's manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitState {
    /// Not probed or not determinable.
    Unknown,
    /// The unit file does not exist.
    NotInstalled,
    /// The unit runs (or is starting, reloading or stopping).
    Active,
    /// The unit is stopped or failed.
    Inactive,
    /// The unit is masked: it cannot start until it is unmasked.
    Masked,
}

/// The `soos-remote` keys the tour shows (missing keys are `false`, the service defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemoteKeys {
    /// `allow_unlock`.
    pub allow_unlock: bool,
    /// `allow_funnel`.
    pub allow_funnel: bool,
    /// `password_alerts`.
    pub password_alerts: bool,
    /// `push_notifications`.
    pub push_notifications: bool,
    /// `camera_view`.
    pub camera_view: bool,
    /// `camera_view_funnel`.
    pub camera_view_funnel: bool,
    /// `offline_alerts`.
    pub offline_alerts: bool,
}

/// What is known about `remote.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteConfigState {
    /// Not read (no path, or not probed yet).
    Unknown,
    /// The file does not exist.
    Missing,
    /// The file exists but could not be read or parsed safely.
    Invalid,
    /// The keys read from the file.
    Keys(RemoteKeys),
}

/// Facts probed by [`crate::feature_control::probe_snapshot`]. `None` = unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureSnapshot {
    /// `/etc/soos/disabled` exists.
    pub global_disabled: Option<bool>,
    /// `/etc/soos/presence.disable` exists.
    pub presence_disabled: Option<bool>,
    /// The Wi-Fi lock polkit rule is installed.
    pub network_lock_installed: Option<bool>,
    /// The optional rfkill udev rule of the Wi-Fi lock is installed.
    pub network_lock_rfkill_installed: Option<bool>,
    /// `[presence] enabled` of `daemon.toml` (default `true`; `None` = unreadable).
    pub daemon_presence_enabled: Option<bool>,
    /// `[preview]` allows the remote camera view for this user (`enabled`, `remote_view` and
    /// the uid in `allowed_uids`; `None` = unreadable).
    pub daemon_remote_view: Option<bool>,
    /// `soos-remote.service` in the user's manager.
    pub remote_unit: UnitState,
    /// Keys of `remote.toml`.
    pub remote_config: RemoteConfigState,
    /// The GUI runs as root: user-level controls are shown as commands.
    pub running_as_root: bool,
}

impl FeatureSnapshot {
    /// A snapshot where nothing is known yet.
    pub const fn unknown() -> Self {
        Self {
            global_disabled: None,
            presence_disabled: None,
            network_lock_installed: None,
            network_lock_rfkill_installed: None,
            daemon_presence_enabled: None,
            daemon_remote_view: None,
            remote_unit: UnitState::Unknown,
            remote_config: RemoteConfigState::Unknown,
            running_as_root: false,
        }
    }

    /// State shown for `feature`, derived only from probed facts.
    pub fn state(&self, feature: Feature) -> FeatureState {
        match feature {
            Feature::FaceUnlock => bool_state(self.global_disabled.map(|d| !d)),
            Feature::PresenceUnlock => {
                if self.global_disabled == Some(true)
                    || self.presence_disabled == Some(true)
                    || self.daemon_presence_enabled == Some(false)
                {
                    FeatureState::Off
                } else if self.global_disabled == Some(false)
                    && self.presence_disabled == Some(false)
                    && self.daemon_presence_enabled == Some(true)
                {
                    FeatureState::On
                } else {
                    FeatureState::Unknown
                }
            }
            Feature::PhoneCompanion => match self.remote_unit {
                UnitState::Active => FeatureState::On,
                UnitState::Inactive | UnitState::Masked => FeatureState::Off,
                UnitState::NotInstalled => FeatureState::NotInstalled,
                UnitState::Unknown => FeatureState::Unknown,
            },
            Feature::RemoteUnlock
            | Feature::InternetAccess
            | Feature::PushAlerts
            | Feature::CameraView
            | Feature::OfflineAlerts => {
                if self.remote_unit == UnitState::NotInstalled {
                    return FeatureState::NotInstalled;
                }
                match self.remote_config {
                    RemoteConfigState::Missing => FeatureState::NotInstalled,
                    RemoteConfigState::Unknown | RemoteConfigState::Invalid => {
                        FeatureState::Unknown
                    }
                    RemoteConfigState::Keys(keys) => match feature {
                        Feature::RemoteUnlock => bool_state(Some(keys.allow_unlock)),
                        Feature::InternetAccess => bool_state(Some(keys.allow_funnel)),
                        Feature::PushAlerts => bool_state(Some(keys.push_notifications)),
                        Feature::OfflineAlerts => bool_state(Some(keys.offline_alerts)),
                        // The view also needs the daemon's `[preview]` permission.
                        _ if !keys.camera_view => FeatureState::Off,
                        _ => bool_state(self.daemon_remote_view),
                    },
                }
            }
            Feature::WifiLock => bool_state(self.network_lock_installed),
        }
    }
}

fn bool_state(on: Option<bool>) -> FeatureState {
    match on {
        Some(true) => FeatureState::On,
        Some(false) => FeatureState::Off,
        None => FeatureState::Unknown,
    }
}

/// The remote switch backing a `soos-remote` key feature.
pub fn remote_switch(feature: Feature) -> Option<RemoteSwitch> {
    match feature {
        Feature::RemoteUnlock => Some(RemoteSwitch::Unlock),
        Feature::InternetAccess => Some(RemoteSwitch::Funnel),
        Feature::PushAlerts => Some(RemoteSwitch::Push),
        Feature::CameraView => Some(RemoteSwitch::CameraView),
        Feature::OfflineAlerts => Some(RemoteSwitch::OfflineAlerts),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Toggle routing
// ---------------------------------------------------------------------------------------------

/// How an on/off request for a feature is carried out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToggleRoute {
    /// Applied by the GUI through [`crate::feature_control::FeatureRunner`].
    Action(FeatureAction),
    /// The exact command or key edit, shown with a Copy button.
    Command(String),
    /// No control; the reason is shown.
    Unavailable(&'static str),
}

/// Path of `remote.toml` as shown in commands (never the real home directory).
const REMOTE_TOML_SHOWN: &str = "~/.config/soos/remote.toml";

fn remote_key_command(key: &str, value: bool) -> String {
    format!(
        "Set `{key} = {value}` in {REMOTE_TOML_SHOWN}, then run: \
         systemctl --user restart soos-remote.service"
    )
}

/// Why Internet access stays on while the native Android app is configured (GitHub #429):
/// `soos-remote` refuses an `[android]` table without `allow_funnel = true` (exit 78), so the
/// GUI never turns Funnel off in such a file.
const FUNNEL_ANDROID_NOTE: &str = "Not while remote.toml has an [android] table: soos-remote \
     requires allow_funnel = true for the Android app, so remove the [android] table first.";

/// What `soos-remote` needs before a key can be turned on (it refuses to start otherwise).
fn remote_prerequisites(switch: RemoteSwitch) -> &'static str {
    match switch {
        RemoteSwitch::Unlock => "It needs `rp_id` and a registered passkey.",
        RemoteSwitch::Funnel => "It needs `rp_id` and the Tailscale Funnel policy of your tailnet.",
        RemoteSwitch::Push => {
            "It needs `password_alerts = true`, `rp_id` and the soos-push-sender user unit."
        }
        RemoteSwitch::CameraView => {
            "It needs `rp_id`, and [preview] of /etc/soos/daemon.toml must allow your uid \
             and `remote_view` (administrator change)."
        }
        RemoteSwitch::OfflineAlerts => {
            "It needs `push_notifications = true` (soos-remote refuses offline alerts without \
             push alerts)."
        }
    }
}

/// Exact daemon-side change, shown when `daemon.toml` keeps a feature off.
const DAEMON_PRESENCE_COMMAND: &str = "As root, set `enabled = true` under [presence] in \
     /etc/soos/daemon.toml, then run: sudo systemctl restart soos-daemon.service";
const DAEMON_PREVIEW_COMMAND: &str = "As root, set `enabled = true`, `remote_view = true` and \
     your uid in `allowed_uids` under [preview] in /etc/soos/daemon.toml, then run: \
     sudo systemctl restart soos-daemon.service";

/// Route of a request to turn `feature` on (`turn_on`) or off.
pub fn toggle_route(snapshot: &FeatureSnapshot, feature: Feature, turn_on: bool) -> ToggleRoute {
    match feature {
        Feature::FaceUnlock => ToggleRoute::Action(FeatureAction::SetSystemFlag {
            flag: SystemFlag::Global,
            present: !turn_on,
        }),
        Feature::PresenceUnlock => {
            if turn_on && snapshot.global_disabled == Some(true) {
                return ToggleRoute::Unavailable("Face unlock is off; turn it on first.");
            }
            if turn_on && snapshot.daemon_presence_enabled == Some(false) {
                // Removing the flag alone would not turn it on: the daemon key keeps it off.
                let mut text = DAEMON_PRESENCE_COMMAND.to_string();
                if snapshot.presence_disabled == Some(true) {
                    text.push_str(" Also remove /etc/soos/presence.disable.");
                }
                return ToggleRoute::Command(text);
            }
            ToggleRoute::Action(FeatureAction::SetSystemFlag {
                flag: SystemFlag::Presence,
                present: !turn_on,
            })
        }
        Feature::PhoneCompanion => {
            if snapshot.remote_unit == UnitState::NotInstalled {
                return ToggleRoute::Unavailable(
                    "The phone companion is not installed for this user \
                     (scripts/install_remote.sh, Docs/REMOTE_COMPANION.md).",
                );
            }
            if snapshot.remote_unit == UnitState::Masked {
                return if turn_on {
                    ToggleRoute::Command(
                        "systemctl --user unmask soos-remote.service && \
                         systemctl --user enable --now soos-remote.service"
                            .to_string(),
                    )
                } else {
                    ToggleRoute::Unavailable("The unit is masked, so it cannot start.")
                };
            }
            if snapshot.running_as_root {
                let verb = if turn_on { "enable" } else { "disable" };
                return ToggleRoute::Command(format!(
                    "Run as your own user (not root): systemctl --user {verb} --now \
                     soos-remote.service"
                ));
            }
            ToggleRoute::Action(FeatureAction::SetCompanion { enabled: turn_on })
        }
        Feature::RemoteUnlock
        | Feature::InternetAccess
        | Feature::PushAlerts
        | Feature::CameraView
        | Feature::OfflineAlerts => {
            let Some(switch) = remote_switch(feature) else {
                return ToggleRoute::Unavailable("Unknown feature.");
            };
            if turn_on {
                // Widening remote exposure has prerequisites only soos-remote validates (AS-1).
                let key_on = matches!(
                    (switch, snapshot.remote_config),
                    (
                        RemoteSwitch::CameraView,
                        RemoteConfigState::Keys(RemoteKeys {
                            camera_view: true,
                            ..
                        })
                    )
                );
                if key_on && snapshot.daemon_remote_view != Some(true) {
                    // Only the daemon side keeps the camera view off.
                    return ToggleRoute::Command(DAEMON_PREVIEW_COMMAND.to_string());
                }
                return ToggleRoute::Command(format!(
                    "{} {}",
                    remote_key_command(switch.key(), true),
                    remote_prerequisites(switch)
                ));
            }
            let has_keys = matches!(snapshot.remote_config, RemoteConfigState::Keys(_));
            if snapshot.running_as_root || !has_keys {
                let mut text = remote_key_command(switch.key(), false);
                if switch == RemoteSwitch::Funnel {
                    text.push(' ');
                    text.push_str(FUNNEL_ANDROID_NOTE);
                }
                ToggleRoute::Command(text)
            } else {
                ToggleRoute::Action(FeatureAction::DisableRemote(switch))
            }
        }
        Feature::WifiLock => {
            // `network-lock install` without `--rfkill` removes the rfkill rule: run it only
            // when that rule is known to be absent (R-1).
            // With the polkit rule state unknown, "on" is never run blind either.
            if turn_on && snapshot.network_lock_rfkill_installed != Some(false) {
                return ToggleRoute::Command(
                    "sudo soos-admin network-lock install --rfkill (keeps the airplane-mode \
                     lock; leave out --rfkill to remove it)"
                        .to_string(),
                );
            }
            if turn_on && snapshot.network_lock_installed.is_none() {
                return ToggleRoute::Command("sudo soos-admin network-lock install".to_string());
            }
            ToggleRoute::Action(FeatureAction::SetNetworkLock { installed: turn_on })
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Tour page machine
// ---------------------------------------------------------------------------------------------

/// A page of the tour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourPage {
    /// Introduction.
    Intro,
    /// One feature.
    Feature(Feature),
    /// Every feature with its control.
    Overview,
}

/// Number of pages: the introduction, one per feature and the overview.
pub const TOUR_PAGE_COUNT: usize = FEATURES.len() + 2;

/// Open state and current page of the tour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tour {
    open: bool,
    index: usize,
    generation: u64,
}

impl Tour {
    /// A tour that is not shown.
    pub fn closed() -> Self {
        Self {
            open: false,
            index: 0,
            generation: 0,
        }
    }

    /// A tour shown on its introduction (first launch).
    pub fn first_run() -> Self {
        Self {
            open: true,
            index: 0,
            generation: 0,
        }
    }

    /// Whether the tour window is shown.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Index of the current page, `0..TOUR_PAGE_COUNT`.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Current page.
    pub fn page(&self) -> TourPage {
        match self.index {
            0 => TourPage::Intro,
            i => match FEATURES.get(i.saturating_sub(1)) {
                Some(feature) => TourPage::Feature(*feature),
                None => TourPage::Overview,
            },
        }
    }

    /// Motion hook (#377): changes on every page or visibility change.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn set(&mut self, open: bool, index: usize) {
        if self.open != open || self.index != index {
            self.open = open;
            self.index = index;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Next page; stays on the overview.
    pub fn next(&mut self) {
        let index = self
            .index
            .saturating_add(1)
            .min(TOUR_PAGE_COUNT.saturating_sub(1));
        self.set(self.open, index);
    }

    /// Previous page; stays on the introduction.
    pub fn back(&mut self) {
        self.set(self.open, self.index.saturating_sub(1));
    }

    /// Opens the tour on its introduction.
    pub fn restart(&mut self) {
        self.set(true, 0);
    }

    /// Opens the tour on the overview (header "Features" button).
    pub fn open_overview(&mut self) {
        self.set(true, TOUR_PAGE_COUNT.saturating_sub(1));
    }

    /// Closes the tour (skip, done or Escape).
    pub fn close(&mut self) {
        self.set(false, self.index);
    }
}

// ---------------------------------------------------------------------------------------------
// Per-user state
// ---------------------------------------------------------------------------------------------

fn absolute_dir(value: Option<&OsStr>) -> Option<PathBuf> {
    let value = value?;
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

/// `<dir>/soos/<file>` where `dir` is an absolute `XDG_CONFIG_HOME`, else `HOME/.config`.
pub(crate) fn soos_config_file(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
    file: &str,
) -> Option<PathBuf> {
    if let Some(xdg) = absolute_dir(xdg_config_home) {
        return Some(xdg.join("soos").join(file));
    }
    absolute_dir(home).map(|home| home.join(".config").join("soos").join(file))
}

/// Path of the tour state file, `None` when neither directory is an absolute path.
pub fn tour_state_path(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    soos_config_file(xdg_config_home, home, TOUR_STATE_FILE_NAME)
}

/// Parses `{"version": N, "seen": true}`; anything else is `None`.
pub fn parse_tour_state(bytes: &[u8]) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    if object.get("seen")?.as_bool()? {
        u32::try_from(object.get("version")?.as_u64()?).ok()
    } else {
        None
    }
}

/// Result of reading the tour state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourLoad {
    /// No valid record: show the tour.
    NotSeen,
    /// The tour of `version` was seen.
    Seen {
        /// Version of the seen tour.
        version: u32,
    },
    /// The state cannot be read safely: never shown automatically.
    Unavailable,
}

/// Whether the tour opens at start-up.
pub fn should_show_tour(load: TourLoad) -> bool {
    match load {
        TourLoad::NotSeen => true,
        TourLoad::Seen { version } => version < TOUR_VERSION,
        TourLoad::Unavailable => false,
    }
}

/// Error of [`TourStore::mark_seen`].
#[derive(Debug, thiserror::Error)]
pub enum TourStoreError {
    /// The state file is a symbolic link.
    #[error("refusing to write through a symbolic link")]
    Symlink,
    /// The deepest existing ancestor of the state file belongs to another user.
    #[error("the configuration directory belongs to another user")]
    NotOwned,
    /// A filesystem operation failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Per-user tour state file.
#[derive(Debug, Clone)]
pub struct TourStore {
    path: PathBuf,
}

impl TourStore {
    /// A store at `path`.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Path of the state file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the state (bounded, non-blocking open, never through a symbolic link).
    pub fn load(&self) -> TourLoad {
        match read_user_file_bounded(&self.path, MAX_TOUR_STATE_BYTES) {
            Ok(None) | Err(UserFileError::TooLarge) => TourLoad::NotSeen,
            Ok(Some(bytes)) => match parse_tour_state(&bytes) {
                Some(version) => TourLoad::Seen { version },
                None => TourLoad::NotSeen,
            },
            Err(_) => TourLoad::Unavailable,
        }
    }

    /// Records that the current tour version was seen (atomic write, mode 0600).
    ///
    /// # Errors
    ///
    /// [`TourStoreError::Symlink`] when the state file is a symbolic link;
    /// [`TourStoreError::Io`] when a directory or the file cannot be written.
    pub fn mark_seen(&self) -> Result<(), TourStoreError> {
        if let Ok(meta) = std::fs::symlink_metadata(&self.path) {
            if meta.file_type().is_symlink() {
                return Err(TourStoreError::Symlink);
            }
        }
        // Never create or write anything in another user's tree (a root session that kept
        // the user's HOME would leave a root-owned ~/.config/soos behind).
        if !owned_by_euid(&self.path, nix::unistd::geteuid().as_raw()) {
            return Err(TourStoreError::NotOwned);
        }
        if let Some(parent) = self.path.parent() {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        let contents = format!("{{\"version\":{TOUR_VERSION},\"seen\":true}}\n");
        write_atomically(&self.path, contents.as_bytes())?;
        Ok(())
    }
}

/// Whether the deepest existing entry on `path` (the path itself, else its nearest existing
/// ancestor) is owned by `euid`. Anything unreadable counts as not owned.
pub fn owned_by_euid(path: &Path, euid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    for candidate in path.ancestors() {
        if candidate.as_os_str().is_empty() {
            continue;
        }
        match std::fs::symlink_metadata(candidate) {
            Ok(meta) => return meta.uid() == euid,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }
    false
}

/// `O_NOFOLLOW`: never open the final path component through a symbolic link.
fn libc_o_nofollow() -> i32 {
    nix::fcntl::OFlag::O_NOFOLLOW.bits()
}

/// Why a user file was not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UserFileError {
    /// A symbolic link, FIFO, device or directory.
    #[error("not a regular file (symbolic links are refused)")]
    NotARegularFile,
    /// Larger than the bound.
    #[error("file too large")]
    TooLarge,
    /// Any other error.
    #[error("unreadable ({0})")]
    Unreadable(std::io::ErrorKind),
}

/// Reads a user file of at most `max` bytes. The open never follows a symbolic link
/// (`O_NOFOLLOW`) and never blocks (`O_NONBLOCK`: a FIFO is refused by the `fstat` check on the
/// handle). `Ok(None)` = the file does not exist.
///
/// # Errors
///
/// [`UserFileError`] for a link or non-regular file, an oversized file or a read error.
pub fn read_user_file_bounded(path: &Path, max: u64) -> Result<Option<Vec<u8>>, UserFileError> {
    let flags = nix::fcntl::OFlag::O_NOFOLLOW
        | nix::fcntl::OFlag::O_NONBLOCK
        | nix::fcntl::OFlag::O_CLOEXEC
        | nix::fcntl::OFlag::O_NOCTTY;
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(flags.bits())
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) if e.raw_os_error() == Some(nix::errno::Errno::ELOOP as i32) => {
            return Err(UserFileError::NotARegularFile)
        }
        Err(e) => return Err(UserFileError::Unreadable(e.kind())),
    };
    let meta = file
        .metadata()
        .map_err(|e| UserFileError::Unreadable(e.kind()))?;
    if !meta.is_file() {
        return Err(UserFileError::NotARegularFile);
    }
    if meta.len() > max {
        return Err(UserFileError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| UserFileError::Unreadable(e.kind()))?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > max) {
        return Err(UserFileError::TooLarge);
    }
    Ok(Some(bytes))
}

/// Writes `contents` to a fresh 0600 temporary file next to `path`, syncs it and renames it over
/// `path`. The temporary file is removed on any error. Refuses a symbolic link at `path`.
pub(crate) fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(std::io::Error::other(
                "refusing to write through a symbolic link",
            ));
        }
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| std::io::Error::other("path has no parent directory"))?;
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("path has no file name"))?;
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|e| std::io::Error::other(e.to_string()))?;
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let mut tmp_name = name.to_os_string();
    tmp_name.push(format!(".tmp-{}-{suffix}", std::process::id()));
    let tmp = parent.join(tmp_name);
    // Created by this call: only then may the error path remove the temporary name.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc_o_nofollow())
        .open(&tmp)?;
    let result = (|| {
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    drop(file);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
