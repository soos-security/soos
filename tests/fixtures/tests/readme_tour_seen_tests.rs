//! README web captures without the first-run tour overlay (GitHub #448, part of #356; matrix
//! RAC20): the fixture-only page that marks the web app's feature tour as seen before loading
//! `/`, so the committed `web-*.png` captures show the page and not the tour.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and expect"
)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use soos_test_fixtures::readme_web::{
    route, serve, FixtureResponse, WebScene, DEMO_HOST, TOUR_SEEN_PAGE, TOUR_SEEN_PATH,
    TOUR_STORAGE_KEY, TOUR_STORAGE_SEEN,
};

fn workspace_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// RAC20: `GET /readme/tour-seen` (and only `GET` of that exact path) answers the fixed page in
/// every scene; the page stores the exact flag of `app.js` and replaces itself with `/`.
#[test]
fn test_rac_tour_seen_page_marks_the_app_tour_flag() {
    assert_eq!(TOUR_SEEN_PATH, "/readme/tour-seen");
    for scene in WebScene::ALL {
        assert_eq!(
            route(scene, "GET", TOUR_SEEN_PATH, 0),
            FixtureResponse::Page(TOUR_SEEN_PAGE),
            "{}",
            scene.name()
        );
        for (method, path) in [
            ("POST", TOUR_SEEN_PATH),
            ("HEAD", TOUR_SEEN_PATH),
            ("GET", "/readme/tour-seen?x=1"),
            ("GET", "/readme/tour-seen/"),
            ("GET", "/readme/"),
            ("GET", "/readme"),
        ] {
            assert_eq!(
                route(scene, method, path, 0),
                FixtureResponse::NotFound,
                "{method} {path}"
            );
        }
    }

    // The flag is the one the app reads (crates/remote/assets/app.js tour storage block).
    let app = workspace_file("crates/remote/assets/app.js");
    assert!(
        app.contains(&format!("const TOUR_STORAGE_KEY = \"{TOUR_STORAGE_KEY}\";")),
        "app.js tour key changed; update the fixture"
    );
    assert!(
        app.contains(&format!(
            "const TOUR_STORAGE_SEEN = \"{TOUR_STORAGE_SEEN}\";"
        )),
        "app.js tour value changed; update the fixture"
    );
    assert!(TOUR_SEEN_PAGE.starts_with("<!doctype html>"));
    assert!(TOUR_SEEN_PAGE.contains(&format!(
        "window.localStorage.setItem(\"{TOUR_STORAGE_KEY}\", \"{TOUR_STORAGE_SEEN}\")"
    )));
    assert!(TOUR_SEEN_PAGE.contains("try {") && TOUR_SEEN_PAGE.contains("catch (e)"));
    assert!(TOUR_SEEN_PAGE.contains("window.location.replace(\"/\")"));
    assert_eq!(TOUR_SEEN_PAGE.matches("localStorage").count(), 1);

    // No identity and no reference outside the loopback page itself.
    for forbidden in [
        "@",
        "http:",
        "https:",
        "//",
        "src=",
        "href=",
        "fetch(",
        "ts.net",
        "127.0.0.1",
        "localhost",
        "user",
        "login",
        DEMO_HOST,
    ] {
        assert!(
            !TOUR_SEEN_PAGE.contains(forbidden),
            "tour-seen page contains `{forbidden}`"
        );
    }

    // The capture script opens the page first, on the loopback fixture server.
    let script = workspace_file("scripts/capture_readme_web.sh");
    assert!(
        script.contains("http://127.0.0.1:$port/readme/tour-seen"),
        "scripts/capture_readme_web.sh opens the tour-seen page"
    );
}

/// RAC20: the loopback server answers the page as HTML (no asset file needed).
#[test]
fn test_rac_tour_seen_page_is_served_as_html() {
    let dir = std::env::temp_dir().join(format!("soos-rac20-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let assets = dir.clone();
    let server = std::thread::spawn(move || {
        serve(
            listener,
            WebScene::LockedWithAlerts,
            &assets,
            Duration::from_millis(1500),
        )
    });
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET {TOUR_SEEN_PATH} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    assert!(
        out.contains("Content-Type: text/html; charset=utf-8"),
        "{out}"
    );
    assert!(out.ends_with(TOUR_SEEN_PAGE), "{out}");
    let result = server.join().expect("server thread");
    assert!(result.is_ok(), "{result:?}");
    std::fs::remove_dir_all(&dir).ok();
}
