//! Lock-screen adapters contract (GitHub #401, PR 2 of #352; ADR 2026-10-15 "Lock-Screen
//! Adapters for swaylock-plugin and hyprlock"; spec `AI/architect_spec_lockscreen_adapters.md`;
//! matrix LKA11, LKA12).
//!
//! - both new crates forbid `unsafe` in their library and binary roots and have no panic or
//!   print-macro path in production code;
//! - dependency allow-lists: no async runtime, D-Bus, network, font or image crate; the only
//!   Wayland dependency is the already locked `smithay-client-toolkit` 0.20 with `calloop` only;
//! - the widget binds no seat (no keyboard, pointer or touch) and draws a background layer;
//! - the coordination file is private and checked (`O_NOFOLLOW`, owner, `0o077`, one link, flock);
//! - brand colours mirror `crates/gui/src/theme.rs` and motion tokens mirror `Docs/UI_MOTION.md`;
//! - the hyprlock snippet polls `--once` at 2 Hz or slower; documentation and ADR are present.

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

const CRATES: [&str; 2] = ["crates/lockstatus", "crates/lock-widget"];

fn workspace_root() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    Path::new(&manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Production part of a source file: before the first `#[cfg(test)]`, without comment lines.
fn production_code(src: &str) -> String {
    src.split("#[cfg(test)]")
        .next()
        .unwrap_or(src)
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn sources(crate_dir: &str) -> Vec<(String, String)> {
    let dir = workspace_root().join(crate_dir).join("src");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let rel = format!(
                "{crate_dir}/src/{}",
                path.file_name().unwrap().to_string_lossy()
            );
            out.push((rel, fs::read_to_string(&path).unwrap()));
        }
    }
    assert!(!out.is_empty(), "{crate_dir} has no source");
    out.sort();
    out
}

/// Keys of one `[table]` of a manifest (`name = ...` lines until the next table).
fn table_keys(manifest: &str, table: &str) -> BTreeSet<String> {
    let header = format!("[{table}]");
    let Some(start) = manifest.find(&header) else {
        return BTreeSet::new();
    };
    manifest[start + header.len()..]
        .lines()
        .map(str::trim)
        .take_while(|l| !l.starts_with('['))
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('=').next().unwrap().trim().to_string())
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn test_lka12_both_crate_roots_forbid_unsafe() {
    for krate in CRATES {
        for root in ["lib.rs", "main.rs"] {
            let rel = format!("{krate}/src/{root}");
            let src = read(&rel);
            let first_code = src
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with("//"))
                .unwrap();
            assert_eq!(first_code, "#![forbid(unsafe_code)]", "{rel}");
        }
        for (rel, src) in sources(krate) {
            for line in production_code(&src).lines() {
                assert!(
                    !line.contains("unsafe") || line.contains("forbid(unsafe_code)"),
                    "{rel}: {line}"
                );
            }
        }
    }
    let list = read("tests/invariants/src/lib.rs");
    let start = list
        .find("fn test_business_crates_forbid_unsafe_code")
        .unwrap();
    let list = &list[start..start + list[start..].find("];").unwrap()];
    assert!(list.contains("\"lockstatus\"") && list.contains("\"lock-widget\""));
    let candid = read("scripts/candid_review.sh");
    let line = candid
        .lines()
        .find(|l| l.starts_with("BUSINESS_CRATES=("))
        .unwrap();
    assert!(
        line.contains(" lockstatus") && line.contains(" lock-widget"),
        "{line}"
    );
}

#[test]
fn test_lka12_no_panic_or_print_macro_in_production_code() {
    let forbidden = [
        ".unwrap()",
        ".expect(",
        "panic!(",
        "todo!(",
        "unimplemented!(",
        "unreachable!(",
        "println!(",
        "print!(",
        "eprintln!(",
        "eprint!(",
        "dbg!(",
    ];
    for krate in CRATES {
        for (rel, src) in sources(krate) {
            let code = production_code(&src);
            for pattern in forbidden {
                assert!(!code.contains(pattern), "{rel} contains `{pattern}`");
            }
        }
    }
}

#[test]
fn test_lka12_dependency_allow_lists() {
    let lockstatus = read("crates/lockstatus/Cargo.toml");
    assert_eq!(
        table_keys(&lockstatus, "dependencies"),
        set(&["soos-protocol", "nix", "getrandom", "thiserror", "clap"])
    );
    assert_eq!(
        table_keys(&lockstatus, "dev-dependencies"),
        set(&["tempfile"])
    );
    let widget = read("crates/lock-widget/Cargo.toml");
    assert_eq!(
        table_keys(&widget, "dependencies"),
        set(&["soos-lockstatus", "smithay-client-toolkit", "clap"])
    );
    assert_eq!(table_keys(&widget, "dev-dependencies"), set(&["tempfile"]));
    for (name, manifest) in [("lockstatus", &lockstatus), ("lock-widget", &widget)] {
        for banned in [
            "tokio",
            "zbus",
            "soos-remote",
            "ureq",
            "reqwest",
            "hyper",
            "rustls",
            "tiny-skia",
            "fontdue",
            "ab_glyph",
            "image",
            "png",
            "opencv",
            "tracing",
        ] {
            assert!(!manifest.contains(banned), "{name} manifest names {banned}");
        }
        for inherited in [
            "version.workspace = true",
            "edition.workspace = true",
            "rust-version.workspace = true",
            "license.workspace = true",
            "publish.workspace = true",
        ] {
            assert!(manifest.contains(inherited), "{name}: {inherited}");
        }
        assert!(manifest.contains("[lints]\nworkspace = true"), "{name}");
    }
    let root = read("Cargo.toml");
    assert!(root.contains(
        "smithay-client-toolkit = { version = \"0.20\", default-features = false, features = [\"calloop\"] }"
    ));
    for member in ["\"crates/lockstatus\",", "\"crates/lock-widget\","] {
        assert!(root.contains(member), "{member}");
    }
    // The widget reuses the locked 0.20.0; no other new third-party package.
    let lock = read("Cargo.lock");
    let widget_pkg = &lock[lock.find("name = \"soos-lock-widget\"").unwrap()..];
    let widget_pkg = &widget_pkg[..widget_pkg.find("\n\n").unwrap()];
    assert!(
        widget_pkg.contains("\"smithay-client-toolkit 0.20.0\""),
        "{widget_pkg}"
    );
}

#[test]
fn test_lka12_widget_binds_no_seat_and_draws_a_background_layer() {
    let mut all = String::new();
    for (_, src) in sources("crates/lock-widget") {
        all.push_str(&production_code(&src));
    }
    for forbidden in [
        "SeatState",
        "wl_seat",
        "wl_keyboard",
        "wl_pointer",
        "wl_touch",
        "delegate_seat",
        "delegate_keyboard",
        "delegate_pointer",
        "KeyboardInteractivity::Exclusive",
        "KeyboardInteractivity::OnDemand",
        "session_lock",
        "UnlockSession",
    ] {
        assert!(
            !all.contains(forbidden),
            "the widget must not use {forbidden}"
        );
    }
    for required in [
        "KeyboardInteractivity::None",
        "Layer::Background",
        "SETUP_FAILURE_EXIT_DELAY_MS",
    ] {
        assert!(all.contains(required), "the widget must use {required}");
    }
}

#[test]
fn test_lka12_no_network_and_one_socket_literal() {
    for krate in CRATES {
        for (rel, src) in sources(krate) {
            let code = production_code(&src);
            for net in ["TcpStream", "UdpSocket", "TcpListener", "std::net::"] {
                assert!(!code.contains(net), "{rel}: {net}");
            }
            let literal = code.matches("\"/run/soos").count();
            let expected = usize::from(rel == "crates/lockstatus/src/lib.rs");
            assert_eq!(literal, expected, "{rel}: daemon socket literal");
        }
    }
    assert!(read("crates/lockstatus/src/lib.rs")
        .contains("pub const DEFAULT_SOCKET_PATH: &str = \"/run/soos/daemon.sock\";"));
}

#[test]
fn test_lka12_coordination_file_is_private_and_checked() {
    let shared = production_code(&read("crates/lockstatus/src/shared.rs"));
    for required in [
        "O_NOFOLLOW",
        "O_CLOEXEC",
        "0o077",
        "nlink",
        "symlink_metadata",
        "LockExclusiveNonblock",
        "CLOCK_BOOTTIME",
    ] {
        assert!(shared.contains(required), "shared.rs must use {required}");
    }
    assert!(
        shared.contains(".mode(0o600)"),
        "the lock file is created 0600"
    );
}

#[test]
fn test_lka12_stderr_is_only_written_by_the_binaries() {
    for krate in CRATES {
        for (rel, src) in sources(krate) {
            if rel.ends_with("/main.rs") || rel.ends_with("/wayland.rs") {
                continue;
            }
            assert!(
                !production_code(&src).contains("stderr"),
                "{rel} must not write to stderr"
            );
        }
    }
}

/// `pub const NAME: Color32 = Color32::from_rgb(0x.., 0x.., 0x..);` → `[r, g, b]`.
fn theme_rgb(theme: &str, name: &str) -> [u8; 3] {
    let prefix = format!("pub const {name}: Color32 = Color32::from_rgb(");
    let line = theme
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("theme.rs {name}"));
    parse_triplet(&line[prefix.len()..])
}

fn parse_triplet(rest: &str) -> [u8; 3] {
    let inner = &rest[..rest.find(')').unwrap()];
    let v: Vec<u8> = inner
        .split(',')
        .map(|s| u8::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap())
        .collect();
    [v[0], v[1], v[2]]
}

#[test]
fn test_lka11_palette_mirrors_the_gui_theme() {
    let theme = read("crates/gui/src/theme.rs");
    let palette = read("crates/lock-widget/src/palette.rs");
    for name in ["BLUE", "PALE", "PALE_4", "PINK", "WARN_BORDER"] {
        let prefix = format!("pub const {name}: Rgb = Rgb(");
        let line = palette
            .lines()
            .find(|l| l.starts_with(&prefix))
            .unwrap_or_else(|| panic!("palette.rs {name}"));
        assert_eq!(
            parse_triplet(&line[prefix.len()..]),
            theme_rgb(&theme, name),
            "{name}"
        );
    }
}

fn motion_const(src: &str, name: &str) -> String {
    let prefix = format!("pub const {name}: ");
    let line = src
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("motion.rs {name}"));
    let value = line.split('=').nth(1).unwrap();
    value.trim().trim_end_matches(';').to_string()
}

#[test]
fn test_lka11_motion_tokens_mirror_the_normative_table() {
    let doc = read("Docs/UI_MOTION.md");
    let motion = read("crates/lockstatus/src/motion.rs");
    for (css, rust, value, ours, ours_value) in [
        ("--t-short", "T_SHORT", "180ms", "T_SHORT_MS", "180"),
        ("--t-medium", "T_MEDIUM", "260ms", "T_MEDIUM_MS", "260"),
        ("--t-pending", "T_PENDING", "1200ms", "T_PENDING_MS", "1200"),
        ("--slide", "SLIDE_PX", "8px", "SLIDE_PX", "8.0"),
    ] {
        let row = format!("| `{css}` | `{rust}` | `{value}` |");
        assert!(doc.contains(&row), "UI_MOTION.md row {row}");
        assert_eq!(motion_const(&motion, ours), ours_value, "{ours}");
    }
    assert!(doc.contains("| `--ease-standard` | `EASE_STANDARD` | `cubic-bezier(0.2, 0, 0, 1)` |"));
    assert_eq!(
        motion_const(&motion, "EASE_STANDARD"),
        "[0.2, 0.0, 0.0, 1.0]"
    );
    assert!(doc.contains("33 ms (`AMBIENT_REPAINT_MS`"));
    assert_eq!(motion_const(&motion, "AMBIENT_REPAINT_MS"), "33");
    assert!(doc.contains("`SOOS_REDUCE_MOTION`"));
    assert_eq!(
        motion_const(&motion, "REDUCE_MOTION_ENV"),
        "\"SOOS_REDUCE_MOTION\""
    );
}

#[test]
fn test_lka12_hyprlock_snippet_polls_once_at_two_hertz_or_slower() {
    let snippet = read("packaging/lockscreen/hyprlock-soos.conf");
    let commands: Vec<&str> = snippet
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && l.contains("cmd[update:"))
        .collect();
    assert!(!commands.is_empty());
    for line in commands {
        let after = &line[line.find("cmd[update:").unwrap() + "cmd[update:".len()..];
        let period: u64 = after[..after.find(|c: char| !c.is_ascii_digit()).unwrap()]
            .parse()
            .unwrap();
        assert!(period >= 500, "{line}");
        assert!(line.contains("soos-lockstatus --once"), "{line}");
        assert!(
            !line.contains("--watch") && !line.contains("json"),
            "{line}"
        );
    }
}

#[test]
fn test_lka12_documentation_and_adr_are_present() {
    let doc = read("Docs/LOCKSCREEN_WIDGET.md");
    for needle in [
        "soos-lockstatus --once",
        "--watch",
        "swaylock-plugin --command",
        "soos-lock-widget",
        "--reduce-motion",
        "SOOS_REDUCE_MOTION",
        "Scanning",
        "Not recognized",
        "hyprlock-soos.conf",
        "soos-lockstatus.lock",
        "XDG_RUNTIME_DIR",
    ] {
        assert!(
            doc.contains(needle),
            "LOCKSCREEN_WIDGET.md must mention {needle}"
        );
    }
    assert!(read("Docs/README.md").contains("LOCKSCREEN_WIDGET.md"));
    assert!(
        read("AI/DECISIONS.md").contains("Lock-Screen Adapters for swaylock-plugin and hyprlock")
    );
    for layout in ["AGENTS.md", "AI/ARCHITECTURE.md"] {
        let text = read(layout);
        assert!(text.contains("lockstatus/"), "{layout}");
        assert!(text.contains("lock-widget/"), "{layout}");
    }
}
