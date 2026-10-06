//! Static contracts of the failed-password alerts of `soos-remote` (ADR 2026-10-06
//! "Failed-Password Alerts in `soos-remote` From the System Journal", architect spec
//! `AI/architect_spec_remote_auth_alerts.md` §12.8 RMC-S24–RMC-S31, tests 44–50 and 56;
//! matrix RMC48, RMC52, RMC56–RMC58):
//!
//! - RMC-S24 the journal reader is the only spawned process, without a shell, with a cleared
//!   environment, closed stdin/stderr and `kill_on_drop`;
//! - RMC-S25 the journal and alert modules never use a `tracing` macro; the three fixed
//!   audit messages exist;
//! - RMC-S26 the user unit is unchanged by the feature (`RestrictAddressFamilies=AF_UNIX`);
//! - RMC-S27 the tokio `process` feature is enabled in `crates/remote` only;
//! - RMC-S28 the types that may hold journal text have no derived `Debug`;
//! - RMC-S29 the feature is documented (`Docs/REMOTE_COMPANION.md` §2c) and the ADR exists;
//! - RMC-S30 the page shows the alerts with `textContent`, the snapshot headers, no storage;
//! - RMC-S31 line buffers and entry strings are `Zeroizing`; no `BufReader`, no
//!   `serde_json::Value`/`Map` in the journal parser;
//! - RMC-S43 (round 3, owner request 2026-10-06, test 62, matrix RMC75) acknowledged
//!   records are removed, never kept behind a flag; journal entries are never deleted.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use crate::remote_companion_contract::{
    exists, macro_invocations, production_part, read, rust_files, strip_comments, toml_table,
    TRACING_MACROS,
};

const JOURNAL_RS: &str = "crates/remote/src/journal.rs";
const ALERTS_RS: &str = "crates/remote/src/alerts.rs";

/// Production code (comments stripped, trailing test module removed) of `rel`.
fn code_of(rel: &str) -> String {
    assert!(exists(rel), "{rel} must exist (spec §1.1)");
    strip_comments(production_part(&read(rel)))
}

/// Production code of every source of `crates/remote/src`.
fn remote_code() -> Vec<(String, String)> {
    rust_files("crates/remote/src")
        .into_iter()
        .map(|(rel, content)| {
            let code = strip_comments(production_part(&content));
            (rel, code)
        })
        .collect()
}

/// The attribute lines (`#[…]`) directly above the item line containing `item` (doc
/// comments are skipped), joined.
fn attributes_above(code: &str, item: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.contains(item))
        .unwrap_or_else(|| panic!("{item} not found"));
    let mut out = Vec::new();
    let mut i = at;
    while i > 0 {
        i -= 1;
        let line = lines[i].trim();
        if line.is_empty() || line.starts_with("///") || line.starts_with("//") {
            continue;
        }
        if line.starts_with("#[") || line.ends_with(")]") || line.ends_with(',') {
            out.push(line);
            continue;
        }
        break;
    }
    out.join(" ")
}

/// Whether `code` holds a manual `Debug` implementation for `ty`.
fn manual_debug_impl(code: &str, ty: &str) -> Option<String> {
    for prefix in [
        "impl fmt::Debug for ",
        "impl std::fmt::Debug for ",
        "impl Debug for ",
    ] {
        let needle = format!("{prefix}{ty} ");
        if let Some(pos) = code.find(&needle) {
            let rest = &code[pos..];
            let end = rest.find("\n}").map_or(rest.len(), |e| e + 2);
            return Some(rest[..end].to_string());
        }
    }
    None
}

/// Test 44 (RMC-S24, RMC52, A-1): exactly one `Command::new(` in the crate, in
/// `journal.rs`, on `JOURNALCTL_PATH`, with `env_clear()`, `kill_on_drop(true)` and
/// `Stdio::null()`; no shell, no `pre_exec`, no journal-widening option literal.
#[test]
fn test_rmc_s24_journal_reader_is_spawned_safely() {
    let files = remote_code();
    let spawns: Vec<&String> = files
        .iter()
        .filter(|(_, code)| code.contains("Command::new("))
        .map(|(rel, _)| rel)
        .collect();
    assert_eq!(
        spawns,
        vec![JOURNAL_RS],
        "only the journal reader spawns a process"
    );
    let journal = code_of(JOURNAL_RS);
    assert_eq!(journal.matches("Command::new(").count(), 1);
    assert!(
        journal.contains("Command::new(JOURNALCTL_PATH)"),
        "the program is the absolute constant"
    );
    assert!(journal.contains("pub const JOURNALCTL_PATH: &str = \"/usr/bin/journalctl\";"));
    for needle in [
        "env_clear()",
        "kill_on_drop(true)",
        "Stdio::null()",
        "Stdio::piped()",
    ] {
        assert!(journal.contains(needle), "journal.rs must call {needle}");
    }
    assert!(
        journal.matches("Stdio::null()").count() >= 2,
        "stdin and stderr are both closed"
    );
    for (rel, code) in &files {
        for forbidden in [
            "\"sh\"",
            "\"bash\"",
            "\"/bin/sh\"",
            "\"-c\"",
            "pre_exec",
            "\"--user\"",
            "\"--grep\"",
            "\"-g\"",
            "\"--all\"",
            "\"--cursor-file",
            "std::process::Command",
            "env::var",
        ] {
            if forbidden == "env::var" && rel.ends_with("main.rs") {
                continue;
            }
            assert!(
                !code.contains(forbidden),
                "{rel} must not contain {forbidden}"
            );
        }
    }
}

/// Test 45 (RMC-S25, RMC56, A-11): no `tracing` macro in `journal.rs` and `alerts.rs`; the
/// three fixed audit messages are declared in `audit.rs`.
#[test]
fn test_rmc_s25_alert_modules_never_log() {
    for rel in [JOURNAL_RS, ALERTS_RS] {
        let code = code_of(rel);
        let calls = macro_invocations(&code, &TRACING_MACROS);
        assert!(calls.is_empty(), "{rel} must not log: {calls:?}");
        for forbidden in ["tracing::", "use tracing", "println!", "eprintln!", "dbg!"] {
            assert!(
                !code.contains(forbidden),
                "{rel} must not contain {forbidden}"
            );
        }
    }
    let audit = code_of("crates/remote/src/audit.rs");
    for message in [
        "\"password alerts active\"",
        "\"password alerts unavailable\"",
        "\"password alert acknowledgement not persisted\"",
    ] {
        assert!(audit.contains(message), "audit.rs must declare {message}");
    }
    assert!(
        audit.contains("Level::WARN"),
        "two of the messages are WARN"
    );
}

/// Test 46 (RMC-S26, RMC58, A-1): the user unit keeps `RestrictAddressFamilies=AF_UNIX` as
/// its only address-family line and gains no group, network or filesystem directive.
#[test]
fn test_rmc_s26_unit_unchanged_for_alerts() {
    let unit = read("packaging/soos-remote.service");
    let families: Vec<&str> = unit
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("RestrictAddressFamilies="))
        .collect();
    assert_eq!(families, vec!["RestrictAddressFamilies=AF_UNIX"]);
    for forbidden in [
        "SupplementaryGroups=",
        "PrivateNetwork=",
        "ProtectSystem=",
        "ReadWritePaths=",
        "ReadOnlyPaths=",
        "Group=",
        "DynamicUser=",
        "AmbientCapabilities=",
    ] {
        assert!(
            !unit.lines().any(|l| l.trim().starts_with(forbidden)),
            "the unit must not gain {forbidden}"
        );
    }
    for kept in [
        "NoNewPrivileges=yes",
        "MemoryDenyWriteExecute=yes",
        "RestrictSUIDSGID=yes",
        "UMask=0077",
    ] {
        assert!(unit.lines().any(|l| l.trim() == kept), "{kept} stays");
    }
}

/// Test 47 (RMC-S27, RMC58): `crates/remote` enables the tokio `process` feature; the
/// workspace pin and every other manifest do not; `crates/pam` has no tokio at all.
#[test]
fn test_rmc_s27_process_feature_only_in_remote() {
    let remote = read("crates/remote/Cargo.toml");
    let tokio_lines: Vec<&str> = toml_table(&remote, "dependencies")
        .into_iter()
        .filter(|l| l.starts_with("tokio"))
        .collect();
    assert_eq!(tokio_lines.len(), 1, "{tokio_lines:?}");
    assert!(
        tokio_lines[0].contains("workspace = true") && tokio_lines[0].contains("\"process\""),
        "crates/remote takes tokio from the workspace with the process feature: {}",
        tokio_lines[0]
    );
    let root = read("Cargo.toml");
    let workspace_tokio: Vec<&str> = toml_table(&root, "workspace.dependencies")
        .into_iter()
        .filter(|l| l.starts_with("tokio ") || l.starts_with("tokio="))
        .collect();
    assert_eq!(workspace_tokio.len(), 1);
    assert!(
        !workspace_tokio[0].contains("\"process\""),
        "the workspace pin stays without process"
    );
    let crates_dir = crate::remote_companion_contract::workspace_root().join("crates");
    for entry in std::fs::read_dir(crates_dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let manifest = format!("crates/{name}/Cargo.toml");
        if name == "remote" || !exists(&manifest) {
            continue;
        }
        let text = read(&manifest);
        for line in text.lines().filter(|l| l.trim_start().starts_with("tokio")) {
            assert!(!line.contains("\"process\""), "{manifest}: {line}");
        }
    }
    let pam = read("crates/pam/Cargo.toml");
    assert!(
        !pam.lines().any(|l| l.trim_start().starts_with("tokio")),
        "pam_soos.so never links tokio"
    );
}

/// Test 48 (RMC-S28, RMC48, O-2): `JournalEntry` has no `Debug` at all; `OwnerLogin`,
/// `JournalCursor` and `AlertsEpoch` have a redacted manual `Debug` only.
#[test]
fn test_rmc_s28_alert_types_without_redaction_have_no_debug() {
    let journal = code_of(JOURNAL_RS);
    let entry_attrs = attributes_above(&journal, "pub struct JournalEntry");
    assert!(
        !entry_attrs.contains("Debug"),
        "JournalEntry must not derive Debug: {entry_attrs}"
    );
    assert!(
        manual_debug_impl(&journal, "JournalEntry").is_none(),
        "JournalEntry must not implement Debug"
    );
    for (rel, code, ty) in [
        (JOURNAL_RS, &journal, "OwnerLogin"),
        (JOURNAL_RS, &journal, "JournalCursor"),
        (ALERTS_RS, &code_of(ALERTS_RS), "AlertsEpoch"),
    ] {
        let attrs = attributes_above(code, &format!("pub struct {ty}("));
        assert!(
            !attrs.contains("Debug"),
            "{rel}: {ty} must not derive Debug: {attrs}"
        );
        let body = manual_debug_impl(code, ty)
            .unwrap_or_else(|| panic!("{rel}: {ty} needs a manual redacted Debug"));
        assert!(body.contains("<redacted>"), "{rel}: {ty} Debug: {body}");
        assert!(
            !body.contains("self.0"),
            "{rel}: {ty} Debug never prints the value"
        );
    }
}

/// Test 49 (RMC-S29, RMC58): `Docs/REMOTE_COMPANION.md` has the §2c section with the
/// required needles, and `AI/DECISIONS.md` registers the ADR.
#[test]
fn test_rmc_s29_alerts_are_documented() {
    let docs = read("Docs/REMOTE_COMPANION.md");
    let start = docs
        .lines()
        .position(|l| l.starts_with("## 2c."))
        .expect("Docs/REMOTE_COMPANION.md has a `## 2c.` section");
    let section: Vec<&str> = docs
        .lines()
        .skip(start + 1)
        .take_while(|l| !l.starts_with("## "))
        .collect();
    let section = section.join("\n");
    for needle in [
        "password_alerts",
        "lock_screen_programs",
        "wheel",
        "systemd-journal",
        "/api/alerts",
        "alerts-ack",
        "never the typed password",
        "false alerts",
        "swaylock-plugin",
        "stale_view",
    ] {
        assert!(section.contains(needle), "§2c must mention {needle}");
    }
    let decisions = read("AI/DECISIONS.md");
    assert!(decisions.contains("Failed-Password Alerts in `soos-remote` From the System Journal"));
}

/// Test 50 (RMC-S30, RMC57): the page fetches and acknowledges alerts with the snapshot
/// headers, handles `stale_view`, listens for `alerts` events, shows the unavailable and
/// coverage lines and the account labels, and never uses storage, `innerHTML` or a query.
#[test]
fn test_rmc_s30_page_alert_ui() {
    let app = read("crates/remote/assets/app.js");
    for needle in [
        "/api/alerts",
        "/api/alerts/ack",
        "alerts-ack",
        "X-Soos-Alerts-Epoch",
        "X-Soos-Alerts-Through",
        "stale_view",
        "addEventListener(\"alerts\"",
        "Acknowledge",
        "Password alerts unavailable",
        "Lock screen not monitored",
        "No failed password attempts",
        "your account",
        "another account",
        "textContent",
    ] {
        assert!(app.contains(needle), "app.js must contain {needle}");
    }
    for forbidden in [
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "innerHTML",
        "?through=",
        "document.cookie",
    ] {
        assert!(
            !app.contains(forbidden),
            "app.js must not contain {forbidden}"
        );
    }
    let index = read("crates/remote/assets/index.html");
    assert!(
        index.contains("id=\"alerts\""),
        "index.html has the #alerts banner"
    );
}

/// Test 56 (RMC-S31, RMC48, A-14, F-10, F-11): `journal.rs` hands out `Zeroizing` lines,
/// keeps `MESSAGE` in a `Zeroizing<String>`, has no `BufReader` and never parses through
/// `serde_json::Value` / `serde_json::Map` (last-wins on duplicate keys).
#[test]
fn test_rmc_s31_line_buffers_are_zeroizing() {
    let journal = code_of(JOURNAL_RS);
    let compact: String = journal.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        compact.contains("Line(Zeroizing<Vec<u8>>)"),
        "LineRead::Line is Zeroizing"
    );
    assert!(
        compact.contains("pub message: Zeroizing<String>"),
        "JournalEntry.message is Zeroizing"
    );
    assert!(
        !journal.contains("BufReader"),
        "no unwiped intermediate buffer"
    );
    for forbidden in [
        "serde_json::Value",
        "serde_json::Map",
        "serde_json::from_slice::<Value",
    ] {
        assert!(
            !journal.contains(forbidden),
            "journal.rs must not use {forbidden}"
        );
    }
    for line in journal
        .lines()
        .filter(|l| l.trim_start().starts_with("use serde_json"))
    {
        assert!(
            !line.contains("Value") && !line.contains("Map"),
            "journal.rs must not import {line}"
        );
    }
    assert!(
        journal.contains("IgnoredAny"),
        "unknown keys are skipped without being materialised"
    );
}

/// `code` with the contents of every string literal blanked (escapes honoured, raw strings
/// `r"…"` / `r#"…"#` included); char literals and everything else are kept. Used after
/// `strip_comments`, so messages such as "the acknowledged view …" never count as code.
fn blank_string_literals(code: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let raw_start = c == b'r'
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
            && bytes[i + 1..]
                .iter()
                .position(|&b| b != b'#')
                .is_some_and(|p| bytes[i + 1 + p] == b'"');
        if raw_start {
            let hashes = bytes[i + 1..].iter().take_while(|&&b| b == b'#').count();
            let mut close = String::from("\"");
            close.push_str(&"#".repeat(hashes));
            let body_start = i + 2 + hashes;
            let end = code[body_start..]
                .find(&close)
                .map_or(bytes.len(), |p| body_start + p + close.len());
            out.push_str("r\"\"");
            i = end;
            continue;
        }
        if c == b'"' {
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
            out.push_str("\"\"");
            continue;
        }
        if c == b'\'' {
            // Char literal ('x' or an escape) kept whole, so a '"' never opens a string.
            if i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                let start = i;
                i += 2;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                i += 1;
                out.push_str(&code[start..i.min(bytes.len())]);
                continue;
            }
            if i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
                out.push_str(&code[i..i + 3]);
                i += 3;
                continue;
            }
        }
        let ch = code[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Every line of `code` holding `word` as a whole identifier (not part of a longer one such
/// as `unacknowledged_wrong_password` or `acknowledged_until_us`).
fn identifier_uses(code: &str, word: &str) -> Vec<String> {
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    code.lines()
        .filter(|line| {
            let bytes = line.as_bytes();
            line.match_indices(word).any(|(at, _)| {
                let before = at.checked_sub(1).map(|p| bytes[p]);
                let after = bytes.get(at + word.len()).copied();
                !before.is_some_and(ident) && !after.is_some_and(ident)
            })
        })
        .map(|line| line.trim().to_string())
        .collect()
}

/// Test 62 (RMC75, alerts round 3, owner request 2026-10-06 "once the acknowledge button is
/// clicked, delete the entries"; plan-evaluator round-3 F-1): acknowledged records are
/// removed, never kept behind a flag of any visibility: no `acknowledged` identifier
/// (field, binding or access) in the production code of `crates/remote/src`; the page reads
/// and styles no acknowledged row; `Docs/REMOTE_COMPANION.md` §2c states that journal
/// entries are never deleted; the ADR amendment exists.
#[test]
fn test_rmc_s43_acknowledged_entries_are_not_kept() {
    let alerts = code_of(ALERTS_RS);
    assert!(
        !alerts.contains("pub acknowledged:"),
        "AlertRecord has no acknowledged field"
    );
    let mut found = Vec::new();
    for (rel, code) in remote_code() {
        for line in identifier_uses(&blank_string_literals(&code), "acknowledged") {
            found.push(format!("{rel}: {line}"));
        }
    }
    assert!(
        found.is_empty(),
        "acknowledged records must be removed, not flagged: {found:#?}"
    );

    let app = read("crates/remote/assets/app.js");
    for forbidden in [".acknowledged", "\"acknowledged\""] {
        assert!(
            !app.contains(forbidden),
            "app.js must not contain {forbidden}"
        );
    }
    let css = read("crates/remote/assets/style.css");
    assert!(
        !css.contains(".acknowledged"),
        "style.css has no acknowledged row style"
    );

    let docs = read("Docs/REMOTE_COMPANION.md");
    let start = docs
        .lines()
        .position(|l| l.starts_with("## 2c."))
        .expect("Docs/REMOTE_COMPANION.md has a `## 2c.` section");
    let section = docs
        .lines()
        .skip(start + 1)
        .take_while(|l| !l.starts_with("## "))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        section.contains("journal entries are never deleted"),
        "§2c must state that journal entries are never deleted"
    );
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("clear acknowledged entries"),
        "the ADR amendment exists"
    );
}

/// Self-test of the two scanners of test 62 (a wrong scanner would make test 62 vacuous).
#[test]
fn test_rmc_s43_scanner_self_test() {
    let code = "let msg = \"the acknowledged view\";\nlet c = '\"';\nlet raw = r#\"acknowledged\"#;\nrecord.unacknowledged_wrong_password = 1;\nlet acknowledged_until_us = 0;\n";
    assert!(identifier_uses(&blank_string_literals(code), "acknowledged").is_empty());
    for bad in [
        "if record.acknowledged {",
        "    acknowledged: bool,",
        "let acknowledged = true;",
        "Record { id, acknowledged }",
        "r.acknowledged=true",
    ] {
        assert_eq!(
            identifier_uses(&blank_string_literals(bad), "acknowledged").len(),
            1,
            "{bad}"
        );
    }
}
