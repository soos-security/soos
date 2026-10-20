//! Reproducible README web captures (GitHub #448, part of #356; matrix RAC21): one fixed
//! synthetic capture instant for the fixture bodies and the page clock, the same for every
//! scene and theme.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and expect"
)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use soos_test_fixtures::readme_web::{
    capture_page, route, serve_with_clock, FixtureClock, FixtureResponse, WebScene,
    CAPTURE_CLOCK_SCRIPT, CAPTURE_PATH, DEMO_HOST, FIXED_CAPTURE_UNIX_MS,
};

/// 2026-10-01T09:41:00Z.
const PINNED_INSTANT_MS: u64 = 1_790_847_660_000;

fn workspace_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn get(addr: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    out
}

/// RAC21: the fixed instant is pinned, the clock script freezes exactly that instant, and the
/// fixed clock gives identical bodies on every call.
#[test]
fn test_rac_fixed_capture_clock_is_pinned() {
    assert_eq!(FIXED_CAPTURE_UNIX_MS, PINNED_INSTANT_MS);
    assert_eq!(FixtureClock::Fixed.now_unix_ms(), PINNED_INSTANT_MS);
    assert_eq!(FixtureClock::parse("fixed"), Some(FixtureClock::Fixed));
    assert_eq!(FixtureClock::parse("real"), Some(FixtureClock::Real));
    assert_eq!(FixtureClock::parse("Fixed"), None);
    assert_eq!(FixtureClock::parse(""), None);
    assert!(FixtureClock::Real.now_unix_ms() > PINNED_INSTANT_MS / 2);

    assert!(CAPTURE_CLOCK_SCRIPT.contains(&format!("const FIXED_MS = {PINNED_INSTANT_MS};")));
    assert!(CAPTURE_CLOCK_SCRIPT.contains("static now() { return FIXED_MS; }"));
    assert!(CAPTURE_CLOCK_SCRIPT.contains("super(FIXED_MS)"));
    assert!(CAPTURE_CLOCK_SCRIPT.contains("window.Date = FixedDate;"));
    for forbidden in [
        "localStorage",
        "fetch(",
        "http:",
        "https:",
        "//",
        "@",
        "user",
        "login",
        DEMO_HOST,
    ] {
        assert!(
            !CAPTURE_CLOCK_SCRIPT.contains(forbidden),
            "clock script contains `{forbidden}`"
        );
    }

    // Weak check: `route` is pure, so this loop compares a body with itself. The stronger
    // server-level check is `readme_capture_locale_tests::
    // test_rac_fixed_clock_server_bodies_are_pinned_and_instant_dependent` (RAC23, GitHub #458).
    for scene in WebScene::ALL {
        for api in ["/api/status", "/api/alerts", "/api/offline", "/api/events"] {
            let now = FixtureClock::Fixed.now_unix_ms();
            assert_eq!(
                route(scene, "GET", api, now),
                route(scene, "GET", api, FixtureClock::Fixed.now_unix_ms()),
                "{} {api}",
                scene.name()
            );
        }
    }
    let FixtureResponse::Json { body, .. } = route(
        WebScene::LockedWithAlerts,
        "GET",
        "/api/alerts",
        FIXED_CAPTURE_UNIX_MS,
    ) else {
        panic!("alerts body");
    };
    assert!(
        body.contains(&format!("\"last_unix_ms\":{}", PINNED_INSTANT_MS - 180_000)),
        "{body}"
    );
}

/// RAC21: `GET /readme/capture` (exact path, `GET` only) is the capture page; it is the
/// assets' `index.html` with only `<base href="/">` and the clock script inserted, the clock
/// script running before `app.js`.
#[test]
fn test_rac_capture_page_freezes_the_page_clock_before_app_js() {
    assert_eq!(CAPTURE_PATH, "/readme/capture");
    for scene in WebScene::ALL {
        assert_eq!(
            route(scene, "GET", CAPTURE_PATH, 0),
            FixtureResponse::CapturePage
        );
        for (method, path) in [
            ("POST", CAPTURE_PATH),
            ("HEAD", CAPTURE_PATH),
            ("GET", "/readme/capture?x=1"),
            ("GET", "/readme/capture/"),
            ("GET", "/readme/capture.html"),
        ] {
            assert_eq!(
                route(scene, method, path, 0),
                FixtureResponse::NotFound,
                "{method} {path}"
            );
        }
    }

    let index = workspace_file("crates/remote/assets/index.html");
    let page = capture_page(&index).expect("index.html has both markers");
    let base = "\n  <base href=\"/\">";
    let clock = format!("<script>{CAPTURE_CLOCK_SCRIPT}</script>\n  ");
    assert_eq!(page.matches(base).count(), 1);
    assert_eq!(page.matches(&clock).count(), 1);
    let base_at = page.find(base).unwrap();
    let clock_at = page.find(&clock).unwrap();
    let app_at = page.find("<script src=\"app.js\"></script>").unwrap();
    assert!(page.find("<head>").unwrap() < base_at);
    assert!(base_at < page.find("<link").unwrap());
    assert!(clock_at < app_at);
    assert_eq!(
        page.replacen(base, "", 1).replacen(&clock, "", 1),
        index,
        "only the base element and the clock script are inserted"
    );

    assert_eq!(capture_page("<html><body></body></html>"), None);
    assert_eq!(capture_page("<head></head><body></body>"), None);
    assert_eq!(
        capture_page("<head><head><script src=\"app.js\"></script>"),
        None
    );

    let script = workspace_file("scripts/capture_readme_web.sh");
    assert!(
        script.contains("--clock fixed"),
        "server runs the fixed clock"
    );
    assert!(
        script.contains("http://127.0.0.1:$port/readme/capture"),
        "the screenshot is taken on the capture page"
    );
}

/// RAC21: the loopback server with the fixed clock answers bodies built from the fixed instant
/// and the capture page from the assets directory (404 when `index.html` lacks a marker).
#[test]
fn test_rac_fixed_clock_server_serves_the_capture_page() {
    let dir = std::env::temp_dir().join(format!("soos-rac21-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><head><title>t</title></head><body><script src=\"app.js\"></script></body>",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let assets = dir.clone();
    let server = std::thread::spawn(move || {
        serve_with_clock(
            listener,
            WebScene::Unlocked,
            &assets,
            Duration::from_millis(1500),
            FixtureClock::Fixed,
        )
    });
    let status = get(addr, "/api/status");
    assert!(
        status.contains(&format!("\"checked_unix_ms\":{PINNED_INSTANT_MS}")),
        "{status}"
    );
    let page = get(addr, CAPTURE_PATH);
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(
        page.contains("Content-Type: text/html; charset=utf-8"),
        "{page}"
    );
    assert!(page.contains("<base href=\"/\">"), "{page}");
    assert!(page.contains(CAPTURE_CLOCK_SCRIPT), "{page}");
    std::fs::write(dir.join("index.html"), "<!doctype html><title>t</title>").unwrap();
    assert!(get(addr, CAPTURE_PATH).starts_with("HTTP/1.1 404"));
    let result = server.join().expect("server thread");
    assert!(result.is_ok(), "{result:?}");
    std::fs::remove_dir_all(&dir).ok();
}
