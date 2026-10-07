//! Status view, readings and change detection (spec §2.6, D6, D8).
//!
//! The view is the only thing the phone ever sees: a state, two flags, an optional idle
//! timestamp and the read time. It never carries the uid, the session id, the seat or any
//! identity (RC-5), and a logind failure is `unavailable`, never `unlocked` (RC-3).

use crate::logind::SourceError;
use crate::session::{select_session, SessionProps};

/// Session state as shown to the phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// `LockedHint == true` on the selected session.
    Locked,
    /// Fresh read returned `LockedHint == false` on the selected session.
    Unlocked,
    /// No local seat session of the own uid.
    NoSession,
    /// Any logind failure.
    Unavailable,
}

/// JSON body of `/api/status` and of every SSE event. Never contains the uid, user name,
/// session id, seat or any identity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StatusView {
    /// State.
    pub state: SessionState,
    /// `false` unless state ∈ {Locked, Unlocked}.
    pub active: bool,
    /// `false` unless state ∈ {Locked, Unlocked}.
    pub idle: bool,
    /// `None` unless idle.
    pub idle_since_unix_s: Option<u64>,
    /// Unix time of the read in ms; 0 if the clock is before the epoch.
    pub checked_unix_ms: u64,
}

impl StatusView {
    /// A view without a selected session (`NoSession` or `Unavailable`).
    fn neutral(state: SessionState, checked_unix_ms: u64) -> Self {
        Self {
            state,
            active: false,
            idle: false,
            idle_since_unix_s: None,
            checked_unix_ms,
        }
    }
}

/// Pure; D8. `Ok(sessions)` → `select_session` → Locked/Unlocked/NoSession; `Err(_)` →
/// Unavailable. `checked_unix_ms` passes through unchanged.
#[must_use]
pub fn status_from(
    result: &Result<Vec<SessionProps>, SourceError>,
    uid: u32,
    checked_unix_ms: u64,
) -> StatusView {
    let Ok(sessions) = result else {
        return StatusView::neutral(SessionState::Unavailable, checked_unix_ms);
    };
    match select_session(sessions, uid) {
        None => StatusView::neutral(SessionState::NoSession, checked_unix_ms),
        Some(session) => StatusView {
            state: if session.locked {
                SessionState::Locked
            } else {
                SessionState::Unlocked
            },
            active: session.active,
            idle: session.idle,
            idle_since_unix_s: if session.idle {
                session.idle_since_unix_s
            } else {
                None
            },
            checked_unix_ms,
        },
    }
}

/// One logind read as carried on the watch channel (internal, not serialized).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// Strictly increasing per process (shared counter, reserved when the read starts).
    pub seq: u64,
    /// The view built from the read.
    pub view: StatusView,
}

/// Pure: true when the two views differ in anything but `checked_unix_ms`.
#[must_use]
pub fn view_changed(previous: &StatusView, next: &StatusView) -> bool {
    previous.state != next.state
        || previous.active != next.active
        || previous.idle != next.idle
        || previous.idle_since_unix_s != next.idle_since_unix_s
}
