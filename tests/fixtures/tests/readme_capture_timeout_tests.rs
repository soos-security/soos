//! README web capture: every headless browser run is bounded (part of #356; matrix RAC24).
//! Headless Chrome was seen to hang without ever writing the screenshot, which left
//! `scripts/capture_readme_web.sh` waiting forever. Every browser run now starts in the
//! background and `await_browser` waits at most `BROWSER_TIMEOUT_S` seconds (coreutils
//! `timeout`), then stops it (SIGTERM, SIGKILL after `BROWSER_KILL_AFTER_S` seconds) and
//! fails the script with status 124.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and expect"
)]

use std::path::PathBuf;

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

/// The value of the single `NAME=<digits>` definition.
fn integer_setting(lines: &[String], name: &str) -> u64 {
    let prefix = format!("{name}=");
    let values: Vec<&str> = lines
        .iter()
        .filter_map(|line| line.strip_prefix(prefix.as_str()))
        .collect();
    assert_eq!(
        values.len(),
        1,
        "{name} is defined exactly once: {values:?}"
    );
    values[0]
        .parse()
        .unwrap_or_else(|e| panic!("{name}={}: {e}", values[0]))
}

/// The body lines of the shell function `name() {` up to its closing `}`.
fn function_body(lines: &[String], name: &str) -> Vec<String> {
    let header = format!("{name}() {{");
    let start = lines
        .iter()
        .position(|line| *line == header)
        .unwrap_or_else(|| panic!("function {name} is defined"));
    let end = lines
        .iter()
        .skip(start)
        .position(|line| line == "}")
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("end of function {name}"));
    lines[start + 1..end].to_vec()
}

/// The number of milliseconds after `flag ` in the single line carrying it.
fn flag_ms(lines: &[String], flag: &str) -> u64 {
    let needle = format!("{flag} ");
    let carriers: Vec<&String> = lines.iter().filter(|line| line.contains(&needle)).collect();
    assert_eq!(carriers.len(), 1, "{flag}: {carriers:?}");
    let rest = &carriers[0][carriers[0].find(&needle).unwrap() + needle.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits
        .parse()
        .unwrap_or_else(|e| panic!("{flag} {rest}: {e}"))
}

/// RAC24: every browser run of `scripts/capture_readme_web.sh` is bounded. It starts in the
/// background, its pid goes to `browser_pid` and `await_browser` waits for it under coreutils
/// `timeout` (`BROWSER_TIMEOUT_S`); on expiry `stop_browser` sends SIGTERM, then SIGKILL after
/// `BROWSER_KILL_AFTER_S`, and the script exits 124; a failing run fails the script; the exit
/// trap stops a run still going. The bound is well above the virtual-time budget and four
/// bounded runs per scene fit in the fixture server's lifetime.
#[test]
fn test_rac_web_capture_bounds_every_browser_run() {
    let script = workspace_file("scripts/capture_readme_web.sh");
    let lines = code_lines(&script);

    let timeout_s = integer_setting(&lines, "BROWSER_TIMEOUT_S");
    let kill_after_s = integer_setting(&lines, "BROWSER_KILL_AFTER_S");
    let settle_ms = integer_setting(&lines, "SETTLE_MS");
    let serve_ms = flag_ms(&lines, "--serve-ms");
    assert!(kill_after_s >= 1, "a grace period before SIGKILL");
    assert!(
        timeout_s * 1000 >= 5 * settle_ms,
        "the bound ({timeout_s} s) leaves room above the virtual-time budget ({settle_ms} ms)"
    );
    assert!(
        4 * (timeout_s + kill_after_s) * 1000 <= serve_ms,
        "two themes x two runs per scene fit in the server lifetime ({serve_ms} ms)"
    );

    // Every browser run starts in the background and is awaited right away.
    let runs: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("\"$browser\"") && !line.contains("[["))
        .map(|(idx, _)| idx)
        .collect();
    assert_eq!(runs.len(), 2, "tour-seen run and screenshot run");
    for idx in runs {
        let run = &lines[idx];
        assert!(
            run.ends_with(" > /dev/null 2>&1 &"),
            "background run: {run}"
        );
        assert_eq!(
            lines.get(idx + 1).map(String::as_str),
            Some("browser_pid=$!")
        );
        assert!(
            lines
                .get(idx + 2)
                .is_some_and(|line| line.starts_with("await_browser \"")),
            "the run is awaited right away: {:?}",
            lines.get(idx + 2)
        );
    }

    let await_body = function_body(&lines, "await_browser");
    assert!(
        await_body
            .iter()
            .any(|line| line
                .contains("timeout \"${BROWSER_TIMEOUT_S}s\" tail --pid=\"$browser_pid\"")),
        "await_browser waits under coreutils timeout: {await_body:?}"
    );
    let expiry = await_body
        .iter()
        .position(|line| line.contains("did not finish within"))
        .expect("loud message on expiry");
    assert!(
        await_body[expiry..]
            .iter()
            .take_while(|line| *line != "fi")
            .any(|line| line == "exit 124")
            && await_body[expiry..]
                .iter()
                .take_while(|line| *line != "fi")
                .any(|line| line == "stop_browser"),
        "expiry stops the run and fails with 124: {await_body:?}"
    );
    assert!(
        await_body
            .iter()
            .any(|line| line == "wait \"$browser_pid\" || status=$?")
            && await_body.iter().any(|line| line == "exit \"$status\""),
        "a failing run fails the script: {await_body:?}"
    );

    let stop_body = function_body(&lines, "stop_browser");
    for needle in [
        "kill -TERM \"$browser_pid\"",
        "timeout \"${BROWSER_KILL_AFTER_S}s\" tail --pid=\"$browser_pid\"",
        "kill -KILL \"$browser_pid\"",
    ] {
        assert!(
            stop_body.iter().any(|line| line.contains(needle)),
            "stop_browser: {needle} in {stop_body:?}"
        );
    }

    let cleanup_body = function_body(&lines, "cleanup");
    assert!(
        cleanup_body.iter().any(|line| line == "stop_browser"),
        "the exit trap stops a run still going: {cleanup_body:?}"
    );
    assert!(
        lines.iter().any(|line| line == "trap cleanup EXIT"),
        "cleanup runs on exit"
    );
}
