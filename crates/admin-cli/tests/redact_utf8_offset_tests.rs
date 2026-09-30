//! Contract tests for GitHub #229 (review finding STO-13): redaction must not be disabled or
//! shifted by characters whose lowercase form has a different UTF-8 length than the original.
//!
//! Before the fix, the redactor searched `input.to_lowercase()` and sliced `input` with the
//! byte offsets found in the lowered copy. `İ` (U+0130, 2 bytes) lowercases to 3 bytes and the
//! KELVIN SIGN (U+212A, 3 bytes) lowercases to the 1-byte `k`, so every later secret on the line
//! was either left in clear text or cut at the wrong byte.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

use soos_admin_cli::redact::default_redact;

/// Characters whose lowercase UTF-8 encoding is longer or shorter than the original.
const LENGTH_CHANGING: [&str; 4] = ["\u{130}", "\u{212A}", "\u{1E9E}", "\u{2126}"];

#[test]
fn test_229_kv_redaction_survives_dotted_capital_i_prefix() {
    let out = default_redact("user=\u{130}bob password=hunter2 done");
    assert!(!out.contains("hunter2"), "password leaked: {out}");
    assert_eq!(out, "user=\u{130}bob password=[REDACTED] done");
}

#[test]
fn test_229_kv_redaction_survives_kelvin_sign_prefix() {
    let out = default_redact("service=\u{212A}de token=abc123xyz next=1");
    assert!(!out.contains("abc123xyz"), "token leaked: {out}");
    assert_eq!(out, "service=\u{212A}de token=[REDACTED] next=1");
}

#[test]
fn test_229_bracket_redaction_survives_length_changing_prefix() {
    for ch in LENGTH_CHANGING {
        let line = format!("user={ch} embedding: [0.125, -0.5] vector=[1.5, 2.5] end");
        let out = default_redact(&line);
        assert!(
            !out.contains("0.125") && !out.contains("1.5"),
            "vector leaked after {ch:?}: {out}"
        );
        assert!(
            out.starts_with(&format!("user={ch} embedding: [")),
            "prefix mangled: {out}"
        );
        assert_eq!(out.matches("[REDACTED]").count(), 2, "{out}");
        assert!(out.ends_with("] end"), "suffix mangled: {out}");
    }
}

#[test]
fn test_229_bearer_redaction_has_no_off_by_one_after_length_changing_prefix() {
    let out = default_redact("\u{130} Bearer abcdef123456 x");
    assert_eq!(out, "\u{130} Bearer [REDACTED] x");

    let out = default_redact("\u{212A}\u{212A} Authorization: Bearer abcdef123456, next");
    assert_eq!(
        out,
        "\u{212A}\u{212A} Authorization: Bearer [REDACTED], next"
    );
}

#[test]
fn test_229_every_length_changing_char_before_every_secret_kind() {
    let secrets = [
        ("password=", "S3cretPw!"),
        ("pass: ", "S3cretPass"),
        ("token=", "Tok3nValue"),
        ("secret=", "Secr3tValue"),
        ("master_key=", "MasterKeyValue"),
    ];
    for ch in LENGTH_CHANGING {
        for (prefix, value) in secrets {
            for repeat in 1..=3 {
                let lead = ch.repeat(repeat);
                let line = format!("svc={lead} {prefix}{value} tail");
                let out = default_redact(&line);
                assert!(
                    !out.contains(value),
                    "secret {value:?} leaked after {repeat}x{ch:?}: {out}"
                );
                assert!(out.ends_with(" tail"), "suffix mangled: {out}");
                assert!(
                    out.starts_with(&format!("svc={lead} ")),
                    "prefix mangled: {out}"
                );
            }
        }
    }
}

#[test]
fn test_229_multibyte_non_alphanumeric_char_is_a_key_boundary() {
    // An em dash or a guillemet before the key is not alphanumeric: the key is at a boundary.
    let out = default_redact("\u{2014}password=hunter2");
    assert_eq!(out, "\u{2014}password=[REDACTED]");
    let out = default_redact("\u{ab}token=abc123\u{bb}");
    assert!(!out.contains("abc123"), "token leaked: {out}");
}

#[test]
fn test_229_multibyte_alphanumeric_char_is_not_a_key_boundary() {
    // `ébypass=...` is a different identifier: an alphanumeric letter precedes `pass`.
    let line = "\u{e9}pass=visible_value";
    assert_eq!(default_redact(line), line);
}

#[test]
fn test_229_uppercase_ascii_keys_still_match() {
    let out = default_redact("\u{130} PASSWORD=hunter2 Token: abc Bearer zzz9");
    assert!(!out.contains("hunter2"), "{out}");
    assert!(!out.contains("abc"), "{out}");
    assert!(!out.contains("zzz9"), "{out}");
}
