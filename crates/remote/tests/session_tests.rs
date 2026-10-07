//! Contract tests of GitHub #339 for the pure session mapping, selection, status view and
//! change detection (spec §2.6, D7, D8, RC-3, RC-5).
//!
//! No bus is needed: `GetAll` replies are hand-built `zvariant::OwnedValue` maps.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::collections::HashMap;

use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use soos_remote::logind::SourceError;
use soos_remote::session::{select_session, session_props_from_properties, SessionProps};
use soos_remote::status::{status_from, view_changed, Reading, SessionState, StatusView};
use soos_remote::MAX_SESSION_ID_LEN;

const UID: u32 = 1000;

fn ov<'a>(value: impl Into<Value<'a>>) -> OwnedValue {
    OwnedValue::try_from(value.into()).unwrap()
}

/// A `GetAll` reply of an active, local, seat-attached, unlocked, idle user session.
fn reply(id: &str) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    map.insert("Id".to_string(), ov(id));
    map.insert(
        "User".to_string(),
        ov((
            UID,
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
    map.insert("LockedHint".to_string(), ov(false));
    map.insert("IdleHint".to_string(), ov(true));
    map.insert("IdleSinceHint".to_string(), ov(1_700_000_000_123_456u64));
    map
}

fn props(id: &str) -> SessionProps {
    SessionProps {
        id: id.to_string(),
        uid: Some(UID),
        active: true,
        remote: Some(false),
        seat: Some("seat0".to_string()),
        class: Some("user".to_string()),
        locked: false,
        idle: false,
        idle_since_unix_s: None,
    }
}

fn view(state: SessionState, active: bool, idle: bool, since: Option<u64>, ms: u64) -> StatusView {
    StatusView {
        state,
        active,
        idle,
        idle_since_unix_s: since,
        checked_unix_ms: ms,
    }
}

// ---------------------------------------------------------------------------------------
// session_props_from_properties
// ---------------------------------------------------------------------------------------

/// §2.6: a complete reply maps every field.
#[test]
fn test_rmc_session_props_from_get_all_reply_nominal() {
    let mapped = session_props_from_properties("c0ffee42", &reply("c0ffee42")).unwrap();
    assert_eq!(
        mapped,
        SessionProps {
            id: "c0ffee42".to_string(),
            uid: Some(UID),
            active: true,
            remote: Some(false),
            seat: Some("seat0".to_string()),
            class: Some("user".to_string()),
            locked: false,
            idle: true,
            idle_since_unix_s: Some(1_700_000_000),
        }
    );
    let mut locked = reply("c0ffee42");
    locked.insert("LockedHint".to_string(), ov(true));
    assert!(
        session_props_from_properties("c0ffee42", &locked)
            .unwrap()
            .locked
    );
}

/// §2.6: a missing, ill-typed or mismatching `Id` is `Malformed`.
#[test]
fn test_rmc_session_props_malformed_id() {
    let mut missing = reply("4");
    missing.remove("Id");
    assert_eq!(
        session_props_from_properties("4", &missing),
        Err(SourceError::Malformed)
    );
    let mut ill_typed = reply("4");
    ill_typed.insert("Id".to_string(), ov(4u32));
    assert_eq!(
        session_props_from_properties("4", &ill_typed),
        Err(SourceError::Malformed)
    );
    assert_eq!(
        session_props_from_properties("5", &reply("4")),
        Err(SourceError::Malformed)
    );
    assert_eq!(
        session_props_from_properties("4", &HashMap::new()),
        Err(SourceError::Malformed)
    );
}

/// §2.6: optional properties degrade to `None`/`false`, never to a wrong positive.
#[test]
fn test_rmc_session_props_tolerates_missing_or_ill_typed_optionals() {
    let mut r = reply("4");
    r.remove("User");
    r.remove("Active");
    r.insert("State".to_string(), ov("active"));
    r.insert("Remote".to_string(), ov("false"));
    r.insert(
        "Seat".to_string(),
        ov((
            "",
            ObjectPath::try_from("/org/freedesktop/login1/seat/seat0").unwrap(),
        )),
    );
    r.insert("Class".to_string(), ov(""));
    r.insert("LockedHint".to_string(), ov("true"));
    r.remove("IdleHint");
    r.insert("IdleSinceHint".to_string(), ov(0u64));
    let mapped = session_props_from_properties("4", &r).unwrap();
    assert_eq!(mapped.uid, None);
    assert!(mapped.active, "State == active counts as active");
    assert_eq!(
        mapped.remote, None,
        "a non-boolean Remote is unknown, not false"
    );
    assert_eq!(mapped.seat, None);
    assert_eq!(mapped.class, None);
    assert!(!mapped.locked, "a string LockedHint is not locked");
    assert!(!mapped.idle);
    assert_eq!(mapped.idle_since_unix_s, None, "IdleSinceHint 0 is None");

    let mut r = reply("4");
    r.insert("Active".to_string(), ov(false));
    r.insert("State".to_string(), ov("online"));
    r.insert("User".to_string(), ov("1000"));
    r.insert("IdleHint".to_string(), ov("true"));
    r.insert("IdleSinceHint".to_string(), ov(5i32));
    let mapped = session_props_from_properties("4", &r).unwrap();
    assert!(!mapped.active);
    assert_eq!(mapped.uid, None, "a non-structure User is unknown");
    assert!(!mapped.idle, "a string IdleHint is not idle");
    assert_eq!(
        mapped.idle_since_unix_s, None,
        "an ill-typed IdleSinceHint is None"
    );

    let mut r = reply("4");
    r.insert("IdleSinceHint".to_string(), ov(999_999u64));
    let mapped = session_props_from_properties("4", &r).unwrap();
    assert_eq!(
        mapped.idle_since_unix_s,
        Some(0),
        "microseconds truncate to whole seconds"
    );
}

/// §2.6: the id is validated against `MAX_SESSION_ID_LEN` and the alphanumeric rule.
#[test]
fn test_rmc_session_props_validates_the_id() {
    let max = "a".repeat(MAX_SESSION_ID_LEN);
    assert!(session_props_from_properties(&max, &reply(&max)).is_ok());
    for bad in [
        "".to_string(),
        "a".repeat(MAX_SESSION_ID_LEN + 1),
        "c0ffee-42".to_string(),
        "4/5".to_string(),
        "é".to_string(),
        "4 ".to_string(),
    ] {
        assert_eq!(
            session_props_from_properties(&bad, &reply(&bad)),
            Err(SourceError::Malformed),
            "{bad:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// select_session
// ---------------------------------------------------------------------------------------

/// D7: own uid, `Class == user`, explicit `Remote == false`, non-empty seat; active first;
/// ties broken by (byte length, bytes) so `"9" < "10"`.
#[test]
fn test_rmc_select_session_prefers_active_local_user_seat_session() {
    let mut inactive_10 = props("10");
    inactive_10.active = false;
    let mut inactive_9 = props("9");
    inactive_9.active = false;
    let mut inactive_c9 = props("c9");
    inactive_c9.active = false;
    let active_12 = props("12");
    let active_3 = props("3");

    let sessions = vec![
        inactive_10.clone(),
        active_12.clone(),
        inactive_9.clone(),
        active_3.clone(),
    ];
    assert_eq!(
        select_session(&sessions, UID),
        Some(&active_3),
        "smallest active id"
    );

    let sessions = vec![inactive_10.clone(), inactive_9.clone(), inactive_c9.clone()];
    assert_eq!(
        select_session(&sessions, UID),
        Some(&inactive_9),
        "no active session: shortest id, then bytes"
    );
    let sessions = vec![inactive_10.clone(), inactive_c9.clone()];
    assert_eq!(
        select_session(&sessions, UID),
        Some(&inactive_c9),
        "\"c9\" < \"10\" on bytes"
    );
    assert_eq!(select_session(&[], UID), None);
    let only = vec![active_12.clone()];
    assert_eq!(select_session(&only, UID), Some(&active_12));
}

/// D7: every exclusion rule, each one alone turning a perfect candidate into `None`.
#[test]
fn test_rmc_select_session_excludes_remote_greeter_seatless_and_other_uids() {
    type Mutation = Box<dyn Fn(&mut SessionProps)>;
    let exclusions: Vec<(&str, Mutation)> = vec![
        ("other uid", Box::new(|p| p.uid = Some(UID + 1))),
        ("unknown uid", Box::new(|p| p.uid = None)),
        ("remote", Box::new(|p| p.remote = Some(true))),
        ("remote unknown", Box::new(|p| p.remote = None)),
        ("no seat", Box::new(|p| p.seat = None)),
        (
            "greeter",
            Box::new(|p| p.class = Some("greeter".to_string())),
        ),
        (
            "manager",
            Box::new(|p| p.class = Some("manager".to_string())),
        ),
        (
            "background",
            Box::new(|p| p.class = Some("background".to_string())),
        ),
        ("no class", Box::new(|p| p.class = None)),
        (
            "User class",
            Box::new(|p| p.class = Some("User".to_string())),
        ),
    ];
    for (label, mutate) in exclusions {
        let mut p = props("4");
        mutate(&mut p);
        assert_eq!(select_session(&[p.clone()], UID), None, "{label}");
        // An active excluded session never beats an inactive valid one.
        let mut valid = props("7");
        valid.active = false;
        let sessions = vec![p, valid.clone()];
        assert_eq!(select_session(&sessions, UID), Some(&valid), "{label}");
    }
}

// ---------------------------------------------------------------------------------------
// status_from / view_changed / JSON
// ---------------------------------------------------------------------------------------

/// D8: `Ok` maps through `select_session`; every `Err` is `Unavailable` with neutral flags;
/// `checked_unix_ms` passes through.
#[test]
fn test_rmc_status_from_maps_results_fail_safe() {
    let mut locked_idle = props("4");
    locked_idle.locked = true;
    locked_idle.idle = true;
    locked_idle.idle_since_unix_s = Some(1_700_000_000);
    assert_eq!(
        status_from(&Ok(vec![locked_idle]), UID, 42),
        view(SessionState::Locked, true, true, Some(1_700_000_000), 42)
    );
    let mut unlocked_inactive = props("4");
    unlocked_inactive.active = false;
    unlocked_inactive.idle_since_unix_s = Some(1_700_000_000);
    assert_eq!(
        status_from(&Ok(vec![unlocked_inactive]), UID, 43),
        view(SessionState::Unlocked, false, false, None, 43),
        "idle_since is None unless idle"
    );
    assert_eq!(
        status_from(&Ok(vec![]), UID, 44),
        view(SessionState::NoSession, false, false, None, 44)
    );
    let mut remote = props("4");
    remote.remote = Some(true);
    remote.idle = true;
    remote.idle_since_unix_s = Some(1);
    assert_eq!(
        status_from(&Ok(vec![remote]), UID, 45),
        view(SessionState::NoSession, false, false, None, 45),
        "an excluded session leaks nothing"
    );
    for err in [
        SourceError::BusUnavailable,
        SourceError::Timeout,
        SourceError::Call,
        SourceError::Malformed,
        SourceError::TooManySessions,
    ] {
        assert_eq!(
            status_from(&Err(err.clone()), UID, 46),
            view(SessionState::Unavailable, false, false, None, 46),
            "{err:?}"
        );
    }
    assert_eq!(
        status_from(&Ok(vec![props("4")]), UID, 0).checked_unix_ms,
        0
    );
    assert_eq!(
        status_from(&Ok(vec![props("4")]), UID, u64::MAX).checked_unix_ms,
        u64::MAX
    );
}

/// D6: change detection ignores `checked_unix_ms` and nothing else.
#[test]
fn test_rmc_view_changed_ignores_checked_unix_ms_only() {
    let base = view(SessionState::Unlocked, true, false, None, 1);
    assert!(!view_changed(&base, &base));
    assert!(!view_changed(
        &base,
        &view(SessionState::Unlocked, true, false, None, 99_999)
    ));
    assert!(view_changed(
        &base,
        &view(SessionState::Locked, true, false, None, 1)
    ));
    assert!(view_changed(
        &base,
        &view(SessionState::Unlocked, false, false, None, 1)
    ));
    assert!(view_changed(
        &base,
        &view(SessionState::Unlocked, true, true, None, 1)
    ));
    assert!(view_changed(
        &view(SessionState::Unlocked, true, true, Some(10), 1),
        &view(SessionState::Unlocked, true, true, Some(11), 1)
    ));
    assert!(view_changed(
        &base,
        &view(SessionState::Unavailable, false, false, None, 1)
    ));
    let reading = Reading {
        seq: 7,
        view: base.clone(),
    };
    assert_eq!(reading.seq, 7);
    assert_eq!(reading.view, base);
}

/// RC-5: the JSON body has exactly the five documented keys, snake_case states, and no
/// identity (uid, session id, seat, name).
#[test]
fn test_rmc_status_view_json_shape_has_no_identity() {
    let json = serde_json::to_value(view(SessionState::Locked, true, false, None, 123)).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "state": "locked",
            "active": true,
            "idle": false,
            "idle_since_unix_s": null,
            "checked_unix_ms": 123
        })
    );
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "active",
            "checked_unix_ms",
            "idle",
            "idle_since_unix_s",
            "state"
        ]
    );
    for (state, text) in [
        (SessionState::Locked, "locked"),
        (SessionState::Unlocked, "unlocked"),
        (SessionState::NoSession, "no_session"),
        (SessionState::Unavailable, "unavailable"),
    ] {
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!(text)
        );
    }
    let text = serde_json::to_string(&view(
        SessionState::Unlocked,
        true,
        true,
        Some(1_700_000_000),
        5,
    ))
    .unwrap();
    assert!(!text.contains('\n'), "one line for SSE data");
    for forbidden in [
        "1000", "c0ffee42", "seat0", "alice", "uid", "session", "seat", "name",
    ] {
        assert!(
            !text.contains(forbidden),
            "{text} must not contain {forbidden}"
        );
    }
    assert!(text.contains("\"idle_since_unix_s\":1700000000"));
}

/// §4: source errors have fixed text and never carry a logind error name.
#[test]
fn test_rmc_source_error_messages_are_fixed() {
    assert_eq!(
        SourceError::BusUnavailable.to_string(),
        "system bus unavailable"
    );
    assert_eq!(SourceError::Timeout.to_string(), "logind call timed out");
    assert_eq!(SourceError::Call.to_string(), "logind call failed");
    assert_eq!(SourceError::Malformed.to_string(), "logind reply malformed");
    assert_eq!(
        SourceError::TooManySessions.to_string(),
        "too many sessions"
    );
}
