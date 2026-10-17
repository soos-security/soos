//! GNOME Shell lock-screen extension contract (GitHub #405, PR 4 of #352; ADR 2026-10-17
//! "GNOME Shell Lock-Screen Extension"; spec `AI/architect_spec_lockscreen_gnome.md`; matrix
//! LKG1–LKG8).
//!
//! CI has no gnome-shell, so the extension under `packaging/gnome-shell/soos-lockscreen@soos/` is
//! checked statically:
//!
//! - `metadata.json`: uuid, session modes exactly `user` and `unlock-dialog`, a `shell-version`
//!   list of major versions starting at 45 (first ESM release);
//! - ESM shape, and a `node --check` syntax pass when `node` is installed (skipped otherwise);
//! - no unlock path, no keyboard or other input signal, a non-reactive actor;
//! - `Gio.Subprocess` with a fixed argv and absolute root-owned candidates, never a shell, no
//!   network or D-Bus;
//! - exactly three states and three fields, read with a bounded line length;
//! - `disable()` carries the review comment and removes every process, source, signal and actor;
//!   every callback is wrapped in `try`/`catch`;
//! - the `/etc/soos/lockwidget.disable` kill switch and `enable-animations` reduced motion;
//! - brand palette, motion tokens and star spikes mirror their sources; documentation and ADR;
//! - a change set touching the extension leaves PAM, the daemon, the protocol and the bridge alone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::print_stderr,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing; \
              skipped optional checks (no node, shallow clone) say so on stderr"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const EXT_DIR: &str = "packaging/gnome-shell/soos-lockscreen@soos";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn ext(file: &str) -> String {
    read(&format!("{EXT_DIR}/{file}"))
}

/// `extension.js` without `//` line comments and `/* */` block comments (string literals in the
/// extension never contain `//` or `/*`, which `test_lkg2_esm_shape_and_imports` pins).
fn js_code() -> String {
    let src = ext("extension.js");
    let mut out = String::with_capacity(src.len());
    let mut rest = src.as_str();
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find("*/").expect("closed block comment");
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out.lines()
        .map(|l| match l.find("//") {
            Some(i) if !l[..i].contains('\'') => &l[..i],
            _ => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whitespace-free form of a text (for JSON shape checks without a JSON parser).
fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Body of a 4-space-indented class method `name(...) {` up to its closing `    }` line.
fn method_body(src: &str, name: &str) -> String {
    let header = format!("\n    {name}(");
    let start = src
        .find(&header)
        .unwrap_or_else(|| panic!("method {name} missing"));
    let body = &src[start + 1..];
    let end = body.find("\n    }\n").expect("method end");
    body[..end].to_string()
}

#[test]
fn test_lkg1_metadata_uuid_session_modes_and_versions() {
    let meta = compact(&ext("metadata.json"));
    assert!(meta.starts_with('{') && meta.ends_with('}'), "{meta}");
    assert!(meta.contains("\"uuid\":\"soos-lockscreen@soos\""), "{meta}");
    assert!(
        meta.contains("\"session-modes\":[\"user\",\"unlock-dialog\"]"),
        "session modes must be exactly user and unlock-dialog: {meta}"
    );
    assert!(!meta.contains("gdm"), "never the greeter mode");
    assert!(meta.contains("\"name\":\"") && meta.contains("\"description\":\""));
    let key = "\"shell-version\":[";
    let start = meta.find(key).expect("shell-version") + key.len();
    let list = &meta[start..start + meta[start..].find(']').unwrap()];
    let versions: Vec<u32> = list
        .split(',')
        .map(|v| {
            let v = v.trim_matches('"');
            assert!(
                !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()),
                "major versions only: {list}"
            );
            v.parse().unwrap()
        })
        .collect();
    assert_eq!(versions.first(), Some(&45), "ESM starts at GNOME 45");
    assert!(
        versions.windows(2).all(|w| w[1] == w[0] + 1),
        "a contiguous range: {list}"
    );
    // The directory name is the uuid, and only the three extension files ship.
    let names: BTreeSet<String> = fs::read_dir(workspace_root().join(EXT_DIR))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let expected: BTreeSet<String> = ["extension.js", "metadata.json", "stylesheet.css"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert_eq!(names, expected);
}

#[test]
fn test_lkg2_esm_shape_and_imports() {
    let src = ext("extension.js");
    let code = js_code();
    assert!(code.contains("export default class "), "ESM default export");
    assert!(code.contains(" extends Extension {"));
    assert!(code.contains(
        "import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';"
    ));
    for line in code.lines().filter(|l| l.starts_with("import ")) {
        assert!(
            line.contains("from 'gi://") || line.contains("from 'resource:///org/gnome/shell/"),
            "only GI and shell modules: {line}"
        );
    }
    assert!(!code.contains("imports."), "no legacy imports object");
    assert!(!code.contains("require("), "no CommonJS");
    assert!(!code.contains("eval(") && !code.contains("new Function"));
    // String literals never hold a comment opener (keeps `js_code` exact).
    for line in src.lines() {
        if let Some(q) = line.find('\'') {
            let literal_part = &line[q..];
            assert!(
                !literal_part.contains("'//") && !literal_part.contains("/*'"),
                "{line}"
            );
        }
    }
}

#[test]
fn test_lkg2_syntax_check_with_node_when_available() {
    let available = Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !available {
        eprintln!("node not installed: syntax check skipped");
        return;
    }
    // `node --check` needs a module file name to parse `import`/`export`.
    let dir = std::env::temp_dir().join(format!("soos_lkg2_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("extension.mjs");
    fs::write(&copy, ext("extension.js")).unwrap();
    let out = Command::new("node")
        .arg("--check")
        .arg(&copy)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "node --check: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_lkg3_no_unlock_path_and_no_input_signal() {
    let src = ext("extension.js");
    for forbidden in [
        "Main.screenShield",
        "screenShield.",
        "ensureUnlockDialog",
        ".unlock(",
        ".deactivate(",
        ".activate(",
        "LoginManager",
        "AuthPrompt",
        "gi://Gdm",
        "UnlockSession",
        "key-press",
        "key-release",
        "key-focus",
        "keyboard",
        "Keyboard",
        "captured-event",
        "'event'",
        "button-press",
        "button-release",
        "motion-event",
        "scroll-event",
        "touch-event",
        "enter-event",
        "leave-event",
        "grab",
        "pushModal",
        "set_key_focus",
        "Clutter.InputDevice",
        "virtual",
        "reactive: true",
        "can_focus: true",
        "track_hover: true",
    ] {
        assert!(
            !src.contains(forbidden),
            "extension.js must not use {forbidden}"
        );
    }
    let code = js_code();
    for required in ["reactive: false", "can_focus: false", "track_hover: false"] {
        assert!(
            code.contains(required),
            "the card is created with {required}"
        );
    }
    assert!(code.contains("Main.layoutManager.screenShieldGroup"));
    assert!(code.contains("Main.sessionMode.isLocked"));
    // The only signals connected anywhere.
    let mut signals = BTreeSet::new();
    for (i, _) in code.match_indices(".connect('") {
        let rest = &code[i + ".connect('".len()..];
        signals.insert(rest[..rest.find('\'').unwrap()].to_string());
    }
    let allowed: BTreeSet<String> = ["updated", "notify::enable-animations", "repaint"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert_eq!(signals, allowed, "connected signals");
    assert_eq!(
        code.matches(".connect(").count(),
        code.matches(".connect('").count(),
        "every signal name is a literal"
    );
}

#[test]
fn test_lkg4_subprocess_without_shell_and_no_network() {
    let src = ext("extension.js");
    let code = js_code();
    for forbidden in [
        "GLib.spawn",
        "spawn_command_line",
        "spawn_async",
        "Util.spawn",
        "trySpawn",
        "/bin/sh",
        "'sh'",
        "bash",
        "'-c'",
        "SubprocessLauncher",
        "'PATH'",
        "getenv",
        "get_home_dir",
        "Soup",
        "DBus",
        "dbus",
        "Gio.Settings",
        "getSettings",
        "Gio.Socket",
        "http",
    ] {
        assert!(
            !code.contains(forbidden),
            "extension.js must not use {forbidden}"
        );
    }
    assert!(!src.contains("http://"), "no plain-http reference");
    assert_eq!(
        code.matches("Gio.Subprocess.new(").count(),
        1,
        "one spawn site"
    );
    assert!(code.contains("Gio.Subprocess.new([path, '--watch'],"));
    assert!(code.contains("Gio.SubprocessFlags.STDOUT_PIPE"));
    assert!(code.contains("Gio.SubprocessFlags.STDERR_SILENCE"));
    assert!(code.contains(
        "const SOOS_LOCKSTATUS_CANDIDATES = ['/usr/bin/soos-lockstatus', '/usr/local/bin/soos-lockstatus'];"
    ));
    for check in [
        "'unix::uid'",
        "'unix::mode'",
        "'access::can-execute'",
        "0o022",
    ] {
        assert!(code.contains(check), "binary check uses {check}");
    }
    assert!(code.contains("force_exit()"));
}

#[test]
fn test_lkg5_three_states_three_fields_bounded_lines() {
    let code = js_code();
    assert!(code.contains("const WATCH_KEYS = ['available', 'sequence', 'state'];"));
    assert!(code.contains("const STATES = ['idle', 'scanning', 'not_recognized'];"));
    assert!(code.contains("const MAX_LINE_BYTES = 256;"));
    assert!(code.contains("const READ_CHUNK_BYTES = 256;"));
    assert!(code.contains("read_bytes_async(READ_CHUNK_BYTES,"));
    for forbidden in [
        "read_line",
        "DataInputStream",
        "read_upto",
        "communicate",
        "splice",
    ] {
        assert!(!code.contains(forbidden), "unbounded read {forbidden}");
    }
    // Only the three state words and the three keys appear as quoted words describing the view.
    for forbidden in [
        "'recognized'",
        "'unlocked'",
        "'success'",
        "'paused'",
        "'disabled'",
        "'unavailable'",
        ".reason",
        ".score",
        ".uid",
        ".frame",
        ".embedding",
    ] {
        assert!(
            !code.contains(forbidden),
            "extension.js must not use {forbidden}"
        );
    }
    let mut quoted_states = BTreeSet::new();
    for word in ["'idle'", "'scanning'", "'not_recognized'"] {
        if code.contains(word) {
            quoted_states.insert(word);
        }
    }
    assert_eq!(quoted_states.len(), 3);
    for words in ["'Scanning'", "'Not recognized'"] {
        assert!(code.contains(words), "label {words}");
    }
    // `available: false` renders like Idle.
    assert!(code.contains("view.available ? view.state : 'idle'"));
}

#[test]
fn test_lkg6_disable_cleans_up_and_callbacks_are_guarded() {
    let src = ext("extension.js");
    let disable = method_body(&src, "disable");
    assert!(
        disable.contains("unlock-dialog"),
        "disable() explains the unlock-dialog use"
    );
    for call in [
        "this._stopBridge()",
        "this._destroyCard()",
        "this._removeSources()",
        ".disconnect(",
    ] {
        assert!(disable.contains(call), "disable() calls {call}");
    }
    let stop = method_body(&src, "_stopBridge");
    for call in [".cancel()", "force_exit()"] {
        assert!(stop.contains(call), "_stopBridge() calls {call}");
    }
    assert!(method_body(&src, "_destroyCard").contains(".destroy()"));
    assert!(method_body(&src, "_removeSources").contains("GLib.Source.remove("));
    // Every arrow callback has a block body that starts with `try {`.
    let code = js_code();
    let lines: Vec<&str> = code.lines().collect();
    assert_eq!(
        code.matches("=>").count(),
        code.matches("=> {").count(),
        "no expression-bodied arrow function"
    );
    for (i, line) in lines.iter().enumerate() {
        if line.trim_end().ends_with("=> {") {
            let next = lines[i + 1..]
                .iter()
                .find(|l| !l.trim().is_empty())
                .unwrap();
            assert_eq!(next.trim(), "try {", "callback at line {}", i + 1);
        }
    }
    // Every `_on*` handler and every lifecycle method guards its body.
    for name in ["enable", "_sync"] {
        let body = method_body(&src, name);
        assert_eq!(body.lines().nth(1).map(str::trim), Some("try {"), "{name}");
    }
    let mut handlers = 0;
    for (i, _) in code.match_indices("\n    _on") {
        let body = &code[i + 1..];
        let second = body.lines().nth(1).map(str::trim);
        assert_eq!(second, Some("try {"), "{}", body.lines().next().unwrap());
        handlers += 1;
    }
    assert!(handlers >= 4, "handlers found: {handlers}");
    // No exception text or bridge output is logged.
    for log in ["console.log", "log(", "logError"] {
        assert!(!code.contains(log), "extension.js must not use {log}");
    }
    for line in code.lines().filter(|l| l.contains("console.warn(")) {
        let arg = &line[line.find("console.warn(").unwrap() + "console.warn(".len()..];
        assert!(arg.starts_with('\''), "fixed message only: {line}");
        assert!(!arg.contains('+') && !arg.contains("${"), "{line}");
    }
}

#[test]
fn test_lkg7_kill_switch_and_reduced_motion() {
    let code = js_code();
    assert!(code.contains("const KILL_SWITCH_PATH = '/etc/soos/lockwidget.disable';"));
    assert!(code.contains("GLib.file_test(KILL_SWITCH_PATH, GLib.FileTest.EXISTS)"));
    let sync = method_body(&code, "_sync");
    assert!(
        sync.contains("this._killSwitchSet()") && sync.contains("Main.sessionMode.isLocked"),
        "_sync checks the kill switch before building"
    );
    assert!(code.contains("St.Settings.get()"));
    assert!(code.contains("enable_animations"));
    assert!(code.contains("'soos-reduced-motion'"));
    let css = ext("stylesheet.css");
    let reduced = &css[css.find(".soos-reduced-motion").expect("reduced rule")..];
    let reduced = &reduced[..reduced.find('}').unwrap()];
    assert!(
        compact(reduced).contains("transition-duration:0ms"),
        "{reduced}"
    );
}

#[test]
fn test_lkg7_extension_change_set_leaves_pam_daemon_protocol_and_bridge_alone() {
    let root = workspace_root();
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    if git(&["rev-parse", "--is-shallow-repository"]).as_deref() != Some("false") {
        eprintln!("shallow or no git checkout: change-set check skipped");
        return;
    }
    let Some(base) = git(&["merge-base", "HEAD", "origin/main"]) else {
        eprintln!("no origin/main: change-set check skipped");
        return;
    };
    let changed = git(&["diff", "--name-only", &base]).unwrap_or_default();
    let untracked = git(&["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    let files: Vec<&str> = changed.lines().chain(untracked.lines()).collect();
    if !files
        .iter()
        .any(|f| f.starts_with("packaging/gnome-shell/"))
    {
        return;
    }
    for file in files {
        for guarded in [
            "crates/pam/",
            "crates/daemon/",
            "crates/protocol/",
            "crates/lockstatus/",
        ] {
            assert!(
                !file.starts_with(guarded),
                "a GNOME extension change set must not touch {file}"
            );
        }
    }
}

/// `pub const NAME: Color32 = Color32::from_rgb(0x.., 0x.., 0x..);` → `#rrggbb`.
fn theme_hex(theme: &str, name: &str) -> String {
    let prefix = format!("pub const {name}: Color32 = Color32::from_rgb(");
    let line = theme
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("theme.rs {name}"));
    let inner = &line[prefix.len()..line.find(')').unwrap()];
    let mut hex = String::from("#");
    for part in inner.split(',') {
        hex.push_str(&part.trim().trim_start_matches("0x").to_ascii_lowercase());
    }
    hex
}

#[test]
fn test_lkg8_palette_tokens_and_star_mirror_their_sources() {
    let theme = read("crates/gui/src/theme.rs");
    let css = ext("stylesheet.css").to_ascii_lowercase();
    let mut palette = BTreeSet::new();
    for name in ["BLUE", "PALE", "PALE_4", "PINK", "WARN_BORDER"] {
        let hex = theme_hex(&theme, name);
        assert!(css.contains(&hex), "stylesheet.css uses {name} {hex}");
        palette.insert(hex);
    }
    for (i, _) in css.match_indices('#') {
        let hex: String = css[i..].chars().take(7).collect();
        if hex[1..].chars().all(|c| c.is_ascii_hexdigit()) && hex.len() == 7 {
            assert!(palette.contains(&hex), "colour {hex} is not a brand colour");
        }
    }
    for rule in [
        ".soos-lock-ring.soos-idle",
        ".soos-lock-ring.soos-scanning",
        ".soos-lock-ring.soos-not-recognized",
        ".soos-lock-star",
        ".soos-lock-label",
        ".soos-lock-card",
    ] {
        assert!(css.contains(rule), "stylesheet.css rule {rule}");
    }
    assert!(compact(&css).contains("transition-duration:180ms"));
    let doc = read("Docs/UI_MOTION.md");
    let code = js_code();
    for (css_name, rust, value, ours) in [
        ("--t-short", "T_SHORT", "180", "T_SHORT_MS"),
        ("--t-medium", "T_MEDIUM", "260", "T_MEDIUM_MS"),
        ("--t-pending", "T_PENDING", "1200", "T_PENDING_MS"),
        ("--slide", "SLIDE_PX", "8", "SLIDE_PX"),
    ] {
        let unit = if css_name == "--slide" { "px" } else { "ms" };
        assert!(doc.contains(&format!("| `{css_name}` | `{rust}` | `{value}{unit}` |")));
        assert!(
            code.contains(&format!("const {ours} = {value};")),
            "extension.js {ours}"
        );
    }
    assert!(code.contains("const PENDING_LOOP_MAX_MS = 4800;"));
    assert!(read("crates/lockstatus/src/motion.rs")
        .contains("pub const PENDING_LOOP_MAX_MS: u64 = 4800;"));
    let brand = read("crates/gui/src/brand.rs");
    assert!(brand.contains("C(254.0, 140.28, 367.72, 254.0, 508.0, 254.0)"));
    assert!(brand.contains("C(140.28, 254.0, 254.0, 367.72, 254.0, 508.0)"));
    assert!(code.contains("cr.curveTo(254, 140.28, 367.72, 254, 508, 254);"));
    assert!(code.contains("cr.curveTo(140.28, 254, 254, 367.72, 254, 508);"));
    assert!(code.contains("const STAR_VIEWBOX = 508;"));
}

#[test]
fn test_lkg8_documentation_and_adr_are_present() {
    let doc = read("Docs/LOCKSCREEN_WIDGET.md");
    for needle in [
        "soos-lockscreen@soos",
        "/usr/share/gnome-shell/extensions",
        "gnome-extensions enable soos-lockscreen@soos",
        "/etc/soos/lockwidget.disable",
        "enable-animations",
        "unlock-dialog",
    ] {
        assert!(
            doc.contains(needle),
            "LOCKSCREEN_WIDGET.md must mention {needle}"
        );
    }
    assert!(read("Docs/DISTRIBUTION_DEPLOYMENT.md").contains("soos-lockscreen@soos"));
    let decisions = read("AI/DECISIONS.md");
    assert!(decisions.contains("GNOME Shell Lock-Screen Extension"));
    let matrix = read("AI/VERIFICATION_MATRIX.md");
    for row in 1..=10 {
        assert!(
            matrix.contains(&format!("| LKG{row} |")),
            "matrix row LKG{row}"
        );
    }
    assert!(workspace_root()
        .join("AI/walkthroughs/215_lockscreen_gnome.md")
        .is_file());
}
