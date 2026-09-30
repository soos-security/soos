//! Sensitive data redaction engine for daemon logs and diagnostic outputs.
//!
//! Provides defense-in-depth sanitization of sensitive data (passwords, tokens,
//! cryptographic keys, and biometric embeddings) before logs or diagnostic outputs
//! are displayed on terminal streams.

pub trait RedactionFilter: Send + Sync {
    /// Redacts sensitive patterns in a log line.
    fn redact(&self, line: &str) -> String;
}

/// Default redaction filter applying sensitive pattern sanitization.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultRedactionFilter;

impl RedactionFilter for DefaultRedactionFilter {
    fn redact(&self, line: &str) -> String {
        redact_line(line)
    }
}

/// Convenience function executing default redaction on a string slice.
#[must_use]
pub fn default_redact(line: &str) -> String {
    DefaultRedactionFilter.redact(line)
}

/// Core redaction pipeline scanning for sensitive key-value pairs, tokens, and vectors.
fn redact_line(line: &str) -> String {
    let mut result = line.to_string();

    // 1. Redact biometric embedding vectors: `embedding: [...]` or `vector: [...]`
    result = redact_brackets(&result, "embedding");
    result = redact_brackets(&result, "vector");

    // 2. Redact passwords and credentials
    result = redact_kv_field(&result, "password");
    result = redact_kv_field(&result, "pass");

    // 3. Redact tokens and secrets
    result = redact_kv_field(&result, "token");
    result = redact_kv_field(&result, "secret");
    result = redact_bearer_token(&result);

    // 4. Redact master key and cryptographic key labels
    result = redact_kv_field(&result, "master_key");

    // 5. Redact raw 64+ char hex strings (e.g. SHA-256 or 256-bit keys)
    result = redact_long_hex(&result, 64);

    result
}

/// Returns an ASCII-only lowercase copy of `input` with exactly the same byte layout.
///
/// Every byte offset found in the returned string is a valid offset into `input`
/// (GitHub #229): only ASCII bytes are changed and a non-ASCII UTF-8 byte (`>= 0x80`) can
/// never match the ASCII keys searched by this module. `str::to_lowercase` must never be used
/// here because characters such as `İ` (U+0130) or the KELVIN SIGN (U+212A) change their
/// UTF-8 length when lowercased, which shifted every later offset and disabled redaction.
fn ascii_lowered(input: &str) -> String {
    input.to_ascii_lowercase()
}

/// Redacts content inside bracket pairs following a key (e.g. `embedding: [0.123, -0.456]`).
fn redact_brackets(input: &str, key: &str) -> String {
    let lower = ascii_lowered(input);
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(pos) = lower.get(cursor..).and_then(|s| s.find(key)) {
        let match_idx = cursor.saturating_add(pos);
        let key_end = match_idx.saturating_add(key.len());

        // Push preceding content
        if let Some(prefix) = input.get(cursor..key_end) {
            out.push_str(prefix);
        }
        cursor = key_end;

        // Check if colon or equals sign followed by bracket exists
        if let Some(rem) = input.get(cursor..) {
            let mut skipped: usize = 0;
            let mut found_bracket = false;

            for ch in rem.chars() {
                skipped = skipped.saturating_add(ch.len_utf8());
                if ch == '[' {
                    found_bracket = true;
                    break;
                }
                if ch != ':' && ch != '=' && !ch.is_whitespace() {
                    break;
                }
            }

            if found_bracket {
                if let Some(between) = rem.get(..skipped) {
                    out.push_str(between);
                }
                cursor = cursor.saturating_add(skipped);

                // Find closing bracket
                if let Some(close_pos) = input.get(cursor..).and_then(|s| s.find(']')) {
                    out.push_str("[REDACTED]");
                    cursor = cursor.saturating_add(close_pos).saturating_add(1);
                }
            }
        }
    }

    if let Some(suffix) = input.get(cursor..) {
        out.push_str(suffix);
    }
    out
}

/// Redacts key-value fields such as `password=secret` or `password: "secret"`.
fn redact_kv_field(input: &str, key: &str) -> String {
    let lower = ascii_lowered(input);
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(pos) = lower.get(cursor..).and_then(|s| s.find(key)) {
        let match_idx = cursor.saturating_add(pos);

        // Ensure key is at boundary (start of string or non-alphanumeric preceding)
        let is_boundary = if match_idx == 0 {
            true
        } else {
            input
                .get(..match_idx)
                .and_then(|before| before.chars().next_back())
                .is_some_and(|prev| !(prev.is_alphanumeric() || prev == '_'))
        };

        if !is_boundary {
            let skip_len = pos.saturating_add(key.len());
            if let Some(part) = input.get(cursor..cursor.saturating_add(skip_len)) {
                out.push_str(part);
            }
            cursor = cursor.saturating_add(skip_len);
            continue;
        }

        let key_end = match_idx.saturating_add(key.len());
        if let Some(prefix) = input.get(cursor..key_end) {
            out.push_str(prefix);
        }
        cursor = key_end;

        // Consume delimiter (: or =) and optional spaces
        if let Some(rem) = input.get(cursor..) {
            let mut delim_end: usize = 0;
            let mut found_delim = false;

            for ch in rem.chars() {
                delim_end = delim_end.saturating_add(ch.len_utf8());
                if ch == ':' || ch == '=' {
                    found_delim = true;
                    // Also consume subsequent whitespace
                    let after_delim = rem.get(delim_end..).unwrap_or("");
                    for next_ch in after_delim.chars() {
                        if next_ch.is_whitespace() {
                            delim_end = delim_end.saturating_add(next_ch.len_utf8());
                        } else {
                            break;
                        }
                    }
                    break;
                }
                if !ch.is_whitespace() {
                    break;
                }
            }

            if found_delim {
                if let Some(delim_str) = rem.get(..delim_end) {
                    out.push_str(delim_str);
                }
                cursor = cursor.saturating_add(delim_end);

                // Now redact the value
                if let Some(val_rem) = input.get(cursor..) {
                    if val_rem.starts_with('"') {
                        out.push_str("\"[REDACTED]\"");
                        // find closing quote
                        if let Some(end_quote) = val_rem.get(1..).and_then(|s| s.find('"')) {
                            cursor = cursor.saturating_add(end_quote).saturating_add(2);
                        } else {
                            cursor = input.len();
                        }
                    } else if val_rem.starts_with('\'') {
                        out.push_str("\'[REDACTED]\'");
                        if let Some(end_quote) = val_rem.get(1..).and_then(|s| s.find('\'')) {
                            cursor = cursor.saturating_add(end_quote).saturating_add(2);
                        } else {
                            cursor = input.len();
                        }
                    } else {
                        // Unquoted value: consume until space, comma, or end of string
                        let mut val_len: usize = 0;
                        for ch in val_rem.chars() {
                            if ch.is_whitespace() || ch == ',' || ch == ';' {
                                break;
                            }
                            val_len = val_len.saturating_add(ch.len_utf8());
                        }
                        if val_len > 0 {
                            out.push_str("[REDACTED]");
                            cursor = cursor.saturating_add(val_len);
                        }
                    }
                }
            }
        }
    }

    if let Some(suffix) = input.get(cursor..) {
        out.push_str(suffix);
    }
    out
}

/// Redacts Bearer authorization tokens: `Bearer <token>`.
fn redact_bearer_token(input: &str) -> String {
    let lower = ascii_lowered(input);
    let key = "bearer ";
    if let Some(pos) = lower.find(key) {
        let token_start = pos.saturating_add(key.len());
        let mut out = String::with_capacity(input.len());
        if let Some(prefix) = input.get(..token_start) {
            out.push_str(prefix);
        }
        if let Some(rem) = input.get(token_start..) {
            let mut token_len: usize = 0;
            for ch in rem.chars() {
                if ch.is_whitespace() || ch == ',' || ch == ';' {
                    break;
                }
                token_len = token_len.saturating_add(ch.len_utf8());
            }
            out.push_str("[REDACTED]");
            if let Some(rest) = rem.get(token_len..) {
                out.push_str(rest);
            }
        }
        out
    } else {
        input.to_string()
    }
}

/// Redacts continuous hexadecimal strings of length >= `min_len` (e.g. 64-char keys).
fn redact_long_hex(input: &str, min_len: usize) -> String {
    let mut out = String::with_capacity(input.len());
    let mut word_start = None;
    let chars: Vec<(usize, char)> = input.char_indices().collect();

    let mut last_copied = 0;

    for (i, &(byte_idx, ch)) in chars.iter().enumerate() {
        if ch.is_ascii_hexdigit() {
            if word_start.is_none() {
                word_start = Some((byte_idx, i));
            }
        } else if let Some((start_byte, start_char_idx)) = word_start {
            let hex_len = i.saturating_sub(start_char_idx);
            if hex_len >= min_len {
                if let Some(prefix) = input.get(last_copied..start_byte) {
                    out.push_str(prefix);
                }
                out.push_str("[REDACTED]");
                last_copied = byte_idx;
            }
            word_start = None;
        }
    }

    if let Some((start_byte, start_char_idx)) = word_start {
        let hex_len = chars.len().saturating_sub(start_char_idx);
        if hex_len >= min_len {
            if let Some(prefix) = input.get(last_copied..start_byte) {
                out.push_str(prefix);
            }
            out.push_str("[REDACTED]");
            last_copied = input.len();
        }
    }

    if let Some(rem) = input.get(last_copied..) {
        out.push_str(rem);
    }

    out
}
