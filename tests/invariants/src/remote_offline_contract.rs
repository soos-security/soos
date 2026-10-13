//! Static contracts of the offline history and the back-online push of `soos-remote`
//! (GitHub #376, tier 1 of #359 Part B; ADR 2026-10-13 "Offline History and Back-Online Push
//! Notifications in `soos-remote`"; architect spec `AI/architect_spec_remote_offline_alerts.md`
//! §7 and §10.6, tests 31–35; matrix ROA1, ROA2, ROA6, ROA9–ROA11):
//!
//! - ROA-S1 RC-1 kept: no `crates/remote/src` file names an `AF_INET`/`AF_INET6` socket or a
//!   network socket API; the user unit keeps `RestrictAddressFamilies=AF_UNIX`;
//! - ROA-S2 the bus is read-only: `offline.rs` calls only `Properties.Get` of four named
//!   properties on NetworkManager and logind, no remote source names a NetworkManager or
//!   logind mutating method, no signal match is ever installed (RMC-S9 unchanged);
//! - ROA-S3 privacy: no network identifier (SSID, BSSID, hardware address, IP configuration,
//!   access point, connection object, location) anywhere in the crate or its assets; the
//!   offline code never logs through a `tracing` macro; the notice handed to the payload
//!   builder holds only enums and integers;
//! - ROA-S4 the page card and the service worker tag;
//! - ROA-S5 documentation, ADR, matrix rows and walkthrough 202.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use crate::remote_companion_contract::{
    exists, macro_invocations, production_part, read, rust_files, strip_comments, TRACING_MACROS,
};

const OFFLINE_RS: &str = "crates/remote/src/offline.rs";
const PUSH_RS: &str = "crates/remote/src/push.rs";
const LOGIND_RS: &str = "crates/remote/src/logind.rs";
const LIB_RS: &str = "crates/remote/src/lib.rs";
const UNIT: &str = "packaging/soos-remote.service";
const APP_JS: &str = "crates/remote/assets/app.js";
const SW_JS: &str = "crates/remote/assets/sw.js";
const INDEX: &str = "crates/remote/assets/index.html";
const STYLE: &str = "crates/remote/assets/style.css";
const REMOTE_DOC: &str = "Docs/REMOTE_COMPANION.md";
const DECISIONS: &str = "AI/DECISIONS.md";
const MATRIX: &str = "AI/VERIFICATION_MATRIX.md";
const WALKTHROUGH: &str = "AI/walkthroughs/202_remote_offline_alerts.md";
const ADR_TITLE: &str = "Offline History and Back-Online Push Notifications in `soos-remote`";

/// Production code (comments stripped, trailing test module removed) of `rel`.
fn code_of(rel: &str) -> String {
    assert!(exists(rel), "{rel} must exist (spec §1.1)");
    strip_comments(production_part(&read(rel)))
}

/// Production code of every source of `crates/remote/src`.
fn remote_code() -> Vec<(String, String)> {
    let files: Vec<(String, String)> = rust_files("crates/remote/src")
        .into_iter()
        .map(|(rel, content)| (rel, strip_comments(production_part(&content))))
        .collect();
    assert!(files.len() > 20, "remote sources found: {}", files.len());
    files
}

/// Every double-quoted string literal of `code` (escapes kept verbatim, no raw strings).
fn string_literals(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' && i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
            i += 3;
            continue;
        }
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            out.push(code[start..j.min(code.len())].to_string());
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Test 31 — ROA-S1
// ---------------------------------------------------------------------------------------

/// Test 31 (ROA1, RC-1): the companion still opens no `AF_INET`/`AF_INET6` socket and the
/// unit still restricts it to Unix sockets.
#[test]
fn test_roa_s1_no_inet_socket_and_unit_unchanged() {
    for (rel, code) in remote_code() {
        for forbidden in [
            "AF_INET",
            "AddressFamily::Inet",
            "nix::sys::socket",
            "socket::socket(",
            "TcpStream",
            "TcpListener",
            "UdpSocket",
            "std::net",
            "tokio::net::Tcp",
            "tokio::net::Udp",
            "SocketAddr",
        ] {
            assert!(
                !code.contains(forbidden),
                "{rel}: `{forbidden}` is forbidden (RC-1: Unix sockets only)"
            );
        }
    }
    let unit = read(UNIT);
    let families: Vec<&str> = unit
        .lines()
        .filter(|l| l.starts_with("RestrictAddressFamilies="))
        .collect();
    assert_eq!(
        families,
        vec!["RestrictAddressFamilies=AF_UNIX"],
        "{UNIT} keeps exactly AF_UNIX"
    );
    let manifest = read("crates/remote/Cargo.toml");
    for forbidden in [
        "reqwest",
        "ureq",
        "hyper",
        "rustls",
        "native-tls",
        "socket2",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "crates/remote/Cargo.toml must not depend on {forbidden}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Test 32 — ROA-S2
// ---------------------------------------------------------------------------------------

/// Test 32 (ROA2, D-1, D-2): `offline.rs` reads four named properties through
/// `Properties.Get` only, on NetworkManager and logind only; no remote source names a
/// mutating NetworkManager or logind method or installs a match; the single connection
/// builder stays in `logind.rs`.
#[test]
fn test_roa_s2_bus_is_read_only() {
    let offline = code_of(OFFLINE_RS);
    let literals = string_literals(&offline);
    for required in [
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
        "org.freedesktop.DBus.Properties",
        "Get",
        "State",
        "Connectivity",
        "WirelessEnabled",
        "PreparingForShutdown",
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "NameHasOwner",
    ] {
        assert!(
            literals.iter().any(|l| l == required),
            "offline.rs names \"{required}\": {literals:?}"
        );
    }
    // Every bus member or name literal of offline.rs is one of the allowed ones.
    let allowed_bus = [
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
        "org.freedesktop.DBus.Properties",
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
    ];
    for literal in &literals {
        if literal.starts_with("org.freedesktop") || literal.starts_with("/org/freedesktop") {
            assert!(
                allowed_bus.contains(&literal.as_str()),
                "offline.rs names an unexpected bus object or interface \"{literal}\""
            );
        }
    }
    assert_eq!(
        offline.matches("call_method(").count(),
        2,
        "offline.rs has two bus call sites: Properties.Get and the bus driver's NameHasOwner"
    );
    // The only method literals of offline.rs.
    let methods: Vec<&String> = literals
        .iter()
        .filter(|l| {
            !l.is_empty()
                && l.chars().next().is_some_and(char::is_uppercase)
                && l.chars().all(char::is_alphanumeric)
        })
        .collect();
    for literal in methods {
        assert!(
            [
                "Get",
                "NameHasOwner",
                "State",
                "Connectivity",
                "WirelessEnabled",
                "PreparingForShutdown"
            ]
            .contains(&literal.as_str()),
            "offline.rs names an unexpected member \"{literal}\""
        );
    }
    for forbidden in [
        "Builder::address(",
        "add_match",
        "AddMatch",
        "MatchRule",
        "receive_",
        "Stream",
        "\"Set\"",
        "\"GetAll\"",
        "call_noreply",
        "emit_signal",
    ] {
        assert!(
            !offline.contains(forbidden),
            "offline.rs must not contain `{forbidden}`"
        );
    }
    let mutating = [
        "\"Enable\"",
        "\"Sleep\"",
        "\"ActivateConnection\"",
        "\"AddAndActivateConnection\"",
        "\"AddAndActivateConnection2\"",
        "\"DeactivateConnection\"",
        "\"Reload\"",
        "\"SetLogging\"",
        "\"CheckpointCreate\"",
        "\"CheckpointRollback\"",
        "\"Update\"",
        "\"Delete\"",
        "\"Set\"",
        "\"RequestScan\"",
        "\"Disconnect\"",
        "\"Inhibit\"",
        "\"AddMatch\"",
        "\"Suspend\"",
        "\"PowerOff\"",
        "\"Reboot\"",
        "\"Hibernate\"",
        "\"SetWirelessEnabled\"",
    ];
    for (rel, code) in remote_code() {
        for forbidden in mutating {
            assert!(
                !code.contains(forbidden),
                "{rel}: the D-Bus method literal {forbidden} is forbidden (ROA-S2)"
            );
        }
        if !rel.ends_with("/offline.rs") {
            assert!(
                !code.contains("org.freedesktop.NetworkManager"),
                "{rel}: only offline.rs talks to NetworkManager"
            );
        }
    }
    // The NameHasOwner gate precedes every NetworkManager read (R2-2).
    let gate = offline
        .find("\"NameHasOwner\"")
        .expect("NameHasOwner literal");
    let allowed = offline
        .find("nm_reads_allowed(")
        .expect("nm_reads_allowed call");
    let first_state = offline.find("\"State\"").expect("State literal");
    assert!(
        gate < first_state && allowed < first_state,
        "offline.rs checks NameHasOwner before reading NetworkManager properties"
    );
    let logind = code_of(LOGIND_RS);
    assert!(
        logind.contains("pub(crate) async fn connect_system_bus("),
        "logind.rs exposes the single pinned connection builder"
    );
    assert!(
        offline.contains("connect_system_bus("),
        "offline.rs connects through logind::connect_system_bus"
    );
    assert_eq!(
        remote_code()
            .iter()
            .map(|(_, c)| c.matches("Builder::address(").count())
            .sum::<usize>(),
        1,
        "exactly one zbus connection builder in the crate (RMC-S9)"
    );
}

// ---------------------------------------------------------------------------------------
// Test 33 — ROA-S3
// ---------------------------------------------------------------------------------------

/// The body of `pub struct <name> {` in `code`.
fn struct_body(code: &str, name: &str) -> String {
    let at = code
        .find(&format!("pub struct {name} {{"))
        .unwrap_or_else(|| panic!("pub struct {name} {{ not found"));
    let open = at + code[at..].find('{').unwrap();
    let close = open + code[open..].find('}').unwrap();
    code[open + 1..close].to_string()
}

/// Test 33 (ROA6, ROA10, ROA-S3): no network identifier anywhere; the offline code and the
/// back-online push path never log through a `tracing` macro; the notice holds only enums
/// and integers and is `Copy`; the payload builder takes the notice, the previews setting and
/// the configured `rp_id` only.
#[test]
fn test_roa_s3_no_network_identifiers_and_notice_fields() {
    let identifiers = [
        "ssid",
        "bssid",
        "hwaddress",
        "permhwaddress",
        "ip4config",
        "ip6config",
        "ipaddress",
        "accesspoint",
        "activeconnection",
        "primaryconnection",
        "latitude",
        "longitude",
        "macaddress",
    ];
    for (rel, code) in remote_code() {
        let lower = code.to_ascii_lowercase();
        for needle in identifiers {
            assert!(!lower.contains(needle), "{rel} names `{needle}` (ROA-S3)");
        }
    }
    for rel in [APP_JS, SW_JS, INDEX, STYLE] {
        let lower = read(rel).to_ascii_lowercase();
        for needle in identifiers {
            assert!(!lower.contains(needle), "{rel} names `{needle}` (ROA-S3)");
        }
    }
    let offline = code_of(OFFLINE_RS);
    let lower = offline.to_ascii_lowercase();
    for needle in ["hostname", "geolocation", "address"] {
        assert!(!lower.contains(needle), "offline.rs names `{needle}`");
    }
    assert_eq!(
        macro_invocations(&offline, &TRACING_MACROS),
        Vec::<String>::new(),
        "offline.rs never logs through a tracing macro"
    );
    assert!(
        !offline.contains("tracing::"),
        "offline.rs never uses tracing directly"
    );
    for macro_name in ["println!", "eprintln!", "print!", "eprint!", "dbg!"] {
        assert!(!offline.contains(macro_name), "offline.rs: {macro_name}");
    }

    // The notice: enums and integers only, `Copy`.
    let fields = struct_body(&offline, "OfflineNotice");
    let mut count = 0;
    for line in fields
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("pub "))
    {
        count += 1;
        let ty = line.split(':').nth(1).unwrap().trim().trim_end_matches(',');
        assert!(
            ["OfflineReason", "u32", "u64"].contains(&ty),
            "OfflineNotice field `{line}` must be an enum or an integer"
        );
    }
    assert_eq!(count, 5, "OfflineNotice has five fields: {fields}");
    let derive_at = offline.find("pub struct OfflineNotice {").unwrap();
    let derive = &offline[offline[..derive_at].rfind("#[derive(").unwrap()..derive_at];
    assert!(derive.contains("Copy"), "OfflineNotice derives Copy");
    for name in ["OfflineInterval", "NmReading", "ClockSample"] {
        let body = struct_body(&offline, name);
        assert!(
            !body.contains("String") && !body.contains("str") && !body.contains("Vec"),
            "{name} holds no text: {body}"
        );
    }

    // The payload builder signature and the push path stay silent.
    let push = code_of(PUSH_RS);
    assert!(
        push.contains(
            "pub fn back_online_payload(\n    notice: &OfflineNotice,\n    previews: PushPreviews,\n    rp_id: &str,\n)"
        ) || push.contains(
            "pub fn back_online_payload(notice: &OfflineNotice, previews: PushPreviews, rp_id: &str)"
        ),
        "back_online_payload takes only the notice, the previews setting and rp_id"
    );
    assert_eq!(
        macro_invocations(&push, &TRACING_MACROS),
        Vec::<String>::new(),
        "push.rs never logs through a tracing macro"
    );
    let lib = code_of(LIB_RS);
    assert!(lib.contains("pub const PUSH_OFFLINE_TOPIC: &str = \"soosonline\";"));
    assert!(lib.contains("pub const NETWORK_STORE_FILE_NAME: &str = \"remote-network.json\";"));
    assert!(lib.contains("pub const MAX_NETWORK_STORE_BYTES: usize = 8192;"));
    assert!(lib.contains("pub const MAX_OFFLINE_INTERVALS: usize = 32;"));
    // The fixed audit lines.
    let audit = code_of("crates/remote/src/audit.rs");
    for message in [
        "\"offline alerts active\"",
        "\"offline alerts unavailable\"",
        "\"offline history not persisted\"",
    ] {
        assert!(audit.contains(message), "audit.rs declares {message}");
    }
}

// ---------------------------------------------------------------------------------------
// Test 34 — ROA-S4
// ---------------------------------------------------------------------------------------

/// Test 34 (ROA9, ROA-S4): the page's "Offline history" card is built at run time (no markup
/// id, RMC-S50) and filled with fixed strings through `setText`, fetched from `/api/offline`; the service worker shows a
/// back-online notice under its own tag.
#[test]
fn test_roa_s4_page_card_and_service_worker() {
    // The page id set is pinned by RMC-S50: the card is built at run time, with no markup id.
    let index = read(INDEX);
    assert!(
        !index.contains("offline"),
        "index.html is unchanged (no offline markup)"
    );
    let app = read(APP_JS);
    for needle in [
        "const OFFLINE_PATH = \"/api/offline\";",
        "const OFFLINE_HISTORY_SHOWN = 10;",
        "function fetchOffline()",
        "function renderOffline(view)",
        "network_lost: \"Network lost\"",
        "wifi_disabled: \"Wi-Fi turned off\"",
        "sleep: \"Sleep\"",
        "shutdown_or_crash: \"Shutdown or crash\"",
        "No offline period recorded",
        "offlineSection.className = \"card offline\";",
        "offlineSection.hidden = true;",
        "setText(offlineTitle, \"Offline history\");",
        "pushSection.parentNode.insertBefore(offlineSection, pushSection);",
        "Offline alerts unavailable: the history file on the PC was refused",
        "Network changes are not monitored on this PC (NetworkManager not found); sleep and shutdown are",
    ] {
        assert!(app.contains(needle), "app.js must contain {needle}");
    }
    let render_at = app.find("function renderOffline(view)").unwrap();
    let render_end = render_at + app[render_at..].find("\n}\n").unwrap();
    let render = &app[render_at..render_end];
    assert!(
        render.contains("setText("),
        "renderOffline writes through setText"
    );
    for forbidden in [
        "innerHTML",
        "insertAdjacentHTML",
        "outerHTML",
        ".textContent =",
    ] {
        assert!(!render.contains(forbidden), "renderOffline: {forbidden}");
    }
    let fetch_status_at = app.find("function fetchStatus(").unwrap();
    let fetch_status_end = fetch_status_at + app[fetch_status_at..].find("\n}\n").unwrap();
    assert!(
        app[fetch_status_at..fetch_status_end].contains("fetchOffline()"),
        "the history is refreshed with the status"
    );
    assert!(
        render.contains("signedOut"),
        "renderOffline returns early when signed out"
    );
    let login_at = app.find("function showLogin(").unwrap();
    let login_end = login_at + app[login_at..].find("\n}\n").unwrap();
    let login = &app[login_at..login_end];
    assert!(
        login.contains("offlineSection.hidden = true") && login.contains("offlineHistory"),
        "showLogin hides the offline card and clears its list"
    );
    let sw = read(SW_JS);
    assert!(
        sw.contains("\"back_online\""),
        "sw.js knows the notice kind"
    );
    assert!(
        sw.contains("\"soos-online\""),
        "sw.js shows it under its own tag"
    );
    let css = read(STYLE);
    assert!(
        css.contains(".offline-history"),
        "style.css styles the list"
    );
}

// ---------------------------------------------------------------------------------------
// Test 35 — ROA-S5
// ---------------------------------------------------------------------------------------

/// Test 35 (ROA11, ROA-S5): documentation, ADR, matrix rows and walkthrough.
#[test]
fn test_roa_s5_docs_adr_matrix_walkthrough() {
    let doc = read(REMOTE_DOC);
    let start = doc
        .find("\n## 2j. Offline history and back-online notifications")
        .expect("## 2j. Offline history and back-online notifications");
    let three = doc.find("\n## 3. ").expect("## 3.");
    assert!(start < three, "## 2j. precedes ## 3.");
    let section = &doc[start..three];
    for needle in [
        "offline_alerts = true",
        "push_notifications",
        "remote-network.json",
        "network_lost",
        "wifi_disabled",
        "sleep",
        "shutdown_or_crash",
        "PreparingForShutdown",
        "CLOCK_BOOTTIME",
        "soosonline",
        "push_previews = \"generic\"",
        "SSID",
        "GET /api/offline",
        "ROA12",
        "ROA13",
        "ROA14",
    ] {
        assert!(section.contains(needle), "§2j must mention {needle}");
    }
    let config_at = doc.find("\n## 5. ").expect("## 5.");
    let http_at = doc.find("\n## 6. ").expect("## 6.");
    assert!(
        doc[config_at..http_at].contains("| `offline_alerts` |"),
        "§5 lists the offline_alerts key"
    );
    let seven = doc.find("\n## 7. ").expect("## 7.");
    assert!(
        doc[http_at..seven].contains("`GET /api/offline`"),
        "§6 lists the route"
    );
    let eight = doc.find("\n## 8. ").expect("## 8.");
    let nine = doc.find("\n## 9. ").expect("## 9.");
    assert!(
        doc[eight..nine].contains("section 2j"),
        "§8 lists the offline limitations"
    );
    assert!(
        doc[nine..].contains("section 2j"),
        "§9 points to section 2j"
    );
    let decisions = read(DECISIONS);
    let adr_at = decisions.find(ADR_TITLE).expect("the ADR is registered");
    let adr_line = decisions[adr_at..].lines().next().unwrap();
    for needle in [
        "item (1)",
        "2026-10-06",
        "#376",
        "#359",
        "exit 78",
        "RMC-S9",
    ] {
        assert!(adr_line.contains(needle), "the ADR mentions {needle}");
    }
    let matrix = read(MATRIX);
    for row in 1..=14 {
        assert!(
            matrix.contains(&format!("| ROA{row} |")),
            "matrix row ROA{row}"
        );
    }
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH} exists");
    let walkthrough = read(WALKTHROUGH);
    assert!(walkthrough.contains("#376"), "the walkthrough names #376");
    let installer = read("scripts/install_remote.sh");
    assert!(
        installer.contains("# offline_alerts = false"),
        "the installer template documents the key"
    );
}
