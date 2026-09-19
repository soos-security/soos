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

use clap::Parser;
use soos_enrollment_cli::args::{Cli, Commands, OutputFormat};

#[test]
fn test_cli_parse_enroll_subcommand_with_uid() {
    let args = [
        "soos-enroll",
        "enroll",
        "--uid",
        "1000",
        "--frames",
        "10",
        "--yes",
    ];
    let cli = Cli::try_parse_from(args).expect("Failed to parse enroll args");

    match cli.command {
        Commands::Enroll(enroll) => {
            assert_eq!(enroll.uid, Some(1000));
            assert_eq!(enroll.frames, 10);
            assert!(enroll.yes);
            assert_eq!(enroll.model_id, "mobilefacenet");
            assert_eq!(enroll.model_version, "1.0.0");
        }
        _ => panic!("Expected Commands::Enroll"),
    }
}

#[test]
fn test_cli_parse_enroll_subcommand_with_username() {
    let args = ["soos-enroll", "enroll", "-u", "alice", "-y"];
    let cli = Cli::try_parse_from(args).expect("Failed to parse enroll args");

    match cli.command {
        Commands::Enroll(enroll) => {
            assert_eq!(enroll.username.as_deref(), Some("alice"));
            assert_eq!(enroll.uid, None);
            assert!(enroll.yes);
            assert_eq!(enroll.frames, 5); // default frames
        }
        _ => panic!("Expected Commands::Enroll"),
    }
}

#[test]
fn test_cli_parse_verify_subcommand() {
    let args = ["soos-enroll", "verify", "--uid", "1001"];
    let cli = Cli::try_parse_from(args).expect("Failed to parse verify args");

    match cli.command {
        Commands::Verify(verify) => {
            assert_eq!(verify.uid, Some(1001));
            assert_eq!(verify.username, None);
        }
        _ => panic!("Expected Commands::Verify"),
    }
}

#[test]
fn test_cli_parse_delete_subcommand() {
    let args = ["soos-enroll", "delete", "--uid", "1000", "-y"];
    let cli = Cli::try_parse_from(args).expect("Failed to parse delete args");

    match cli.command {
        Commands::Delete(delete) => {
            assert_eq!(delete.uid, Some(1000));
            assert!(delete.yes);
        }
        _ => panic!("Expected Commands::Delete"),
    }
}

#[test]
fn test_cli_parse_list_subcommand_table_and_json() {
    let args_table = ["soos-enroll", "list", "--format", "table"];
    let cli_table = Cli::try_parse_from(args_table).expect("Failed to parse list table args");
    match cli_table.command {
        Commands::List(list) => assert_eq!(list.format, OutputFormat::Table),
        _ => panic!("Expected Commands::List"),
    }

    let args_json = ["soos-enroll", "list", "-f", "json"];
    let cli_json = Cli::try_parse_from(args_json).expect("Failed to parse list json args");
    match cli_json.command {
        Commands::List(list) => assert_eq!(list.format, OutputFormat::Json),
        _ => panic!("Expected Commands::List"),
    }
}

#[test]
fn test_cli_global_options() {
    let args = [
        "soos-enroll",
        "--biometrics-dir",
        "/tmp/soos/bio",
        "--key-file",
        "/tmp/soos/master.key",
        "--models-dir",
        "/tmp/soos/models",
        "--camera-device",
        "/dev/video2",
        "--mock",
        "list",
    ];
    let cli = Cli::try_parse_from(args).expect("Failed to parse global args");

    assert_eq!(
        cli.biometrics_dir.unwrap().to_str().unwrap(),
        "/tmp/soos/bio"
    );
    assert_eq!(
        cli.key_file.unwrap().to_str().unwrap(),
        "/tmp/soos/master.key"
    );
    assert_eq!(
        cli.models_dir.unwrap().to_str().unwrap(),
        "/tmp/soos/models"
    );
    assert_eq!(cli.camera_device.unwrap().to_str().unwrap(), "/dev/video2");
    assert!(cli.mock);
}
