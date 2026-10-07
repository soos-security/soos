//! Static contracts of the live battery level of `soos-remote` (GitHub #346, ADR 2026-10-07
//! "Live Battery Level in `soos-remote`", architect spec `AI/architect_spec_remote_battery.md`
//! §10.5 tests 32–37 and §10.6 test 39; matrix RBS6, RBS7, RBS11):
//!
//! - RBS-S1 `battery.rs` never logs, never writes, opens with `O_NOFOLLOW | O_NONBLOCK`;
//! - RBS-S2 only the ten allowlisted attributes are read; no identifying attribute name
//!   appears in the production code of the crate (comments excluded, plan evaluation R3-2);
//! - RBS-S3 `"/sys` appears once in the crate (`POWER_SUPPLY_ROOT`, the needle keeps its
//!   opening quote, plan evaluation R3-1); `main.rs` builds the source with
//!   `SysfsBattery::kernel()` only;
//! - RBS-S4 the page builds the battery line at run time, fed only by `event: battery`;
//! - RBS-S5 the feature is documented and the installer template carries the key;
//! - RBS-S6 no new dependency;
//! - RBS-S7 the service runtime shutdown is bounded.

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

const BATTERY_RS: &str = "crates/remote/src/battery.rs";
const LIB_RS: &str = "crates/remote/src/lib.rs";
const MAIN_RS: &str = "crates/remote/src/main.rs";
const APP_JS: &str = "crates/remote/assets/app.js";

/// The ten attributes `read_power_supplies` may open (spec §5.4).
const ATTRIBUTE_ALLOWLIST: [&str; 10] = [
    "type",
    "scope",
    "present",
    "capacity",
    "status",
    "energy_now",
    "energy_full",
    "charge_now",
    "charge_full",
    "online",
];

/// Production code (comments stripped, trailing test module removed) of `rel`.
fn code_of(rel: &str) -> String {
    assert!(exists(rel), "{rel} must exist (spec §1.1)");
    strip_comments(production_part(&read(rel)))
}

/// Production code of every source of `crates/remote/src`.
fn remote_code() -> Vec<(String, String)> {
    let files: Vec<(String, String)> = rust_files("crates/remote/src")
        .into_iter()
        .map(|(rel, content)| {
            let code = strip_comments(production_part(&content));
            (rel, code)
        })
        .collect();
    assert!(
        files.iter().any(|(rel, _)| rel == BATTERY_RS),
        "{BATTERY_RS} must exist (spec §1.1)"
    );
    files
}

/// The body (between the first `{` after `signature` and its matching `}`).
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

/// The string literals (`"…"`, escapes kept) of `code`.
fn string_literals(code: &str) -> Vec<String> {
    let bytes = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            out.push(code[start..i.min(bytes.len())].to_string());
        }
        i += 1;
    }
    out
}

/// The argument lists of every `name(` call in `code` (balanced parentheses).
fn call_arguments(code: &str, name: &str) -> Vec<String> {
    let needle = format!("{name}(");
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(pos) = code[start..].find(&needle) {
        let at = start + pos;
        let preceded_by_ident = at > 0 && {
            let b = code.as_bytes()[at - 1];
            b.is_ascii_alphanumeric() || b == b'_'
        };
        let open = at + needle.len();
        let mut depth = 1usize;
        let mut end = open;
        for (i, c) in code[open..].char_indices() {
            match c {
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
    out
}

/// The lines of the Markdown section whose heading line contains `title`, until the next
/// heading of the same or a higher level.
fn markdown_section(doc: &str, title: &str) -> String {
    let lines: Vec<&str> = doc.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with('#') && l.contains(title))
        .unwrap_or_else(|| panic!("no heading containing {title:?}"));
    let level = lines[start].chars().take_while(|c| *c == '#').count();
    let mut out = String::new();
    for line in &lines[start + 1..] {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if hashes > 0 && hashes <= level && line[hashes..].starts_with(' ') {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------------------
// Test 32 — RBS-S1
// ---------------------------------------------------------------------------------------

/// Test 32 (RBS-S1, RBS6, RBS7, RBS-I3, RBS-I5, RBS-I6): `battery.rs` has no logging macro,
/// no write or permission change on any path, opens attributes with `O_NOFOLLOW` and
/// `O_NONBLOCK`, and no `unsafe`.
#[test]
fn test_rbs_s1_battery_module_never_logs_and_never_writes() {
    let code = code_of(BATTERY_RS);
    let calls = macro_invocations(&code, &TRACING_MACROS);
    assert!(calls.is_empty(), "battery.rs must not log: {calls:?}");
    for forbidden in [
        "tracing::",
        "use tracing",
        "log::",
        "println!",
        "eprintln!",
        "print!",
        "dbg!",
        "File::create",
        "OpenOptions::new().write",
        ".write(true)",
        ".append(true)",
        ".create(true)",
        ".truncate(true)",
        "fs::write",
        "set_permissions",
        "remove_file",
        "remove_dir",
        "create_dir",
        "rename(",
        "unsafe",
        "SystemTime",
    ] {
        assert!(
            !code.contains(forbidden),
            "battery.rs must not contain {forbidden:?}"
        );
    }
    for required in ["O_NOFOLLOW", "O_NONBLOCK", "custom_flags"] {
        assert!(
            code.contains(required),
            "battery.rs opens attributes with {required} (spec §5.4)"
        );
    }
    let lib = code_of(LIB_RS);
    assert!(
        lib.contains("#![forbid(unsafe_code)]"),
        "the crate forbids unsafe"
    );
    assert!(
        lib.contains("pub mod battery;"),
        "lib.rs declares the battery module"
    );
}

// ---------------------------------------------------------------------------------------
// Test 33 — RBS-S2
// ---------------------------------------------------------------------------------------

/// Test 33 (RBS-S2, RBS7, RBS-I2): every attribute string literal passed to `read_attr` is in
/// the §5.4 allowlist; `serial_number`, `model_name`, `manufacturer` and `uevent` never
/// appear in the production code of `crates/remote/src` (comments excluded: the module doc
/// may say what is never read; plan evaluation R3-2).
#[test]
fn test_rbs_s2_attribute_allowlist_and_no_identifiers() {
    let code = code_of(BATTERY_RS);
    assert!(
        code.contains("fn read_attr("),
        "battery.rs reads every attribute through read_attr (spec §5.4)"
    );
    let mut seen = Vec::new();
    for args in call_arguments(&code, "read_attr") {
        if args.contains("dir: &") || args.contains(": &Path") {
            continue; // the definition
        }
        for literal in string_literals(&args) {
            assert!(
                ATTRIBUTE_ALLOWLIST.contains(&literal.as_str()),
                "read_attr opens {literal:?}, which is outside the allowlist"
            );
            seen.push(literal);
        }
    }
    for required in ["type", "capacity", "status", "online"] {
        assert!(
            seen.iter().any(|s| s == required),
            "read_attr is called with the literal {required:?}: {seen:?}"
        );
    }
    for (rel, code) in remote_code() {
        for forbidden in ["serial_number", "model_name", "manufacturer", "uevent"] {
            assert!(
                !code.contains(forbidden),
                "{rel}: `{forbidden}` must never appear in production code (RBS-I2)"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// Test 34 — RBS-S3
// ---------------------------------------------------------------------------------------

/// Test 34 (RBS-S3, RBS11, F-9 d, R2-1, R3-1): over the production code of every file of
/// `crates/remote/src`, `"/sys` (opening double quote included: a bare `/sys` also matches
/// `SYSTEM_BUS_ADDRESS`) appears exactly once, in `lib.rs` (`POWER_SUPPLY_ROOT`); `main.rs`
/// builds the source with `SysfsBattery::kernel()` and never with `SysfsBattery::new(`; no
/// file names `SysfsBattery::system`.
#[test]
fn test_rbs_s3_sysfs_root_single_source() {
    let files = remote_code();
    let hits: Vec<(String, usize)> = files
        .iter()
        .map(|(rel, code)| (rel.clone(), code.matches("\"/sys").count()))
        .filter(|(_, n)| *n > 0)
        .collect();
    assert_eq!(
        hits,
        vec![(LIB_RS.to_string(), 1)],
        "\"/sys appears once in the crate, in lib.rs"
    );
    let lib = code_of(LIB_RS);
    assert!(
        lib.contains("pub const POWER_SUPPLY_ROOT: &str = \"/sys/class/power_supply\";"),
        "lib.rs pins POWER_SUPPLY_ROOT"
    );
    let main = code_of(MAIN_RS);
    assert!(
        main.contains("SysfsBattery::kernel()"),
        "main.rs builds the production source with SysfsBattery::kernel()"
    );
    assert!(
        main.contains("with_battery("),
        "main.rs wires the source through ServerState::with_battery"
    );
    assert!(
        main.contains("config.battery.enabled"),
        "main.rs wires the source only when battery_status is on"
    );
    assert!(
        !main.contains("SysfsBattery::new("),
        "main.rs never builds a source over another root"
    );
    for (rel, code) in &files {
        assert!(
            !code.contains("SysfsBattery::system"),
            "{rel}: the constructor is `kernel`, never `system` (RMC-S9)"
        );
    }
    let battery = code_of(BATTERY_RS);
    assert!(
        battery.contains("POWER_SUPPLY_ROOT"),
        "SysfsBattery::kernel() uses POWER_SUPPLY_ROOT"
    );
}

// ---------------------------------------------------------------------------------------
// Test 35 — RBS-S4
// ---------------------------------------------------------------------------------------

/// Test 35 (RBS-S4, RBS11, B-7, B-11, F-2, F-8): the page builds one battery line at run
/// time, feeds it only from `event: battery`, clears it when the stream closes or errors,
/// never fetches `/api/battery`, and never writes "on battery, plugged in".
#[test]
fn test_rbs_s4_page_battery_line() {
    let js = read(APP_JS);
    for required in [
        "addEventListener(\"battery\"",
        "createElement(\"p\")",
        "\"detail battery\"",
        "insertBefore(",
        "function clearBattery",
        "function renderBattery",
        "function batteryText",
        "suffix === \"\" && view.external_power === true",
    ] {
        assert!(js.contains(required), "app.js must contain {required:?}");
    }
    assert!(
        js.contains("className = \"detail battery\""),
        "the line carries the class \"detail battery\""
    );
    let onerror = body_of(&js, "source.onerror = function");
    assert!(
        onerror.contains("clearBattery()"),
        "source.onerror clears the battery line: {onerror}"
    );
    let close = body_of(&js, "function closeStream");
    assert!(
        close.contains("clearBattery()"),
        "closeStream clears the battery line: {close}"
    );
    let login = body_of(&js, "function showLogin");
    assert!(
        login.contains("clearBattery()"),
        "showLogin clears the battery line: {login}"
    );
    let render = body_of(&js, "function render(");
    assert!(
        render.contains("showBattery("),
        "render applies the reachability rule to the battery line: {render}"
    );
    for forbidden in [
        "/api/battery",
        "BATTERY_PATH",
        "fetchBattery",
        "getElementById(\"battery",
    ] {
        assert!(
            !js.contains(forbidden),
            "app.js must not contain {forbidden:?}"
        );
    }
    assert_eq!(
        js.matches("\", plugged in\"").count(),
        1,
        "the plugged-in fallback exists once"
    );
    assert_eq!(js.matches("\", on battery\"").count(), 1);
    let battery_code = body_of(&js, "function batteryText");
    assert!(
        !battery_code.contains("innerHTML"),
        "the battery line is written with textContent only"
    );
    assert!(
        !js.contains("on battery, plugged in"),
        "never the contradictory text"
    );
    let html = read("crates/remote/assets/index.html");
    assert!(
        !html.to_lowercase().contains("battery"),
        "index.html is unchanged"
    );
    let sw = read("crates/remote/assets/sw.js");
    assert!(!sw.to_lowercase().contains("battery"), "sw.js is unchanged");
}

// ---------------------------------------------------------------------------------------
// Test 36 — RBS-S5
// ---------------------------------------------------------------------------------------

/// Test 36 (RBS-S5, RBS11, B-14): the operator documentation, the architecture, the mock
/// strategy and the installer template describe the feature.
#[test]
fn test_rbs_s5_documented() {
    let doc = read("Docs/REMOTE_COMPANION.md");
    let section = markdown_section(&doc, "Battery level");
    for required in [
        "battery_status",
        "/sys/class/power_supply",
        "GET /api/battery",
        "event: battery",
        "5 s",
        "login_required",
    ] {
        assert!(
            section.contains(required),
            "the Battery level section mentions {required:?}"
        );
    }
    let configuration = markdown_section(&doc, "## 5. Configuration");
    assert!(
        configuration
            .lines()
            .any(|l| l.starts_with("| `battery_status` |")
                && (l.contains("| `true` |") || l.contains("| true |"))),
        "§5 has a battery_status row with default `true`"
    );
    let http = markdown_section(&doc, "## 6. HTTP surface");
    assert!(
        http.lines()
            .any(|l| l.starts_with('|') && l.contains("GET /api/battery")),
        "§6 has a GET /api/battery row"
    );
    let architecture = read("AI/ARCHITECTURE.md");
    let thirteen = markdown_section(&architecture, "## 13.");
    assert!(
        thirteen.to_lowercase().contains("battery"),
        "AI/ARCHITECTURE.md §13 mentions the battery level"
    );
    let mocks = read("AI/MOCK_STRATEGY.md");
    let doubles = markdown_section(&mocks, "Remote Companion Doubles");
    assert!(
        doubles.to_lowercase().contains("battery"),
        "AI/MOCK_STRATEGY.md Remote Companion Doubles mention the battery doubles"
    );
    let installer = read("scripts/install_remote.sh");
    assert!(
        installer
            .lines()
            .any(|l| l.trim() == "# battery_status = true"),
        "the remote.toml template carries `# battery_status = true`"
    );
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("Live Battery Level in `soos-remote`"),
        "the ADR exists"
    );
}

// ---------------------------------------------------------------------------------------
// Test 37 — RBS-S6
// ---------------------------------------------------------------------------------------

/// Test 37 (RBS-S6, RBS11, F-9 a): the `[dependencies]` key set of `crates/remote` equals the
/// set of the `main` that already contains #345 (`soos-protocol`, `jpeg-encoder`): the
/// battery level adds no dependency.
#[test]
fn test_rbs_s6_no_new_dependency() {
    let manifest = read("crates/remote/Cargo.toml");
    let mut keys: Vec<String> = toml_table(&manifest, "dependencies")
        .iter()
        .filter_map(|line| line.split('=').next())
        .map(|key| key.trim().to_string())
        .collect();
    keys.sort();
    let mut expected = vec![
        "tokio",
        "nix",
        "tracing",
        "tracing-subscriber",
        "thiserror",
        "serde",
        "serde_json",
        "toml",
        "clap",
        "httparse",
        "zbus",
        "zeroize",
        "p256",
        "sha2",
        "ciborium",
        "getrandom",
        "base64ct",
        "subtle",
        "hmac",
        "aes-gcm",
        "soos-push-protocol",
        "soos-protocol",
        "jpeg-encoder",
    ];
    expected.sort_unstable();
    assert_eq!(
        keys, expected,
        "crates/remote [dependencies] must equal the post-#345 set (no new dependency)"
    );
    let dev: Vec<&str> = toml_table(&manifest, "dev-dependencies");
    for line in &dev {
        let key = line.split('=').next().unwrap_or("").trim();
        assert!(
            ["tokio", "tempfile", "proptest"].contains(&key),
            "no new dev-dependency: {line}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Test 39 — RBS-S7
// ---------------------------------------------------------------------------------------

/// Test 39 (RBS-S7, RBS6, B-13, F-7, R2-5): `run_service` bounds the runtime shutdown with
/// `shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT_MS)` after `block_on`, and `lib.rs` defines the
/// constant once (1000 ms, above the read bound).
#[test]
fn test_rbs_s7_runtime_shutdown_is_bounded() {
    let main = code_of(MAIN_RS);
    let body = body_of(&main, "fn run_service(");
    let block_on = body
        .find("block_on(")
        .expect("run_service runs the service with block_on");
    let shutdown = body
        .find("shutdown_timeout(")
        .expect("run_service bounds the runtime shutdown (B-13)");
    assert!(
        block_on < shutdown,
        "shutdown_timeout( comes after block_on( (B-13 shape)"
    );
    assert!(
        body.contains("RUNTIME_SHUTDOWN_TIMEOUT_MS"),
        "the bound is RUNTIME_SHUTDOWN_TIMEOUT_MS"
    );
    assert!(
        main.contains("new_current_thread()") && main.contains("EXIT_RUNTIME"),
        "the RMC-S10 needles are kept"
    );
    let lib = code_of(LIB_RS);
    assert_eq!(
        lib.matches("pub const RUNTIME_SHUTDOWN_TIMEOUT_MS: u64 = 1000;")
            .count(),
        1,
        "lib.rs defines RUNTIME_SHUTDOWN_TIMEOUT_MS = 1000 once"
    );
    for (constant, value) in [
        ("POWER_SUPPLY_ROOT: &str", "\"/sys/class/power_supply\""),
        ("MAX_POWER_SUPPLIES: usize", "64"),
        ("MAX_BATTERIES: usize", "8"),
        ("MAX_POWER_SUPPLY_NAME_LEN: usize", "64"),
        ("MAX_SYSFS_VALUE_BYTES: usize", "32"),
        ("MAX_SYSFS_MICRO_DIGITS: usize", "19"),
        ("BATTERY_READ_TIMEOUT_MS: u64", "500"),
        ("BATTERY_SAMPLE_INTERVAL_MS: u64", "5000"),
    ] {
        let line = format!("pub const {constant} = {value};");
        assert_eq!(
            lib.matches(&line).count(),
            1,
            "lib.rs defines `{line}` once"
        );
    }
    for assertion in [
        "assert!(BATTERY_READ_TIMEOUT_MS < BATTERY_SAMPLE_INTERVAL_MS)",
        "assert!(BATTERY_SAMPLE_INTERVAL_MS < SSE_KEEPALIVE_MS)",
        "assert!(BATTERY_READ_TIMEOUT_MS < RUNTIME_SHUTDOWN_TIMEOUT_MS)",
    ] {
        assert!(
            lib.contains(assertion),
            "lib.rs keeps the compile-time `{assertion}`"
        );
    }
    for (rel, code) in rust_files("crates/remote/src") {
        if rel == LIB_RS {
            continue;
        }
        let code = strip_comments(production_part(&code));
        assert!(
            !code.contains("const RUNTIME_SHUTDOWN_TIMEOUT_MS"),
            "{rel}: lib.rs is the single source of the constant"
        );
    }
}
