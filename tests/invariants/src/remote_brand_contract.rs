//! Static contracts of the soos brand applied to the `soos-remote` web app (ADR 2026-10-06
//! "soos Brand Direction Applied to the `soos-remote` Web App", architect spec
//! `AI/architect_spec_remote_brand.md` §9 tests 63–74, matrix RMC76–RMC87):
//!
//! - RMC-S44 the page tokens equal the desktop GUI theme (`crates/gui/src/theme.rs`): a pinned
//!   copy of the 25 palette colors and six metrics now, a live parse of that file whenever it
//!   is in the tree;
//! - RMC-S45 every color of `style.css` comes from the palette or a recomputable dark tint;
//!   components use role and scale tokens only;
//! - RMC-S46 light default and dark variant through `prefers-color-scheme`;
//! - RMC-S47 WCAG AA text contrast and 3:1 control boundaries, computed from the tokens, and
//!   the selector-to-role bindings that make the computed pairs the real ones;
//! - RMC-S48 focus ring, touch targets, safe areas, reduced motion, zoom kept;
//! - RMC-S49 no inline style, script, event handler or `data:` URI;
//! - RMC-S50 every id, label and initial attribute `app.js` and the docs rely on is kept;
//!   the push card has no switch;
//! - RMC-S51 the header band carries the SOOS wordmark;
//! - RMC-S52 `icon.svg` is the owner's star mark, the touch icon an opaque 180 x 180 RGB PNG;
//! - RMC-S53 manifest and meta colors come from the palette;
//! - RMC-S54 system fonts only and the fixed seven-file asset set;
//! - RMC-S55 the design is documented and the ADR exists.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use crate::remote_companion_contract::{exists, read, workspace_root};

const INDEX: &str = "crates/remote/assets/index.html";
const STYLE: &str = "crates/remote/assets/style.css";
const APP_JS: &str = "crates/remote/assets/app.js";
const ICON: &str = "crates/remote/assets/icon.svg";
const TOUCH_ICON: &str = "crates/remote/assets/apple-touch-icon.png";
const MANIFEST: &str = "crates/remote/assets/manifest.webmanifest";
const ASSETS_DIR: &str = "crates/remote/assets";
const ASSETS_RS: &str = "crates/remote/src/assets.rs";
const GUI_THEME: &str = "crates/gui/src/theme.rs";
const REMOTE_DOC: &str = "Docs/REMOTE_COMPANION.md";
const DECISIONS: &str = "AI/DECISIONS.md";
const TESTER_CONTRACT: &str = "AI/tester_contract_brand.md";

const ADR_TITLE: &str = "soos Brand Direction Applied to the `soos-remote` Web App";

// ---------------------------------------------------------------------------------------------
// Pinned brand data
// ---------------------------------------------------------------------------------------------

/// `(css token, theme.rs constant, value)`, copied from `origin/main:crates/gui/src/theme.rs`
/// at `47ab53e`.
const PALETTE: [(&str, &str, u32); 25] = [
    ("--blue", "BLUE", 0x0047BB),
    ("--blue-hover", "BLUE_HOVER", 0x1D5CC3),
    ("--blue-pressed", "BLUE_PRESSED", 0x003A99),
    ("--pale", "PALE", 0xEDF1FF),
    ("--pale-2", "PALE_2", 0xE4EAFD),
    ("--pale-3", "PALE_3", 0xDCE4FC),
    ("--pale-4", "PALE_4", 0xC9D6FA),
    ("--line", "LINE", 0xC9D0E1),
    ("--ink", "INK", 0x101820),
    ("--ink-muted", "INK_MUTED", 0x5A6270),
    ("--ink-weak", "INK_WEAK", 0x7A818E),
    ("--pink", "PINK", 0xE59BDC),
    ("--white", "WHITE", 0xFFFFFF),
    ("--success", "SUCCESS", 0x29B53B),
    ("--success-toggle", "SUCCESS_TOGGLE", 0x46BE82),
    ("--success-bg", "SUCCESS_BG", 0xE6F6EA),
    ("--success-text", "SUCCESS_TEXT", 0x146C2A),
    ("--danger", "DANGER", 0xD92D45),
    ("--danger-hover", "DANGER_HOVER", 0xB81F35),
    ("--danger-bg", "DANGER_BG", 0xFDE8EB),
    ("--danger-text", "DANGER_TEXT", 0x8A1426),
    ("--warn", "WARN", 0xF59E0B),
    ("--warn-border", "WARN_BORDER", 0xF5B547),
    ("--warn-bg", "WARN_BG", 0xFFF4E0),
    ("--warn-text", "WARN_TEXT", 0x8A4B00),
];

/// `(css token, theme.rs constant, px)`: the GUI-named numeric scale tokens.
const METRICS: [(&str, &str, u64); 6] = [
    ("--r-body", "BODY_TOP_RADIUS", 20),
    ("--r-card", "R_CARD", 20),
    ("--r-table", "R_TABLE", 14),
    ("--r-input", "R_INPUT", 10),
    ("--stroke-card", "STROKE_CARD", 2),
    ("--gap-card", "GAP_CARD", 14),
];

/// `(css token, a, b, n percent, expected value)`: `mix(a, b, n)` (spec §2.2b).
const DARK_TINTS: [(&str, &str, &str, u32, u32); 14] = [
    ("--ink-2", "--ink", "--pale", 5, 0x1B232B),
    ("--ink-3", "--ink", "--pale", 10, 0x262E36),
    ("--ink-4", "--ink", "--pale", 16, 0x333B44),
    ("--ink-line", "--ink", "--pale", 22, 0x414851),
    ("--pale-muted", "--pale", "--ink", 30, 0xABB0BC),
    ("--pale-weak", "--pale", "--ink", 50, 0x7F8590),
    ("--blue-edge", "--blue", "--pale", 30, 0x477ACF),
    ("--blue-soft", "--blue", "--pale", 55, 0x82A5E0),
    ("--danger-soft", "--danger", "--pale", 45, 0xE28599),
    ("--danger-bg-dark", "--ink", "--danger", 18, 0x341C27),
    ("--success-soft", "--success", "--pale", 45, 0x81D093),
    ("--success-bg-dark", "--ink", "--success", 16, 0x143124),
    ("--warn-soft", "--warn", "--pale", 35, 0xF2BB60),
    ("--warn-bg-dark", "--ink", "--warn", 16, 0x352D1D),
];

/// `(role, light value token, dark value token)` (spec §2.3).
const ROLES: [(&str, &str, &str); 55] = [
    ("--page", "--pale", "--ink"),
    ("--band", "--blue", "--blue"),
    ("--on-band", "--pale", "--pale"),
    ("--card", "--pale", "--ink-2"),
    ("--card-border", "--blue", "--blue-edge"),
    ("--text", "--ink", "--pale"),
    ("--text-muted", "--ink-muted", "--pale-muted"),
    ("--text-disabled", "--ink-weak", "--pale-weak"),
    ("--link", "--blue", "--blue-soft"),
    ("--table", "--pale-2", "--ink-3"),
    ("--divider", "--line", "--ink-line"),
    ("--tile", "--blue", "--blue"),
    ("--on-tile", "--pale", "--pale"),
    ("--strip", "--pale", "--ink-2"),
    ("--on-strip", "--ink", "--pale"),
    ("--primary", "--blue", "--blue"),
    ("--on-primary", "--pale", "--pale"),
    ("--primary-hover", "--blue-hover", "--blue-hover"),
    ("--primary-pressed", "--blue-pressed", "--blue-pressed"),
    ("--secondary-fg", "--blue", "--blue-soft"),
    ("--secondary-hover", "--pale-3", "--ink-3"),
    ("--secondary-pressed", "--pale-4", "--ink-4"),
    ("--danger-fill", "--danger", "--danger"),
    ("--danger-fill-hover", "--danger-hover", "--danger-hover"),
    ("--on-danger", "--white", "--white"),
    ("--danger-fg", "--danger-text", "--danger-soft"),
    ("--danger-outline-hover", "--danger-bg", "--danger-bg-dark"),
    ("--disabled-fill", "--line", "--ink-4"),
    ("--disabled-fg", "--ink-weak", "--pale-weak"),
    ("--input-bg", "--white", "--ink"),
    ("--input-border", "--ink-weak", "--pale-weak"),
    ("--placeholder", "--ink-muted", "--pale-muted"),
    ("--focus", "--blue", "--blue-soft"),
    ("--focus-on-blue", "--pink", "--pink"),
    ("--banner-info-bg", "--pale-2", "--ink-3"),
    ("--banner-info-border", "--line", "--ink-line"),
    ("--banner-info-fg", "--ink", "--pale"),
    ("--banner-info-icon", "--blue", "--blue-soft"),
    ("--banner-success-bg", "--success-bg", "--success-bg-dark"),
    ("--banner-success-border", "--success", "--success-soft"),
    ("--banner-success-fg", "--success-text", "--success-soft"),
    ("--banner-success-icon", "--success-text", "--success-soft"),
    ("--banner-warn-bg", "--warn-bg", "--warn-bg-dark"),
    ("--banner-warn-border", "--warn-border", "--warn-soft"),
    ("--banner-warn-fg", "--warn-text", "--warn-soft"),
    ("--banner-warn-icon", "--warn-text", "--warn-soft"),
    ("--banner-danger-bg", "--danger-bg", "--danger-bg-dark"),
    ("--banner-danger-border", "--danger", "--danger-soft"),
    ("--banner-danger-fg", "--danger-text", "--danger-soft"),
    ("--banner-danger-icon", "--danger-text", "--danger-soft"),
    ("--dot-locked", "--success-text", "--success"),
    ("--dot-unlocked", "--warn-text", "--warn"),
    ("--dot-alarm", "--danger-text", "--danger-soft"),
    ("--dot-idle", "--ink-weak", "--pale-weak"),
    ("--star", "--pink", "--pink"),
];

/// Scale tokens of spec §2.4 (allowed inside color-bearing properties, e.g. widths).
const SCALE: [&str; 27] = [
    "--font",
    "--font-mono",
    "--f-display",
    "--f-display-long",
    "--f-title",
    "--f-section",
    "--f-body",
    "--f-button",
    "--f-small",
    "--f-caption",
    "--f-code",
    "--r-body",
    "--r-card",
    "--r-table",
    "--r-banner",
    "--r-input",
    "--r-pill",
    "--stroke-card",
    "--stroke-thin",
    "--gap-card",
    "--pad-card",
    "--gap-item",
    "--gutter",
    "--band-h",
    "--strip-h",
    "--touch",
    "--t-fast",
];

/// The six `d` strings of the owner's `text_logo.svg` (equal to `brand.rs` `WORDMARK`).
const WORDMARK_PATHS: [&str; 6] = [
    "M128.5 32H112.438V16H58.3913C54.7967 16.0001 53.9151 20.9841 57.2932 22.2078L128.5 48V56C128.5 60.4183 124.904 64 120.469 64H0V32H16.0625V48H70.1087C73.7034 48 74.585 43.0159 71.2068 41.7922L0 16V8C0 3.58172 3.59571 0 8.03125 0H128.5V32Z",
    "M514 32H497.938V16H443.891C440.297 16.0001 439.415 20.9841 442.793 22.2078L514 48V56C514 60.4183 510.404 64 505.969 64H385.5V32H401.562V48H455.609C459.203 48 460.085 43.0159 456.707 41.7922L385.5 16V8C385.5 3.58172 389.096 0 393.531 0H514V32Z",
    "M257 48C257 56.8366 249.809 64 240.938 64H144.562C135.691 64 128.5 56.8366 128.5 48H257ZM240.938 0C249.809 0 257 7.16344 257 16V32H240.938V24C240.938 19.5817 237.342 16 232.906 16H152.594C148.158 16 144.562 19.5817 144.562 24V32H128.5V16C128.5 7.16344 135.691 0 144.562 0H240.938Z",
    "M240.938 48H257C257 39.1634 249.809 32 240.938 32V48Z",
    "M257 32H273.062V40C273.062 44.4183 276.658 48 281.094 48H361.406C365.842 48 369.438 44.4183 369.438 40V32H385.5V48C385.5 56.8366 378.309 64 369.438 64H273.062C264.191 64 257 56.8366 257 48V32ZM369.438 0C378.309 0 385.5 7.16344 385.5 16H363.013V16.1594C362.494 16.0545 361.956 16 361.406 16H257C257 7.16344 264.191 0 273.062 0H369.438Z",
    "M273.062 16H257C257 24.8366 264.191 32 273.062 32V16Z",
];

/// The four blue `d` strings of the owner's `Icon_logo.svg`.
const ICON_PATHS: [&str; 4] = [
    "M0 0H254V254H0V0Z",
    "M254 254H508V508H254V254Z",
    "M0 254C140.28 254 254 367.72 254 508H0V254Z",
    "M254 0L508 0V254C367.72 254 254 140.28 254 0Z",
];

/// The two star spikes of `brand.rs` `STAR_SPIKES` (the pink star of the GUI tiles).
const STAR_SPIKE_PATHS: [&str; 2] = [
    "M254 0C254 140.28 367.72 254 508 254L254 254Z",
    "M0 254C140.28 254 254 367.72 254 508V254H0Z",
];

/// Verbatim lines of `origin/main:crates/gui/src/theme.rs` at `47ab53e` (parser self-test).
const THEME_EXCERPT_47AB53E: &str = r"/// Brand blue `#0047BB`: header band, borders, primary buttons, stat tiles.
pub const BLUE: Color32 = Color32::from_rgb(0x00, 0x47, 0xBB);
/// Brilliant White `#EDF1FF`: body, cards, active tab, text on blue.
pub const PALE: Color32 = Color32::from_rgb(0xED, 0xF1, 0xFF);
/// Pink `#E59BDC`: brand accent (star, active reticle, guidance arrows).
pub const PINK: Color32 = Color32::from_rgb(0xE5, 0x9B, 0xDC);
/// Pure white, reserved for text-edit backgrounds and text on danger buttons.
pub const WHITE: Color32 = Color32::WHITE;
/// Danger banner text.
pub const DANGER_TEXT: Color32 = Color32::from_rgb(0x8A, 0x14, 0x26);
/// Eye axis line (semi-transparent light cyan).
pub const EYE_AXIS: Color32 = Color32::from_rgba_premultiplied(132, 180, 188, 200);
/// Corner radius of the pale body under the header.
pub const BODY_TOP_RADIUS: u8 = 20;
/// Gap between stacked cards and tiles.
pub const GAP_CARD: f32 = 14.0;
/// Card corner radius.
pub const R_CARD: u8 = 20;
/// Video corner radius.
pub const R_VIDEO: u8 = 16;
/// Inner table corner radius.
pub const R_TABLE: u8 = 14;
/// Pill button corner radius.
pub const R_BUTTON: u8 = 18;
/// Text input corner radius.
pub const R_INPUT: u8 = 10;
/// Card border width (drawn inside the card).
pub const STROKE_CARD: f32 = 2.0;
";

/// `(id, tag)`: the 29 ids of the page (spec §5), each exactly once on that tag.
const PAGE_IDS: [(&str, &str); 29] = [
    ("login", "section"),
    ("status-card", "section"),
    ("alerts", "section"),
    ("push", "section"),
    ("enroll", "section"),
    ("login-feedback", "p"),
    ("state", "p"),
    ("activity", "p"),
    ("updated", "p"),
    ("feedback", "p"),
    ("alerts-summary", "p"),
    ("alerts-coverage", "p"),
    ("alerts-feedback", "p"),
    ("push-state", "p"),
    ("push-hint", "p"),
    ("push-feedback", "p"),
    ("enroll-feedback", "p"),
    ("alerts-history", "ul"),
    ("push-devices", "ul"),
    ("login-button", "button"),
    ("lock", "button"),
    ("unlock", "button"),
    ("logout", "button"),
    ("alerts-ack", "button"),
    ("push-enable", "button"),
    ("push-test", "button"),
    ("push-disable", "button"),
    ("enroll-button", "button"),
    ("enroll-code", "input"),
];

/// `(button id, exact label)`.
const BUTTON_LABELS: [(&str, &str); 9] = [
    ("login-button", "Sign in with Face ID"),
    ("lock", "Lock now"),
    ("unlock", "Unlock now"),
    ("logout", "Sign out"),
    ("alerts-ack", "Acknowledge"),
    ("push-enable", "Enable notifications"),
    ("push-test", "Send test notification"),
    ("push-disable", "Disable notifications"),
    ("enroll-button", "Add this device's passkey"),
];

const INITIALLY_HIDDEN: [&str; 11] = [
    "login",
    "logout",
    "alerts",
    "alerts-coverage",
    "alerts-ack",
    "push",
    "push-hint",
    "push-enable",
    "push-test",
    "push-disable",
    "enroll",
];

const FEEDBACK_IDS: [&str; 5] = [
    "login-feedback",
    "feedback",
    "alerts-feedback",
    "push-feedback",
    "enroll-feedback",
];

const PUSH_HINT_INNER: &str = "Not yours? Run <code>soos-remote push list</code> and <code>push remove N</code> on the PC, and, if a passkey is not yours either, <code>soos-remote passkeys remove N</code>.";

const ENROLL_CODE_ATTRIBUTES: [(&str, &str); 8] = [
    ("type", "text"),
    ("inputmode", "text"),
    ("autocomplete", "one-time-code"),
    ("autocapitalize", "characters"),
    ("spellcheck", "false"),
    ("maxlength", "11"),
    ("placeholder", "XXXXX-XXXXX"),
    ("aria-label", "Enrollment code"),
];

const ASSET_FILES: [&str; 7] = [
    "index.html",
    "app.js",
    "style.css",
    "sw.js",
    "manifest.webmanifest",
    "icon.svg",
    "apple-touch-icon.png",
];

// ---------------------------------------------------------------------------------------------
// Color arithmetic
// ---------------------------------------------------------------------------------------------

/// Integer mix of two `0xRRGGBB` colors at `n` percent (spec §2.2b): per channel
/// `(a·(100−n) + b·n + 50) / 100`, i.e. rounded half up, no floating point.
fn mix(a: u32, b: u32, n: u32) -> u32 {
    assert!(n <= 100, "mix percentage {n} out of range");
    let channel = |shift: u32| {
        let ca = (a >> shift) & 0xFF;
        let cb = (b >> shift) & 0xFF;
        ((ca * (100 - n) + cb * n + 50) / 100) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// WCAG 2.x relative luminance of a `0xRRGGBB` color (sRGB, 0.04045 threshold).
fn luminance(c: u32) -> f64 {
    let lin = |v: u32| {
        let s = f64::from(v & 0xFF) / 255.0;
        if s <= 0.040_45 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c >> 16) + 0.7152 * lin(c >> 8) + 0.0722 * lin(c)
}

/// WCAG 2.x contrast ratio of two colors.
fn contrast(a: u32, b: u32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn parse_hex6(value: &str) -> Option<u32> {
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

// ---------------------------------------------------------------------------------------------
// theme.rs parser (D4, round 2 F2)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum ThemeValue {
    Color(u32),
    /// A numeric literal: integer part and the digits after the point (empty for integers).
    Number {
        int: u64,
        frac: String,
    },
}

impl ThemeValue {
    fn equals_integer(&self, n: u64) -> bool {
        matches!(self, Self::Number { int, frac } if *int == n && frac.bytes().all(|b| b == b'0'))
    }
}

fn parse_color_byte(text: &str) -> Option<u32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x") {
        if (1..=2).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return u32::from_str_radix(hex, 16).ok();
        }
        return None;
    }
    if !text.is_empty() && text.len() <= 3 && text.bytes().all(|b| b.is_ascii_digit()) {
        return text.parse::<u32>().ok().filter(|v| *v <= 255);
    }
    None
}

/// The value of `pub const <name>: <TYPE> = <EXPR>;` in `src`, for exactly the declaration
/// forms `theme.rs` uses at `47ab53e`; a missing name, a duplicate or any other form is an
/// error naming the constant.
fn theme_constant(src: &str, name: &str) -> Result<ThemeValue, String> {
    let prefix = format!("pub const {name}:");
    let lines: Vec<&str> = src
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with(&prefix))
        .collect();
    let line = match lines.as_slice() {
        [] => return Err(format!("{name}: constant not found")),
        [one] => *one,
        _ => return Err(format!("{name}: declared {} times", lines.len())),
    };
    let rest = &line[prefix.len()..];
    let (ty, expr) = rest
        .split_once('=')
        .ok_or_else(|| format!("{name}: no initializer in `{line}`"))?;
    let ty = ty.trim();
    let expr = expr
        .trim()
        .strip_suffix(';')
        .ok_or_else(|| format!("{name}: declaration does not end with `;`"))?
        .trim();
    let unsupported = || format!("{name}: unsupported declaration form `{line}`");
    match ty {
        "Color32" => {
            if expr == "Color32::WHITE" {
                return Ok(ThemeValue::Color(0xFF_FFFF));
            }
            if expr == "Color32::BLACK" {
                return Ok(ThemeValue::Color(0));
            }
            let args = expr
                .strip_prefix("Color32::from_rgb(")
                .and_then(|a| a.strip_suffix(')'))
                .ok_or_else(unsupported)?;
            let parts: Vec<&str> = args.split(',').collect();
            if parts.len() != 3 {
                return Err(unsupported());
            }
            let mut value = 0u32;
            for part in parts {
                value = (value << 8) | parse_color_byte(part).ok_or_else(unsupported)?;
            }
            Ok(ThemeValue::Color(value))
        }
        "u8" | "u16" | "u32" | "usize" => {
            if expr.is_empty() || !expr.bytes().all(|b| b.is_ascii_digit()) {
                return Err(unsupported());
            }
            let int = expr.parse::<u64>().map_err(|_| unsupported())?;
            Ok(ThemeValue::Number {
                int,
                frac: String::new(),
            })
        }
        "f32" => {
            let (int, frac) = expr.split_once('.').unwrap_or((expr, ""));
            let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
            if !digits(int) || (expr.contains('.') && !digits(frac)) {
                return Err(unsupported());
            }
            Ok(ThemeValue::Number {
                int: int.parse::<u64>().map_err(|_| unsupported())?,
                frac: frac.to_string(),
            })
        }
        _ => Err(unsupported()),
    }
}

// ---------------------------------------------------------------------------------------------
// CSS parsing
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Rule {
    /// `(block index, normalized prelude)` of the enclosing at-rule, if any.
    media: Option<(usize, String)>,
    /// Selector list items, whitespace-normalized.
    selectors: Vec<String>,
    /// `(property, value)`, both trimmed.
    decls: Vec<(String, String)>,
}

impl Rule {
    fn is_top_root(&self) -> bool {
        self.media.is_none() && self.selectors == [":root"]
    }

    fn has_selector(&self, wanted: &[&str]) -> bool {
        self.selectors.iter().any(|s| wanted.contains(&s.as_str()))
    }
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => {
                rest = "";
            }
        }
        out.push(' ');
    }
    out.push_str(rest);
    out
}

fn parse_decls(body: &str) -> Vec<(String, String)> {
    body.split(';')
        .filter_map(|d| {
            let (p, v) = d.split_once(':')?;
            let p = p.trim();
            if p.is_empty() {
                return None;
            }
            Some((p.to_string(), normalize_ws(v)))
        })
        .collect()
}

/// A flat list of style rules (one level of at-rule nesting), comments removed.
fn parse_css(css: &str) -> Result<Vec<Rule>, String> {
    let css = strip_css_comments(css);
    let mut rules = Vec::new();
    let mut media: Option<(usize, String)> = None;
    let mut media_count = 0usize;
    let mut rest = css.as_str();
    loop {
        let open = rest.find('{');
        let close = rest.find('}');
        if let Some(c) = close.filter(|c| open.is_none_or(|o| *c < o)) {
            if media.is_none() || !rest[..c].trim().is_empty() {
                return Err(format!("unbalanced `}}` near `{}`", rest[..c].trim()));
            }
            media = None;
            rest = &rest[c + 1..];
            continue;
        }
        let Some(o) = open else {
            if !rest.trim().is_empty() {
                return Err(format!("trailing CSS text `{}`", rest.trim()));
            }
            if media.is_some() {
                return Err("unclosed at-rule block".to_string());
            }
            return Ok(rules);
        };
        let prelude = normalize_ws(&rest[..o]);
        if prelude.starts_with('@') {
            if media.is_some() {
                return Err(format!("nested at-rule `{prelude}`"));
            }
            media = Some((media_count, prelude));
            media_count += 1;
            rest = &rest[o + 1..];
            continue;
        }
        let body_end = rest[o + 1..]
            .find('}')
            .ok_or_else(|| format!("unclosed rule `{prelude}`"))?;
        let body = &rest[o + 1..o + 1 + body_end];
        if body.contains('{') {
            return Err(format!("nested block inside rule `{prelude}`"));
        }
        rules.push(Rule {
            media: media.clone(),
            selectors: prelude.split(',').map(normalize_ws).collect(),
            decls: parse_decls(body),
        });
        rest = &rest[o + 1 + body_end + 1..];
    }
}

fn css_rules() -> Vec<Rule> {
    parse_css(&read(STYLE)).unwrap_or_else(|e| panic!("style.css does not parse: {e}"))
}

fn top_root_decls(rules: &[Rule]) -> Vec<(String, String)> {
    rules
        .iter()
        .filter(|r| r.is_top_root())
        .flat_map(|r| r.decls.clone())
        .collect()
}

fn media_blocks(rules: &[Rule], needle: &str) -> BTreeSet<usize> {
    rules
        .iter()
        .filter_map(|r| r.media.as_ref())
        .filter(|(_, prelude)| normalize_ws(&prelude.replace(':', ": ")).contains(needle))
        .map(|(i, _)| *i)
        .collect()
}

fn dark_root_decls(rules: &[Rule]) -> Vec<(String, String)> {
    rules
        .iter()
        .filter(|r| {
            r.selectors == [":root"]
                && r.media.as_ref().is_some_and(|(_, p)| {
                    normalize_ws(&p.replace(':', ": ")).contains("prefers-color-scheme: dark")
                })
        })
        .flat_map(|r| r.decls.clone())
        .collect()
}

/// `#` hex literals (3–8 hex digits ending at a non-identifier character), excluding HTML
/// numeric entities (`&#…`).
fn hex_literals(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for (at, _) in text.match_indices('#') {
        if at > 0 && bytes[at - 1] == b'&' {
            continue;
        }
        let run = bytes[at + 1..]
            .iter()
            .take_while(|b| b.is_ascii_hexdigit())
            .count();
        let next = bytes.get(at + 1 + run).copied();
        let ident = next.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if (3..=8).contains(&run) && !ident {
            out.push(text[at..at + 1 + run].to_string());
        }
    }
    out
}

/// Names referenced by `var(--name)` in `value`; a fallback (`var(--a, x)`) is an error.
fn var_refs(value: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(at) = rest.find("var(") {
        let after = &rest[at + 4..];
        let end = after
            .find(')')
            .ok_or_else(|| format!("unclosed var() in `{value}`"))?;
        let inner = after[..end].trim();
        if inner.contains(',') {
            return Err(format!("var() fallback is not allowed: `{value}`"));
        }
        out.push(inner.to_string());
        rest = &after[end + 1..];
    }
    Ok(out)
}

fn without_vars(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(at) = rest.find("var(") {
        out.push_str(&rest[..at]);
        out.push(' ');
        let after = &rest[at + 4..];
        match after.find(')') {
            Some(end) => rest = &after[end + 1..],
            None => rest = "",
        }
    }
    out.push_str(rest);
    out
}

fn without_quoted(value: &str) -> String {
    let mut out = String::new();
    let mut quote: Option<char> = None;
    for ch in value.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None => out.push(ch),
        }
    }
    out
}

fn is_color_property(prop: &str) -> bool {
    matches!(
        prop,
        "color"
            | "background"
            | "background-color"
            | "fill"
            | "stroke"
            | "box-shadow"
            | "caret-color"
            | "accent-color"
            | "text-decoration-color"
            | "-webkit-tap-highlight-color"
    ) || prop.starts_with("border")
        || prop.starts_with("outline")
}

fn is_length_or_number(word: &str) -> bool {
    let w = word.trim_start_matches(['-', '+']);
    let digits_end = w
        .bytes()
        .position(|b| !(b.is_ascii_digit() || b == b'.'))
        .unwrap_or(w.len());
    let (num, unit) = w.split_at(digits_end);
    num.bytes().any(|b| b.is_ascii_digit())
        && [
            "", "px", "em", "rem", "%", "vw", "vh", "dvh", "svh", "lvh", "s", "ms", "deg", "ch",
            "ex",
        ]
        .contains(&unit)
}

const NON_COLOR_KEYWORDS: [&str; 36] = [
    "solid",
    "dashed",
    "dotted",
    "double",
    "none",
    "hidden",
    "inset",
    "outset",
    "auto",
    "inherit",
    "initial",
    "unset",
    "transparent",
    "currentcolor",
    "calc",
    "min",
    "max",
    "clamp",
    "env",
    "safe-area-inset-top",
    "safe-area-inset-bottom",
    "safe-area-inset-left",
    "safe-area-inset-right",
    "padding-box",
    "border-box",
    "content-box",
    "no-repeat",
    "center",
    "top",
    "bottom",
    "left",
    "right",
    "thin",
    "medium",
    "thick",
    "!important",
];

/// The token-by-token rule of spec §2.1 for a color-bearing property outside the token blocks:
/// every `var()` is a role or a scale token, never a palette or tint name; every other word is
/// a length, a number or a non-color keyword; `linear-gradient(` only in `html`, exactly.
fn check_color_value(selectors: &[String], prop: &str, value: &str) -> Result<(), String> {
    let roles: BTreeSet<&str> = ROLES.iter().map(|r| r.0).collect();
    let scale: BTreeSet<&str> = SCALE.iter().copied().collect();
    if value.contains("linear-gradient(") {
        let expected = "linear-gradient(var(--band) 50%, var(--page) 50%)";
        if selectors == ["html"] && value == expected {
            return Ok(());
        }
        return Err(format!(
            "`{prop}: {value}`: linear-gradient is allowed only in `html` as `{expected}`"
        ));
    }
    for name in var_refs(value)? {
        if !roles.contains(name.as_str()) && !scale.contains(name.as_str()) {
            return Err(format!(
                "`{prop}: {value}`: var({name}) is neither a role (§2.3) nor a scale token (§2.4)"
            ));
        }
    }
    let rest = without_vars(value).replace(['(', ')', ',', '/', '*', '+'], " ");
    for word in rest.split_whitespace() {
        if word == "-" || is_length_or_number(word) {
            continue;
        }
        if !NON_COLOR_KEYWORDS.contains(&word.to_ascii_lowercase().as_str()) {
            return Err(format!("`{prop}: {value}`: word `{word}` is not allowed"));
        }
    }
    Ok(())
}

const NAMED_COLORS: [&str; 44] = [
    "aqua",
    "azure",
    "beige",
    "black",
    "blue",
    "brown",
    "coral",
    "crimson",
    "cyan",
    "fuchsia",
    "gold",
    "gray",
    "green",
    "grey",
    "indigo",
    "ivory",
    "khaki",
    "lavender",
    "lime",
    "linen",
    "magenta",
    "maroon",
    "navy",
    "olive",
    "orange",
    "orchid",
    "pink",
    "plum",
    "purple",
    "red",
    "salmon",
    "silver",
    "snow",
    "tan",
    "teal",
    "tomato",
    "turquoise",
    "violet",
    "wheat",
    "white",
    "yellow",
    "whitesmoke",
    "lightgray",
    "darkgray",
];

const COLOR_FUNCTIONS: [&str; 11] = [
    "rgb(",
    "rgba(",
    "hsl(",
    "hsla(",
    "hwb(",
    "lab(",
    "lch(",
    "oklab(",
    "oklch(",
    "color-mix(",
    "color(",
];

fn color_functions_in(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for f in COLOR_FUNCTIONS {
        for (at, _) in text.match_indices(f) {
            let before = at.checked_sub(1).map(|p| bytes[p]);
            if !before.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                out.push(f.to_string());
            }
        }
    }
    out
}

fn named_colors_in(value: &str) -> Vec<String> {
    without_quoted(value)
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|w| NAMED_COLORS.contains(&w.to_ascii_lowercase().as_str()))
        .map(str::to_string)
        .collect()
}

/// Whether `selector` contains the class selector `.class` (not `.class-x`).
fn has_class_selector(selector: &str, class: &str) -> bool {
    let needle = format!(".{class}");
    selector.match_indices(&needle).any(|(at, _)| {
        !selector[at + needle.len()..]
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

fn px_value(value: &str) -> Option<u32> {
    value.strip_suffix("px")?.trim().parse::<u32>().ok()
}

// ---------------------------------------------------------------------------------------------
// HTML parsing
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Tag {
    name: String,
    attrs: Vec<(String, Option<String>)>,
    /// Byte offset of the byte after `>`.
    end: usize,
}

impl Tag {
    fn attr(&self, name: &str) -> Option<Option<&str>> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_deref())
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.attr(name).flatten()
    }

    fn classes(&self) -> Vec<&str> {
        self.value("class")
            .map(|c| c.split_whitespace().collect())
            .unwrap_or_default()
    }
}

fn parse_attrs(inner: &str) -> Vec<(String, Option<String>)> {
    let bytes = inner.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if start == i {
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }
        let name = inner[start..i].to_ascii_lowercase();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            let value = if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                let vstart = i + 1;
                let vend = bytes[vstart..]
                    .iter()
                    .position(|&b| b == q)
                    .map_or(bytes.len(), |p| vstart + p);
                i = (vend + 1).min(bytes.len());
                inner[vstart..vend].to_string()
            } else {
                let vstart = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                inner[vstart..i].to_string()
            };
            out.push((name, Some(value)));
        } else {
            out.push((name, None));
        }
    }
    out
}

/// Every opening (or self-closing) tag of `html`, comments and doctype skipped.
fn open_tags(html: &str) -> Vec<Tag> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = html[i..].find('<') {
        let start = i + p;
        if html[start..].starts_with("<!--") {
            i = html[start..]
                .find("-->")
                .map_or(html.len(), |e| start + e + 3);
            continue;
        }
        // Find the closing `>` outside quotes.
        let mut j = start + 1;
        let mut quote: Option<u8> = None;
        while j < bytes.len() {
            match quote {
                Some(q) if bytes[j] == q => quote = None,
                Some(_) => {}
                None if bytes[j] == b'"' || bytes[j] == b'\'' => quote = Some(bytes[j]),
                None if bytes[j] == b'>' => break,
                None => {}
            }
            j += 1;
        }
        let end = (j + 1).min(bytes.len());
        let inner = &html[start + 1..j.min(bytes.len())];
        i = end;
        if inner.starts_with('/') || inner.starts_with('!') {
            continue;
        }
        let name_end = inner
            .find(|c: char| c.is_ascii_whitespace() || c == '/')
            .unwrap_or(inner.len());
        out.push(Tag {
            name: inner[..name_end].to_string(),
            attrs: parse_attrs(&inner[name_end..]),
            end,
        });
    }
    out
}

fn tag_by_id<'a>(tags: &'a [Tag], id: &str) -> Option<&'a Tag> {
    tags.iter().find(|t| t.value("id") == Some(id))
}

/// Inner HTML of the element opened by `tag` (no same-name nesting expected).
fn inner_html<'a>(html: &'a str, tag: &Tag) -> &'a str {
    let close = format!("</{}>", tag.name);
    let end = html[tag.end..]
        .find(&close)
        .map_or(html.len(), |p| tag.end + p);
    &html[tag.end..end]
}

/// The whole `<svg …>…</svg>` segments of `html`.
fn svg_segments(html: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = html[i..].find("<svg") {
        let start = i + p;
        let end = html[start..]
            .find("</svg>")
            .map_or(html.len(), |e| start + e + "</svg>".len());
        out.push((start, &html[start..end]));
        i = end;
    }
    out
}

fn path_ds(svg: &str) -> Vec<String> {
    open_tags(svg)
        .into_iter()
        .filter(|t| t.name == "path")
        .filter_map(|t| t.value("d").map(str::to_string))
        .collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn sorted_strs(v: &[&str]) -> Vec<String> {
    sorted(v.iter().map(|s| (*s).to_string()).collect())
}

// ---------------------------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
struct PngInfo {
    width: u32,
    height: u32,
    depth: u8,
    color_type: u8,
    interlace: u8,
    chunks: Vec<String>,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn png_info(bytes: &[u8]) -> Result<PngInfo, String> {
    const MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 8 || bytes[..8] != MAGIC {
        return Err("not a PNG (magic)".to_string());
    }
    let mut chunks = Vec::new();
    let mut header: Option<(u32, u32, u8, u8, u8)> = None;
    let mut i = 8usize;
    while i < bytes.len() {
        if i + 12 > bytes.len() {
            return Err(format!("truncated chunk header at {i}"));
        }
        let len = be32(&bytes[i..i + 4]) as usize;
        let kind = String::from_utf8_lossy(&bytes[i + 4..i + 8]).to_string();
        let data_end = i + 8 + len;
        if data_end + 4 > bytes.len() {
            return Err(format!("truncated chunk {kind}"));
        }
        if kind == "IHDR" {
            if len != 13 {
                return Err("IHDR length is not 13".to_string());
            }
            let d = &bytes[i + 8..data_end];
            header = Some((be32(&d[0..4]), be32(&d[4..8]), d[8], d[9], d[12]));
        }
        chunks.push(kind);
        i = data_end + 4;
    }
    let (width, height, depth, color_type, interlace) =
        header.ok_or_else(|| "no IHDR chunk".to_string())?;
    Ok(PngInfo {
        width,
        height,
        depth,
        color_type,
        interlace,
        chunks,
    })
}

/// Spec §6.2: 180 x 180, depth 8, color type 2 (RGB), not interlaced, chunks exactly
/// `IHDR`, `IDAT`+, `IEND`.
fn check_touch_icon(bytes: &[u8]) -> Result<(), String> {
    let info = png_info(bytes)?;
    if (info.width, info.height) != (180, 180) {
        return Err(format!(
            "size {}x{} is not 180x180",
            info.width, info.height
        ));
    }
    if info.depth != 8 {
        return Err(format!("bit depth {} is not 8", info.depth));
    }
    if info.color_type != 2 {
        return Err(format!(
            "color type {} is not 2 (opaque RGB)",
            info.color_type
        ));
    }
    if info.interlace != 0 {
        return Err("interlaced".to_string());
    }
    let n = info.chunks.len();
    let ok = n >= 3
        && info.chunks[0] == "IHDR"
        && info.chunks[n - 1] == "IEND"
        && info.chunks[1..n - 1].iter().all(|c| c == "IDAT");
    if !ok {
        return Err(format!(
            "chunks {:?} are not exactly IHDR, IDAT+, IEND",
            info.chunks
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Helper self-tests
// ---------------------------------------------------------------------------------------------

#[test]
fn test_rmc_brand_helper_theme_parser_self_test() {
    let ex = THEME_EXCERPT_47AB53E;
    assert_eq!(theme_constant(ex, "BLUE"), Ok(ThemeValue::Color(0x0047BB)));
    assert_eq!(theme_constant(ex, "WHITE"), Ok(ThemeValue::Color(0xFFFFFF)));
    assert_eq!(theme_constant(ex, "PINK"), Ok(ThemeValue::Color(0xE59BDC)));
    assert_eq!(
        theme_constant(ex, "DANGER_TEXT"),
        Ok(ThemeValue::Color(0x8A1426))
    );
    assert!(theme_constant(ex, "BODY_TOP_RADIUS")
        .unwrap()
        .equals_integer(20));
    assert!(theme_constant(ex, "STROKE_CARD").unwrap().equals_integer(2));
    assert!(theme_constant(ex, "GAP_CARD").unwrap().equals_integer(14));
    assert!(theme_constant(ex, "R_CARD").unwrap().equals_integer(20));
    assert!(theme_constant(ex, "R_TABLE").unwrap().equals_integer(14));
    assert!(theme_constant(ex, "R_INPUT").unwrap().equals_integer(10));
    let axis = theme_constant(ex, "EYE_AXIS").unwrap_err();
    assert!(axis.contains("EYE_AXIS"), "{axis}");
    let missing = theme_constant(ex, "NOT_THERE").unwrap_err();
    assert!(missing.contains("NOT_THERE"), "{missing}");
    let half = theme_constant("pub const R_CARD: f32 = 2.5;", "R_CARD").unwrap();
    assert_eq!(
        half,
        ThemeValue::Number {
            int: 2,
            frac: "5".to_string()
        }
    );
    assert!(!half.equals_integer(20) && !half.equals_integer(2));
    // Decimal bytes, duplicates, expressions and const fn calls.
    assert_eq!(
        theme_constant("pub const X: Color32 = Color32::from_rgb(0, 71, 187);", "X"),
        Ok(ThemeValue::Color(0x0047BB))
    );
    assert!(theme_constant("pub const X: Color32 = Color32::from_rgb(0, 71, 256);", "X").is_err());
    assert!(theme_constant("pub const X: u8 = 1;\npub const X: u8 = 1;", "X").is_err());
    assert!(theme_constant("pub const X: u8 = 10 + 10;", "X").is_err());
    assert!(theme_constant("pub const X: Color32 = make(1);", "X").is_err());
    assert!(theme_constant("pub const X: Color32 = Color32::from_rgb(A, 0, 0);", "X").is_err());
}

#[test]
fn test_rmc_brand_helper_mix_contrast_and_color_rule_self_test() {
    let pal = |n: &str| PALETTE.iter().find(|p| p.0 == n).unwrap().2;
    assert_eq!(mix(pal("--pale"), pal("--ink"), 50), 0x7F8590);
    assert_eq!(mix(pal("--blue"), pal("--pale"), 55), 0x82A5E0);
    assert_eq!(mix(pal("--ink"), pal("--pale"), 0), pal("--ink"));
    assert_eq!(mix(pal("--ink"), pal("--pale"), 100), pal("--pale"));

    let r = contrast(0x101820, 0xEDF1FF);
    assert!((15.86..=15.88).contains(&r), "ink/pale ratio {r}");
    let r = contrast(0xFFFFFF, 0xD92D45);
    assert!((4.75..=4.77).contains(&r), "white/danger ratio {r}");

    let sel = vec![".card".to_string()];
    assert!(check_color_value(
        &sel,
        "border",
        "var(--stroke-card) solid var(--card-border)"
    )
    .is_ok());
    assert!(check_color_value(&sel, "color", "var(--blue)").is_err());
    assert!(check_color_value(
        &sel,
        "box-shadow",
        "inset 0 0 0 var(--stroke-card) var(--ink-2)"
    )
    .is_err());
    assert!(check_color_value(&sel, "color", "red").is_err());
    assert!(check_color_value(&sel, "color", "var(--text, red)").is_err());
    let grad = "linear-gradient(var(--band) 50%, var(--page) 50%)";
    assert!(check_color_value(&["html".to_string()], "background", grad).is_ok());
    assert!(check_color_value(&sel, "background", grad).is_err());

    assert_eq!(
        hex_literals("a { --x: #0047BB; } #feedback #abc, &#39;"),
        ["#0047BB", "#abc"]
    );
    assert_eq!(named_colors_in("1px solid White"), ["White"]);
    assert!(named_colors_in("var(--pink) \"white\" white-space").is_empty());
    assert_eq!(color_functions_in("x: rgba(0,0,0,0)"), ["rgba("]);
    assert!(color_functions_in("background-color: var(--a)").is_empty());

    let rules = parse_css(
        "/* c { x } */ :root { --a: #FFFFFF; }\n@media (prefers-color-scheme: dark) { :root { --a: #000000; } }\nh1, h2  p { color: var(--text); }",
    )
    .unwrap();
    assert_eq!(rules.len(), 3);
    assert!(rules[0].is_top_root());
    assert_eq!(
        dark_root_decls(&rules),
        [("--a".to_string(), "#000000".to_string())]
    );
    assert_eq!(rules[2].selectors, ["h1", "h2 p"]);
    assert!(parse_css("a { b { } }").is_err());
    assert!(has_class_selector(".push .switch", "switch"));
    assert!(has_class_selector("span.switch:checked", "switch"));
    assert!(!has_class_selector(".switch-like", "switch"));
    assert!(parse_css("a { x: y; } }").is_err());
}

#[test]
fn test_rmc_brand_helper_html_self_test() {
    let html = r#"<main aria-live="polite"><button id="go" type="button" class="secondary danger" hidden>Go now</button><input id="c" maxlength="11" aria-hidden="true"></main>"#;
    let tags = open_tags(html);
    let go = tag_by_id(&tags, "go").unwrap();
    assert_eq!(go.name, "button");
    assert_eq!(inner_html(html, go), "Go now");
    assert_eq!(go.attr("hidden"), Some(None));
    assert_eq!(go.classes(), ["secondary", "danger"]);
    let c = tag_by_id(&tags, "c").unwrap();
    assert_eq!(c.name, "input");
    assert_eq!(c.attr("hidden"), None, "aria-hidden is not hidden");
    assert_eq!(c.value("maxlength"), Some("11"));
    assert!(tag_by_id(&tags, "missing").is_none());
}

#[test]
fn test_rmc_brand_helper_png_self_test() {
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }
    fn png(color_type: u8, extra: Option<&[u8; 4]>) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = 180u32.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&180u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
        out.extend(chunk(b"IHDR", &ihdr));
        if let Some(kind) = extra {
            out.extend(chunk(kind, &[0, 0]));
        }
        out.extend(chunk(b"IDAT", &[1, 2, 3]));
        out.extend(chunk(b"IEND", &[]));
        out
    }
    assert_eq!(check_touch_icon(&png(2, None)), Ok(()));
    assert!(check_touch_icon(&png(0, None)).is_err(), "grey");
    assert!(check_touch_icon(&png(6, None)).is_err(), "RGBA");
    assert!(check_touch_icon(&png(2, Some(b"tRNS"))).is_err(), "tRNS");
    assert!(check_touch_icon(&png(2, Some(b"tIME"))).is_err(), "tIME");
    assert!(check_touch_icon(b"GIF89a").is_err());
}

// ---------------------------------------------------------------------------------------------
// Test 63 – RMC-S44
// ---------------------------------------------------------------------------------------------

/// Test 63 (RMC76): the page tokens carry the `theme.rs` values: 25 palette colors and six
/// metrics, pinned now and cross-checked live against `crates/gui/src/theme.rs` whenever it
/// exists in the tree (never skipped once present).
#[test]
fn test_rmc_s44_brand_tokens_match_the_gui_theme() {
    let rules = css_rules();
    let all: Vec<(String, String, bool)> = rules
        .iter()
        .flat_map(|r| {
            let top = r.is_top_root();
            r.decls
                .iter()
                .map(move |(p, v)| (p.clone(), v.clone(), top))
        })
        .collect();
    for (css, theme, value) in PALETTE {
        let decls: Vec<&(String, String, bool)> = all.iter().filter(|d| d.0 == css).collect();
        assert_eq!(
            decls.len(),
            1,
            "style.css must declare {css} (theme.rs {theme}) exactly once, found {}",
            decls.len()
        );
        let (_, v, top) = decls[0];
        assert!(*top, "{css} must be declared in a top-level :root block");
        assert_eq!(
            v,
            &format!("#{value:06X}"),
            "{css} must be #{value:06X} (uppercase), like theme.rs {theme}"
        );
    }
    for (css, theme, px) in METRICS {
        let decls: Vec<&(String, String, bool)> = all.iter().filter(|d| d.0 == css).collect();
        assert_eq!(
            decls.len(),
            1,
            "{css} (theme.rs {theme}) declared exactly once"
        );
        assert!(decls[0].2, "{css} in a top-level :root block");
        assert_eq!(
            decls[0].1,
            format!("{px}px"),
            "{css} must equal theme.rs {theme}"
        );
    }
    if exists(GUI_THEME) {
        let src = read(GUI_THEME);
        for (_, theme, value) in PALETTE {
            let parsed = theme_constant(&src, theme).unwrap_or_else(|e| panic!("{GUI_THEME}: {e}"));
            assert_eq!(
                parsed,
                ThemeValue::Color(value),
                "{GUI_THEME} {theme} differs from the pinned #{value:06X}"
            );
        }
        for (_, theme, px) in METRICS {
            let parsed = theme_constant(&src, theme).unwrap_or_else(|e| panic!("{GUI_THEME}: {e}"));
            assert!(
                parsed.equals_integer(px),
                "{GUI_THEME} {theme} = {parsed:?}, pinned {px}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Test 64 – RMC-S45
// ---------------------------------------------------------------------------------------------

/// Test 64 (RMC77): every color in `style.css` is a palette token or a recomputable dark tint;
/// no functional or named color; components use role and scale tokens only; the page and the
/// icon carry no other color literal.
#[test]
fn test_rmc_s45_style_colors_come_from_the_palette_only() {
    let css = read(STYLE);
    let stripped = strip_css_comments(&css);
    let rules = css_rules();
    let palette: BTreeMap<&str, u32> = PALETTE.iter().map(|p| (p.0, p.2)).collect();
    let tints: BTreeSet<&str> = DARK_TINTS.iter().map(|t| t.0).collect();
    let roles: BTreeSet<&str> = ROLES.iter().map(|r| r.0).collect();

    // Tints: declared once, recomputed in integer arithmetic from the pinned palette.
    for (name, a, b, n, expected) in DARK_TINTS {
        let computed = mix(palette[a], palette[b], n);
        assert_eq!(
            computed, expected,
            "pinned tint {name} disagrees with mix({a}, {b}, {n})"
        );
        let decls: Vec<&(String, String)> = rules
            .iter()
            .filter(|r| r.is_top_root())
            .flat_map(|r| r.decls.iter())
            .filter(|d| d.0 == name)
            .collect();
        let anywhere = rules
            .iter()
            .flat_map(|r| r.decls.iter())
            .filter(|d| d.0 == name)
            .count();
        assert_eq!(
            decls.len(),
            1,
            "{name} declared once in a top-level :root block"
        );
        assert_eq!(anywhere, 1, "{name} declared exactly once in style.css");
        assert_eq!(
            decls[0].1,
            format!("#{computed:06X}"),
            "{name} = mix({a}, {b}, {n})"
        );
    }

    // Hex literals only as the full value of a palette or tint declaration.
    let mut hex_decls = 0usize;
    for rule in &rules {
        for (prop, value) in &rule.decls {
            if hex_literals(value).is_empty() {
                continue;
            }
            hex_decls += 1;
            assert!(
                rule.is_top_root()
                    && (palette.contains_key(prop.as_str()) || tints.contains(prop.as_str()))
                    && parse_hex6(value).is_some(),
                "hex literal outside a palette/tint token: `{}` {{ {prop}: {value} }}",
                rule.selectors.join(", ")
            );
        }
    }
    assert_eq!(
        hex_literals(&stripped).len(),
        hex_decls,
        "every hex literal of style.css must be the value of a token declaration"
    );
    let funcs = color_functions_in(&stripped);
    assert!(
        funcs.is_empty(),
        "functional color notation in style.css: {funcs:?}"
    );

    for rule in &rules {
        let sel = rule.selectors.join(", ");
        for (prop, value) in &rule.decls {
            let named = named_colors_in(value);
            assert!(
                named.is_empty(),
                "named color {named:?} in `{sel} {{ {prop}: {value} }}`"
            );
            let refs = var_refs(value).unwrap_or_else(|e| panic!("`{sel}`: {e}"));
            for name in &refs {
                if palette.contains_key(name.as_str()) || tints.contains(name.as_str()) {
                    assert!(
                        rule.selectors == [":root"] && roles.contains(prop.as_str()),
                        "palette/tint {name} used outside a role declaration: `{sel} {{ {prop}: {value} }}`"
                    );
                }
            }
            if rule.selectors != [":root"] && is_color_property(prop) {
                check_color_value(&rule.selectors, prop, value)
                    .unwrap_or_else(|e| panic!("`{sel}`: {e}"));
            }
        }
    }

    let html = read(INDEX);
    let html_hex = hex_literals(&html);
    assert!(
        html_hex.iter().all(|h| h == "#0047BB"),
        "index.html may only carry #0047BB (theme-color): {html_hex:?}"
    );
    let icon_hex: BTreeSet<String> = hex_literals(&read(ICON)).into_iter().collect();
    let allowed: BTreeSet<String> = ["#EDF1FF".to_string(), "#0047BB".to_string()].into();
    assert_eq!(
        icon_hex, allowed,
        "icon.svg uses exactly #EDF1FF and #0047BB"
    );
}

// ---------------------------------------------------------------------------------------------
// Test 65 – RMC-S46
// ---------------------------------------------------------------------------------------------

/// Test 65 (RMC78): light roles in `:root`, one dark block with every differing role.
#[test]
fn test_rmc_s46_light_default_and_dark_variant() {
    let rules = css_rules();
    let top = top_root_decls(&rules);
    for (role, light, _) in ROLES {
        let found: Vec<&String> = top.iter().filter(|d| d.0 == role).map(|d| &d.1).collect();
        assert_eq!(
            found.len(),
            1,
            "light :root must define {role} exactly once"
        );
        assert_eq!(found[0], &format!("var({light})"), "light {role}");
    }
    assert!(
        top.iter()
            .any(|(p, v)| p == "color-scheme" && v == "light dark"),
        ":root must declare `color-scheme: light dark`"
    );
    assert_eq!(
        media_blocks(&rules, "prefers-color-scheme: dark").len(),
        1,
        "exactly one @media (prefers-color-scheme: dark) block"
    );
    let dark = dark_root_decls(&rules);
    let role_names: BTreeSet<&str> = ROLES.iter().map(|r| r.0).collect();
    for (p, _) in &dark {
        assert!(
            role_names.contains(p.as_str()) || p == "color-scheme",
            "the dark block only redefines role tokens, found {p}"
        );
    }
    for (role, light, dark_value) in ROLES {
        let found: Vec<&String> = dark.iter().filter(|d| d.0 == role).map(|d| &d.1).collect();
        assert!(found.len() <= 1, "dark block defines {role} at most once");
        if light != dark_value {
            assert_eq!(
                found.first().map(|s| s.as_str()),
                Some(format!("var({dark_value})").as_str()),
                "dark {role} must be var({dark_value})"
            );
        } else if let Some(v) = found.first() {
            assert_eq!(*v, &format!("var({dark_value})"), "dark {role}");
        }
    }
    let html = read(INDEX);
    assert!(
        html.contains(r#"<meta name="color-scheme" content="light dark">"#),
        "color-scheme meta kept"
    );
}

// ---------------------------------------------------------------------------------------------
// Test 66 – RMC-S47
// ---------------------------------------------------------------------------------------------

fn resolve_theme(rules: &[Rule], dark: bool) -> BTreeMap<String, u32> {
    let top = top_root_decls(rules);
    let base: BTreeMap<&str, u32> = top
        .iter()
        .filter_map(|(p, v)| parse_hex6(v).map(|h| (p.as_str(), h)))
        .collect();
    let dark_decls = dark_root_decls(rules);
    let mut out = BTreeMap::new();
    for (role, _, _) in ROLES {
        let light_v = top.iter().find(|d| d.0 == role).map(|d| d.1.clone());
        let dark_v = dark_decls.iter().find(|d| d.0 == role).map(|d| d.1.clone());
        let value = if dark { dark_v.or(light_v) } else { light_v }
            .unwrap_or_else(|| panic!("role {role} is not defined"));
        let refs = var_refs(&value).unwrap();
        assert!(
            refs.len() == 1 && value == format!("var({})", refs[0]),
            "role {role} must be exactly var(--token), got `{value}`"
        );
        let hex = base.get(refs[0].as_str()).unwrap_or_else(|| {
            panic!("role {role} refers to {} which is no palette/tint", refs[0])
        });
        out.insert(role.to_string(), *hex);
    }
    out
}

/// `(selector spellings, property, value)`: selector-to-role bindings (round 2, F5).
const BINDINGS: [(&[&str], &str, &str); 17] = [
    (&[".band", "header.band"], "background", "var(--band)"),
    (&[".stat-tile"], "background", "var(--tile)"),
    (&[".stat-tile"], "color", "var(--on-tile)"),
    (&[".stat-strip"], "background", "var(--strip)"),
    (&[".stat-strip"], "color", "var(--on-strip)"),
    (&["button"], "background", "var(--primary)"),
    (&["button"], "color", "var(--on-primary)"),
    (&["button.secondary"], "color", "var(--secondary-fg)"),
    (
        &["button.secondary.danger", "button.danger.secondary"],
        "color",
        "var(--danger-fg)",
    ),
    (
        &[
            ".alerts-summary",
            ".alerts-summary.banner",
            ".banner.alerts-summary",
            "#alerts-summary",
        ],
        "background",
        "var(--banner-info-bg)",
    ),
    (
        &[
            ".alerts-summary",
            ".alerts-summary.banner",
            ".banner.alerts-summary",
            "#alerts-summary",
        ],
        "color",
        "var(--banner-info-fg)",
    ),
    (
        &[
            ".alerts-attention .alerts-summary",
            "#alerts.alerts-attention .alerts-summary",
            ".alerts.alerts-attention .alerts-summary",
        ],
        "background",
        "var(--banner-danger-bg)",
    ),
    (
        &[
            ".alerts-attention .alerts-summary",
            "#alerts.alerts-attention .alerts-summary",
            ".alerts.alerts-attention .alerts-summary",
        ],
        "color",
        "var(--banner-danger-fg)",
    ),
    (
        &[".banner-warn", ".banner.banner-warn"],
        "background",
        "var(--banner-warn-bg)",
    ),
    (
        &[".banner-warn", ".banner.banner-warn"],
        "color",
        "var(--banner-warn-fg)",
    ),
    (&["footer a"], "color", "var(--link)"),
    (&[".card"], "background", "var(--card)"),
];

/// Test 66 (RMC79): WCAG AA text contrast (4.5:1) and 3:1 control boundaries in both themes,
/// computed from the declared tokens, plus the bindings that make those pairs the real ones.
#[test]
fn test_rmc_s47_text_contrast_meets_wcag_aa() {
    let rules = css_rules();
    let text_pairs: Vec<(String, Vec<&str>)> = {
        let mut v: Vec<(String, Vec<&str>)> = vec![
            (
                "--text".into(),
                vec!["--page", "--card", "--table", "--input-bg"],
            ),
            ("--text-muted".into(), vec!["--page", "--card", "--table"]),
            ("--link".into(), vec!["--page", "--card"]),
            ("--on-band".into(), vec!["--band"]),
            ("--on-tile".into(), vec!["--tile"]),
            ("--on-strip".into(), vec!["--strip"]),
            (
                "--on-primary".into(),
                vec!["--primary", "--primary-hover", "--primary-pressed"],
            ),
            (
                "--secondary-fg".into(),
                vec!["--card", "--secondary-hover", "--secondary-pressed"],
            ),
            (
                "--on-danger".into(),
                vec!["--danger-fill", "--danger-fill-hover"],
            ),
            (
                "--danger-fg".into(),
                vec!["--card", "--danger-outline-hover"],
            ),
            ("--placeholder".into(), vec!["--input-bg"]),
        ];
        for k in ["info", "success", "warn", "danger"] {
            v.push((format!("--banner-{k}-fg"), vec![]));
        }
        v
    };
    for dark in [false, true] {
        let theme = resolve_theme(&rules, dark);
        let scheme = if dark { "dark" } else { "light" };
        let check = |fg: &str, bg: &str, min: f64| {
            let r = contrast(theme[fg], theme[bg]);
            assert!(
                r >= min,
                "{scheme}: {fg} on {bg} is {r:.2}:1, below {min}:1"
            );
        };
        for (fg, bgs) in &text_pairs {
            if let Some(kind) = fg
                .strip_prefix("--banner-")
                .and_then(|s| s.strip_suffix("-fg"))
            {
                check(fg, &format!("--banner-{kind}-bg"), 4.5);
            }
            for bg in bgs {
                check(fg, bg, 4.5);
            }
        }
        check("--input-border", "--input-bg", 3.0);
        check("--focus", "--page", 3.0);
        check("--focus", "--card", 3.0);
        for dot in [
            "--dot-locked",
            "--dot-unlocked",
            "--dot-alarm",
            "--dot-idle",
        ] {
            check(dot, "--strip", 3.0);
        }
        check("--secondary-fg", "--card", 3.0);
        check("--danger-fg", "--card", 3.0);
        for k in ["info", "success", "warn", "danger"] {
            check(
                &format!("--banner-{k}-bg"),
                &format!("--banner-{k}-icon"),
                3.0,
            );
        }
    }

    for (selectors, prop, value) in BINDINGS {
        let props: &[&str] = if prop == "background" {
            &["background", "background-color"]
        } else {
            &[prop]
        };
        let declared: Vec<&String> = rules
            .iter()
            .filter(|r| r.has_selector(selectors))
            .flat_map(|r| r.decls.iter())
            .filter(|(p, _)| props.contains(&p.as_str()))
            .map(|(_, v)| v)
            .collect();
        assert!(
            !declared.is_empty(),
            "a rule for `{}` must declare `{prop}: {value}`",
            selectors[0]
        );
        for v in declared {
            assert_eq!(
                v, value,
                "`{}` {prop} must be exactly {value}",
                selectors[0]
            );
        }
    }
    for rule in &rules {
        for (p, v) in &rule.decls {
            if p != "color" {
                continue;
            }
            for fill_only in ["--danger-fill", "--dot-unlocked", "--dot-locked", "--star"] {
                assert!(
                    !var_refs(v)
                        .unwrap_or_default()
                        .iter()
                        .any(|r| r == fill_only),
                    "`{}` uses the fill-only role {fill_only} as text color",
                    rule.selectors.join(", ")
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Test 67 – RMC-S48
// ---------------------------------------------------------------------------------------------

fn min_height_ok(value: &str, touch_px: u32) -> bool {
    px_value(value).is_some_and(|v| v >= 44) || (value == "var(--touch)" && touch_px >= 44)
}

/// Test 67 (RMC80): focus ring, 44 px touch targets, safe areas, reduced motion, zoom kept.
#[test]
fn test_rmc_s48_mobile_frame_focus_touch_motion() {
    let css = strip_css_comments(&read(STYLE));
    let rules = css_rules();
    let top = top_root_decls(&rules);
    let touch = top
        .iter()
        .find(|d| d.0 == "--touch")
        .and_then(|d| px_value(&d.1))
        .expect("--touch declared in px in :root");
    assert!(touch >= 44, "--touch is {touch}px, below 44px");

    let has = |selectors: &[&str], prop: &str, ok: &dyn Fn(&str) -> bool| {
        rules
            .iter()
            .filter(|r| r.has_selector(selectors))
            .flat_map(|r| r.decls.iter())
            .any(|(p, v)| p == prop && ok(v))
    };
    assert!(
        has(&["button"], "min-height", &|v| v == "var(--touch)"),
        "`button` must declare `min-height: var(--touch)`"
    );
    assert!(
        has(
            &["input", "#enroll-code", "input#enroll-code"],
            "min-height",
            &|v| min_height_ok(v, touch)
        ),
        "the code input must have min-height >= 44px"
    );
    assert!(
        has(&["footer a"], "min-height", &|v| min_height_ok(v, touch)),
        "`footer a` must have min-height >= 44px"
    );

    let focus_ok = rules.iter().any(|r| {
        r.selectors
            .iter()
            .any(|s| s.contains(":focus-visible") && !s.contains(":not(:focus-visible)"))
            && r.decls.iter().any(|(p, v)| {
                p == "outline"
                    && v.contains("var(--focus)")
                    && v.split_whitespace()
                        .find_map(px_value)
                        .is_some_and(|w| w >= 2)
            })
    });
    assert!(
        focus_ok,
        "a :focus-visible rule with `outline: <>=2px> solid var(--focus)`"
    );
    for rule in &rules {
        for (p, v) in &rule.decls {
            let removes = (p == "outline" && (v == "none" || v == "0"))
                || (p == "outline-style" && v == "none")
                || (p == "outline-width" && (v == "0" || v == "0px"));
            if removes {
                assert!(
                    rule.selectors
                        .iter()
                        .all(|s| s.ends_with(":focus:not(:focus-visible)")),
                    "outline removed outside `:focus:not(:focus-visible)`: `{}`",
                    rule.selectors.join(", ")
                );
            }
        }
    }
    for side in ["top", "bottom", "left", "right"] {
        assert!(
            css.contains(&format!("env(safe-area-inset-{side})")),
            "style.css must honour env(safe-area-inset-{side})"
        );
    }
    assert_eq!(
        media_blocks(&rules, "prefers-reduced-motion: reduce").len(),
        1,
        "exactly one @media (prefers-reduced-motion: reduce) block"
    );

    let html = read(INDEX);
    let tags = open_tags(&html);
    let viewport = tags
        .iter()
        .find(|t| t.name == "meta" && t.value("name") == Some("viewport"))
        .and_then(|t| t.value("content"))
        .expect("viewport meta");
    assert!(
        viewport.contains("viewport-fit=cover"),
        "viewport-fit=cover kept"
    );
    assert!(
        !viewport.contains("user-scalable") && !viewport.contains("maximum-scale"),
        "zoom must stay possible: {viewport}"
    );
}

// ---------------------------------------------------------------------------------------------
// Test 68 – RMC-S49
// ---------------------------------------------------------------------------------------------

/// Test 68 (RMC81): CSP-clean markup and styles: no inline style, script, handler or
/// `data:`/`blob:`/`javascript:` URL; inline SVGs take their colors from `style.css`.
#[test]
fn test_rmc_s49_no_inline_style_script_or_data_uri() {
    let html = read(INDEX);
    let lower = html.to_ascii_lowercase();
    let tags = open_tags(&html);
    for t in &tags {
        for (name, _) in &t.attrs {
            assert!(name != "style", "inline style attribute on <{}>", t.name);
            let handler = name.len() > 2
                && name.starts_with("on")
                && name[2..].bytes().all(|b| b.is_ascii_lowercase());
            assert!(!handler, "inline event handler `{name}` on <{}>", t.name);
        }
    }
    assert!(!lower.contains("<style"), "no <style> element");
    let scripts: Vec<&Tag> = tags.iter().filter(|t| t.name == "script").collect();
    assert_eq!(scripts.len(), 1, "exactly one <script>");
    assert_eq!(lower.matches("<script").count(), 1, "exactly one <script>");
    assert!(
        html.contains(r#"<script src="app.js"></script>"#),
        "the only script is <script src=\"app.js\"></script>"
    );
    for scheme in ["javascript:", "data:", "blob:"] {
        assert!(
            !lower.contains(scheme),
            "index.html must not contain `{scheme}`"
        );
    }
    for (_, svg) in svg_segments(&html) {
        for needle in [
            "fill=",
            "stroke=",
            "href=",
            "xlink:href",
            "<use",
            "<image",
            "<foreignobject",
            "<script",
            " style=",
        ] {
            assert!(
                !svg.to_ascii_lowercase().contains(needle),
                "inline SVG must not contain `{needle}`"
            );
        }
    }
    let css = strip_css_comments(&read(STYLE)).to_ascii_lowercase();
    for needle in ["url(", "@import", "@font-face", "expression("] {
        assert!(
            !css.contains(needle),
            "style.css must not contain `{needle}`"
        );
    }
    let icon = read(ICON).to_ascii_lowercase();
    for needle in ["<script", "href", "style", "<image", "<foreignobject"] {
        assert!(
            !icon.contains(needle),
            "icon.svg must not contain `{needle}`"
        );
    }
    for t in open_tags(&icon) {
        for (name, _) in &t.attrs {
            assert!(!name.starts_with("on"), "icon.svg handler `{name}`");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Test 69 – RMC-S50
// ---------------------------------------------------------------------------------------------

/// Test 69 (RMC82): every id, label and initial attribute `app.js` and the docs rely on is
/// kept; no push switch (D11). Regression guard: passes on the page before the redesign too.
#[test]
fn test_rmc_s50_ui_contract_ids_labels_and_attributes_are_kept() {
    let html = read(INDEX);
    let tags = open_tags(&html);
    let ids: Vec<(&str, &str)> = tags
        .iter()
        .filter_map(|t| t.value("id").map(|id| (id, t.name.as_str())))
        .collect();
    let expected: BTreeSet<&str> = PAGE_IDS.iter().map(|p| p.0).collect();
    for (id, tag) in PAGE_IDS {
        let found: Vec<&&str> = ids
            .iter()
            .filter(|(i, _)| *i == id)
            .map(|(_, t)| t)
            .collect();
        assert_eq!(found.len(), 1, "id `{id}` must appear exactly once");
        assert_eq!(*found[0], tag, "id `{id}` must stay on <{tag}>");
    }
    for (id, _) in &ids {
        assert!(expected.contains(id), "unexpected id `{id}` in index.html");
    }
    let app = read(APP_JS);
    let used: BTreeSet<&str> = app
        .match_indices("getElementById(\"")
        .filter_map(|(at, m)| {
            let rest = &app[at + m.len()..];
            rest.find('"').map(|e| &rest[..e])
        })
        .collect();
    assert_eq!(
        used, expected,
        "app.js getElementById set equals the 29 page ids"
    );

    for (id, label) in BUTTON_LABELS {
        let t = tag_by_id(&tags, id).unwrap();
        assert_eq!(inner_html(&html, t), label, "label of #{id}");
    }
    for t in tags.iter().filter(|t| t.name == "button") {
        assert_eq!(
            t.value("type"),
            Some("button"),
            "every button is type=\"button\""
        );
    }
    for id in INITIALLY_HIDDEN {
        assert_eq!(
            tag_by_id(&tags, id).unwrap().attr("hidden"),
            Some(None),
            "#{id} hidden"
        );
    }
    assert_eq!(
        tag_by_id(&tags, "status-card").unwrap().attr("hidden"),
        None,
        "#status-card visible"
    );
    for id in ["lock", "unlock"] {
        assert_eq!(
            tag_by_id(&tags, id).unwrap().attr("disabled"),
            Some(None),
            "#{id} disabled"
        );
    }
    for id in FEEDBACK_IDS {
        assert_eq!(
            tag_by_id(&tags, id).unwrap().value("role"),
            Some("status"),
            "#{id} role"
        );
    }
    let main = tags.iter().find(|t| t.name == "main").expect("<main>");
    assert_eq!(main.value("aria-live"), Some("polite"), "main aria-live");
    for id in ["alerts", "push"] {
        assert_eq!(
            tag_by_id(&tags, id).unwrap().value("aria-live"),
            Some("polite"),
            "#{id} aria-live"
        );
    }
    let state = tag_by_id(&tags, "state").unwrap();
    assert_eq!(
        state.value("class"),
        Some("state state-unknown"),
        "#state class"
    );
    assert_eq!(
        state.value("data-state"),
        Some("unknown"),
        "#state data-state"
    );
    assert_eq!(inner_html(&html, state), "Connecting", "#state text");
    assert!(
        tag_by_id(&tags, "unlock")
            .unwrap()
            .classes()
            .contains(&"secondary"),
        "#unlock keeps the class secondary"
    );
    for h2 in [
        "<h2>Failed passwords on the PC</h2>",
        "<h2>Notifications</h2>",
        "<h2>Add a passkey</h2>",
    ] {
        assert!(html.contains(h2), "kept heading {h2}");
    }
    for sentence in [
        "Sign in with your passkey to reach this PC over the internet.",
        "Run <code>soos-remote enroll-code</code> on the PC, then type the code.",
    ] {
        assert!(html.contains(sentence), "kept sentence `{sentence}`");
    }
    assert_eq!(
        inner_html(&html, tag_by_id(&tags, "push-hint").unwrap()),
        PUSH_HINT_INNER,
        "#push-hint verbatim"
    );
    let code = tag_by_id(&tags, "enroll-code").unwrap();
    for (attr, value) in ENROLL_CODE_ATTRIBUTES {
        assert_eq!(code.value(attr), Some(value), "#enroll-code {attr}");
    }
    assert!(html.contains("<title>soos remote</title>"), "title kept");

    // No push switch (D11).
    for t in &tags {
        let classes = t.classes();
        assert!(
            !classes.contains(&"switch") && !classes.contains(&"switch-knob"),
            "no switch element in index.html"
        );
        assert_ne!(t.value("role"), Some("switch"), "no role=\"switch\"");
        assert!(t.attr("aria-checked").is_none(), "no aria-checked");
    }
    let rules = css_rules();
    for r in &rules {
        for s in &r.selectors {
            assert!(
                !has_class_selector(s, "switch"),
                "no .switch selector in style.css: `{s}`"
            );
            assert!(
                !s.contains(":has(#push-disable"),
                "no :has(#push-disable selector: `{s}`"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Test 70 – RMC-S51
// ---------------------------------------------------------------------------------------------

/// Test 70 (RMC83): the blue header band carries the inline SOOS wordmark and the accessible
/// name `soos remote`.
#[test]
fn test_rmc_s51_header_band_carries_the_wordmark() {
    let html = read(INDEX);
    let header_at = html
        .find(r#"<header class="band">"#)
        .expect("index.html must open <header class=\"band\">");
    let main_at = html.find("<main").expect("<main>");
    assert!(header_at < main_at, "the band comes before <main>");
    let header_end = html[header_at..]
        .find("</header>")
        .map(|e| header_at + e)
        .expect("</header>");
    let header = &html[header_at..header_end];
    assert_eq!(header.matches("<h1").count(), 1, "one h1 in the band");
    let h1_start = header.find("<h1").unwrap();
    let h1_end = header.find("</h1>").expect("</h1>");
    let h1 = &header[h1_start..h1_end];
    let svgs = svg_segments(h1);
    assert_eq!(svgs.len(), 1, "one wordmark svg in the h1");
    let svg = svgs[0].1;
    let svg_tag = &open_tags(svg)[0];
    assert_eq!(
        svg_tag.value("viewbox"),
        Some("0 0 514 64"),
        "wordmark viewBox"
    );
    assert_eq!(
        svg_tag.value("aria-hidden"),
        Some("true"),
        "wordmark aria-hidden"
    );
    assert_eq!(
        svg_tag.value("focusable"),
        Some("false"),
        "wordmark focusable"
    );
    assert!(
        svg_tag.classes().contains(&"wordmark"),
        "svg class wordmark"
    );
    assert_eq!(
        sorted(path_ds(svg)),
        sorted_strs(&WORDMARK_PATHS),
        "the six wordmark paths"
    );
    let after_svg = &h1[h1.find("</svg>").unwrap()..];
    assert!(
        after_svg.contains(r#"class="visually-hidden">soos remote</span>"#),
        "visually hidden `soos remote` follows the wordmark"
    );
    let main_end = html.find("</main>").expect("</main>");
    assert!(
        !html[main_at..main_end].contains("<h1"),
        "<main> no longer holds an h1"
    );

    let rules = css_rules();
    let fill_ok = rules
        .iter()
        .filter(|r| r.has_selector(&[".wordmark", ".wordmark path", "svg.wordmark"]))
        .flat_map(|r| r.decls.iter())
        .any(|(p, v)| p == "fill" && v == "var(--on-band)");
    assert!(fill_ok, ".wordmark fill must be var(--on-band)");
    let band_ok = rules
        .iter()
        .filter(|r| r.has_selector(&[".band", "header.band"]))
        .flat_map(|r| r.decls.iter())
        .any(|(p, v)| (p == "background" || p == "background-color") && v == "var(--band)");
    assert!(band_ok, ".band background must be var(--band)");
}

// ---------------------------------------------------------------------------------------------
// Test 71 – RMC-S52
// ---------------------------------------------------------------------------------------------

/// The tag that opens right before `at` (the enclosing element of an inline SVG).
fn previous_open_tag(html: &str, at: usize) -> Option<Tag> {
    open_tags(&html[..at]).pop()
}

/// Test 71 (RMC84): `icon.svg` is the owner's star mark; the inline tile stars are the two
/// `STAR_SPIKES`; the touch icon is an opaque 180 x 180 RGB PNG with only IHDR/IDAT/IEND.
#[test]
fn test_rmc_s52_icons_are_the_brand_mark() {
    let icon = read(ICON);
    let icon_tags = open_tags(&icon);
    let svg = icon_tags.iter().find(|t| t.name == "svg").expect("<svg>");
    assert_eq!(
        svg.value("viewbox"),
        Some("0 0 508 508"),
        "icon.svg viewBox"
    );
    assert!(
        icon.contains(r##"<rect width="508" height="508" fill="#EDF1FF"/>"##),
        "icon.svg pale background rect"
    );
    assert_eq!(
        sorted(path_ds(&icon)),
        sorted_strs(&ICON_PATHS),
        "the four blue mark paths"
    );

    let html = read(INDEX);
    let stars: Vec<(usize, &str)> = svg_segments(&html)
        .into_iter()
        .filter(|(_, s)| open_tags(s)[0].value("viewbox") == Some("0 0 508 508"))
        .collect();
    assert!(stars.len() >= 2, "a stat-tile star and a login-tile star");
    let mut stat_star = false;
    let mut login_star = false;
    for (at, s) in &stars {
        let tag = &open_tags(s)[0];
        assert_eq!(
            sorted(path_ds(s)),
            sorted_strs(&STAR_SPIKE_PATHS),
            "star spikes"
        );
        let parent = previous_open_tag(&html, *at);
        let hidden = tag.value("aria-hidden") == Some("true")
            || parent
                .as_ref()
                .is_some_and(|p| p.value("aria-hidden") == Some("true"));
        assert!(hidden, "decorative star svg must be aria-hidden");
        stat_star |= tag.classes().contains(&"stat-star");
        login_star |= parent.is_some_and(|p| p.classes().contains(&"login-tile"));
    }
    assert!(stat_star, "the status tile carries an svg.stat-star");
    assert!(login_star, "the login tile carries a star svg");
    assert!(
        html.contains(r#"<link rel="icon" href="icon.svg" type="image/svg+xml">"#),
        "icon link keeps type image/svg+xml"
    );

    let png = fs::read(workspace_root().join(TOUCH_ICON)).expect("apple-touch-icon.png");
    check_touch_icon(&png).unwrap_or_else(|e| panic!("apple-touch-icon.png: {e}"));
    assert!(
        png.len() <= 8192,
        "apple-touch-icon.png is {} bytes, above 8 KiB",
        png.len()
    );
}

// ---------------------------------------------------------------------------------------------
// Test 72 – RMC-S53
// ---------------------------------------------------------------------------------------------

/// JSON text with the whitespace outside strings removed.
fn compact_json(text: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_str {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
        } else if ch == '"' {
            in_str = true;
            out.push(ch);
        } else if !ch.is_whitespace() {
            out.push(ch);
        }
    }
    out
}

/// Test 72 (RMC85): manifest and meta colors from the palette; the rest of the manifest kept.
#[test]
fn test_rmc_s53_manifest_and_meta_colors() {
    let manifest = compact_json(&read(MANIFEST));
    for pair in [
        r##""theme_color":"#0047BB""##,
        r##""background_color":"#EDF1FF""##,
        r#""display":"standalone""#,
        r#""start_url":"/""#,
        r#""scope":"/""#,
        r#""short_name":"soos""#,
        r#""name":"soos remote""#,
    ] {
        assert!(manifest.contains(pair), "manifest must contain {pair}");
    }
    let icons_at = manifest.find(r#""icons":["#).expect("icons array");
    let icons_end = manifest[icons_at..]
        .find(']')
        .map(|e| icons_at + e)
        .expect("]");
    let objects: Vec<&str> = manifest[icons_at..icons_end]
        .split('}')
        .filter(|o| o.contains('{'))
        .collect();
    assert_eq!(objects.len(), 2, "exactly two manifest icons");
    for entry in [
        [
            r#""src":"icon.svg""#,
            r#""sizes":"any""#,
            r#""type":"image/svg+xml""#,
        ],
        [
            r#""src":"apple-touch-icon.png""#,
            r#""sizes":"180x180""#,
            r#""type":"image/png""#,
        ],
    ] {
        assert!(
            objects
                .iter()
                .any(|o| entry.iter().all(|kv| o.contains(kv))),
            "manifest icon entry {entry:?}"
        );
    }
    let html = read(INDEX);
    assert_eq!(
        html.matches(r#"name="theme-color""#).count(),
        1,
        "one theme-color meta"
    );
    assert!(
        html.contains(r##"<meta name="theme-color" content="#0047BB">"##),
        "theme-color #0047BB"
    );
    assert!(
        html.contains(
            r#"<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">"#
        ),
        "status bar black-translucent"
    );
}

// ---------------------------------------------------------------------------------------------
// Test 73 – RMC-S54
// ---------------------------------------------------------------------------------------------

/// Test 73 (RMC86): the system font stack only and the fixed seven-file asset set.
#[test]
fn test_rmc_s54_system_fonts_and_fixed_asset_set() {
    let rules = css_rules();
    let top = top_root_decls(&rules);
    let font = top.iter().find(|d| d.0 == "--font").map(|d| d.1.as_str());
    assert!(
        font.is_some_and(|f| f.starts_with("-apple-system")),
        "--font must start with -apple-system, got {font:?}"
    );
    for r in rules.iter().filter(|r| r.selectors != [":root"]) {
        for (p, v) in &r.decls {
            if p == "font-family" {
                assert!(
                    v == "var(--font)" || v == "var(--font-mono)",
                    "`{}` font-family must be var(--font) or var(--font-mono), got `{v}`",
                    r.selectors.join(", ")
                );
            }
        }
    }
    let mut listed: Vec<String> = fs::read_dir(workspace_root().join(ASSETS_DIR))
        .expect("assets dir")
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        sorted_strs(&ASSET_FILES),
        "the asset directory holds exactly seven files"
    );
    let assets_rs = read(ASSETS_RS);
    assert_eq!(
        assets_rs.matches("include_bytes!(\"../assets/").count(),
        7,
        "assets.rs embeds seven files"
    );
    for f in ASSET_FILES {
        assert!(
            assets_rs.contains(&format!("include_bytes!(\"../assets/{f}\")")),
            "assets.rs embeds {f}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Test 74 – RMC-S55
// ---------------------------------------------------------------------------------------------

/// Test 74 (RMC87): the page design is documented in `Docs/REMOTE_COMPANION.md` §2e, the ADR
/// exists and the tester contract is recorded.
#[test]
fn test_rmc_s55_brand_is_documented() {
    let doc = read(REMOTE_DOC);
    let start = doc
        .find("\n## 2e.")
        .expect("Docs/REMOTE_COMPANION.md needs a section `## 2e.`");
    let section_end = doc[start + 1..]
        .find("\n## ")
        .map_or(doc.len(), |e| start + 1 + e);
    let section = &doc[start..section_end];
    for needle in [
        "#0047BB",
        "#EDF1FF",
        "#101820",
        "#E59BDC",
        "crates/gui/src/theme.rs",
        "prefers-color-scheme",
        "Unlock now",
        "apple-touch-icon.png",
        "rsvg-convert",
    ] {
        assert!(section.contains(needle), "§2e must mention `{needle}`");
    }
    assert!(
        read(DECISIONS).contains(ADR_TITLE),
        "ADR `{ADR_TITLE}` in AI/DECISIONS.md"
    );
    assert!(exists(TESTER_CONTRACT), "{TESTER_CONTRACT} exists");
}
