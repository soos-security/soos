//! Contract test of GitHub #346 for the `GET|HEAD /api/battery` route of `soos-remote` (ADR
//! 2026-10-07 "Live Battery Level in `soos-remote`" item (4), architect spec
//! `AI/architect_spec_remote_battery.md` §6.1, §10.3 test 19; matrix RBS8).
//!
//! The spec maps test 19 to `routes_tests.rs`; it lives in this new file (spec name
//! unchanged, the #345 precedent) so the existing routes suite keeps compiling while the new
//! API is missing. Pure functions only.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_remote::http::{parse_request_head, Method};
use soos_remote::routes::{
    accepts_body, allow_header, is_funnel_public, route, Route, BATTERY_PATH,
};

/// Test 19 (RBS8, B-5): GET and HEAD (query ignored) reach `Route::Battery`; every other
/// method is `405` with `Allow: GET, HEAD`; near paths are `404`; the route is not
/// Funnel-public and takes no body.
#[test]
fn test_rbs_battery_route() {
    assert_eq!(BATTERY_PATH, "/api/battery");
    for method in [Method::Get, Method::Head] {
        assert_eq!(route(method, "/api/battery"), Route::Battery);
        assert_eq!(route(method, "/api/battery?x=1"), Route::Battery);
        assert_eq!(route(method, BATTERY_PATH), Route::Battery);
    }
    for method in [Method::Post, Method::Other] {
        assert_eq!(
            route(method, "/api/battery"),
            Route::MethodNotAllowed,
            "{method:?}"
        );
        assert_eq!(
            route(method, "/api/battery?x=1"),
            Route::MethodNotAllowed,
            "{method:?}"
        );
    }
    assert_eq!(allow_header("/api/battery"), "GET, HEAD");
    assert_eq!(allow_header("/api/battery?x=1"), "GET, HEAD");
    for near in [
        "/api/battery/",
        "/API/BATTERY",
        "/api/Battery",
        "/api/battery2",
        "/api/batter",
        "/api/battery/status",
        "/battery",
    ] {
        for method in [Method::Get, Method::Head, Method::Post, Method::Other] {
            assert_eq!(route(method, near), Route::NotFound, "{method:?} {near}");
        }
    }
    let parsed =
        parse_request_head(b"GET /api/battery?v=2 HTTP/1.1\r\nHost: x.ts.net\r\n\r\n").unwrap();
    assert_eq!(parsed.path, "/api/battery");
    assert_eq!(route(parsed.method, &parsed.path), Route::Battery);
    assert!(
        !is_funnel_public(Route::Battery),
        "visibility equals /api/status"
    );
    assert!(!is_funnel_public(Route::Status));
    assert!(!accepts_body(Method::Post, BATTERY_PATH));
    assert!(!accepts_body(Method::Get, BATTERY_PATH));
    // The existing read routes are unchanged.
    assert_eq!(route(Method::Get, "/api/status"), Route::Status);
    assert_eq!(route(Method::Get, "/api/events"), Route::Events);
}
