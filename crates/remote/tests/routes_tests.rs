//! Contract tests of GitHub #339 for the route table, the asset table and the lock CSRF
//! check (spec §2.5, D4, D11).

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

use soos_remote::assets::{asset, AssetId};
use soos_remote::http::{Method, RequestHead};
use soos_remote::routes::{
    allow_header, check_lock_csrf, check_unlock_csrf, route, CsrfError, Route,
};

const HOST: &str = "pc.tail1234.ts.net";

fn head(headers: &[(&str, &[u8])]) -> RequestHead {
    RequestHead {
        method: Method::Post,
        path: "/api/lock".to_string(),
        headers: headers
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_vec()))
            .collect(),
    }
}

fn csrf(headers: &[(&str, &[u8])]) -> Result<(), CsrfError> {
    check_lock_csrf(&head(headers), HOST)
}

// ---------------------------------------------------------------------------------------
// route
// ---------------------------------------------------------------------------------------

/// §2.5 route table: every documented pair, both GET and HEAD for the read routes.
#[test]
fn test_rmc_route_table() {
    for method in [Method::Get, Method::Head] {
        assert_eq!(route(method, "/"), Route::Asset(AssetId::Index));
        assert_eq!(route(method, "/index.html"), Route::Asset(AssetId::Index));
        assert_eq!(route(method, "/app.js"), Route::Asset(AssetId::AppJs));
        assert_eq!(route(method, "/style.css"), Route::Asset(AssetId::StyleCss));
        assert_eq!(
            route(method, "/manifest.webmanifest"),
            Route::Asset(AssetId::Manifest)
        );
        assert_eq!(route(method, "/icon.svg"), Route::Asset(AssetId::IconSvg));
        assert_eq!(
            route(method, "/apple-touch-icon.png"),
            Route::Asset(AssetId::AppleTouchIcon)
        );
        assert_eq!(route(method, "/api/status"), Route::Status);
        assert_eq!(route(method, "/api/events"), Route::Events);
        assert_eq!(route(method, "/api/lock"), Route::MethodNotAllowed);
    }
    assert_eq!(route(Method::Post, "/api/lock"), Route::Lock);
    assert_eq!(route(Method::Get, "/api/status?x=1"), Route::Status);
    assert_eq!(route(Method::Get, "/?v=2"), Route::Asset(AssetId::Index));
}

/// §2.5: a known path with a wrong method is `405`; an unknown path is `404` whatever the
/// method; paths are matched exactly (case, trailing slash, dot segments, encoding).
#[test]
fn test_rmc_route_wrong_method_and_unknown_paths() {
    for path in [
        "/",
        "/index.html",
        "/app.js",
        "/style.css",
        "/manifest.webmanifest",
        "/icon.svg",
        "/apple-touch-icon.png",
        "/api/status",
        "/api/events",
    ] {
        assert_eq!(route(Method::Post, path), Route::MethodNotAllowed, "{path}");
        assert_eq!(
            route(Method::Other, path),
            Route::MethodNotAllowed,
            "{path}"
        );
    }
    assert_eq!(route(Method::Other, "/api/lock"), Route::MethodNotAllowed);
    for path in [
        "/nope",
        "/api",
        "/api/",
        "/api/status/",
        "/API/STATUS",
        "/Index.html",
        "/./index.html",
        "/../index.html",
        "/%2e%2e/index.html",
        "/index.html/",
        "/app.js.map",
        "/api/lock/",
        "/api/unlock/",
        "/API/UNLOCK",
        "/api/events2",
        "//",
        "",
        "index.html",
    ] {
        for method in [Method::Get, Method::Head, Method::Post, Method::Other] {
            assert_eq!(route(method, path), Route::NotFound, "{method:?} {path:?}");
        }
    }
}

// ---------------------------------------------------------------------------------------
// assets
// ---------------------------------------------------------------------------------------

/// D11 / §2.5: every embedded asset has its documented content type and a non-empty body;
/// the icon is a PNG.
#[test]
fn test_rmc_assets_table_content_types_and_bodies() {
    for (id, content_type) in [
        (AssetId::Index, "text/html; charset=utf-8"),
        (AssetId::AppJs, "text/javascript; charset=utf-8"),
        (AssetId::StyleCss, "text/css; charset=utf-8"),
        (AssetId::Manifest, "application/manifest+json"),
        (AssetId::IconSvg, "image/svg+xml"),
        (AssetId::AppleTouchIcon, "image/png"),
    ] {
        let a = asset(id);
        assert_eq!(a.content_type, content_type, "{id:?}");
        assert!(!a.body.is_empty(), "{id:?} must be embedded");
    }
    let png = asset(AssetId::AppleTouchIcon).body;
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "PNG signature");
    assert!(
        asset(AssetId::Index).body.starts_with(b"<!doctype html")
            || asset(AssetId::Index).body.starts_with(b"<!DOCTYPE html"),
        "index.html starts with a doctype"
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(asset(AssetId::Manifest).body).unwrap();
    assert_eq!(manifest["display"], "standalone");
    assert_eq!(manifest["start_url"], "/");
    assert_eq!(manifest["scope"], "/");
}

// ---------------------------------------------------------------------------------------
// check_lock_csrf
// ---------------------------------------------------------------------------------------

/// D4: `X-Soos-Action: lock` is mandatory; its absence or any other value is a refusal.
#[test]
fn test_rmc_check_lock_csrf_requires_the_action_header() {
    assert_eq!(csrf(&[("x-soos-action", b"lock")]), Ok(()));
    assert_eq!(csrf(&[]), Err(CsrfError::MissingActionHeader));
    assert_eq!(
        csrf(&[
            ("host", HOST.as_bytes()),
            ("tailscale-user-login", b"owner@example.com")
        ]),
        Err(CsrfError::MissingActionHeader)
    );
    assert_eq!(
        csrf(&[("x-soos-action", b"unlock")]),
        Err(CsrfError::MissingActionHeader)
    );
    assert_eq!(
        csrf(&[("x-soos-action", b"")]),
        Err(CsrfError::MissingActionHeader)
    );
    assert_eq!(
        csrf(&[("x-soos-action", b"lock,lock")]),
        Err(CsrfError::MissingActionHeader)
    );
    assert_eq!(
        csrf(&[("x-soos-action-2", b"lock")]),
        Err(CsrfError::MissingActionHeader)
    );
}

/// D4: `Sec-Fetch-Site` must be absent or `same-origin`.
#[test]
fn test_rmc_check_lock_csrf_sec_fetch_site() {
    assert_eq!(
        csrf(&[
            ("x-soos-action", b"lock"),
            ("sec-fetch-site", b"same-origin")
        ]),
        Ok(())
    );
    for value in [
        &b"cross-site"[..],
        b"same-site",
        b"none",
        b"",
        b"SAME-ORIGIN ",
        b"\xff",
    ] {
        assert_eq!(
            csrf(&[("x-soos-action", b"lock"), ("sec-fetch-site", value)]),
            Err(CsrfError::CrossSite),
            "{value:?}"
        );
    }
}

/// D4 / R2-6: `Origin` must be absent or `https://<normalized host>` (compared after the
/// same normalisation: lowercase, `:443` stripped).
#[test]
fn test_rmc_check_lock_csrf_origin() {
    assert_eq!(
        csrf(&[
            ("x-soos-action", b"lock"),
            ("origin", b"https://pc.tail1234.ts.net")
        ]),
        Ok(())
    );
    assert_eq!(
        csrf(&[
            ("x-soos-action", b"lock"),
            ("origin", b"https://PC.Tail1234.TS.NET:443")
        ]),
        Ok(())
    );
    for value in [
        &b"https://other.tail1234.ts.net"[..],
        b"http://pc.tail1234.ts.net",
        b"https://pc.tail1234.ts.net:8443",
        b"https://pc.tail1234.ts.net/",
        b"https://pc.tail1234.ts.net.evil.com",
        b"https://evil.com#pc.tail1234.ts.net",
        b"null",
        b"",
        b"pc.tail1234.ts.net",
        b"\xff",
    ] {
        assert_eq!(
            csrf(&[("x-soos-action", b"lock"), ("origin", value)]),
            Err(CsrfError::OriginMismatch),
            "{value:?}"
        );
    }
    assert_eq!(
        csrf(&[
            ("x-soos-action", b"lock"),
            ("origin", b"https://pc.tail1234.ts.net"),
            ("origin", b"https://pc.tail1234.ts.net"),
        ]),
        Err(CsrfError::OriginMismatch),
        "a repeated Origin never passes"
    );
    // The comparison uses the normalized host handed in, not the raw header.
    assert_eq!(
        check_lock_csrf(
            &head(&[
                ("x-soos-action", b"lock"),
                ("origin", b"https://lan-box.example")
            ]),
            "lan-box.example"
        ),
        Ok(())
    );
}

/// §4: refusal messages are fixed text.
#[test]
fn test_rmc_csrf_error_messages() {
    assert_eq!(
        CsrfError::MissingActionHeader.to_string(),
        "action header missing"
    );
    assert_eq!(CsrfError::CrossSite.to_string(), "cross-site request");
    assert_eq!(CsrfError::OriginMismatch.to_string(), "origin mismatch");
}

// ---------------------------------------------------------------------------------------
// /api/unlock (ADR 2026-10-06 "Remote Unlock in soos-remote", matrix RMC22, RMC24)
// ---------------------------------------------------------------------------------------

fn unlock_csrf(headers: &[(&str, &[u8])]) -> Result<(), CsrfError> {
    let mut request = head(headers);
    request.path = "/api/unlock".to_string();
    check_unlock_csrf(&request, HOST)
}

/// RMC22: `POST /api/unlock` is the unlock route; any other method is `405 Allow: POST`.
#[test]
fn test_rmc_unlock_route() {
    assert_eq!(route(Method::Post, "/api/unlock"), Route::Unlock);
    assert_eq!(route(Method::Post, "/api/unlock?x=1"), Route::Unlock);
    for method in [Method::Get, Method::Head, Method::Other] {
        assert_eq!(
            route(method, "/api/unlock"),
            Route::MethodNotAllowed,
            "{method:?}"
        );
    }
    assert_eq!(allow_header("/api/unlock"), "POST");
    assert_eq!(allow_header("/api/lock"), "POST");
    assert_eq!(allow_header("/api/status"), "GET, HEAD");
}

/// RMC24: the unlock CSRF check requires exactly `X-Soos-Action: unlock` and applies the
/// same `Sec-Fetch-Site` and `Origin` rules as the lock.
#[test]
fn test_rmc_check_unlock_csrf() {
    assert_eq!(unlock_csrf(&[("x-soos-action", b"unlock")]), Ok(()));
    for value in [&b"lock"[..], b"", b"UNLOCK", b"unlock ", b"unlock,unlock"] {
        assert_eq!(
            unlock_csrf(&[("x-soos-action", value)]),
            Err(CsrfError::MissingActionHeader),
            "{value:?}"
        );
    }
    assert_eq!(unlock_csrf(&[]), Err(CsrfError::MissingActionHeader));
    assert_eq!(
        unlock_csrf(&[("x-soos-action", b"unlock"), ("x-soos-action", b"unlock")]),
        Err(CsrfError::MissingActionHeader)
    );
    assert_eq!(
        unlock_csrf(&[
            ("x-soos-action", b"unlock"),
            ("sec-fetch-site", b"same-origin")
        ]),
        Ok(())
    );
    assert_eq!(
        unlock_csrf(&[
            ("x-soos-action", b"unlock"),
            ("sec-fetch-site", b"cross-site")
        ]),
        Err(CsrfError::CrossSite)
    );
    assert_eq!(
        unlock_csrf(&[
            ("x-soos-action", b"unlock"),
            ("origin", b"https://PC.tail1234.ts.net:443")
        ]),
        Ok(())
    );
    assert_eq!(
        unlock_csrf(&[
            ("x-soos-action", b"unlock"),
            ("origin", b"https://evil.ts.net")
        ]),
        Err(CsrfError::OriginMismatch)
    );
    // The lock check never accepts the unlock action (no action confusion).
    assert_eq!(
        csrf(&[("x-soos-action", b"unlock")]),
        Err(CsrfError::MissingActionHeader)
    );
}
