//! One-time, read-only XDG desktop portal read of the reduced-motion preference
//! (`Docs/UI_MOTION.md` §8; ADR 2026-10-13 "Motion System for soos-gui and soos-remote" item (6);
//! GitHub #438).
//!
//! One named background thread runs its own current-thread Tokio runtime and opens one zbus
//! connection to the user's session bus. It calls only `org.freedesktop.portal.Settings.ReadOne`
//! (then `Read`, for portal version 1) on `org.freedesktop.portal.Desktop`, for
//! `org.freedesktop.appearance` / `reduced-motion` and, when that key does not decide,
//! `org.gnome.desktop.interface` / `enable-animations`, all under one overall deadline. No
//! object server, no well-known name, no match rule, no signal subscription, no retry. The value
//! is validated (one `u32` or one `bool`) and never logged. The UI thread only polls a channel
//! ([`PortalReader::poll`] uses `try_recv`) and enforces the deadline itself, so a bus that never
//! answers cannot hold the preference past it.

#![forbid(unsafe_code)]

use std::num::NonZeroUsize;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use super::PortalPreference;
use crate::theme;

/// Well-known name of the desktop portal.
pub const PORTAL_DESTINATION: &str = "org.freedesktop.portal.Desktop";
/// Object path of the desktop portal.
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
/// Settings interface of the desktop portal.
pub const PORTAL_SETTINGS_IFACE: &str = "org.freedesktop.portal.Settings";
/// Method reading one setting (portal version 2).
pub const METHOD_READ_ONE: &str = "ReadOne";
/// Method reading one setting wrapped in an extra variant (portal version 1).
pub const METHOD_READ: &str = "Read";
/// Namespace of the standard appearance keys.
pub const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";
/// `u32`: 1 = reduce motion, 0 = no preference.
pub const REDUCED_MOTION_KEY: &str = "reduced-motion";
/// GNOME interface settings namespace.
pub const GNOME_INTERFACE_NAMESPACE: &str = "org.gnome.desktop.interface";
/// `bool`: `false` = animations disabled (reduced motion).
pub const ENABLE_ANIMATIONS_KEY: &str = "enable-animations";

/// D-Bus error name of a method the portal does not implement (`ReadOne` on version 1).
const UNKNOWN_METHOD: &str = "org.freedesktop.DBus.Error.UnknownMethod";

/// Bound of the queue of unsolicited messages held by the connection.
const MAX_QUEUED_MESSAGES: NonZeroUsize = match NonZeroUsize::new(8) {
    Some(n) => n,
    None => NonZeroUsize::MIN,
};

/// Name of the reader thread.
const READER_THREAD_NAME: &str = "soos-motion-portal";

/// A validated setting value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingValue {
    /// A `u32`.
    U32(u32),
    /// A `bool`.
    Bool(bool),
    /// Any other type.
    Other,
}

/// What the bus returned for the two keys (`None` = no value or an error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortalReadings {
    /// `org.freedesktop.appearance` / `reduced-motion`.
    pub reduced_motion: Option<SettingValue>,
    /// `org.gnome.desktop.interface` / `enable-animations`.
    pub enable_animations: Option<SettingValue>,
}

/// Maps the readings to a preference: `reduced-motion` 1 or 0 decides; any other value falls
/// back to `enable-animations` (`false` reduce, `true` no preference); otherwise unavailable.
pub fn interpret(readings: PortalReadings) -> PortalPreference {
    match readings.reduced_motion {
        Some(SettingValue::U32(1)) => PortalPreference::Reduce,
        Some(SettingValue::U32(0)) => PortalPreference::NoPreference,
        _ => match readings.enable_animations {
            Some(SettingValue::Bool(false)) => PortalPreference::Reduce,
            Some(SettingValue::Bool(true)) => PortalPreference::NoPreference,
            _ => PortalPreference::Unavailable,
        },
    }
}

/// Whether the appearance key decides on its own (the GNOME key is then not read).
fn appearance_decides(value: Option<SettingValue>) -> bool {
    matches!(value, Some(SettingValue::U32(0 | 1)))
}

/// Source of the two settings, run on the reader thread (the session portal in production, a
/// fake in tests). It should return within `deadline`; the reader enforces it regardless.
pub trait SettingsBus: Send + 'static {
    /// Reads both keys once.
    fn read_settings(&mut self, deadline: Duration) -> PortalReadings;
}

/// Non-blocking handle on the one-time portal read.
#[derive(Debug)]
pub struct PortalReader {
    rx: Option<Receiver<PortalPreference>>,
    deadline_at: Instant,
    result: Option<PortalPreference>,
}

impl PortalReader {
    /// Starts the read of `bus` on one background thread; a spawn failure is `Unavailable`.
    pub fn spawn<B: SettingsBus>(mut bus: B, deadline: Duration) -> Self {
        let started = Instant::now();
        let deadline_at = started.checked_add(deadline).unwrap_or(started);
        let (tx, rx) = mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new()
            .name(READER_THREAD_NAME.to_owned())
            .spawn(move || {
                let answer = interpret(bus.read_settings(deadline));
                // The UI may have given up (deadline) and dropped the receiver.
                let _ = tx.try_send(answer);
            });
        match spawned {
            Ok(_) => Self {
                rx: Some(rx),
                deadline_at,
                result: None,
            },
            Err(_) => Self {
                rx: None,
                deadline_at,
                result: Some(PortalPreference::Unavailable),
            },
        }
    }

    /// Starts the read of the user's session portal with `PORTAL_READ_DEADLINE_MS`.
    pub fn spawn_session_portal() -> Self {
        Self::spawn(
            ZbusPortalBus,
            Duration::from_millis(theme::PORTAL_READ_DEADLINE_MS),
        )
    }

    /// Instant from which a missing answer is `Unavailable`.
    pub fn deadline_at(&self) -> Instant {
        self.deadline_at
    }

    /// [`Self::poll_at`] now.
    pub fn poll(&mut self) -> PortalPreference {
        self.poll_at(Instant::now())
    }

    /// Never blocks: the cached result, else the answer if one arrived, else `Pending` before
    /// the deadline and `Unavailable` from it on; a dead reader (disconnected channel) is
    /// `Unavailable`. Once not `Pending`, the result never changes (no retry).
    pub fn poll_at(&mut self, now: Instant) -> PortalPreference {
        if let Some(result) = self.result {
            return result;
        }
        let answer = match self.rx.as_ref().map(Receiver::try_recv) {
            Some(Ok(PortalPreference::Pending)) | Some(Err(TryRecvError::Disconnected)) | None => {
                PortalPreference::Unavailable
            }
            Some(Ok(answer)) => answer,
            Some(Err(TryRecvError::Empty)) => {
                if now < self.deadline_at {
                    return PortalPreference::Pending;
                }
                PortalPreference::Unavailable
            }
        };
        self.result = Some(answer);
        self.rx = None;
        answer
    }
}

/// The production bus: the user's session portal through zbus.
#[derive(Debug, Clone, Copy, Default)]
pub struct ZbusPortalBus;

impl SettingsBus for ZbusPortalBus {
    fn read_settings(&mut self, deadline: Duration) -> PortalReadings {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return PortalReadings::default();
        };
        runtime.block_on(async move {
            tokio::time::timeout(deadline, read_both(deadline))
                .await
                .unwrap_or_default()
        })
    }
}

/// Failure of one call.
enum CallError {
    /// The portal does not implement the method (fall back to `Read`).
    UnknownMethod,
    /// Anything else: no value.
    Other,
}

async fn read_both(deadline: Duration) -> PortalReadings {
    let Ok(connection) = zbus::connection::Builder::session()
        .method_timeout(deadline)
        .max_queued(MAX_QUEUED_MESSAGES)
        .build()
        .await
    else {
        return PortalReadings::default();
    };
    let reduced_motion = read_setting(&connection, APPEARANCE_NAMESPACE, REDUCED_MOTION_KEY).await;
    let enable_animations = if appearance_decides(reduced_motion) {
        None
    } else {
        read_setting(
            &connection,
            GNOME_INTERFACE_NAMESPACE,
            ENABLE_ANIMATIONS_KEY,
        )
        .await
    };
    PortalReadings {
        reduced_motion,
        enable_animations,
    }
}

/// `ReadOne`, then `Read` when the portal does not know `ReadOne`.
async fn read_setting(
    connection: &zbus::Connection,
    namespace: &'static str,
    key: &'static str,
) -> Option<SettingValue> {
    match call(connection, METHOD_READ_ONE, namespace, key).await {
        Ok(value) => Some(value),
        Err(CallError::UnknownMethod) => call(connection, METHOD_READ, namespace, key).await.ok(),
        Err(CallError::Other) => None,
    }
}

async fn call(
    connection: &zbus::Connection,
    method: &'static str,
    namespace: &'static str,
    key: &'static str,
) -> Result<SettingValue, CallError> {
    let reply = connection
        .call_method(
            Some(PORTAL_DESTINATION),
            PORTAL_PATH,
            Some(PORTAL_SETTINGS_IFACE),
            method,
            &(namespace, key),
        )
        .await
        .map_err(|error| classify(&error))?;
    let body = reply.body();
    let value: zbus::OwnedVariant = body.deserialize().map_err(|_| CallError::Other)?;
    // `Read` wraps the value in one more variant.
    Ok(setting_value(&value, 1))
}

fn classify(error: &zbus::Error) -> CallError {
    match error {
        zbus::Error::FDO(fdo) if matches!(**fdo, zbus::fdo::Error::UnknownMethod(_)) => {
            CallError::UnknownMethod
        }
        zbus::Error::MethodError(name, _, _) if name.as_str() == UNKNOWN_METHOD => {
            CallError::UnknownMethod
        }
        _ => CallError::Other,
    }
}

/// Validates a reply: a `u32` or a `bool`, unwrapping at most `depth` nested variants.
fn setting_value(value: &zbus::Variant<'_>, depth: u8) -> SettingValue {
    match value {
        zbus::Variant::U32(n) => SettingValue::U32(*n),
        zbus::Variant::Bool(b) => SettingValue::Bool(*b),
        zbus::Variant::Variant(inner) if depth > 0 => setting_value(inner, depth.saturating_sub(1)),
        _ => SettingValue::Other,
    }
}
