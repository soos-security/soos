//! Contractual tests for adding user to soos group (Sub-issue #26.4).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

use soos_admin_cli::user::{add_user_to_group_with_runner, validate_username};

#[test]
fn test_add_user_to_soos_group() {
    // 1. Valid usernames
    assert!(validate_username("alice").is_ok());
    assert!(validate_username("bob_123").is_ok());
    assert!(validate_username("_service-account").is_ok());
    assert!(validate_username("systemd-coredump").is_ok());

    // 2. Invalid usernames must fail closed
    assert!(validate_username("").is_err());
    assert!(validate_username("user with spaces").is_err());
    assert!(validate_username("user;rm -rf /").is_err());
    assert!(validate_username("user$name").is_err());
    assert!(validate_username("-badstart").is_err());
    assert!(validate_username(&"a".repeat(33)).is_err());

    // 3. Command execution pattern: usermod -aG soos <username>
    let mut executed_cmd = None;
    let result = add_user_to_group_with_runner("alice", "soos", |cmd, args| {
        executed_cmd = Some((
            cmd.to_string(),
            args.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
        ));
        Ok(())
    });

    assert!(result.is_ok());
    let (cmd, args) = executed_cmd.expect("mock runner was called");
    assert_eq!(cmd, "usermod");
    assert_eq!(args, vec!["-aG", "soos", "alice"]);
}
