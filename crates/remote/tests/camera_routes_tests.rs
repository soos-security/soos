//! Contract tests of the live camera view routes and their CSRF check (GitHub #345, ADR
//! 2026-10-07 "Live Camera View in `soos-remote` Through the Daemon Preview Channel",
//! architect spec `AI/architect_spec_remote_live_camera.md` §8.1, tests 37–38, matrix RLC6,
//! RLC8).
//!
//! The spec maps tests 37–38 to `routes_tests.rs`; they live in this new file so that the
//! existing route suite keeps compiling while the camera API does not exist yet (names
//! unchanged, traceability greps the test names).

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

use soos_remote::camera_slot::ViewToken;
use soos_remote::http::{Method, RequestHead};
use soos_remote::routes::{
    accepts_body, allow_header, camera_stream_token, check_auth_csrf, check_camera_csrf,
    is_funnel_public, route, CsrfError, Route, CAMERA_OPTIONS_PATH, CAMERA_PATH, CAMERA_START_PATH,
    CAMERA_STOP_PATH, CAMERA_STREAM_PREFIX,
};
use soos_remote::{
    ACTION_CAMERA_OPTIONS, ACTION_CAMERA_STOP, ACTION_CAMERA_STREAM, ACTION_CAMERA_VIEW,
    CAMERA_VIEW_TOKEN_BYTES,
};

const HOST: &str = "pc.tail1234.ts.net";

fn token_text(n: u8) -> String {
    let mut bytes = [0u8; CAMERA_VIEW_TOKEN_BYTES];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = n.wrapping_add((i as u8).wrapping_mul(7));
    }
    ViewToken::from_bytes(bytes).encode().as_str().to_string()
}

fn stream_path(n: u8) -> String {
    format!("{CAMERA_STREAM_PREFIX}{}", token_text(n))
}

fn head(method: Method, path: &str, headers: &[(&str, &[u8])]) -> RequestHead {
    RequestHead {
        method,
        path: path.to_string(),
        headers: headers
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_vec()))
            .collect(),
    }
}

/// Test 37 (RLC8, RLC6): the §8.1 table: methods, `405` + `Allow`, near paths `404`, the
/// single body route, nothing public on the Funnel, and no token in `Route`'s `Debug`.
#[test]
fn test_rlc_camera_route_table() {
    assert_eq!(CAMERA_PATH, "/api/camera");
    assert_eq!(CAMERA_OPTIONS_PATH, "/api/auth/camera/options");
    assert_eq!(CAMERA_START_PATH, "/api/camera/start");
    assert_eq!(CAMERA_STOP_PATH, "/api/camera/stop");
    assert_eq!(CAMERA_STREAM_PREFIX, "/api/camera/stream/");

    // GET|HEAD /api/camera.
    assert_eq!(route(Method::Get, CAMERA_PATH), Route::Camera);
    assert_eq!(route(Method::Head, CAMERA_PATH), Route::Camera);
    assert_eq!(route(Method::Post, CAMERA_PATH), Route::MethodNotAllowed);
    assert_eq!(route(Method::Other, CAMERA_PATH), Route::MethodNotAllowed);
    assert_eq!(allow_header(CAMERA_PATH), "GET, HEAD");

    // The three POST routes.
    for (path, expected) in [
        (CAMERA_OPTIONS_PATH, Route::CameraOptions),
        (CAMERA_START_PATH, Route::CameraStart),
        (CAMERA_STOP_PATH, Route::CameraStop),
    ] {
        assert_eq!(route(Method::Post, path), expected, "{path}");
        for m in [Method::Get, Method::Head, Method::Other] {
            assert_eq!(route(m, path), Route::MethodNotAllowed, "{path}");
        }
        assert_eq!(allow_header(path), "POST", "{path}");
    }

    // The stream route: GET only, HEAD → 405 with `Allow: GET`.
    let path = stream_path(1);
    assert_eq!(path.len(), CAMERA_STREAM_PREFIX.len() + 43);
    assert_eq!(route(Method::Get, &path), Route::CameraStream);
    for m in [Method::Head, Method::Post, Method::Other] {
        assert_eq!(route(m, &path), Route::MethodNotAllowed, "{m:?}");
    }
    assert_eq!(allow_header(&path), "GET");

    // Near paths are 404 for every method.
    let good = token_text(2);
    let near = [
        "/api/camera/stream".to_string(),
        "/api/camera/stream/".to_string(),
        format!("/api/camera/stream/{}", &good[..42]),
        format!("/api/camera/stream/{good}A"),
        format!("/api/camera/stream/{good}/x"),
        format!("/api/camera/stream/{good}/"),
        format!("/api/camera/stream//{good}"),
        format!("/api/camera/stream/{}=", &good[..42]),
        format!("/api/camera/stream/{}+", &good[..42]),
        format!("/api/camera/stream/{}.", &good[..42]),
        format!("/api/camera/streams/{good}"),
        format!("/API/camera/stream/{good}"),
        "/api/camera/".to_string(),
        "/api/cameras".to_string(),
        "/api/camera/start/".to_string(),
        "/api/auth/camera".to_string(),
    ];
    for p in &near {
        for m in [Method::Get, Method::Head, Method::Post] {
            assert_eq!(route(m, p), Route::NotFound, "{m:?} {p}");
        }
        assert!(camera_stream_token(p).is_none(), "{p}");
    }

    // camera_stream_token returns the strict token of a well-formed stream path only.
    let t = camera_stream_token(&path).expect("well-formed stream path");
    assert_eq!(t.encode().as_str(), token_text(1));
    assert!(
        camera_stream_token(&token_text(1)).is_none(),
        "the prefix is required"
    );
    assert!(camera_stream_token(CAMERA_PATH).is_none());
    // A non-canonical token is not a stream path either (strict parse).
    let non_canonical = format!("{CAMERA_STREAM_PREFIX}{}B", "A".repeat(42));
    assert!(camera_stream_token(&non_canonical).is_none());

    // Body: only POST /api/camera/start among the camera routes.
    assert!(accepts_body(Method::Post, CAMERA_START_PATH));
    assert!(!accepts_body(Method::Get, CAMERA_START_PATH));
    for p in [
        CAMERA_PATH,
        CAMERA_OPTIONS_PATH,
        CAMERA_STOP_PATH,
        path.as_str(),
    ] {
        assert!(!accepts_body(Method::Post, p), "{p}");
        assert!(!accepts_body(Method::Get, p), "{p}");
    }
    // The existing body routes are unchanged.
    assert!(accepts_body(Method::Post, "/api/unlock"));

    // Nothing camera-related is reachable on the Funnel without a session.
    for r in [
        Route::Camera,
        Route::CameraOptions,
        Route::CameraStart,
        Route::CameraStop,
        Route::CameraStream,
    ] {
        assert!(!is_funnel_public(r), "{r:?}");
    }

    // The resolved route (which is Debug-logged) never carries the token.
    let resolved = route(Method::Get, &path);
    let debug = format!("{resolved:?}");
    let text = token_text(1);
    for start in 0..=(text.len() - 6) {
        assert!(!debug.contains(&text[start..start + 6]), "{debug}");
    }
    assert_eq!(format!("{:?}", Route::CameraStream), debug);
}

/// Test 38 (RLC6): `check_camera_csrf` applies the lock rules (exact `X-Soos-Action`,
/// `Sec-Fetch-Site` absent or same-origin, `Origin` absent or same host) for
/// `camera-stream` and `camera-stop`; any other action argument is refused; the options
/// and start routes use `check_auth_csrf` (Origin required).
#[test]
fn test_rlc_camera_csrf_rules() {
    for action in [ACTION_CAMERA_STREAM, ACTION_CAMERA_STOP] {
        let a = action.as_bytes();
        let check = |headers: &[(&str, &[u8])]| {
            check_camera_csrf(
                &head(Method::Get, "/api/camera/stop", headers),
                HOST,
                action,
            )
        };
        assert_eq!(check(&[("x-soos-action", a)]), Ok(()), "{action}");
        assert_eq!(check(&[]), Err(CsrfError::MissingActionHeader));
        for wrong in [
            &b""[..],
            b"lock",
            b"unlock",
            b"camera-view",
            b"camera-options",
            b"CAMERA-STREAM",
            b"camera-stream ",
            b"camera-stop,camera-stop",
        ] {
            if wrong == a {
                continue;
            }
            assert_eq!(
                check(&[("x-soos-action", wrong)]),
                Err(CsrfError::MissingActionHeader),
                "{action} {wrong:?}"
            );
        }
        // The other camera action never passes (no action confusion).
        let other = if action == ACTION_CAMERA_STREAM {
            ACTION_CAMERA_STOP
        } else {
            ACTION_CAMERA_STREAM
        };
        assert_eq!(
            check(&[("x-soos-action", other.as_bytes())]),
            Err(CsrfError::MissingActionHeader)
        );
        assert_eq!(
            check(&[("x-soos-action", a), ("x-soos-action", a)]),
            Err(CsrfError::MissingActionHeader),
            "repeated action header"
        );
        // Sec-Fetch-Site.
        assert_eq!(
            check(&[("x-soos-action", a), ("sec-fetch-site", b"same-origin")]),
            Ok(())
        );
        for site in [&b"cross-site"[..], b"same-site", b"none", b""] {
            assert_eq!(
                check(&[("x-soos-action", a), ("sec-fetch-site", site)]),
                Err(CsrfError::CrossSite),
                "{site:?}"
            );
        }
        // Origin: optional, same host only.
        assert_eq!(
            check(&[
                ("x-soos-action", a),
                ("origin", b"https://pc.tail1234.ts.net")
            ]),
            Ok(())
        );
        assert_eq!(
            check(&[
                ("x-soos-action", a),
                ("origin", b"https://PC.tail1234.ts.net:443")
            ]),
            Ok(())
        );
        for origin in [
            &b"https://evil.ts.net"[..],
            b"http://pc.tail1234.ts.net",
            b"null",
            b"",
        ] {
            assert_eq!(
                check(&[("x-soos-action", a), ("origin", origin)]),
                Err(CsrfError::OriginMismatch),
                "{origin:?}"
            );
        }
        assert_eq!(
            check(&[
                ("x-soos-action", a),
                ("origin", b"https://pc.tail1234.ts.net"),
                ("origin", b"https://pc.tail1234.ts.net"),
            ]),
            Err(CsrfError::OriginMismatch)
        );
    }

    // Any other action argument is refused, even with the matching header.
    for action in [
        "",
        "lock",
        "unlock",
        "alerts-ack",
        "push-test",
        ACTION_CAMERA_VIEW,
        ACTION_CAMERA_OPTIONS,
    ] {
        let h = head(
            Method::Post,
            "/api/camera/stop",
            &[("x-soos-action", action.as_bytes())],
        );
        assert_eq!(
            check_camera_csrf(&h, HOST, action),
            Err(CsrfError::MissingActionHeader),
            "{action:?}"
        );
    }

    // Options and start: the auth CSRF rules with Origin required.
    for action in [ACTION_CAMERA_OPTIONS, ACTION_CAMERA_VIEW] {
        let with_origin = head(
            Method::Post,
            CAMERA_START_PATH,
            &[
                ("x-soos-action", action.as_bytes()),
                ("origin", b"https://pc.tail1234.ts.net"),
            ],
        );
        assert_eq!(check_auth_csrf(&with_origin, HOST, action), Ok(()));
        let without = head(
            Method::Post,
            CAMERA_START_PATH,
            &[("x-soos-action", action.as_bytes())],
        );
        assert_eq!(
            check_auth_csrf(&without, HOST, action),
            Err(CsrfError::OriginMismatch)
        );
    }
}
