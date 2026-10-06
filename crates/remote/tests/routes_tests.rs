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

// ---------------------------------------------------------------------------------------
// Passkey routes (ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey
// Authentication for `soos-remote`", spec §4.7, tests 14–15, matrix RMC27 / RMC29)
// ---------------------------------------------------------------------------------------

mod passkey_routes {
    use super::*;
    use soos_remote::routes::{accepts_body, check_auth_csrf, is_funnel_public};
    use soos_remote::{
        ACTION_LOGIN, ACTION_LOGIN_OPTIONS, ACTION_LOGOUT, ACTION_REGISTER,
        ACTION_REGISTER_OPTIONS, ACTION_UNLOCK_OPTIONS,
    };

    const POST_ROUTES: [(&str, Route); 6] = [
        ("/api/auth/login/options", Route::LoginOptions),
        ("/api/auth/login/verify", Route::LoginVerify),
        ("/api/auth/logout", Route::Logout),
        ("/api/auth/unlock/options", Route::UnlockOptions),
        ("/api/auth/register/options", Route::RegisterOptions),
        ("/api/auth/register/verify", Route::RegisterVerify),
    ];

    /// Test 14 (RMC27/RMC29, plan-evaluator G-2b): every new route and method, `405` +
    /// `Allow`, the four body routes, and the routes reachable on Funnel without a session
    /// (logout included, so an expired session can still clear its cookie).
    #[test]
    fn test_rmc_auth_route_table() {
        for method in [Method::Get, Method::Head] {
            assert_eq!(route(method, "/api/auth/state"), Route::AuthState);
            assert_eq!(route(method, "/api/auth/state?x=1"), Route::AuthState);
        }
        assert_eq!(
            route(Method::Post, "/api/auth/state"),
            Route::MethodNotAllowed
        );
        assert_eq!(
            route(Method::Other, "/api/auth/state"),
            Route::MethodNotAllowed
        );
        assert_eq!(allow_header("/api/auth/state"), "GET, HEAD");
        for (path, expected) in POST_ROUTES {
            assert_eq!(route(Method::Post, path), expected, "{path}");
            assert_eq!(
                route(Method::Post, &format!("{path}?q=1")),
                expected,
                "{path} with a query"
            );
            for method in [Method::Get, Method::Head, Method::Other] {
                assert_eq!(route(method, path), Route::MethodNotAllowed, "{path}");
            }
            assert_eq!(allow_header(path), "POST", "{path}");
        }
        for unknown in [
            "/api/auth",
            "/api/auth/",
            "/api/auth/login",
            "/api/auth/login/",
            "/api/auth/Login/options",
            "/api/auth/register",
            "/api/auth/unlock",
            "/api/auth/state/",
            "/api/auth/logout/",
        ] {
            assert_eq!(route(Method::Post, unknown), Route::NotFound, "{unknown}");
            assert_eq!(route(Method::Get, unknown), Route::NotFound, "{unknown}");
        }

        // accepts_body: exactly the four body routes, POST only, query ignored.
        for path in [
            "/api/unlock",
            "/api/auth/login/verify",
            "/api/auth/register/options",
            "/api/auth/register/verify",
        ] {
            assert!(accepts_body(Method::Post, path), "{path}");
            assert!(accepts_body(Method::Post, &format!("{path}?x=1")), "{path}");
            for method in [Method::Get, Method::Head, Method::Other] {
                assert!(!accepts_body(method, path), "{path}");
            }
        }
        for path in [
            "/",
            "/api/status",
            "/api/events",
            "/api/lock",
            "/api/auth/state",
            "/api/auth/login/options",
            "/api/auth/logout",
            "/api/auth/unlock/options",
            "/api/unlock/",
            "/api/auth/login/verify/",
            "/nope",
        ] {
            assert!(!accepts_body(Method::Post, path), "{path}");
        }

        // is_funnel_public.
        for public in [
            Route::Asset(AssetId::Index),
            Route::Asset(AssetId::AppJs),
            Route::Asset(AssetId::StyleCss),
            Route::Asset(AssetId::Manifest),
            Route::Asset(AssetId::IconSvg),
            Route::Asset(AssetId::AppleTouchIcon),
            Route::AuthState,
            Route::LoginOptions,
            Route::LoginVerify,
            Route::Logout,
            Route::NotFound,
            Route::MethodNotAllowed,
        ] {
            assert!(is_funnel_public(public), "{public:?}");
        }
        for private in [
            Route::Status,
            Route::Events,
            Route::Lock,
            Route::Unlock,
            Route::UnlockOptions,
            Route::RegisterOptions,
            Route::RegisterVerify,
        ] {
            assert!(!is_funnel_public(private), "{private:?}");
        }
    }

    fn auth_head(headers: &[(&str, &[u8])]) -> RequestHead {
        RequestHead {
            method: Method::Post,
            path: "/api/auth/login/verify".to_string(),
            headers: headers
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_vec()))
                .collect(),
        }
    }

    fn auth_csrf(headers: &[(&str, &[u8])], action: &str) -> Result<(), CsrfError> {
        check_auth_csrf(&auth_head(headers), HOST, action)
    }

    /// Test 15 (RMC33): the lock CSRF rules with the route's action and a **required**
    /// `Origin` equal to `https://<rp_id>` (`:443` accepted, ASCII-lowercased); sibling
    /// tailnet nodes (same-site, cross-origin) are refused.
    #[test]
    fn test_rmc_check_auth_csrf_requires_origin() {
        let origin: &[u8] = b"https://pc.tail1234.ts.net";
        for action in [
            ACTION_LOGIN_OPTIONS,
            ACTION_LOGIN,
            ACTION_LOGOUT,
            ACTION_UNLOCK_OPTIONS,
            ACTION_REGISTER_OPTIONS,
            ACTION_REGISTER,
        ] {
            let act = action.as_bytes();
            assert_eq!(
                auth_csrf(&[("x-soos-action", act), ("origin", origin)], action),
                Ok(()),
                "{action}"
            );
            assert_eq!(
                auth_csrf(
                    &[
                        ("x-soos-action", act),
                        ("sec-fetch-site", b"same-origin"),
                        ("origin", b"HTTPS://PC.Tail1234.TS.NET:443"),
                    ],
                    action
                ),
                Ok(()),
                "{action}"
            );
            assert_eq!(
                auth_csrf(&[("x-soos-action", act)], action),
                Err(CsrfError::OriginMismatch),
                "Origin is required on {action}"
            );
            assert_eq!(
                auth_csrf(&[("origin", origin)], action),
                Err(CsrfError::MissingActionHeader)
            );
        }
        // The action must be exactly the route's value.
        for wrong in [&b"login-options"[..], b"LOGIN", b"login ", b"unlock", b""] {
            assert_eq!(
                auth_csrf(
                    &[("x-soos-action", wrong), ("origin", origin)],
                    ACTION_LOGIN
                ),
                Err(CsrfError::MissingActionHeader),
                "{wrong:?}"
            );
        }
        assert_eq!(
            auth_csrf(
                &[
                    ("x-soos-action", b"login"),
                    ("x-soos-action", b"login"),
                    ("origin", origin)
                ],
                ACTION_LOGIN
            ),
            Err(CsrfError::MissingActionHeader)
        );
        for site in [&b"cross-site"[..], b"same-site", b"none", b""] {
            assert_eq!(
                auth_csrf(
                    &[
                        ("x-soos-action", b"login"),
                        ("sec-fetch-site", site),
                        ("origin", origin)
                    ],
                    ACTION_LOGIN
                ),
                Err(CsrfError::CrossSite),
                "{site:?}"
            );
        }
        for bad in [
            &b"https://other.tail1234.ts.net"[..],
            b"https://tail1234.ts.net",
            b"http://pc.tail1234.ts.net",
            b"https://pc.tail1234.ts.net:8443",
            b"https://pc.tail1234.ts.net/",
            b"null",
            b"",
        ] {
            assert_eq!(
                auth_csrf(
                    &[("x-soos-action", b"login"), ("origin", bad)],
                    ACTION_LOGIN
                ),
                Err(CsrfError::OriginMismatch),
                "{bad:?}"
            );
        }
        assert_eq!(
            auth_csrf(
                &[
                    ("x-soos-action", b"login"),
                    ("origin", origin),
                    ("origin", origin)
                ],
                ACTION_LOGIN
            ),
            Err(CsrfError::OriginMismatch),
            "a repeated Origin"
        );
        // The comparison uses rp_id, never a request header.
        assert_eq!(
            check_auth_csrf(
                &auth_head(&[
                    ("x-soos-action", b"login"),
                    ("origin", b"https://evil.example"),
                    ("host", b"evil.example"),
                    ("x-forwarded-host", b"evil.example"),
                ]),
                HOST,
                ACTION_LOGIN
            ),
            Err(CsrfError::OriginMismatch)
        );
    }
}

// ---------------------------------------------------------------------------------------
// Failed-password alerts (ADR 2026-10-06 "Failed-Password Alerts in `soos-remote` From the
// System Journal", architect spec `AI/architect_spec_remote_auth_alerts.md` §6, §6.1, tests 39
// and 40; matrix RMC53, RMC54). New tests only; nothing above is changed.
// ---------------------------------------------------------------------------------------

mod alerts_contract {
    use soos_remote::alerts::AlertsEpoch;
    use soos_remote::http::{parse_request_head, Method, RequestHead};
    use soos_remote::routes::{
        accepts_body, allow_header, check_alerts_ack_csrf, is_funnel_public,
        parse_alerts_ack_headers, route, AckHeaderError, AckTarget, CsrfError, Route,
        ALERTS_ACK_PATH, ALERTS_PATH,
    };

    const HOST: &str = "pc.tail1234.ts.net";

    fn head(headers: &[(&str, &[u8])]) -> RequestHead {
        RequestHead {
            method: Method::Post,
            path: ALERTS_ACK_PATH.to_string(),
            headers: headers
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_vec()))
                .collect(),
        }
    }

    /// Test 39 (RMC53, §6): the two routes, their methods and `Allow`; the parser keeps the
    /// query out of the path; neither route is Funnel-public nor accepts a body.
    #[test]
    fn test_rmc_alerts_routes_table() {
        assert_eq!(ALERTS_PATH, "/api/alerts");
        assert_eq!(ALERTS_ACK_PATH, "/api/alerts/ack");
        for method in [Method::Get, Method::Head] {
            assert_eq!(route(method, "/api/alerts"), Route::Alerts);
            assert_eq!(route(method, "/api/alerts/ack"), Route::MethodNotAllowed);
        }
        assert_eq!(route(Method::Post, "/api/alerts/ack"), Route::AlertsAck);
        assert_eq!(route(Method::Post, "/api/alerts"), Route::MethodNotAllowed);
        assert_eq!(route(Method::Other, "/api/alerts"), Route::MethodNotAllowed);
        assert_eq!(
            route(Method::Other, "/api/alerts/ack"),
            Route::MethodNotAllowed
        );
        assert_eq!(allow_header("/api/alerts/ack"), "POST");
        assert_eq!(allow_header("/api/alerts"), "GET, HEAD");
        for near in [
            "/api/alerts/",
            "/api/Alerts",
            "/api/alerts/ack/",
            "/api/alert",
        ] {
            assert_eq!(route(Method::Get, near), Route::NotFound, "{near}");
            assert_eq!(route(Method::Post, near), Route::NotFound, "{near}");
        }
        let parsed = parse_request_head(
            b"POST /api/alerts/ack?through=5 HTTP/1.1\r\nHost: x.ts.net\r\n\r\n",
        )
        .unwrap();
        assert_eq!(parsed.path, "/api/alerts/ack");
        assert!(!is_funnel_public(Route::Alerts));
        assert!(!is_funnel_public(Route::AlertsAck));
        assert!(!accepts_body(Method::Post, "/api/alerts/ack"));
        assert!(!accepts_body(Method::Post, "/api/alerts"));
    }

    /// Test 40 (RMC54, §6, §6.1, F-1): the CSRF table of the lock route with `alerts-ack`,
    /// and the strict snapshot headers (exactly one each, lowercased names only).
    #[test]
    fn test_rmc_alerts_ack_csrf_rules() {
        let csrf = |headers: &[(&str, &[u8])]| check_alerts_ack_csrf(&head(headers), HOST);
        assert_eq!(csrf(&[("x-soos-action", b"alerts-ack")]), Ok(()));
        assert_eq!(csrf(&[]), Err(CsrfError::MissingActionHeader));
        for action in [&b"lock"[..], b"unlock", b"Alerts-Ack", b"alerts-ack ", b""] {
            assert_eq!(
                csrf(&[("x-soos-action", action)]),
                Err(CsrfError::MissingActionHeader),
                "{:?}",
                String::from_utf8_lossy(action)
            );
        }
        assert_eq!(
            csrf(&[
                ("x-soos-action", b"alerts-ack"),
                ("x-soos-action", b"alerts-ack")
            ]),
            Err(CsrfError::MissingActionHeader)
        );
        assert_eq!(
            csrf(&[
                ("x-soos-action", b"alerts-ack"),
                ("sec-fetch-site", b"same-origin")
            ]),
            Ok(())
        );
        for site in [&b"cross-site"[..], b"same-site", b"none"] {
            assert_eq!(
                csrf(&[("x-soos-action", b"alerts-ack"), ("sec-fetch-site", site)]),
                Err(CsrfError::CrossSite)
            );
        }
        for origin in [
            &b"https://pc.tail1234.ts.net"[..],
            b"https://pc.tail1234.ts.net:443",
            b"HTTPS://PC.TAIL1234.TS.NET",
        ] {
            assert_eq!(
                csrf(&[("x-soos-action", b"alerts-ack"), ("origin", origin)]),
                Ok(())
            );
        }
        for origin in [
            &b"https://evil.example.com"[..],
            b"http://pc.tail1234.ts.net",
            b"https://pc.tail1234.ts.net:8443",
            b"null",
        ] {
            assert_eq!(
                csrf(&[("x-soos-action", b"alerts-ack"), ("origin", origin)]),
                Err(CsrfError::OriginMismatch)
            );
        }

        // Snapshot headers.
        let parse = |epoch: Option<&[u8]>, through: Option<&[u8]>| {
            let mut headers: Vec<(&str, &[u8])> = Vec::new();
            if let Some(e) = epoch {
                headers.push(("x-soos-alerts-epoch", e));
            }
            if let Some(t) = through {
                headers.push(("x-soos-alerts-through", t));
            }
            parse_alerts_ack_headers(&head(&headers))
        };
        assert_eq!(
            parse(Some(b"0000000000000000"), Some(b"0")),
            Ok(AckTarget {
                epoch: AlertsEpoch::from_u64(0),
                through: 0
            })
        );
        assert_eq!(
            parse(Some(b"ffffffffffffffff"), Some(b"18446744073709551615")),
            Ok(AckTarget {
                epoch: AlertsEpoch::from_u64(u64::MAX),
                through: u64::MAX
            })
        );
        assert_eq!(
            parse(Some(b"0123456789abcdef"), Some(b"00000000000000000007")),
            Ok(AckTarget {
                epoch: AlertsEpoch::from_u64(0x0123_4567_89ab_cdef),
                through: 7
            })
        );
        let good_epoch: &[u8] = b"0123456789abcdef";
        let bad_epochs: [Option<&[u8]>; 8] = [
            None,
            Some(&b""[..]),
            Some(b"0123456789abcde"),
            Some(b"0123456789abcdef0"),
            Some(b"0123456789ABCDEF"),
            Some(b"0123456789abcdeg"),
            Some(b" 123456789abcdef"),
            Some(b"0123456789abcd\xc3\xa9"),
        ];
        for bad in bad_epochs {
            assert_eq!(
                parse(bad, Some(b"1")),
                Err(AckHeaderError::Epoch),
                "{:?}",
                bad.map(String::from_utf8_lossy)
            );
        }
        let bad_throughs: [Option<&[u8]>; 14] = [
            None,
            Some(&b""[..]),
            Some(b"a"),
            Some(b"-1"),
            Some(b"+1"),
            Some(b" 5"),
            Some(b"5 "),
            Some(b"1.0"),
            Some(b"0x1"),
            Some(b"1e3"),
            Some(b"123456789012345678901"),
            Some(b"18446744073709551616"),
            Some(b"99999999999999999999"),
            Some("\u{ff15}".as_bytes()),
        ];
        for bad in bad_throughs {
            assert_eq!(
                parse(Some(good_epoch), bad),
                Err(AckHeaderError::Through),
                "{:?}",
                bad.map(String::from_utf8_lossy)
            );
        }
        // Repeated headers.
        assert_eq!(
            parse_alerts_ack_headers(&head(&[
                ("x-soos-alerts-epoch", good_epoch),
                ("x-soos-alerts-epoch", good_epoch),
                ("x-soos-alerts-through", b"1"),
            ])),
            Err(AckHeaderError::Epoch)
        );
        assert_eq!(
            parse_alerts_ack_headers(&head(&[
                ("x-soos-alerts-epoch", good_epoch),
                ("x-soos-alerts-through", b"1"),
                ("x-soos-alerts-through", b"1"),
            ])),
            Err(AckHeaderError::Through)
        );
        // Names are matched only in their lowercased parsed form.
        assert_eq!(
            parse_alerts_ack_headers(&head(&[
                ("X-Soos-Alerts-Epoch", good_epoch),
                ("x-soos-alerts-through", b"1"),
            ])),
            Err(AckHeaderError::Epoch)
        );
        // The path (and any query in it) is never read.
        let mut with_query = head(&[
            ("x-soos-alerts-epoch", good_epoch),
            ("x-soos-alerts-through", b"3"),
        ]);
        with_query.path = "/api/alerts/ack?through=9".to_string();
        assert_eq!(
            parse_alerts_ack_headers(&with_query).map(|t| t.through),
            Ok(3)
        );
        let mut no_through = head(&[("x-soos-alerts-epoch", good_epoch)]);
        no_through.path = "/api/alerts/ack?through=9".to_string();
        assert_eq!(
            parse_alerts_ack_headers(&no_through),
            Err(AckHeaderError::Through)
        );
        // Errors never echo a value.
        assert!(!AckHeaderError::Epoch.to_string().contains("0123"));
    }
}

// ---------------------------------------------------------------------------------------
// Web Push routes (ADR 2026-10-06 "Web Push Notifications for Failed-Password Alerts Through
// a Separate Sender Unit", architect spec AI/architect_spec_remote_web_push.md §6, tests
// 39–41; matrix RMC66, RMC73)
// ---------------------------------------------------------------------------------------

mod push_contract {
    use soos_remote::assets::{asset, AssetId};
    use soos_remote::http::{Method, RequestHead};
    use soos_remote::routes::{
        accepts_body, allow_header, check_push_csrf, is_funnel_public, route, CsrfError, Route,
        PUSH_PATH, PUSH_SUBSCRIBE_PATH, PUSH_TEST_PATH, PUSH_UNSUBSCRIBE_PATH, SERVICE_WORKER_PATH,
    };
    use soos_remote::{ACTION_PUSH_SUBSCRIBE, ACTION_PUSH_TEST, ACTION_PUSH_UNSUBSCRIBE};

    const HOST: &str = "pc.tail1234.ts.net";

    fn head(path: &str, headers: &[(&str, &[u8])]) -> RequestHead {
        RequestHead {
            method: Method::Post,
            path: path.to_string(),
            headers: headers
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_vec()))
                .collect(),
        }
    }

    /// Test 39 (RMC66, RMC73, §6.1): the route table, methods and `Allow`; near paths are
    /// `404`; exactly subscribe and unsubscribe join the body routes; the four push API
    /// routes are not Funnel-public, the service worker is.
    #[test]
    fn test_rwp_push_route_table() {
        assert_eq!(PUSH_PATH, "/api/push");
        assert_eq!(PUSH_SUBSCRIBE_PATH, "/api/push/subscribe");
        assert_eq!(PUSH_UNSUBSCRIBE_PATH, "/api/push/unsubscribe");
        assert_eq!(PUSH_TEST_PATH, "/api/push/test");
        assert_eq!(SERVICE_WORKER_PATH, "/sw.js");
        for method in [Method::Get, Method::Head] {
            assert_eq!(route(method, "/api/push"), Route::Push);
            assert_eq!(
                route(method, "/sw.js"),
                Route::Asset(AssetId::ServiceWorker)
            );
            for post_only in [
                "/api/push/subscribe",
                "/api/push/unsubscribe",
                "/api/push/test",
            ] {
                assert_eq!(
                    route(method, post_only),
                    Route::MethodNotAllowed,
                    "{post_only}"
                );
            }
        }
        assert_eq!(
            route(Method::Post, "/api/push/subscribe"),
            Route::PushSubscribe
        );
        assert_eq!(
            route(Method::Post, "/api/push/unsubscribe"),
            Route::PushUnsubscribe
        );
        assert_eq!(route(Method::Post, "/api/push/test"), Route::PushTest);
        assert_eq!(route(Method::Post, "/api/push"), Route::MethodNotAllowed);
        assert_eq!(route(Method::Post, "/sw.js"), Route::MethodNotAllowed);
        assert_eq!(route(Method::Other, "/api/push"), Route::MethodNotAllowed);
        assert_eq!(route(Method::Other, "/sw.js"), Route::MethodNotAllowed);
        assert_eq!(
            route(Method::Other, "/api/push/test"),
            Route::MethodNotAllowed
        );
        assert_eq!(allow_header("/api/push"), "GET, HEAD");
        assert_eq!(allow_header("/sw.js"), "GET, HEAD");
        for post_only in [
            "/api/push/subscribe",
            "/api/push/unsubscribe",
            "/api/push/test",
        ] {
            assert_eq!(allow_header(post_only), "POST", "{post_only}");
        }
        for near in [
            "/api/push/",
            "/api/Push",
            "/api/push/subscribe/",
            "/api/push/Subscribe",
            "/api/push/unsubscribe/",
            "/api/push/test/",
            "/api/pushes",
            "/sw.js/",
            "/SW.js",
            "/sw.JS",
            "/sw",
            "/service-worker.js",
        ] {
            assert_eq!(route(Method::Get, near), Route::NotFound, "{near}");
            assert_eq!(route(Method::Post, near), Route::NotFound, "{near}");
        }

        // Body routes: exactly the four existing ones plus subscribe and unsubscribe.
        for path in [
            "/api/push/subscribe",
            "/api/push/subscribe?x=1",
            "/api/push/unsubscribe",
            "/api/push/unsubscribe?x=1",
            "/api/unlock",
            "/api/auth/login/verify",
            "/api/auth/register/options",
            "/api/auth/register/verify",
        ] {
            assert!(accepts_body(Method::Post, path), "{path}");
        }
        for path in [
            "/api/push",
            "/api/push/test",
            "/sw.js",
            "/api/lock",
            "/api/alerts/ack",
        ] {
            assert!(!accepts_body(Method::Post, path), "{path}");
        }
        assert!(!accepts_body(Method::Get, "/api/push/subscribe"));
        assert!(!accepts_body(Method::Other, "/api/push/unsubscribe"));

        for r in [
            Route::Push,
            Route::PushSubscribe,
            Route::PushUnsubscribe,
            Route::PushTest,
        ] {
            assert!(!is_funnel_public(r), "{r:?}");
        }
        assert!(is_funnel_public(Route::Asset(AssetId::ServiceWorker)));
        // Existing answers unchanged.
        assert!(is_funnel_public(Route::AuthState));
        assert!(!is_funnel_public(Route::Alerts));
        assert!(!is_funnel_public(Route::Lock));
    }

    /// Test 40 (RMC66, §6.1): `check_push_csrf` applies the lock table to each push action;
    /// an action value of another route, or an unknown action argument, is refused.
    #[test]
    fn test_rwp_push_csrf_rules() {
        assert_eq!(ACTION_PUSH_SUBSCRIBE, "push-subscribe");
        assert_eq!(ACTION_PUSH_UNSUBSCRIBE, "push-unsubscribe");
        assert_eq!(ACTION_PUSH_TEST, "push-test");
        for (path, action) in [
            (PUSH_SUBSCRIBE_PATH, "push-subscribe"),
            (PUSH_UNSUBSCRIBE_PATH, "push-unsubscribe"),
            (PUSH_TEST_PATH, "push-test"),
        ] {
            let csrf =
                |headers: &[(&str, &[u8])]| check_push_csrf(&head(path, headers), HOST, action);
            let a = action.as_bytes();
            assert_eq!(csrf(&[("x-soos-action", a)]), Ok(()), "{action}");
            assert_eq!(csrf(&[]), Err(CsrfError::MissingActionHeader));
            let upper = action.to_ascii_uppercase();
            let spaced = format!("{action} ");
            for other in [
                &b"lock"[..],
                b"unlock",
                b"alerts-ack",
                b"",
                upper.as_bytes(),
                spaced.as_bytes(),
            ] {
                assert_eq!(
                    csrf(&[("x-soos-action", other)]),
                    Err(CsrfError::MissingActionHeader),
                    "{action} vs {:?}",
                    String::from_utf8_lossy(other)
                );
            }
            for sibling in ["push-subscribe", "push-unsubscribe", "push-test"] {
                if sibling != action {
                    assert_eq!(
                        csrf(&[("x-soos-action", sibling.as_bytes())]),
                        Err(CsrfError::MissingActionHeader),
                        "{action} vs {sibling}"
                    );
                }
            }
            assert_eq!(
                csrf(&[("x-soos-action", a), ("x-soos-action", a)]),
                Err(CsrfError::MissingActionHeader),
                "repeated"
            );
            assert_eq!(
                csrf(&[("x-soos-action", a), ("sec-fetch-site", b"same-origin")]),
                Ok(())
            );
            for site in [&b"cross-site"[..], b"same-site", b"none", b""] {
                assert_eq!(
                    csrf(&[("x-soos-action", a), ("sec-fetch-site", site)]),
                    Err(CsrfError::CrossSite)
                );
            }
            assert_eq!(
                csrf(&[
                    ("x-soos-action", a),
                    ("origin", b"https://pc.tail1234.ts.net")
                ]),
                Ok(())
            );
            assert_eq!(
                csrf(&[
                    ("x-soos-action", a),
                    ("origin", b"https://PC.tail1234.ts.net:443")
                ]),
                Ok(())
            );
            for origin in [
                &b"https://evil.example"[..],
                b"http://pc.tail1234.ts.net",
                b"https://pc.tail1234.ts.net:8443",
                b"null",
                b"",
            ] {
                assert_eq!(
                    csrf(&[("x-soos-action", a), ("origin", origin)]),
                    Err(CsrfError::OriginMismatch)
                );
            }
        }
        // Only the three push actions are valid arguments.
        for bad in ["lock", "unlock", "alerts-ack", "", "push-other"] {
            assert_eq!(
                check_push_csrf(
                    &head(PUSH_TEST_PATH, &[("x-soos-action", bad.as_bytes())]),
                    HOST,
                    bad
                ),
                Err(CsrfError::MissingActionHeader),
                "{bad:?}"
            );
        }
    }

    /// Test 41 (RMC73, §6.1, §9): the service worker asset is `assets/sw.js`, served as
    /// JavaScript.
    #[test]
    fn test_rwp_service_worker_asset() {
        let a = asset(AssetId::ServiceWorker);
        assert_eq!(a.content_type, "text/javascript; charset=utf-8");
        let on_disk =
            std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/sw.js"))
                .unwrap();
        assert!(!on_disk.is_empty());
        assert_eq!(a.body, on_disk.as_slice());
    }
}
