//! Static contracts of the phone onboarding tour of the `soos-remote` web app (GitHub #414,
//! PR 2 of #363; owner decision of 2026-10-17 on #363: one `localStorage` key
//! `soos-remote-tour-v1`, a narrow exception to the web app's no-storage contract; ADR
//! 2026-10-17 "Phone Onboarding Tour in the `soos-remote` Web App and Its Single Storage
//! Exception"; spec `AI/architect_spec_phone_onboarding_tour.md`; matrix PTR1–PTR11):
//!
//! - PTR-S1 the tour storage block of `app.js` exists exactly once, at top level, and holds
//!   every `localStorage` use;
//! - PTR-S2 its only key is `soos-remote-tour-v1` and its only value the fixed word `seen`;
//! - PTR-S3 every access sits inside one `try`/`catch`, so a blocked storage reads as "not
//!   seen" and the tour stays dismissable without an error;
//! - PTR-S4 no other storage API anywhere in the page, and no request or DOM access in the block;
//! - PTR-S5 the tour text is fixed English, with the `soos-gui` tour's wording for the shared
//!   features and the exact off keys and their dependants;
//! - PTR-S6 the tour reads no server or user data, makes no request and switches nothing;
//! - PTR-S7 first run after sign-in, replay from the footer, dismissal and in-page links;
//! - PTR-S8 brand and motion tokens, reduced motion;
//! - PTR-S9 no inline style, script animation or markup change;
//! - PTR-S10 ADR, matrix rows, operator documentation and walkthrough.
//!
//! The project has no JavaScript runtime in CI: the checks are static. This module carries its
//! own small lexer (strings, template literals and regular expression literals kept intact,
//! self-tested) so the helpers of the other remote contracts stay untouched.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::collections::BTreeSet;

use crate::remote_companion_contract::{
    app_without_tour_storage, read, TOUR_STORAGE_BEGIN, TOUR_STORAGE_END,
};

const APP_JS: &str = "crates/remote/assets/app.js";
const SW_JS: &str = "crates/remote/assets/sw.js";
const INDEX: &str = "crates/remote/assets/index.html";
const STYLE: &str = "crates/remote/assets/style.css";
const GUI_ONBOARDING: &str = "crates/gui/src/onboarding.rs";
const REMOTE_DOC: &str = "Docs/REMOTE_COMPANION.md";
const DECISIONS: &str = "AI/DECISIONS.md";
const MATRIX: &str = "AI/VERIFICATION_MATRIX.md";
const WALKTHROUGH: &str = "AI/walkthroughs/222_phone_onboarding_tour.md";

const ADR_TITLE: &str =
    "Phone Onboarding Tour in the `soos-remote` Web App and Its Single Storage Exception";

/// The only key, and the only value ever written (owner decision 2026-10-17, #363).
const TOUR_KEY: &str = "soos-remote-tour-v1";
const TOUR_VALUE: &str = "seen";

/// The comment-free code of the storage block, compared whitespace-insensitively (spec §3).
const CANONICAL_BLOCK: &str = r#"
const TOUR_STORAGE_KEY = "soos-remote-tour-v1";
const TOUR_STORAGE_SEEN = "seen";
function tourStorage(markSeen) {
  try {
    if (markSeen === true) {
      window.localStorage.setItem(TOUR_STORAGE_KEY, TOUR_STORAGE_SEEN);
      return true;
    }
    return window.localStorage.getItem(TOUR_STORAGE_KEY) === TOUR_STORAGE_SEEN;
  } catch (_) {
    return false;
  }
}
"#;

/// The functions of the tour (spec §5); each exists exactly once.
const TOUR_FUNCTIONS: [&str; 11] = [
    "buildTourCard",
    "tourTarget",
    "shownOnPage",
    "renderTour",
    "openTour",
    "closeTour",
    "tourNext",
    "tourBack",
    "showTourTarget",
    "offerTour",
    "tourKeydown",
];

/// The steps in order: title, in-page target, substrings the "where" line must carry.
const STEPS: [(&str, &str, &[&str]); 8] = [
    (
        "Phone companion",
        "",
        &[
            "soos-gui",
            "systemctl --user disable --now soos-remote.service",
        ],
    ),
    (
        "Battery level",
        "status",
        &["battery_status = false", "remote.toml", RESTART],
    ),
    (
        "Remote lock and unlock",
        "status",
        &["soos-gui", "allow_unlock = false", "remote.toml", RESTART],
    ),
    (
        "Internet access",
        "logout",
        &[
            "Sign out",
            "allow_funnel = false",
            "camera_view_funnel = false",
            "[android]",
            "soos-gui",
            "remote.toml",
            RESTART,
        ],
    ),
    (
        "Failed-password history",
        "alerts",
        &[
            "password_alerts = false",
            "push_notifications = false",
            "offline_alerts = false",
            "remote.toml",
            RESTART,
        ],
    ),
    (
        "Push alerts",
        "push",
        &[
            "Disable notifications",
            "soos-gui",
            "push_notifications = false",
            "offline_alerts = false",
            "remote.toml",
            RESTART,
        ],
    ),
    (
        "Live camera view",
        "camera",
        &[
            "soos-gui",
            "camera_view = false",
            "camera_view_funnel = false",
            "remote.toml",
            RESTART,
        ],
    ),
    (
        "Offline alerts",
        "offline",
        &["soos-gui", "offline_alerts = false", "remote.toml", RESTART],
    ),
];

const RESTART: &str = "systemctl --user restart soos-remote.service";

/// The features shared with the `soos-gui` tour (their wording is the GUI's, GUI order).
const SHARED_TITLES: [&str; 6] = [
    "Phone companion",
    "Remote lock and unlock",
    "Internet access",
    "Push alerts",
    "Live camera view",
    "Offline alerts",
];

/// The fixed labels and texts of the tour outside `TOUR_STEPS`.
const TOUR_LABELS: [(&str, &str); 7] = [
    ("TOUR_NEXT_LABEL", "Next"),
    ("TOUR_DONE_LABEL", "Done"),
    ("TOUR_BACK_LABEL", "Back"),
    ("TOUR_SKIP_LABEL", "Skip tour"),
    ("TOUR_SHOW_LABEL", "Show it on this page"),
    ("TOUR_REPLAY_LABEL", "Feature tour"),
    ("TOUR_REGION_LABEL", "Feature tour"),
];
const TOUR_INTRO: [&str; 3] = ["TOUR_INTRO_TITLE", "TOUR_INTRO_TEXT", "TOUR_INTRO_NOTE"];

/// The only second arguments of `setText` inside the tour functions (spec §4).
const TOUR_SET_TEXT_ARGS: [&str; 15] = [
    "TOUR_INTRO_TITLE",
    "TOUR_INTRO_TEXT",
    "TOUR_INTRO_NOTE",
    "TOUR_SHOW_LABEL",
    "TOUR_BACK_LABEL",
    "TOUR_SKIP_LABEL",
    "TOUR_REPLAY_LABEL",
    "TOUR_NEXT_LABEL",
    "step.title",
    "step.text",
    "step.off",
    "step.where",
    "\" \"",
    "index === TOUR_STEPS.length ? TOUR_DONE_LABEL : TOUR_NEXT_LABEL",
    "\"Step \" + (index + 1) + \" of \" + total",
];

// ---------------------------------------------------------------------------------------------
// JavaScript lexer helpers
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lex {
    Code,
    LineComment,
    BlockComment,
    Str(u8),
    Template,
    Regex { class: bool },
}

/// Bytes that, as the previous significant code byte, make a `/` start a regular expression.
const REGEX_PREFIX: &[u8] = b"(,=:[!&|?{};+-*%<>~^";

/// True when a `/` at the end of `mask` starts a regular expression literal: after nothing,
/// after an operator or opening bracket, or after `return`, `typeof` or `case`.
fn starts_regex(mask: &[u8]) -> bool {
    let code: Vec<u8> = mask
        .iter()
        .rev()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .collect();
    let Some(&last) = code.first() else {
        return true;
    };
    if REGEX_PREFIX.contains(&last) {
        return true;
    }
    let mut word: Vec<u8> = code
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
        .copied()
        .collect();
    word.reverse();
    [b"return".as_slice(), b"typeof", b"case"].contains(&word.as_slice())
}

/// One pass over `src`: `mask` has the same length, with comments turned into spaces (line
/// breaks kept) and the content of string, template and regular expression literals turned
/// into `x` (delimiters kept); `stripped` is `src` without its comments.
fn lex(src: &str) -> (String, String) {
    let bytes = src.as_bytes();
    let mut mask = Vec::with_capacity(bytes.len());
    let mut stripped = Vec::with_capacity(bytes.len());
    let mut state = Lex::Code;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let next = bytes.get(i + 1).copied();
        match state {
            Lex::Code => {
                if b == b'/' && next == Some(b'/') {
                    state = Lex::LineComment;
                    mask.push(b' ');
                } else if b == b'/' && next == Some(b'*') {
                    state = Lex::BlockComment;
                    mask.extend_from_slice(b"  ");
                    i += 2;
                    continue;
                } else if b == b'"' || b == b'\'' {
                    state = Lex::Str(b);
                    mask.push(b);
                    stripped.push(b);
                } else if b == b'`' {
                    state = Lex::Template;
                    mask.push(b);
                    stripped.push(b);
                } else if b == b'/' && starts_regex(&mask) {
                    state = Lex::Regex { class: false };
                    mask.push(b);
                    stripped.push(b);
                } else {
                    mask.push(b);
                    stripped.push(b);
                }
            }
            Lex::LineComment => {
                if b == b'\n' {
                    state = Lex::Code;
                    mask.push(b'\n');
                    stripped.push(b'\n');
                } else {
                    mask.push(b' ');
                }
            }
            Lex::BlockComment => {
                if b == b'*' && next == Some(b'/') {
                    state = Lex::Code;
                    mask.extend_from_slice(b"  ");
                    stripped.push(b' ');
                    i += 2;
                    continue;
                }
                mask.push(if b == b'\n' { b'\n' } else { b' ' });
            }
            Lex::Str(_) | Lex::Template if b == b'\\' => {
                mask.extend_from_slice(b"xx");
                stripped.push(b);
                if let Some(n) = next {
                    stripped.push(n);
                }
                i += 2;
                continue;
            }
            Lex::Str(quote) => {
                stripped.push(b);
                if b == quote {
                    state = Lex::Code;
                    mask.push(b);
                } else {
                    mask.push(b'x');
                }
            }
            Lex::Template => {
                stripped.push(b);
                if b == b'`' {
                    state = Lex::Code;
                    mask.push(b);
                } else {
                    mask.push(b'x');
                }
            }
            Lex::Regex { class } => {
                stripped.push(b);
                if b == b'\\' {
                    mask.extend_from_slice(b"xx");
                    if let Some(n) = next {
                        stripped.push(n);
                    }
                    i += 2;
                    continue;
                }
                if !class && b == b'/' {
                    state = Lex::Code;
                    mask.push(b);
                } else {
                    state = Lex::Regex {
                        class: (class || b == b'[') && b != b']',
                    };
                    mask.push(b'x');
                }
            }
        }
        i += 1;
    }
    mask.truncate(bytes.len());
    (
        String::from_utf8(mask).expect("ASCII mask"),
        String::from_utf8(stripped).expect("UTF-8 stays UTF-8"),
    )
}

/// The code mask of `src` (see [`lex`]).
fn mask_of(src: &str) -> String {
    lex(src).0
}

/// `src` without its JavaScript comments.
fn strip_js_comments(src: &str) -> String {
    lex(src).1
}

/// All whitespace removed.
fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Index of the bracket that closes the one at `open` (same kind), on the mask.
fn matching(mask: &str, open: usize) -> usize {
    let bytes = mask.as_bytes();
    let (o, c) = match bytes[open] {
        b'{' => (b'{', b'}'),
        b'(' => (b'(', b')'),
        b'[' => (b'[', b']'),
        other => panic!("not an opening bracket: {}", char::from(other)),
    };
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if b == o {
            depth += 1;
        } else if b == c {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
    }
    panic!("unbalanced bracket at {open}");
}

/// Brace depth of the code at byte `at` (0 = top level).
fn brace_depth_at(mask: &str, at: usize) -> usize {
    let mut depth = 0isize;
    for &b in &mask.as_bytes()[..at] {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
        }
    }
    usize::try_from(depth).expect("non-negative depth")
}

/// The full text (`function name(...) { ... }`) of the only top-level function `name`.
fn function_text<'a>(src: &'a str, mask: &str, name: &str) -> &'a str {
    let head = format!("function {name}(");
    let found: Vec<usize> = mask.match_indices(&head).map(|(at, _)| at).collect();
    assert_eq!(found.len(), 1, "exactly one `{head}` in app.js");
    let start = found[0];
    let close_paren = matching(mask, start + head.len() - 1);
    let open = close_paren + mask[close_paren..].find('{').expect("function body");
    let end = matching(mask, open);
    &src[start..=end]
}

/// The body (between the outer braces) of the only top-level function `name`.
fn function_body<'a>(src: &'a str, mask: &str, name: &str) -> &'a str {
    let text = function_text(src, mask, name);
    let open = text.find('{').unwrap();
    &text[open + 1..text.len() - 1]
}

/// The arguments of every call `callee(...)` in `src`, split at top-level commas and trimmed.
fn call_arguments(src: &str, callee: &str) -> Vec<Vec<String>> {
    let mask = mask_of(src);
    let needle = format!("{callee}(");
    let mut calls = Vec::new();
    for (at, _) in mask.match_indices(&needle) {
        let before = mask[..at].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
            continue;
        }
        let open = at + needle.len() - 1;
        let close = matching(&mask, open);
        let mut args = Vec::new();
        let mut depth = 0usize;
        let mut from = open + 1;
        for (i, b) in mask.bytes().enumerate().take(close).skip(open + 1) {
            match b {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    args.push(src[from..i].trim().to_string());
                    from = i + 1;
                }
                _ => {}
            }
        }
        args.push(src[from..close].trim().to_string());
        calls.push(args);
    }
    calls
}

/// The double-quoted string literals of `src` (comments ignored), in order.
fn string_literals(src: &str) -> Vec<String> {
    let mask = mask_of(src);
    let bytes = mask.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            let end = i + 1 + bytes[i + 1..].iter().position(|&b| b == quote).unwrap();
            out.push(src[i + 1..end].to_string());
            i = end + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// The value of `const NAME = "literal";` (exactly one declaration).
fn js_const_string(src: &str, name: &str) -> String {
    let head = format!("const {name} =");
    assert_eq!(src.matches(&head).count(), 1, "exactly one `{head}`");
    let at = src.find(&head).unwrap() + head.len();
    let rest = src[at..].trim_start();
    assert!(rest.starts_with('"'), "{name} is a string literal");
    let end = rest[1..].find('"').unwrap();
    let value = &rest[1..=end];
    assert!(
        rest[end + 2..].trim_start().starts_with(';'),
        "{name} is one literal, no concatenation"
    );
    value.to_string()
}

/// One step of `TOUR_STEPS`: `(field, value)` pairs in source order.
type Step = Vec<(String, String)>;

/// Parses `const TOUR_STEPS = [ {field: "literal", ...}, ... ];`: object literals of plain
/// double-quoted literals only (no comment, identifier, escape or concatenation).
fn tour_steps(src: &str) -> Vec<Step> {
    let mask = mask_of(src);
    let head = "const TOUR_STEPS = [";
    assert_eq!(mask.matches(head).count(), 1, "exactly one `{head}`");
    let open = mask.find(head).unwrap() + head.len() - 1;
    let close = matching(&mask, open);
    assert!(
        src[close + 1..].starts_with(';'),
        "TOUR_STEPS ends with `];`"
    );
    let inner = &src[open + 1..close];
    assert_eq!(
        strip_js_comments(inner),
        inner,
        "no comment inside TOUR_STEPS"
    );
    let b = inner.as_bytes();
    let mut i = 0;
    let skip_ws = |i: &mut usize| {
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
    };
    let mut steps = Vec::new();
    loop {
        skip_ws(&mut i);
        if i == b.len() {
            break;
        }
        assert_eq!(b[i], b'{', "each step is an object literal");
        i += 1;
        let mut step = Vec::new();
        loop {
            skip_ws(&mut i);
            if b[i] == b'}' {
                i += 1;
                break;
            }
            let start = i;
            while b[i].is_ascii_alphabetic() {
                i += 1;
            }
            let field = inner[start..i].to_string();
            assert!(!field.is_empty(), "a field name at {start}");
            skip_ws(&mut i);
            assert_eq!(b[i], b':', "`{field}:`");
            i += 1;
            skip_ws(&mut i);
            assert_eq!(b[i], b'"', "`{field}` is a double-quoted literal");
            let end = i + 1 + inner[i + 1..].find('"').unwrap();
            let value = inner[i + 1..end].to_string();
            assert!(!value.contains('\\'), "no escape in `{field}`: {value}");
            i = end + 1;
            skip_ws(&mut i);
            if b[i] == b',' {
                i += 1;
            }
            step.push((field, value));
        }
        steps.push(step);
        skip_ws(&mut i);
        if i < b.len() && b[i] == b',' {
            i += 1;
        }
    }
    steps
}

fn field<'a>(step: &'a Step, name: &str) -> &'a str {
    step.iter()
        .find(|(f, _)| f == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("step field `{name}`: {step:?}"))
}

/// Plain English text the page may show: printable ASCII without markup, escape or emphasis.
fn plain_text(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| {
            (0x20..=0x7e).contains(&b)
                && !matches!(
                    b,
                    b'\\' | b'`' | b'!' | b'<' | b'>' | b'{' | b'}' | b'$' | b'&' | b'"'
                )
        })
        && text == text.trim()
        && !text.contains("  ")
}

// ---------------------------------------------------------------------------------------------
// Rust helpers (the `soos-gui` wording)
// ---------------------------------------------------------------------------------------------

/// Parses a Rust string literal starting at the `"` of `src[at..]`: `\` + line break skips the
/// following whitespace, `\"` and `\\` are unescaped; any other escape is refused.
fn rust_literal(src: &str, at: usize) -> String {
    let b = src.as_bytes();
    assert_eq!(b[at], b'"', "a Rust string literal");
    let mut out = String::new();
    let mut i = at + 1;
    loop {
        match b[i] {
            b'"' => return out,
            b'\\' => {
                match b[i + 1] {
                    b'\n' => {
                        i += 2;
                        while b[i].is_ascii_whitespace() {
                            i += 1;
                        }
                        continue;
                    }
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    other => panic!("unexpected escape \\{}", char::from(other)),
                }
                i += 2;
                continue;
            }
            _ => {
                let ch = src[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
}

/// `(variant, string)` of every `Self::Variant => "..."` arm of `pub fn <name>(self)` in
/// `onboarding.rs`, in source order.
fn gui_strings(name: &str) -> Vec<(String, String)> {
    let src = read(GUI_ONBOARDING);
    let head = format!("pub fn {name}(self) -> &'static str {{");
    let start = src
        .find(&head)
        .unwrap_or_else(|| panic!("{GUI_ONBOARDING}: `{head}`"));
    let body_start = start + head.len();
    let end = body_start + src[body_start..].find("\n    }\n").expect("end of fn");
    let body = &src[body_start..end];
    let mut out = Vec::new();
    for (at, _) in body.match_indices("Self::") {
        let rest = &body[at + "Self::".len()..];
        let variant: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let arrow = at + body[at..].find("=>").unwrap();
        let quote = arrow + body[arrow..].find('"').unwrap();
        out.push((variant, rust_literal(body, quote)));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// CSS helpers
// ---------------------------------------------------------------------------------------------

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        let end = rest[at..].find("*/").expect("closed CSS comment");
        rest = &rest[at + end + 2..];
    }
    out.push_str(rest);
    out
}

/// The declarations of every rule whose selector list is exactly `selector` (whitespace
/// normalized), at any nesting level.
fn css_rule_bodies(css: &str, selector: &str) -> Vec<String> {
    let css = strip_css_comments(css);
    let want = squash(selector);
    let bytes = css.as_bytes();
    let mut out = Vec::new();
    let mut seg_start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => {
                if squash(&css[seg_start..i]) == want {
                    let end = i + css[i..].find('}').unwrap();
                    out.push(css[i + 1..end].to_string());
                }
                seg_start = i + 1;
            }
            b'}' | b';' => seg_start = i + 1,
            _ => {}
        }
    }
    out
}

/// The value of `property` in a declaration block (last one wins), whitespace normalized.
fn css_value(body: &str, property: &str) -> Option<String> {
    body.split(';')
        .filter_map(|decl| decl.split_once(':'))
        .filter(|(p, _)| p.trim() == property)
        .map(|(_, v)| v.split_whitespace().collect::<Vec<_>>().join(" "))
        .next_back()
}

/// The inner text of the `@<name> ... {` block that starts with `prelude` (exactly one).
fn css_at_block(css: &str, prelude: &str) -> String {
    let css = strip_css_comments(css);
    let found: Vec<usize> = css.match_indices(prelude).map(|(at, _)| at).collect();
    assert_eq!(found.len(), 1, "exactly one `{prelude}` block in style.css");
    let open = found[0] + css[found[0]..].find('{').unwrap();
    let mut depth = 0usize;
    for (i, b) in css.bytes().enumerate().skip(open) {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return css[open + 1..i].to_string();
            }
        }
    }
    panic!("unclosed `{prelude}` block");
}

// ---------------------------------------------------------------------------------------------
// Page helpers
// ---------------------------------------------------------------------------------------------

/// The text strictly between the two markers.
fn tour_block(app: &str) -> &str {
    let begin = app
        .find(TOUR_STORAGE_BEGIN)
        .expect("tour storage begin marker");
    let end = app.find(TOUR_STORAGE_END).expect("tour storage end marker");
    assert!(begin < end, "the block ends after it begins");
    &app[begin + TOUR_STORAGE_BEGIN.len()..end]
}

fn app() -> String {
    read(APP_JS)
}

// ---------------------------------------------------------------------------------------------
// Helper self-test
// ---------------------------------------------------------------------------------------------

/// The lexer and parsers of this module behave on known inputs.
#[test]
fn test_ptr_helper_self_test() {
    let src = "a(\"x // y\"); // gone\nconst r = /}[/]{/; b = c / d; /* gone */ e(`t{`);\n";
    let mask = mask_of(src);
    assert_eq!(mask.len(), src.len());
    assert!(!mask.contains("gone") && !mask.contains("x // y"));
    assert_eq!(brace_depth_at(&mask, mask.len()), 0, "{mask}");
    let stripped = strip_js_comments(src);
    assert!(stripped.contains("\"x // y\"") && !stripped.contains("gone"));
    assert!(stripped.contains("/}[/]{/") && stripped.contains("c / d"));

    let code = "function f(a) {\n  if (a) { return \"}\"; }\n  setText(n, \"A, b\");\n}\nfunction g() {}\n";
    let mask = mask_of(code);
    assert_eq!(function_body(code, &mask, "g"), "");
    assert!(function_body(code, &mask, "f").contains("return \"}\";"));
    assert_eq!(
        call_arguments(code, "setText"),
        vec![vec!["n".to_string(), "\"A, b\"".to_string()]]
    );
    assert_eq!(
        string_literals(code),
        vec!["}".to_string(), "A, b".to_string()]
    );

    let steps =
        tour_steps("const TOUR_STEPS = [\n  { a: \"x\", b: \"y z\" },\n  { a: \"w\" },\n];\n");
    assert_eq!(steps.len(), 2);
    assert_eq!(field(&steps[0], "b"), "y z");
    for bad in [
        "const TOUR_STEPS = [{ a: NAME }];",
        "const TOUR_STEPS = [{ a: \"x\" + \"y\" }];",
        "const TOUR_STEPS = [{ a: \"x\\u0041\" }];",
        "const TOUR_STEPS = [\n// c\n{ a: \"x\" }];",
    ] {
        assert!(
            std::panic::catch_unwind(|| tour_steps(bad)).is_err(),
            "rejected: {bad}"
        );
    }

    assert_eq!(
        rust_literal("\"a \\\n     b \\\"c\\\"\"", 0),
        "a b \"c\"".to_string()
    );
    assert!(plain_text(
        "Off: set a = false in ~/.config/x.toml, then run: y --user z."
    ));
    for bad in [
        "Hi!",
        "a <b>",
        "x\\y",
        " lead",
        "two  spaces",
        "",
        "caf\u{e9}",
    ] {
        assert!(!plain_text(bad), "{bad:?}");
    }

    let css = "/* c */\n:root { --a: 1px; }\n.x,\n.y { color: red; transition: a 1s; }\n@media (m) { .x, .y { color: blue; } }\n";
    assert_eq!(css_rule_bodies(css, ".x, .y").len(), 2);
    assert_eq!(
        css_value(&css_rule_bodies(css, ".x, .y")[0], "transition").as_deref(),
        Some("a 1s")
    );
    assert!(css_at_block(css, "@media (m)").contains(".x, .y"));
}

// ---------------------------------------------------------------------------------------------
// PTR-S1
// ---------------------------------------------------------------------------------------------

/// PTR-S1 (PTR1): the storage block exists exactly once, on its own marker lines at top level,
/// and holds every `localStorage` use of the page.
#[test]
fn test_ptr_s1_storage_block_exists_exactly_once() {
    let app = app();
    assert_eq!(
        app.matches(TOUR_STORAGE_BEGIN).count(),
        1,
        "one begin marker"
    );
    assert_eq!(app.matches(TOUR_STORAGE_END).count(), 1, "one end marker");
    for marker in [TOUR_STORAGE_BEGIN, TOUR_STORAGE_END] {
        assert_eq!(
            app.lines().filter(|line| *line == marker).count(),
            1,
            "`{marker}` is a whole line at column 0"
        );
    }
    let mask = mask_of(&app);
    let begin = app.find(TOUR_STORAGE_BEGIN).unwrap();
    let end = app.find(TOUR_STORAGE_END).unwrap();
    assert_eq!(brace_depth_at(&mask, begin), 0, "the block is at top level");
    assert_eq!(
        brace_depth_at(&mask, end),
        0,
        "the block closes at top level"
    );
    let block = tour_block(&app);
    let inside = block.matches("localStorage").count();
    assert!(inside >= 1, "the block uses localStorage");
    assert_eq!(
        app.matches("localStorage").count(),
        inside,
        "every localStorage use is inside the block"
    );
    assert!(!app_without_tour_storage(&app).contains("localStorage"));
    for rel in [SW_JS, INDEX, STYLE] {
        assert!(
            !read(rel).contains("localStorage"),
            "{rel} must not contain localStorage"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// PTR-S2
// ---------------------------------------------------------------------------------------------

/// PTR-S2 (PTR2): the block is exactly the specified helper: one key `soos-remote-tour-v1`,
/// one fixed value `seen`, one read and one write, nothing else.
#[test]
fn test_ptr_s2_only_one_key_and_one_fixed_value() {
    let app = app();
    let block = tour_block(&app);
    let code = strip_js_comments(block);
    assert_eq!(
        squash(&code),
        squash(CANONICAL_BLOCK),
        "the comment-free storage block is exactly spec section 3"
    );
    assert_eq!(
        string_literals(&code),
        vec![TOUR_KEY.to_string(), TOUR_VALUE.to_string()],
        "the block holds exactly the key and the value literals"
    );
    assert_eq!(
        app.matches(&format!("\"{TOUR_KEY}\"")).count(),
        1,
        "the key literal appears once in app.js"
    );
    assert_eq!(
        app.matches(TOUR_KEY).count(),
        1,
        "the key is never rebuilt elsewhere"
    );
    let outside = app_without_tour_storage(&app);
    for name in ["TOUR_STORAGE_KEY", "TOUR_STORAGE_SEEN"] {
        assert!(
            !outside.contains(name),
            "{name} is used inside the block only"
        );
    }
    assert_eq!(code.matches(".setItem(").count(), 1, "one write");
    assert_eq!(code.matches(".getItem(").count(), 1, "one read");
    assert!(code.contains("window.localStorage.setItem(TOUR_STORAGE_KEY, TOUR_STORAGE_SEEN)"));
    assert!(code.contains("window.localStorage.getItem(TOUR_STORAGE_KEY) === TOUR_STORAGE_SEEN"));
    for forbidden in [
        "removeItem",
        ".clear(",
        ".key(",
        ".length",
        "JSON",
        "Date",
        "navigator",
        "location",
        "[",
    ] {
        assert!(
            !code.contains(forbidden),
            "the block must not contain `{forbidden}`"
        );
    }
    for rel in [SW_JS, INDEX, STYLE] {
        assert!(!read(rel).contains(TOUR_KEY), "{rel} never names the key");
    }
}

// ---------------------------------------------------------------------------------------------
// PTR-S3
// ---------------------------------------------------------------------------------------------

/// PTR-S3 (PTR3): every access is inside one `try`/`catch` that answers "not seen", the card
/// is hidden before the flag is written, and the tour opens on "not seen": a blocked storage
/// shows the tour as unseen and dismissable without an error.
#[test]
fn test_ptr_s3_every_access_is_guarded_and_blocked_storage_stays_dismissable() {
    let app = app();
    let block = tour_block(&app);
    let code = strip_js_comments(block);
    let mask = mask_of(&code);
    let body = function_body(&code, &mask, "tourStorage");
    let body_mask = mask_of(body);
    let trimmed = body.trim_start();
    assert!(trimmed.starts_with("try {"), "the helper starts with try");
    let try_open = body.find("try {").unwrap() + "try ".len();
    let try_close = matching(&body_mask, try_open);
    for (at, _) in body.match_indices("localStorage") {
        assert!(
            try_open < at && at < try_close,
            "every localStorage access is inside the try block"
        );
    }
    let after = body[try_close + 1..].trim();
    assert_eq!(
        squash(after),
        "catch(_){returnfalse;}",
        "the catch answers false (not seen) and nothing follows"
    );
    assert!(!code.contains("throw"), "the block never throws");
    assert_eq!(
        code.matches("function ").count(),
        1,
        "one helper function in the block"
    );

    let outside = app_without_tour_storage(&app);
    let omask = mask_of(&outside);
    assert_eq!(
        omask.matches("tourStorage(").count(),
        2,
        "tourStorage is called exactly twice"
    );
    let close = function_body(&outside, &omask, "closeTour");
    let hide = close
        .find(".section.hidden = true;")
        .expect("closeTour hides the card");
    let write = close
        .find("tourStorage(true);")
        .expect("closeTour remembers the tour");
    assert!(
        hide < write,
        "the card is hidden before the flag is written"
    );
    assert_eq!(
        squash(&close[write..]),
        "tourStorage(true);",
        "nothing depends on the write"
    );
    let offer = function_body(&outside, &omask, "offerTour");
    assert!(
        squash(offer).contains("if(!tourStorage()){openTour();}"),
        "the tour opens when the flag reads false: {offer}"
    );
    for name in [
        "openTour",
        "renderTour",
        "buildTourCard",
        "tourNext",
        "tourBack",
    ] {
        assert!(
            !function_body(&outside, &omask, name).contains("tourStorage"),
            "{name} never touches the storage"
        );
    }
    // Dismissal never depends on the storage: Skip, Done and Escape all reach closeTour.
    let next = function_body(&outside, &omask, "tourNext");
    assert!(
        squash(next).contains("if(tourIndex>=TOUR_STEPS.length){closeTour();return;}"),
        "Done closes the tour: {next}"
    );
    let build = function_body(&outside, &omask, "buildTourCard");
    assert!(
        squash(build).contains("skipButton.addEventListener(\"click\",closeTour);"),
        "Skip closes the tour"
    );
    let key = function_body(&outside, &omask, "tourKeydown");
    assert!(
        key.contains("\"Escape\"") && key.contains("closeTour();"),
        "Escape closes the tour"
    );
}

// ---------------------------------------------------------------------------------------------
// PTR-S4
// ---------------------------------------------------------------------------------------------

/// PTR-S4 (PTR4): no other storage API anywhere in the page, and the block makes no request
/// and touches no DOM.
#[test]
fn test_ptr_s4_no_other_storage_api() {
    let app = app();
    for rel in [APP_JS, SW_JS, INDEX] {
        let text = read(rel);
        for forbidden in [
            "sessionStorage",
            "indexedDB",
            "IDBFactory",
            "openDatabase",
            "document.cookie",
            "cookieStore",
            "CacheStorage",
            "navigator.storage",
            "StorageManager",
            "globalThis",
            "Reflect.",
        ] {
            assert!(
                !text.contains(forbidden),
                "{rel} must not contain {forbidden}"
            );
        }
    }
    for forbidden in ["caches", "window[", "self[", "eval(", "Function("] {
        assert!(
            !app.contains(forbidden),
            "app.js must not contain {forbidden}"
        );
    }
    let code = strip_js_comments(tour_block(&app));
    for forbidden in [
        "fetch(",
        "postJson(",
        "XMLHttpRequest",
        "sendBeacon",
        "EventSource",
        "innerHTML",
        "document.",
        "setText(",
        "setTimeout(",
        "setInterval(",
        "http",
        "console",
    ] {
        assert!(
            !code.contains(forbidden),
            "the storage block must not contain {forbidden}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// PTR-S5
// ---------------------------------------------------------------------------------------------

/// PTR-S5 (PTR5): fixed English text; the shared features keep the `soos-gui` wording in the
/// GUI order; every "where" line names the exact off path and the dependent keys.
#[test]
fn test_ptr_s5_tour_text_is_fixed_english_with_the_gui_wording() {
    let app = app();
    let steps = tour_steps(&app);
    assert_eq!(steps.len(), STEPS.len(), "eight feature steps");
    let titles: Vec<&str> = steps.iter().map(|s| field(s, "title")).collect();
    let expected: Vec<&str> = STEPS.iter().map(|s| s.0).collect();
    assert_eq!(titles, expected, "the steps, in order");
    let fields: BTreeSet<&str> = ["off", "target", "text", "title", "where"].into();
    for (step, (title, target, needles)) in steps.iter().zip(STEPS) {
        let names: Vec<&str> = step.iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(names.len(), 5, "{title}: five fields");
        assert_eq!(
            names.iter().copied().collect::<BTreeSet<_>>(),
            fields,
            "{title}: title, text, off, where, target"
        );
        assert_eq!(field(step, "target"), target, "{title}: target");
        for name in ["title", "text", "off", "where"] {
            let value = field(step, name);
            assert!(
                plain_text(value),
                "{title}.{name} is plain English: {value:?}"
            );
        }
        assert!(
            field(step, "text").len() >= 80,
            "{title}: a real explanation"
        );
        assert!(field(step, "off").starts_with("Off: "), "{title}: Off line");
        let place = field(step, "where");
        for needle in needles {
            assert!(
                place.contains(needle),
                "{title}: where names `{needle}`: {place}"
            );
        }
    }

    let gui_titles = gui_strings("title");
    let explanations = gui_strings("explanation");
    let offs = gui_strings("when_off");
    let shared: Vec<&str> = gui_titles
        .iter()
        .map(|(_, t)| t.as_str())
        .filter(|t| titles.contains(t))
        .collect();
    assert_eq!(
        shared, SHARED_TITLES,
        "the shared features, in the GUI order"
    );
    for (variant, title) in &gui_titles {
        let Some(step) = steps.iter().find(|s| field(s, "title") == title) else {
            continue;
        };
        let lookup = |table: &[(String, String)]| -> String {
            table
                .iter()
                .find(|(v, _)| v == variant)
                .map(|(_, s)| s.clone())
                .unwrap_or_else(|| panic!("{variant} in onboarding.rs"))
        };
        assert_eq!(
            field(step, "text"),
            lookup(&explanations),
            "{title}: GUI explanation"
        );
        assert_eq!(field(step, "off"), lookup(&offs), "{title}: GUI Off line");
    }
    let phone_only: Vec<&str> = titles
        .iter()
        .copied()
        .filter(|t| !SHARED_TITLES.contains(t))
        .collect();
    assert_eq!(phone_only, ["Battery level", "Failed-password history"]);
    let battery = field(&steps[1], "text");
    assert!(battery.contains("battery level"), "{battery}");
    let failed = field(&steps[4], "text");
    assert!(failed.contains("never the typed text"), "{failed}");

    for (name, value) in TOUR_LABELS {
        assert_eq!(js_const_string(&app, name), value, "{name}");
    }
    for name in TOUR_INTRO {
        let value = js_const_string(&app, name);
        assert!(plain_text(&value), "{name} is plain English: {value:?}");
    }
    assert_eq!(js_const_string(&app, "TOUR_INTRO_TITLE"), "Welcome to soos");
    let note = js_const_string(&app, "TOUR_INTRO_NOTE");
    assert!(
        note.contains("only remembers that you closed the tour") && note.contains("Feature tour"),
        "the introduction says what is remembered and how to reopen: {note}"
    );
    assert!(
        js_const_string(&app, "TOUR_INTRO_TEXT").contains("changes no setting"),
        "the introduction says the tour switches nothing"
    );
}

// ---------------------------------------------------------------------------------------------
// PTR-S6
// ---------------------------------------------------------------------------------------------

/// PTR-S6 (PTR6): the tour reads no server or user data, makes no request, shows fixed text
/// only and switches nothing.
#[test]
fn test_ptr_s6_tour_reads_no_data_requests_nothing_and_switches_nothing() {
    let app = app();
    let mask = mask_of(&app);
    let mut set_text_calls = 0usize;
    for name in TOUR_FUNCTIONS {
        let text = function_text(&app, &mask, name);
        let code = strip_js_comments(text);
        for forbidden in [
            "fetch(",
            "postJson(",
            "EventSource",
            "XMLHttpRequest",
            "sendBeacon",
            ".register(",
            "window.confirm(",
            ".click(",
            ".submit(",
            "latest",
            "alertsView",
            "pushView",
            "batteryView",
            "pushRegistration",
            ".json(",
            "JSON.",
            "event.data",
            ".value",
            "location",
            "navigator",
            "Date",
            "document.cookie",
            "textContent",
            "innerHTML",
            "outerHTML",
            "insertAdjacent",
            "matchMedia",
            "startViewTransition",
            "requestLock",
            "requestUnlock",
            "requestLogin",
            "requestLogout",
            "requestEnroll",
            "requestAlertsAck",
            "enableNotifications",
            "sendTestNotification",
            "disableNotifications",
            "startCameraView",
            "stopCameraView",
            "fetchStatus",
            "fetchAlerts",
            "fetchOffline",
            "fetchPush",
            "fetchCamera",
            "resume(",
            "showLogin",
            "showApp",
        ] {
            assert!(
                !code.contains(forbidden),
                "{name} must not contain `{forbidden}`"
            );
        }
        for args in call_arguments(&code, "setText") {
            assert_eq!(args.len(), 2, "{name}: setText(node, text): {args:?}");
            assert!(
                TOUR_SET_TEXT_ARGS.contains(&args[1].as_str()),
                "{name}: setText shows fixed text only, not `{}`",
                args[1]
            );
            set_text_calls += 1;
        }
    }
    assert!(set_text_calls >= 12, "tour setText calls: {set_text_calls}");
    let show = strip_js_comments(function_body(&app, &mask, "showTourTarget"));
    assert!(show.contains(".focus()"), "the link only moves the focus");
    assert!(
        show.contains("closeTour();"),
        "following a link closes the tour"
    );
    let render = function_body(&app, &mask, "renderTour");
    assert!(
        render.contains("const step = TOUR_STEPS[index - 1];"),
        "steps come from TOUR_STEPS only"
    );
}

// ---------------------------------------------------------------------------------------------
// PTR-S7
// ---------------------------------------------------------------------------------------------

/// PTR-S7 (PTR7): the first-run tour is offered once per page after sign-in (tailnet page or
/// Funnel session), never on the sign-in screen; Feature tour replays it; links target only
/// visible cards of the page.
#[test]
fn test_ptr_s7_first_run_offer_replay_and_links() {
    let app = app();
    let mask = mask_of(&app);
    assert!(
        function_body(&app, &mask, "showApp").contains("offerTour();"),
        "showApp offers the tour"
    );
    assert_eq!(
        mask.matches("offerTour();").count(),
        1,
        "offered from showApp only"
    );
    assert!(!function_body(&app, &mask, "showLogin").contains("Tour"));
    assert!(app.contains("let tourOffered = false;"));
    let offer = squash(function_body(&app, &mask, "offerTour"));
    assert!(
        offer.starts_with("if(tourOffered){return;}tourOffered=true;"),
        "offered once per page lifetime: {offer}"
    );

    let build = squash(function_body(&app, &mask, "buildTourCard"));
    for needle in [
        "if(tour!==null){returntour;}",
        "section.className=\"cardtour\";",
        "section.hidden=true;",
        "section.setAttribute(\"aria-label\",TOUR_REGION_LABEL);",
        "loginSection.parentNode.insertBefore(section,loginSection);",
        "document.querySelector(\"footer\")",
        "replayButton.addEventListener(\"click\",openTour);",
        "nextButton.addEventListener(\"click\",tourNext);",
        "backButton.addEventListener(\"click\",tourBack);",
        "showButton.addEventListener(\"click\",showTourTarget);",
        "skipButton.addEventListener(\"click\",closeTour);",
    ] {
        assert!(build.contains(needle), "buildTourCard: `{needle}`");
    }
    assert_eq!(
        build.matches(".type=\"button\";").count(),
        5,
        "five type=button buttons"
    );
    let (_, top) = top_level_statements(&app, &mask);
    assert!(
        top.contains("buildTourCard();"),
        "the card and button exist from load"
    );
    assert!(top.contains("document.addEventListener(\"keydown\", tourKeydown);"));

    let target = squash(function_body(&app, &mask, "tourTarget"));
    for (name, node) in [
        ("status", "statusCard"),
        ("logout", "logoutButton"),
        ("alerts", "alertsSection"),
        ("push", "pushSection"),
        ("offline", "offlineSection"),
        ("camera", "camera.section"),
    ] {
        assert!(
            target.contains(&format!("name===\"{name}\"")) && target.contains(node),
            "tourTarget maps {name} to {node}"
        );
    }
    assert!(
        target.contains("shownOnPage(node)"),
        "only a visible card is a target"
    );
    let shown = squash(function_body(&app, &mask, "shownOnPage"));
    assert!(
        shown.contains(".hidden===true") && shown.contains("document.body"),
        "visibility walks every ancestor: {shown}"
    );
    let key = squash(function_body(&app, &mask, "tourKeydown"));
    assert!(
        key.contains("!cameraFull") && key.contains("!tour.section.hidden"),
        "Escape closes an open tour only, never under a camera full screen: {key}"
    );
    let open = squash(function_body(&app, &mask, "openTour"));
    assert!(
        open.contains("tourIndex=0;")
            && open.contains("renderTour();")
            && open.contains("section.hidden=false;")
            && open.contains(".title.focus();"),
        "openTour starts on the introduction and moves the focus to its title: {open}"
    );
    let render = squash(function_body(&app, &mask, "renderTour"));
    assert!(
        render.contains("showButton.hidden=tourTarget(step.target)===null;"),
        "the link shows only for a visible target"
    );
    assert!(render.contains("backButton.hidden=index===0;"));
    assert!(render.contains("skipButton.hidden=index===TOUR_STEPS.length;"));
}

/// The top-level statements of `app.js` (function bodies removed) and the function names.
fn top_level_statements(app: &str, mask: &str) -> (Vec<String>, String) {
    let mut names = Vec::new();
    let mut out = String::new();
    let mut i = 0;
    while i < app.len() {
        if (mask[i..].starts_with("function ") || mask[i..].starts_with("async function "))
            && brace_depth_at(mask, i) == 0
        {
            let at = i + mask[i..].find("function ").unwrap() + "function ".len();
            let name: String = app[at..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let paren = at + name.len();
            let close_paren = matching(mask, paren);
            let open = close_paren + mask[close_paren..].find('{').unwrap();
            i = matching(mask, open) + 1;
            names.push(name);
            continue;
        }
        let ch = app[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    (names, out)
}

// ---------------------------------------------------------------------------------------------
// PTR-S8
// ---------------------------------------------------------------------------------------------

/// PTR-S8 (PTR8): the tour uses the brand role tokens and the motion tokens of
/// `Docs/UI_MOTION.md` (W4 card entry), and reduced motion still removes every transition.
#[test]
fn test_ptr_s8_style_uses_brand_and_motion_tokens() {
    let css = read(STYLE);
    let clean = strip_css_comments(&css);
    for (token, value) in [
        ("--t-medium", "260ms"),
        ("--ease-enter", "cubic-bezier(0.16, 1, 0.3, 1)"),
        ("--rise", "6px"),
    ] {
        let decl = format!("{token}:");
        assert_eq!(clean.matches(&decl).count(), 1, "{token} declared once");
        let at = clean.find(&decl).unwrap() + decl.len();
        let found = clean[at..].split(';').next().unwrap().trim();
        assert_eq!(found, value, "{token} has the Docs/UI_MOTION.md value");
    }
    // `.tour` twice: the base rule and its `@starting-style` entry state.
    let tour: Vec<String> = css_rule_bodies(&css, ".tour")
        .into_iter()
        .filter(|body| css_value(body, "transition").is_some())
        .collect();
    assert_eq!(tour.len(), 1, "one .tour rule with the transition");
    assert_eq!(
        css_rule_bodies(&css, ".tour").len(),
        2,
        "base rule and entry state"
    );
    assert_eq!(
        css_value(&tour[0], "transition").as_deref(),
        Some("opacity var(--t-medium) var(--ease-enter), transform var(--t-medium) var(--ease-enter)"),
        "the card enters with the W4 tokens"
    );
    let starting = css_at_block(&css, "@starting-style");
    let entry = css_rule_bodies(&starting, ".tour");
    assert_eq!(entry.len(), 1, "one .tour entry state");
    assert_eq!(css_value(&entry[0], "opacity").as_deref(), Some("0"));
    assert_eq!(
        css_value(&entry[0], "transform").as_deref(),
        Some("translateY(var(--rise))")
    );
    let reduce = css_at_block(&css, "@media (prefers-reduced-motion: reduce)");
    let all = css_rule_bodies(&reduce, "*, *::before, *::after");
    assert_eq!(
        all.len(),
        1,
        "the reduced-motion block keeps its universal rule"
    );
    assert_eq!(
        css_value(&all[0], "transition").as_deref(),
        Some("none !important"),
        "reduced motion removes every transition, the tour entry included"
    );
    for (selector, body) in [
        (".tour", tour[0].clone()),
        (".tour-step", one(&css, ".tour-step")),
        (".tour-text", one(&css, ".tour-text")),
        (".tour-nav", one(&css, ".tour-nav")),
        (".tour-nav button", one(&css, ".tour-nav button")),
        (
            "footer button.tour-replay",
            one(&css, "footer button.tour-replay"),
        ),
    ] {
        assert!(!body.contains('#'), "{selector}: no colour literal");
        assert!(
            !body.contains("rgb") && !body.contains("hsl"),
            "{selector}: no functional colour"
        );
        for (property, value) in body
            .split(';')
            .filter_map(|d| d.split_once(':'))
            .map(|(p, v)| (p.trim(), v.trim()))
        {
            if property.contains("color") || property == "background" {
                assert!(
                    value.starts_with("var(--") || value == "transparent",
                    "{selector}: {property} uses a role token, not `{value}`"
                );
            }
            let literal_time = value
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
                .any(|w| {
                    (w.ends_with("ms") || w.ends_with('s'))
                        && w.starts_with(|c: char| c.is_ascii_digit())
                });
            assert!(
                !literal_time,
                "{selector}: no literal duration in `{value}`"
            );
        }
    }
    let replay = one(&css, "footer button.tour-replay");
    assert_eq!(css_value(&replay, "color").as_deref(), Some("var(--link)"));
    assert_eq!(
        css_value(&replay, "background").as_deref(),
        Some("transparent")
    );
    assert!(
        css_value(&replay, "min-height").is_none(),
        "the replay button keeps the 48 px button touch height"
    );
    let hover = css_at_block(&css, "@media (hover: hover)");
    assert_eq!(
        css_rule_bodies(&hover, "footer button.tour-replay:hover:not(:disabled)").len(),
        1,
        "the hover background stays transparent"
    );
    assert!(
        css_rule_bodies(
            &css,
            "button:focus-visible, a:focus-visible, input:focus-visible, .tour h2:focus-visible"
        )
        .len()
            == 1,
        "the tour title gets the brand focus ring"
    );
}

fn one(css: &str, selector: &str) -> String {
    let bodies = css_rule_bodies(css, selector);
    assert_eq!(bodies.len(), 1, "exactly one `{selector}` rule");
    bodies[0].clone()
}

// ---------------------------------------------------------------------------------------------
// PTR-S9
// ---------------------------------------------------------------------------------------------

/// PTR-S9 (PTR9): the tour adds no inline style, script animation, markup or new origin; the
/// card is built at run time like the camera and offline cards.
#[test]
fn test_ptr_s9_no_inline_style_animation_or_markup_change() {
    let app = app();
    for forbidden in [
        ".style.",
        "cssText",
        ".animate(",
        "requestAnimationFrame",
        "setAttribute(\"style\"",
        "insertAdjacentHTML",
        "outerHTML",
        "document.write",
        "createElement(\"a\")",
        "createElement(\"script\")",
        "createElement(\"style\")",
        "createElement(\"img\")",
        "createElement(\"iframe\")",
    ] {
        assert!(
            !app.contains(forbidden),
            "app.js must not contain {forbidden}"
        );
    }
    let index = read(INDEX).to_ascii_lowercase();
    assert!(
        !index.contains("tour"),
        "index.html is unchanged: the tour is built by app.js"
    );
    let mask = mask_of(&app);
    let build = function_body(&app, &mask, "buildTourCard");
    let created: BTreeSet<String> = call_arguments(build, "document.createElement")
        .into_iter()
        .map(|args| args.join(","))
        .collect();
    let allowed: BTreeSet<String> = ["\"section\"", "\"p\"", "\"h2\"", "\"div\"", "\"button\""]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert!(
        created.is_subset(&allowed) && created.len() == allowed.len(),
        "the card uses section, p, h2, div and button only: {created:?}"
    );
    for attribute in call_arguments(build, "setAttribute") {
        assert_eq!(
            attribute,
            vec![
                "\"aria-label\"".to_string(),
                "TOUR_REGION_LABEL".to_string()
            ],
            "the only attribute set is the region label"
        );
    }
    let show = function_body(&app, &mask, "showTourTarget");
    for attribute in call_arguments(show, "setAttribute") {
        assert_eq!(
            attribute,
            vec!["\"tabindex\"".to_string(), "\"-1\"".to_string()],
            "a target only becomes programmatically focusable"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// PTR-S10
// ---------------------------------------------------------------------------------------------

/// PTR-S10 (PTR10): ADR, matrix rows, operator documentation and walkthrough.
#[test]
fn test_ptr_s10_documentation_adr_matrix_and_walkthrough() {
    let decisions = read(DECISIONS);
    let adr = decisions
        .lines()
        .find(|l| l.contains(ADR_TITLE))
        .unwrap_or_else(|| panic!("{DECISIONS}: ADR `{ADR_TITLE}`"));
    for needle in ["#414", "#363", TOUR_KEY, "PTR1", "#357"] {
        assert!(adr.contains(needle), "the ADR mentions {needle}");
    }
    let doc = read(REMOTE_DOC);
    let start = doc
        .find("\n## 2k. Feature tour")
        .expect("Docs/REMOTE_COMPANION.md section 2k");
    let rest = &doc[start + 1..];
    let section = &rest[..rest[3..].find("\n## ").map_or(rest.len(), |e| e + 3)];
    for needle in [
        TOUR_KEY,
        "Feature tour",
        "Skip tour",
        "private",
        "native Android app",
        "soos-gui",
    ] {
        assert!(section.contains(needle), "section 2k mentions {needle}");
    }
    let matrix = read(MATRIX);
    for row in 1..=10 {
        let line = matrix
            .lines()
            .find(|l| l.starts_with(&format!("| PTR{row} |")))
            .unwrap_or_else(|| panic!("matrix row PTR{row}"));
        assert!(
            line.contains("remote_tour_contract::test_ptr_s"),
            "PTR{row} cites the contract"
        );
    }
    let walkthrough = read(WALKTHROUGH);
    for needle in ["#414", "#363", TOUR_KEY, "native Android app"] {
        assert!(
            walkthrough.contains(needle),
            "walkthrough 222 mentions {needle}"
        );
    }
}
