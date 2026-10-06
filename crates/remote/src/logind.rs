//! Mockable logind access (spec §2.6, D7–D10).
//!
//! The production source mirrors the daemon's presence worker: the connection is opened
//! lazily through the pinned system bus address [`SYSTEM_BUS_ADDRESS`] (never an
//! environment-derived bus), registers no object server, no well-known name and no signal
//! match; every property is read with a fresh `Properties.GetAll` round trip (no proxy, no
//! cache); every call is bounded by [`DBUS_CALL_TIMEOUT_MS`] and the connect by
//! [`DBUS_CONNECT_TIMEOUT_MS`]. A transport failure, a timeout or a malformed reply drops
//! the connection, which the next call reopens.
//!
//! The only state-changing calls are `Manager.LockSession` and, for the opt-in remote unlock
//! (ADR 2026-10-06), `Manager.UnlockSession`, both on the owner's own session.

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::session::{is_valid_session_id, session_props_from_properties, SessionProps};
use crate::{
    DBUS_CALL_TIMEOUT_MS, DBUS_CONNECT_TIMEOUT_MS, MAX_LISTED_SESSIONS, MAX_LOGIND_ERROR_LEN,
    MAX_OWN_SESSIONS, SYSTEM_BUS_ADDRESS,
};

/// logind bus name (a well-known name only root may own).
const LOGIND_DESTINATION: &str = "org.freedesktop.login1";
/// logind manager object.
const LOGIND_MANAGER_PATH: &str = "/org/freedesktop/login1";
/// logind manager interface.
const LOGIND_MANAGER_IFACE: &str = "org.freedesktop.login1.Manager";
/// logind session interface.
const LOGIND_SESSION_IFACE: &str = "org.freedesktop.login1.Session";
/// Standard properties interface.
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";
/// Error names meaning "the session vanished between two calls".
const VANISHED_ERRORS: [&str; 2] = [
    "org.freedesktop.DBus.Error.UnknownObject",
    "org.freedesktop.login1.NoSuchSession",
];
/// Bound of the queue of unsolicited messages held by the connection.
const MAX_QUEUED_MESSAGES: usize = 16;

/// Failure of a logind access; status → `unavailable`, lock → `503`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// The system bus could not be reached.
    #[error("system bus unavailable")]
    BusUnavailable,
    /// A call or the whole snapshot exceeded its deadline.
    #[error("logind call timed out")]
    Timeout,
    /// logind answered with an error (name never logged beyond `MAX_LOGIND_ERROR_LEN`).
    #[error("logind call failed")]
    Call,
    /// The reply did not have the expected shape.
    #[error("logind reply malformed")]
    Malformed,
    /// More sessions than the bounds allow.
    #[error("too many sessions")]
    TooManySessions,
}

/// Mockable logind access (the tester's injection point).
pub trait SessionSource: Send + Sync + 'static {
    /// Every session of `uid` (ListSessions filtered on the uid column, then GetAll each);
    /// more than `MAX_LISTED_SESSIONS` listed or `MAX_OWN_SESSIONS` of the uid →
    /// `TooManySessions`; a session that vanished between the two calls is skipped.
    fn own_sessions(
        &self,
        uid: u32,
    ) -> impl Future<Output = Result<Vec<SessionProps>, SourceError>> + Send;

    /// `Manager.LockSession(id)`.
    fn lock_session(&self, id: &str) -> impl Future<Output = Result<(), SourceError>> + Send;

    /// `Manager.UnlockSession(id)` (ADR 2026-10-06).
    fn unlock_session(&self, id: &str) -> impl Future<Output = Result<(), SourceError>> + Send;
}

/// Production logind access over the pinned system bus.
#[derive(Debug, Default)]
pub struct ZbusSessionSource {
    slot: tokio::sync::Mutex<Option<zbus::Connection>>,
}

/// One logind call failure, before mapping.
enum CallFailure {
    /// logind answered with this error name.
    Method(String),
    /// Transport failure or timeout: the connection was dropped.
    Transport(SourceError),
}

/// Truncates a logind error name to `MAX_LOGIND_ERROR_LEN` bytes on a char boundary, for
/// a `debug!` line (the error message body is never logged).
fn truncated_error_name(name: &str) -> &str {
    let mut end = name.len().min(MAX_LOGIND_ERROR_LEN);
    while end > 0 && !name.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    name.get(..end).unwrap_or_default()
}

impl ZbusSessionSource {
    /// Creates the client; performs no I/O.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a connection is currently held.
    pub async fn is_connected(&self) -> bool {
        self.slot.lock().await.is_some()
    }

    /// The held connection, opened lazily through the pinned address under
    /// `DBUS_CONNECT_TIMEOUT_MS`. The slot guard is held only for the build or the swap.
    async fn connection(&self) -> Result<zbus::Connection, SourceError> {
        let mut slot = self.slot.lock().await;
        if let Some(connection) = slot.as_ref() {
            return Ok(connection.clone());
        }
        let builder = zbus::connection::Builder::address(SYSTEM_BUS_ADDRESS)
            .map_err(|_| SourceError::BusUnavailable)?
            .method_timeout(Duration::from_millis(DBUS_CALL_TIMEOUT_MS))
            .max_queued(MAX_QUEUED_MESSAGES);
        match tokio::time::timeout(
            Duration::from_millis(DBUS_CONNECT_TIMEOUT_MS),
            builder.build(),
        )
        .await
        {
            Ok(Ok(connection)) => {
                *slot = Some(connection.clone());
                Ok(connection)
            }
            Ok(Err(_)) => Err(SourceError::BusUnavailable),
            Err(_) => Err(SourceError::Timeout),
        }
    }

    /// Drops the cached connection (the next call opens a new one).
    async fn reset(&self) {
        *self.slot.lock().await = None;
    }

    /// One bounded method call to logind (destination always the well-known logind name;
    /// object paths come from logind replies, never from string formatting).
    async fn call<B>(
        &self,
        path: &str,
        interface: &str,
        method: &'static str,
        body: &B,
    ) -> Result<zbus::Message, CallFailure>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType + Sync,
    {
        let connection = self.connection().await.map_err(CallFailure::Transport)?;
        let reply = tokio::time::timeout(
            Duration::from_millis(DBUS_CALL_TIMEOUT_MS),
            connection.call_method(
                Some(LOGIND_DESTINATION),
                path,
                Some(interface),
                method,
                body,
            ),
        )
        .await;
        match reply {
            Ok(Ok(message)) => Ok(message),
            Ok(Err(zbus::Error::MethodError(name, _, _))) => {
                Err(CallFailure::Method(name.as_str().to_string()))
            }
            Ok(Err(_)) => {
                self.reset().await;
                Err(CallFailure::Transport(SourceError::BusUnavailable))
            }
            Err(_) => {
                self.reset().await;
                Err(CallFailure::Transport(SourceError::Timeout))
            }
        }
    }

    /// Maps a method error: the name is logged at `debug!` (truncated), the message body
    /// never.
    fn method_error(name: &str) -> SourceError {
        tracing::debug!(
            dbus_error = truncated_error_name(name),
            "logind refused a call"
        );
        SourceError::Call
    }

    /// Decodes a reply body; a shape mismatch drops the connection.
    async fn decode<T>(&self, message: &zbus::Message) -> Result<T, SourceError>
    where
        T: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        match message.body().deserialize::<T>() {
            Ok(value) => Ok(value),
            Err(_) => {
                self.reset().await;
                Err(SourceError::Malformed)
            }
        }
    }

    /// One `Manager.<method>(id)` call on a validated session id (lock or unlock).
    async fn session_action(&self, id: &str, method: &'static str) -> Result<(), SourceError> {
        if !is_valid_session_id(id) {
            return Err(SourceError::Malformed);
        }
        match self
            .call(LOGIND_MANAGER_PATH, LOGIND_MANAGER_IFACE, method, &(id,))
            .await
        {
            Ok(_) => Ok(()),
            Err(CallFailure::Method(name)) => Err(Self::method_error(&name)),
            Err(CallFailure::Transport(err)) => Err(err),
        }
    }

    /// `Properties.GetAll` of one own session object; `Ok(None)` when it vanished.
    async fn session_properties(
        &self,
        path: &str,
    ) -> Result<Option<HashMap<String, OwnedValue>>, SourceError> {
        match self
            .call(path, PROPERTIES_IFACE, "GetAll", &(LOGIND_SESSION_IFACE,))
            .await
        {
            Ok(message) => self.decode(&message).await.map(Some),
            Err(CallFailure::Method(name)) if VANISHED_ERRORS.contains(&name.as_str()) => Ok(None),
            Err(CallFailure::Method(name)) => Err(Self::method_error(&name)),
            Err(CallFailure::Transport(err)) => Err(err),
        }
    }
}

impl SessionSource for ZbusSessionSource {
    async fn own_sessions(&self, uid: u32) -> Result<Vec<SessionProps>, SourceError> {
        let listing = match self
            .call(
                LOGIND_MANAGER_PATH,
                LOGIND_MANAGER_IFACE,
                "ListSessions",
                &(),
            )
            .await
        {
            Ok(message) => message,
            Err(CallFailure::Method(name)) => return Err(Self::method_error(&name)),
            Err(CallFailure::Transport(err)) => return Err(err),
        };
        let sessions: Vec<(String, u32, String, String, OwnedObjectPath)> =
            self.decode(&listing).await?;
        if sessions.len() > MAX_LISTED_SESSIONS {
            return Err(SourceError::TooManySessions);
        }
        let own: Vec<(String, OwnedObjectPath)> = sessions
            .into_iter()
            .filter(|(_, session_uid, _, _, _)| *session_uid == uid)
            .filter(|(id, _, _, _, _)| is_valid_session_id(id))
            .map(|(id, _, _, _, path)| (id, path))
            .collect();
        if own.len() > MAX_OWN_SESSIONS {
            return Err(SourceError::TooManySessions);
        }
        let mut states = Vec::with_capacity(own.len());
        for (id, path) in own {
            if let Some(properties) = self.session_properties(path.as_str()).await? {
                match session_props_from_properties(&id, &properties) {
                    Ok(props) => states.push(props),
                    Err(err) => {
                        self.reset().await;
                        return Err(err);
                    }
                }
            }
        }
        Ok(states)
    }

    async fn lock_session(&self, id: &str) -> Result<(), SourceError> {
        self.session_action(id, "LockSession").await
    }

    async fn unlock_session(&self, id: &str) -> Result<(), SourceError> {
        self.session_action(id, "UnlockSession").await
    }
}
