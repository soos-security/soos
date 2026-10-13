//! Static contracts of browser capability detection and graceful degradation in the
//! `soos-remote` web app (GitHub #366, PR 1 of #355; ADR 2026-10-13 "Browser Support Policy for
//! the `soos-remote` Web App"; architect spec `AI/architect_spec_remote_cross_browser.md` §8;
//! matrix RXB1–RXB11):
//!
//! - RXB-S1 one pure capability probe, feature detection only;
//! - RXB-S2 every sensitive browser API is called behind the probe;
//! - RXB-S3 passkey controls degrade without WebAuthn, lock and status stay independent;
//! - RXB-S4 push failures map to distinct fixed texts, the enable tap is bounded;
//! - RXB-S5 user messages are fixed strings (a browser message is never shown);
//! - RXB-S6 no User-Agent logic in the page, the worker or the server;
//! - RXB-S7 CSS fallbacks for `:has()`, container units and `dvh`;
//! - RXB-S8 no new network surface (CSP, asset set, no external URL);
//! - RXB-S9 status keeps working without `EventSource`;
//! - RXB-S10 the camera view degrades without streams or image decoding;
//! - RXB-S11 documentation, ADR and walkthrough.
//!
//! The project has no JavaScript runtime in CI: the checks are static. This module carries its
//! own small lexer (strings, template literals and regular expression literals kept intact) so
//! the helpers of the other remote contracts stay untouched.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;

use crate::remote_companion_contract::{
    exists, production_part, read, rust_files, strip_comments, workspace_root,
};

const APP_JS: &str = "crates/remote/assets/app.js";
const SW_JS: &str = "crates/remote/assets/sw.js";
const STYLE: &str = "crates/remote/assets/style.css";
const INDEX: &str = "crates/remote/assets/index.html";
const MANIFEST: &str = "crates/remote/assets/manifest.webmanifest";
const ASSETS_DIR: &str = "crates/remote/assets";
const HTTP_RS: &str = "crates/remote/src/http.rs";
const REMOTE_DOC: &str = "Docs/REMOTE_COMPANION.md";
const DECISIONS: &str = "AI/DECISIONS.md";
const MATRIX: &str = "AI/VERIFICATION_MATRIX.md";
const WALKTHROUGH: &str = "AI/walkthroughs/197_remote_cross_browser.md";

const ADR_TITLE: &str = "Browser Support Policy for the `soos-remote` Web App";

/// The exact Content-Security-Policy value (unchanged by this issue).
const PINNED_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The nine keys of the probe (spec §3.1).
const PROBE_KEYS: [&str; 9] = [
    "webauthn",
    "serviceWorker",
    "push",
    "notifications",
    "standalone",
    "homeScreen",
    "eventSource",
    "streams",
    "imageBitmap",
];

/// `(API token, guard that must precede it in the same top-level function)` (spec D-2).
const GUARDS: [(&str, &str); 7] = [
    ("navigator.credentials", "!browserCapabilities().webauthn"),
    ("navigator.serviceWorker", "!pushSupported()"),
    (".pushManager", "!pushSupported()"),
    ("Notification.", "!pushSupported()"),
    ("new EventSource(", "!browserCapabilities().eventSource"),
    (".getReader(", "!browserCapabilities().streams"),
    ("createImageBitmap(", "!browserCapabilities().imageBitmap"),
];

const PASSKEY_UNSUPPORTED: (&str, &str) = (
    "PASSKEY_UNSUPPORTED_TEXT",
    "This browser cannot use passkeys, so sign-in, unlock, passkey enrollment and the camera view are off here; lock and status still work on the tailnet",
);

const CAMERA_UNSUPPORTED: (&str, &str) = (
    "CAMERA_UNSUPPORTED_TEXT",
    "This browser cannot show the live camera view (no streamed download or image decoding): use Safari, Chrome, Firefox or Samsung Internet",
);

/// The seven new push texts (spec §3.2, §14 R-3).
const NEW_PUSH_TEXTS: [(&str, &str); 7] = [
    (
        "PUSH_HOME_SCREEN_TEXT",
        "On iPhone and iPad, notifications only work in the home-screen app: tap Share, then Add to Home Screen, and open soos from the new icon (iOS 16.4 or later)",
    ),
    (
        "PUSH_NO_WORKER_TEXT",
        "Notifications need a service worker, which this window does not allow (private windows block it): open the page in a normal window",
    ),
    (
        "PUSH_NO_SERVICE_TEXT",
        "This browser has no working push service: in Brave, turn on Use Google services for push messaging; otherwise use Chrome, Firefox, Safari or Samsung Internet",
    ),
    (
        "PUSH_STALE_TEXT",
        "An older notification subscription is in the way: reload the page, then enable notifications again",
    ),
    (
        "PUSH_TIMEOUT_TEXT",
        "The PC did not confirm the subscription in time: check the connection and try again",
    ),
    (
        "PUSH_UNREACHABLE_TEXT",
        "The PC could not be reached to save the subscription: check the connection and try again",
    ),
    (
        "PUSH_SLOW_TEXT",
        "The browser's push service did not answer in time: try again",
    ),
];

/// The four texts pinned by RAN-S6 (kept, never redefined).
const PINNED_PUSH_TEXTS: [&str; 4] = [
    "PASSKEY_PROMPT_TEXT",
    "PUSH_UNSUPPORTED_TEXT",
    "PUSH_BLOCKED_TEXT",
    "PUSH_WORKER_FAILED_TEXT",
];

const UNSUPPORTED_PUSH_SERVICE_ENTRY: &str = "unsupported_push_service: \"The PC does not accept this browser's push service (Edge on a computer, for example): use Chrome, Firefox, Safari or Samsung Internet for notifications\",";

const PUSH_ERROR_TEXTS_BODY: &str = "NotAllowedError: PUSH_BLOCKED_TEXT, AbortError: PUSH_NO_SERVICE_TEXT, NotSupportedError: PUSH_NO_SERVICE_TEXT, InvalidStateError: PUSH_STALE_TEXT, SecurityError: PUSH_NO_WORKER_TEXT,";

const PASSKEY_ERROR_TEXTS_BODY: &str = "NotAllowedError: \"The passkey check was cancelled or timed out\", SecurityError: \"This address cannot use the passkey: open the exact address the passkey was made for\", NotSupportedError: \"This browser or device cannot create this kind of passkey\",";

/// Enrollment adds the `create()`-only `InvalidStateError` (spec §14 R-6).
const ENROLL_ERROR_TEXTS_LINE: &str = "const ENROLL_ERROR_TEXTS = Object.assign({}, PASSKEY_ERROR_TEXTS, { InvalidStateError: PASSKEY_REASONS.already_registered, });";

/// `(:has() selector, fallback selector)` (spec §4).
const HAS_FALLBACKS: [(&str, &str); 5] = [
    (
        ".stat-tile:has(.state-locked) .stat-dot",
        ".stat-tile[data-state=\"locked\"] .stat-dot",
    ),
    (
        ".stat-tile:has(.state-unlocked) .stat-dot",
        ".stat-tile[data-state=\"unlocked\"] .stat-dot",
    ),
    (
        ".stat-tile:has(.state-unavailable) .stat-dot",
        ".stat-tile[data-state=\"unavailable\"] .stat-dot",
    ),
    (
        ".stat-tile:has(.state-unreachable) .stat-dot",
        ".stat-tile[data-state=\"unreachable\"] .stat-dot",
    ),
    (
        ".card > .feedback:has(+ [hidden]:last-child)",
        ".card > .feedback.feedback-last",
    ),
];

const HAS_BLOCK: &str = "@supports not selector(:has(*))";
const CQ_BLOCK: &str = "@supports not (container-type: size)";

// ---------------------------------------------------------------------------------------------
// JavaScript helpers
// ---------------------------------------------------------------------------------------------

/// Whether a `/` at byte `i` of `code` starts a regular expression literal (not a division).
fn starts_regex(code: &[u8], i: usize) -> bool {
    let mut j = i;
    while j > 0 {
        j -= 1;
        let b = code[j];
        if b.is_ascii_whitespace() {
            continue;
        }
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
            let mut k = j + 1;
            while k > 0
                && (code[k - 1].is_ascii_alphanumeric()
                    || code[k - 1] == b'_'
                    || code[k - 1] == b'$')
            {
                k -= 1;
            }
            let word = &code[k..=j];
            return [
                &b"return"[..],
                b"typeof",
                b"case",
                b"in",
                b"of",
                b"void",
                b"delete",
                b"throw",
                b"yield",
                b"await",
            ]
            .contains(&word);
        }
        return b"(,=:[!&|?{};+-*%<>~^".contains(&b);
    }
    true
}

/// End (exclusive) of the string, template or regex literal starting at `i`.
fn literal_end(code: &[u8], i: usize) -> usize {
    let quote = code[i];
    let mut j = i + 1;
    let mut in_class = false;
    while j < code.len() {
        let b = code[j];
        if b == b'\\' {
            j += 2;
            continue;
        }
        if quote == b'/' {
            if b == b'[' {
                in_class = true;
            } else if b == b']' {
                in_class = false;
            } else if b == b'/' && !in_class {
                return j + 1;
            } else if b == b'\n' {
                return j;
            }
        } else if b == quote {
            return j + 1;
        }
        j += 1;
    }
    code.len()
}

/// JavaScript source without comments; literals kept verbatim.
fn strip_js(src: &str) -> String {
    let code = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(code.len());
    let mut i = 0;
    while i < code.len() {
        let b = code[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            let end = literal_end(code, i);
            out.extend_from_slice(&code[i..end]);
            i = end;
            continue;
        }
        if b == b'/' && code.get(i + 1) == Some(&b'/') {
            while i < code.len() && code[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && code.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < code.len() && !(code[i] == b'*' && code[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if b == b'/' && starts_regex(&out, out.len()) {
            let end = literal_end(code, i);
            out.extend_from_slice(&code[i..end]);
            i = end;
            continue;
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8(out).expect("UTF-8 source")
}

/// Index of the `}` matching the `{` at `open` (braces inside literals ignored).
fn matching_brace(code: &str, open: usize) -> usize {
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' || b == b'`' || (b == b'/' && starts_regex(bytes, i)) {
            i = literal_end(bytes, i);
            continue;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
        i += 1;
    }
    panic!("unbalanced block at byte {open}");
}

/// The text between the first `{` at or after `at` and its matching `}`.
fn block_from(code: &str, at: usize) -> String {
    let open = at
        + code[at..]
            .find('{')
            .unwrap_or_else(|| panic!("no block after byte {at}"));
    code[open + 1..matching_brace(code, open)].to_string()
}

/// The body of the function, listener or object literal whose text starts with `signature`.
fn body(code: &str, signature: &str) -> String {
    let at = code
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} not found"));
    block_from(code, at)
}

/// Top-level function declarations `(name, full text)` and the remaining top-level text.
fn split_top_level(code: &str) -> (Vec<(String, String)>, String) {
    let bytes = code.as_bytes();
    let mut functions = Vec::new();
    let mut statements = String::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if depth == 0 && (i == 0 || bytes[i - 1] == b'\n') {
            let rest = &code[i..];
            let decl = if rest.starts_with("function ") {
                Some("function ".len())
            } else if rest.starts_with("async function ") {
                Some("async function ".len())
            } else {
                None
            };
            if let Some(skip) = decl {
                let name_end = rest.find('(').expect("function parameters");
                let name = rest[skip..name_end].trim().to_string();
                let open = i + rest.find('{').expect("function body");
                let close = matching_brace(code, open);
                functions.push((name, code[i..=close].to_string()));
                i = close + 1;
                continue;
            }
        }
        if b == b'"' || b == b'\'' || b == b'`' || (b == b'/' && starts_regex(bytes, i)) {
            let end = literal_end(bytes, i);
            statements.push_str(&code[i..end]);
            i = end;
            continue;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth = depth.saturating_sub(1);
        }
        statements.push(b as char);
        i += 1;
    }
    (functions, statements)
}

fn function_text<'a>(functions: &'a [(String, String)], name: &str) -> &'a str {
    functions
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, t)| t.as_str())
        .unwrap_or_else(|| panic!("function {name} not found"))
}

/// The value of `const NAME = "value";` (no inner double quote).
fn js_const_string(code: &str, name: &str) -> String {
    let prefix = format!("const {name} = \"");
    let squashed = squash(code);
    let at = squashed
        .find(&prefix)
        .unwrap_or_else(|| panic!("const {name} = \"…\"; not found"));
    let rest = &squashed[at + prefix.len()..];
    rest[..rest.find('"').expect("closing quote")].to_string()
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn no_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Whether the guard at `at` is a negated test inside an `if (` condition whose block exits.
fn guard_exits(text: &str, at: usize) -> bool {
    let Some(cond) = text[..at].rfind("if (") else {
        return false;
    };
    if text[cond..at].contains([';', '{', '}']) || !text[at..].starts_with('!') {
        return false;
    }
    let block = block_from(text, at);
    ["return", "throw ", "Promise.reject("]
        .iter()
        .any(|exit| block.contains(exit))
}

/// The top-level argument texts of every call `name(` (literals and nesting honoured).
fn call_arguments(code: &str, name: &str) -> Vec<Vec<String>> {
    let bytes = code.as_bytes();
    let needle = format!("{name}(");
    let mut calls = Vec::new();
    for (at, _) in code.match_indices(&needle) {
        if at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_') {
            continue;
        }
        if code[..at].ends_with("function ") {
            continue;
        }
        let mut args = Vec::new();
        let mut depth = 0usize;
        let mut start = at + needle.len();
        let mut i = start;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'"' || b == b'\'' || b == b'`' || (b == b'/' && starts_regex(bytes, i)) {
                i = literal_end(bytes, i);
                continue;
            }
            match b {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth > 0 => depth -= 1,
                b')' => {
                    args.push(code[start..i].trim().to_string());
                    break;
                }
                b',' if depth == 0 => {
                    args.push(code[start..i].trim().to_string());
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        calls.push(args);
    }
    calls
}

/// Top-level `const`/`let` names whose first top-level use precedes their declaration (a
/// load-time `ReferenceError` that would stop the whole page, lock and status included).
fn used_before_declared(statements: &str) -> Vec<String> {
    // String, template and regex literal contents never name a binding: blank them first.
    let raw = statements.as_bytes();
    let mut blanked = raw.to_vec();
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        if b == b'"' || b == b'\'' || b == b'`' || (b == b'/' && starts_regex(raw, i)) {
            let end = literal_end(raw, i);
            for byte in blanked.iter_mut().take(end.saturating_sub(1)).skip(i + 1) {
                *byte = b' ';
            }
            i = end;
            continue;
        }
        i += 1;
    }
    let blanked = String::from_utf8(blanked).expect("ASCII blanks keep UTF-8 boundaries");
    let statements = blanked.as_str();
    let mut names: Vec<(String, usize)> = Vec::new();
    for (at, _) in statements.match_indices('\n') {
        let line = &statements[at + 1..];
        for kw in ["const ", "let "] {
            if let Some(rest) = line.strip_prefix(kw) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                names.push((name, at + 1 + kw.len()));
            }
        }
    }
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let bytes = statements.as_bytes();
    let mut offenders = Vec::new();
    for (name, declared) in &names {
        let early = statements[..*declared]
            .match_indices(name.as_str())
            .any(|(i, _)| {
                let end = i + name.len();
                (i == 0 || !ident(bytes[i - 1]))
                    && (end >= bytes.len() || !ident(bytes[end]))
                    && (i == 0 || bytes[i - 1] != b'.')
            });
        if early {
            offenders.push(name.clone());
        }
    }
    offenders
}

fn app() -> String {
    strip_js(&read(APP_JS))
}

// ---------------------------------------------------------------------------------------------
// CSS helpers
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Rule {
    at: Option<String>,
    selectors: Vec<String>,
    decls: Vec<(String, String)>,
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::new();
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |end| &rest[start + 2 + end + 2..]);
        out.push(' ');
    }
    out.push_str(rest);
    out
}

/// Flat rule list, one level of at-rule nesting.
fn parse_css(css: &str) -> Vec<Rule> {
    let css = strip_css_comments(css);
    let mut rules = Vec::new();
    let mut at: Option<String> = None;
    let mut rest = css.as_str();
    loop {
        let open = rest.find('{');
        let close = rest.find('}');
        if let Some(c) = close.filter(|c| open.is_none_or(|o| *c < o)) {
            assert!(
                at.is_some() && rest[..c].trim().is_empty(),
                "unbalanced CSS near `{}`",
                rest[..c].trim()
            );
            at = None;
            rest = &rest[c + 1..];
            continue;
        }
        let Some(o) = open else {
            assert!(rest.trim().is_empty() && at.is_none(), "trailing CSS");
            return rules;
        };
        let prelude = squash(&rest[..o]);
        if prelude.starts_with('@') {
            assert!(at.is_none(), "nested at-rule `{prelude}`");
            at = Some(prelude);
            rest = &rest[o + 1..];
            continue;
        }
        let end = rest[o + 1..].find('}').expect("rule end");
        let decls = rest[o + 1..o + 1 + end]
            .split(';')
            .filter_map(|d| {
                let (p, v) = d.split_once(':')?;
                let p = p.trim();
                (!p.is_empty()).then(|| (p.to_string(), squash(v)))
            })
            .collect();
        rules.push(Rule {
            at: at.clone(),
            selectors: prelude.split(',').map(squash).collect(),
            decls,
        });
        rest = &rest[o + 1 + end + 1..];
    }
}

/// Whether `value` uses one of `units` as a numeric unit (`100cqw`).
fn uses_unit(value: &str, units: &[&str]) -> bool {
    value
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '%'))
        .any(|word| {
            let digits = word
                .bytes()
                .take_while(|b| b.is_ascii_digit() || *b == b'.')
                .count();
            digits > 0 && units.contains(&&word[digits..])
        })
}

fn in_block<'a>(rules: &'a [Rule], block: &str) -> Vec<&'a Rule> {
    rules
        .iter()
        .filter(|r| r.at.as_deref() == Some(block))
        .collect()
}

/// Declarations of every rule (filtered by `keep`) listing `selector`, in order.
fn decls_of(rules: &[&Rule], selector: &str) -> Vec<(String, String)> {
    rules
        .iter()
        .filter(|r| r.selectors.iter().any(|s| s == selector))
        .flat_map(|r| r.decls.clone())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Helper self-test
// ---------------------------------------------------------------------------------------------

#[test]
fn test_rxb_helper_self_test() {
    let src = "const A = \"// kept\"; // gone\nconst R = /^[0-9]{1,7}$/;\nfunction f(x) {\n  if (x) { return \"}\"; }\n  return /}/.test(x);\n}\nasync function g() { await h(); }\ndocument.addEventListener(\"x\", function () { k(); });\n";
    let code = strip_js(src);
    assert!(
        code.contains("\"// kept\"") && !code.contains("gone"),
        "{code}"
    );
    assert!(code.contains("/^[0-9]{1,7}$/"), "{code}");
    let (functions, statements) = split_top_level(&code);
    let names: Vec<&str> = functions.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["f", "g"]);
    assert!(function_text(&functions, "f").contains("return /}/.test(x);"));
    assert!(statements.contains("addEventListener") && statements.contains("k();"));
    assert!(!statements.contains("await h()"));
    assert_eq!(js_const_string("const P = \"a  b\";", "P"), "a b");
    assert_eq!(
        used_before_declared("\nconst A = B;\nconst B = 1;\nconst C = { x: A.B };\n"),
        ["B"]
    );
    assert_eq!(
        used_before_declared("\nconst P = \"/api/later\";\nconst later = 1;\n"),
        Vec::<String>::new()
    );
    assert_eq!(
        call_arguments("setText(a, f(b, c)); function setText(n, t) {}", "setText"),
        [vec!["a".to_string(), "f(b, c)".to_string()]]
    );
    let rules = parse_css(
        "/* c */ a { width: 100%; width: 100cqw; }\n@supports not selector(:has(*)) { b[x=\"y\"] .c, d { margin: 0; } }",
    );
    assert_eq!(rules.len(), 2);
    assert_eq!(
        rules[1].at.as_deref(),
        Some("@supports not selector(:has(*))")
    );
    assert_eq!(rules[1].selectors, ["b[x=\"y\"] .c", "d"]);
    assert!(uses_unit("100cqw", &["cqw"]) && !uses_unit("var(--cqw)", &["cqw"]));
    assert!(uses_unit("calc(100dvh - 2px)", &["dvh"]) && !uses_unit("100vh", &["dvh"]));
}

// ---------------------------------------------------------------------------------------------
// RXB-S1
// ---------------------------------------------------------------------------------------------

/// RXB-S1 (RXB1, spec D-1, §3.1): one pure probe from feature detection only.
#[test]
fn test_rxb_s1_single_pure_capability_probe() {
    let code = app();
    assert_eq!(
        code.matches("function browserCapabilities(").count(),
        1,
        "exactly one browserCapabilities()"
    );
    let probe = body(&code, "function browserCapabilities(");
    for key in PROBE_KEYS {
        assert!(
            probe.contains(&format!("{key}:")),
            "the probe returns `{key}`"
        );
    }
    for forbidden in [
        "fetch(",
        "setText(",
        "textContent",
        ".register(",
        "addEventListener",
        "postJson(",
        "new ",
        "localStorage",
        "sessionStorage",
        "userAgent",
        "console",
        "requestPermission",
    ] {
        assert!(
            !probe.contains(forbidden),
            "the probe is pure: no `{forbidden}`"
        );
    }
    for token in ["PublicKeyCredential", "PushManager"] {
        let inside = probe.matches(token).count();
        assert!(inside >= 1, "the probe detects {token}");
        assert_eq!(
            code.matches(token).count(),
            inside,
            "{token} is only read by the probe"
        );
    }
    let push = body(&code, "function pushSupported(");
    assert_eq!(push.matches("browserCapabilities()").count(), 1, "{push}");
    for flag in [".serviceWorker", ".push", ".notifications"] {
        assert!(push.contains(flag), "pushSupported uses {flag}: {push}");
    }
    assert!(!push.contains(" in "), "pushSupported does not re-detect");
}

// ---------------------------------------------------------------------------------------------
// RXB-S2
// ---------------------------------------------------------------------------------------------

/// RXB-S2 (RXB2, spec D-2): every sensitive browser API sits behind its capability guard.
#[test]
fn test_rxb_s2_browser_apis_are_called_behind_the_probe() {
    let code = app();
    let (functions, statements) = split_top_level(&code);
    assert!(functions.len() > 40, "functions found: {}", functions.len());
    let mut guarded = 0usize;
    for (name, text) in &functions {
        if name == "browserCapabilities" {
            continue;
        }
        for (token, guard) in GUARDS {
            let Some(first) = text.find(token) else {
                continue;
            };
            let at = text.find(guard);
            assert!(
                at.is_some_and(|g| g < first && guard_exits(text, g)),
                "{name}: `if ({guard}…) {{ return|throw|Promise.reject }}` must precede the first `{token}`"
            );
            guarded += 1;
        }
    }
    assert!(guarded >= 10, "guarded uses found: {guarded}");
    for (token, _) in GUARDS {
        assert!(
            !statements.contains(token),
            "`{token}` is never used outside a guarded function"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// RXB-S3
// ---------------------------------------------------------------------------------------------

/// RXB-S3 (RXB3, spec D-4, §3.3): passkey controls disabled with a fixed text without WebAuthn;
/// lock and status never depend on passkeys.
#[test]
fn test_rxb_s3_passkey_controls_degrade_without_webauthn() {
    let code = app();
    let (functions, _) = split_top_level(&code);
    let (name, value) = PASSKEY_UNSUPPORTED;
    assert_eq!(js_const_string(&code, name), value);
    assert_eq!(code.matches(&format!("const {name} =")).count(), 1);

    let mut assignments = 0usize;
    for target in [
        "loginButton.disabled =",
        "enrollButton.disabled =",
        "unlockButton.disabled =",
        "startButton.disabled =",
    ] {
        for (at, _) in code.match_indices(target) {
            let rest = &code[at + target.len()..];
            let rhs = rest[..rest.find(';').expect("statement end")].trim();
            assert!(
                rhs == "true" || rhs.contains("browserCapabilities().webauthn"),
                "`{target} {rhs}` must be true or depend on browserCapabilities().webauthn"
            );
            assignments += 1;
        }
    }
    assert!(assignments >= 9, "assignments found: {assignments}");

    for handler in [
        "requestLogin",
        "requestUnlock",
        "requestEnroll",
        "startCameraView",
    ] {
        let text = function_text(&functions, handler);
        let guard = text
            .find("!browserCapabilities().webauthn")
            .unwrap_or_else(|| panic!("{handler} checks webauthn first"));
        for request in ["window.confirm(", "postJson(", "fetch("] {
            if let Some(at) = text.find(request) {
                assert!(guard < at, "{handler}: the guard precedes `{request}`");
            }
        }
        let block = block_from(text, guard);
        assert!(
            block.contains(name) && block.contains("return"),
            "{handler}: the guard shows {name} and returns: {block}"
        );
    }
    for user in [
        "showLogin",
        "showApp",
        "renderCamera",
        "getAssertion",
        "requestEnroll",
    ] {
        assert!(
            function_text(&functions, user).contains(name),
            "{user} uses {name}"
        );
    }
    assert!(
        function_text(&functions, "getAssertion").contains(&format!("fixedError({name})")),
        "getAssertion rejects with the fixed text"
    );
    assert!(
        no_whitespace(function_text(&functions, "render")).contains(
            "unlockButton.disabled=!(reachable&&view.state===\"locked\"&&browserCapabilities().webauthn);"
        ),
        "render keeps the unlock button off without WebAuthn"
    );
    for independent in ["requestLock", "fetchStatus", "openStream", "closeStream"] {
        assert!(
            !function_text(&functions, independent).contains("webauthn"),
            "{independent} never depends on passkeys"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// RXB-S4
// ---------------------------------------------------------------------------------------------

/// RXB-S4 (RXB4, spec D-5, §3.4): distinct fixed push texts and a bounded enable tap.
#[test]
fn test_rxb_s4_push_failures_map_to_distinct_fixed_texts() {
    let code = app();
    let (functions, _) = split_top_level(&code);
    let mut values = Vec::new();
    for (name, value) in NEW_PUSH_TEXTS {
        assert_eq!(js_const_string(&code, name), value, "{name}");
        assert_eq!(
            code.matches(&format!("const {name} =")).count(),
            1,
            "{name} defined once"
        );
        values.push(value.to_string());
    }
    for name in PINNED_PUSH_TEXTS {
        values.push(js_const_string(&code, name));
    }
    let mut unique = values.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), values.len(), "every push text is distinct");

    assert_eq!(
        squash(&body(&code, "const PUSH_ERROR_TEXTS =")),
        PUSH_ERROR_TEXTS_BODY,
        "PUSH_ERROR_TEXTS of spec §3.2"
    );
    assert!(
        squash(&body(&code, "const PUSH_REASONS =")).contains(UNSUPPORTED_PUSH_SERVICE_ENTRY),
        "the unsupported_push_service text names Edge on a computer"
    );

    let unsupported = function_text(&functions, "pushUnsupportedText");
    for needle in [
        "browserCapabilities()",
        "homeScreen",
        "standalone",
        "serviceWorker",
        "return fallback",
    ] {
        assert!(
            unsupported.contains(needle),
            "pushUnsupportedText: {needle}"
        );
    }
    let home = unsupported
        .find("PUSH_HOME_SCREEN_TEXT")
        .expect("home text");
    let worker = unsupported
        .find("PUSH_NO_WORKER_TEXT")
        .expect("worker text");
    assert!(home < worker, "the home-screen case is decided first");
    for user in ["renderPush", "enableNotifications"] {
        assert!(
            function_text(&functions, user).contains("pushUnsupportedText(PUSH_UNSUPPORTED_TEXT)"),
            "{user} words the unsupported case"
        );
    }

    let enable = function_text(&functions, "enableNotifications");
    let flat = no_whitespace(enable);
    let permission = enable
        .find("Notification.requestPermission(")
        .expect("permission");
    let granted = enable.find("\"granted\"").expect("granted check");
    let disabled = enable
        .find("pushEnableButton.disabled = true")
        .expect("the button is disabled");
    let deadline = enable
        .find("const deadline = Date.now() + SERVICE_WORKER_READY_TIMEOUT_MS;")
        .expect("the deadline is fixed from SERVICE_WORKER_READY_TIMEOUT_MS");
    let ready = enable.find("await readyRegistration()").expect("ready");
    assert!(
        permission < granted && granted < disabled && disabled < deadline && deadline < ready,
        "the button is disabled and the deadline set only after a granted permission, before the worker wait"
    );
    assert_eq!(
        enable.matches("pushEnableButton.disabled = true").count(),
        1,
        "never disabled during the permission prompt"
    );
    assert!(
        flat.contains("awaitbeforeDeadline(registration.pushManager.subscribe({"),
        "subscribe is bounded"
    );
    assert!(
        flat.contains("}),deadline,PUSH_SLOW_TEXT)"),
        "a subscribe past the deadline shows PUSH_SLOW_TEXT"
    );
    assert!(
        flat.contains(
            "awaitbeforeDeadline(postJson(PUSH_SUBSCRIBE_PATH,\"push-subscribe\",subscription.toJSON()).catch(function(){throwfixedError(PUSH_UNREACHABLE_TEXT);}),deadline,PUSH_TIMEOUT_TEXT)"
        ),
        "the hand-off to the PC is bounded and a network failure blames the PC link"
    );
    assert_eq!(
        enable.matches("PUSH_NO_SERVICE_TEXT").count(),
        0,
        "PUSH_NO_SERVICE_TEXT only through PUSH_ERROR_TEXTS"
    );
    let refused = enable
        .find("\"unsupported_push_service\"")
        .expect("the server refusal is handled");
    assert!(
        block_from(enable, refused).contains("subscription.unsubscribe()"),
        "a refused subscription is dropped locally"
    );
    assert!(
        enable.contains("failureText(error, PUSH_ERROR_TEXTS, \"Notifications not enabled\")"),
        "push failures are mapped to fixed texts"
    );
    assert!(!enable.contains(".message"), "no browser message shown");

    let bounded = function_text(&functions, "beforeDeadline");
    for needle in [
        "setTimeout(",
        "fixedError(text)",
        "Math.max(0, deadline - Date.now())",
    ] {
        assert!(bounded.contains(needle), "beforeDeadline: {needle}");
    }
    assert!(
        bounded.matches("clearTimeout(timer)").count() >= 2,
        "the timer is cleared on both outcomes"
    );
    for guarded in ["readyRegistration", "disableNotifications"] {
        assert!(
            function_text(&functions, guarded).contains("!pushSupported()"),
            "{guarded} is guarded"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// RXB-S5
// ---------------------------------------------------------------------------------------------

/// RXB-S5 (RXB5, spec D-6, §3.2): the page shows fixed strings only and logs nothing.
#[test]
fn test_rxb_s5_user_messages_are_fixed_strings() {
    let code = app();
    let (functions, _) = split_top_level(&code);
    let failure = function_text(&functions, "failureText");
    for needle in [
        "error.fixed === true",
        "hasOwnProperty.call(names, error.name)",
        "return fallback",
    ] {
        assert!(failure.contains(needle), "failureText: {needle}");
    }
    let inside = failure.matches(".message").count();
    assert!(inside >= 1, "failureText reads the fixed message");
    assert_eq!(
        code.matches(".message").count(),
        inside,
        "`.message` is read by failureText only"
    );
    let fixed = function_text(&functions, "fixedError");
    assert!(
        fixed.contains("new Error(text)") && fixed.contains(".fixed = true"),
        "fixedError marks its text: {fixed}"
    );
    assert_eq!(
        squash(&body(&code, "const PASSKEY_ERROR_TEXTS =")),
        PASSKEY_ERROR_TEXTS_BODY,
        "PASSKEY_ERROR_TEXTS of spec §3.2, §14 R-6"
    );
    assert!(
        squash(&code).contains(ENROLL_ERROR_TEXTS_LINE),
        "ENROLL_ERROR_TEXTS of spec §14 R-6"
    );
    let (_, statements) = split_top_level(&code);
    assert_eq!(
        used_before_declared(&statements),
        Vec::<String>::new(),
        "top-level constants are declared before their first top-level use (spec §14 R-8)"
    );
    let enroll = code
        .find("const ENROLL_ERROR_TEXTS =")
        .expect("ENROLL_ERROR_TEXTS");
    for earlier in ["const PASSKEY_REASONS =", "const PASSKEY_ERROR_TEXTS ="] {
        assert!(
            code.find(earlier).is_some_and(|at| at < enroll),
            "{earlier} precedes ENROLL_ERROR_TEXTS"
        );
    }
    let calls = call_arguments(&code, "setText");
    assert!(calls.len() > 40, "setText calls found: {}", calls.len());
    for args in &calls {
        assert_eq!(args.len(), 2, "setText(node, text): {args:?}");
        for arg in args {
            assert!(
                !arg.contains(".message") && arg != "error" && arg != "err",
                "setText never shows a browser error: {args:?}"
            );
        }
    }
    for (function, call) in [
        (
            "requestLogin",
            "failureText(error, PASSKEY_ERROR_TEXTS, \"Sign-in cancelled\")",
        ),
        (
            "requestEnroll",
            "failureText(error, ENROLL_ERROR_TEXTS, \"Enrollment cancelled\")",
        ),
        (
            "requestUnlock",
            "failureText(error, PASSKEY_ERROR_TEXTS, \"Unlock cancelled or failed\")",
        ),
        (
            "startCameraView",
            "failureText(error, PASSKEY_ERROR_TEXTS, \"Camera view cancelled\")",
        ),
        (
            "openCameraStream",
            "failureText(error, {}, \"The camera view ended\")",
        ),
    ] {
        assert!(
            function_text(&functions, function).contains(call),
            "{function} shows {call}"
        );
    }
    for (at, _) in code.match_indices("throw new Error(") {
        assert!(
            code[at + "throw new Error(".len()..].starts_with("\"status "),
            "a displayable error is a fixedError: {}",
            &code[at..(at + 60).min(code.len())]
        );
    }
    assert!(
        code.matches("throw fixedError(").count() >= 8,
        "the displayable errors are fixed"
    );
    assert!(!code.contains('`'), "no template literal in app.js");
    for rel in [APP_JS, SW_JS] {
        assert!(!read(rel).contains("console."), "{rel} logs nothing");
    }
}

// ---------------------------------------------------------------------------------------------
// RXB-S6
// ---------------------------------------------------------------------------------------------

/// RXB-S6 (RXB6, spec D-3): no User-Agent logic in the page, the worker or the server.
#[test]
fn test_rxb_s6_no_user_agent_logic() {
    for rel in [APP_JS, SW_JS] {
        let text = read(rel);
        for forbidden in [
            "userAgent",
            "userAgentData",
            "navigator.vendor",
            "navigator.platform",
            "appVersion",
        ] {
            assert!(!text.contains(forbidden), "{rel} must not read {forbidden}");
        }
    }
    let sources = rust_files("crates/remote/src");
    assert!(!sources.is_empty(), "remote sources found");
    for (rel, content) in sources {
        let code = strip_comments(production_part(&content)).to_ascii_lowercase();
        for forbidden in ["user-agent", "user_agent", "useragent"] {
            assert!(
                !code.contains(forbidden),
                "{rel} must not read the User-Agent ({forbidden})"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// RXB-S7
// ---------------------------------------------------------------------------------------------

/// RXB-S7 (RXB7, spec D-9, §3.7, §4): CSS fallbacks for `:has()`, container units and `dvh`.
#[test]
fn test_rxb_s7_css_fallbacks() {
    let rules = parse_css(&read(STYLE));
    let has_block = in_block(&rules, HAS_BLOCK);
    let cq_block = in_block(&rules, CQ_BLOCK);
    assert!(!has_block.is_empty(), "a `{HAS_BLOCK}` block exists");
    assert!(!cq_block.is_empty(), "a `{CQ_BLOCK}` block exists");
    let outside: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.at.as_deref() != Some(HAS_BLOCK) && r.at.as_deref() != Some(CQ_BLOCK))
        .collect();

    for rule in &outside {
        if rule.selectors.iter().any(|s| s.contains(":has(")) {
            assert!(
                rule.selectors.iter().all(|s| s.contains(":has(")),
                "a :has() selector never shares a rule with another selector: {:?}",
                rule.selectors
            );
        }
    }
    assert!(
        decls_of(&outside, ".card > .feedback:last-child")
            .contains(&("margin-bottom".to_string(), "-16px".to_string())),
        "the last-child feedback margin survives without :has()"
    );
    let mut seen = 0usize;
    for rule in &outside {
        for selector in rule.selectors.iter().filter(|s| s.contains(":has(")) {
            let (_, fallback) = HAS_FALLBACKS
                .iter()
                .find(|(s, _)| s == selector)
                .unwrap_or_else(|| panic!("`{selector}` has no declared fallback"));
            let original = decls_of(&outside, selector);
            let replacement = decls_of(&has_block, fallback);
            assert_eq!(
                original, replacement,
                "`{fallback}` repeats the declarations of `{selector}`"
            );
            seen += 1;
        }
    }
    assert_eq!(
        seen,
        HAS_FALLBACKS.len(),
        "every :has() rule has a fallback"
    );
    for rule in &has_block {
        for selector in &rule.selectors {
            assert!(!selector.contains(":has("), "no :has() in the fallback");
        }
    }

    for rule in &outside {
        for (index, (prop, value)) in rule.decls.iter().enumerate() {
            if uses_unit(value, &["cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax"]) {
                let earlier = rule.decls[..index].iter().any(|(p, v)| {
                    p == prop && !uses_unit(v, &["cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax"])
                });
                let fallback = rule.selectors.iter().any(|s| {
                    decls_of(&cq_block, s)
                        .iter()
                        .any(|(p, v)| p == prop && !uses_unit(v, &["cqw", "cqh", "cqi", "cqb"]))
                });
                assert!(
                    earlier || fallback,
                    "`{} {{ {prop}: {value} }}` needs a fallback without container units",
                    rule.selectors.join(", ")
                );
            }
            if uses_unit(value, &["dvh", "svh", "lvh"]) {
                assert!(
                    rule.decls[..index]
                        .iter()
                        .any(|(p, v)| p == prop && uses_unit(v, &["vh"])),
                    "`{prop}: {value}` must follow a vh value in the same rule"
                );
            }
        }
    }
    assert_eq!(
        decls_of(
            &cq_block,
            ".camera-stage.camera-full.camera-portrait .camera-canvas"
        ),
        [
            ("width".to_string(), "100vh".to_string()),
            ("height".to_string(), "100vw".to_string())
        ],
        "the full-screen portrait canvas uses the viewport"
    );
    assert_eq!(
        decls_of(&cq_block, ".camera-stage.camera-portrait .camera-canvas"),
        [
            ("width".to_string(), "133.333%".to_string()),
            ("height".to_string(), "75%".to_string())
        ],
        "the card portrait canvas is the 3:4 stage turned"
    );

    let code = app();
    let (functions, _) = split_top_level(&code);
    assert!(
        code.contains("const statTile = statusCard.querySelector(\".stat-tile\");"),
        "the stat tile hook"
    );
    assert!(
        function_text(&functions, "render").contains("statTile.dataset.state ="),
        "render sets the tile state"
    );
    let logout = function_text(&functions, "setLogoutHidden");
    assert!(
        logout.contains("logoutButton.hidden = hidden;")
            && logout.contains("feedbackNode.classList.toggle(\"feedback-last\", hidden);"),
        "setLogoutHidden keeps the feedback hook: {logout}"
    );
    assert_eq!(
        code.matches("logoutButton.hidden =").count(),
        1,
        "only setLogoutHidden hides the Sign out button"
    );
    assert!(function_text(&functions, "showLogin").contains("setLogoutHidden(true)"));
    assert!(
        function_text(&functions, "showApp").contains("setLogoutHidden(state.mode !== \"funnel\")")
    );
}

// ---------------------------------------------------------------------------------------------
// RXB-S8
// ---------------------------------------------------------------------------------------------

/// RXB-S8 (RXB8, spec D-10): no new network surface; CSP and the asset set unchanged.
#[test]
fn test_rxb_s8_no_new_network_surface() {
    let http = read(HTTP_RS);
    let flat_http = http
        .replace("\\\n", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        flat_http.contains(PINNED_CSP),
        "the CSP of http.rs is unchanged"
    );
    assert_eq!(http.matches("Content-Security-Policy:").count(), 1);
    let assets = fs::read_dir(workspace_root().join(ASSETS_DIR))
        .expect("assets dir")
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .count();
    assert_eq!(assets, 11, "the asset set keeps eleven files");
    for rel in [APP_JS, SW_JS, STYLE, MANIFEST] {
        let text = read(rel).to_ascii_lowercase();
        for forbidden in [
            "http://",
            "https://",
            "importscripts",
            "blob:",
            "unsafe-inline",
        ] {
            assert!(
                !text.contains(forbidden),
                "{rel} must not contain {forbidden}"
            );
        }
    }
    let css = read(STYLE).to_ascii_lowercase();
    for forbidden in ["url(", "@import", "@font-face"] {
        assert!(
            !css.contains(forbidden),
            "style.css loads nothing ({forbidden})"
        );
    }
    let index = read(INDEX);
    assert_eq!(index.matches("https://").count(), 1, "only the source link");
    assert!(index.contains("href=\"https://github.com/Mysticaly622/soos\""));
    assert!(!index.contains("http://"));
}

// ---------------------------------------------------------------------------------------------
// RXB-S9
// ---------------------------------------------------------------------------------------------

/// RXB-S9 (RXB9, spec D-7, §3.5): status keeps working without `EventSource`.
#[test]
fn test_rxb_s9_status_without_event_source() {
    let code = app();
    let (functions, _) = split_top_level(&code);
    assert!(code.contains("const STATUS_POLL_MS = 15000;"));
    assert!(code.contains("let statusPoll = null;"));
    let open = function_text(&functions, "openStream");
    let guard = open
        .find("!browserCapabilities().eventSource")
        .expect("openStream checks EventSource");
    let stream = open.find("new EventSource(").expect("the stream");
    assert!(guard < stream, "the check precedes the stream");
    let block = block_from(open, guard);
    for needle in [
        "statusPoll = setInterval(",
        "fetchStatus(true)",
        "STATUS_POLL_MS",
        "return",
    ] {
        assert!(block.contains(needle), "the polling fallback: {needle}");
    }
    let close = function_text(&functions, "closeStream");
    assert!(
        close.contains("clearInterval(statusPoll)") && close.contains("statusPoll = null"),
        "closeStream stops the poll: {close}"
    );
}

// ---------------------------------------------------------------------------------------------
// RXB-S10
// ---------------------------------------------------------------------------------------------

/// RXB-S10 (RXB10, spec D-8, §3.6): the camera view degrades without streams or decoding.
#[test]
fn test_rxb_s10_camera_degrades_without_streams() {
    let code = app();
    let (functions, _) = split_top_level(&code);
    let (name, value) = CAMERA_UNSUPPORTED;
    assert_eq!(js_const_string(&code, name), value);
    assert_eq!(code.matches(&format!("const {name} =")).count(), 1);
    let render = function_text(&functions, "renderCamera");
    let text = render.find(name).expect("renderCamera shows the text");
    let streaming = render.find("const streaming").expect("streaming state");
    let reachable = render
        .find("view.reachable === false")
        .expect("reachability");
    assert!(
        reachable < text && text < streaming,
        "decided before the buttons"
    );
    let flat = no_whitespace(render);
    assert!(
        flat.contains("if(!(caps.streams&&caps.imageBitmap)){"),
        "both features are required"
    );
    let block = block_from(render, render.find("!(caps.streams").unwrap());
    assert!(
        block.contains("startButton.hidden = true") && block.contains("return"),
        "the start button is hidden: {block}"
    );
    assert!(
        function_text(&functions, "readCameraStream").contains(&format!("fixedError({name})")),
        "the reader refuses without streams"
    );
    for bound in [
        "const CAMERA_MAX_PART_BYTES = 524288;",
        "const CAMERA_MAX_BUFFER_BYTES = 1048576;",
        "const CAMERA_STREAM_PATTERN = /^\\/api\\/camera\\/stream\\/[A-Za-z0-9_-]{43}$/;",
    ] {
        assert!(code.contains(bound), "parser bound unchanged: {bound}");
    }
}

// ---------------------------------------------------------------------------------------------
// RXB-S11
// ---------------------------------------------------------------------------------------------

/// RXB-S11 (RXB11, spec §10, §11): browser support documented; ADR, matrix and walkthrough.
#[test]
fn test_rxb_s11_docs_adr_and_walkthrough() {
    let doc = read(REMOTE_DOC);
    let h = doc.find("\n## 2h. ").expect("## 2h.");
    let i = doc
        .find("\n## 2i. Browser support")
        .expect("## 2i. Browser support");
    let three = doc.find("\n## 3. ").expect("## 3.");
    assert!(h < i && i < three, "## 2i. sits between ## 2h. and ## 3.");
    let section = &doc[i..three];
    let lower = section.to_ascii_lowercase();
    for needle in [
        "Safari",
        "Chrome",
        "Firefox",
        "Samsung Internet",
        "Edge",
        "Brave",
        "Use Google services for push messaging",
        "Add to Home Screen",
        "16.4",
        "Baseline",
        "User-Agent",
        "notify.windows.com",
    ] {
        assert!(section.contains(needle), "§2i must mention {needle}");
    }
    for needle in [
        "mandatory",
        "best effort",
        "private window",
        "feature detection",
        "no polyfill",
    ] {
        assert!(lower.contains(needle), "§2i must mention {needle}");
    }
    for row in 12..=20 {
        assert!(section.contains(&format!("RXB{row}")), "§2i cites RXB{row}");
    }
    let seven = doc.find("\n## 7. Troubleshooting").expect("## 7.");
    let eight = doc.find("\n## 8. ").expect("## 8.");
    let troubleshooting = &doc[seven..eight];
    for needle in ["DNS-over-HTTPS", "MagicDNS", "push service"] {
        assert!(troubleshooting.contains(needle), "§7 mentions {needle}");
    }
    let nine = &doc[doc.find("\n## 9. ").expect("## 9.")..];
    assert!(nine.contains("section 2i"), "§9 points to section 2i");
    assert!(read(DECISIONS).contains(ADR_TITLE), "the ADR is registered");
    let matrix = read(MATRIX);
    for row in 1..=20 {
        assert!(
            matrix.contains(&format!("| RXB{row} |")),
            "matrix row RXB{row}"
        );
    }
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH} exists");
    assert!(
        read(WALKTHROUGH).contains("#366"),
        "the walkthrough names #366"
    );
}
