//! Remote companion static contracts (GitHub #339, architect spec §8 RMC-S1–RMC-S12 and
//! RMC-S7b plus the plan-evaluator round-3 needles R3-2; matrix rows RMC*).
//!
//! - RMC-S1 `crates/remote/src/{lib,main}.rs` forbid `unsafe`;
//! - RMC-S2 no TCP/UDP type is ever named in the crate;
//! - RMC-S3 no `Unlock` / `SetLockedHint` / session-ending string literal (D10); since the
//!   ADR 2026-10-06 "Remote Unlock in soos-remote", exactly one `UnlockSession` literal, in
//!   `logind.rs` (as for `LockSession`);
//! - RMC-S4 `soos-remote` is a leaf: nothing depends on it, it depends on no daemon crate;
//! - RMC-S5 / RMC-S11 the user unit is sandboxed and never retries configuration errors;
//! - RMC-S6 the installer refuses root and never runs `sudo`, `tailscale` or enables the unit;
//! - RMC-S7 the operator documentation exists and covers the Tailscale and `LockedHint` facts;
//! - RMC-S8 the web assets load nothing remote and contain no inline script;
//! - RMC-S9 the bus rules of the presence worker apply verbatim (pinned address only, no
//!   proxy, no cache, no environment-derived bus, no signal stream, no blocking API);
//! - RMC-S10 `main` refuses root before reading any configuration;
//! - RMC-S12 both installer scripts are executable in the checkout (R4-5);
//! - RMC-S7b the documentation describes the D5a′ effective host and nothing is pending;
//! - workspace registration, lint hygiene and logging hygiene of the new crate.

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
use std::process::Command;

pub(crate) fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

pub(crate) fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("Cannot read {}: {e}", path.display()))
}

fn read_bytes(rel: &str) -> Vec<u8> {
    let path = workspace_root().join(rel);
    fs::read(&path).unwrap_or_else(|e| panic!("Cannot read {}: {e}", path.display()))
}

pub(crate) fn exists(rel: &str) -> bool {
    workspace_root().join(rel).is_file()
}

/// Every `.rs` file under `rel` (recursive), as `(repo-relative path, content)`.
pub(crate) fn rust_files(rel: &str) -> Vec<(String, String)> {
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

/// Production sources of the companion (`crates/remote/src/**`), at least the §2.2 modules.
fn remote_sources() -> Vec<(String, String)> {
    let files = rust_files("crates/remote/src");
    for module in [
        "lib.rs",
        "main.rs",
        "config.rs",
        "identity.rs",
        "http.rs",
        "routes.rs",
        "session.rs",
        "logind.rs",
        "status.rs",
        "server.rs",
        "socket.rs",
        "assets.rs",
    ] {
        assert!(
            files
                .iter()
                .any(|(rel, _)| rel == &format!("crates/remote/src/{module}")),
            "crates/remote/src/{module} must exist (spec §2.2)"
        );
    }
    files
}

/// `src` with `//` line comments and (nested) `/* */` block comments removed; string and
/// char literals are preserved verbatim.
pub(crate) fn strip_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' {
            // String literal (escapes honoured).
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push_str(&src[start..i.min(bytes.len())]);
            continue;
        }
        if c == b'\'' {
            // Char literal or lifetime.
            if i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                let start = i;
                i += 2;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                i += 1;
                out.push_str(&src[start..i.min(bytes.len())]);
                continue;
            }
            if i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
                out.push_str(&src[i..i + 3]);
                i += 3;
                continue;
            }
            out.push('\'');
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            i += 2;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

/// Production part of a source file: everything before a trailing `#[cfg(test)] mod … { … }`
/// block (any other `#[cfg(test)]` hides nothing).
pub(crate) fn production_part(content: &str) -> &str {
    let mut search = 0;
    while let Some(pos) = content[search..].find("#[cfg(test)]") {
        let idx = search + pos;
        let rest = &content[idx + "#[cfg(test)]".len()..];
        let item = rest.trim_start();
        if item.starts_with("mod ")
            || item.starts_with("pub mod ")
            || item.starts_with("pub(crate) mod ")
        {
            if let Some(open) = item.find('{') {
                let mut depth = 0usize;
                let mut end = None;
                for (i, c) in item[open..].char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(open + i);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(end) = end {
                    if item[end + 1..].trim().is_empty() {
                        return &content[..idx];
                    }
                }
            }
        }
        search = idx + "#[cfg(test)]".len();
    }
    content
}

/// Body of the TOML table `[name]` (until the next table header).
pub(crate) fn toml_table<'a>(manifest: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("[{name}]");
    let mut out = Vec::new();
    let mut inside = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == header;
            continue;
        }
        if inside && !trimmed.is_empty() && !trimmed.starts_with('#') {
            out.push(trimmed);
        }
    }
    out
}

fn first_code_line(content: &str) -> &str {
    content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("//"))
        .unwrap_or("")
}

/// Arguments of every `macro!(...)` invocation in `code` (balanced parentheses, strings
/// skipped), for the given macro names.
pub(crate) fn macro_invocations(code: &str, macros: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for name in macros {
        let needle = format!("{name}!(");
        let mut start = 0;
        while let Some(pos) = code[start..].find(&needle) {
            let at = start + pos;
            let preceded_by_ident = at > 0 && {
                let b = code.as_bytes()[at - 1];
                b.is_ascii_alphanumeric() || b == b'_' || b == b':'
            };
            let open = at + needle.len();
            let mut depth = 1usize;
            let mut in_string = false;
            let mut escaped = false;
            let mut end = open;
            for (i, c) in code[open..].char_indices() {
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        in_string = false;
                    }
                    continue;
                }
                match c {
                    '"' => in_string = true,
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if !preceded_by_ident {
                out.push(code[open..end].to_string());
            }
            start = end.max(open);
        }
    }
    out
}

/// Field keys of a tracing macro argument list (`key = value`, `%key`, `?key`, `key`).
pub(crate) fn field_keys(args: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut current = String::new();
    let mut parts = Vec::new();
    for c in args.chars() {
        if in_string {
            current.push(c);
            if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                current.push(c);
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    for part in parts {
        let part = part.trim();
        if part.is_empty() || part.starts_with('"') {
            continue;
        }
        let key = part.split('=').next().unwrap_or("").trim();
        let key = key.trim_start_matches(['%', '?']).trim();
        let key = key.rsplit('.').next().unwrap_or(key);
        keys.push(key.to_lowercase());
    }
    keys
}

pub(crate) const TRACING_MACROS: [&str; 10] = [
    "trace",
    "debug",
    "info",
    "warn",
    "error",
    "tracing::trace",
    "tracing::debug",
    "tracing::info",
    "tracing::warn",
    "tracing::error",
];

// ---------------------------------------------------------------------------------------
// Workspace registration and manifest shape (spec §1.1)
// ---------------------------------------------------------------------------------------

#[test]
fn test_rmc_crate_is_registered_in_the_workspace() {
    let root = read("Cargo.toml");
    let members_start = root.find("members = [").expect("members");
    let members_end = members_start + root[members_start..].find(']').expect("members end");
    let members = &root[members_start..members_end];
    assert!(
        members.contains("\"crates/remote\""),
        "root Cargo.toml must list crates/remote as a workspace member"
    );
    let deps = toml_table(&root, "workspace.dependencies");
    assert!(
        deps.contains(&"httparse = \"1.10\""),
        "[workspace.dependencies] must pin httparse = \"1.10\" (already locked through ureq)"
    );
    assert!(
        deps.contains(&"soos-remote = { path = \"crates/remote\", version = \"0.1.0\" }"),
        "[workspace.dependencies] must declare soos-remote"
    );
    let lock = read("Cargo.lock");
    for forbidden in ["name = \"hyper\"", "name = \"axum\"", "name = \"tower\""] {
        assert!(
            !lock.contains(forbidden),
            "no HTTP framework enters the lockfile (spec §0): {forbidden}"
        );
    }
    assert_eq!(
        lock.matches("name = \"httparse\"").count(),
        1,
        "one httparse version in Cargo.lock"
    );
}

#[test]
fn test_rmc_manifest_inherits_workspace_keys_and_test_util() {
    let manifest = read("crates/remote/Cargo.toml");
    let package = toml_table(&manifest, "package");
    assert!(package.contains(&"name = \"soos-remote\""), "{package:?}");
    for key in [
        "version.workspace = true",
        "edition.workspace = true",
        "license.workspace = true",
        "publish.workspace = true",
    ] {
        assert!(
            package.contains(&key),
            "crates/remote/Cargo.toml [package] needs `{key}`"
        );
    }
    assert!(
        manifest.contains("[lints]\nworkspace = true"),
        "crates/remote/Cargo.toml must opt into the workspace lints"
    );
    let bins = toml_table(&manifest, "[bin]");
    assert!(
        bins.contains(&"name = \"soos-remote\""),
        "the binary is named soos-remote: {bins:?}"
    );
    let deps = toml_table(&manifest, "dependencies");
    assert!(
        deps.contains(&"httparse = { workspace = true }")
            || deps.contains(&"httparse.workspace = true"),
        "httparse comes from the workspace: {deps:?}"
    );
    let dev = toml_table(&manifest, "dev-dependencies");
    assert!(
        dev.iter().any(|l| l.starts_with("tokio = {")
            && l.contains("workspace = true")
            && l.contains("\"test-util\"")),
        "dev-dependency tokio with the test-util feature (paused-time tests): {dev:?}"
    );
    assert!(
        dev.iter().any(|l| l.starts_with("tempfile")),
        "dev-dependency tempfile: {dev:?}"
    );
}

#[test]
fn test_rmc_forbid_unsafe_list_and_review_tooling_include_remote() {
    let invariants = read("tests/invariants/src/lib.rs");
    let list_start = invariants
        .find("let business_crates = [")
        .expect("business_crates list in test_business_crates_forbid_unsafe_code");
    let list_end = list_start + invariants[list_start..].find("];").expect("list end");
    assert!(
        invariants[list_start..list_end].contains("\"remote\""),
        "test_business_crates_forbid_unsafe_code must list \"remote\" (spec §1.1)"
    );
    let review = read("scripts/candid_review.sh");
    let line = review
        .lines()
        .find(|l| l.starts_with("BUSINESS_CRATES=("))
        .expect("BUSINESS_CRATES array in scripts/candid_review.sh");
    assert!(
        line.split(['(', ')', ' ']).any(|w| w == "remote"),
        "scripts/candid_review.sh BUSINESS_CRATES must include remote (spec §1.1): {line}"
    );
}

// ---------------------------------------------------------------------------------------
// RMC-S1 / RMC-S2 / RMC-S3 — unsafe, network types, unlock literals
// ---------------------------------------------------------------------------------------

#[test]
fn test_rmc_s1_lib_and_main_forbid_unsafe_code() {
    for rel in ["crates/remote/src/lib.rs", "crates/remote/src/main.rs"] {
        let content = read(rel);
        assert_eq!(
            first_code_line(&content),
            "#![forbid(unsafe_code)]",
            "{rel} must start (after its doc comment) with #![forbid(unsafe_code)]"
        );
    }
    for (rel, content) in remote_sources() {
        let code = strip_comments(&content);
        for token in [
            "unsafe {",
            "unsafe{",
            "unsafe fn",
            "unsafe impl",
            "unsafe trait",
            "unsafe extern",
        ] {
            assert!(
                !code.contains(token),
                "{rel}: the companion contains no unsafe code at all (`{token}`)"
            );
        }
    }
}

#[test]
fn test_rmc_s2_no_network_socket_type_is_named() {
    for (rel, content) in remote_sources() {
        for forbidden in [
            "TcpListener",
            "TcpStream",
            "UdpSocket",
            "std::net",
            "SocketAddr",
            "Ipv4Addr",
            "Ipv6Addr",
        ] {
            assert!(
                !content.contains(forbidden),
                "{rel}: `{forbidden}` must never be named (RMC-S2, D2: Unix socket only)"
            );
        }
    }
}

#[test]
fn test_rmc_s3_no_unlock_or_locked_hint_literal() {
    for (rel, content) in remote_sources() {
        let code = strip_comments(&content);
        for forbidden in [
            "\"UnlockSessions\"",
            "\"Unlock\"",
            "\"SetLockedHint\"",
            "\"TerminateSession\"",
            "\"KillSession\"",
            "\"ActivateSession\"",
        ] {
            assert!(
                !code.contains(forbidden),
                "{rel}: the D-Bus method literal {forbidden} is forbidden (RMC-S3, D10)"
            );
        }
    }
    let logind = strip_comments(&read("crates/remote/src/logind.rs"));
    assert!(
        logind.contains("\"LockSession\""),
        "logind.rs names Manager.LockSession through a string literal (D9)"
    );
    assert_eq!(
        remote_sources()
            .iter()
            .map(|(_, c)| strip_comments(c).matches("\"LockSession\"").count())
            .sum::<usize>(),
        1,
        "exactly one \"LockSession\" literal in the crate"
    );
    // ADR 2026-10-06: the remote unlock names Manager.UnlockSession exactly once, in
    // logind.rs, and nowhere else.
    assert_eq!(
        logind.matches("\"UnlockSession\"").count(),
        1,
        "logind.rs names Manager.UnlockSession through exactly one string literal"
    );
    assert_eq!(
        remote_sources()
            .iter()
            .map(|(_, c)| strip_comments(c).matches("\"UnlockSession\"").count())
            .sum::<usize>(),
        1,
        "exactly one \"UnlockSession\" literal in the crate"
    );
}

/// ADR 2026-10-06 "Remote Unlock in soos-remote": the operator documentation covers the
/// opt-in unlock and its accepted risk (the default itself is pinned by `config_tests`).
#[test]
fn test_rmc_unlock_is_opt_in_and_documented() {
    let doc = read("Docs/REMOTE_COMPANION.md");
    for needle in [
        "allow_unlock = true",
        "POST /api/unlock",
        "X-Soos-Action` = `unlock",
        "unlock_disabled",
        "already_unlocked",
        "Accepted risk",
    ] {
        assert!(
            doc.contains(needle),
            "Docs/REMOTE_COMPANION.md must mention `{needle}`"
        );
    }
    let js = read("crates/remote/assets/app.js");
    for required in [
        "/api/unlock",
        "\"X-Soos-Action\": \"unlock\"",
        "Unlock now",
        "window.confirm(",
        "unlock_disabled",
    ] {
        assert!(
            js.contains(required),
            "app.js must contain `{required}` (ADR 2026-10-06)"
        );
    }
    assert!(
        read("crates/remote/assets/index.html").contains("id=\"unlock\""),
        "index.html carries the unlock button"
    );
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("Remote Unlock in `soos-remote`, Tailscale Identity Only, Opt-In"),
        "the remote unlock ADR is registered"
    );
}

// ---------------------------------------------------------------------------------------
// RMC-S4 — leaf crate
// ---------------------------------------------------------------------------------------

#[test]
fn test_rmc_s4_remote_is_a_leaf_crate() {
    let root = workspace_root();
    for parent in ["crates", "tests"] {
        for entry in fs::read_dir(root.join(parent)).unwrap().flatten() {
            let manifest = entry.path().join("Cargo.toml");
            if !manifest.is_file() {
                continue;
            }
            let rel = manifest
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if rel == "crates/remote/Cargo.toml" {
                continue;
            }
            let content = fs::read_to_string(&manifest).unwrap();
            assert!(
                !content.contains("soos-remote"),
                "{rel} must not depend on soos-remote (RMC-S4)"
            );
        }
    }
    let manifest = read("crates/remote/Cargo.toml");
    for forbidden in [
        "soos-daemon",
        "soos-pam",
        "soos-camera-v4l",
        "soos-inference-ort",
        "soos-vision",
        "soos-biometric-store",
        "soos-evidence-store",
        "soos-protocol",
        "soos-policy",
        "ort",
        "v4l",
        "pam-bindings",
        "pam_bindings",
    ] {
        let hit = manifest.lines().map(str::trim).any(|l| {
            l.starts_with(&format!("{forbidden} ")) || l.starts_with(&format!("{forbidden}."))
        });
        assert!(
            !hit,
            "crates/remote/Cargo.toml must not depend on {forbidden} (RMC-S4)"
        );
    }
}

// ---------------------------------------------------------------------------------------
// RMC-S5 / RMC-S11 — user unit
// ---------------------------------------------------------------------------------------

#[test]
fn test_rmc_s5_s11_user_unit_is_sandboxed_and_never_retries_config_errors() {
    assert!(
        exists("packaging/soos-remote.service"),
        "packaging/soos-remote.service must exist (spec §2.9)"
    );
    let unit = read("packaging/soos-remote.service");
    let lines: Vec<&str> = unit.lines().map(str::trim).collect();
    for required in [
        "RestrictAddressFamilies=AF_UNIX",
        "NoNewPrivileges=yes",
        "UMask=0077",
        "RestartPreventExitStatus=78",
        "Restart=on-failure",
        "Type=exec",
        "ExecStart=%h/.local/bin/soos-remote",
        "WantedBy=default.target",
        "LockPersonality=yes",
        "MemoryDenyWriteExecute=yes",
        "RestrictRealtime=yes",
        "RestrictSUIDSGID=yes",
        "SystemCallArchitectures=native",
    ] {
        assert!(lines.contains(&required), "unit must contain `{required}`");
    }
    for forbidden_prefix in [
        "User=",
        "Group=",
        "DynamicUser=",
        "WantedBy=multi-user.target",
    ] {
        assert!(
            !lines.iter().any(|l| l.starts_with(forbidden_prefix)),
            "a user unit never carries `{forbidden_prefix}` (RMC-S5)"
        );
    }
    let lib = read("crates/remote/src/lib.rs");
    assert!(
        lib.contains("pub const EXIT_CONFIG: u8 = 78;"),
        "lib.rs defines EXIT_CONFIG: u8 = 78 (RMC-S11)"
    );
    assert!(
        lib.contains("pub const EXIT_RUNTIME: u8 = 1;"),
        "lib.rs defines EXIT_RUNTIME: u8 = 1"
    );
}

// ---------------------------------------------------------------------------------------
// RMC-S6 — installer
// ---------------------------------------------------------------------------------------

#[test]
fn test_rmc_s6_installer_refuses_root_and_never_runs_privileged_commands() {
    assert!(
        exists("scripts/install_remote.sh"),
        "scripts/install_remote.sh must exist (spec §2.10)"
    );
    let script = read("scripts/install_remote.sh");
    assert!(
        script
            .lines()
            .next()
            .is_some_and(|l| l.starts_with("#!/usr/bin/env bash") || l.starts_with("#!/bin/bash")),
        "bash shebang"
    );
    assert!(script.contains("set -euo pipefail"), "strict mode");
    assert!(
        script.contains("EUID") && (script.contains("-eq 0") || script.contains("== 0")),
        "the installer refuses to run as root through an EUID check (RMC-S6)"
    );
    let commands: Vec<&str> = script
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    for (needle, label) in [
        ("sudo", "sudo"),
        ("tailscale", "tailscale"),
        ("systemctl --user enable", "systemctl --user enable"),
    ] {
        for line in &commands {
            let is_word = line
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' '))
                .any(|segment| {
                    segment
                        .split(' ')
                        .collect::<Vec<_>>()
                        .windows(needle.split(' ').count())
                        .any(|w| w.join(" ") == needle)
                });
            if is_word {
                assert!(
                    line.starts_with("echo") || line.starts_with("printf"),
                    "`{label}` may only appear inside echo/printf instruction text (RMC-S6): {line}"
                );
            }
        }
    }
    for required in [
        "cargo build --release --locked -p soos-remote",
        "install -Dm755",
        "install -Dm644",
        "packaging/soos-remote.service",
        "systemd/user/soos-remote.service",
        "soos/remote.toml",
        "allowed_logins = []",
        "daemon-reload",
        "--uninstall",
    ] {
        assert!(
            script.contains(required),
            "installer must contain `{required}`"
        );
    }
    assert!(
        script.contains("0600") || script.contains("-m 600") || script.contains("umask 077"),
        "the configuration template is written with mode 0600"
    );
    if Command::new("bash").arg("--version").output().is_ok() {
        let output = Command::new("bash")
            .arg("-n")
            .arg(workspace_root().join("scripts/install_remote.sh"))
            .output()
            .expect("bash -n");
        assert!(
            output.status.success(),
            "bash -n: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

// ---------------------------------------------------------------------------------------
// RMC-S7 — documentation
// ---------------------------------------------------------------------------------------

#[test]
fn test_rmc_s7_operator_documentation_exists_and_covers_the_requirements() {
    assert!(
        exists("Docs/REMOTE_COMPANION.md"),
        "Docs/REMOTE_COMPANION.md must exist (RMC-S7)"
    );
    let doc = read("Docs/REMOTE_COMPANION.md");
    for needle in [
        "tailscale serve --bg unix:",
        "allowed_logins",
        "allowed_hosts",
        "LockedHint",
        "Tailscale-User-Login",
        "remote unlock",
        "systemctl --user",
        "soos-remote.service",
        "install_remote.sh",
        "RestrictAddressFamilies=AF_UNIX",
        "funnel",
    ] {
        assert!(
            doc.contains(needle),
            "Docs/REMOTE_COMPANION.md must mention `{needle}`"
        );
    }
    assert!(
        doc.to_lowercase().contains("out of scope"),
        "the out-of-scope list (remote unlock, push notifications, live camera) is documented"
    );
    for needle in ["push notification", "live camera"] {
        assert!(
            doc.to_lowercase().contains(needle),
            "the out-of-scope list must name `{needle}`"
        );
    }
    let index = read("Docs/README.md");
    assert!(
        index.contains("REMOTE_COMPANION.md"),
        "Docs/README.md indexes REMOTE_COMPANION.md"
    );
}

#[test]
fn test_rmc_architecture_and_strategy_documents_describe_the_companion() {
    let architecture = read("AI/ARCHITECTURE.md");
    assert!(
        architecture.contains("Remote Companion") && architecture.contains("soos-remote"),
        "AI/ARCHITECTURE.md documents the Remote Companion (spec §1.1)"
    );
    let mock = read("AI/MOCK_STRATEGY.md");
    assert!(
        mock.contains("Remote Companion Doubles"),
        "AI/MOCK_STRATEGY.md gains a \"Remote Companion Doubles\" section"
    );
    let security = read("Docs/SECURITY_AND_QUALITY_GUIDELINES.md");
    let zbus_line = security
        .lines()
        .find(|l| l.contains("`zbus` 5"))
        .expect("the zbus dependency line");
    assert!(
        zbus_line.contains("soos-remote"),
        "the zbus line of Docs/SECURITY_AND_QUALITY_GUIDELINES.md names soos-remote (ADR item 7)"
    );
    let facts = read(".claude/skills/dev-workflow/references/project-facts.md");
    assert!(
        facts.contains("`crates/remote`"),
        "project-facts.md workspace map lists crates/remote"
    );
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("Remote Companion `soos-remote`") && decisions.contains("GitHub #339"),
        "AI/DECISIONS.md carries the 2026-10-05 Remote Companion ADR"
    );
}

// ---------------------------------------------------------------------------------------
// RMC-S8 — web assets
// ---------------------------------------------------------------------------------------

fn attribute_values(html: &str, tag: &str, attribute: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut start = 0;
    let open = format!("<{tag}");
    while let Some(pos) = lower[start..].find(&open) {
        let at = start + pos;
        let end = lower[at..].find('>').map_or(lower.len(), |e| at + e);
        let tag_text = &html[at..end];
        let lower_tag = &lower[at..end];
        let needle = format!("{attribute}=");
        let mut search = 0;
        while let Some(p) = lower_tag[search..].find(&needle) {
            let value_start = search + p + needle.len();
            let rest = &tag_text[value_start..];
            let value = if let Some(quoted) = rest.strip_prefix('"') {
                quoted.split('"').next().unwrap_or("")
            } else if let Some(quoted) = rest.strip_prefix('\'') {
                quoted.split('\'').next().unwrap_or("")
            } else {
                rest.split([' ', '>']).next().unwrap_or("")
            };
            out.push(value.to_string());
            search = value_start;
        }
        start = end.max(at + 1);
    }
    out
}

fn is_remote_url(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    v.starts_with("http://") || v.starts_with("https://") || v.starts_with("//")
}

#[test]
fn test_rmc_s8_assets_exist_load_nothing_remote_and_have_no_inline_script() {
    for asset in [
        "index.html",
        "app.js",
        "style.css",
        "manifest.webmanifest",
        "icon.svg",
        "apple-touch-icon.png",
    ] {
        assert!(
            exists(&format!("crates/remote/assets/{asset}")),
            "crates/remote/assets/{asset} must exist (D11)"
        );
    }
    let html = read("crates/remote/assets/index.html");
    let lower = html.to_ascii_lowercase();
    // No inline script body: every <script> must have a src and an empty body.
    let mut search = 0;
    let mut scripts = 0;
    while let Some(pos) = lower[search..].find("<script") {
        let at = search + pos;
        let open_end = at + lower[at..].find('>').expect("script tag end");
        let close = open_end + lower[open_end..].find("</script>").expect("script close");
        let body = html[open_end + 1..close].trim();
        assert!(
            body.is_empty(),
            "index.html must not contain an inline <script> body (RMC-S8): {body:?}"
        );
        assert!(
            lower[at..open_end].contains("src="),
            "every <script> loads a same-origin file"
        );
        scripts += 1;
        search = close;
    }
    assert!(
        scripts >= 1,
        "index.html loads app.js through a <script src>"
    );
    for value in attribute_values(&html, "script", "src")
        .into_iter()
        .chain(attribute_values(&html, "link", "href"))
        .chain(attribute_values(&html, "img", "src"))
        .chain(attribute_values(&html, "iframe", "src"))
    {
        assert!(
            !is_remote_url(&value),
            "index.html must not load an absolute URL (RMC-S8): {value}"
        );
        assert!(
            !value.trim().to_ascii_lowercase().starts_with("javascript:"),
            "no javascript: URL: {value}"
        );
    }
    assert!(
        !lower.contains(" onclick=") && !lower.contains(" onload=") && !lower.contains(" onerror="),
        "no inline event handler attributes (CSP default-src 'self')"
    );
    for needle in [
        "app.js",
        "style.css",
        "manifest.webmanifest",
        "apple-touch-icon.png",
        "apple-mobile-web-app-capable",
        "https://github.com/Mysticaly622/soos",
    ] {
        assert!(
            html.contains(needle),
            "index.html must reference `{needle}` (spec §2.11)"
        );
    }
    assert!(
        lower.contains("<a ") && lower.contains("href=\"https://github.com/mysticaly622/soos\""),
        "the source link is a plain <a href> (ADR item 9)"
    );
    let css = read("crates/remote/assets/style.css");
    let css_lower = css.to_ascii_lowercase();
    let mut search = 0;
    while let Some(pos) = css_lower[search..].find("url(") {
        let at = search + pos + 4;
        let end = at + css_lower[at..].find(')').expect("url close");
        let value = css[at..end].trim().trim_matches(['"', '\'']);
        assert!(
            !is_remote_url(value),
            "style.css must not load an absolute URL (RMC-S8): {value}"
        );
        search = end;
    }
    assert!(
        !css_lower.contains("@import"),
        "style.css loads no other stylesheet"
    );
    assert!(
        css.contains("prefers-color-scheme"),
        "light/dark through prefers-color-scheme (spec §2.11)"
    );
}

#[test]
fn test_rmc_s8_app_js_is_textcontent_only_and_carries_the_ui_constants() {
    for asset in [
        "app.js",
        "manifest.webmanifest",
        "apple-touch-icon.png",
        "icon.svg",
    ] {
        assert!(
            exists(&format!("crates/remote/assets/{asset}")),
            "crates/remote/assets/{asset} must exist (D11)"
        );
    }
    let js = read("crates/remote/assets/app.js");
    for forbidden in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "eval(",
        "new Function",
        "http://",
        "https://",
    ] {
        assert!(
            !js.contains(forbidden),
            "app.js must not use `{forbidden}` (spec §2.11, RMC-S8)"
        );
    }
    for required in [
        "textContent",
        "EventSource",
        "/api/events",
        "/api/status",
        "/api/lock",
        "X-Soos-Action",
        "visibilitychange",
        "pageshow",
        "STALE_UI_MS = 45000",
        "LOCK_CONFIRM_UI_MS = 5000",
        "Unreachable",
        "Lock now",
        "Lock requested",
        "LockedHint unchanged",
    ] {
        assert!(
            js.contains(required),
            "app.js must contain `{required}` (spec §2.11)"
        );
    }
    let manifest = read("crates/remote/assets/manifest.webmanifest");
    for required in [
        "\"display\": \"standalone\"",
        "\"start_url\": \"/\"",
        "\"scope\": \"/\"",
        "icon.svg",
    ] {
        assert!(
            manifest.contains(required),
            "manifest.webmanifest must contain `{required}`"
        );
    }
    assert!(
        !manifest.contains("http://") && !manifest.contains("https://"),
        "the manifest references same-origin resources only"
    );
    let png = read_bytes("crates/remote/assets/apple-touch-icon.png");
    assert!(
        png.starts_with(b"\x89PNG\r\n\x1a\n"),
        "apple-touch-icon.png is a PNG"
    );
    assert!(&png[12..16] == b"IHDR", "IHDR chunk first");
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    assert_eq!(
        (width, height),
        (180, 180),
        "apple-touch-icon.png is 180x180 (D11)"
    );
    let svg = read("crates/remote/assets/icon.svg");
    assert!(svg.contains("<svg"), "icon.svg is an SVG document");
    assert!(
        !svg.to_ascii_lowercase().contains("<script"),
        "icon.svg contains no script"
    );
}

// ---------------------------------------------------------------------------------------
// RMC-S9 — bus rules (presence C2 parity, R3-2)
// ---------------------------------------------------------------------------------------

#[test]
fn test_rmc_s9_zbus_manifest_line_and_pinned_system_bus() {
    let manifest = read("crates/remote/Cargo.toml");
    assert!(
        toml_table(&manifest, "dependencies").contains(&"zbus = { workspace = true }"),
        "crates/remote/Cargo.toml declares exactly `zbus = {{ workspace = true }}` (no feature override)"
    );
    let lib = read("crates/remote/src/lib.rs");
    assert!(
        lib.contains(
            "pub const SYSTEM_BUS_ADDRESS: &str = \"unix:path=/run/dbus/system_bus_socket\";"
        ),
        "SYSTEM_BUS_ADDRESS is pinned in lib.rs (RMC-S9)"
    );
    let logind = strip_comments(&read("crates/remote/src/logind.rs"));
    assert!(
        logind.contains("SYSTEM_BUS_ADDRESS"),
        "logind.rs connects to SYSTEM_BUS_ADDRESS"
    );
    assert!(
        logind.contains("zbus::connection::Builder::address("),
        "logind.rs opens the connection only through zbus::connection::Builder::address( (RMC-S9)"
    );
    assert!(
        logind.contains(".max_queued("),
        "logind.rs bounds the unsolicited message queue (max_queued 16, spec §2.6)"
    );
    assert!(
        logind.contains("DBUS_CALL_TIMEOUT_MS") && logind.contains("DBUS_CONNECT_TIMEOUT_MS"),
        "every call and the connect are bounded by the §3 constants"
    );
    // The pinned address is the only `Builder::address(` call site in the crate.
    assert_eq!(
        remote_sources()
            .iter()
            .map(|(_, c)| strip_comments(c).matches("Builder::address(").count())
            .sum::<usize>(),
        1,
        "exactly one zbus connection builder in the crate"
    );
}

#[test]
fn test_rmc_s9_bus_rules_match_the_presence_worker() {
    let forbidden_everywhere = [
        "Connection::system",
        "Connection::session",
        "Builder::system",
        "Builder::session",
        "::system(",
        "::session(",
        "DBUS_SYSTEM_BUS_ADDRESS",
        "DBUS_SESSION_BUS_ADDRESS",
        "object_server",
        "#[proxy",
        "zbus::proxy",
        "receive_signal",
        "MessageStream",
        "CacheProperties::Yes",
        "CacheProperties::Lazily",
        // R3-2 needles (presence C2 parity).
        "serve_at",
        "request_name",
        "#[interface",
        "SignalStream",
        "zbus::blocking",
        "Address::system",
    ];
    for (rel, content) in remote_sources() {
        let code = strip_comments(&content);
        for forbidden in forbidden_everywhere {
            assert!(
                !code.contains(forbidden),
                "{rel}: `{forbidden}` is forbidden in the companion (RMC-S9 / R3-2)"
            );
        }
        if !rel.ends_with("/main.rs") {
            for forbidden in ["env::var", "env::vars", "std::env", "env!("] {
                assert!(
                    !code.contains(forbidden),
                    "{rel}: the environment is read by main.rs only (RMC-S9): `{forbidden}`"
                );
            }
        }
    }
    // main.rs reads only the three documented variables (spec §2.3), never a bus address.
    let main = strip_comments(&read("crates/remote/src/main.rs"));
    let mut search = 0;
    let mut env_names = Vec::new();
    while let Some(pos) = main[search..].find("env::var") {
        let at = search + pos;
        let after = &main[at..];
        if let Some(open) = after.find('(') {
            let literal = &after[open + 1..];
            if let Some(quoted) = literal.strip_prefix('"') {
                env_names.push(quoted.split('"').next().unwrap_or("").to_string());
            }
        }
        search = at + "env::var".len();
    }
    for name in &env_names {
        assert!(
            ["XDG_RUNTIME_DIR", "XDG_CONFIG_HOME", "HOME"].contains(&name.as_str()),
            "main.rs reads an undocumented environment variable: {name}"
        );
    }
    assert!(
        !main.contains("_BUS_ADDRESS"),
        "main.rs never derives a bus address from the environment"
    );
}

// ---------------------------------------------------------------------------------------
// RMC-S10 — main refuses root first, current-thread runtime
// ---------------------------------------------------------------------------------------

#[test]
fn test_rmc_s10_main_refuses_root_before_loading_configuration() {
    let main = strip_comments(&read("crates/remote/src/main.rs"));
    let root_check = main
        .find("check_not_root(")
        .expect("main.rs must call check_not_root (RMC-S10, D1)");
    let load = main
        .find("load_config(")
        .expect("main.rs must call load_config");
    assert!(
        root_check < load,
        "check_not_root must be called before load_config (RMC-S10)"
    );
    assert!(
        main.contains("getuid()") && main.contains("geteuid()"),
        "both the real and the effective uid are checked (D1)"
    );
    assert!(
        main.contains("new_current_thread()"),
        "main.rs builds a current-thread Tokio runtime (spec §2.2)"
    );
    assert!(
        !main.contains("new_multi_thread") && !main.contains("#[tokio::main"),
        "no multi-thread runtime in the companion"
    );
    assert!(
        main.contains("EXIT_CONFIG") && main.contains("EXIT_RUNTIME"),
        "main.rs maps failures to EXIT_CONFIG / EXIT_RUNTIME (spec §4)"
    );
    assert!(
        main.contains("default_config_path("),
        "the default configuration path is resolved through default_config_path (spec §2.3)"
    );
}

// ---------------------------------------------------------------------------------------
// Lint and logging hygiene (RC-5, D12)
// ---------------------------------------------------------------------------------------

#[test]
fn test_rmc_production_code_never_panics_or_prints() {
    for (rel, content) in remote_sources() {
        let code = strip_comments(production_part(&content));
        for forbidden in [
            ".unwrap()",
            ".expect(",
            "panic!(",
            "unreachable!(",
            "todo!(",
            "unimplemented!(",
            "println!(",
            "eprintln!(",
            "print!(",
            "eprint!(",
            "dbg!(",
        ] {
            assert!(
                !code.contains(forbidden),
                "{rel}: `{forbidden}` is forbidden in companion production code"
            );
        }
        for lint in [
            "clippy::unwrap_used",
            "clippy::expect_used",
            "clippy::panic",
            "clippy::indexing_slicing",
            "clippy::arithmetic_side_effects",
            "clippy::print_stderr",
            "clippy::print_stdout",
        ] {
            assert!(
                !code.contains(&format!("allow({lint}"))
                    && !code.contains(&format!("allow(\n    {lint}")),
                "{rel}: production code must not allow `{lint}` (D12)"
            );
        }
        assert!(
            !code.contains("SystemTime::now")
                || rel.ends_with("/main.rs")
                || rel.ends_with("/server.rs"),
            "{rel}: the Unix clock is read in one place and injected elsewhere (R3-1)"
        );
    }
}

#[test]
fn test_rmc_logging_never_names_identity_header_or_session_fields() {
    let forbidden_keys = [
        "login",
        "logins",
        "host",
        "hosts",
        "origin",
        "header",
        "headers",
        "body",
        "session",
        "session_id",
        "sid",
        "id",
        "user",
        "user_name",
        "username",
        "name",
        "value",
        "values",
        "identity",
        "request_path",
        "target",
        "uri",
        "url",
    ];
    for (rel, content) in remote_sources() {
        let code = strip_comments(production_part(&content));
        for args in macro_invocations(&code, &TRACING_MACROS) {
            for key in field_keys(&args) {
                assert!(
                    !forbidden_keys.contains(&key.as_str())
                        && !key.ends_with("_login")
                        && !key.ends_with("_host")
                        && !key.ends_with("_header")
                        && !key.ends_with("_id")
                        && !key.ends_with("_name")
                        && !key.starts_with("header_"),
                    "{rel}: field `{key}` must never be logged (RC-5): {args}"
                );
            }
            for capture in [
                "{login", "{host", "{header", "{origin", "{body", "{session", "{id}", "{id:",
                "{value", "{name", "{user", "{path", "{target", "{uri",
            ] {
                assert!(
                    !args.contains(capture),
                    "{rel}: inline capture `{capture}` must never be logged (RC-5): {args}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// RMC-S12 — installer scripts are executable (spec §8, R4-5)
// ---------------------------------------------------------------------------------------

/// RMC-S12: both installer scripts carry an execute bit in the checkout (git mode
/// `100755`), so the documented `./scripts/install_remote.sh` invocation works. The
/// checkout honours the git mode, so `std::fs::metadata` is enough (no `git` subprocess).
#[test]
fn test_rmc_s12_installer_scripts_are_executable() {
    use std::os::unix::fs::PermissionsExt;
    for rel in ["scripts/install_remote.sh", "scripts/install.sh"] {
        let path = workspace_root().join(rel);
        let metadata =
            fs::metadata(&path).unwrap_or_else(|e| panic!("Cannot stat {}: {e}", path.display()));
        assert!(metadata.is_file(), "{rel} must be a regular file");
        let mode = metadata.permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "{rel} must be executable (git mode 100755, RMC-S12); found {:o}",
            mode & 0o777
        );
    }
}

// ---------------------------------------------------------------------------------------
// RMC-S7b — documentation describes the verified effective host (spec §8, R4-4)
// ---------------------------------------------------------------------------------------

/// RMC-S7b: `Docs/REMOTE_COMPANION.md` documents the D5a′ effective host (`X-Forwarded-Host`,
/// `X-Forwarded-Proto`) and no longer calls the Serve forwarding pending.
#[test]
fn test_rmc_s7b_documentation_describes_the_effective_host() {
    let doc = read("Docs/REMOTE_COMPANION.md");
    for needle in ["X-Forwarded-Host", "X-Forwarded-Proto", "effective host"] {
        assert!(
            doc.contains(needle),
            "Docs/REMOTE_COMPANION.md must mention `{needle}` (RMC-S7b)"
        );
    }
    for stale in ["pending owner verification", "(pending)"] {
        assert!(
            !doc.contains(stale),
            "Docs/REMOTE_COMPANION.md must no longer contain `{stale}` (RMC-S7b, RMC20 verified)"
        );
    }
}
