//! Contractual tests for log retrieval and redaction filtering (Sub-issue #11.4).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

use std::fs;
use tempfile::tempdir;

use soos_admin_cli::args::LogsArgs;
use soos_admin_cli::logs::fetch_and_filter_logs;
use soos_admin_cli::redact::DefaultRedactionFilter;

#[test]
fn test_fetch_and_filter_logs_from_file_with_redaction() {
    let dir = tempdir().expect("tempdir");
    let log_file = dir.path().join("daemon.log");

    let raw_logs = "\
Sep 14 10:00:01 host soos-daemon[100]: Starting soos-daemon
Sep 14 10:00:02 host soos-daemon[100]: User 1000 provided password=\"VerySecretPass!\" for auth
Sep 14 10:00:03 host soos-daemon[100]: Loaded master_key: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
Sep 14 10:00:04 host soos-daemon[100]: Face vector embedding: [0.123, -0.456, 0.789]
Sep 14 10:00:05 host soos-daemon[100]: Verified UID 1000 in 22ms
";
    fs::write(&log_file, raw_logs).expect("write log fixture");

    let args = LogsArgs {
        lines: 10,
        follow: false,
        priority: None,
        since: None,
        unit: "soos-daemon".to_string(),
        file: Some(log_file),
    };

    let filter = DefaultRedactionFilter;
    let mut output = Vec::new();

    fetch_and_filter_logs(&args, &filter, &mut output).expect("fetch_and_filter_logs must succeed");

    let rendered = String::from_utf8(output).expect("valid utf8 output");

    // Must NOT contain sensitive data
    assert!(!rendered.contains("VerySecretPass!"));
    assert!(!rendered.contains("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
    assert!(!rendered.contains("0.123, -0.456, 0.789"));

    // Must contain [REDACTED] replacements
    assert!(rendered.contains("[REDACTED]"));

    // Must retain benign log entries
    assert!(rendered.contains("Starting soos-daemon"));
    assert!(rendered.contains("Verified UID 1000 in 22ms"));
}

#[test]
fn test_fetch_and_filter_logs_limits_line_count() {
    let dir = tempdir().expect("tempdir");
    let log_file = dir.path().join("lines.log");

    let mut content = String::new();
    for i in 1..=20 {
        content.push_str(&format!("Line {i}: soos-daemon info message\n"));
    }
    fs::write(&log_file, content).expect("write lines fixture");

    let args = LogsArgs {
        lines: 5,
        follow: false,
        priority: None,
        since: None,
        unit: "soos-daemon".to_string(),
        file: Some(log_file),
    };

    let filter = DefaultRedactionFilter;
    let mut output = Vec::new();

    fetch_and_filter_logs(&args, &filter, &mut output).expect("fetch_and_filter_logs must succeed");

    let rendered = String::from_utf8(output).expect("valid utf8 output");
    let line_count = rendered.lines().count();
    assert_eq!(line_count, 5, "Output should contain exactly 5 lines");
    assert!(rendered.contains("Line 20"));
}
