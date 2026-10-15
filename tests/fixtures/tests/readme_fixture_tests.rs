//! README capture fixtures (GitHub #381, PR 1 of #356; spec `AI/architect_spec_readme_capture.md`
//! §3.3–§3.5; matrix RAC7–RAC9): the rendered placeholder face and the web app fixture routes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use soos_frame::PixelFormat;
use soos_test_fixtures::readme::create_placeholder_face_frame;
use soos_test_fixtures::readme_web::{
    route, serve, strip_png_metadata, FixtureResponse, WebScene, ASSET_NAMES, DEMO_HOST,
};

fn pixel(data: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * width + x) * 3) as usize;
    [data[i], data[i + 1], data[i + 2]]
}

/// RAC7: deterministic, Rgb24, head inside the mock detector box, plain background above it,
/// out-of-range sizes degrade to 16×16 without a panic.
#[test]
fn test_rac_placeholder_face_is_deterministic_and_centred() {
    let a = create_placeholder_face_frame(640, 480);
    let b = create_placeholder_face_frame(640, 480);
    assert_eq!(a.data, b.data, "pure function of the size");
    assert_eq!(
        (a.width, a.height, a.format),
        (640, 480, PixelFormat::Rgb24)
    );
    assert_eq!(a.data.len(), 640 * 480 * 3);

    let background = pixel(&a.data, 640, 2, 2);
    // Top band above the mock box (y < 20 %): background only.
    for y in (0..80).step_by(7) {
        for x in (0..640).step_by(13) {
            assert_eq!(
                pixel(&a.data, 640, x, y),
                background,
                "({x},{y}) above the head"
            );
        }
    }
    // Left and right margins beside the head, above the shoulders: background.
    for y in (100..260).step_by(11) {
        for x in [0, 40, 120, 520, 600, 639] {
            assert_eq!(
                pixel(&a.data, 640, x, y),
                background,
                "({x},{y}) beside the head"
            );
        }
    }
    // The face centre and both eye rows lie inside the box and differ from the background.
    let centre = pixel(&a.data, 640, 320, 250);
    assert_ne!(
        centre, background,
        "the face is drawn at the centre of the mock box"
    );
    let distinct: std::collections::BTreeSet<[u8; 3]> = (160..480)
        .step_by(4)
        .flat_map(|x| (96..384).step_by(4).map(move |y| (x, y)))
        .map(|(x, y)| pixel(&a.data, 640, x, y))
        .collect();
    assert!(
        (3..=64).contains(&distinct.len()),
        "a flat illustration (few colours, no photo texture): {} colours",
        distinct.len()
    );

    // A small size keeps the proportions (head centred).
    let small = create_placeholder_face_frame(320, 240);
    assert_ne!(
        pixel(&small.data, 320, 160, 125),
        pixel(&small.data, 320, 2, 2)
    );

    for (w, h) in [(0, 0), (15, 480), (640, 4097), (u32::MAX, u32::MAX)] {
        let f = create_placeholder_face_frame(w, h);
        assert_eq!((f.width, f.height), (16, 16), "{w}x{h} degrades to 16x16");
        assert_eq!(f.data.len(), 16 * 16 * 3);
    }
}

/// RAC8: only allow-listed assets and the fixed API routes are served; everything else is
/// NotFound.
#[test]
fn test_rac_web_routes_serve_only_allow_listed_assets() {
    let scene = WebScene::Unlocked;
    assert_eq!(
        route(scene, "GET", "/", 0),
        FixtureResponse::Asset("index.html")
    );
    assert_eq!(
        route(scene, "GET", "/index.html", 0),
        FixtureResponse::Asset("index.html")
    );
    assert_eq!(ASSET_NAMES.len(), 11);
    let remote_assets =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/remote/assets");
    let mut on_disk: Vec<String> = std::fs::read_dir(&remote_assets)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = ASSET_NAMES.iter().map(|s| (*s).to_string()).collect();
    listed.sort();
    assert_eq!(listed, on_disk, "ASSET_NAMES mirrors crates/remote/assets");
    for name in ASSET_NAMES {
        assert_eq!(
            route(scene, "GET", &format!("/{name}"), 0),
            FixtureResponse::Asset(name)
        );
    }
    for api in [
        "/api/auth/state",
        "/api/status",
        "/api/alerts",
        "/api/push",
        "/api/camera",
    ] {
        assert!(
            matches!(
                route(scene, "GET", api, 0),
                FixtureResponse::Json { status: 200, .. }
            ),
            "{api}"
        );
    }
    assert!(matches!(
        route(scene, "GET", "/api/events", 0),
        FixtureResponse::EventStream(_)
    ));
    for (method, path) in [
        ("POST", "/api/lock"),
        ("POST", "/api/status"),
        ("GET", "/api/lock"),
        ("GET", "/api/unlock"),
        ("GET", "/../Cargo.toml"),
        ("GET", "/..%2fCargo.toml"),
        ("GET", "/assets/../../etc/passwd"),
        ("GET", "/app.js?x=1"),
        ("GET", "/style.css/"),
        ("GET", "//app.js"),
        ("GET", "/api/camera/stream/abc"),
        ("GET", ""),
        ("HEAD", "/"),
    ] {
        assert_eq!(
            route(scene, method, path, 0),
            FixtureResponse::NotFound,
            "{method} {path}"
        );
    }
}

/// RAC9: no identity in any body; the two scenes show different states.
#[test]
fn test_rac_web_fixture_bodies_carry_no_identity() {
    let now = 1_790_812_800_000u64;
    for scene in WebScene::ALL {
        assert_eq!(WebScene::parse(scene.name()), Some(scene));
        let mut bodies = Vec::new();
        for api in [
            "/api/auth/state",
            "/api/status",
            "/api/alerts",
            "/api/push",
            "/api/camera",
            "/api/events",
        ] {
            match route(scene, "GET", api, now) {
                FixtureResponse::Json { body, .. } | FixtureResponse::EventStream(body) => {
                    bodies.push((api, body));
                }
                other => panic!("{api}: {other:?}"),
            }
        }
        for (api, body) in &bodies {
            for forbidden in [
                "@",
                "ts.net",
                "tailnet.",
                "127.0.0.1",
                "localhost",
                "credential",
                "endpoint",
                "https://",
                "uid",
                "user",
                "login\"",
                DEMO_HOST,
            ] {
                assert!(
                    !body.contains(forbidden),
                    "{} {api} contains `{forbidden}`: {body}",
                    scene.name()
                );
            }
            let ipv4 = body
                .split(|c: char| !(c.is_ascii_digit() || c == '.'))
                .any(|token| {
                    token.split('.').count() == 4 && token.split('.').all(|p| !p.is_empty())
                });
            assert!(!ipv4, "{api} contains an IPv4 address: {body}");
        }
        let events = &bodies
            .iter()
            .find(|(api, _)| *api == "/api/events")
            .unwrap()
            .1;
        for event in ["event: status\n", "event: alerts\n", "event: battery\n"] {
            assert!(events.contains(event), "{events}");
        }
        assert!(events.ends_with("\n\n"), "SSE frames end with a blank line");
    }
    let unlocked = route(WebScene::Unlocked, "GET", "/api/status", now);
    let locked = route(WebScene::LockedWithAlerts, "GET", "/api/status", now);
    let FixtureResponse::Json { body: unlocked, .. } = unlocked else {
        panic!()
    };
    let FixtureResponse::Json { body: locked, .. } = locked else {
        panic!()
    };
    assert!(unlocked.contains("\"state\":\"unlocked\""), "{unlocked}");
    assert!(locked.contains("\"state\":\"locked\""), "{locked}");
    assert!(unlocked.contains(&format!("\"checked_unix_ms\":{now}")));
    let FixtureResponse::Json { body: alerts, .. } =
        route(WebScene::LockedWithAlerts, "GET", "/api/alerts", now)
    else {
        panic!()
    };
    assert!(
        alerts.contains("\"unacknowledged_wrong_password\":2"),
        "{alerts}"
    );
    assert!(alerts.contains("\"last_source\":\"sudo\""), "{alerts}");
    assert!(WebScene::parse("nope").is_none());
}

/// RAC8 (server): the loopback server answers an allow-listed asset, refuses traversal and
/// stops by itself.
#[test]
fn test_rac_web_fixture_server_serves_assets_and_refuses_the_rest() {
    let dir = std::env::temp_dir().join(format!("soos-rac8-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), b"<!doctype html><title>t</title>").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let assets = dir.clone();
    let server = std::thread::spawn(move || {
        serve(
            listener,
            WebScene::Unlocked,
            &assets,
            Duration::from_millis(1500),
        )
    });

    let get = |path: &str| -> String {
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
    };
    let index = get("/");
    assert!(index.starts_with("HTTP/1.1 200"), "{index}");
    assert!(
        index.contains("text/html") && index.contains("<title>t</title>"),
        "{index}"
    );
    let status = get("/api/status");
    assert!(
        status.contains("application/json") && status.contains("\"unlocked\""),
        "{status}"
    );
    assert!(get("/../Cargo.toml").starts_with("HTTP/1.1 404"));
    // A file of the allow-list that is absent from the directory is a 404, never a panic.
    assert!(get("/app.js").starts_with("HTTP/1.1 404"));

    let result = server.join().expect("server thread");
    assert!(result.is_ok(), "{result:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// Top-level keys of a flat JSON object body (the fixture bodies are compact `{"k":v,...}`).
fn top_level_keys(body: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut current = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            if c == '"' {
                in_string = false;
                if depth == 1 && chars.peek() == Some(&':') {
                    keys.push(current.clone());
                }
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                current.clear();
            }
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    keys
}

fn json_body(scene: WebScene, path: &str) -> String {
    match route(scene, "GET", path, 1_790_812_800_000) {
        FixtureResponse::Json { body, .. } => body,
        other => panic!("{path}: {other:?}"),
    }
}

/// The `pub` field names of `pub struct <name>` in a remote source file.
fn struct_fields(rel: &str, name: &str) -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let source = std::fs::read_to_string(&path).unwrap();
    let start = source
        .find(&format!("pub struct {name} {{"))
        .unwrap_or_else(|| panic!("{name} in {rel}"));
    let body = &source[start..start + source[start..].find("\n}").unwrap()];
    let mut fields = Vec::new();
    let mut skip_next = false;
    for line in body.lines().skip(1) {
        let line = line.trim();
        if line.starts_with("#[serde(skip") {
            skip_next = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("pub ") {
            if let Some((field, _)) = rest.split_once(':') {
                if !skip_next {
                    fields.push(field.trim().to_string());
                }
                skip_next = false;
            }
        }
    }
    fields
}

/// RAC9 (F-6): the fixture bodies carry exactly the serialized keys of the remote types.
#[test]
fn test_rac_web_fixture_keys_match_the_remote_types() {
    for scene in WebScene::ALL {
        let status = json_body(scene, "/api/status");
        assert_eq!(
            top_level_keys(&status),
            struct_fields("crates/remote/src/status.rs", "StatusView"),
            "{status}"
        );
        let alerts = json_body(scene, "/api/alerts");
        assert_eq!(
            top_level_keys(&alerts),
            struct_fields("crates/remote/src/alerts.rs", "AlertsView"),
            "{alerts}"
        );
        let push = json_body(scene, "/api/push");
        assert_eq!(
            top_level_keys(&push),
            struct_fields("crates/remote/src/push.rs", "PushView"),
            "{push}"
        );
        assert!(push.starts_with("{\"state\":\"disabled\""), "{push}");
        let FixtureResponse::EventStream(events) = route(scene, "GET", "/api/events", 0) else {
            panic!("events")
        };
        let battery = events
            .split("event: battery\ndata: ")
            .nth(1)
            .and_then(|rest| rest.lines().next())
            .expect("battery event");
        assert_eq!(
            top_level_keys(battery),
            struct_fields("crates/remote/src/battery.rs", "BatteryView"),
            "{battery}"
        );
    }
    let alerts = json_body(WebScene::LockedWithAlerts, "/api/alerts");
    let record = &alerts[alerts.find("\"history\":[").unwrap() + 11..];
    let record = &record[..=record.find('}').unwrap()];
    assert_eq!(
        top_level_keys(record),
        struct_fields("crates/remote/src/alerts.rs", "AlertRecord"),
        "{record}"
    );
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&[1, 2, 3, 4]);
    out
}

/// RAC9 (F-7): browser screenshots lose every ancillary chunk; APNG and garbage are refused.
#[test]
fn test_rac_strip_png_metadata_keeps_only_image_chunks() {
    let signature = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let ihdr = png_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    let idat = png_chunk(b"IDAT", &[9, 9]);
    let iend = png_chunk(b"IEND", &[]);
    let mut dirty = signature.to_vec();
    dirty.extend(&ihdr);
    dirty.extend(png_chunk(b"tEXt", b"Software\0Chrome"));
    dirty.extend(png_chunk(b"iCCP", b"icc"));
    dirty.extend(png_chunk(b"pHYs", &[0; 9]));
    dirty.extend(&idat);
    dirty.extend(png_chunk(b"eXIf", b"exif"));
    dirty.extend(&iend);
    let mut expected = signature.to_vec();
    expected.extend(&ihdr);
    expected.extend(&idat);
    expected.extend(&iend);
    assert_eq!(strip_png_metadata(&dirty).unwrap(), expected);
    assert_eq!(
        strip_png_metadata(&expected).unwrap(),
        expected,
        "idempotent"
    );

    let mut apng = signature.to_vec();
    apng.extend(&ihdr);
    apng.extend(png_chunk(b"acTL", &[0; 8]));
    apng.extend(&idat);
    apng.extend(&iend);
    assert!(strip_png_metadata(&apng).is_err(), "animation refused");
    assert!(strip_png_metadata(b"GIF89a").is_err());
    assert!(
        strip_png_metadata(&expected[..expected.len() - 2]).is_err(),
        "truncated"
    );
    let mut no_end = signature.to_vec();
    no_end.extend(&ihdr);
    no_end.extend(&idat);
    assert!(strip_png_metadata(&no_end).is_err(), "missing IEND");
}

/// The serialized (snake_case) variant names of `pub enum <name>` in a remote source file.
fn enum_variants_snake_case(rel: &str, name: &str) -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let source = std::fs::read_to_string(&path).unwrap();
    let start = source
        .find(&format!("pub enum {name} {{"))
        .unwrap_or_else(|| panic!("{name} in {rel}"));
    let body = &source[start..start + source[start..].find("\n}").unwrap()];
    let mut variants = Vec::new();
    for line in body.lines().skip(1) {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with("#[") || line.is_empty() {
            continue;
        }
        let ident = line.trim_end_matches(',');
        let mut snake = String::new();
        for (i, c) in ident.chars().enumerate() {
            if c.is_ascii_uppercase() {
                if i > 0 {
                    snake.push('_');
                }
                snake.push(c.to_ascii_lowercase());
            } else {
                snake.push(c);
            }
        }
        variants.push(snake);
    }
    variants
}

/// The string value of `"key":"value"` at the top level of a flat object (`None` for `null`).
fn string_field(object: &str, key: &str) -> Option<String> {
    let rest = &object[object.find(&format!("\"{key}\":")).unwrap() + key.len() + 3..];
    if rest.starts_with("null") {
        return None;
    }
    let rest = rest.strip_prefix('"').unwrap();
    Some(rest[..rest.find('"').unwrap()].to_string())
}

fn number_field(object: &str, key: &str) -> u64 {
    let rest = &object[object.find(&format!("\"{key}\":")).unwrap() + key.len() + 3..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().unwrap()
}

/// The objects of the `"intervals":[...]` array (flat objects, no nesting).
fn interval_objects(body: &str) -> Vec<String> {
    let start = body.find("\"intervals\":[").unwrap() + 13;
    let array = &body[start..start + body[start..].find(']').unwrap()];
    array
        .split("},{")
        .filter(|s| !s.is_empty())
        .map(|s| {
            let s = s.trim_start_matches('{').trim_end_matches('}');
            format!("{{{s}}}")
        })
        .collect()
}

/// RAC19 (GitHub #398): `GET /api/offline` serves a synthetic, identity-free offline history with
/// exactly the serialized keys and enum values of `OfflineView` / `OfflineInterval`, so the web
/// captures show the "Offline history" card.
#[test]
fn test_rac_web_fixture_serves_the_offline_history() {
    const OFFLINE: &str = "crates/remote/src/offline.rs";
    let now = 1_790_812_800_000u64;
    let states = enum_variants_snake_case(OFFLINE, "OfflineState");
    let monitors = enum_variants_snake_case(OFFLINE, "NetworkMonitor");
    let reasons = enum_variants_snake_case(OFFLINE, "OfflineReason");
    assert_eq!(states, ["disabled", "active", "unavailable"]);
    assert_eq!(
        reasons,
        [
            "network_lost",
            "wifi_disabled",
            "sleep",
            "shutdown_or_crash"
        ]
    );
    let mut bodies = Vec::new();
    for scene in WebScene::ALL {
        let body = match route(scene, "GET", "/api/offline", now) {
            FixtureResponse::Json { status: 200, body } => body,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            top_level_keys(&body),
            struct_fields(OFFLINE, "OfflineView"),
            "{body}"
        );
        // The card is shown only for `active`; `reason` is set only when unavailable.
        assert_eq!(string_field(&body, "state").as_deref(), Some("active"));
        assert_eq!(string_field(&body, "reason"), None, "{body}");
        let monitor = string_field(&body, "network_monitor").unwrap();
        assert!(monitors.contains(&monitor), "{monitor}");
        let intervals = interval_objects(&body);
        assert!(!intervals.is_empty(), "{body}");
        let mut previous_start = u64::MAX;
        for interval in &intervals {
            assert_eq!(
                top_level_keys(interval),
                struct_fields(OFFLINE, "OfflineInterval"),
                "{interval}"
            );
            let reason = string_field(interval, "reason").unwrap();
            assert!(reasons.contains(&reason), "{reason}");
            let start = number_field(interval, "start_unix_ms");
            let end = number_field(interval, "end_unix_ms");
            assert!(start <= end && end <= now, "{interval}");
            assert_eq!(number_field(interval, "duration_ms"), end - start);
            assert!(start <= previous_start, "newest first: {body}");
            previous_start = start;
        }
        for forbidden in [
            "@",
            "ts.net",
            "tailnet.",
            "127.0.0.1",
            "localhost",
            "credential",
            "endpoint",
            "https://",
            "uid",
            "user",
            "login\"",
            "ssid",
            DEMO_HOST,
        ] {
            assert!(!body.contains(forbidden), "{forbidden}: {body}");
        }
        assert!(
            !body
                .split(|c: char| !(c.is_ascii_digit() || c == '.'))
                .any(|t| t.split('.').count() == 4 && t.split('.').all(|p| !p.is_empty())),
            "{body}"
        );
        // Times follow the capture clock and never underflow.
        match route(scene, "GET", "/api/offline", 0) {
            FixtureResponse::Json { status: 200, body } => {
                for interval in interval_objects(&body) {
                    assert_eq!(number_field(&interval, "start_unix_ms"), 0, "{body}");
                    assert_eq!(number_field(&interval, "duration_ms"), 0, "{body}");
                }
            }
            other => panic!("{other:?}"),
        }
        for method in ["HEAD", "POST"] {
            assert_eq!(
                route(scene, method, "/api/offline", now),
                FixtureResponse::NotFound
            );
        }
        bodies.push(body);
    }
    assert_ne!(bodies[0], bodies[1], "the scenes show different histories");
}
