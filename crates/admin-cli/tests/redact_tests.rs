//! Contractual tests for sensitive data redaction filter (Sub-issue #11.4).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

use soos_admin_cli::redact::{default_redact, DefaultRedactionFilter, RedactionFilter};

#[test]
fn test_redact_preserves_benign_logs() {
    let benign_lines = [
        "Sep 14 12:00:00 localhost soos-daemon[1234]: Starting soos-daemon (Linux Local Biometric PAM Daemon)",
        "Sep 14 12:00:01 localhost soos-daemon[1234]: soos-daemon initialized and listening for PAM requests",
        "Sep 14 12:00:05 localhost soos-daemon[1234]: Rendered authentication response peer_uid=1000 verdict=Allow reason=FaceMatch",
        "Sep 14 12:00:10 localhost soos-daemon[1234]: Connection accepted from pid=4567 peer_uid=1000",
        "Sep 14 12:00:12 localhost soos-daemon[1234]: Camera manager warmed up 20 frames discarded",
    ];

    for line in benign_lines {
        let redacted = default_redact(line);
        assert_eq!(redacted, line, "Benign line should not be modified: {line}");
    }
}

#[test]
fn test_redact_masks_password_fields() {
    let filter = DefaultRedactionFilter;

    let input1 = "user entered password=SuperSecretPassword123 during auth";
    let out1 = filter.redact(input1);
    assert!(!out1.contains("SuperSecretPassword123"));
    assert!(out1.contains("[REDACTED]"));

    let input2 = "pam_authenticate failed: password: \"AnotherPassword!\"";
    let out2 = filter.redact(input2);
    assert!(!out2.contains("AnotherPassword!"));
    assert!(out2.contains("[REDACTED]"));
}

#[test]
fn test_redact_masks_tokens_and_secrets() {
    let filter = DefaultRedactionFilter;

    let input1 = "session token=d8e8fca2dc0f896fd7cb4cb0031ba249 granted";
    let out1 = filter.redact(input1);
    assert!(!out1.contains("d8e8fca2dc0f896fd7cb4cb0031ba249"));
    assert!(out1.contains("[REDACTED]"));

    let input2 = "Authorization: Bearer secret_api_token_value_xyz123";
    let out2 = filter.redact(input2);
    assert!(!out2.contains("secret_api_token_value_xyz123"));
    assert!(out2.contains("[REDACTED]"));
}

#[test]
fn test_redact_masks_master_key_and_hex_keys() {
    let filter = DefaultRedactionFilter;

    let input =
        "master_key: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef loaded";
    let out = filter.redact(input);
    assert!(!out.contains("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
    assert!(out.contains("[REDACTED]"));
}

#[test]
fn test_redact_masks_embedding_vector_arrays() {
    let filter = DefaultRedactionFilter;

    let input = "Face vector embedding: [0.12345, -0.67890, 0.43210, -0.98765] extracted";
    let out = filter.redact(input);
    assert!(!out.contains("0.12345, -0.67890"));
    assert!(out.contains("[REDACTED]"));
}

#[test]
fn test_redact_handles_multiple_sensitive_items_in_one_line() {
    let filter = DefaultRedactionFilter;

    let input =
        "Debug event: password=\"pass123\" and token=\"tok456\" with embedding: [0.11, 0.22]";
    let out = filter.redact(input);
    assert!(!out.contains("pass123"));
    assert!(!out.contains("tok456"));
    assert!(!out.contains("0.11, 0.22"));
    assert!(out.contains("[REDACTED]"));
}
