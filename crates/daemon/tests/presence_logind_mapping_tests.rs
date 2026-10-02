//! Contract tests of GitHub #323 for the pure mapping of a logind `Properties.GetAll` reply
//! to a presence session state (matrix PAU18, PAU27 `Name`, PAU14 error text bound).
//!
//! No bus is needed: replies are hand-built `zvariant::OwnedValue` maps.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::collections::HashMap;

use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use soos_daemon::presence::logind::{
    session_state_from_properties, PresenceLogindError, SessionId,
};
use soos_daemon::presence::MAX_LOGIND_ERROR_LEN;
use soos_daemon::session_policy::SessionRecord;

fn ov<'a>(value: impl Into<Value<'a>>) -> OwnedValue {
    OwnedValue::try_from(value.into()).unwrap()
}

fn id(raw: &str) -> SessionId {
    SessionId::parse(raw).unwrap()
}

/// A `GetAll` reply of an active, local, seat-attached, locked user session of UID 1000.
fn reply(session_id: &str) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    map.insert("Id".to_string(), ov(session_id));
    map.insert(
        "User".to_string(),
        ov((
            1000u32,
            ObjectPath::try_from("/org/freedesktop/login1/user/_1000").unwrap(),
        )),
    );
    map.insert("Name".to_string(), ov("alice"));
    map.insert("Active".to_string(), ov(true));
    map.insert("State".to_string(), ov("active"));
    map.insert("Remote".to_string(), ov(false));
    map.insert(
        "Seat".to_string(),
        ov((
            "seat0",
            ObjectPath::try_from("/org/freedesktop/login1/seat/seat0").unwrap(),
        )),
    );
    map.insert("Class".to_string(), ov("user"));
    map.insert("LockedHint".to_string(), ov(true));
    map
}

fn map(props: &HashMap<String, OwnedValue>) -> soos_daemon::presence::logind::LogindSessionState {
    session_state_from_properties(&id("4"), props).expect("well-formed reply")
}

/// PAU18: a complete reply maps to a bound, locked state with the owner's name.
#[test]
fn test_pau_get_all_reply_maps_to_a_bound_locked_state() {
    let state = map(&reply("4"));
    assert_eq!(state.id, id("4"));
    assert_eq!(
        state.record,
        SessionRecord {
            uid: Some(1000),
            active: true,
            remote: Some(false),
            seat: Some("seat0".into()),
            class: Some("user".into()),
        }
    );
    assert!(state.locked);
    assert_eq!(state.user_name.as_ref().map(|n| n.as_str()), Some("alice"));
    assert!(state.record.check_local_seat_session_of(1000).is_ok());
}

/// PAU18: an `Id` different from the queried one is `Malformed`.
#[test]
fn test_pau_id_mismatch_is_malformed() {
    assert_eq!(
        session_state_from_properties(&id("4"), &reply("5")),
        Err(PresenceLogindError::Malformed)
    );
    let mut missing = reply("4");
    missing.remove("Id");
    assert_eq!(
        session_state_from_properties(&id("4"), &missing),
        Err(PresenceLogindError::Malformed),
        "a reply without Id cannot be matched to the queried session"
    );
}

/// PAU18: `active = Active || State == "active"` (same rule as `SessionRecord::parse`).
#[test]
fn test_pau_active_follows_the_session_record_rule() {
    let mut only_state = reply("4");
    only_state.insert("Active".to_string(), ov(false));
    assert!(map(&only_state).record.active, "State = active suffices");

    let mut only_flag = reply("4");
    only_flag.insert("State".to_string(), ov("online"));
    assert!(map(&only_flag).record.active, "Active = true suffices");

    let mut neither = reply("4");
    neither.insert("Active".to_string(), ov(false));
    neither.insert("State".to_string(), ov("online"));
    assert!(!map(&neither).record.active);

    let mut absent = reply("4");
    absent.remove("Active");
    absent.remove("State");
    assert!(!map(&absent).record.active);
}

/// PAU18: `Remote` counts only as a boolean; missing or ill-typed ⇒ `None` (denied by the
/// binding, never treated as local).
#[test]
fn test_pau_remote_is_only_trusted_as_a_boolean() {
    let mut remote = reply("4");
    remote.insert("Remote".to_string(), ov(true));
    assert_eq!(map(&remote).record.remote, Some(true));

    let mut missing = reply("4");
    missing.remove("Remote");
    let state = map(&missing);
    assert_eq!(state.record.remote, None);
    assert!(state.record.check_local_seat_session_of(1000).is_err());

    for ill_typed in [ov("0"), ov(0u32), ov("false")] {
        let mut props = reply("4");
        props.insert("Remote".to_string(), ill_typed);
        let state = map(&props);
        assert_eq!(state.record.remote, None);
        assert!(
            state.record.check_local_seat_session_of(1000).is_err(),
            "an ill-typed Remote must be refused by the binding"
        );
    }
}

/// PAU18: an empty seat or class maps to `None`; a missing seat or class too.
#[test]
fn test_pau_empty_seat_and_class_map_to_none() {
    let mut empty_seat = reply("4");
    empty_seat.insert(
        "Seat".to_string(),
        ov(("", ObjectPath::try_from("/").unwrap())),
    );
    assert_eq!(map(&empty_seat).record.seat, None);

    let mut empty_class = reply("4");
    empty_class.insert("Class".to_string(), ov(""));
    assert_eq!(map(&empty_class).record.class, None);

    let mut missing = reply("4");
    missing.remove("Seat");
    missing.remove("Class");
    let state = map(&missing);
    assert_eq!(state.record.seat, None);
    assert_eq!(state.record.class, None);
    assert!(state.record.check_local_seat_session_of(1000).is_err());

    let mut greeter = reply("4");
    greeter.insert("Class".to_string(), ov("greeter"));
    assert_eq!(map(&greeter).record.class.as_deref(), Some("greeter"));
}

/// PAU18: a missing or ill-typed `LockedHint` is "not locked" (never scans).
#[test]
fn test_pau_missing_or_ill_typed_locked_hint_is_not_locked() {
    let mut missing = reply("4");
    missing.remove("LockedHint");
    assert!(!map(&missing).locked);

    for ill_typed in [ov("true"), ov(1u32), ov(1u8)] {
        let mut props = reply("4");
        props.insert("LockedHint".to_string(), ill_typed);
        assert!(!map(&props).locked, "an ill-typed LockedHint is not locked");
    }

    let mut unlocked = reply("4");
    unlocked.insert("LockedHint".to_string(), ov(false));
    assert!(!map(&unlocked).locked);
}

/// PAU18: a missing or ill-typed `User` leaves the UID unset (the binding refuses).
#[test]
fn test_pau_missing_or_ill_typed_user_leaves_uid_unset() {
    let mut missing = reply("4");
    missing.remove("User");
    let state = map(&missing);
    assert_eq!(state.record.uid, None);
    assert!(state.record.check_local_seat_session_of(1000).is_err());

    let mut ill_typed = reply("4");
    ill_typed.insert("User".to_string(), ov(1000u32));
    assert_eq!(map(&ill_typed).record.uid, None);
}

/// PAU27: a missing, ill-typed or invalid `Name` maps to `None` (no unlock downstream).
#[test]
fn test_pau_missing_or_invalid_name_maps_to_none() {
    let mut missing = reply("4");
    missing.remove("Name");
    assert!(map(&missing).user_name.is_none());

    for invalid in [ov(""), ov("../etc"), ov("a/b"), ov("-x"), ov(1000u32)] {
        let mut props = reply("4");
        props.insert("Name".to_string(), invalid);
        assert!(
            map(&props).user_name.is_none(),
            "an invalid Name must never become a UserName"
        );
    }
}

/// PAU14: `PresenceLogindError::Call` text is bounded by `MAX_LOGIND_ERROR_LEN` bytes and
/// truncated on a char boundary; short messages are kept.
#[test]
fn test_pau_logind_call_error_text_is_bounded() {
    let short = PresenceLogindError::call("org.freedesktop.login1.NoSuchSession");
    assert_eq!(
        short,
        PresenceLogindError::Call("org.freedesktop.login1.NoSuchSession".into())
    );

    let long = "é".repeat(MAX_LOGIND_ERROR_LEN);
    let PresenceLogindError::Call(text) = PresenceLogindError::call(&long) else {
        panic!("call() must build the Call variant");
    };
    assert!(text.len() <= MAX_LOGIND_ERROR_LEN, "{} bytes", text.len());
    assert!(!text.is_empty());
    assert!(long.starts_with(&text), "truncation keeps a prefix");

    let ascii = "x".repeat(MAX_LOGIND_ERROR_LEN + 100);
    let PresenceLogindError::Call(text) = PresenceLogindError::call(&ascii) else {
        panic!("call() must build the Call variant");
    };
    assert_eq!(text.len(), MAX_LOGIND_ERROR_LEN);

    // Display stays bounded too (prefix + bounded text).
    let shown = PresenceLogindError::call(&ascii).to_string();
    assert!(shown.len() <= MAX_LOGIND_ERROR_LEN + "logind call failed: ".len());
}
