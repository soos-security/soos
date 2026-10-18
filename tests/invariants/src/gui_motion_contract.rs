//! `soos-gui` motion system static contracts (GitHub #438 = PR 3 of #358; ADR 2026-10-13 "Motion
//! System for soos-gui and soos-remote" item (6); `AI/tester_contract_motion.md` Part B.2a; spec
//! `AI/architect_spec_gui_motion.md` §5; matrix GMO15–GMO19).
//!
//! - GMO15 zbus is used in `crates/gui/src` only by the portal reader `motion/portal.rs`, which
//!   opens one session connection and calls only `org.freedesktop.portal.Settings.ReadOne` /
//!   `Read` on `org.freedesktop.portal.Desktop`, with no object server, well-known name, match
//!   rule, proxy or signal stream;
//! - GMO16 Tokio is confined to the same file: one current-thread runtime on a named
//!   background thread, never a multi-thread runtime or a `#[tokio::main]`;
//! - GMO17 `crates/gui/Cargo.toml` takes `zbus` and `tokio` from the workspace (no feature
//!   override);
//! - GMO18 the portal reader logs nothing (the value is never logged);
//! - GMO19 the security guideline names the read-only portal exception and still names
//!   `soos-remote`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

const PORTAL: &str = "crates/gui/src/motion/portal.rs";
const GUI_SRC: &str = "crates/gui/src";
const GUI_MANIFEST: &str = "crates/gui/Cargo.toml";
const GUIDELINES: &str = "Docs/SECURITY_AND_QUALITY_GUIDELINES.md";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("Cannot read {}: {e}", path.display()))
}

/// Every `.rs` file under `rel` (recursive), as `(repo-relative path, content)`.
fn rust_files(rel: &str) -> Vec<(String, String)> {
    let root = workspace_root();
    let mut out = Vec::new();
    let mut stack = vec![root.join(rel)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let content = fs::read_to_string(&path).unwrap();
                let rel_path = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((rel_path, content));
            }
        }
    }
    out.sort();
    out
}

/// Code of a source file without line comments (doc comments included).
fn code(content: &str) -> String {
    content
        .lines()
        .map(|l| match l.find("//") {
            Some(i) if !l[..i].contains('"') => &l[..i],
            _ => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Lines of the `[name]` table of a TOML manifest, trimmed.
fn toml_table<'a>(manifest: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("[{name}]");
    let mut inside = false;
    let mut lines = Vec::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == header;
            continue;
        }
        if inside && !trimmed.is_empty() && !trimmed.starts_with('#') {
            lines.push(trimmed);
        }
    }
    lines
}

/// The string literal arguments of every `call_method(` in `src` (destination, path,
/// interface, method), as the source text between the parentheses.
fn call_method_args(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(pos) = rest.find("call_method(") {
        let after = &rest[pos + "call_method(".len()..];
        let mut depth = 1_usize;
        let mut end = after.len();
        for (i, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push(
            after[..end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        );
        rest = &after[end..];
    }
    out
}

#[test]
fn test_gmo_helper_self_test() {
    let src = "a.call_method(Some(DEST), PATH, Some(IFACE), method, &(ns, key)).await; // x\n\
               b.call_method(f(g(1)), h)";
    assert_eq!(
        call_method_args(src),
        vec![
            "Some(DEST), PATH, Some(IFACE), method, &(ns, key)".to_string(),
            "f(g(1)), h".to_string()
        ]
    );
    assert_eq!(
        code("let a = 1; // zbus\n/// zbus doc\nlet b = \"//x\";"),
        "let a = 1; \n\nlet b = \"//x\";"
    );
    let manifest =
        "[dependencies]\n# comment\nzbus = { workspace = true }\n\n[dev-dependencies]\nx = 1\n";
    assert_eq!(
        toml_table(manifest, "dependencies"),
        vec!["zbus = { workspace = true }"]
    );
}

/// GMO15: zbus only in the portal reader, only `ReadOne` / `Read` of the portal Settings
/// interface on `org.freedesktop.portal.Desktop`, one session connection, nothing served.
#[test]
fn test_gmo15_zbus_is_confined_to_the_portal_settings_reader() {
    let files = rust_files(GUI_SRC);
    assert!(files.len() > 10, "gui sources found: {}", files.len());
    for (rel, content) in &files {
        if rel != PORTAL {
            assert!(
                !code(content).contains("zbus"),
                "{rel}: zbus is only used by {PORTAL}"
            );
        }
    }
    let portal = read(PORTAL);
    let src = code(&portal);
    for needle in [
        "\"org.freedesktop.portal.Desktop\"",
        "\"/org/freedesktop/portal/desktop\"",
        "\"org.freedesktop.portal.Settings\"",
        "\"ReadOne\"",
        "\"Read\"",
        "\"org.freedesktop.appearance\"",
        "\"reduced-motion\"",
        "\"org.gnome.desktop.interface\"",
        "\"enable-animations\"",
    ] {
        assert_eq!(
            src.matches(needle).count(),
            1,
            "{PORTAL} declares {needle} exactly once"
        );
    }
    assert_eq!(
        src.matches("zbus::connection::Builder::session()").count(),
        1,
        "exactly one session connection builder"
    );
    let calls = call_method_args(&src);
    assert!(
        !calls.is_empty() && calls.len() <= 2,
        "one call site per method at most: {calls:?}"
    );
    for args in &calls {
        assert!(
            args.starts_with(
                "Some(PORTAL_DESTINATION), PORTAL_PATH, Some(PORTAL_SETTINGS_IFACE), "
            ),
            "every call targets the portal Settings interface: {args}"
        );
    }
    for forbidden in [
        "ObjectServer",
        "object_server",
        "serve_at",
        "request_name",
        "MatchRule",
        "add_match",
        "receive_signal",
        "SignalStream",
        "MessageStream",
        "#[proxy",
        "zbus::proxy",
        "#[interface",
        "zbus::blocking",
        "Builder::system",
        "Builder::address",
        "Connection::system",
        "Connection::session",
        "SettingChanged",
        "ReadAll",
    ] {
        assert!(
            !src.contains(forbidden),
            "{PORTAL} must not use `{forbidden}`"
        );
    }
}

/// GMO16: Tokio only on the portal reader thread: one current-thread runtime built on a named
/// `std::thread::Builder` thread; nothing else in `crates/gui/src` names Tokio.
#[test]
fn test_gmo16_tokio_runtime_only_on_the_reader_thread() {
    for (rel, content) in rust_files(GUI_SRC) {
        if rel != PORTAL {
            assert!(
                !code(&content).contains("tokio"),
                "{rel}: Tokio is only used by {PORTAL}"
            );
        }
    }
    let src = code(&read(PORTAL));
    assert_eq!(src.matches("new_current_thread()").count(), 1);
    assert_eq!(src.matches("std::thread::Builder::new()").count(), 1);
    assert!(src.contains(".name("), "the reader thread is named");
    for forbidden in [
        "new_multi_thread",
        "tokio::main",
        "tokio::spawn",
        "spawn_blocking",
        "Handle::current",
    ] {
        assert!(
            !src.contains(forbidden),
            "{PORTAL} must not use `{forbidden}`"
        );
    }
    assert!(
        src.contains("tokio::time::timeout("),
        "the whole read runs under one deadline"
    );
}

/// GMO17: the GUI manifest takes zbus and Tokio from the workspace without overrides.
#[test]
fn test_gmo17_gui_manifest_declares_workspace_zbus_and_tokio() {
    let manifest = read(GUI_MANIFEST);
    let deps = toml_table(&manifest, "dependencies");
    for line in [
        "zbus = { workspace = true }",
        "tokio = { workspace = true }",
    ] {
        assert_eq!(
            deps.iter().filter(|l| **l == line).count(),
            1,
            "{GUI_MANIFEST}: {line}"
        );
    }
    assert_eq!(manifest.matches("zbus").count(), 1);
    assert_eq!(manifest.matches("tokio").count(), 1);
}

/// GMO18: the portal reader logs nothing, so the desktop preference is never logged.
#[test]
fn test_gmo18_portal_value_is_never_logged() {
    let src = code(&read(PORTAL));
    for forbidden in [
        "tracing::",
        "log::",
        "println!",
        "eprintln!",
        "print!",
        "eprint!",
        "dbg!",
        "info!(",
        "debug!(",
        "warn!(",
        "error!(",
        "trace!(",
    ] {
        assert!(
            !src.contains(forbidden),
            "{PORTAL} must not log (`{forbidden}`)"
        );
    }
}

/// GMO19: the "Audited D-Bus client" guideline names the read-only `soos-gui` portal exception
/// and still names `soos-remote` (RMC check unchanged).
#[test]
fn test_gmo19_security_guideline_names_the_portal_exception() {
    let guidelines = read(GUIDELINES);
    let line = guidelines
        .lines()
        .find(|l| l.contains("Audited D-Bus client"))
        .expect("the Audited D-Bus client line");
    for needle in [
        "`zbus` 6",
        "soos-remote",
        "soos-gui",
        "org.freedesktop.portal.Settings",
        "ReadOne",
        "org.freedesktop.portal.Desktop",
        "session bus",
        "200 ms",
        "never logged",
    ] {
        assert!(
            line.contains(needle),
            "{GUIDELINES} D-Bus line must mention {needle}"
        );
    }
}
