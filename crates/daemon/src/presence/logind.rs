//! systemd-logind access for the presence worker (GitHub #323, D1–D3).
//!
//! The lock state (`LockedHint`) is only available over D-Bus, so the daemon reads it from
//! `org.freedesktop.login1` on the pinned system bus address [`SYSTEM_BUS_ADDRESS`]. The
//! connection registers no object server, no well-known name and no signal match; every
//! property is read with a fresh `Properties.GetAll` / `Get` round trip (no cache); every
//! call is bounded by [`DBUS_CALL_TIMEOUT_MS`].

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::account::UserName;
use super::{
    DBUS_CALL_TIMEOUT_MS, DBUS_CONNECT_TIMEOUT_MS, MAX_LOGIND_ERROR_LEN,
    MAX_PRESENCE_SEAT_SESSIONS, SYSTEM_BUS_ADDRESS,
};
use crate::session_policy::{is_valid_session_id, non_empty, SessionRecord, MAX_SCANNED_SESSIONS};

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

/// Validated logind session ID (ASCII alphanumeric, 1..=`MAX_SESSION_ID_LEN` bytes).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(String);

impl SessionId {
    /// `None` for an invalid ID (never sent to logind).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        if is_valid_session_id(raw) {
            Some(Self(raw.to_string()))
        } else {
            None
        }
    }

    /// The ID.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One logind snapshot of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogindSessionState {
    /// Session ID.
    pub id: SessionId,
    /// Same struct and predicate as the `Auth` path (`check_local_seat_session_of`).
    pub record: SessionRecord,
    /// `LockedHint`; `false` when the property is absent or ill-typed (never scans).
    pub locked: bool,
    /// Session property `Name` (owner's user name), validated; `None` when absent,
    /// ill-typed or invalid (the account guard then refuses).
    pub user_name: Option<UserName>,
}

/// Failure of a logind access; never leads to an unlock.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PresenceLogindError {
    /// The system bus could not be reached.
    #[error("system bus unavailable")]
    BusUnavailable,
    /// A call exceeded `DBUS_CALL_TIMEOUT_MS`.
    #[error("logind call timed out")]
    Timeout,
    /// logind answered with an error (name only, at most `MAX_LOGIND_ERROR_LEN` bytes).
    #[error("logind call failed: {0}")]
    Call(String),
    /// The reply did not have the expected shape.
    #[error("logind reply malformed")]
    Malformed,
    /// More sessions than the bounds allow.
    #[error("too many logind sessions")]
    TooManySessions,
}

impl PresenceLogindError {
    /// Builds a `Call` error whose text is truncated to [`MAX_LOGIND_ERROR_LEN`] bytes on a
    /// char boundary.
    #[must_use]
    pub fn call(message: &str) -> Self {
        let mut end = message.len().min(MAX_LOGIND_ERROR_LEN);
        while end > 0 && !message.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        Self::Call(message.get(..end).unwrap_or_default().to_string())
    }
}

/// Mockable logind access for the presence worker.
pub trait PresenceLogind: Send + Sync + 'static {
    /// Every session with a non-empty seat, mapped through
    /// [`session_state_from_properties`]. More than `MAX_SCANNED_SESSIONS` listed, or more
    /// than `MAX_PRESENCE_SEAT_SESSIONS` seat sessions, is `TooManySessions`; invalid IDs
    /// are skipped; a session that vanished between the listing and its properties is
    /// skipped; any other per-session error fails the whole snapshot.
    fn seat_sessions(
        &self,
    ) -> impl Future<Output = Result<Vec<LogindSessionState>, PresenceLogindError>> + Send;

    /// Fresh state of one session; `Ok(None)` when it no longer exists.
    fn session_state(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = Result<Option<LogindSessionState>, PresenceLogindError>> + Send;

    /// `Manager.LidClosed`.
    fn lid_closed(&self) -> impl Future<Output = Result<bool, PresenceLogindError>> + Send;

    /// `Manager.UnlockSession(id)`.
    fn unlock_session(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = Result<(), PresenceLogindError>> + Send;
}

/// String content of a property.
fn as_str(value: &OwnedValue) -> Option<&str> {
    match &**value {
        Value::Str(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Boolean content of a property.
fn as_bool(value: &OwnedValue) -> Option<bool> {
    match &**value {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

/// First field of a structure property (`User (uo)`, `Seat (so)`).
fn first_field(value: &OwnedValue) -> Option<&Value<'static>> {
    match &**value {
        Value::Structure(s) => s.fields().first(),
        _ => None,
    }
}

/// Pure mapping of a `Properties.GetAll("org.freedesktop.login1.Session")` reply.
///
/// `active = Active || State == "active"` (same rule as `SessionRecord::parse`);
/// `remote = Some(Remote)` only for a boolean; `seat` / `class` through `non_empty`;
/// `LockedHint` must be a boolean `true` to count as locked.
///
/// # Errors
///
/// [`PresenceLogindError::Malformed`] when `Id` is missing, ill-typed or differs from
/// `expected_id`.
pub fn session_state_from_properties<S: std::hash::BuildHasher>(
    expected_id: &SessionId,
    properties: &HashMap<String, OwnedValue, S>,
) -> Result<LogindSessionState, PresenceLogindError> {
    let id = properties
        .get("Id")
        .and_then(as_str)
        .ok_or(PresenceLogindError::Malformed)?;
    if id != expected_id.as_str() {
        return Err(PresenceLogindError::Malformed);
    }
    let uid = properties
        .get("User")
        .and_then(first_field)
        .and_then(|field| match field {
            Value::U32(uid) => Some(*uid),
            _ => None,
        });
    let active = properties.get("Active").and_then(as_bool) == Some(true)
        || properties.get("State").and_then(as_str) == Some("active");
    let remote = properties.get("Remote").and_then(as_bool);
    let seat = properties
        .get("Seat")
        .and_then(first_field)
        .and_then(|field| match field {
            Value::Str(s) => non_empty(s.as_str()),
            _ => None,
        });
    let class = properties.get("Class").and_then(as_str).and_then(non_empty);
    let locked = properties.get("LockedHint").and_then(as_bool) == Some(true);
    let user_name = properties
        .get("Name")
        .and_then(as_str)
        .and_then(UserName::parse);
    Ok(LogindSessionState {
        id: expected_id.clone(),
        record: SessionRecord {
            uid,
            active,
            remote,
            seat,
            class,
        },
        locked,
        user_name,
    })
}

/// Production logind access over the pinned system bus (lazy connection, no I/O before
/// the first call).
#[derive(Debug, Default)]
pub struct ZbusLogind {
    slot: tokio::sync::Mutex<Option<zbus::Connection>>,
}

/// One logind call failure, before mapping.
enum CallFailure {
    /// logind answered with this error name.
    Method(String),
    /// Transport failure or timeout: the connection is dropped.
    Transport(PresenceLogindError),
}

impl ZbusLogind {
    /// Creates the client; performs no I/O (the connection is opened on first use).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The pinned system bus address (never derived from the environment).
    #[must_use]
    pub const fn address(&self) -> &'static str {
        SYSTEM_BUS_ADDRESS
    }

    /// Returns the cached connection or opens one (bounded by `DBUS_CONNECT_TIMEOUT_MS`).
    async fn connection(&self) -> Result<zbus::Connection, PresenceLogindError> {
        let mut slot = self.slot.lock().await;
        if let Some(connection) = slot.as_ref() {
            return Ok(connection.clone());
        }
        let builder = zbus::connection::Builder::address(self.address())
            .map_err(|_| PresenceLogindError::BusUnavailable)?
            .method_timeout(Duration::from_millis(DBUS_CALL_TIMEOUT_MS))
            .max_queued(MAX_QUEUED_MESSAGES);
        let connection = match tokio::time::timeout(
            Duration::from_millis(DBUS_CONNECT_TIMEOUT_MS),
            builder.build(),
        )
        .await
        {
            Ok(Ok(connection)) => connection,
            Ok(Err(_)) => return Err(PresenceLogindError::BusUnavailable),
            Err(_) => return Err(PresenceLogindError::Timeout),
        };
        *slot = Some(connection.clone());
        Ok(connection)
    }

    /// Drops the cached connection (the next call reconnects).
    async fn reset(&self) {
        *self.slot.lock().await = None;
    }

    /// One bounded method call to logind.
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
                Err(CallFailure::Transport(PresenceLogindError::BusUnavailable))
            }
            Err(_) => {
                self.reset().await;
                Err(CallFailure::Transport(PresenceLogindError::Timeout))
            }
        }
    }

    /// Decodes a reply body; a shape mismatch drops the connection.
    async fn decode<T>(&self, message: &zbus::Message) -> Result<T, PresenceLogindError>
    where
        T: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        match message.body().deserialize::<T>() {
            Ok(value) => Ok(value),
            Err(_) => {
                self.reset().await;
                Err(PresenceLogindError::Malformed)
            }
        }
    }

    /// [`session_state_from_properties`]; a malformed reply drops the connection.
    async fn mapped(
        &self,
        id: &SessionId,
        properties: &HashMap<String, OwnedValue>,
    ) -> Result<LogindSessionState, PresenceLogindError> {
        let mapped = session_state_from_properties(id, properties);
        if mapped.is_err() {
            self.reset().await;
        }
        mapped
    }

    /// `Properties.GetAll` of one session object; `Ok(None)` when it vanished.
    async fn session_properties(
        &self,
        path: &str,
    ) -> Result<Option<HashMap<String, OwnedValue>>, PresenceLogindError> {
        match self
            .call(path, PROPERTIES_IFACE, "GetAll", &(LOGIND_SESSION_IFACE,))
            .await
        {
            Ok(message) => self.decode(&message).await.map(Some),
            Err(CallFailure::Method(name)) if VANISHED_ERRORS.contains(&name.as_str()) => Ok(None),
            Err(CallFailure::Method(name)) => Err(PresenceLogindError::call(&name)),
            Err(CallFailure::Transport(err)) => Err(err),
        }
    }
}

impl PresenceLogind for ZbusLogind {
    async fn seat_sessions(&self) -> Result<Vec<LogindSessionState>, PresenceLogindError> {
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
            Err(CallFailure::Method(name)) => return Err(PresenceLogindError::call(&name)),
            Err(CallFailure::Transport(err)) => return Err(err),
        };
        let sessions: Vec<(String, u32, String, String, OwnedObjectPath)> =
            self.decode(&listing).await?;
        if sessions.len() > MAX_SCANNED_SESSIONS {
            return Err(PresenceLogindError::TooManySessions);
        }
        let seat_sessions: Vec<(SessionId, OwnedObjectPath)> = sessions
            .into_iter()
            .filter(|(_, _, _, seat, _)| !seat.is_empty())
            .filter_map(|(id, _, _, _, path)| SessionId::parse(&id).map(|id| (id, path)))
            .collect();
        if seat_sessions.len() > MAX_PRESENCE_SEAT_SESSIONS {
            return Err(PresenceLogindError::TooManySessions);
        }
        let mut states = Vec::with_capacity(seat_sessions.len());
        for (id, path) in seat_sessions {
            if let Some(properties) = self.session_properties(path.as_str()).await? {
                states.push(self.mapped(&id, &properties).await?);
            }
        }
        Ok(states)
    }

    async fn session_state(
        &self,
        id: &SessionId,
    ) -> Result<Option<LogindSessionState>, PresenceLogindError> {
        let path: OwnedObjectPath = match self
            .call(
                LOGIND_MANAGER_PATH,
                LOGIND_MANAGER_IFACE,
                "GetSession",
                &(id.as_str(),),
            )
            .await
        {
            Ok(message) => self.decode(&message).await?,
            Err(CallFailure::Method(name)) if VANISHED_ERRORS.contains(&name.as_str()) => {
                return Ok(None);
            }
            Err(CallFailure::Method(name)) => return Err(PresenceLogindError::call(&name)),
            Err(CallFailure::Transport(err)) => return Err(err),
        };
        match self.session_properties(path.as_str()).await? {
            Some(properties) => self.mapped(id, &properties).await.map(Some),
            None => Ok(None),
        }
    }

    async fn lid_closed(&self) -> Result<bool, PresenceLogindError> {
        let reply = match self
            .call(
                LOGIND_MANAGER_PATH,
                PROPERTIES_IFACE,
                "Get",
                &(LOGIND_MANAGER_IFACE, "LidClosed"),
            )
            .await
        {
            Ok(message) => message,
            Err(CallFailure::Method(name)) => return Err(PresenceLogindError::call(&name)),
            Err(CallFailure::Transport(err)) => return Err(err),
        };
        let value: OwnedValue = self.decode(&reply).await?;
        as_bool(&value).ok_or(PresenceLogindError::Malformed)
    }

    async fn unlock_session(&self, id: &SessionId) -> Result<(), PresenceLogindError> {
        match self
            .call(
                LOGIND_MANAGER_PATH,
                LOGIND_MANAGER_IFACE,
                "UnlockSession",
                &(id.as_str(),),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(CallFailure::Method(name)) => Err(PresenceLogindError::call(&name)),
            Err(CallFailure::Transport(err)) => Err(err),
        }
    }
}
