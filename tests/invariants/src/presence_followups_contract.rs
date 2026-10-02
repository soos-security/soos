//! Presence auto-unlock review follow-ups (GitHub #325; matrix PFU2, PFU3, PFU4, PFU6):
//!
//! - only `ZbusLogind::connect` builds a bus connection, and the worker bounds `connect()` with
//!   `DBUS_CONNECT_TIMEOUT_MS`, never with the `bounded()` call wrapper;
//! - the coverage of `DBUS_CALL_TIMEOUT_MS` / `DBUS_CONNECT_TIMEOUT_MS` is documented in code
//!   and in `Docs/DAEMON.md` §6;
//! - the PAM line scan's deliberate over-detection and the `/etc/pam.conf` rule are documented;
//! - the worker probes the store before any D-Bus traffic.

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

/// Source with `//` comment lines removed (doc comments included).
fn code(content: &str) -> String {
    content
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The doc comment lines (`///`) directly above the first line containing `item`.
fn doc_of(content: &str, item: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let index = lines
        .iter()
        .position(|l| l.contains(item))
        .unwrap_or_else(|| panic!("`{item}` not found"));
    let mut doc = Vec::new();
    for line in lines[..index].iter().rev() {
        let trimmed = line.trim_start();
        if let Some(text) = trimmed.strip_prefix("///") {
            doc.push(text.trim());
        } else if trimmed.starts_with("#[") {
            continue;
        } else {
            break;
        }
    }
    doc.reverse();
    doc.join(" ")
}

/// The body (braces included) of the first item whose header contains `signature`.
fn fn_body<'a>(content: &'a str, signature: &str) -> &'a str {
    let start = content
        .find(signature)
        .unwrap_or_else(|| panic!("`{signature}` not found"));
    let open = start + content[start..].find('{').expect("function body");
    let mut depth = 0usize;
    for (i, c) in content[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &content[open..=open + i];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces after `{signature}`")
}

/// PFU2: a bus connection is built only inside `ZbusLogind::connect`; the call path never
/// opens one (so no reconnect can hide under the 500 ms call bound).
#[test]
fn test_pfu_only_connect_opens_a_bus_connection() {
    let logind = code(&read("crates/daemon/src/presence/logind.rs"));
    assert_eq!(
        logind.matches(".build()").count(),
        1,
        "exactly one connection build in logind.rs"
    );
    assert_eq!(
        logind.matches("Builder::address(").count(),
        1,
        "exactly one connection builder in logind.rs"
    );
    let connect = fn_body(&logind, "async fn connect(&self)");
    assert!(
        connect.contains(".build()") && connect.contains("Builder::address("),
        "the builder and `.build()` live in `ZbusLogind::connect`"
    );
    assert!(
        connect.contains("DBUS_CONNECT_TIMEOUT_MS"),
        "`ZbusLogind::connect` keeps its own connect bound"
    );
    let call = fn_body(&logind, "async fn call<");
    assert!(
        !call.contains(".build()") && !call.contains("Builder::"),
        "`ZbusLogind::call` never builds a connection"
    );
    let trait_body = fn_body(&logind, "pub trait PresenceLogind");
    assert!(
        trait_body.contains("fn connect(&self)"),
        "`PresenceLogind::connect` is a trait method"
    );
}

/// PFU2: the worker awaits `connect()` under `DBUS_CONNECT_TIMEOUT_MS`, never inside the
/// `bounded()` call wrapper, and before the snapshot.
#[test]
fn test_pfu_worker_bounds_connect_outside_the_call_bound() {
    let worker = code(&read("crates/daemon/src/presence/worker.rs"));
    let compact: String = worker.split_whitespace().collect();
    assert!(
        !compact.contains("bounded(self.logind.connect()"),
        "connect() must not run under the DBUS_CALL_TIMEOUT_MS wrapper"
    );
    assert!(
        compact.contains("Duration::from_millis(DBUS_CONNECT_TIMEOUT_MS),self.logind.connect()"),
        "the worker bounds connect() with DBUS_CONNECT_TIMEOUT_MS"
    );
    let connect_at = compact.find("self.logind.connect()").unwrap();
    let snapshot_at = compact
        .find("bounded(self.logind.seat_sessions())")
        .expect("the snapshot stays under bounded()");
    assert!(connect_at < snapshot_at, "connect() precedes the snapshot");
    assert_eq!(
        compact.matches("self.logind.connect()").count(),
        1,
        "one connect step per tick"
    );
}

/// PFU6: the store probe precedes every D-Bus call of the tick.
#[test]
fn test_pfu_worker_probes_the_store_before_any_dbus_call() {
    let worker = code(&read("crates/daemon/src/presence/worker.rs"));
    let compact: String = worker.split_whitespace().collect();
    let tick = fn_body(&compact, "pubasyncfntick(&mutself)");
    let probe = tick
        .find(".has_enrolled_template()")
        .expect("the tick probes the store");
    let first_dbus = tick.find("self.logind.").expect("the tick calls logind");
    assert!(
        probe < first_dbus,
        "the store probe runs before the first logind access"
    );
}

/// PFU2: what each bound covers is documented on the constants.
#[test]
fn test_pfu_logind_bounds_are_documented() {
    let module = read("crates/daemon/src/presence/mod.rs");
    let call = doc_of(&module, "pub const DBUS_CALL_TIMEOUT_MS");
    for needle in ["snapshot", "round trip"] {
        assert!(
            call.contains(needle),
            "DBUS_CALL_TIMEOUT_MS doc must say it bounds the whole snapshot and each round \
             trip (missing `{needle}`): {call}"
        );
    }
    let connect = doc_of(&module, "pub const DBUS_CONNECT_TIMEOUT_MS");
    for needle in ["connect()", "outside"] {
        assert!(
            connect.contains(needle),
            "DBUS_CONNECT_TIMEOUT_MS doc must say it bounds connect() outside the call bound \
             (missing `{needle}`): {connect}"
        );
    }
    assert!(
        module.contains("pub const DBUS_CALL_TIMEOUT_MS: u64 = 500;")
            && module.contains("pub const DBUS_CONNECT_TIMEOUT_MS: u64 = 1000;"),
        "the bound values are unchanged"
    );
}

/// PFU4 / PFU3: the deliberate over-detection of the PAM line scan and the `/etc/pam.conf`
/// rule are documented in `account.rs`.
#[test]
fn test_pfu_account_guard_documents_over_detection_and_pam_conf() {
    let account = read("crates/daemon/src/presence/account.rs");
    let docs: String = account
        .lines()
        .filter_map(|l| {
            let t = l.trim_start();
            t.strip_prefix("///").or_else(|| t.strip_prefix("//!"))
        })
        .collect::<Vec<_>>()
        .join(" ");
    for needle in ["over-detect", "superset", "Unicode whitespace", "pam.conf"] {
        assert!(
            docs.contains(needle),
            "account.rs doc comments must mention `{needle}`"
        );
    }
    let module = read("crates/daemon/src/presence/mod.rs");
    assert!(module.contains("pub const DEFAULT_PAM_CONF: &str = \"/etc/pam.conf\";"));
}

/// PFU2 / PFU3 / PFU6: the operator reference describes the follow-ups.
#[test]
fn test_pfu_daemon_docs_describe_the_followups() {
    let docs = read("Docs/DAEMON.md");
    let start = docs
        .find("## 6. Presence Auto-Unlock")
        .expect("Docs/DAEMON.md §6");
    let section: String = docs[start..]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for needle in [
        "/etc/pam.conf",
        "DBUS_CONNECT_TIMEOUT_MS",
        "whole snapshot",
        "no template",
    ] {
        assert!(
            section.contains(needle),
            "Docs/DAEMON.md §6 must mention `{needle}`"
        );
    }
}
