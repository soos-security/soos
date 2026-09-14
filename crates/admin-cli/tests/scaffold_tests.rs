//! Contractual tests for CLI scaffolding and argument parsing (Sub-issue #11.1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

use clap::Parser;
use std::path::PathBuf;

use soos_admin_cli::args::{
    Cli, Commands, OutputFormat, DEFAULT_SERVICE, DEFAULT_SOCKET_PATH, DEFAULT_SYSTEMD_UNIT,
    DEFAULT_TIMEOUT_MS,
};

#[test]
fn test_cli_parse_status_defaults() {
    let args = ["soos-admin", "status"];
    let cli =
        Cli::try_parse_from(args).expect("parsing status subcommand with defaults must succeed");

    assert_eq!(cli.socket_path, None);
    assert_eq!(cli.format, OutputFormat::Table);

    match cli.command {
        Commands::Status(status_args) => {
            assert_eq!(status_args.unit, DEFAULT_SYSTEMD_UNIT);
        }
        _ => panic!("expected Commands::Status variant"),
    }
}

#[test]
fn test_cli_parse_status_custom_unit_and_global_flags() {
    let args = [
        "soos-admin",
        "--socket-path",
        "/tmp/custom_daemon.sock",
        "--format",
        "json",
        "status",
        "--unit",
        "soos-custom.service",
    ];
    let cli = Cli::try_parse_from(args).expect("parsing status with custom flags must succeed");

    assert_eq!(
        cli.socket_path,
        Some(PathBuf::from("/tmp/custom_daemon.sock"))
    );
    assert_eq!(cli.format, OutputFormat::Json);

    match cli.command {
        Commands::Status(status_args) => {
            assert_eq!(status_args.unit, "soos-custom.service");
        }
        _ => panic!("expected Commands::Status variant"),
    }
}

#[test]
fn test_cli_parse_test_pam_defaults() {
    let args = ["soos-admin", "test-pam"];
    let cli =
        Cli::try_parse_from(args).expect("parsing test-pam subcommand with defaults must succeed");

    match cli.command {
        Commands::TestPam(test_args) => {
            assert_eq!(test_args.uid, None);
            assert_eq!(test_args.service, DEFAULT_SERVICE);
            assert_eq!(test_args.timeout_ms, DEFAULT_TIMEOUT_MS);
        }
        _ => panic!("expected Commands::TestPam variant"),
    }
}

#[test]
fn test_cli_parse_test_pam_custom_args() {
    let args = [
        "soos-admin",
        "test-pam",
        "--uid",
        "1001",
        "--service",
        "sudo-custom",
        "--timeout-ms",
        "500",
    ];
    let cli =
        Cli::try_parse_from(args).expect("parsing test-pam with custom arguments must succeed");

    match cli.command {
        Commands::TestPam(test_args) => {
            assert_eq!(test_args.uid, Some(1001));
            assert_eq!(test_args.service, "sudo-custom");
            assert_eq!(test_args.timeout_ms, 500);
        }
        _ => panic!("expected Commands::TestPam variant"),
    }
}

#[test]
fn test_cli_parse_logs_defaults() {
    let args = ["soos-admin", "logs"];
    let cli =
        Cli::try_parse_from(args).expect("parsing logs subcommand with defaults must succeed");

    match cli.command {
        Commands::Logs(logs_args) => {
            assert_eq!(logs_args.lines, 50);
            assert!(!logs_args.follow);
            assert_eq!(logs_args.priority, None);
            assert_eq!(logs_args.since, None);
            assert_eq!(logs_args.unit, DEFAULT_SYSTEMD_UNIT);
            assert_eq!(logs_args.file, None);
        }
        _ => panic!("expected Commands::Logs variant"),
    }
}

#[test]
fn test_cli_parse_logs_custom_args() {
    let args = [
        "soos-admin",
        "logs",
        "-n",
        "100",
        "-f",
        "-p",
        "warning",
        "--since",
        "1 hour ago",
        "-u",
        "soos-daemon-test",
        "--file",
        "/tmp/soos_test.log",
    ];
    let cli = Cli::try_parse_from(args).expect("parsing logs with custom arguments must succeed");

    match cli.command {
        Commands::Logs(logs_args) => {
            assert_eq!(logs_args.lines, 100);
            assert!(logs_args.follow);
            assert_eq!(logs_args.priority.as_deref(), Some("warning"));
            assert_eq!(logs_args.since.as_deref(), Some("1 hour ago"));
            assert_eq!(logs_args.unit, "soos-daemon-test");
            assert_eq!(logs_args.file, Some(PathBuf::from("/tmp/soos_test.log")));
        }
        _ => panic!("expected Commands::Logs variant"),
    }
}

#[test]
fn test_cli_constants_conform_to_spec() {
    assert_eq!(DEFAULT_SOCKET_PATH, "/run/soos/daemon.sock");
    assert_eq!(DEFAULT_SYSTEMD_UNIT, "soos-daemon");
    assert_eq!(DEFAULT_TIMEOUT_MS, 250);
    assert_eq!(DEFAULT_SERVICE, "soos-admin");
}
