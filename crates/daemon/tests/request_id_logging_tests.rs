//! Request nonce logging contract (GitHub #257, review finding DMN-11).
//!
//! The 256-bit `request_id` is the single-use nonce that binds a response to its request.
//! Logging policy D5 (`crates/daemon/src/logging.rs`) permits only an anonymized id, so a
//! log statement may carry the nonce solely as `request_id = %short_request_id(..)`: an
//! 8-hex-digit truncated SHA-256 digest that reveals no nonce bit and cannot be reversed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual security audit tests use assertions, unwrap, and expect"
)]

use std::fs;
use std::path::{Path, PathBuf};

use soos_daemon::logging::{short_request_id, SHORT_REQUEST_ID_HEX_LEN};

const LOG_MACROS: [&str; 6] = [
    "info!(", "warn!(", "error!(", "debug!(", "trace!(", "event!(",
];

/// Placeholder substituted for every `short_request_id(<arg>)` call before scanning.
const SHORT_ID_PLACEHOLDER: &str = "__SOOS_SHORT_ID__";

fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, files);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
}

/// Removes `//` line comments so documentation never matches.
fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns the balanced-parenthesis body that starts right after `open_idx` (an `(`).
fn balanced_body(source: &str, open_idx: usize) -> &str {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    for (offset, &b) in bytes[open_idx..].iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open_idx + 1..open_idx + offset];
                }
            }
            _ => {}
        }
    }
    &source[open_idx + 1..]
}

/// Replaces every `short_request_id(<balanced arg>)` call by the placeholder.
fn replace_short_id_calls(body: &str) -> String {
    let needle = "short_request_id(";
    let mut out = String::new();
    let mut rest = body;
    while let Some(pos) = rest.find(needle) {
        out.push_str(&rest[..pos]);
        let open = pos + needle.len() - 1;
        let arg = balanced_body(rest, open);
        out.push_str(SHORT_ID_PLACEHOLDER);
        rest = &rest[open + arg.len() + 2..];
    }
    out.push_str(rest);
    out
}

/// Every log macro invocation (multi-line aware) of a source file.
fn log_macro_bodies(source: &str) -> Vec<String> {
    let mut bodies = Vec::new();
    for pattern in LOG_MACROS {
        let mut search_from = 0;
        while let Some(rel) = source[search_from..].find(pattern) {
            let start = search_from + rel;
            let open = start + pattern.len() - 1;
            bodies.push(balanced_body(source, open).to_string());
            search_from = open + 1;
        }
    }
    bodies
}

/// Violations of the nonce logging rule inside one log macro body.
fn body_violations(body: &str) -> Vec<String> {
    let scrubbed = replace_short_id_calls(body);
    let compact: String = scrubbed.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut violations = Vec::new();
    let mut rest = compact.as_str();
    while let Some(pos) = rest.find("request_id") {
        let after = &rest[pos + "request_id".len()..];
        let allowed = format!(" = %{SHORT_ID_PLACEHOLDER}");
        if !after.starts_with(&allowed) {
            violations.push(format!("raw nonce reference in `{compact}`"));
            break;
        }
        rest = &after[allowed.len()..];
    }
    for whole in [
        "?req,",
        "?req)",
        "%req,",
        "?request,",
        "?resp,",
        "?response,",
    ] {
        if compact.contains(whole) || compact.ends_with(whole.trim_end_matches([',', ')'])) {
            violations.push(format!("whole request/response logged in `{compact}`"));
        }
    }
    violations
}

#[test]
fn test_scanner_flags_raw_nonce_and_accepts_short_id() {
    // Guard the scanner itself so the source audit below cannot pass vacuously.
    let raw = "warn!(\n    request_id = ?req.request_id,\n    \"msg\"\n);";
    let bodies = log_macro_bodies(raw);
    assert_eq!(bodies.len(), 1);
    assert!(!body_violations(&bodies[0]).is_empty());

    let display = "info!(request_id = %hex(req.request_id), \"msg\");";
    assert!(!body_violations(&log_macro_bodies(display)[0]).is_empty());

    let whole = "debug!(request = ?req, \"msg\");";
    assert!(!body_violations(&log_macro_bodies(whole)[0]).is_empty());

    let short = "warn!(\n request_id = %short_request_id(&req.request_id),\n \"msg\"\n);";
    assert!(body_violations(&log_macro_bodies(short)[0]).is_empty());
}

#[test]
fn test_daemon_logs_never_carry_the_raw_request_nonce() {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src_dir, &mut files);
    assert!(!files.is_empty(), "no daemon sources found");

    let mut violations = Vec::new();
    for file in files {
        let source = strip_line_comments(&fs::read_to_string(&file).expect("read source"));
        for body in log_macro_bodies(&source) {
            for v in body_violations(&body) {
                violations.push(format!("{}: {v}", file.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "Logging policy D5 violation (GitHub #257): log only `request_id = \
         %short_request_id(..)`:\n{}",
        violations.join("\n")
    );
}

#[test]
fn test_short_request_id_is_short_lowercase_hex() {
    let rendered = short_request_id(&[0xAB; 32]).to_string();
    assert_eq!(SHORT_REQUEST_ID_HEX_LEN, 8);
    assert_eq!(rendered.len(), SHORT_REQUEST_ID_HEX_LEN);
    assert!(rendered
        .chars()
        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
    // Debug renders the same anonymized form, never the raw bytes.
    assert_eq!(format!("{:?}", short_request_id(&[0xAB; 32])), rendered);
}

#[test]
fn test_short_request_id_is_deterministic_and_discriminating() {
    let a = [7u8; 32];
    let mut b = a;
    b[31] ^= 1;
    assert_eq!(
        short_request_id(&a).to_string(),
        short_request_id(&a).to_string()
    );
    assert_ne!(
        short_request_id(&a).to_string(),
        short_request_id(&b).to_string()
    );
}

#[test]
fn test_short_request_id_is_not_a_prefix_of_the_nonce() {
    // A digest, not a truncation: no nonce byte appears verbatim in the rendering.
    let nonce: [u8; 32] = core::array::from_fn(|i| u8::try_from(i).unwrap() * 7 + 3);
    let full_hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let short = short_request_id(&nonce).to_string();
    assert!(!full_hex.starts_with(&short));
    assert!(!full_hex.contains(&short));
    // Known-answer vector: SHA-256("soos.request-id.log.v1" || [0u8; 32])[..4].
    assert_eq!(
        short_request_id(&[0u8; 32]).to_string(),
        expected_zero_vector()
    );
}

/// Independent computation of the zero-nonce vector with the same public construction.
fn expected_zero_vector() -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"soos.request-id.log.v1");
    hasher.update([0u8; 32]);
    let digest = hasher.finalize();
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}
