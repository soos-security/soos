//! Presence auto-unlock static contracts (GitHub #323, spec §8 C1–C9; matrix PAU2, PAU10,
//! PAU11, PAU16, PAU17, PAU19, PAU20, PAU29).
//!
//! - C1 `zbus` is a daemon-only dependency, declared once in the workspace, never in PAM;
//! - C2 the new modules forbid `unsafe`, never panic through `unwrap`/`expect`, and connect
//!   only to the pinned system bus address (never an environment-derived bus);
//! - C3 exactly one production call of `unlock_session`;
//! - C4 the PAD consensus is built only in `consensus.rs` and shared by both callers;
//! - C5 the shared rate-limit default is 40 and documented;
//! - C6 the presence kill-switch directory is the PAM flag directory;
//! - C7 the ADR and ARCHITECTURE invariant 6 record the owner decisions and guard;
//! - C8 the presence code never writes (no tally reset);
//! - C9 the unit keeps the sandbox the account guard relies on.

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

/// The new code governed by C2/C8/PAU16: `presence/**` and `consensus.rs`.
fn presence_and_consensus_sources() -> Vec<(String, String)> {
    let mut files = rust_files("crates/daemon/src/presence");
    assert!(
        files.len() >= 8,
        "crates/daemon/src/presence must hold mod, config, logind, display, switch, tracker, \
         account and worker (found {})",
        files.len()
    );
    files.push((
        "crates/daemon/src/consensus.rs".to_string(),
        read("crates/daemon/src/consensus.rs"),
    ));
    files
}

/// Source lines with `//` comment lines removed (doc comments included).
fn code_lines(content: &str) -> Vec<&str> {
    content
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect()
}

/// Production part of a source file: everything before the first `#[cfg(test)]`.
/// Production part of a source file. Only a **trailing** `#[cfg(test)] mod … { … }` block
/// (its closing brace is the end of the file) is treated as test code; any other
/// `#[cfg(test)]` hides nothing (auditor T5).
fn production_part(content: &str) -> &str {
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
        if inside {
            lines.push(trimmed);
        }
    }
    lines
}

fn contains_word(haystack: &str, word: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(word) {
        let at = start + pos;
        let before_ok =
            at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        let end = at + word.len();
        let after_ok =
            end >= bytes.len() || !(bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
        if before_ok && after_ok {
            return true;
        }
        start = end;
    }
    false
}

// ---------------------------------------------------------------------------------------
// C1 / PAU17 — dependency
// ---------------------------------------------------------------------------------------

#[test]
fn test_pau_zbus_is_declared_once_in_the_workspace() {
    let root = read("Cargo.toml");
    let deps = toml_table(&root, "workspace.dependencies");
    let zbus: Vec<&&str> = deps.iter().filter(|l| l.starts_with("zbus ")).collect();
    assert_eq!(
        zbus.len(),
        1,
        "exactly one zbus entry in [workspace.dependencies]"
    );
    let line = zbus[0];
    assert!(line.contains("version = \"5\""), "zbus 5: {line}");
    assert!(line.contains("default-features = false"), "{line}");
    assert!(line.contains("features = [\"tokio\"]"), "{line}");
}

/// Contract migration (GitHub #339, architect spec §2.12, ADR 2026-10-05 "Remote Companion
/// `soos-remote`" item (7)): the 2026-10-02 decision "zbus is daemon-only" is widened to
/// exactly two crates, `soos-daemon` and `soos-remote` (the remote companion must read the
/// logind `LockedHint`). Every other manifest under `crates/` and `tests/` must still not
/// mention `zbus`; the PAM assertion and the test name are unchanged (matrix row PAU17).
#[test]
fn test_pau_zbus_is_used_only_by_the_daemon() {
    const ZBUS_LINE: &str = "zbus = { workspace = true }";
    const ALLOWED: [&str; 2] = ["crates/daemon/Cargo.toml", "crates/remote/Cargo.toml"];
    for rel in ALLOWED {
        let manifest = read(rel);
        assert!(
            toml_table(&manifest, "dependencies").contains(&ZBUS_LINE),
            "{rel} must declare `{ZBUS_LINE}` in [dependencies]"
        );
        assert_eq!(
            manifest.matches("zbus").count(),
            1,
            "{rel} names zbus exactly once (no feature override, no second zbus crate)"
        );
    }
    let root = workspace_root();
    let mut manifests = Vec::new();
    for parent in ["crates", "tests"] {
        for entry in fs::read_dir(root.join(parent)).unwrap().flatten() {
            let manifest = entry.path().join("Cargo.toml");
            if manifest.is_file() {
                manifests.push(manifest);
            }
        }
    }
    let mut allowed_seen = 0;
    for manifest in manifests {
        let rel = manifest
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if ALLOWED.contains(&rel.as_str()) {
            allowed_seen += 1;
            continue;
        }
        let content = fs::read_to_string(&manifest).unwrap();
        assert!(!content.contains("zbus"), "{rel} must not depend on zbus");
    }
    assert_eq!(
        allowed_seen,
        ALLOWED.len(),
        "both allowed manifests exist: {ALLOWED:?}"
    );
    let pam = read("crates/pam/Cargo.toml");
    assert!(
        !pam.contains("zbus") && !pam.contains("dbus"),
        "pam_soos.so never links a D-Bus client"
    );
}

#[test]
fn test_pau_lockfile_pins_zbus_5_without_libdbus() {
    let lock = read("Cargo.lock");
    let mut zbus_versions = Vec::new();
    let mut lines = lock.lines().peekable();
    while let Some(line) = lines.next() {
        if line == "name = \"zbus\"" {
            if let Some(version) = lines.peek() {
                zbus_versions.push(version.to_string());
            }
        }
    }
    assert_eq!(zbus_versions.len(), 1, "one zbus version in Cargo.lock");
    assert!(
        zbus_versions[0].starts_with("version = \"5."),
        "zbus 5.x: {}",
        zbus_versions[0]
    );
    for forbidden in ["name = \"dbus\"", "name = \"libdbus-sys\""] {
        assert!(
            !lock.contains(forbidden),
            "no C libdbus binding: {forbidden}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// C2 — unsafe, panics, bus address
// ---------------------------------------------------------------------------------------

#[test]
fn test_pau_new_modules_forbid_unsafe_code() {
    for rel in [
        "crates/daemon/src/presence/mod.rs",
        "crates/daemon/src/consensus.rs",
    ] {
        let content = read(rel);
        let first_code = content
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("//"))
            .unwrap_or("");
        assert_eq!(
            first_code, "#![forbid(unsafe_code)]",
            "{rel} must start (after its doc comment) with #![forbid(unsafe_code)]"
        );
    }
    for (rel, content) in presence_and_consensus_sources() {
        for line in code_lines(&content) {
            assert!(
                !contains_word(line, "unsafe") || line.contains("forbid(unsafe_code)"),
                "{rel}: no `unsafe` allowed: {line}"
            );
        }
    }
}

#[test]
fn test_pau_new_modules_never_unwrap_or_expect() {
    for (rel, content) in presence_and_consensus_sources() {
        for line in code_lines(&content) {
            assert!(!line.contains(".unwrap()"), "{rel}: no .unwrap(): {line}");
            assert!(!line.contains(".expect("), "{rel}: no .expect(: {line}");
            for forbidden in ["panic!(", "unreachable!(", "todo!(", "unimplemented!("] {
                assert!(!line.contains(forbidden), "{rel}: no `{forbidden}`: {line}");
            }
            if line.contains("allow(") {
                for lint in [
                    "clippy::panic",
                    "clippy::indexing_slicing",
                    "clippy::arithmetic_side_effects",
                    "clippy::unwrap_used",
                    "clippy::expect_used",
                ] {
                    assert!(
                        !line.contains(lint),
                        "{rel}: `{lint}` may not be allowed in the new modules: {line}"
                    );
                }
            }
        }
    }
}

#[test]
fn test_pau_presence_connects_only_to_the_pinned_system_bus() {
    for (rel, content) in presence_and_consensus_sources() {
        for line in code_lines(&content) {
            for forbidden in [
                "Connection::system",
                "Connection::session",
                "Builder::system",
                "Builder::session",
                "DBUS_SYSTEM_BUS_ADDRESS",
                "DBUS_SESSION_BUS_ADDRESS",
            ] {
                assert!(
                    !line.contains(forbidden),
                    "{rel}: environment-derived bus `{forbidden}` is forbidden: {line}"
                );
            }
        }
    }
    for (rel, content) in rust_files("crates/daemon/src/presence") {
        for line in code_lines(&content) {
            for forbidden in [
                "::system(",
                "::session(",
                "env::var",
                "Address::system",
                "zbus::blocking",
                "request_name",
                "serve_at",
                "object_server",
                "#[proxy",
                "zbus::proxy",
                "#[interface",
                "MessageStream",
                "SignalStream",
                "CacheProperties::Yes",
                "CacheProperties::Lazily",
            ] {
                assert!(
                    !line.contains(forbidden),
                    "{rel}: `{forbidden}` is forbidden in presence (constraints 4–6): {line}"
                );
            }
        }
    }
    for (rel, content) in rust_files("crates/daemon/src") {
        for line in code_lines(&content) {
            for forbidden in ["Builder::system", "Builder::session"] {
                assert!(!line.contains(forbidden), "{rel}: `{forbidden}`: {line}");
            }
        }
    }
    let module = read("crates/daemon/src/presence/mod.rs");
    assert!(
        module.contains(
            "pub const SYSTEM_BUS_ADDRESS: &str = \"unix:path=/run/dbus/system_bus_socket\";"
        ),
        "SYSTEM_BUS_ADDRESS is pinned in presence/mod.rs"
    );
    let logind = read("crates/daemon/src/presence/logind.rs");
    let code = code_lines(&logind).join("\n");
    assert!(
        code.contains("SYSTEM_BUS_ADDRESS"),
        "logind.rs uses SYSTEM_BUS_ADDRESS"
    );
    assert!(
        code.contains(".address("),
        "logind.rs connects through Builder::address"
    );
    for (rel, content) in rust_files("crates/daemon/src") {
        for line in code_lines(&content) {
            for forbidden in ["Connection::system", "Connection::session"] {
                assert!(!line.contains(forbidden), "{rel}: `{forbidden}`: {line}");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// C3 / PAU11 — single unlock call site
// ---------------------------------------------------------------------------------------

#[test]
fn test_pau_unlock_session_has_exactly_one_production_call_site() {
    let mut sites = Vec::new();
    for (rel, content) in rust_files("crates/daemon/src") {
        for line in code_lines(production_part(&content)) {
            let count = line.matches(".unlock_session(").count();
            for _ in 0..count {
                sites.push(rel.clone());
            }
        }
    }
    assert_eq!(
        sites,
        vec!["crates/daemon/src/presence/worker.rs".to_string()],
        "exactly one production `.unlock_session(` call, in presence/worker.rs"
    );
    let logind = read("crates/daemon/src/presence/logind.rs");
    assert!(
        logind.contains("\"UnlockSession\""),
        "ZbusLogind calls Manager.UnlockSession"
    );

    // Round 2 (auditor T5): UFCS / fully qualified calls count too; only the trait
    // declaration and the impls (`fn unlock_session`) are not call sites.
    let mut any_form = Vec::new();
    let mut literals = Vec::new();
    for (rel, content) in rust_files("crates/daemon/src") {
        for line in code_lines(production_part(&content)) {
            if !line.contains("fn unlock_session") {
                for _ in 0..line.matches("unlock_session(").count() {
                    any_form.push(rel.clone());
                }
                for _ in 0..line.matches("unlock_session::<").count() {
                    any_form.push(rel.clone());
                }
            }
            for _ in 0..line.matches("\"UnlockSession\"").count() {
                literals.push(rel.clone());
            }
        }
        for line in code_lines(&content) {
            for forbidden in [
                "\"UnlockSessions\"",
                "\"Unlock\"",
                "\"LockSession\"",
                "\"LockSessions\"",
                "\"SetLockedHint\"",
                "\"TerminateSession\"",
                "\"KillSession\"",
                "\"ActivateSession\"",
            ] {
                assert!(
                    !line.contains(forbidden),
                    "{rel}: logind method literal {forbidden} is forbidden: {line}"
                );
            }
            assert!(
                !line.contains("unlock-session") && !line.contains("loginctl"),
                "{rel}: no loginctl spawning: {line}"
            );
        }
    }
    assert_eq!(
        any_form,
        vec!["crates/daemon/src/presence/worker.rs".to_string()],
        "exactly one production `unlock_session(` call in any syntax (method, UFCS, \
         fully qualified), in presence/worker.rs"
    );
    assert_eq!(
        literals,
        vec!["crates/daemon/src/presence/logind.rs".to_string()],
        "exactly one production \"UnlockSession\" method literal, in the zbus implementation"
    );
}

/// Round 2 (auditor T5): the trailing-test-module rule of `production_part` itself.
#[test]
fn test_pau_production_part_only_strips_a_trailing_test_module() {
    let early = "fn a() {}\n#[cfg(test)]\nfn helper() {}\nfn prod() { x.unlock_session(id); }\n";
    assert_eq!(
        production_part(early),
        early,
        "a non-module cfg(test) hides nothing"
    );
    let middle = "#[cfg(test)]\nmod tests { fn t() {} }\nfn prod() { x.unlock_session(id); }\n";
    assert_eq!(
        production_part(middle),
        middle,
        "a test module followed by code hides nothing"
    );
    let trailing =
        "fn prod() {}\n#[cfg(test)]\nmod tests {\n    fn t() { x.unlock_session(id); }\n}\n";
    assert_eq!(production_part(trailing), "fn prod() {}\n");
}

// ---------------------------------------------------------------------------------------
// C4 / PAU10 — one consensus
// ---------------------------------------------------------------------------------------

#[test]
fn test_pau_pad_consensus_is_built_only_in_consensus_rs() {
    let mut sites = Vec::new();
    for (rel, content) in rust_files("crates/daemon/src") {
        for line in code_lines(&content) {
            if line.contains("PadAggregator::with_defaults") {
                sites.push(rel.clone());
            }
        }
    }
    assert!(
        !sites.is_empty(),
        "PadAggregator::with_defaults must still be used"
    );
    for site in &sites {
        assert_eq!(
            site, "crates/daemon/src/consensus.rs",
            "PadAggregator::with_defaults only in consensus.rs"
        );
    }
    for rel in [
        "crates/daemon/src/dispatcher.rs",
        "crates/daemon/src/presence/worker.rs",
    ] {
        let code = code_lines(&read(rel)).join("\n");
        assert!(
            code.contains("run_face_consensus("),
            "{rel} must call consensus::run_face_consensus"
        );
    }
    let presence_code: String = rust_files("crates/daemon/src/presence")
        .into_iter()
        .map(|(_, c)| code_lines(&c).join("\n"))
        .collect();
    for literal in ["0.50", "0.85", "0.40", "match_threshold:", "pad_threshold:"] {
        assert!(
            !presence_code.contains(literal),
            "no threshold literal in presence/: `{literal}`"
        );
    }
    assert!(
        read("crates/daemon/src/dispatcher.rs").contains("register_interactive()"),
        "the dispatcher registers interactive demand for Auth requests (PAU12)"
    );
}

// ---------------------------------------------------------------------------------------
// C5 / PAU2 — rate-limit default
// ---------------------------------------------------------------------------------------

#[test]
fn test_pau_rate_limit_default_is_forty_and_documented() {
    let source = read("crates/policy/src/rate_limit.rs");
    assert!(
        source.contains("pub const DEFAULT_MAX_ATTEMPTS: u32 = 40;"),
        "RateLimitConfig::DEFAULT_MAX_ATTEMPTS must be 40"
    );
    let daemon_doc = read("Docs/DAEMON.md");
    let rows: Vec<&str> = daemon_doc
        .lines()
        .filter(|l| l.trim_start().starts_with('|') && l.contains("`max_attempts`"))
        .collect();
    assert!(
        !rows.is_empty(),
        "Docs/DAEMON.md documents `max_attempts` in a table"
    );
    for row in rows {
        assert!(row.contains("`40`"), "Docs/DAEMON.md must state 40: {row}");
    }
    let policy_doc = read("Docs/POLICY_CRATE.md");
    assert!(
        policy_doc
            .lines()
            .any(|l| l.contains("40") && l.to_lowercase().contains("attempts")),
        "Docs/POLICY_CRATE.md states the 40-attempt default"
    );
    assert!(
        policy_doc.contains("check_and_record_with_reserve"),
        "Docs/POLICY_CRATE.md documents the reserve-keeping call"
    );
}

// ---------------------------------------------------------------------------------------
// C6 — kill-switch directory
// ---------------------------------------------------------------------------------------

fn string_const(source: &str, name: &str) -> Option<String> {
    let needle = format!("{name}: &str = \"");
    let start = source.find(&needle)? + needle.len();
    let end = source[start..].find('"')?;
    Some(source[start..start + end].to_string())
}

#[test]
fn test_pau_kill_switch_dir_equals_the_pam_flag_dir() {
    let presence = read("crates/daemon/src/presence/mod.rs");
    let pam = read("crates/pam/src/config.rs");
    let daemon_dir = string_const(&presence, "DEFAULT_KILL_SWITCH_DIR")
        .expect("DEFAULT_KILL_SWITCH_DIR in presence/mod.rs");
    let pam_dir = string_const(&pam, "DEFAULT_FLAG_DIR").expect("DEFAULT_FLAG_DIR in pam config");
    assert_eq!(daemon_dir, pam_dir);
    assert_eq!(
        string_const(&presence, "GLOBAL_DISABLE_FLAG").as_deref(),
        Some("disabled")
    );
    assert_eq!(
        string_const(&presence, "PRESENCE_DISABLE_FLAG").as_deref(),
        Some("presence.disable")
    );
}

// ---------------------------------------------------------------------------------------
// C7 / PAU20 — ADR, architecture, operator documentation
// ---------------------------------------------------------------------------------------

#[test]
fn test_pau_adr_records_the_owner_decisions_and_the_account_guard() {
    let decisions = read("AI/DECISIONS.md");
    let adr = decisions
        .lines()
        .find(|l| l.contains("Presence Auto-Unlock Through logind"))
        .expect("AI/DECISIONS.md must contain the ADR \"Presence Auto-Unlock Through logind\"");
    let lower = adr.to_lowercase();
    for keyword in [
        "default-on",
        "40",
        "sudo",
        "led",
        "presentation",
        "pam_faillock",
        "/etc/shadow",
        "no tally reset",
        "unlocksession",
        "zbus",
    ] {
        assert!(
            lower.contains(keyword),
            "the presence ADR must mention `{keyword}`"
        );
    }
}

#[test]
fn test_pau_architecture_states_invariant_six() {
    let architecture = read("AI/ARCHITECTURE.md");
    let section: Vec<&str> = architecture
        .lines()
        .skip_while(|l| !l.starts_with("### Mandatory Security Invariants"))
        .skip(1)
        .take_while(|l| !l.starts_with('#') && !l.starts_with("---"))
        .collect();
    assert!(
        !section.is_empty(),
        "AI/ARCHITECTURE.md keeps the \"### Mandatory Security Invariants\" list"
    );
    let invariant = section
        .iter()
        .find(|l| l.starts_with("6. "))
        .expect("AI/ARCHITECTURE.md §2 \"Mandatory Security Invariants\" must list invariant 6");
    for keyword in [
        "logind",
        "Allow",
        "grace",
        "REMOTE=0",
        "pam_faillock",
        "locked",
    ] {
        assert!(
            invariant.contains(keyword),
            "invariant 6 must mention `{keyword}`: {invariant}"
        );
    }
    assert!(
        architecture.contains("zbus"),
        "ARCHITECTURE lists the zbus dependency"
    );
}

#[test]
fn test_pau_operator_docs_describe_presence() {
    let daemon = read("Docs/DAEMON.md");
    for needle in [
        "[presence]",
        "`scan_interval_ms`",
        "`lock_grace_ms`",
        "presence.disable",
        "/etc/soos/disabled",
        "gdm.disable",
        "pam_faillock",
        "faillock.conf",
        "/etc/shadow",
        "no tally reset",
    ] {
        assert!(
            daemon.contains(needle),
            "Docs/DAEMON.md must document `{needle}`"
        );
    }
    let deployment = read("Docs/DISTRIBUTION_DEPLOYMENT.md");
    for needle in [
        "presence.disable",
        "swayidle",
        "SetLockedHint",
        "GNOME",
        "KDE",
    ] {
        assert!(
            deployment.contains(needle),
            "Docs/DISTRIBUTION_DEPLOYMENT.md must mention `{needle}`"
        );
    }
    assert!(
        read("AI/MOCK_STRATEGY.md").contains("MockPresenceLogind"),
        "AI/MOCK_STRATEGY.md describes MockPresenceLogind"
    );
    assert!(
        read("Docs/SECURITY_AND_QUALITY_GUIDELINES.md").contains("zbus"),
        "the security guidelines list zbus as an audited daemon-only dependency"
    );
    let packaging = read("Docs/PACKAGING_AND_PROVISIONING.md");
    for needle in ["CAP_DAC_OVERRIDE", "system bus", "/run/faillock"] {
        assert!(
            packaging.contains(needle),
            "Docs/PACKAGING_AND_PROVISIONING.md §8.1 must mention `{needle}`"
        );
    }
}

// ---------------------------------------------------------------------------------------
// PAU16 — no biometric field above debug
// ---------------------------------------------------------------------------------------

/// Arguments of every `macro!(...)` invocation in `code` (balanced parentheses, strings
/// skipped), for the given macro names.
fn macro_invocations(code: &str, macros: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for name in macros {
        let needle = format!("{name}!(");
        let mut start = 0;
        while let Some(pos) = code[start..].find(&needle) {
            let at = start + pos;
            let preceded_by_ident = at > 0 && {
                let b = code.as_bytes()[at - 1];
                b.is_ascii_alphanumeric() || b == b'_'
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
fn field_keys(args: &str) -> Vec<String> {
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

#[test]
fn test_pau_presence_logs_no_biometric_field_above_debug() {
    let forbidden = [
        "score",
        "similarity",
        "sim",
        "embedding",
        "template",
        "frame",
        "data",
    ];
    for (rel, content) in presence_and_consensus_sources() {
        let code = code_lines(&content).join("\n");
        for args in macro_invocations(
            &code,
            &[
                "info",
                "warn",
                "error",
                "tracing::info",
                "tracing::warn",
                "tracing::error",
            ],
        ) {
            for key in field_keys(&args) {
                for word in forbidden {
                    assert!(
                        key != word
                            && !key.ends_with(&format!("_{word}"))
                            && !key.starts_with(&format!("{word}_")),
                        "{rel}: field `{key}` must not be logged above debug: {args}"
                    );
                }
                // Round 2 (auditor T7, constraint 20): users are identified by uid only.
                if rel.contains("/presence/") {
                    assert!(
                        !["name", "user_name", "username", "user", "login"].contains(&key.as_str()),
                        "{rel}: the user name must not be logged above debug: {args}"
                    );
                }
            }
            // Inline format captures (`"{score}"`) bypass the field-key check.
            for word in forbidden {
                for pattern in [format!("{{{word}"), format!("{{:?{word}")] {
                    assert!(
                        !args.contains(&pattern),
                        "{rel}: inline `{pattern}` must not be logged above debug: {args}"
                    );
                }
            }
            if rel.contains("/presence/") {
                for pattern in ["{name", "{user_name", "{user}"] {
                    assert!(
                        !args.contains(pattern),
                        "{rel}: inline `{pattern}` must not be logged above debug: {args}"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// PAU19 — main.rs wiring
// ---------------------------------------------------------------------------------------

#[test]
fn test_pau_main_spawns_presence_after_ready_and_stops_it_at_shutdown() {
    let main = read("crates/daemon/src/main.rs");
    let code = code_lines(&main).join("\n");
    let ready = code
        .find("notify_ready()")
        .expect("main.rs reports READY=1");
    let spawn = code
        .find("PresenceWorker::new(")
        .expect("main.rs builds the presence worker");
    assert!(
        spawn > ready,
        "the presence worker starts after READY=1 (never delays startup)"
    );
    for needle in [
        "ZbusLogind::new()",
        "SysfsDisplayProbe::new(",
        "DEFAULT_DRM_SYSFS_DIR",
        "PresenceSwitch::new(",
        "DEFAULT_KILL_SWITCH_DIR",
        "SystemAccountGuard::new()",
        "presence.enabled",
        "enforce_active_session",
    ] {
        assert!(code.contains(needle), "main.rs must use `{needle}`");
    }
    let accept = code
        .find("accept_until_shutdown(")
        .expect("main.rs runs the accept loop");
    let after_accept = &code[accept..];
    assert!(
        after_accept.contains(".send(true)"),
        "the presence worker is signalled to stop after the accept loop returns"
    );
    assert!(
        after_accept.contains(".abort()"),
        "a worker that does not stop in time is aborted"
    );
}

// ---------------------------------------------------------------------------------------
// C8 / PAU29 — no writes, no tally reset
// ---------------------------------------------------------------------------------------

#[test]
fn test_pau_presence_code_never_writes() {
    for (rel, content) in rust_files("crates/daemon/src/presence") {
        for line in code_lines(&content) {
            for forbidden in [
                ".write(true)",
                ".append(true)",
                ".create(true)",
                ".create_new(true)",
                ".truncate(true)",
                "fs::write(",
                "File::create(",
                "OpenOptions::new().write",
                "--reset",
                "faillock --reset",
                "remove_file(",
                "set_permissions(",
                "O_WRONLY",
                "O_RDWR",
                "O_TRUNC",
                "O_CREAT",
                "rename(",
                "fs::copy(",
                "create_dir",
                "remove_dir",
                "hard_link",
                "Command::new",
                "LockExclusive",
            ] {
                assert!(
                    !line.contains(forbidden),
                    "{rel}: presence never writes (`{forbidden}`): {line}"
                );
            }
        }
    }
    let account = code_lines(&read("crates/daemon/src/presence/account.rs")).join("\n");
    for needle in [
        "O_NOFOLLOW",
        "O_NONBLOCK",
        "O_CLOEXEC",
        "LockSharedNonblock",
    ] {
        assert!(
            account.contains(needle),
            "the tally reader uses `{needle}` (PAU23)"
        );
    }
    assert!(
        account.contains("Zeroizing"),
        "the shadow file is read into a Zeroizing buffer"
    );
}

// ---------------------------------------------------------------------------------------
// C9 / PAU29 — unit sandbox
// ---------------------------------------------------------------------------------------

#[test]
fn test_pau_unit_keeps_the_account_guard_sandbox() {
    let unit = read("packaging/soos-daemon.service");
    let directive = |key: &str| -> Vec<String> {
        unit.lines()
            .map(str::trim)
            .filter(|l| l.starts_with(&format!("{key}=")))
            .map(str::to_string)
            .collect()
    };
    let caps = directive("CapabilityBoundingSet");
    assert_eq!(caps.len(), 1, "one CapabilityBoundingSet line");
    assert!(
        caps[0].split(['=', ' ']).any(|c| c == "CAP_DAC_OVERRIDE"),
        "CAP_DAC_OVERRIDE is required to read the 0660 user:root tally: {}",
        caps[0]
    );
    assert!(
        !caps[0].contains("CAP_SYS_ADMIN"),
        "logind >= 255 grants root without CAP_SYS_ADMIN"
    );
    assert_eq!(
        directive("ProtectSystem"),
        vec!["ProtectSystem=strict".to_string()]
    );
    assert_eq!(
        directive("ReadWritePaths"),
        vec!["ReadWritePaths=/var/lib/soos /run/soos".to_string()],
        "the unit never gains a write path over the guard's sources"
    );
    assert!(
        directive("RestrictAddressFamilies")
            .iter()
            .any(|l| l.contains("AF_UNIX")),
        "the system bus is AF_UNIX"
    );
    let guarded = [
        "/etc/shadow",
        "/etc/security",
        "/etc/pam.d",
        "/run/faillock",
        "/usr/lib/pam.d",
        "/usr/etc/pam.d",
        "/run/dbus",
        "/sys/class/drm",
    ];
    for key in ["InaccessiblePaths", "TemporaryFileSystem"] {
        for line in directive(key) {
            let value = line.split_once('=').map_or("", |(_, v)| v);
            for path in value.split_whitespace() {
                let path = path.trim_start_matches(['-', '+']);
                for source in guarded {
                    assert!(
                        !(source == path
                            || source.starts_with(&format!("{}/", path.trim_end_matches('/')))),
                        "{key}={path} would hide {source} from the presence guard"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Round 4 (candid review MINOR 2) — suspend-aware Allow window
// ---------------------------------------------------------------------------------------

/// The `Allow`-to-unlock window is measured on `CLOCK_BOOTTIME` (counts suspend), through a
/// boot clock the worker owns and tests can inject; production never overrides either clock.
#[test]
fn test_pau_allow_window_uses_the_boot_clock() {
    let worker = code_lines(&read("crates/daemon/src/presence/worker.rs")).join("\n");
    let module = code_lines(&read("crates/daemon/src/presence/mod.rs")).join("\n");
    assert!(
        worker.contains("CLOCK_BOOTTIME") || module.contains("CLOCK_BOOTTIME"),
        "presence/worker.rs (or the presence clock helper in mod.rs) must read CLOCK_BOOTTIME"
    );
    assert!(
        worker.contains("pub fn with_boot_clock_fn("),
        "PresenceWorker exposes the injectable boot clock `with_boot_clock_fn`"
    );
    let main = code_lines(&read("crates/daemon/src/main.rs")).join("\n");
    for hook in ["with_boot_clock_fn", "with_clock_fn"] {
        assert!(
            !main.contains(hook),
            "main.rs never overrides the presence clocks (`{hook}` is a test hook)"
        );
    }
}
