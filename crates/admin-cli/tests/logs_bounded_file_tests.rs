//! Contract tests for GitHub #312 (review finding STO-NEW-8): `soos-admin logs --file` reads
//! a regular file only (opened `O_NONBLOCK`, so a FIFO is refused instead of blocking), keeps
//! at most the requested tail in a bounded ring buffer (`MAX_LOG_TAIL_LINES`) and bounds every
//! line (`MAX_LOG_LINE_BYTES`), so memory never grows with the size of the file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses direct assertions and unwrap"
)]

use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;

use soos_admin_cli::args::LogsArgs;
use soos_admin_cli::logs::{fetch_and_filter_logs, MAX_LOG_LINE_BYTES, MAX_LOG_TAIL_LINES};
use soos_admin_cli::redact::DefaultRedactionFilter;
use tempfile::tempdir;

fn args_for(path: &std::path::Path, lines: usize) -> LogsArgs {
    LogsArgs {
        lines,
        file: Some(path.to_path_buf()),
        ..LogsArgs::default()
    }
}

fn run(args: &LogsArgs) -> Result<String, String> {
    let mut out = Vec::new();
    fetch_and_filter_logs(args, &DefaultRedactionFilter, &mut out)
        .map(|()| String::from_utf8(out).unwrap())
        .map_err(|e| e.to_string())
}

#[test]
fn test_312_logs_file_refuses_a_fifo_without_blocking() {
    let dir = tempdir().unwrap();
    let fifo = dir.path().join("daemon.log");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    let args = args_for(&fifo, 10);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(run(&args));
    });
    let res = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("logs --file must not block on a FIFO");
    assert!(res.is_err(), "a FIFO is not a log file: {res:?}");
}

#[test]
fn test_312_logs_file_refuses_a_directory() {
    let dir = tempdir().unwrap();
    assert!(run(&args_for(dir.path(), 10)).is_err());
}

#[test]
fn test_312_logs_file_tail_is_bounded_by_the_ring_buffer() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("daemon.log");
    let mut f = std::fs::File::create(&path).unwrap();
    let total = MAX_LOG_TAIL_LINES + 50;
    for i in 0..total {
        writeln!(f, "line {i}").unwrap();
    }
    drop(f);

    // `-n 0` (and any larger request) is clamped to MAX_LOG_TAIL_LINES.
    for lines in [0, usize::MAX] {
        let out = run(&args_for(&path, lines)).unwrap();
        let printed: Vec<&str> = out.lines().collect();
        assert_eq!(printed.len(), MAX_LOG_TAIL_LINES, "lines={lines}");
        assert_eq!(printed.first().copied(), Some("line 50"));
        assert_eq!(
            printed.last().copied(),
            Some(format!("line {}", total - 1).as_str())
        );
    }

    let out = run(&args_for(&path, 3)).unwrap();
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec![
            format!("line {}", total - 3),
            format!("line {}", total - 2),
            format!("line {}", total - 1)
        ]
    );
}

#[test]
fn test_312_logs_file_bounds_each_line_and_tolerates_invalid_utf8() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("daemon.log");
    let mut f = std::fs::File::create(&path).unwrap();
    // Spaces keep the redaction filter from treating the line as an opaque blob.
    f.write_all("log ".repeat(MAX_LOG_LINE_BYTES).as_bytes())
        .unwrap();
    f.write_all(b"\n").unwrap();
    f.write_all(b"bad \xff byte\n").unwrap();
    f.write_all(b"last").unwrap();
    drop(f);

    let out = run(&args_for(&path, 10)).unwrap();
    let printed: Vec<&str> = out.lines().collect();
    assert_eq!(printed.len(), 3, "{printed:?}");
    assert!(
        printed[0].len() <= MAX_LOG_LINE_BYTES + 32,
        "long line bounded"
    );
    assert!(printed[0].starts_with("log log "));
    assert!(printed[1].starts_with("bad "));
    assert_eq!(printed[2], "last");
}
