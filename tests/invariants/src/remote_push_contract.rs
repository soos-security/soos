//! Static contracts of Web Push for the remote companion (ADR 2026-10-06 "Web Push
//! Notifications for Failed-Password Alerts Through a Separate Sender Unit", architect spec
//! `AI/architect_spec_remote_web_push.md` §12.9 RMC-S32–RMC-S42, tests 42–51 and 53; matrix
//! RMC62, RMC69, RMC71–RMC73):
//!
//! - RMC-S32 the two new crates are registered, isolated and forbid raw-memory code;
//! - RMC-S33 the TLS stack (ureq + rustls + ring + webpki-roots) lives in the sender only;
//! - RMC-S34 the sender's outbound policy literals and its single filtering resolver;
//! - RMC-S35 the sender unit is exactly the sandbox of spec §8.2 (plan-evaluator F-13:
//!   `ProtectControlGroups=` is unsupported in per-user managers and is not part of it);
//! - RMC-S36 production hygiene of the push code (no panicking construct, constant logs);
//! - RMC-S37 the push modules never log and secret-bearing types are redacted;
//! - RMC-S38 the page and the service worker;
//! - RMC-S39 the feature is documented and the ADR exists;
//! - RMC-S40 the installer installs the sender without enabling anything;
//! - RMC-S41 keys never reach the sender;
//! - RMC-S42 the sender's logging is fixed and has no `log` bridge (plan-evaluator F-15:
//!   `SubscriberInitExt::set_default()` installs the bridge as well and is forbidden).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::process::Command;

use crate::remote_companion_contract::{
    exists, macro_invocations, production_part, read, rust_files, strip_comments, toml_table,
    workspace_root, TRACING_MACROS,
};

const PROTOCOL_LIB: &str = "crates/push-protocol/src/lib.rs";
const SENDER_LIB: &str = "crates/push-sender/src/lib.rs";
const SENDER_MAIN: &str = "crates/push-sender/src/main.rs";
const PROTOCOL_MANIFEST: &str = "crates/push-protocol/Cargo.toml";
const SENDER_MANIFEST: &str = "crates/push-sender/Cargo.toml";
const PUSH_RS: &str = "crates/remote/src/push.rs";
const WEBPUSH_RS: &str = "crates/remote/src/webpush.rs";
const SENDER_UNIT: &str = "packaging/soos-push-sender.service";
const ADR_TITLE: &str =
    "Web Push Notifications for Failed-Password Alerts Through a Separate Sender Unit";

/// Production code (comments stripped, trailing test module removed) of `rel`.
fn code_of(rel: &str) -> String {
    assert!(exists(rel), "{rel} must exist (spec §1.1)");
    strip_comments(production_part(&read(rel)))
}

/// Production code of every source under `dir`.
fn code_under(dir: &str) -> Vec<(String, String)> {
    let files: Vec<(String, String)> = rust_files(dir)
        .into_iter()
        .map(|(rel, content)| {
            let code = strip_comments(production_part(&content));
            (rel, code)
        })
        .collect();
    assert!(!files.is_empty(), "{dir} has sources");
    files
}

/// The dependency keys of a manifest table (`name = …` or `name.workspace = true`).
fn dependency_keys(table: &[&str]) -> Vec<String> {
    table
        .iter()
        .filter_map(|line| {
            let key = line.split('=').next()?.trim();
            let key = key.split('.').next()?.trim().trim_matches('"');
            (!key.is_empty()).then(|| key.to_string())
        })
        .collect()
}

/// The attribute lines (`#[…]`) directly above the item line containing `item`.
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

/// A manual `Debug` implementation for `ty`, if any.
fn manual_debug_impl(code: &str, ty: &str) -> Option<String> {
    for prefix in [
        "impl fmt::Debug for ",
        "impl std::fmt::Debug for ",
        "impl core::fmt::Debug for ",
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

/// The body (between the first `{` and its matching `}`) of the first item whose line
/// contains `signature`.
fn body_of(code: &str, signature: &str) -> String {
    let at = code
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} not found"));
    let open = at + code[at..].find('{').expect("a body");
    let mut depth = 0usize;
    for (i, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return code[open + 1..open + i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced body of {signature}");
}

// ---------------------------------------------------------------------------------------
// Test 42 — RMC-S32
// ---------------------------------------------------------------------------------------

/// Test 42 (RMC-S32, RMC71, W-3, W-4): both crates are workspace members inheriting the
/// workspace package fields and lints; no new manifest names `soos-remote`; the protocol
/// crate has no runtime, network or system dependency; `soos-remote` depends on the
/// protocol crate, `hmac` and `aes-gcm` from the workspace and never on the TLS stack or the
/// sender; both crates forbid raw-memory code and are listed as business crates.
#[test]
fn test_rmc_s32_push_crates_are_registered_and_isolated() {
    let root = read("Cargo.toml");
    let members = toml_table(&root, "workspace");
    for member in ["\"crates/push-protocol\",", "\"crates/push-sender\","] {
        assert!(
            members.contains(&member),
            "workspace member {member} missing: {members:?}"
        );
    }
    let workspace = toml_table(&root, "workspace.dependencies");
    assert!(
        workspace.contains(&"hmac = \"0.12\""),
        "[workspace.dependencies] hmac = \"0.12\": {workspace:?}"
    );
    assert!(
        workspace.contains(
            &"soos-push-protocol = { path = \"crates/push-protocol\", version = \"0.1.0\" }"
        ),
        "[workspace.dependencies] soos-push-protocol path entry"
    );

    for (manifest_path, name) in [
        (PROTOCOL_MANIFEST, "soos-push-protocol"),
        (SENDER_MANIFEST, "soos-push-sender"),
    ] {
        let manifest = read(manifest_path);
        let package = toml_table(&manifest, "package");
        assert!(
            package.contains(&format!("name = \"{name}\"").as_str()),
            "{manifest_path}: package name {name}"
        );
        for field in ["version", "edition", "license", "publish"] {
            assert!(
                package.contains(&format!("{field}.workspace = true").as_str()),
                "{manifest_path}: {field}.workspace = true"
            );
        }
        assert_eq!(
            toml_table(&manifest, "lints"),
            vec!["workspace = true"],
            "{manifest_path}: [lints] workspace = true"
        );
        assert!(
            !manifest.contains("soos-remote"),
            "{manifest_path} must not contain the text soos-remote (RMC-S4)"
        );
    }

    let protocol = read(PROTOCOL_MANIFEST);
    for table in ["dependencies", "build-dependencies"] {
        for key in dependency_keys(&toml_table(&protocol, table)) {
            assert!(
                !["tokio", "ureq", "rustls", "nix", "ring", "libc"].contains(&key.as_str()),
                "soos-push-protocol [{table}] must not depend on {key}"
            );
        }
    }

    let remote = read("crates/remote/Cargo.toml");
    let deps = toml_table(&remote, "dependencies");
    for name in ["soos-push-protocol", "hmac", "aes-gcm"] {
        assert!(
            deps.contains(&format!("{name} = {{ workspace = true }}").as_str()),
            "crates/remote [dependencies] takes {name} from the workspace: {deps:?}"
        );
    }
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        for key in dependency_keys(&toml_table(&remote, table)) {
            assert!(
                !["ureq", "rustls", "soos-push-sender", "webpki-roots"].contains(&key.as_str()),
                "crates/remote [{table}] must not depend on {key}"
            );
        }
    }

    for rel in [PROTOCOL_LIB, SENDER_LIB, SENDER_MAIN] {
        let content = read(rel);
        let first = content
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("//"))
            .unwrap_or("");
        assert_eq!(
            first, "#![forbid(unsafe_code)]",
            "{rel} starts with the crate-level forbid attribute"
        );
    }

    let lib = read("tests/invariants/src/lib.rs");
    let start = lib
        .find("fn test_business_crates_forbid_unsafe_code")
        .expect("business crate invariant");
    let list = &lib[start..start + lib[start..].find("];").unwrap()];
    for name in ["\"push-protocol\"", "\"push-sender\""] {
        assert!(list.contains(name), "business_crates lists {name}");
    }
    let candid = read("scripts/candid_review.sh");
    let line = candid
        .lines()
        .find(|l| l.starts_with("BUSINESS_CRATES=("))
        .expect("BUSINESS_CRATES line");
    let names: Vec<&str> = line
        .trim_start_matches("BUSINESS_CRATES=(")
        .trim_end_matches(')')
        .split_whitespace()
        .collect();
    for name in ["push-protocol", "push-sender", "remote"] {
        assert!(
            names.contains(&name),
            "candid_review.sh BUSINESS_CRATES lists {name}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Test 43 — RMC-S33
// ---------------------------------------------------------------------------------------

fn tree_names(package: &str) -> Option<Vec<String>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    if Command::new(&cargo).arg("--version").output().is_err() {
        return None;
    }
    let output = Command::new(&cargo)
        .current_dir(workspace_root())
        .args([
            "tree",
            "--offline",
            "--locked",
            "-p",
            package,
            "-e",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])
        .output()
        .expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo tree -p {package} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .map(str::to_string)
            .collect(),
    )
}

/// Test 43 (RMC-S33, RMC72, W-6): the workspace `ureq` line is the exact pin without default
/// features; the sender's normal tree has rustls, ring and webpki-roots and no OpenSSL,
/// native-tls, aws-lc or `rand`; `soos-remote`'s tree has none of ureq, rustls, ring; the
/// `p256` line is unchanged.
#[test]
fn test_rmc_s33_sender_tls_stack_without_openssl() {
    let root = read("Cargo.toml");
    let workspace = toml_table(&root, "workspace.dependencies");
    assert!(
        workspace.contains(
            &"ureq = { version = \"=3.4.2\", default-features = false, features = [\"rustls\"] }"
        ),
        "[workspace.dependencies] ureq exact line: {workspace:?}"
    );
    assert!(
        workspace.contains(
            &"p256 = { version = \"0.13.2\", default-features = false, features = [\"ecdsa\"] }"
        ),
        "the p256 line is unchanged (RMC-S13)"
    );
    let sender = read(SENDER_MANIFEST);
    assert!(
        toml_table(&sender, "dependencies").contains(&"ureq = { workspace = true }"),
        "the sender takes ureq from the workspace"
    );
    let Some(names) = tree_names("soos-push-sender") else {
        return;
    };
    for required in [
        "rustls",
        "ring",
        "webpki-roots",
        "ureq",
        "soos-push-protocol",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "{required} is in the sender's normal tree"
        );
    }
    for forbidden in [
        "openssl",
        "openssl-sys",
        "native-tls",
        "aws-lc-rs",
        "aws-lc-sys",
        "rand",
        "rustls-native-certs",
    ] {
        assert!(
            !names.iter().any(|n| n == forbidden),
            "the sender's normal tree contains {forbidden}"
        );
    }
    let remote = tree_names("soos-remote").expect("cargo available");
    for forbidden in ["ureq", "rustls", "ring", "webpki-roots", "soos-push-sender"] {
        assert!(
            !remote.iter().any(|n| n == forbidden),
            "soos-remote's normal tree contains {forbidden}"
        );
    }
    assert!(remote.iter().any(|n| n == "soos-push-protocol"));
}

// ---------------------------------------------------------------------------------------
// Test 44 — RMC-S34
// ---------------------------------------------------------------------------------------

/// Test 44 (RMC-S34, RMC62, W-5, F-3): the sender builds its only agent with
/// `Agent::with_parts` and the filtering resolver, https only, no redirect, no proxy, a
/// global timeout and statuses as values; no unfiltered constructor, no custom certificate
/// verifier, no process spawn; the only environment variable read is `XDG_RUNTIME_DIR`.
#[test]
fn test_rmc_s34_sender_policy_literals() {
    let files = code_under("crates/push-sender/src");
    let all: String = files
        .iter()
        .map(|(_, c)| c.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for needle in [
        ".https_only(true)",
        ".max_redirects(0)",
        ".proxy(None)",
        "timeout_global",
        "http_status_as_error(false)",
        "FilteringResolver",
        "Agent::with_parts(",
        "ResolveRefused",
    ] {
        assert!(
            all.contains(needle),
            "the sender code must contain {needle}"
        );
    }
    assert!(
        all.contains("impl Resolver for FilteringResolver")
            || all.contains("impl ureq::unversioned::resolver::Resolver for FilteringResolver")
            || all.contains("impl resolver::Resolver for FilteringResolver"),
        "FilteringResolver implements ureq's Resolver"
    );
    assert_eq!(
        all.matches("Agent::with_parts(").count(),
        1,
        "one agent construction"
    );
    for (rel, code) in &files {
        for forbidden in [
            "Agent::new_with_config",
            "Agent::new_with_defaults",
            "Agent::from(",
            "ureq::post(",
            "ureq::get(",
            "ureq::agent(",
            "danger",
            "Dangerous",
            "ServerCertVerifier",
            "NoVerifier",
            "native_certs",
            "Command::new",
            "std::process::Command",
            "set_var(",
            "RUST_LOG",
        ] {
            assert!(!code.contains(forbidden), "{rel} contains {forbidden}");
        }
        for reader in ["env::var(", "env::var_os(", "std::env::vars"] {
            let mut start = 0;
            while let Some(pos) = code[start..].find(reader) {
                let at = start + pos + reader.len();
                assert!(
                    code[at..].starts_with("\"XDG_RUNTIME_DIR\")"),
                    "{rel}: {reader} reads something other than XDG_RUNTIME_DIR"
                );
                start = at;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Test 45 — RMC-S35
// ---------------------------------------------------------------------------------------

/// The exact non-comment line set of `packaging/soos-push-sender.service` (spec §8.2 minus
/// `ProtectControlGroups=yes`, plan-evaluator F-13).
const SENDER_UNIT_LINES: [&str; 37] = [
    "[Unit]",
    "Description=soos push sender (outbound Web Push for the remote companion)",
    "Documentation=https://github.com/Mysticaly622/soos/blob/main/Docs/REMOTE_COMPANION.md",
    "[Service]",
    "Type=exec",
    "ExecStart=%h/.local/bin/soos-push-sender",
    "Restart=on-failure",
    "RestartSec=5",
    "RestartPreventExitStatus=78",
    "RuntimeDirectory=soos-push",
    "RuntimeDirectoryMode=0700",
    "NoNewPrivileges=yes",
    "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
    "LockPersonality=yes",
    "MemoryDenyWriteExecute=yes",
    "RestrictRealtime=yes",
    "RestrictSUIDSGID=yes",
    "SystemCallArchitectures=native",
    "ProtectSystem=strict",
    "ProtectHome=tmpfs",
    "BindReadOnlyPaths=%h/.local/bin/soos-push-sender",
    "TemporaryFileSystem=/run:ro",
    "PrivateTmp=yes",
    "PrivateDevices=yes",
    "PrivateIPC=yes",
    "ProtectKernelTunables=yes",
    "ProtectKernelModules=yes",
    "ProtectKernelLogs=yes",
    "ProtectClock=yes",
    "ProtectHostname=yes",
    "RestrictNamespaces=yes",
    "CapabilityBoundingSet=",
    "SystemCallFilter=@system-service",
    "SystemCallErrorNumber=EPERM",
    "UMask=0077",
    "[Install]",
    "WantedBy=default.target",
];

/// Test 45 (RMC-S35, RMC71, W-3, F-2, F-13): the sender unit is exactly the §8.2 sandbox
/// (no line missing, none added, in order); the user manager's unsupported directives and
/// every widening directive are absent; `soos-remote.service` keeps `AF_UNIX` only.
#[test]
fn test_rmc_s35_sender_unit_is_sandboxed() {
    let unit = read(SENDER_UNIT);
    let lines: Vec<&str> = unit
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with(';'))
        .collect();
    assert_eq!(
        lines,
        SENDER_UNIT_LINES.to_vec(),
        "exact sandbox of spec §8.2"
    );
    for forbidden in [
        "ProtectHome=read-only",
        "ProtectHome=no",
        "BindPaths=",
        "ReadWritePaths=",
        "ReadOnlyPaths=",
        "SupplementaryGroups=",
        "User=",
        "Group=",
        "DynamicUser=",
        "AmbientCapabilities=",
        "PrivateNetwork=",
        "ProtectProc=",
        "ProcSubset=",
        "ProtectControlGroups=",
        "Environment=",
        "EnvironmentFile=",
    ] {
        assert!(
            !lines.iter().any(|l| l.starts_with(forbidden)),
            "{SENDER_UNIT} must not contain {forbidden}"
        );
    }
    let remote_unit = read("packaging/soos-remote.service");
    let families: Vec<&str> = remote_unit
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("RestrictAddressFamilies="))
        .collect();
    assert_eq!(families, vec!["RestrictAddressFamilies=AF_UNIX"]);
}

// ---------------------------------------------------------------------------------------
// Test 46 — RMC-S36
// ---------------------------------------------------------------------------------------

/// Test 46 (RMC-S36, O-5, F-8): no panicking or printing construct and no lint allowance in
/// the push production code; every `tracing` macro of the sender has exactly one constant
/// string argument; no wall clock in `push.rs` and `webpush.rs`.
#[test]
fn test_rmc_s36_push_production_code_hygiene() {
    let mut files = code_under("crates/push-protocol/src");
    files.extend(code_under("crates/push-sender/src"));
    files.push((PUSH_RS.to_string(), code_of(PUSH_RS)));
    files.push((WEBPUSH_RS.to_string(), code_of(WEBPUSH_RS)));
    for (rel, code) in &files {
        for forbidden in [
            ".unwrap()",
            ".expect(",
            "panic!(",
            "unreachable!(",
            "todo!(",
            "unimplemented!(",
            "print!(",
            "println!(",
            "eprint!(",
            "eprintln!(",
            "dbg!(",
            "allow(clippy::unwrap_used",
            "allow(clippy::expect_used",
            "allow(clippy::panic",
            "allow(clippy::indexing_slicing",
            "expect(clippy::",
            "process::exit",
        ] {
            if forbidden == "process::exit" && rel == SENDER_MAIN {
                continue;
            }
            assert!(!code.contains(forbidden), "{rel} contains {forbidden}");
        }
    }
    for (rel, code) in code_under("crates/push-sender/src") {
        for args in macro_invocations(&code, &TRACING_MACROS) {
            let args = args.trim();
            assert!(
                args.starts_with('"') && args.ends_with('"') && args.matches('"').count() == 2,
                "{rel}: a tracing macro takes exactly one string literal: {args}"
            );
            assert!(!args.contains('{'), "{rel}: no format argument: {args}");
        }
    }
    for rel in [PUSH_RS, WEBPUSH_RS] {
        let code = code_of(rel);
        assert!(
            !code.contains("SystemTime::now"),
            "{rel} reads the wall clock"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Test 47 — RMC-S37
// ---------------------------------------------------------------------------------------

/// Test 47 (RMC-S37, RMC69, RMC70, W-14): `push.rs` and `webpush.rs` use no `tracing` macro;
/// `audit.rs` declares the five fixed push messages; `VapidKey` and `PushEndpoint` have a
/// manual redacted `Debug` only; the secret-bearing types have no `Debug` at all.
#[test]
fn test_rmc_s37_push_modules_never_log_and_types_are_redacted() {
    for rel in [PUSH_RS, WEBPUSH_RS] {
        let code = code_of(rel);
        assert!(
            macro_invocations(&code, &TRACING_MACROS).is_empty(),
            "{rel} must not use a tracing macro"
        );
        assert!(!code.contains("tracing::"), "{rel} must not use tracing");
    }
    let audit = code_of("crates/remote/src/audit.rs");
    for message in [
        "\"push subscription added\"",
        "\"push subscription removed\"",
        "\"push delivery failed\"",
        "\"push sender unavailable\"",
        "\"push notifications unavailable\"",
    ] {
        assert!(audit.contains(message), "audit.rs declares {message}");
    }

    let webpush = code_of(WEBPUSH_RS);
    let protocol = code_of(PROTOCOL_LIB);
    let push = code_of(PUSH_RS);
    for (code, ty, item) in [
        (&webpush, "VapidKey", "pub struct VapidKey"),
        (&protocol, "PushEndpoint", "pub struct PushEndpoint"),
    ] {
        let attrs = attributes_above(code, item);
        assert!(
            !attrs.contains("Debug"),
            "{ty} has no derived Debug: {attrs}"
        );
        let manual =
            manual_debug_impl(code, ty).unwrap_or_else(|| panic!("{ty} has a manual Debug"));
        assert!(
            manual.contains("<redacted>"),
            "{ty} Debug is redacted: {manual}"
        );
    }
    for (code, ty, item) in [
        (&protocol, "DeliveryRequest", "pub struct DeliveryRequest"),
        (&webpush, "UaKeys", "pub struct UaKeys"),
        (&push, "PushSubscription", "pub struct PushSubscription"),
        (&push, "PushStoreFile", "pub struct PushStoreFile"),
    ] {
        let attrs = attributes_above(code, item);
        assert!(
            !attrs.contains("Debug"),
            "{ty} has no derived Debug: {attrs}"
        );
        assert!(
            manual_debug_impl(code, ty).is_none(),
            "{ty} has no Debug at all"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Test 48 — RMC-S38
// ---------------------------------------------------------------------------------------

/// Test 48 (RMC-S38, RMC73, W-11, §9): the worker always shows a notification and does
/// nothing else; the page registers it, asks permission from a user gesture before
/// subscribing, and offers the three buttons.
#[test]
fn test_rmc_s38_page_and_service_worker() {
    let sw = read("crates/remote/assets/sw.js");
    for needle in [
        "addEventListener(\"push\"",
        "waitUntil(",
        "showNotification(",
        "notificationclick",
        "openWindow(",
        "soos-test",
        "soos-alerts",
    ] {
        assert!(sw.contains(needle), "sw.js must contain {needle}");
    }
    for forbidden in [
        "addEventListener(\"fetch\"",
        "importScripts",
        "caches",
        "localStorage",
        "indexedDB",
        "http://",
        "https://",
        "eval(",
        "innerHTML",
        "Function(",
    ] {
        assert!(
            !sw.contains(forbidden),
            "sw.js must not contain {forbidden}"
        );
    }
    let app = read("crates/remote/assets/app.js");
    for needle in [
        "/sw.js",
        "serviceWorker.register(",
        "Notification.requestPermission(",
        "pushManager.subscribe(",
        "userVisibleOnly: true",
        "/api/push/subscribe",
        "/api/push/unsubscribe",
        "/api/push/test",
        "push-subscribe",
        "push-unsubscribe",
        "push-test",
        "Enable notifications",
        "Send test notification",
        "Disable notifications",
        "home-screen app",
        "Notifications must be re-enabled on this phone",
        "Push sender not running on the PC",
        "soos-remote push reset",
    ] {
        assert!(app.contains(needle), "app.js must contain {needle}");
    }
    let body = body_of(&app, "async function enableNotifications(");
    let first_await = body.find("await").expect("enableNotifications awaits");
    assert!(
        body[first_await + "await".len()..]
            .trim_start()
            .starts_with("Notification.requestPermission("),
        "the first awaited call of enableNotifications is Notification.requestPermission()"
    );
    for forbidden in ["innerHTML", "localStorage", "sessionStorage", "indexedDB"] {
        assert!(
            !app.contains(forbidden),
            "app.js must not contain {forbidden}"
        );
    }
    let index = read("crates/remote/assets/index.html");
    assert!(
        index.contains("id=\"push\""),
        "index.html has the push card"
    );
}

// ---------------------------------------------------------------------------------------
// Test 49 — RMC-S39
// ---------------------------------------------------------------------------------------

/// Test 49 (RMC-S39, RMC73, §16): `Docs/REMOTE_COMPANION.md` §2d documents the owner steps,
/// options and risks; §9 no longer lists push notifications as out of scope but still
/// lists the live camera; the ADR exists.
#[test]
fn test_rmc_s39_push_is_documented() {
    let doc = read("Docs/REMOTE_COMPANION.md");
    let lines: Vec<&str> = doc.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with("## 2d."))
        .expect("Docs/REMOTE_COMPANION.md has a `## 2d.` section");
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("## "))
        .map_or(lines.len(), |p| start + 1 + p);
    let section = lines[start..end].join("\n");
    for needle in [
        "push_notifications",
        "soos-push-sender.service",
        "systemctl --user enable --now soos-push-sender",
        "home-screen",
        "16.4",
        "Show Previews",
        "push_previews",
        "vapid_subject",
        "BadJwtToken",
        "never the typed password",
        "false notifications",
        "web.push.apple.com",
        "soos-remote push list",
        "soos-remote push remove",
        "soos-remote push reset",
        "abstract",
        "RUST_LOG",
    ] {
        assert!(section.contains(needle), "§2d must mention {needle}");
    }
    let out_start = lines
        .iter()
        .position(|l| l.starts_with("## 9."))
        .expect("§9 exists");
    let out_end = lines[out_start + 1..]
        .iter()
        .position(|l| l.starts_with("## "))
        .map_or(lines.len(), |p| out_start + 1 + p);
    let out = &lines[out_start..out_end];
    assert!(
        !out.iter()
            .any(|l| l.to_ascii_lowercase().starts_with("- **push notification")),
        "§9 no longer lists push notifications as out of scope"
    );
    assert!(
        out.iter()
            .any(|l| l.to_ascii_lowercase().contains("live camera")),
        "§9 still lists the live camera"
    );
    let decisions = read("AI/DECISIONS.md");
    assert!(decisions.contains(ADR_TITLE), "the ADR exists");
}

// ---------------------------------------------------------------------------------------
// Test 50 — RMC-S40
// ---------------------------------------------------------------------------------------

/// Test 50 (RMC-S40, RMC73): the installer builds and installs the sender binary and unit,
/// writes the commented template key, removes both on `--uninstall`, and only prints the
/// enabling commands.
#[test]
fn test_rmc_s40_installer_installs_the_sender_without_enabling_it() {
    let script = read("scripts/install_remote.sh");
    for needle in [
        "-p soos-push-sender",
        "packaging/soos-push-sender.service",
        "systemd/user/soos-push-sender.service",
        "# push_notifications = false",
    ] {
        assert!(
            script.contains(needle),
            "install_remote.sh must contain {needle}"
        );
    }
    assert!(
        script.contains(".local/bin/soos-push-sender")
            || script.contains("${BIN_DIR}/soos-push-sender"),
        "install_remote.sh installs the sender binary"
    );
    let uninstall = script
        .find("--uninstall")
        .expect("install_remote.sh has --uninstall");
    let rest = &script[uninstall..];
    assert!(
        rest.contains("soos-push-sender.service") && rest.matches("soos-push-sender").count() >= 2,
        "the uninstall path removes the sender binary and unit"
    );
    for line in script.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if line.contains("systemctl --user enable") || line.contains("tailscale ") {
            assert!(
                trimmed.starts_with("echo")
                    || trimmed.starts_with("printf")
                    || trimmed.starts_with('"')
                    || trimmed.starts_with("'"),
                "enabling or tailscale commands are only printed: {line}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// Test 51 — RMC-S41
// ---------------------------------------------------------------------------------------

/// Test 51 (RMC-S41, RMC71, W-3): the sender names no key type, no crypto crate and no
/// store or configuration file of `soos-remote`, and its manifest has no crypto dependency.
#[test]
fn test_rmc_s41_push_keys_never_reach_the_sender() {
    for (rel, code) in code_under("crates/push-sender/src") {
        for forbidden in [
            "VapidKey",
            "SigningKey",
            "SecretKey",
            "p256",
            "aes_gcm",
            "hmac",
            "remote-push.json",
            "remote-passkeys.json",
            "remote-alerts.json",
            "remote.toml",
            "remote.sock",
        ] {
            assert!(!code.contains(forbidden), "{rel} names {forbidden}");
        }
    }
    let manifest = read(SENDER_MANIFEST);
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        for key in dependency_keys(&toml_table(&manifest, table)) {
            assert!(
                !["p256", "aes-gcm", "hmac", "sha2", "ecdsa", "getrandom"].contains(&key.as_str()),
                "soos-push-sender [{table}] must not depend on {key}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// Test 53 — RMC-S42
// ---------------------------------------------------------------------------------------

/// Test 53 (RMC-S42, RMC69, W-14, F-1, F-15): the sender installs exactly one subscriber
/// with a fixed filter (dependency targets off) through the free function
/// `tracing::subscriber::set_global_default`, inside `install_logging`, first thing in
/// `main`; no `log` bridge (`.init()`, `try_init`, `SubscriberInitExt`, `.set_default(`,
/// `LogTracer`), no environment filter; no `log`/`tracing-log` dependency.
#[test]
fn test_rmc_s42_sender_logging_is_fixed_and_bridgeless() {
    let files = code_under("crates/push-sender/src");
    let all: String = files
        .iter()
        .map(|(_, c)| c.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for needle in [
        "set_global_default",
        "Targets::new()",
        "LevelFilter::INFO",
        ".with_target(\"ureq\", LevelFilter::OFF)",
        ".with_target(\"ureq_proto\", LevelFilter::OFF)",
        ".with_target(\"rustls\", LevelFilter::OFF)",
    ] {
        assert!(
            all.contains(needle),
            "the sender's logging must contain {needle}"
        );
    }
    for (rel, code) in &files {
        for forbidden in [
            ".init()",
            "try_init",
            "SubscriberInitExt",
            ".set_default(",
            "set_default(",
            "LogTracer",
            "tracing_log",
            "EnvFilter",
            "from_default_env",
            "RUST_LOG",
            "log::set_logger",
            "set_boxed_logger",
            "log::set_max_level",
            "with_env_filter",
        ] {
            assert!(!code.contains(forbidden), "{rel} contains {forbidden}");
        }
    }
    let lib = code_of(SENDER_LIB);
    let install = body_of(&lib, "pub fn install_logging(");
    assert!(
        install.contains("tracing::subscriber::set_global_default(")
            || install.contains("subscriber::set_global_default("),
        "install_logging calls the free set_global_default: {install}"
    );
    let main = code_of(SENDER_MAIN);
    let body = body_of(&main, "fn main(");
    let first = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    assert!(
        first == "install_logging();" || first == "soos_push_sender::install_logging();",
        "main calls install_logging() first: {first}"
    );
    let manifest = read(SENDER_MANIFEST);
    let deps = toml_table(&manifest, "dependencies");
    for key in dependency_keys(&deps) {
        assert!(
            key != "log" && key != "tracing-log",
            "soos-push-sender must not depend on {key}"
        );
    }
    assert!(
        deps.contains(&"tracing-subscriber = { workspace = true }"),
        "tracing-subscriber exactly from the workspace, no added feature: {deps:?}"
    );
}
