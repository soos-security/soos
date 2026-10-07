//! Pure session projection and selection (spec §2.6, D7).
//!
//! Mirrors `soos-daemon`'s `presence/logind.rs::session_state_from_properties`: every
//! property is read defensively (an ill-typed value degrades to "unknown", never to a
//! wrong positive), and the selection keeps only the owner's local seat session.

use std::collections::HashMap;
use std::hash::BuildHasher;

use zbus::zvariant::{OwnedValue, Value};

use crate::logind::SourceError;
use crate::MAX_SESSION_ID_LEN;

/// Pure projection of one session's logind properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionProps {
    /// Validated: ASCII alphanumeric, 1..=`MAX_SESSION_ID_LEN`.
    pub id: String,
    /// `User` first field.
    pub uid: Option<u32>,
    /// `Active == true || State == "active"`.
    pub active: bool,
    /// `Some` only for a boolean `Remote`.
    pub remote: Option<bool>,
    /// Non-empty seat only.
    pub seat: Option<String>,
    /// Non-empty class only.
    pub class: Option<String>,
    /// `LockedHint` must be boolean `true`.
    pub locked: bool,
    /// `IdleHint` must be boolean `true`.
    pub idle: bool,
    /// `IdleSinceHint / 1_000_000`; 0 or ill-typed → `None`.
    pub idle_since_unix_s: Option<u64>,
}

/// Microseconds per second (`IdleSinceHint` is in µs).
const MICROS_PER_SECOND: u64 = 1_000_000;

/// A logind session id: ASCII alphanumeric, 1..=`MAX_SESSION_ID_LEN` bytes.
#[must_use]
pub fn is_valid_session_id(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= MAX_SESSION_ID_LEN
        && raw.bytes().all(|b| b.is_ascii_alphanumeric())
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

/// Unsigned 64-bit content of a property.
fn as_u64(value: &OwnedValue) -> Option<u64> {
    match &**value {
        Value::U64(n) => Some(*n),
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

/// `Some(s)` unless empty.
fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// Pure mapping of a `Properties.GetAll(Session)` reply.
///
/// # Errors
///
/// [`SourceError::Malformed`] when `expected_id` is not a valid session id or `Id` is
/// missing, ill-typed or differs from `expected_id`.
pub fn session_props_from_properties<S: BuildHasher>(
    expected_id: &str,
    properties: &HashMap<String, OwnedValue, S>,
) -> Result<SessionProps, SourceError> {
    if !is_valid_session_id(expected_id) {
        return Err(SourceError::Malformed);
    }
    let id = properties
        .get("Id")
        .and_then(as_str)
        .ok_or(SourceError::Malformed)?;
    if id != expected_id {
        return Err(SourceError::Malformed);
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
    let idle = properties.get("IdleHint").and_then(as_bool) == Some(true);
    let idle_since_unix_s = properties
        .get("IdleSinceHint")
        .and_then(as_u64)
        .filter(|micros| *micros > 0)
        .and_then(|micros| micros.checked_div(MICROS_PER_SECOND));
    Ok(SessionProps {
        id: expected_id.to_string(),
        uid,
        active,
        remote,
        seat,
        class,
        locked,
        idle,
        idle_since_unix_s,
    })
}

/// Pure; D7. Candidates are the sessions of `uid` with `Class == "user"`, an explicit
/// `Remote == false` and a non-empty seat; active ones are preferred; ties are broken by
/// the shortest id (`"9"` before `"10"`) and, among ids of the same length, by the
/// greatest byte sequence (the contract's `"c9"` before `"10"`: for same-length numeric
/// ids this is the most recently allocated session). The choice is deterministic for any
/// input order.
#[must_use]
pub fn select_session(sessions: &[SessionProps], uid: u32) -> Option<&SessionProps> {
    let candidates = sessions.iter().filter(|s| {
        s.uid == Some(uid)
            && s.class.as_deref() == Some("user")
            && s.remote == Some(false)
            && s.seat.is_some()
    });
    candidates.min_by_key(|s| (!s.active, s.id.len(), std::cmp::Reverse(s.id.as_bytes())))
}
