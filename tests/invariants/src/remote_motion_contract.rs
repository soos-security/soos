//! Static contracts of the web motion system of the `soos-remote` app (GitHub #433 = PR 2 of
//! #358; ADR "[2026-10-13] Motion System for soos-gui and soos-remote"; normative document
//! `Docs/UI_MOTION.md` §2–§6 and §8; tester contract `AI/tester_contract_motion.md` Part B.1;
//! spec `AI/architect_spec_remote_motion.md`; matrix UIM8 and RMO1–RMO13):
//!
//! - RMO-S1 the seventeen tokens and the gated spring curve, declared once each in `:root`;
//! - RMO-S2 every timing value outside `:root` uses the tokens only;
//! - RMO-S3 only compositor-friendly and colour properties are transitioned;
//! - RMO-S4 keyframes move `transform` and `opacity` only, every name is declared once, loops
//!   only under `.is-pending`, the attention nudge runs two cycles;
//! - RMO-S5 the single reduced-motion block removes every animation and transition;
//! - RMO-S6 the brand is kept (no new shadow, gradient, filter or `:has()`; the star turn);
//! - RMO-S7 `@starting-style` entries for sections revealed from `[hidden]`, instant exits;
//! - RMO-S8 `app.js` hooks: class toggles and one guarded `startViewTransition`;
//! - RMO-S9 security truth first: state, text and `disabled` before any motion class;
//! - RMO-S10 the pending star follows the request flags and is bounded by the 5 s timers;
//! - RMO-S11 the list stagger runs on the first render after the section appears only;
//! - RMO-S12 unchanged surface: CSP, asset set, element ids and labels.
//!
//! The project has no JavaScript runtime in CI: the checks are static. The module carries its
//! own copy of a one-level CSS parser and a small JavaScript function extractor (self-tested),
//! so the helpers of the other remote contracts stay untouched.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const STYLE: &str = "crates/remote/assets/style.css";
const APP_JS: &str = "crates/remote/assets/app.js";
const INDEX: &str = "crates/remote/assets/index.html";
const HTTP_RS: &str = "crates/remote/src/http.rs";
const ASSETS_RS: &str = "crates/remote/src/assets.rs";
const ASSETS_DIR: &str = "crates/remote/assets";
const GUI_THEME: &str = "crates/gui/src/theme.rs";
const MOTION_DOC: &str = "Docs/UI_MOTION.md";

/// `(CSS custom property, theme.rs constant, exact CSS value, milliseconds when a duration)`
/// (`Docs/UI_MOTION.md` §2).
const TOKENS: [(&str, &str, &str, Option<u32>); 17] = [
    ("--t-fast", "T_FAST", "120ms", Some(120)),
    ("--t-short", "T_SHORT", "180ms", Some(180)),
    ("--t-medium", "T_MEDIUM", "260ms", Some(260)),
    ("--t-long", "T_LONG", "380ms", Some(380)),
    ("--t-spring", "T_SPRING", "400ms", Some(400)),
    ("--t-short-exit", "T_SHORT_EXIT", "126ms", Some(126)),
    ("--t-medium-exit", "T_MEDIUM_EXIT", "182ms", Some(182)),
    ("--t-long-exit", "T_LONG_EXIT", "266ms", Some(266)),
    ("--t-stagger", "T_STAGGER", "30ms", Some(30)),
    ("--t-pending", "T_PENDING", "1200ms", Some(1200)),
    (
        "--ease-standard",
        "EASE_STANDARD",
        "cubic-bezier(0.2, 0, 0, 1)",
        None,
    ),
    (
        "--ease-enter",
        "EASE_ENTER",
        "cubic-bezier(0.16, 1, 0.3, 1)",
        None,
    ),
    (
        "--ease-exit",
        "EASE_EXIT",
        "cubic-bezier(0.4, 0, 1, 1)",
        None,
    ),
    ("--press-scale", "PRESS_SCALE", "0.98", None),
    ("--rise", "RISE_PX", "6px", None),
    ("--slide", "SLIDE_PX", "8px", None),
    ("--star-turn", "STAR_TURN_DEG", "45deg", None),
];

/// `--ease-spring` (`Docs/UI_MOTION.md` §3): the fallback and the gated `linear()` curve.
const SPRING_FALLBACK: &str = "cubic-bezier(0.37, 0.73, 0.1, 1.15)";
const SPRING_LINEAR: &str = "linear(0, 0.0063 1.5%, 0.0263 3%, 0.0595 4.8%, 0.1065 6.6%, 0.1522 8.2%, 0.2104 10%, 0.4504 16.9%, 0.5562 20.1%, 0.6552 23.4%, 0.7391 26.6%, 0.8083 29.6%, 0.8658 32.8%, 0.9136 35.9%, 0.9524 39.3%, 0.9829 42.9%, 1.0052 46.8%, 1.0197 51%, 1.0271 55.9%, 1.0269 63.8%, 1.0085 84.4%, 1)";
const SPRING_GATE: &str = "@supports (transition-timing-function: linear(0, 1))";

const REDUCE_NEEDLE: &str = "prefers-reduced-motion: reduce";

/// Timing properties whose values must use the tokens (Part B.1 item 2).
const TIMING_PROPERTIES: [&str; 8] = [
    "transition",
    "transition-duration",
    "transition-delay",
    "transition-timing-function",
    "animation",
    "animation-duration",
    "animation-delay",
    "animation-timing-function",
];

/// Properties a transition may name (Part B.1 item 3).
const TRANSITIONABLE: [&str; 11] = [
    "transform",
    "translate",
    "scale",
    "rotate",
    "opacity",
    "color",
    "background-color",
    "border-color",
    "outline-color",
    "fill",
    "box-shadow",
];

/// Properties a `@keyframes` step may declare (Part B.1 item 4).
const KEYFRAME_PROPERTIES: [&str; 5] = ["transform", "translate", "scale", "rotate", "opacity"];

/// The transform family: the only properties the spring curve may drive (§3).
const TRANSFORM_FAMILY: [&str; 4] = ["transform", "translate", "scale", "rotate"];

/// Rules that may transition `box-shadow`: the elements that already draw a 2 px inset
/// outline (buttons and the unlocked status dot); no shadow is added.
const BOX_SHADOW_TRANSITION_RULES: [&str; 4] = [
    "button",
    ".push button",
    ".stat-dot",
    "button:active:not(:disabled)",
];

/// Every `box-shadow` declaration of the base tree `3f76090` (`selectors`, value); the set
/// stays exactly this (no new shadow, Part B.1 item 6).
const BASE_BOX_SHADOWS: [(&str, &str); 6] = [
    (
        ".stat-tile:has(.state-unlocked) .stat-dot",
        "inset 0 0 0 var(--stroke-card) var(--dot-unlocked)",
    ),
    (
        "button.secondary",
        "inset 0 0 0 var(--stroke-card) var(--secondary-fg)",
    ),
    (
        "button.secondary.danger",
        "inset 0 0 0 var(--stroke-card) var(--danger-fg)",
    ),
    ("button:disabled", "none"),
    (
        "button.secondary:disabled, button.secondary.danger:disabled",
        "inset 0 0 0 var(--stroke-card) var(--divider)",
    ),
    (
        ".stat-tile[data-state=\"unlocked\"] .stat-dot",
        "inset 0 0 0 var(--stroke-card) var(--dot-unlocked)",
    ),
];

/// `(state class selector, keyframe name)`: one keyframe name per state class (W1).
const STATE_KEYFRAMES: [(&str, &str); 6] = [
    (".state-locked", "soos-state-locked"),
    (".state-unlocked", "soos-state-unlocked"),
    (".state-no_session", "soos-state-no-session"),
    (".state-unavailable", "soos-state-unavailable"),
    (".state-unreachable", "soos-state-unreachable"),
    (".state-unknown", "soos-state-unknown"),
];

/// The selectors `@starting-style` may target: elements revealed from `[hidden]` (W4, W5, W7
/// and the feature tour card).
const ENTRY_SELECTORS: [&str; 8] = [
    ".card.login",
    ".card.alerts",
    ".card.push",
    ".card.enroll",
    ".card.camera",
    ".tour",
    ".banner",
    ".push button",
];

const STAR_TURN_SELECTOR: &str = ".stat-tile[data-state=\"unlocked\"] .stat-star";

/// The scaled star mark of the pending star (508 → 16 units).
const PENDING_STAR_PATH: &str =
    "path(\"M8 0C8 4.418 11.582 8 16 8L8 8ZM0 8C4.418 8 8 11.582 8 16V8H0Z\")";

/// The single pending-state helper of `app.js` (spec D-12), compared without whitespace.
const CANONICAL_SYNC_PENDING: &str = r#"
function syncPending() {
  lockButton.classList.toggle("is-pending", lockRequestedAt !== 0);
  unlockButton.classList.toggle("is-pending", unlockRequestedAt !== 0);
  statTile.classList.toggle("is-pending", lockRequestedAt !== 0 || unlockRequestedAt !== 0);
}
"#;

/// The camera full-screen class writer (spec D-14): it reads the current flag.
const CANONICAL_APPLY_CAMERA_FULL: &str = r#"
function applyCameraFull() {
  const c = buildCameraCard();
  c.stage.classList.toggle("camera-full", cameraFull);
  document.documentElement.classList.toggle("camera-lock", cameraFull);
  c.closeButton.hidden = !cameraFull;
}
"#;

/// The CSP of every response (`crates/remote/src/http.rs`), unchanged.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The eleven embedded web assets.
const ASSET_FILES: [&str; 11] = [
    "app.js",
    "apple-touch-icon.png",
    "badge-96.png",
    "icon-192.png",
    "icon-512.png",
    "icon-maskable-512.png",
    "icon.svg",
    "index.html",
    "manifest.webmanifest",
    "style.css",
    "sw.js",
];

/// Every element id of `index.html` at the base tree.
const ELEMENT_IDS: [&str; 29] = [
    "login",
    "login-button",
    "login-feedback",
    "status-card",
    "state",
    "activity",
    "updated",
    "lock",
    "unlock",
    "feedback",
    "logout",
    "alerts",
    "alerts-summary",
    "alerts-coverage",
    "alerts-history",
    "alerts-ack",
    "alerts-feedback",
    "push",
    "push-state",
    "push-devices",
    "push-hint",
    "push-enable",
    "push-test",
    "push-disable",
    "push-feedback",
    "enroll",
    "enroll-code",
    "enroll-button",
    "enroll-feedback",
];

/// `(button id, label)` of `index.html`, unchanged.
const BUTTON_LABELS: [(&str, &str); 9] = [
    ("login-button", "Sign in with your passkey"),
    ("lock", "Lock now"),
    ("unlock", "Unlock now"),
    ("logout", "Sign out"),
    ("alerts-ack", "Acknowledge"),
    ("push-enable", "Enable notifications"),
    ("push-test", "Send test notification"),
    ("push-disable", "Disable notifications"),
    ("enroll-button", "Add this device&#39;s passkey"),
];

/// The delays of the five `setTimeout` calls of the base tree: motion adds none (item 9).
const TIMEOUT_DELAYS: [&str; 5] = [
    "Math.max(0, deadline - Date.now())",
    "LOCK_CONFIRM_UI_MS",
    "UNLOCK_CONFIRM_UI_MS",
    "SERVICE_WORKER_READY_TIMEOUT_MS",
    "3000",
];

// ---------------------------------------------------------------------------------------------
// File helpers
// ---------------------------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn exists(rel: &str) -> bool {
    workspace_root().join(rel).exists()
}

// ---------------------------------------------------------------------------------------------
// CSS helpers (own copy of the one-level parser of `remote_brand_contract.rs`)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    /// The enclosing at-rule prelude (whitespace normalized), if any.
    at: Option<String>,
    selectors: Vec<String>,
    decls: Vec<(String, String)>,
}

impl Rule {
    fn is_top_root(&self) -> bool {
        self.at.is_none() && self.selectors == [":root"]
    }

    fn selector_text(&self) -> String {
        self.selectors.join(", ")
    }

    fn value(&self, prop: &str) -> Option<&str> {
        self.decls
            .iter()
            .rev()
            .find(|(p, _)| p == prop)
            .map(|(_, v)| v.as_str())
    }
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => rest = "",
        }
        out.push(' ');
    }
    out.push_str(rest);
    out
}

/// A flat list of style rules (one level of at-rule nesting), comments removed.
fn parse_css(css: &str) -> Result<Vec<Rule>, String> {
    let css = strip_css_comments(css);
    let mut rules = Vec::new();
    let mut at: Option<String> = None;
    let mut rest = css.as_str();
    loop {
        let open = rest.find('{');
        let close = rest.find('}');
        if let Some(c) = close.filter(|c| open.is_none_or(|o| *c < o)) {
            if at.is_none() || !rest[..c].trim().is_empty() {
                return Err(format!("unbalanced `}}` near `{}`", rest[..c].trim()));
            }
            at = None;
            rest = &rest[c + 1..];
            continue;
        }
        let Some(o) = open else {
            if !rest.trim().is_empty() {
                return Err(format!("trailing CSS text `{}`", rest.trim()));
            }
            if at.is_some() {
                return Err("unclosed at-rule block".to_string());
            }
            return Ok(rules);
        };
        let prelude = squash(&rest[..o]);
        if prelude.starts_with('@') {
            if at.is_some() {
                return Err(format!("nested at-rule `{prelude}`"));
            }
            at = Some(prelude);
            rest = &rest[o + 1..];
            continue;
        }
        let end = rest[o + 1..]
            .find('}')
            .ok_or_else(|| format!("unclosed rule `{prelude}`"))?;
        let body = &rest[o + 1..o + 1 + end];
        if body.contains('{') {
            return Err(format!("nested block inside rule `{prelude}`"));
        }
        let decls = body
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

fn css_rules() -> Vec<Rule> {
    parse_css(&read(STYLE)).unwrap_or_else(|e| panic!("style.css does not parse: {e}"))
}

/// Whether an at-rule prelude is the reduced-motion media block.
fn is_reduce_block(prelude: &str) -> bool {
    prelude.starts_with("@media") && squash(&prelude.replace(':', ": ")).contains(REDUCE_NEEDLE)
}

/// The rules whose selector list is exactly `selectors` (outside any at-rule).
fn top_rules<'a>(rules: &'a [Rule], selectors: &str) -> Vec<&'a Rule> {
    rules
        .iter()
        .filter(|r| r.at.is_none() && r.selector_text() == selectors)
        .collect()
}

/// Splits `value` at the commas outside parentheses.
fn split_top_commas(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in value.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                out.push(squash(&current));
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        out.push(squash(&current));
    }
    out
}

/// The words of `value` with the parenthesized groups kept whole (`var(--a) 2` → `var(--a)`, `2`).
fn css_words(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in value.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Whether `word` is a literal CSS time (`120ms`, `0.2s`, `0s`).
fn is_literal_time(word: &str) -> bool {
    let w = word.trim_start_matches(['-', '+']);
    let digits = w
        .bytes()
        .take_while(|b| b.is_ascii_digit() || *b == b'.')
        .count();
    digits > 0 && matches!(&w[digits..], "ms" | "s")
}

/// Checks one timing value: every timing word is a `var(--t-*)` / `var(--ease-*)` token or a
/// `calc(var(--t-stagger) * N)` with an integer `N` ≤ 5; `0s` only where `allow_zero`.
fn check_timing_value(value: &str, allow_zero: bool) -> Result<(), String> {
    for item in split_top_commas(value) {
        for word in css_words(&item) {
            if word.starts_with("var(") {
                let inner = word.trim_start_matches("var(").trim_end_matches(')').trim();
                if !(inner.starts_with("--t-") || inner.starts_with("--ease-")) {
                    return Err(format!("`{value}`: {word} is not a motion token"));
                }
            } else if word.starts_with("calc(") {
                let inner = squash(word.trim_start_matches("calc(").trim_end_matches(')'));
                let Some(n) = inner.strip_prefix("var(--t-stagger) * ") else {
                    return Err(format!("`{value}`: calc() only of --t-stagger, got {word}"));
                };
                match n.parse::<u32>() {
                    Ok(n) if n <= 5 => {}
                    _ => return Err(format!("`{value}`: stagger factor `{n}` is not 0..=5")),
                }
            } else if word.contains('(') {
                return Err(format!("`{value}`: literal function {word}"));
            } else if is_literal_time(&word) && !(allow_zero && word == "0s") {
                return Err(format!("`{value}`: literal duration {word}"));
            }
        }
    }
    Ok(())
}

/// The property of each transition item (`transform var(--t-fast)` → `transform`).
fn transition_properties(value: &str) -> Vec<String> {
    if squash(value).trim_end_matches("!important").trim() == "none" {
        return Vec::new();
    }
    split_top_commas(value)
        .iter()
        .filter_map(|item| css_words(item).into_iter().next())
        .collect()
}

/// Keyframe names declared by `@keyframes` preludes, in order (duplicates kept).
fn keyframe_names(css: &str) -> Vec<String> {
    let clean = strip_css_comments(css);
    clean
        .match_indices("@keyframes")
        .map(|(at, _)| {
            let after = &clean[at + "@keyframes".len()..];
            squash(&after[..after.find('{').expect("keyframes block")])
        })
        .collect()
}

/// Words of an `animation` shorthand that are keyframe names (not keywords, numbers or tokens).
fn animation_names(value: &str) -> Vec<String> {
    const KEYWORDS: [&str; 14] = [
        "none",
        "infinite",
        "normal",
        "reverse",
        "alternate",
        "alternate-reverse",
        "forwards",
        "backwards",
        "both",
        "running",
        "paused",
        "!important",
        "linear",
        "ease",
    ];
    let mut out = Vec::new();
    for item in split_top_commas(value) {
        for word in css_words(&item) {
            let numeric = word.parse::<f64>().is_ok();
            if word.contains('(') || numeric || KEYWORDS.contains(&word.as_str()) {
                continue;
            }
            if is_literal_time(&word) {
                continue;
            }
            out.push(word);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// JavaScript helpers
// ---------------------------------------------------------------------------------------------

/// Byte index of the `}` matching the `{` at `open`, skipping strings and comments.
fn matching_brace(code: &str, open: usize) -> usize {
    let bytes = code.as_bytes();
    assert_eq!(bytes[open], b'{', "matching_brace starts at a brace");
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' | b'`' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    panic!("unclosed block at {open}");
}

/// The full text (`function name(...) { ... }`) of the single top-level function `name`.
fn function_text<'a>(code: &'a str, name: &str) -> &'a str {
    let needles = [
        format!("\nfunction {name}("),
        format!("\nasync function {name}("),
    ];
    let found: Vec<usize> = needles
        .iter()
        .flat_map(|n| code.match_indices(n.as_str()).map(|(at, _)| at + 1))
        .collect();
    assert_eq!(found.len(), 1, "app.js defines `{name}` exactly once");
    let start = found[0];
    let open = start + code[start..].find('{').expect("function body");
    &code[start..=matching_brace(code, open)]
}

/// The text of `code` without `//` and `/* */` comments (strings kept).
fn strip_js_comments(code: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' | b'`' => {
                let quote = bytes[i];
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(bytes.len());
                out.push_str(&code[start..i]);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            _ => {}
        }
        let ch = code[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn no_whitespace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The JavaScript of the fenced `js` block of `Docs/UI_MOTION.md` that defines `name`.
fn doc_js_function(doc: &str, name: &str) -> String {
    let needle = format!("function {name}(");
    let at = doc.find(&needle).expect("the document defines the helper");
    let block_start = doc[..at].rfind("```js").expect("inside a js block");
    let block_end = at + doc[at..].find("```").expect("end of the js block");
    let block = &doc[block_start + "```js".len()..block_end];
    let local = block.find(&needle).unwrap();
    let open = local + block[local..].find('{').unwrap();
    block[local..=matching_brace(block, open)].to_string()
}

/// Every `name(` call argument list in `code` (a shallow split, enough for the hooks).
fn call_argument_texts(code: &str, name: &str) -> Vec<String> {
    let needle = format!("{name}(");
    let mut out = Vec::new();
    for (at, _) in code.match_indices(&needle) {
        let before = code[..at].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
            continue;
        }
        let start = at + needle.len();
        let mut depth = 1usize;
        let mut end = start;
        for (i, ch) in code[start..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push(squash(&code[start..end]));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Helper self-test
// ---------------------------------------------------------------------------------------------

/// The helpers can fail: parser errors, timing and property classification, JS extraction.
#[test]
fn test_rmo_helper_self_test() {
    let css = "/* c */ :root { --a: 1ms; }\n@supports (x: y) { :root { --a: 2ms; } }\n.b, .c { transition: opacity var(--t-fast) var(--ease-enter), transform var(--t-spring); }\n@keyframes k { from { opacity: 0; } }\n";
    let rules = parse_css(css).expect("sample parses");
    assert_eq!(rules.len(), 4);
    assert!(rules[0].is_top_root());
    assert_eq!(rules[1].at.as_deref(), Some("@supports (x: y)"));
    assert_eq!(rules[2].selectors, [".b", ".c"]);
    assert_eq!(rules[3].at.as_deref(), Some("@keyframes k"));
    assert!(parse_css("@media a { @supports b { x { y: z; } } }").is_err());
    assert!(parse_css("a { b: c; ").is_err());
    assert!(parse_css("a { b { c: d; } }").is_err());
    assert_eq!(keyframe_names(css), ["k"]);

    assert_eq!(
        transition_properties(rules[2].value("transition").unwrap()),
        ["opacity", "transform"]
    );
    assert!(transition_properties("none !important").is_empty());
    assert!(check_timing_value("opacity var(--t-fast) var(--ease-enter)", false).is_ok());
    assert!(check_timing_value("a var(--t-medium) calc(var(--t-stagger) * 5)", false).is_ok());
    assert!(check_timing_value("a calc(var(--t-stagger) * 6)", false).is_err());
    assert!(check_timing_value("a calc(var(--t-fast) * 2)", false).is_err());
    assert!(check_timing_value("a 120ms", false).is_err());
    assert!(check_timing_value("a .2s", false).is_err());
    assert!(check_timing_value("a var(--t-fast) cubic-bezier(0, 0, 1, 1)", false).is_err());
    assert!(check_timing_value("a var(--t-fast) linear(0, 1)", false).is_err());
    assert!(check_timing_value("a var(--t-fast) steps(2)", false).is_err());
    assert!(check_timing_value("a var(--rise)", false).is_err());
    assert!(check_timing_value("0s", false).is_err());
    assert!(check_timing_value("0s", true).is_ok());
    assert!(is_literal_time("0s") && is_literal_time("1.5s") && !is_literal_time("ease"));
    assert_eq!(
        animation_names("k var(--t-medium) var(--ease-standard) 2"),
        ["k"]
    );
    assert_eq!(
        animation_names("p var(--t-pending) var(--ease-standard) infinite"),
        ["p"]
    );
    assert!(animation_names("none !important").is_empty());
    assert_eq!(css_words("a var(--x y) 2"), ["a", "var(--x y)", "2"]);

    let js = "const s = \"{\";\nfunction f(a) {\n  if (a) { return \"}\"; } // }\n  return 1;\n}\nfunction g() {}\n";
    assert_eq!(
        function_text(js, "f"),
        "function f(a) {\n  if (a) { return \"}\"; } // }\n  return 1;\n}"
    );
    assert_eq!(function_text(js, "g"), "function g() {}");
    assert_eq!(
        strip_js_comments("a(); // x\n/* y */b(\"//\");"),
        "a(); \nb(\"//\");"
    );
    assert_eq!(
        call_argument_texts("x(1); f(a, g(b)); y.f(2);", "f"),
        ["a, g(b)"]
    );
    let doc = "text\n```js\n// c\nfunction h(x) {\n  return { a: x };\n}\n```\n";
    assert_eq!(
        doc_js_function(doc, "h"),
        "function h(x) {\n  return { a: x };\n}"
    );
}

// ---------------------------------------------------------------------------------------------
// RMO-S1 — tokens
// ---------------------------------------------------------------------------------------------

/// RMO-S1 (RMO1, Part B.1 item 1): the seventeen tokens are declared once each, in a top-level
/// `:root`, with the values of `Docs/UI_MOTION.md` §2; `--ease-spring` twice (fallback at top
/// level, `linear()` behind the `@supports` gate); durations mirror `theme.rs` once it has them.
#[test]
fn test_rmo_s1_motion_tokens_declared_once_with_pinned_values() {
    let rules = css_rules();
    let doc = read(MOTION_DOC);
    for (css, rust, value, _) in TOKENS {
        let row = format!("| `{css}` | `{rust}` | `{value}` |");
        assert!(doc.contains(&row), "{MOTION_DOC} still pins {row}");
        let decls: Vec<(&Rule, &String)> = rules
            .iter()
            .flat_map(|r| {
                r.decls
                    .iter()
                    .filter(|d| d.0 == css)
                    .map(move |d| (r, &d.1))
            })
            .collect();
        assert_eq!(decls.len(), 1, "style.css declares {css} exactly once");
        let (rule, found) = decls[0];
        assert!(rule.is_top_root(), "{css} lives in a top-level :root");
        assert_eq!(found, value, "{css} has the Docs/UI_MOTION.md value");
    }

    let spring: Vec<(&Rule, &String)> = rules
        .iter()
        .flat_map(|r| {
            r.decls
                .iter()
                .filter(|d| d.0 == "--ease-spring")
                .map(move |d| (r, &d.1))
        })
        .collect();
    assert_eq!(spring.len(), 2, "--ease-spring is declared exactly twice");
    let top: Vec<&(&Rule, &String)> = spring.iter().filter(|(r, _)| r.is_top_root()).collect();
    assert_eq!(top.len(), 1, "one top-level --ease-spring");
    assert_eq!(
        top[0].1, SPRING_FALLBACK,
        "the top-level value is the fallback"
    );
    let gated: Vec<&(&Rule, &String)> = spring
        .iter()
        .filter(|(r, _)| r.at.as_deref() == Some(SPRING_GATE) && r.selectors == [":root"])
        .collect();
    assert_eq!(gated.len(), 1, "one --ease-spring inside `{SPRING_GATE}`");
    assert_eq!(
        gated[0].1, SPRING_LINEAR,
        "the gated value is the pinned linear()"
    );
    assert!(
        rules
            .iter()
            .filter(|r| r.at.as_deref() == Some(SPRING_GATE))
            .all(|r| r.selectors == [":root"] && r.decls.len() == 1),
        "the gate holds the spring curve only"
    );

    // Forward-compatible cross-check with the GUI theme (feat/gui-motion adds the constants).
    if exists(GUI_THEME) {
        let theme = read(GUI_THEME);
        for (css, rust, _, ms) in TOKENS {
            let Some(ms) = ms else { continue };
            let prefix = format!("pub const {rust}: f32 = ");
            if let Some(line) = theme.lines().find(|l| l.trim_start().starts_with(&prefix)) {
                let value = line.trim_start()[prefix.len()..]
                    .trim_end_matches(';')
                    .trim()
                    .parse::<f64>()
                    .unwrap_or_else(|e| panic!("theme.rs {rust}: {e}"));
                assert!(
                    (value * 1000.0 - f64::from(ms)).abs() < 1e-6,
                    "theme.rs {rust} = {value} s must equal {css} {ms} ms"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// RMO-S2 — token use
// ---------------------------------------------------------------------------------------------

/// RMO-S2 (RMO2, Part B.1 item 2): outside `:root`, every timing value uses the duration and
/// easing tokens (and `calc()` of `--t-stagger` by an integer ≤ 5); no literal duration or
/// curve; the only literal allowed is `0s` inside the reduced-motion block.
#[test]
fn test_rmo_s2_timing_values_use_the_tokens_only() {
    let rules = css_rules();
    let mut checked = 0usize;
    for rule in rules.iter().filter(|r| r.selectors != [":root"]) {
        let in_reduce = rule.at.as_deref().is_some_and(is_reduce_block);
        for (prop, value) in &rule.decls {
            if !TIMING_PROPERTIES.contains(&prop.as_str()) {
                continue;
            }
            check_timing_value(value, in_reduce)
                .unwrap_or_else(|e| panic!("`{} {{ {prop} }}`: {e}", rule.selector_text()));
            checked += 1;
        }
    }
    assert!(checked >= 20, "timing declarations found: {checked}");
    let clean = strip_css_comments(&read(STYLE));
    for (at, _) in clean.match_indices("cubic-bezier(") {
        let line_start = clean[..at].rfind(['{', ';']).map_or(0, |p| p + 1);
        let decl = clean[line_start..at].trim();
        assert!(
            decl.starts_with("--ease-"),
            "a literal cubic-bezier( only as an --ease-* token value, found after `{decl}`"
        );
    }
    for needle in ["steps(", "transition-duration: 0", "animation-duration: 0"] {
        assert!(
            !clean.contains(needle),
            "style.css must not contain `{needle}`"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// RMO-S3 — transitioned properties
// ---------------------------------------------------------------------------------------------

/// RMO-S3 (RMO3, Part B.1 item 3): only `transform`, the individual transform properties,
/// `opacity` and colour properties are transitioned; `box-shadow` only where the inset outline
/// already exists; never `all` nor a layout property; the spring drives the transform family only.
#[test]
fn test_rmo_s3_only_compositor_and_colour_properties_transition() {
    let rules = css_rules();
    let mut seen = BTreeSet::new();
    for rule in &rules {
        for (prop, value) in &rule.decls {
            let props = match prop.as_str() {
                "transition" => transition_properties(value),
                "transition-property" => split_top_commas(value),
                _ => continue,
            };
            for p in &props {
                assert!(
                    TRANSITIONABLE.contains(&p.as_str()),
                    "`{} {{ {prop}: {value} }}` transitions `{p}`",
                    rule.selector_text()
                );
                if p == "box-shadow" {
                    assert!(
                        BOX_SHADOW_TRANSITION_RULES.contains(&rule.selector_text().as_str()),
                        "box-shadow is only transitioned on an existing inset outline, not `{}`",
                        rule.selector_text()
                    );
                }
                seen.insert(p.clone());
            }
            if prop == "transition" {
                for item in split_top_commas(value) {
                    if item.contains("var(--ease-spring)") {
                        let p = css_words(&item).into_iter().next().unwrap_or_default();
                        assert!(
                            TRANSFORM_FAMILY.contains(&p.as_str()),
                            "the spring drives the transform family only, not `{p}` in `{}`",
                            rule.selector_text()
                        );
                    }
                }
            }
        }
    }
    for wanted in [
        "transform",
        "rotate",
        "opacity",
        "background-color",
        "border-color",
    ] {
        assert!(seen.contains(wanted), "a `{wanted}` transition exists");
    }
    let clean = strip_css_comments(&read(STYLE));
    for forbidden in [
        "transition: all",
        "transition:all",
        "transition-property: all",
    ] {
        assert!(
            !clean.contains(forbidden),
            "style.css must not contain `{forbidden}`"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// RMO-S4 — keyframes
// ---------------------------------------------------------------------------------------------

/// RMO-S4 (RMO4, Part B.1 item 4): keyframes move the transform family and opacity only; each
/// name is declared once and used; `infinite` only under `.is-pending`; the attention nudge
/// runs exactly two cycles; one keyframe name per state class (W1).
#[test]
fn test_rmo_s4_keyframes_loops_and_state_word() {
    let css = read(STYLE);
    let rules = css_rules();
    let declared = keyframe_names(&css);
    let unique: BTreeSet<&String> = declared.iter().collect();
    assert_eq!(
        unique.len(),
        declared.len(),
        "each @keyframes name is declared once: {declared:?}"
    );
    assert!(declared.len() >= 9, "keyframes declared: {declared:?}");
    for rule in rules
        .iter()
        .filter(|r| r.at.as_deref().is_some_and(|a| a.starts_with("@keyframes")))
    {
        for (prop, _) in &rule.decls {
            assert!(
                KEYFRAME_PROPERTIES.contains(&prop.as_str()),
                "`{}` step `{}` declares `{prop}`",
                rule.at.as_deref().unwrap_or_default(),
                rule.selector_text()
            );
        }
    }

    let mut used = BTreeSet::new();
    for rule in rules
        .iter()
        .filter(|r| !r.at.as_deref().is_some_and(|a| a.starts_with("@keyframes")))
    {
        for (prop, value) in &rule.decls {
            if prop != "animation"
                && prop != "animation-name"
                && prop != "animation-iteration-count"
            {
                continue;
            }
            let words = css_words(value);
            if words.iter().any(|w| w == "infinite") {
                assert!(
                    rule.selectors.iter().all(|s| s.contains(".is-pending")),
                    "`infinite` only under .is-pending, not `{}`",
                    rule.selector_text()
                );
            }
            if prop == "animation-iteration-count" {
                continue;
            }
            for name in animation_names(value) {
                assert!(
                    declared.contains(&name),
                    "`{}` uses the undeclared animation `{name}`",
                    rule.selector_text()
                );
                used.insert(name);
            }
        }
    }
    for name in &declared {
        assert!(used.contains(name), "@keyframes {name} is used");
    }

    let nudge = top_rules(&rules, ".alerts-attention .alerts-summary");
    assert_eq!(nudge.len(), 1, "one attention rule");
    let value = nudge[0].value("animation").expect("the attention nudge");
    assert_eq!(
        value, "soos-nudge var(--t-medium) var(--ease-standard) 2",
        "the nudge runs two cycles of --t-medium, never infinite"
    );
    let steps: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.at.as_deref() == Some("@keyframes soos-nudge"))
        .collect();
    assert_eq!(steps.len(), 2, "the nudge has two steps");
    for rule in &steps {
        assert_eq!(rule.decls.len(), 1);
        let (prop, v) = &rule.decls[0];
        assert_eq!(prop, "translate", "the nudge moves `translate`");
        assert!(v == "-3px 0" || v == "3px 0", "the nudge is ±3 px: {v}");
    }

    let mut names = BTreeSet::new();
    for (selector, name) in STATE_KEYFRAMES {
        let found: Vec<&Rule> = top_rules(&rules, selector)
            .into_iter()
            .filter(|r| r.value("animation").is_some())
            .collect();
        assert_eq!(found.len(), 1, "{selector} has one animation rule");
        assert_eq!(
            found[0].value("animation"),
            Some(format!("{name} var(--t-medium) var(--ease-enter)").as_str()),
            "{selector} re-enters with its own keyframes"
        );
        names.insert(name);
        let steps: Vec<&Rule> = rules
            .iter()
            .filter(|r| r.at.as_deref() == Some(format!("@keyframes {name}").as_str()))
            .collect();
        assert_eq!(steps.len(), 1, "{name}: one `from` step");
        assert_eq!(steps[0].selectors, ["from"]);
        assert_eq!(
            steps[0].decls,
            [
                ("opacity".to_string(), "0.4".to_string()),
                ("translate".to_string(), "0 4px".to_string())
            ],
            "{name}: opacity 0.4 and a 4 px rise to the element's own values"
        );
    }
    assert_eq!(
        names.len(),
        STATE_KEYFRAMES.len(),
        "one name per state class"
    );
}

// ---------------------------------------------------------------------------------------------
// RMO-S5 — reduced motion
// ---------------------------------------------------------------------------------------------

/// RMO-S5 (RMO5, Part B.1 item 5): exactly one reduced-motion block; its universal rule sets
/// `animation: none !important` and `transition: none !important`; nothing in it transitions
/// a transform property or runs longer than `--t-fast`.
#[test]
fn test_rmo_s5_single_reduced_motion_block_removes_motion() {
    let rules = css_rules();
    let blocks: BTreeSet<&str> = rules
        .iter()
        .filter_map(|r| r.at.as_deref())
        .filter(|a| is_reduce_block(a))
        .collect();
    assert_eq!(blocks.len(), 1, "exactly one reduced-motion block");
    let clean = strip_css_comments(&read(STYLE));
    assert_eq!(
        clean.matches("prefers-reduced-motion").count(),
        1,
        "prefers-reduced-motion appears once in style.css"
    );
    let inside: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.at.as_deref().is_some_and(is_reduce_block))
        .collect();
    let universal: Vec<&&Rule> = inside
        .iter()
        .filter(|r| r.selectors == ["*", "*::before", "*::after"])
        .collect();
    assert_eq!(universal.len(), 1, "one universal rule in the block");
    assert_eq!(universal[0].value("animation"), Some("none !important"));
    assert_eq!(universal[0].value("transition"), Some("none !important"));
    for rule in &inside {
        for (prop, value) in &rule.decls {
            if prop == "transition" || prop == "transition-property" {
                let props = if prop == "transition" {
                    transition_properties(value)
                } else {
                    split_top_commas(value)
                };
                for p in props {
                    assert!(
                        !TRANSFORM_FAMILY.contains(&p.as_str()),
                        "reduced motion keeps no transform transition: `{}`",
                        rule.selector_text()
                    );
                }
                for word in css_words(value) {
                    assert!(
                        !word.starts_with("var(--t-") || word == "var(--t-fast)",
                        "reduced motion keeps at most --t-fast: `{value}`"
                    );
                }
            }
            if prop == "animation" || prop == "animation-name" {
                assert!(
                    value.starts_with("none"),
                    "no animation under reduced motion: `{}`",
                    rule.selector_text()
                );
            }
        }
    }
    assert!(
        inside
            .iter()
            .any(|r| r.selector_text() == "button:active:not(:disabled)"
                && r.value("transform") == Some("none")),
        "the press scale is off under reduced motion"
    );
}

// ---------------------------------------------------------------------------------------------
// RMO-S6 — brand kept
// ---------------------------------------------------------------------------------------------

/// RMO-S6 (RMO6, Part B.1 item 6): no new `box-shadow`, gradient, filter, blur or `:has()`; the
/// star turn is keyed on `data-state` and the star turns about its centre on the spring.
#[test]
fn test_rmo_s6_brand_kept_and_star_turn() {
    let rules = css_rules();
    let shadows: BTreeSet<(String, String)> = rules
        .iter()
        .flat_map(|r| {
            r.decls
                .iter()
                .filter(|d| d.0 == "box-shadow")
                .map(move |d| (r.selector_text(), d.1.clone()))
        })
        .collect();
    let base: BTreeSet<(String, String)> = BASE_BOX_SHADOWS
        .iter()
        .map(|(s, v)| ((*s).to_string(), (*v).to_string()))
        .collect();
    assert_eq!(
        shadows, base,
        "the box-shadow declarations are the base set"
    );
    let shadow_count: usize = rules
        .iter()
        .map(|r| r.decls.iter().filter(|d| d.0 == "box-shadow").count())
        .sum();
    assert_eq!(shadow_count, BASE_BOX_SHADOWS.len(), "no box-shadow added");

    let clean = strip_css_comments(&read(STYLE)).to_ascii_lowercase();
    for rule in &rules {
        for (prop, value) in &rule.decls {
            if value.contains("gradient(") {
                assert_eq!(rule.selectors, ["html"], "a gradient only on html: {value}");
                assert!(!value.contains("radial-") && !value.contains("conic-"));
            }
            assert!(
                prop != "filter" && prop != "backdrop-filter" && prop != "-webkit-backdrop-filter",
                "no {prop} in `{}`",
                rule.selector_text()
            );
        }
    }
    for needle in ["blur(", "drop-shadow(", "text-shadow"] {
        assert!(
            !clean.contains(needle),
            "style.css must not contain `{needle}`"
        );
    }
    let has: usize = rules
        .iter()
        .flat_map(|r| r.selectors.iter())
        .filter(|s| s.contains(":has("))
        .count();
    assert_eq!(has, 5, "the five pinned :has() selectors only (RXB-S7)");

    let turn = top_rules(&rules, STAR_TURN_SELECTOR);
    assert_eq!(turn.len(), 1, "one star-turn rule");
    assert_eq!(
        turn[0].decls,
        [("rotate".to_string(), "var(--star-turn)".to_string())]
    );
    let star_turn_uses = rules
        .iter()
        .flat_map(|r| r.decls.iter())
        .filter(|d| d.1.contains("var(--star-turn)"))
        .count();
    assert_eq!(
        star_turn_uses, 1,
        "--star-turn is used by the star-turn rule only"
    );
    let star = top_rules(&rules, ".stat-star");
    assert_eq!(star.len(), 1, "one .stat-star rule");
    assert!(
        matches!(
            star[0].value("transform-origin"),
            Some("center" | "50% 50%")
        ),
        "the star turns about its centre"
    );
    assert_eq!(
        star[0].value("transition"),
        Some("rotate var(--t-spring) var(--ease-spring)"),
        "the star moves between angles on the spring"
    );
    assert!(
        rules
            .iter()
            .flat_map(|r| r.decls.iter())
            .filter(|d| d.0 == "rotate")
            .count()
            == 1,
        "only the star-turn rule sets `rotate`"
    );
}

// ---------------------------------------------------------------------------------------------
// RMO-S7 — entries
// ---------------------------------------------------------------------------------------------

/// RMO-S7 (RMO7, Part B.1 item 7): one top-level `@starting-style` block setting only opacity
/// and transform on elements revealed from `[hidden]`, each with a matching transition and no
/// rest transform; no `allow-discrete`, so every hide is instant.
#[test]
fn test_rmo_s7_starting_style_entries_for_revealed_sections() {
    let css = read(STYLE);
    let clean = strip_css_comments(&css);
    assert_eq!(
        clean.matches("@starting-style").count(),
        1,
        "one @starting-style block"
    );
    for needle in ["allow-discrete", "transition-behavior"] {
        assert!(!clean.contains(needle), "exits stay instant: no `{needle}`");
    }
    let rules = css_rules();
    let entries: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.at.as_deref() == Some("@starting-style"))
        .collect();
    let mut targeted = BTreeSet::new();
    for rule in &entries {
        for selector in &rule.selectors {
            assert!(
                ENTRY_SELECTORS.contains(&selector.as_str()),
                "@starting-style targets `{selector}`, not a revealed element"
            );
            targeted.insert(selector.clone());
        }
        for (prop, _) in &rule.decls {
            assert!(
                matches!(prop.as_str(), "opacity" | "transform" | "translate"),
                "@starting-style sets `{prop}` in `{}`",
                rule.selector_text()
            );
        }
        // Each entry has a base transition covering what it sets, and no rest transform.
        for selector in &rule.selectors {
            let base: Vec<&Rule> = rules
                .iter()
                .filter(|r| r.at.is_none() && r.selectors.iter().any(|s| s == selector))
                .collect();
            let props: BTreeSet<String> = base
                .iter()
                .filter_map(|r| r.value("transition"))
                .flat_map(transition_properties)
                .collect();
            for (prop, _) in &rule.decls {
                assert!(
                    props.contains(prop),
                    "`{selector}` enters on `{prop}` but does not transition it: {props:?}"
                );
            }
            for r in &base {
                if ENTRY_SELECTORS[..5].contains(&selector.as_str()) {
                    assert!(
                        r.value("transform").is_none() && r.value("translate").is_none(),
                        "`{selector}` keeps no rest transform (the fixed camera stage)"
                    );
                }
            }
        }
    }
    let all: BTreeSet<String> = ENTRY_SELECTORS.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(targeted, all, "every revealed element enters");

    // The targets are revealed from [hidden].
    let html = read(INDEX);
    for id in [
        "login",
        "alerts",
        "push",
        "enroll",
        "alerts-coverage",
        "push-enable",
        "push-test",
        "push-disable",
    ] {
        let at = html
            .find(&format!("id=\"{id}\""))
            .unwrap_or_else(|| panic!("index.html #{id}"));
        let tag_end = at + html[at..].find('>').unwrap();
        assert!(
            html[at..tag_end].contains(" hidden"),
            "#{id} starts hidden in index.html"
        );
    }
    let app = read(APP_JS);
    for (class_line, hidden_line) in [
        (
            "section.className = \"card camera\";",
            "section.hidden = true;",
        ),
        (
            "section.className = \"card tour\";",
            "section.hidden = true;",
        ),
        (
            "offlineMonitor.className = \"banner banner-warn\";",
            "offlineMonitor.hidden = true;",
        ),
    ] {
        let at = app
            .find(class_line)
            .unwrap_or_else(|| panic!("app.js {class_line}"));
        assert!(
            app[at..].find(hidden_line).is_some_and(|d| d < 200),
            "{class_line} is followed by {hidden_line}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// RMO-S8 — app.js hooks
// ---------------------------------------------------------------------------------------------

/// RMO-S8 (RMO8, Part B.1 item 8): no script animation; one `document.startViewTransition(`
/// call, inside `withViewTransition`, which is the helper of `Docs/UI_MOTION.md` §6 verbatim;
/// it is called only for the CSS-overlay full-screen path, after `cameraFull` is written, with
/// a callback that reads the flag; the view-ending exits apply the classes directly.
#[test]
fn test_rmo_s8_app_hooks_and_guarded_view_transition() {
    let app = read(APP_JS);
    for forbidden in [
        ".animate(",
        ".style.",
        "cssText",
        "setAttribute(\"style\"",
        "getAnimations",
        "requestAnimationFrame",
        "element.animate",
        "Animation(",
        "KeyframeEffect",
        "animationend",
        "transitionend",
        "gsap",
        "anime(",
        "lottie",
        "Velocity",
    ] {
        assert!(
            !app.contains(forbidden),
            "app.js must not contain `{forbidden}`"
        );
    }
    let code = strip_js_comments(&app);
    assert_eq!(
        code.matches("startViewTransition(").count(),
        1,
        "one startViewTransition call"
    );
    let helper = function_text(&code, "withViewTransition");
    assert_eq!(
        code.matches("startViewTransition").count(),
        helper.matches("startViewTransition").count(),
        "startViewTransition is only named inside withViewTransition"
    );
    let doc = read(MOTION_DOC);
    let canonical = strip_js_comments(&doc_js_function(&doc, "withViewTransition"));
    assert_eq!(
        no_whitespace(helper),
        no_whitespace(&canonical),
        "withViewTransition is the helper of Docs/UI_MOTION.md §6"
    );
    let call = helper
        .find("document.startViewTransition(apply)")
        .expect("the call");
    let guard = helper
        .find("typeof document.startViewTransition !== \"function\"")
        .expect("feature detection");
    let reduce = helper
        .find("window.matchMedia(\"(prefers-reduced-motion: reduce)\").matches")
        .expect("reduced-motion check");
    let try_at = helper.find("try {").expect("try");
    let catch_at = helper.rfind("} catch (_) {").expect("catch");
    assert!(guard < call && reduce < call && try_at < call && call < catch_at);
    assert!(
        no_whitespace(&helper[catch_at..]).starts_with("}catch(_){apply();}"),
        "a throw applies the change directly"
    );
    for promise in [
        "transition.ready",
        "transition.updateCallbackDone",
        "transition.finished",
    ] {
        assert!(helper.contains(promise), "{promise} gets a catch");
    }
    assert!(
        helper.contains("p.catch(function () {})"),
        "the promises are never unhandled"
    );

    let calls = call_argument_texts(&code, "withViewTransition");
    assert_eq!(
        calls,
        ["apply", "applyCameraFull", "applyCameraFull"],
        "def + two calls"
    );
    let apply = function_text(&code, "applyCameraFull");
    assert_eq!(
        no_whitespace(apply),
        no_whitespace(CANONICAL_APPLY_CAMERA_FULL),
        "the update callback reads the current cameraFull flag"
    );
    let available = function_text(&code, "elementFullscreenAvailable");
    assert!(
        no_whitespace(available).contains(
            "returntypeof(stage.requestFullscreen||stage.webkitRequestFullscreen)===\"function\";"
        ),
        "element fullscreen is feature-detected: {available}"
    );

    let enter = no_whitespace(function_text(&code, "enterCameraFullscreen"));
    let written = enter
        .find("cameraFull=true;")
        .expect("enter writes the flag");
    let branch = enter
        .find(
            "if(!elementFullscreenAvailable(c.stage)){withViewTransition(applyCameraFull);return;}",
        )
        .expect("the overlay-only branch runs the View Transition");
    let direct = enter
        .find("applyCameraFull();")
        .expect("direct apply otherwise");
    let request = enter
        .find("request.call(c.stage)")
        .expect("element fullscreen request");
    assert!(written < branch && branch < direct && direct < request);

    let exit = no_whitespace(function_text(&code, "exitCameraFullscreen"));
    assert!(
        exit.starts_with("functionexitCameraFullscreen(animate){"),
        "{exit}"
    );
    let written = exit
        .find("cameraFull=false;")
        .expect("exit writes the flag");
    let branch = exit
        .find("if(animate===true&&!elementFullscreenAvailable(c.stage)){withViewTransition(applyCameraFull);return;}")
        .expect("only an animated overlay exit runs the View Transition");
    let direct = exit
        .find("applyCameraFull();")
        .expect("direct apply otherwise");
    assert!(written < branch && branch < direct);

    let exits = call_argument_texts(&code, "exitCameraFullscreen");
    for (function, argument) in [
        ("endCameraView", ""),
        ("cameraFullscreenChanged", ""),
        ("toggleCameraFullscreen", "true"),
    ] {
        let text = function_text(&code, function);
        assert_eq!(
            call_argument_texts(text, "exitCameraFullscreen"),
            [argument],
            "{function} calls exitCameraFullscreen({argument})"
        );
    }
    assert_eq!(
        exits.iter().filter(|a| a.as_str() == "true").count(),
        3,
        "the tap toggle, the close button and the Escape key animate the overlay exit: {exits:?}"
    );
    assert_eq!(
        exits.len(),
        6,
        "the definition, two direct exits and three animated ones: {exits:?}"
    );
    assert!(
        no_whitespace(function_text(&code, "buildCameraCard")).contains(
            "closeButton.addEventListener(\"click\",function(){exitCameraFullscreen(true);});"
        ),
        "the close button animates the overlay exit"
    );
    assert!(
        no_whitespace(&code)
            .contains("if(event.key===\"Escape\"&&cameraFull){exitCameraFullscreen(true);}"),
        "Escape leaves the overlay with the transition"
    );
}

// ---------------------------------------------------------------------------------------------
// RMO-S9 — security truth first
// ---------------------------------------------------------------------------------------------

/// RMO-S9 (RMO9, Part B.1 item 9): `render()` writes the state word, its class, both
/// `data-state` attributes and the `disabled` flags before any motion hook; `requestUnlock`
/// never writes a state before the answer and keeps `window.confirm` then the passkey; motion
/// adds no timer and no animation-end listener.
#[test]
fn test_rmo_s9_security_truth_first() {
    let code = strip_js_comments(&read(APP_JS));
    let render = function_text(&code, "render");
    let first_motion = [
        "classList",
        "syncPending(",
        "confirmLock(",
        "confirmUnlock(",
    ]
    .iter()
    .filter_map(|n| render.find(n))
    .min()
    .expect("render confirms requests");
    for write in [
        "setText(stateNode, label);",
        "stateNode.dataset.state = state;",
        "stateNode.className = ",
        "statTile.dataset.state = ",
        "lockButton.disabled = ",
        "unlockButton.disabled = ",
    ] {
        let at = render
            .find(write)
            .unwrap_or_else(|| panic!("render writes {write}"));
        assert!(
            at < first_motion,
            "render writes `{write}` before any motion hook"
        );
    }
    assert!(
        !render.contains("classList"),
        "render toggles no motion class itself"
    );

    let unlock = function_text(&code, "requestUnlock");
    for forbidden in [
        "\"Unlocked\"",
        "className",
        "dataset.state",
        "state-",
        "classList",
    ] {
        assert!(
            !unlock.contains(forbidden),
            "requestUnlock writes no state before the answer: `{forbidden}`"
        );
    }
    let confirm = unlock.find("window.confirm(").expect("native confirmation");
    let options = unlock
        .find("postJson(UNLOCK_OPTIONS_PATH")
        .expect("options request");
    let passkey = unlock.find("getAssertion(").expect("passkey");
    let pending = unlock
        .find("unlockRequestedAt = Date.now();")
        .expect("pending flag");
    let request = unlock.find("fetch(UNLOCK_PATH").expect("unlock request");
    assert!(
        confirm < options && options < passkey && passkey < pending && pending < request,
        "confirm, then the passkey, then the pending flag, then the request"
    );

    let delays: Vec<String> = call_argument_texts(&code, "setTimeout")
        .iter()
        .map(|args| {
            let parts = split_top_commas(args);
            parts.last().cloned().unwrap_or_default()
        })
        .collect();
    let mut sorted = delays.clone();
    sorted.sort();
    let mut expected: Vec<String> = TIMEOUT_DELAYS.iter().map(|s| (*s).to_string()).collect();
    expected.sort();
    assert_eq!(sorted, expected, "motion adds no timer: {delays:?}");
    for (name, value) in [
        ("LOCK_CONFIRM_UI_MS", "5000"),
        ("UNLOCK_CONFIRM_UI_MS", "5000"),
    ] {
        assert!(
            code.contains(&format!("const {name} = {value};")),
            "{name} stays {value} ms"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// RMO-S10 — pending bound
// ---------------------------------------------------------------------------------------------

/// RMO-S10 (RMO10, Part B.1 item 10): `is-pending` is written only by `syncPending()`, which
/// reads the two request flags; every write of a flag is followed by `syncPending();`, so the
/// star starts with the request and stops at its confirmation, failure or 5 s deadline; the
/// CSS draws the button star with `clip-path: path()` and `currentColor` without moving the label.
#[test]
fn test_rmo_s10_pending_star_follows_the_request_flags() {
    let code = strip_js_comments(&read(APP_JS));
    let sync = function_text(&code, "syncPending");
    assert_eq!(
        no_whitespace(sync),
        no_whitespace(CANONICAL_SYNC_PENDING),
        "syncPending toggles is-pending from the flags only"
    );
    assert_eq!(
        code.matches("\"is-pending\"").count(),
        sync.matches("\"is-pending\"").count(),
        "only syncPending writes is-pending"
    );
    for flag in ["lockRequestedAt", "unlockRequestedAt"] {
        let needle = format!("{flag} = ");
        let mut sets = 0usize;
        let mut resets = 0usize;
        for (at, _) in code.match_indices(&needle) {
            let before = code[..at].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue; // `unlockRequestedAt` when scanning `lockRequestedAt`.
            }
            if code[..at].ends_with("let ") {
                assert!(
                    code[at..].starts_with(&format!("{flag} = 0;")),
                    "{flag} starts at 0"
                );
                continue;
            }
            let end = at + code[at..].find(';').unwrap();
            let statement = &code[at..=end];
            match statement {
                s if s == format!("{flag} = 0;") => resets += 1,
                s if s == format!("{flag} = Date.now();") => sets += 1,
                other => panic!("unexpected write `{other}`"),
            }
            assert!(
                code[end + 1..].trim_start().starts_with("syncPending();"),
                "`{statement}` is followed by syncPending();"
            );
        }
        assert_eq!(sets, 1, "{flag} is set once (the request)");
        let minimum = if flag == "lockRequestedAt" { 4 } else { 3 };
        assert!(resets >= minimum, "{flag} resets found: {resets}");
    }
    for (function, flag) in [
        ("confirmLock", "lockRequestedAt"),
        ("confirmUnlock", "unlockRequestedAt"),
        ("unlockFailed", "unlockRequestedAt"),
    ] {
        assert!(
            no_whitespace(function_text(&code, function))
                .contains(&format!("{flag}=0;syncPending();")),
            "{function} clears the pending star"
        );
    }
    let lock = no_whitespace(function_text(&code, "requestLock"));
    assert_eq!(
        lock.matches("lockRequestedAt=0;syncPending();").count(),
        3,
        "requestLock clears the star on the deadline, a refusal and a failure"
    );
    let unlock = no_whitespace(function_text(&code, "requestUnlock"));
    assert!(
        unlock.contains("unlockRequestedAt=0;syncPending();"),
        "the unlock deadline clears the star"
    );

    let rules = css_rules();
    let after = top_rules(&rules, "button.is-pending::after");
    assert_eq!(after.len(), 1, "one pending star rule");
    let star = after[0];
    for (prop, value) in [
        ("content", "\"\""),
        ("position", "absolute"),
        ("width", "16px"),
        ("height", "16px"),
        ("background", "currentColor"),
        ("clip-path", PENDING_STAR_PATH),
        (
            "animation",
            "soos-pending-turn var(--t-pending) var(--ease-standard) infinite",
        ),
    ] {
        assert_eq!(
            star.value(prop),
            Some(value),
            "button.is-pending::after {prop}"
        );
    }
    let host = top_rules(&rules, "button.is-pending");
    assert_eq!(host.len(), 1, "one pending button rule");
    assert_eq!(
        host[0].decls,
        [("position".to_string(), "relative".to_string())]
    );
    let tile = top_rules(&rules, ".stat-tile.is-pending .stat-star");
    assert_eq!(tile.len(), 1, "one pending tile star rule");
    assert_eq!(
        tile[0].decls,
        [(
            "animation".to_string(),
            "soos-pending-turn var(--t-pending) var(--ease-standard) infinite".to_string()
        )]
    );
    let turn: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.at.as_deref() == Some("@keyframes soos-pending-turn"))
        .collect();
    assert_eq!(turn.len(), 2, "from and to");
    assert_eq!(
        turn[0].decls,
        [("transform".to_string(), "rotate(0deg)".to_string())]
    );
    assert_eq!(
        turn[1].decls,
        [("transform".to_string(), "rotate(90deg)".to_string())]
    );
}

// ---------------------------------------------------------------------------------------------
// RMO-S11 — stagger
// ---------------------------------------------------------------------------------------------

/// RMO-S11 (RMO11, Part B.1 item 11): `stagger-once` is set on the alert history and device
/// lists for the first render after their section becomes visible and removed on the next;
/// the CSS staggers the first six items (at most 5 × `--t-stagger` = 150 ms of delay).
#[test]
fn test_rmo_s11_stagger_on_first_render_only() {
    let code = strip_js_comments(&read(APP_JS));
    assert_eq!(
        code.matches("\"stagger-once\"").count(),
        2,
        "two list hooks"
    );
    for (function, section, list) in [
        ("renderAlerts", "alertsSection", "alertsHistory"),
        ("renderPush", "pushSection", "pushDevices"),
    ] {
        let text = no_whitespace(function_text(&code, function));
        let read_at = text
            .find(&format!("constfirstShow={section}.hidden;"))
            .unwrap_or_else(|| panic!("{function} reads the hidden flag first"));
        let reveal = text
            .find(&format!("{section}.hidden=false;"))
            .unwrap_or_else(|| panic!("{function} reveals the section"));
        let toggle = text
            .find(&format!(
                "{list}.classList.toggle(\"stagger-once\",firstShow);"
            ))
            .unwrap_or_else(|| panic!("{function} toggles stagger-once from firstShow"));
        assert!(
            read_at < reveal && reveal < toggle,
            "{function}: read, reveal, toggle"
        );
        assert!(
            !text[reveal..toggle].contains("return"),
            "{function}: no early return between the reveal and the toggle"
        );
        let clear = text
            .find(&format!("while({list}.firstChild)"))
            .unwrap_or_else(|| panic!("{function} rebuilds the list"));
        assert!(
            toggle < clear,
            "{function}: the class is set before the items are built"
        );
    }

    let rules = css_rules();
    let stagger: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.selectors.iter().any(|s| s.contains(".stagger-once")))
        .collect();
    let base = top_rules(&rules, ".stagger-once li");
    assert_eq!(base.len(), 1, "one stagger animation rule");
    assert_eq!(
        base[0].decls,
        [(
            "animation".to_string(),
            "soos-rise-in var(--t-medium) var(--ease-enter) backwards".to_string()
        )]
    );
    let expected = [
        (".stagger-once li:nth-child(2)", "var(--t-stagger)"),
        (
            ".stagger-once li:nth-child(3)",
            "calc(var(--t-stagger) * 2)",
        ),
        (
            ".stagger-once li:nth-child(4)",
            "calc(var(--t-stagger) * 3)",
        ),
        (
            ".stagger-once li:nth-child(5)",
            "calc(var(--t-stagger) * 4)",
        ),
        (
            ".stagger-once li:nth-child(n+6)",
            "calc(var(--t-stagger) * 5)",
        ),
    ];
    assert_eq!(stagger.len(), expected.len() + 1, "no other stagger rule");
    for (selector, delay) in expected {
        let found = top_rules(&rules, selector);
        assert_eq!(found.len(), 1, "{selector}");
        assert_eq!(
            found[0].decls,
            [("animation-delay".to_string(), delay.to_string())],
            "{selector} waits {delay}"
        );
    }
    let rise: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.at.as_deref() == Some("@keyframes soos-rise-in"))
        .collect();
    assert_eq!(rise.len(), 1, "one `from` step");
    assert_eq!(
        rise[0].decls,
        [
            ("opacity".to_string(), "0".to_string()),
            ("translate".to_string(), "0 var(--rise)".to_string())
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// RMO-S12 — unchanged surface
// ---------------------------------------------------------------------------------------------

/// RMO-S12 (RMO12, Part B.1 item 12): the CSP, the eleven-file asset set, every element id and
/// button label are those of the base tree; motion hooks live in `style.css` and `app.js` only.
#[test]
fn test_rmo_s12_csp_assets_ids_and_labels_unchanged() {
    let http = read(HTTP_RS);
    let at = http
        .find("Content-Security-Policy: ")
        .expect("the CSP header");
    let after = &http[at + "Content-Security-Policy: ".len()..];
    let raw = &after[..after.find("\\r\\n").expect("end of the CSP line")];
    let csp = squash(&raw.replace("\\\n", " "));
    assert_eq!(csp, CSP, "the CSP is unchanged");
    assert_eq!(
        http.matches("Content-Security-Policy:").count(),
        1,
        "one CSP header"
    );

    let mut files: Vec<String> = fs::read_dir(workspace_root().join(ASSETS_DIR))
        .expect("assets directory")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    let mut expected: Vec<String> = ASSET_FILES.iter().map(|s| (*s).to_string()).collect();
    expected.sort();
    assert_eq!(files, expected, "the eleven assets, no new file");
    let assets = read(ASSETS_RS);
    assert_eq!(
        assets.matches("include_bytes!(\"../assets/").count(),
        ASSET_FILES.len(),
        "eleven embedded assets"
    );

    let html = read(INDEX);
    let mut ids: Vec<String> = html
        .match_indices(" id=\"")
        .map(|(at, _)| {
            let start = at + " id=\"".len();
            html[start..start + html[start..].find('"').unwrap()].to_string()
        })
        .collect();
    ids.sort();
    let mut pinned: Vec<String> = ELEMENT_IDS.iter().map(|s| (*s).to_string()).collect();
    pinned.sort();
    assert_eq!(ids, pinned, "the element ids are unchanged");
    for (id, label) in BUTTON_LABELS {
        let at = html.find(&format!("id=\"{id}\"")).expect("button id");
        let open_end = at + html[at..].find('>').unwrap() + 1;
        let close = open_end + html[open_end..].find("</button>").expect("button end");
        let text = html[open_end..close].replace("&#39;", "'");
        assert_eq!(text, label.replace("&#39;", "'"), "#{id} keeps its label");
    }
    let lower = html.to_ascii_lowercase();
    for needle in [
        "is-pending",
        "stagger-once",
        "view-transition",
        "<style",
        " style=",
    ] {
        assert!(!lower.contains(needle), "index.html carries no `{needle}`");
    }
    let app = read(APP_JS);
    for needle in [".id = ", "setAttribute(\"id\"", "setAttribute('id'"] {
        assert!(
            !app.contains(needle),
            "app.js creates no element id: `{needle}`"
        );
    }
}
