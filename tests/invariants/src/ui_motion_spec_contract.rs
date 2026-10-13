//! Static contracts of the soos motion specification (GitHub #377 = PR 1 of #358; ADR
//! "[2026-10-13] Motion System for soos-gui and soos-remote"; architect spec
//! `AI/architect_spec_ui_motion.md` §8; matrix UIM1–UIM6):
//!
//! - UIM-S1 the ADR, the specification, the tester contract and the prototype exist and are indexed;
//! - UIM-S2 every motion token is documented with its CSS name, Rust name and value, exits are
//!   0.7 × entry, and any token already present in `style.css` or `theme.rs` has the pinned value;
//! - UIM-S3 the spring and easing mathematics: overshoot at most 4 % (owner decision), settle time,
//!   the CSS `linear()` approximation and the `cubic-bezier` fallback;
//! - UIM-S4 the HTML prototype is self-contained, on brand and declares every token;
//! - UIM-S5 the reduced-motion sources and the egui budgets are documented;
//! - UIM-S6 the remote companion and GUI documents point to the motion specification.
//!
//! The pinned values below are the contract: the later `feat/remote-motion` and `feat/gui-motion`
//! pull requests turn the forward-compatible checks of UIM-S2 into live cross-checks.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, indexing and float math"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const DECISIONS: &str = "AI/DECISIONS.md";
const MOTION_DOC: &str = "Docs/UI_MOTION.md";
const DOCS_INDEX: &str = "Docs/README.md";
const REMOTE_DOC: &str = "Docs/REMOTE_COMPANION.md";
const GUI_DOC: &str = "Docs/GUI_APPLICATION.md";
const TESTER_CONTRACT: &str = "AI/tester_contract_motion.md";
const PROTOTYPE: &str = "Docs/prototypes/ui_motion_prototype.html";
const STYLE: &str = "crates/remote/assets/style.css";
const GUI_THEME: &str = "crates/gui/src/theme.rs";

const ADR_TITLE: &str = "[2026-10-13] Motion System for soos-gui and soos-remote";

/// Upper bound of the prototype size (bytes).
const MAX_PROTOTYPE_BYTES: usize = 96 * 1024;

/// How a token value compares between CSS and Rust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// CSS milliseconds, Rust `f32` seconds.
    Ms,
    /// CSS `cubic-bezier(a, b, c, d)`, Rust `[f32; 4]`.
    Bezier,
    /// Same number on both sides (`px`, `deg` or unitless in CSS).
    Plain,
}

/// `(CSS custom property, theme.rs constant, exact CSS value, kind)`.
const TOKENS: [(&str, &str, &str, Kind); 17] = [
    ("--t-fast", "T_FAST", "120ms", Kind::Ms),
    ("--t-short", "T_SHORT", "180ms", Kind::Ms),
    ("--t-medium", "T_MEDIUM", "260ms", Kind::Ms),
    ("--t-long", "T_LONG", "380ms", Kind::Ms),
    ("--t-spring", "T_SPRING", "400ms", Kind::Ms),
    ("--t-short-exit", "T_SHORT_EXIT", "126ms", Kind::Ms),
    ("--t-medium-exit", "T_MEDIUM_EXIT", "182ms", Kind::Ms),
    ("--t-long-exit", "T_LONG_EXIT", "266ms", Kind::Ms),
    ("--t-stagger", "T_STAGGER", "30ms", Kind::Ms),
    ("--t-pending", "T_PENDING", "1200ms", Kind::Ms),
    (
        "--ease-standard",
        "EASE_STANDARD",
        "cubic-bezier(0.2, 0, 0, 1)",
        Kind::Bezier,
    ),
    (
        "--ease-enter",
        "EASE_ENTER",
        "cubic-bezier(0.16, 1, 0.3, 1)",
        Kind::Bezier,
    ),
    (
        "--ease-exit",
        "EASE_EXIT",
        "cubic-bezier(0.4, 0, 1, 1)",
        Kind::Bezier,
    ),
    ("--press-scale", "PRESS_SCALE", "0.98", Kind::Plain),
    ("--rise", "RISE_PX", "6px", Kind::Plain),
    ("--slide", "SLIDE_PX", "8px", Kind::Plain),
    ("--star-turn", "STAR_TURN_DEG", "45deg", Kind::Plain),
];

/// `(exit token, entry token)`: exits run at 0.7 × the entry duration.
const EXITS: [(&str, &str); 3] = [
    ("--t-short-exit", "--t-short"),
    ("--t-medium-exit", "--t-medium"),
    ("--t-long-exit", "--t-long"),
];

/// Spring parameters (owner decision: overshoot at most 4 %).
const SPRING_STIFFNESS: f64 = 400.0;
const SPRING_DAMPING: f64 = 30.0;
const SPRING_MASS: f64 = 1.0;
const MAX_OVERSHOOT: f64 = 0.04;
/// `T_SPRING` in seconds and the bound of the GUI settle time.
const T_SPRING_S: f64 = 0.4;
const MAX_SETTLE_S: f64 = 0.45;
/// GUI settle criterion (fractions of the span).
const SETTLE_EPSILON: f64 = 0.001;
const SETTLE_VELOCITY: f64 = 0.1;
/// Standard easing tokens of `--ease-*` (control points), checked for monotonicity.
const STANDARD_EASINGS: [[f64; 4]; 3] = [
    [0.2, 0.0, 0.0, 1.0],
    [0.16, 1.0, 0.3, 1.0],
    [0.4, 0.0, 1.0, 1.0],
];

/// `--ease-spring` sampled from the spring over `T_SPRING`.
const SPRING_LINEAR: &str = "linear(0, 0.0063 1.5%, 0.0263 3%, 0.0595 4.8%, 0.1065 6.6%, 0.1522 8.2%, 0.2104 10%, 0.4504 16.9%, 0.5562 20.1%, 0.6552 23.4%, 0.7391 26.6%, 0.8083 29.6%, 0.8658 32.8%, 0.9136 35.9%, 0.9524 39.3%, 0.9829 42.9%, 1.0052 46.8%, 1.0197 51%, 1.0271 55.9%, 1.0269 63.8%, 1.0085 84.4%, 1)";
/// Fallback of `--ease-spring` for engines without `linear()`.
const SPRING_FALLBACK: &str = "cubic-bezier(0.37, 0.73, 0.1, 1.15)";
const SPRING_FALLBACK_POINTS: [f64; 4] = [0.37, 0.73, 0.1, 1.15];
const MAX_LINEAR_ERROR: f64 = 0.005;
const MAX_FALLBACK_ERROR: f64 = 0.05;

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn exists(rel: &str) -> bool {
    workspace_root().join(rel).is_file()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Removes `/* ... */` comments.
fn strip_block_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Every number of `text` (digits with an optional fraction and sign).
fn numbers_in(text: &str) -> Vec<f64> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let starts = bytes[i].is_ascii_digit()
            || (bytes[i] == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
            || (bytes[i] == b'-'
                && bytes
                    .get(i + 1)
                    .is_some_and(|b| b.is_ascii_digit() || *b == b'.'));
        let after_ident = i > 0 && (bytes[i - 1].is_ascii_alphabetic() || bytes[i - 1] == b'_');
        if starts && !after_ident {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            if let Ok(v) = text[i..j].parse::<f64>() {
                out.push(v);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Value text of `pub const NAME: ... = <value>;` in Rust source, if declared.
fn rust_const_value(src: &str, name: &str) -> Option<String> {
    let needle = format!("pub const {name}:");
    let start = src.find(&needle)?;
    let rest = &src[start + needle.len()..];
    let eq = rest.find('=')?;
    let end = rest[eq..].find(';')?;
    Some(rest[eq + 1..eq + end].trim().to_string())
}

/// Every value declared for the custom property `name` in `css` (comments removed).
fn css_decl_values(css: &str, name: &str) -> Vec<String> {
    let css = strip_block_comments(css);
    let needle = format!("{name}:");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(pos) = css[from..].find(&needle) {
        let at = from + pos;
        let boundary_ok = at == 0 || {
            let prev = css.as_bytes()[at - 1];
            !(prev.is_ascii_alphanumeric() || prev == b'-' || prev == b'_')
        };
        let value_start = at + needle.len();
        if boundary_ok {
            let end = css[value_start..]
                .find([';', '}'])
                .map_or(css.len(), |e| value_start + e);
            out.push(normalize_ws(&css[value_start..end]));
        }
        from = value_start;
    }
    out
}

/// Spring step response `x(t)` for a unit step from rest, and its velocity.
fn spring_response(t: f64) -> (f64, f64) {
    let wn = (SPRING_STIFFNESS / SPRING_MASS).sqrt();
    let zeta = SPRING_DAMPING / (2.0 * (SPRING_STIFFNESS * SPRING_MASS).sqrt());
    let wd = wn * (1.0 - zeta * zeta).sqrt();
    let decay = (-zeta * wn * t).exp();
    let x = 1.0 - decay * ((wd * t).cos() + zeta * wn / wd * (wd * t).sin());
    let v = decay * (wn * wn / wd) * (wd * t).sin();
    (x, v)
}

/// Closed-form advance of the displacement `e = value - target` and velocity over `dt`.
fn spring_step(e: f64, v: f64, dt: f64) -> (f64, f64) {
    let wn = (SPRING_STIFFNESS / SPRING_MASS).sqrt();
    let zeta = SPRING_DAMPING / (2.0 * (SPRING_STIFFNESS * SPRING_MASS).sqrt());
    let wd = wn * (1.0 - zeta * zeta).sqrt();
    let a = zeta * wn;
    let b = (v + a * e) / wd;
    let decay = (-a * dt).exp();
    let (s, c) = (wd * dt).sin_cos();
    let ne = decay * (e * c + b * s);
    let nv = decay * (-a * (e * c + b * s) + (-e * wd * s + b * wd * c));
    (ne, nv)
}

/// Cubic-bezier easing `y(x)` with control points `(x1, y1, x2, y2)` (bisection on x).
fn cubic_bezier(p: [f64; 4], x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    let coord = |t: f64, a: f64, b: f64| {
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    };
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if coord(mid, p[0], p[2]) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    coord((lo + hi) / 2.0, p[1], p[3])
}

/// Stops `(position in [0, 1], value)` of a CSS `linear()` easing written with explicit
/// percentages (first `0`, last `1`).
fn parse_linear(text: &str) -> Result<Vec<(f64, f64)>, String> {
    let inner = text
        .strip_prefix("linear(")
        .and_then(|t| t.strip_suffix(')'))
        .ok_or("not a linear() value")?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() < 3 || parts[0] != "0" || parts[parts.len() - 1] != "1" {
        return Err("linear() must start with 0 and end with 1".to_string());
    }
    let mut stops = vec![(0.0, 0.0)];
    for part in &parts[1..parts.len() - 1] {
        let (value, pos) = part
            .split_once(' ')
            .ok_or_else(|| format!("stop `{part}` lacks a position"))?;
        let pos = pos
            .strip_suffix('%')
            .ok_or_else(|| format!("stop `{part}` position is not a percentage"))?;
        let value: f64 = value
            .parse()
            .map_err(|_| format!("bad value in `{part}`"))?;
        let pos: f64 = pos
            .parse()
            .map_err(|_| format!("bad position in `{part}`"))?;
        stops.push((pos / 100.0, value));
    }
    stops.push((1.0, 1.0));
    Ok(stops)
}

fn eval_linear(stops: &[(f64, f64)], x: f64) -> f64 {
    for w in stops.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x >= x0 && x <= x1 {
            return if x1 > x0 {
                y0 + (y1 - y0) * (x - x0) / (x1 - x0)
            } else {
                y1
            };
        }
    }
    stops.last().map_or(0.0, |s| s.1)
}

/// Every `#` colour literal (3, 4, 6 or 8 hex digits) of `text`, uppercased.
fn hex_colors(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for (i, b) in bytes.iter().enumerate() {
        if *b != b'#' {
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && bytes[j].is_ascii_hexdigit() {
            j += 1;
        }
        let len = j - i - 1;
        let ends_word = bytes
            .get(j)
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-'));
        if matches!(len, 3 | 4 | 6 | 8) && ends_word {
            out.push(text[i..j].to_ascii_uppercase());
        }
    }
    out
}

/// `#RRGGBB` of every `Color32::from_rgb(0x.., 0x.., 0x..)` constant of `theme.rs`, plus white.
fn theme_palette() -> BTreeSet<String> {
    let src = read(GUI_THEME);
    let mut out = BTreeSet::new();
    let mut rest = src.as_str();
    while let Some(pos) = rest.find("Color32::from_rgb(") {
        let args = &rest[pos + "Color32::from_rgb(".len()..];
        let end = args.find(')').expect("closing parenthesis");
        let channels: Vec<u8> = args[..end]
            .split(',')
            .map(|c| {
                let c = c.trim();
                c.strip_prefix("0x").map_or_else(
                    || c.parse().expect("decimal channel"),
                    |h| u8::from_str_radix(h, 16).expect("hex channel"),
                )
            })
            .collect();
        assert_eq!(channels.len(), 3, "three channels in from_rgb");
        out.insert(format!(
            "#{:02X}{:02X}{:02X}",
            channels[0], channels[1], channels[2]
        ));
        rest = &args[end..];
    }
    if src.contains("Color32::WHITE") {
        out.insert("#FFFFFF".to_string());
    }
    out
}

/// The markdown section starting at the heading `heading` (up to the next `## ` heading).
fn section<'a>(doc: &'a str, heading: &str) -> &'a str {
    let start = doc
        .find(heading)
        .unwrap_or_else(|| panic!("section `{heading}` not found"));
    let end = doc[start + heading.len()..]
        .find("\n## ")
        .map_or(doc.len(), |e| start + heading.len() + e);
    &doc[start..end]
}

/// Milliseconds of the backticked `NNNms` value in the token-table row of `token`.
fn doc_ms(doc: &str, token: &str) -> f64 {
    let row = table_row(doc, &[token]).unwrap_or_else(|| panic!("no table row for `{token}`"));
    row.split('`')
        .filter_map(|cell| cell.strip_suffix("ms"))
        .find_map(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no `NNNms` value in the row of `{token}`"))
}

/// CSS named colours the prototype must not use (palette custom properties only).
const NAMED_COLORS: [&str; 38] = [
    "red",
    "green",
    "blue",
    "white",
    "black",
    "gray",
    "grey",
    "silver",
    "maroon",
    "purple",
    "fuchsia",
    "lime",
    "olive",
    "yellow",
    "navy",
    "teal",
    "aqua",
    "orange",
    "pink",
    "gold",
    "cyan",
    "magenta",
    "violet",
    "indigo",
    "brown",
    "beige",
    "coral",
    "crimson",
    "salmon",
    "tomato",
    "khaki",
    "lavender",
    "plum",
    "orchid",
    "turquoise",
    "ivory",
    "snow",
    "chartreuse",
];

/// Named colours used in declaration values of the `<style>` blocks of `html`.
fn named_colors_in_style(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<style>") {
        let body = &rest[start + "<style>".len()..];
        let end = body.find("</style>").unwrap_or(body.len());
        let css = strip_block_comments(&body[..end]).to_ascii_lowercase();
        for decl in css.split([';', '{', '}']) {
            let Some((_, value)) = decl.split_once(':') else {
                continue;
            };
            for word in value.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
                if NAMED_COLORS.contains(&word) {
                    found.push(word.to_string());
                }
            }
        }
        rest = &body[end..];
    }
    found
}

/// The `|`-table line of `doc` that carries every backticked needle, if any.
fn table_row<'a>(doc: &'a str, needles: &[&str]) -> Option<&'a str> {
    doc.lines().find(|line| {
        line.trim_start().starts_with('|')
            && needles.iter().all(|n| line.contains(&format!("`{n}`")))
    })
}

// ---------------------------------------------------------------------------------------------
// Helper self-test
// ---------------------------------------------------------------------------------------------

/// The helpers can fail: the parsers and solvers reject or compute known inputs exactly.
#[test]
fn test_uim_helper_self_test() {
    for x in [0.0, 0.25, 0.5, 0.75, 1.0] {
        assert!((cubic_bezier([0.0, 0.0, 1.0, 1.0], x) - x).abs() < 1e-9);
    }
    assert!(cubic_bezier([0.37, 0.73, 0.1, 1.15], 0.5) > 0.9);
    let stops = parse_linear("linear(0, 0.5 50%, 1)").unwrap();
    assert_eq!(stops, vec![(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)]);
    assert!((eval_linear(&stops, 0.25) - 0.25).abs() < 1e-12);
    assert!(parse_linear("linear(0, 0.5, 1)").is_err());
    assert!(parse_linear("linear(0.1, 1)").is_err());
    assert_eq!(
        numbers_in("cubic-bezier(0.2, 0, 0, 1)"),
        vec![0.2, 0.0, 0.0, 1.0]
    );
    assert_eq!(
        numbers_in("[0.16, 1.0, -0.3, 1.0]"),
        vec![0.16, 1.0, -0.3, 1.0]
    );
    assert_eq!(numbers_in("T_FAST2 = 0.12"), vec![0.12]);
    assert_eq!(
        rust_const_value("pub const T_FAST: f32 = 0.12;\n", "T_FAST").as_deref(),
        Some("0.12")
    );
    assert_eq!(
        rust_const_value("pub const T_FASTER: f32 = 1.0;", "T_FAST"),
        None
    );
    assert_eq!(
        css_decl_values(
            ":root { --t-short: 180ms; --t-short-exit: 126ms; }",
            "--t-short"
        ),
        vec!["180ms".to_string()]
    );
    assert_eq!(
        hex_colors("a{color:#0047bb} #lock #ABC; #12345"),
        vec!["#0047BB".to_string(), "#ABC".to_string()]
    );
    assert_eq!(doc_ms("| `--b` | `B` | `126ms` |", "--b"), 126.0);
    assert_eq!(
        named_colors_in_style("<style>a { color: red; --blue: x; }</style>"),
        vec!["red".to_string()]
    );
    assert!(
        named_colors_in_style("<style>a { color: var(--pink); white-space: nowrap; }</style>")
            .is_empty()
    );
    assert_eq!(
        table_row("| `--a` | `A` | `1ms` |\ntext `--a`", &["--a", "1ms"]),
        Some("| `--a` | `A` | `1ms` |")
    );
    let (x0, v0) = spring_response(0.0);
    assert!(x0.abs() < 1e-12 && v0.abs() < 1e-12);
    let (e, v) = spring_step(-1.0, 0.0, 0.1);
    let (x, xv) = spring_response(0.1);
    assert!((e - (x - 1.0)).abs() < 1e-9 && (v - xv).abs() < 1e-9);
}

// ---------------------------------------------------------------------------------------------
// UIM-S1
// ---------------------------------------------------------------------------------------------

/// UIM-S1 (UIM1): the ADR amends the brand ADR with motion, and the specification documents exist.
#[test]
fn test_uim_s1_motion_adr_and_documents_exist() {
    let decisions = read(DECISIONS);
    let adr = decisions
        .lines()
        .find(|l| l.contains(ADR_TITLE))
        .unwrap_or_else(|| panic!("ADR `{ADR_TITLE}` in {DECISIONS}"));
    for needle in [
        "soos Brand Direction Applied to the `soos-remote` Web App",
        "Browser Support Policy for the `soos-remote` Web App",
        "no gradients",
        "decorative shadows",
        "classList",
        "startViewTransition",
        "prefers-reduced-motion",
        "--reduce-motion",
        "SOOS_REDUCE_MOTION",
        "org.freedesktop.appearance",
        "4 %",
        "Docs/UI_MOTION.md",
        "Docs/prototypes/ui_motion_prototype.html",
        "CSP",
        "routes",
        "eleven-file asset set",
    ] {
        assert!(
            adr.contains(needle),
            "the motion ADR must mention `{needle}`"
        );
    }
    for rel in [MOTION_DOC, TESTER_CONTRACT, PROTOTYPE] {
        assert!(exists(rel), "{rel} must exist");
    }
    assert!(
        read(DOCS_INDEX).contains("](UI_MOTION.md)"),
        "{DOCS_INDEX} must index UI_MOTION.md"
    );
    let contract = read(TESTER_CONTRACT);
    for needle in [
        "remote_motion_contract.rs",
        "motion_tests.rs",
        "feat/remote-motion",
        "feat/gui-motion",
    ] {
        assert!(
            contract.contains(needle),
            "{TESTER_CONTRACT} must cover `{needle}`"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// UIM-S2
// ---------------------------------------------------------------------------------------------

/// UIM-S2 (UIM2): the token table carries every token; exits are 0.7 × entry; a token already
/// declared in `style.css` or `theme.rs` has the pinned value.
#[test]
fn test_uim_s2_motion_tokens_are_documented_and_consistent() {
    let doc = read(MOTION_DOC);
    for (css, rust, value, _) in TOKENS {
        assert!(
            table_row(&doc, &[css, rust, value]).is_some(),
            "{MOTION_DOC} needs one table row with `{css}`, `{rust}` and `{value}`"
        );
    }
    // Durations as written in the document's table (not the pinned constants).
    let ms = |name: &str| -> f64 { doc_ms(&doc, name) };
    for (css, _, value, kind) in TOKENS {
        if kind == Kind::Ms {
            assert_eq!(format!("{}ms", ms(css)), value, "{MOTION_DOC} {css}");
        }
    }
    for (exit, entry) in EXITS {
        assert_eq!(
            ms(exit),
            (ms(entry) * 0.7).round(),
            "{exit} must be 0.7 x {entry}"
        );
    }
    assert_eq!(
        ms("--t-spring") / 1000.0,
        T_SPRING_S,
        "--t-spring equals the spring settle time"
    );

    // Forward-compatible live cross-check (today only --t-fast exists in style.css).
    let style = read(STYLE);
    let theme = read(GUI_THEME);
    for (css, rust, value, kind) in TOKENS {
        for found in css_decl_values(&style, css) {
            assert_eq!(found, value, "style.css {css} must be `{value}`");
        }
        if let Some(found) = rust_const_value(&theme, rust) {
            let mut expected = numbers_in(value);
            if kind == Kind::Ms {
                expected = expected.iter().map(|v| v / 1000.0).collect();
            }
            let actual = numbers_in(&found);
            assert_eq!(
                actual.len(),
                expected.len(),
                "theme.rs {rust} = `{found}` must mirror {css} `{value}`"
            );
            for (a, e) in actual.iter().zip(&expected) {
                assert!(
                    (a - e).abs() < 1e-6,
                    "theme.rs {rust} = `{found}` must mirror {css} `{value}`"
                );
            }
        }
    }
    assert_eq!(
        css_decl_values(&style, "--t-fast"),
        vec!["120ms".to_string()],
        "--t-fast stays 120ms in style.css"
    );
    for found in css_decl_values(&style, "--ease-spring") {
        assert!(
            found == SPRING_FALLBACK || found == SPRING_LINEAR,
            "style.css --ease-spring must be the pinned fallback or linear(), got `{found}`"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// UIM-S3
// ---------------------------------------------------------------------------------------------

/// UIM-S3 (UIM3): the spring overshoots by at most 4 % and settles by `T_SPRING`; the CSS
/// `linear()` and fallback curves follow it; the standard easings are monotonic.
#[test]
fn test_uim_s3_spring_and_easing_mathematics() {
    let wn = (SPRING_STIFFNESS / SPRING_MASS).sqrt();
    let zeta = SPRING_DAMPING / (2.0 * (SPRING_STIFFNESS * SPRING_MASS).sqrt());
    assert!(
        zeta > 0.0 && zeta < 1.0,
        "light overshoot needs 0 < zeta < 1, got {zeta}"
    );
    let overshoot = (-zeta * std::f64::consts::PI / (1.0 - zeta * zeta).sqrt()).exp();
    assert!(
        overshoot <= MAX_OVERSHOOT,
        "spring overshoot {overshoot} exceeds 4 %"
    );
    assert!(
        overshoot >= 0.01,
        "the owner chose a visible overshoot, got {overshoot}"
    );
    assert!(wn > 0.0);

    // Continuous settle time (last excursion beyond 0.1 %).
    let mut last_out = 0.0;
    let mut peak = 0.0_f64;
    for i in 0..=40_000 {
        let t = f64::from(i) * 0.000_05;
        let (x, _) = spring_response(t);
        peak = peak.max(x);
        if (x - 1.0).abs() > SETTLE_EPSILON {
            last_out = t;
        }
    }
    assert!(
        (peak - 1.0 - overshoot).abs() < 1e-4,
        "sampled peak matches"
    );
    assert!(
        last_out <= T_SPRING_S + 0.001,
        "the spring must stay within 0.1 % from T_SPRING on, last excursion at {last_out}s"
    );
    const { assert!(T_SPRING_S <= MAX_SETTLE_S) };

    // Discrete closed-form stepping at 30, 60 and 144 Hz with the GUI settle criterion.
    for hz in [30.0, 60.0, 144.0] {
        let dt = 1.0 / hz;
        let (mut e, mut v) = (-1.0_f64, 0.0_f64);
        let mut settled_at: Option<f64> = None;
        let mut max_value = 0.0_f64;
        let mut t = 0.0;
        while t < 1.5 {
            (e, v) = spring_step(e, v, dt);
            t += dt;
            max_value = max_value.max(1.0 + e);
            let settled = e.abs() <= SETTLE_EPSILON && v.abs() <= SETTLE_VELOCITY;
            match (settled, settled_at) {
                (true, None) => settled_at = Some(t),
                (false, Some(at)) => panic!("{hz} Hz: unsettled again at {t}s after {at}s"),
                _ => {}
            }
            assert!(e.is_finite() && v.is_finite());
        }
        let at = settled_at.unwrap_or_else(|| panic!("{hz} Hz: never settled"));
        assert!(at <= MAX_SETTLE_S, "{hz} Hz: settled at {at}s > 450 ms");
        assert!(
            max_value - 1.0 <= MAX_OVERSHOOT,
            "{hz} Hz: overshoot {max_value}"
        );
    }

    // CSS linear() approximation.
    let stops = parse_linear(SPRING_LINEAR).expect("pinned linear() parses");
    for w in stops.windows(2) {
        assert!(w[1].0 > w[0].0, "linear() positions strictly increase");
    }
    let max_stop = stops.iter().map(|s| s.1).fold(0.0_f64, f64::max);
    assert!(
        max_stop <= 1.0 + MAX_OVERSHOOT,
        "linear() peaks at {max_stop}"
    );
    assert!(
        stops.len() >= 16,
        "linear() keeps enough stops ({})",
        stops.len()
    );
    let mut max_err = 0.0_f64;
    let mut max_fallback_err = 0.0_f64;
    let mut fallback_peak = 0.0_f64;
    for i in 0..=2000 {
        let p = f64::from(i) / 2000.0;
        let (x, _) = spring_response(p * T_SPRING_S);
        max_err = max_err.max((eval_linear(&stops, p) - x).abs());
        let fb = cubic_bezier(SPRING_FALLBACK_POINTS, p);
        fallback_peak = fallback_peak.max(fb);
        max_fallback_err = max_fallback_err.max((fb - x).abs());
    }
    assert!(max_err <= MAX_LINEAR_ERROR, "linear() error {max_err}");
    assert!(
        fallback_peak <= 1.0 + MAX_OVERSHOOT,
        "fallback peaks at {fallback_peak}"
    );
    assert!(
        max_fallback_err <= MAX_FALLBACK_ERROR,
        "fallback error {max_fallback_err}"
    );
    assert_eq!(
        numbers_in(SPRING_FALLBACK),
        SPRING_FALLBACK_POINTS.to_vec(),
        "fallback string and points agree"
    );

    for points in STANDARD_EASINGS {
        assert!(cubic_bezier(points, 0.0).abs() < 1e-9);
        assert!((cubic_bezier(points, 1.0) - 1.0).abs() < 1e-9);
        let mut prev = 0.0;
        for i in 0..=1000 {
            let y = cubic_bezier(points, f64::from(i) / 1000.0);
            assert!(y + 1e-9 >= prev, "{points:?} is not monotonic");
            assert!(y <= 1.0 + 1e-9, "{points:?} overshoots");
            prev = y;
        }
    }
    for (css, _, value, kind) in TOKENS {
        if kind == Kind::Bezier {
            let n = numbers_in(value);
            assert!(
                STANDARD_EASINGS
                    .iter()
                    .any(|p| p.iter().zip(&n).all(|(a, b)| (a - b).abs() < 1e-12)),
                "{css} is one of the checked standard easings"
            );
        }
    }

    // The document carries the exact parameters and curves.
    let doc = read(MOTION_DOC);
    for (name, value) in [
        ("SPRING_STIFFNESS", "400.0"),
        ("SPRING_DAMPING", "30.0"),
        ("SPRING_MASS", "1.0"),
    ] {
        assert!(
            table_row(&doc, &[name, value]).is_some(),
            "{MOTION_DOC} needs a table row with `{name}` and `{value}`"
        );
    }
    assert!(
        doc.contains(SPRING_LINEAR),
        "{MOTION_DOC} carries the linear() value"
    );
    assert!(
        doc.contains(SPRING_FALLBACK),
        "{MOTION_DOC} carries the fallback"
    );
    assert!(
        doc.contains("@supports (transition-timing-function: linear(0, 1))"),
        "{MOTION_DOC} states how the linear() value is gated"
    );
}

// ---------------------------------------------------------------------------------------------
// UIM-S4
// ---------------------------------------------------------------------------------------------

/// UIM-S4 (UIM4): the prototype is one self-contained, on-brand file that declares every token.
#[test]
fn test_uim_s4_prototype_is_self_contained_and_on_brand() {
    let html = read(PROTOTYPE);
    assert!(
        html.len() <= MAX_PROTOTYPE_BYTES,
        "the prototype is {} bytes, above {MAX_PROTOTYPE_BYTES}",
        html.len()
    );
    assert!(
        html.starts_with("<!doctype html>"),
        "the prototype starts with <!doctype html>"
    );
    assert!(
        html.contains("<html lang=\"en\">"),
        "the prototype is in English"
    );
    let lower = html.to_ascii_lowercase();
    assert_eq!(
        lower.matches("<script").count(),
        1,
        "exactly one script element"
    );
    assert!(
        lower.contains("<script>"),
        "the script element is inline, without attributes"
    );
    let named = named_colors_in_style(&html);
    assert!(
        named.is_empty(),
        "named colours in the prototype style: {named:?}"
    );
    for forbidden in [
        "http://",
        "https://",
        "//cdn",
        "<link",
        "<base",
        "<form",
        "<source",
        "<audio",
        "<track",
        "<use",
        "src=",
        "srcset",
        "href=",
        "xlink:",
        "action=",
        "import(",
        "importscripts",
        "javascript:",
        "@font-face",
        "text-shadow",
        "drop-shadow",
        "url(",
        "@import",
        "data:",
        "blob:",
        "<img",
        "<video",
        "<iframe",
        "<object",
        "<embed",
        "fetch(",
        "xmlhttprequest",
        "websocket",
        "eventsource",
        "sendbeacon",
        "getusermedia",
        "mediadevices",
        "localstorage",
        "sessionstorage",
        "indexeddb",
        "document.cookie",
        "gradient(",
        "rgb(",
        "rgba(",
        "hsl(",
        "hwb(",
        "oklch(",
        "color-mix(",
        "backdrop-filter",
        "filter:",
        "/home/",
    ] {
        assert!(
            !lower.contains(forbidden),
            "the prototype must not contain `{forbidden}`"
        );
    }
    for pattern in [
        "transition: all",
        "transition:all",
        "transition-property: all",
    ] {
        assert!(!lower.contains(pattern), "no `{pattern}` in the prototype");
    }
    let mut rest = lower.as_str();
    while let Some(pos) = rest.find("box-shadow:") {
        let value = rest[pos + "box-shadow:".len()..].trim_start();
        assert!(
            value.starts_with("inset 0 0 0") || value.starts_with("none"),
            "box-shadow is only a functional inset outline: `{}`",
            value.split([';', '}']).next().unwrap_or("")
        );
        rest = &rest[pos + 1..];
    }

    let palette = theme_palette();
    assert!(
        palette.len() >= 25,
        "theme.rs palette parsed ({})",
        palette.len()
    );
    for color in hex_colors(&html) {
        assert!(
            palette.contains(&color),
            "prototype colour {color} is not a crates/gui/src/theme.rs colour"
        );
    }

    let normalized = normalize_ws(&strip_block_comments(&html));
    for (css, _, value, _) in TOKENS {
        assert!(
            normalized.contains(&format!("{css}: {value};")),
            "the prototype must declare `{css}: {value};`"
        );
    }
    assert!(
        normalized.contains(&format!("--ease-spring: {SPRING_FALLBACK};")),
        "the prototype declares the spring fallback"
    );
    assert!(
        normalized.contains(&format!("--ease-spring: {SPRING_LINEAR};")),
        "the prototype declares the spring linear()"
    );
    assert!(
        normalized.contains("@supports (transition-timing-function: linear(0, 1))"),
        "the prototype gates linear() with @supports"
    );
    assert!(
        lower.contains("prefers-reduced-motion"),
        "the prototype follows prefers-reduced-motion"
    );
    assert!(
        html.contains("id=\"reduce-motion\""),
        "the prototype has a reduced-motion toggle"
    );
}

// ---------------------------------------------------------------------------------------------
// UIM-S5
// ---------------------------------------------------------------------------------------------

/// UIM-S5 (UIM5): reduced-motion sources, budgets and scope are documented.
#[test]
fn test_uim_s5_reduced_motion_sources_and_budgets_documented() {
    let doc = read(MOTION_DOC);
    for needle in [
        "prefers-reduced-motion",
        "org.freedesktop.appearance",
        "`reduced-motion` (`u32`",
        "org.gnome.desktop.interface",
        "enable-animations",
        "--reduce-motion",
        "SOOS_REDUCE_MOTION",
        "200 ms",
        "request_repaint_after",
        "animation_time",
        "startViewTransition",
        "@starting-style",
        "window.confirm",
        "dark variant",
        "out of scope",
        "Docs/prototypes/ui_motion_prototype.html",
        "UIM7",
    ] {
        assert!(doc.contains(needle), "{MOTION_DOC} must mention `{needle}`");
    }
}

// ---------------------------------------------------------------------------------------------
// UIM-S6
// ---------------------------------------------------------------------------------------------

/// UIM-S6 (UIM6): the remote companion §2e and the GUI page point to the motion specification.
#[test]
fn test_uim_s6_remote_and_gui_docs_point_to_the_motion_spec() {
    let remote = read(REMOTE_DOC);
    let page_design = section(&remote, "\n## 2e.");
    for needle in [
        "Docs/UI_MOTION.md",
        "Motion System for soos-gui and soos-remote",
        "prefers-reduced-motion",
        "startViewTransition",
        "--t-fast",
    ] {
        assert!(
            page_design.contains(needle),
            "{REMOTE_DOC} §2e must mention `{needle}`"
        );
    }
    assert!(
        read(GUI_DOC).contains("UI_MOTION.md"),
        "{GUI_DOC} must point to UI_MOTION.md"
    );
}
