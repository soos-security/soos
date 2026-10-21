//! README web capture: no headless browser process outlives its run (part of #356; matrix
//! RAC25). Headless Chrome forks renderer, GPU and zygote children; stopping only the main pid
//! (RAC24) could leave them running after the script ended or timed out. Every browser run now
//! starts as its own process group (`set -m` around the background start), and the whole group
//! is ended after every run, on a timeout and from the exit trap (SIGTERM, SIGKILL after
//! `BROWSER_KILL_AFTER_S` seconds). INT and TERM exit through the same trap.

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

/// The index of the first line of `body` containing `needle`.
fn line_with(body: &[String], needle: &str, what: &str) -> usize {
    body.iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{what}: `{needle}` in {body:?}"))
}

/// RAC25: every browser run is its own process group and the whole group is ended after the
/// run, on a timeout and from the exit trap; INT and TERM run the exit trap.
#[test]
fn test_rac_web_capture_ends_the_whole_browser_process_group() {
    let script = workspace_file("scripts/capture_readme_web.sh");
    let lines = code_lines(&script);

    // Each of the two browser runs is started with job control on, so that its pid is the
    // process group id of Chrome and of every child it forks, and never reads the terminal.
    let runs: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("\"$browser\"") && !line.contains("[["))
        .map(|(idx, _)| idx)
        .collect();
    assert_eq!(runs.len(), 2, "tour-seen run and screenshot run");
    for &idx in &runs {
        let run = &lines[idx];
        assert_eq!(
            lines.get(idx - 1).map(String::as_str),
            Some("set -m"),
            "job control on right before the run: {run}"
        );
        assert!(
            run.ends_with(" < /dev/null > /dev/null 2>&1 &"),
            "the run never reads the terminal: {run}"
        );
    }
    assert_eq!(
        lines.iter().filter(|line| *line == "set -m").count(),
        runs.len(),
        "job control is only switched on for the browser runs"
    );

    // The group is ended by one bounded helper: SIGTERM, a bounded wait, then SIGKILL.
    let group_body = function_body(&lines, "end_browser_group");
    assert!(
        group_body
            .iter()
            .any(|line| line == "local pgid=\"$1\" tick=0"),
        "end_browser_group takes the process group id: {group_body:?}"
    );
    let term = line_with(
        &group_body,
        "kill -TERM -- \"-$pgid\"",
        "SIGTERM to the group",
    );
    let poll = line_with(
        &group_body,
        "while kill -0 -- \"-$pgid\" 2> /dev/null; do",
        "wait for the group",
    );
    let bound = line_with(
        &group_body,
        "if [[ \"$tick\" -ge $((BROWSER_KILL_AFTER_S * 5)) ]]; then",
        "bounded wait",
    );
    let kill = line_with(
        &group_body,
        "kill -KILL -- \"-$pgid\"",
        "SIGKILL to the group",
    );
    assert!(
        term < poll && poll < bound && bound < kill,
        "SIGTERM, bounded wait, then SIGKILL: {group_body:?}"
    );
    assert!(
        group_body.iter().any(|line| line == "sleep 0.2")
            && group_body.iter().any(|line| line == "tick=$((tick + 1))"),
        "the wait polls every 0.2 s: {group_body:?}"
    );

    // stop_browser signals the group (pid only as a fallback) and ends what is left once the
    // main process is reaped.
    let stop_body = function_body(&lines, "stop_browser");
    for needle in [
        "kill -TERM -- \"-$browser_pid\" 2> /dev/null || kill -TERM \"$browser_pid\"",
        "kill -KILL -- \"-$browser_pid\" 2> /dev/null || kill -KILL \"$browser_pid\"",
    ] {
        line_with(&stop_body, needle, "stop_browser signals the group");
    }
    let reaped = line_with(&stop_body, "wait \"$browser_pid\"", "stop_browser reaps");
    let ended = line_with(
        &stop_body,
        "end_browser_group \"$browser_pid\"",
        "stop_browser ends the group",
    );
    assert!(
        reaped < ended,
        "the group is ended after the reap: {stop_body:?}"
    );

    // await_browser: job control off again first, an interruptible wait (a background watcher
    // under the RAC24 timeout, awaited with the `wait` builtin so INT/TERM act at once), and the
    // group of a finished run is ended before its status is checked.
    let await_body = function_body(&lines, "await_browser");
    assert_eq!(
        await_body.first().map(String::as_str),
        Some("set +m"),
        "job control off again right after the start: {await_body:?}"
    );
    let watcher = line_with(
        &await_body,
        "timeout \"${BROWSER_TIMEOUT_S}s\" tail --pid=\"$browser_pid\" --sleep-interval=0.2 -f /dev/null &",
        "background watcher",
    );
    assert_eq!(
        await_body.get(watcher + 1).map(String::as_str),
        Some("watch_pid=$!")
    );
    assert_eq!(
        await_body.get(watcher + 2).map(String::as_str),
        Some("if ! wait \"$watch_pid\"; then")
    );
    let reaped = line_with(&await_body, "wait \"$browser_pid\" || status=$?", "reap");
    let ended = line_with(
        &await_body,
        "end_browser_group \"$browser_pid\"",
        "await_browser ends the group",
    );
    let checked = line_with(&await_body, "exit \"$status\"", "status check");
    assert!(
        reaped < ended && ended < checked,
        "the group of a finished run is ended before its status is checked: {await_body:?}"
    );

    // The exit trap stops the watcher and the run; INT and TERM exit through it.
    let cleanup_body = function_body(&lines, "cleanup");
    line_with(
        &cleanup_body,
        "kill \"$watch_pid\"",
        "cleanup stops the watcher",
    );
    line_with(&cleanup_body, "stop_browser", "cleanup stops the run");
    for trap in [
        "trap cleanup EXIT",
        "trap 'exit 130' INT",
        "trap 'exit 143' TERM",
    ] {
        assert!(lines.iter().any(|line| line == trap), "{trap}");
    }
}
