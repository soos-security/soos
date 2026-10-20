//! README web capture determinism follow-ups (GitHub #458, part of #356; matrix RAC22, RAC23):
//! the capture browser runs with a pinned locale next to `TZ=UTC`, and the fixed-clock server
//! answers, over HTTP and across requests, exactly the bodies `route` builds from the fixed
//! capture instant, which really depend on the instant.

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
    route, serve_with_clock, FixtureClock, FixtureResponse, WebScene, FIXED_CAPTURE_UNIX_MS,
};

/// The exact environment the capture browser runs with.
const BROWSER_ENV_LINE: &str =
    "BROWSER_ENV=(TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 LANGUAGE=en_US)";

/// The JSON API paths whose bodies are built from the clock.
const CLOCKED_JSON_APIS: [&str; 3] = ["/api/status", "/api/alerts", "/api/offline"];

fn workspace_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The script without blank and comment lines, each line trimmed and every `\`
/// continuation joined to its command.
fn code_lines(script: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut pending = String::new();
    for line in script.lines().map(str::trim) {
        if pending.is_empty() && (line.is_empty() || line.starts_with('#')) {
            continue;
        }
        match line.strip_suffix('\\') {
            Some(head) => {
                pending.push_str(head.trim_end());
                pending.push(' ');
            }
            None => {
                pending.push_str(line);
                lines.push(std::mem::take(&mut pending));
            }
        }
    }
    assert!(pending.is_empty(), "dangling continuation: {pending}");
    lines
}

fn json_body(response: FixtureResponse) -> String {
    match response {
        FixtureResponse::Json { status, body } => {
            assert_eq!(status, 200);
            body
        }
        other => panic!("expected a JSON body, got {other:?}"),
    }
}

/// `GET path` on the fixture server: the response body after a `200 OK` JSON head.
fn get_json_body(addr: SocketAddr, path: &str) -> String {
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
    stream.read_to_string(&mut out).unwrap();
    let (head, body) = out
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("{path}: no head/body separator in {out:?}"));
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{path}: {head}");
    assert!(
        head.contains("\r\nContent-Type: application/json\r\n"),
        "{path}: {head}"
    );
    assert!(
        head.contains(&format!("\r\nContent-Length: {}\r\n", body.len())),
        "{path}: {head}"
    );
    body.to_string()
}

/// RAC22: every browser run of `scripts/capture_readme_web.sh` has the same pinned time zone
/// and locale (`env "${BROWSER_ENV[@]}"`, defined once as `TZ=UTC`, `LANG`/`LC_ALL`
/// `en_US.UTF-8`, `LANGUAGE=en_US`), and the shared browser flags carry `--lang=en-US` and
/// `--accept-lang=en-US`, so the 12/24 h clock and month names never depend on the capture host.
#[test]
fn test_rac_web_capture_pins_the_browser_locale() {
    let script = workspace_file("scripts/capture_readme_web.sh");
    let lines = code_lines(&script);

    let env_definitions: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("BROWSER_ENV="))
        .collect();
    assert_eq!(env_definitions, [BROWSER_ENV_LINE], "one pinned definition");

    let browser_runs: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("\"$browser\"") && !line.contains("[["))
        .collect();
    assert_eq!(browser_runs.len(), 2, "tour-seen run and screenshot run");
    for run in &browser_runs {
        assert!(
            run.starts_with("env \"${BROWSER_ENV[@]}\" \"$browser\" \"${flags[@]}\" "),
            "every browser run uses the pinned environment and the shared flags: {run}"
        );
    }
    assert!(
        browser_runs
            .iter()
            .any(|run| run.contains("http://127.0.0.1:$port/readme/tour-seen")),
        "{browser_runs:?}"
    );
    assert!(
        browser_runs.iter().any(|run| run.contains("--screenshot=")
            && run.contains("http://127.0.0.1:$port/readme/capture")),
        "{browser_runs:?}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("TZ=")),
        "no browser run bypasses the pinned environment"
    );

    let flags_start = lines
        .iter()
        .position(|line| line.starts_with("flags=("))
        .expect("shared browser flags");
    let flags_end = lines
        .iter()
        .skip(flags_start)
        .position(|line| line.ends_with(')'))
        .map(|offset| flags_start + offset)
        .expect("end of the shared browser flags");
    let flags = lines
        .get(flags_start..=flags_end)
        .expect("flag lines")
        .join(" ");
    for flag in ["--lang=en-US", "--accept-lang=en-US"] {
        assert_eq!(flags.matches(flag).count(), 1, "{flag} in {flags}");
    }
    assert_eq!(
        lines.join("\n").matches("--lang=").count(),
        1,
        "no second, conflicting --lang"
    );
}

/// RAC23 (stronger than the body-equality loop of `test_rac_fixed_capture_clock_is_pinned`,
/// which compares `route` at one instant with itself): for every scene, the fixed-clock
/// server answers each clocked JSON API with the exact body `route` builds at
/// `FIXED_CAPTURE_UNIX_MS`, byte-identical across requests more than one second apart, while
/// `route` at another instant gives a different body (the bodies do depend on the instant, so
/// the equality is not vacuous); the event stream prelude is pinned the same way.
#[test]
fn test_rac_fixed_clock_server_bodies_are_pinned_and_instant_dependent() {
    let other_instant = FIXED_CAPTURE_UNIX_MS + 61_000;
    for scene in WebScene::ALL {
        for api in CLOCKED_JSON_APIS {
            let fixed = json_body(route(scene, "GET", api, FIXED_CAPTURE_UNIX_MS));
            let other = json_body(route(scene, "GET", api, other_instant));
            if scene == WebScene::Unlocked && api == "/api/alerts" {
                // The unlocked scene has no alert, hence no timestamp.
                assert_eq!(fixed, other, "{} {api}", scene.name());
            } else {
                assert_ne!(
                    fixed,
                    other,
                    "{} {api} depends on the instant",
                    scene.name()
                );
            }
        }
        let FixtureResponse::EventStream(fixed_events) =
            route(scene, "GET", "/api/events", FIXED_CAPTURE_UNIX_MS)
        else {
            panic!("{} event stream", scene.name());
        };
        let FixtureResponse::EventStream(other_events) =
            route(scene, "GET", "/api/events", other_instant)
        else {
            panic!("{} event stream", scene.name());
        };
        assert_ne!(fixed_events, other_events, "{}", scene.name());
        let fixed_status = json_body(route(scene, "GET", "/api/status", FIXED_CAPTURE_UNIX_MS));
        let fixed_alerts = json_body(route(scene, "GET", "/api/alerts", FIXED_CAPTURE_UNIX_MS));
        assert!(
            fixed_events.starts_with(&format!(
                "event: status\ndata: {fixed_status}\n\nevent: alerts\ndata: {fixed_alerts}\n\n"
            )),
            "{fixed_events}"
        );
    }

    let assets = std::env::temp_dir().join(format!("soos-rac23-{}", std::process::id()));
    std::fs::create_dir_all(&assets).unwrap();
    let mut servers = Vec::new();
    for scene in WebScene::ALL {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let dir = assets.clone();
        let server = std::thread::spawn(move || {
            serve_with_clock(
                listener,
                scene,
                &dir,
                Duration::from_millis(4000),
                FixtureClock::Fixed,
            )
        });
        servers.push((scene, addr, server));
    }

    let first: Vec<Vec<String>> = servers
        .iter()
        .map(|(_, addr, _)| {
            CLOCKED_JSON_APIS
                .iter()
                .map(|api| get_json_body(*addr, api))
                .collect()
        })
        .collect();
    std::thread::sleep(Duration::from_millis(1100));
    for ((scene, addr, _), first_bodies) in servers.iter().zip(&first) {
        for (api, first_body) in CLOCKED_JSON_APIS.iter().zip(first_bodies) {
            let expected = json_body(route(*scene, "GET", api, FIXED_CAPTURE_UNIX_MS));
            assert_eq!(first_body, &expected, "{} {api}", scene.name());
            assert_eq!(
                get_json_body(*addr, api),
                expected,
                "{} {api} after one second",
                scene.name()
            );
        }
    }
    assert_eq!(first.len(), 2);
    assert_ne!(first.first(), first.last(), "the scenes differ");

    for (scene, _, server) in servers {
        let result = server.join().expect("server thread");
        assert!(result.is_ok(), "{} {result:?}", scene.name());
    }
    std::fs::remove_dir_all(&assets).ok();
}
