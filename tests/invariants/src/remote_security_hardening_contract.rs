//! Static contracts of the soos-remote security hardening (security review of 2026-10-12, ADR
//! "[2026-10-12] soos-remote Security Hardening"; architect spec
//! `AI/architect_spec_remote_security_hardening.md` §11.1; matrix RSH1, RSH3, RSH5–RSH7,
//! RSH9, RSH10, RSH12–RSH17):
//!
//! - S-1 the push sender unit hides the system logs;
//! - S-2 the verified Tailscale minimum version is single-sourced, documented and printed;
//! - S-3 the auth-reserved daemon connection and the companion's graceful retry close;
//! - S-4 Android lock-screen privacy is documented;
//! - S-5 the preview placement rule is wired after the seat check;
//! - S-6 the hardening response headers, the CSP unchanged;
//! - S-7 the sender checks its sandbox first and reads no other environment variable;
//! - S-8a … S-8d installer symlink refusal, stop re-checks, renewal endpoint, remote
//!   conversion off the `Auth` path;
//! - the ADR, the architecture, the IPC decision table and walkthrough 194.

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
    exists, production_part, read, rust_files, strip_comments, workspace_root,
};

const SENDER_UNIT: &str = "packaging/soos-push-sender.service";
const REMOTE_DOC: &str = "Docs/REMOTE_COMPANION.md";
const INSTALLER: &str = "scripts/install_remote.sh";
const REMOTE_LIB: &str = "crates/remote/src/lib.rs";
const REMOTE_HTTP: &str = "crates/remote/src/http.rs";
const REMOTE_SERVER: &str = "crates/remote/src/server.rs";
const REMOTE_CAMERA_IPC: &str = "crates/remote/src/camera_ipc.rs";
const REMOTE_SW: &str = "crates/remote/assets/sw.js";
const DAEMON_LIMITS: &str = "crates/daemon/src/limits.rs";
const DAEMON_CONFIG: &str = "crates/daemon/src/config.rs";
const DAEMON_DISPATCHER: &str = "crates/daemon/src/dispatcher.rs";
const DAEMON_SESSION_POLICY: &str = "crates/daemon/src/session_policy.rs";
const DAEMON_PREVIEW_IMAGE: &str = "crates/daemon/src/preview_image.rs";
const SENDER_LIB: &str = "crates/push-sender/src/lib.rs";
const SENDER_MAIN: &str = "crates/push-sender/src/main.rs";
const SENDER_SRC: &str = "crates/push-sender/src";

const INACCESSIBLE_LINE: &str = "InaccessiblePaths=-/var/log -/var/lib/systemd/coredump";
const TAILSCALE_MIN: &str = "1.102.4";
const ADR_TITLE: &str = "[2026-10-12] soos-remote Security Hardening";
const WALKTHROUGH: &str = "AI/walkthroughs/194_remote_security_hardening.md";

/// `MANDATORY_HEADERS` of `crates/remote/src/http.rs`, byte for byte (RLC-S6, unchanged).
const PINNED_MANDATORY_HEADERS: &str =
    "const MANDATORY_HEADERS: &str = \"Cache-Control: no-store\\r\\n\\
    Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; \\
    img-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; \\
    form-action 'none'; frame-ancestors 'none'\\r\\n\\
    X-Content-Type-Options: nosniff\\r\\n\\
    Referrer-Policy: no-referrer\\r\\n\\
    X-Frame-Options: DENY\\r\\n\\
    Connection: close\\r\\n\";";

/// `HARDENING_HEADERS` of spec §7, byte for byte.
const PINNED_HARDENING_HEADERS: &str =
    "const HARDENING_HEADERS: &str = \"Strict-Transport-Security: max-age=31536000\\r\\n\\
    Cross-Origin-Opener-Policy: same-origin\\r\\n\\
    Cross-Origin-Resource-Policy: same-origin\\r\\n\\
    Permissions-Policy: accelerometer=(), camera=(), display-capture=(), geolocation=(), \\
    gyroscope=(), magnetometer=(), microphone=(), midi=(), payment=(), usb=()\\r\\n\";";

// ---------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------

/// Production code (comments stripped, trailing test module removed) of `rel`.
fn code_of(rel: &str) -> String {
    assert!(exists(rel), "{rel} must exist");
    strip_comments(production_part(&read(rel)))
}

/// Byte offset of the `}` closing the balanced block opened at `open` (a `{`).
fn block_end(code: &str, open: usize) -> usize {
    assert_eq!(&code[open..=open], "{", "block_end needs an opening brace");
    let mut depth = 0usize;
    for (i, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return open + i;
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced block at byte {open}");
}

/// The body (between the first `{` after `signature` and its matching `}`) of the first item
/// whose text contains `signature`.
fn body_of(code: &str, signature: &str) -> String {
    let at = code
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} not found"));
    let open = at + code[at..].find('{').expect("a body");
    let end = block_end(code, open);
    code[open + 1..end].to_string()
}

/// The `## <number> ` section of a Markdown document (up to the next `## `).
fn section<'a>(doc: &'a str, number: &str) -> &'a str {
    let heading = format!("\n## {number} ");
    let at = doc
        .find(&heading)
        .unwrap_or_else(|| panic!("section ## {number} not found"));
    let rest = &doc[at + 1..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |e| e + 3);
    &rest[..end]
}

/// Position of `needle` in `hay`, panicking with `what` when absent.
fn pos(hay: &str, needle: &str, what: &str) -> usize {
    hay.find(needle)
        .unwrap_or_else(|| panic!("{what}: `{needle}` not found"))
}

/// The non-empty, non-comment, trimmed lines of a unit file.
fn unit_lines(unit: &str) -> Vec<&str> {
    unit.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with(';'))
        .collect()
}

/// The decimal value of `pub const <name>: <ty> = <value>;` in `code` (underscores allowed).
fn const_integer(code: &str, name: &str, ty: &str) -> u64 {
    let prefix = format!("pub const {name}: {ty} = ");
    let at = pos(code, &prefix, name) + prefix.len();
    let end = at + code[at..].find(';').expect("const terminator");
    code[at..end]
        .trim()
        .replace('_', "")
        .parse::<u64>()
        .unwrap_or_else(|e| panic!("{name} is not a decimal literal: {e}"))
}

/// `code` with every whitespace character removed.
fn compact(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

// ---------------------------------------------------------------------------------------
// RSH1 — S-1
// ---------------------------------------------------------------------------------------

/// RSH1 (spec §2): the sender unit makes `/var/log` (journal included) and
/// `/var/lib/systemd/coredump` inaccessible, directly after `TemporaryFileSystem=/run:ro`;
/// no `TemporaryFileSystem=/var` (collides with `PrivateTmp=`); §2d of the companion guide
/// names both paths.
#[test]
fn test_rsh_s1_sender_unit_hides_system_logs() {
    let unit = read(SENDER_UNIT);
    let lines = unit_lines(&unit);
    assert_eq!(
        lines.iter().filter(|l| **l == INACCESSIBLE_LINE).count(),
        1,
        "{SENDER_UNIT} must carry `{INACCESSIBLE_LINE}` exactly once"
    );
    let run = lines
        .iter()
        .position(|l| *l == "TemporaryFileSystem=/run:ro")
        .expect("TemporaryFileSystem=/run:ro kept");
    assert_eq!(
        lines.get(run + 1).copied(),
        Some(INACCESSIBLE_LINE),
        "the directive follows TemporaryFileSystem=/run:ro directly (spec §2.1)"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.starts_with("TemporaryFileSystem=/var")),
        "TemporaryFileSystem=/var was rejected (spec §1.1)"
    );
    assert!(
        unit.lines()
            .filter(|l| l.trim_start().starts_with('#'))
            .any(|l| l.contains("/var/log")),
        "the unit header comment explains the hidden system logs (spec §2.1)"
    );
    let doc = read(REMOTE_DOC);
    let push = section(&doc, "2d.");
    for needle in ["/var/log", "coredump"] {
        assert!(
            push.contains(needle),
            "{REMOTE_DOC} §2d must say the sender cannot read {needle} (spec §2.3)"
        );
    }
}

// ---------------------------------------------------------------------------------------
// RSH3 — S-2
// ---------------------------------------------------------------------------------------

/// RSH3 (spec §3): `TAILSCALE_MIN_VERIFIED_VERSION = "1.102.4"`; the guide's Funnel
/// prerequisites name it and the old `1.38.3` is gone; the installer prints it with the
/// `tailscale version` check in an `echo` line; the installer still never runs `tailscale`
/// (RMC-S6 logic re-applied).
#[test]
fn test_rsh_s2_tailscale_min_version_is_single_sourced() {
    let lib = code_of(REMOTE_LIB);
    assert!(
        lib.contains(&format!(
            "pub const TAILSCALE_MIN_VERIFIED_VERSION: &str = \"{TAILSCALE_MIN}\";"
        )),
        "{REMOTE_LIB} must define TAILSCALE_MIN_VERIFIED_VERSION = \"{TAILSCALE_MIN}\""
    );
    let doc = read(REMOTE_DOC);
    assert!(
        section(&doc, "2b.").contains(&format!("{TAILSCALE_MIN} or later")),
        "{REMOTE_DOC} §2b must require Tailscale {TAILSCALE_MIN} or later"
    );
    assert!(
        !doc.contains("1.38.3"),
        "{REMOTE_DOC} must no longer name Tailscale 1.38.3"
    );

    let script = read(INSTALLER);
    let lines: Vec<&str> = script
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert!(
        lines.iter().any(|l| l.starts_with("echo")
            && l.contains(TAILSCALE_MIN)
            && l.contains("tailscale version")),
        "{INSTALLER} must print the Tailscale {TAILSCALE_MIN} requirement and `tailscale version`"
    );
    for line in &lines {
        let names_tailscale = line
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' '))
            .any(|segment| segment.split(' ').any(|w| w == "tailscale"));
        if names_tailscale {
            assert!(
                line.starts_with("echo") || line.starts_with("printf"),
                "`tailscale` may only appear inside echo/printf text (RMC-S6): {line}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// RSH5, RSH6 — S-3
// ---------------------------------------------------------------------------------------

/// RSH5, RSH6 (spec §4): the reserve constants (`1`, `200` ms ≤ 250 ms) are not configurable;
/// the dispatcher tries the reserve only after a `PerUidCap` rejection, bounds the whole first
/// frame of a reserve connection with `timeout(.., read_client_frame(..))` and admits only
/// `Auth` on it before `handle_request`; no readiness wait; the companion's retry branch
/// closes gracefully.
#[test]
fn test_rsh_s3_reserve_constants_and_dispatch() {
    let limits = code_of(DAEMON_LIMITS);
    assert_eq!(
        const_integer(&limits, "AUTH_RESERVED_CONNECTIONS_PER_UID", "usize"),
        1
    );
    let window = const_integer(&limits, "AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS", "u64");
    assert_eq!(window, 200, "spec §4.1");
    assert!(
        window <= 250,
        "the reserve window stays below the 500 ms close deadlines of existing contracts"
    );
    assert!(
        limits.contains("pub enum AdmissionClass"),
        "{DAEMON_LIMITS} defines AdmissionClass"
    );
    let peer_limits = body_of(&limits, "pub struct PeerLimitsConfig");
    let config = code_of(DAEMON_CONFIG);
    for name in ["AUTH_RESERVE", "auth_reserve"] {
        assert!(
            !peer_limits.contains(name),
            "the reserve is not a PeerLimitsConfig field (PE-SR7)"
        );
        assert!(
            !config.contains(name),
            "the reserve is not a daemon.toml key (PE-SR7)"
        );
    }

    let dispatcher = code_of(DAEMON_DISPATCHER);
    let handle = body_of(&dispatcher, "pub async fn handle_connection(");
    let regular = pos(&handle, ".try_acquire(", "handle_connection");
    let per_uid = pos(&handle, "PerUidCap", "handle_connection");
    let reserve = pos(&handle, "try_acquire_auth_reserve(", "handle_connection");
    assert!(
        regular < per_uid && per_uid < reserve,
        "the reserve is tried only after a PerUidCap rejection of try_acquire (spec §4.3 step 1)"
    );

    let process = body_of(&dispatcher, "async fn read_and_process(");
    let flat = compact(&process);
    let bounded = "Duration::from_millis(AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS),read_client_frame(";
    let at = pos(&flat, bounded, "read_and_process");
    assert!(
        flat[..at].ends_with("timeout("),
        "the reserve's first frame is read under timeout(.., read_client_frame(..)) (PE-SR3)"
    );
    assert!(
        !process.contains("read_exact("),
        "the frame read is factored into read_client_frame"
    );
    let reserve_check = pos(&process, "AdmissionClass::AuthReserve", "read_and_process");
    let handled = pos(&process, "handle_request(", "read_and_process");
    let auth_kind = pos(&process, "RequestKind::Auth", "read_and_process");
    assert!(
        reserve_check < handled && auth_kind < handled,
        "a reserve connection is checked for RequestKind::Auth before handle_request (spec §4.3 step 3)"
    );
    let frame_reader = body_of(&dispatcher, "async fn read_client_frame(");
    for needle in ["read_exact(", "MAX_MESSAGE_SIZE"] {
        assert!(
            frame_reader.contains(needle),
            "read_client_frame keeps the bounded frame read ({needle})"
        );
    }
    assert!(
        !dispatcher.contains(".readable()"),
        "no readiness wait bounds only the first byte (PE-SR3)"
    );

    let ipc = code_of(REMOTE_CAMERA_IPC);
    let exchange = body_of(&ipc, "async fn exchange(");
    let retry_at = pos(
        &exchange,
        "if reused && matches!(outcome, Err(AttemptError::Retryable))",
        "exchange",
    );
    let open = retry_at + exchange[retry_at..].find('{').expect("retry branch");
    let retry = &exchange[open..=block_end(&exchange, open)];
    assert!(
        retry.contains("graceful_close("),
        "the retry branch closes the old connection gracefully (spec §4.4)"
    );
    assert!(
        !exchange.contains("drop(conn)"),
        "no abrupt drop of a daemon connection in exchange (spec §4.4)"
    );
}

/// RSH5 (spec §4.3 step 3; auditor addition, `AI/auditor_constraints_remote_security_hardening.md`
/// C-5): the `Auth`-only rule of a reserve connection is decided before **any** message
/// processing, events included (`PasswordFailed` is never admitted on the reserve), and the
/// refusal is a protocol error, never a processed message.
#[test]
fn test_rsh_s3_reserve_refuses_events_before_processing() {
    let dispatcher = code_of(DAEMON_DISPATCHER);
    let process = body_of(&dispatcher, "async fn read_and_process(");
    // The first frame read may also name `AuthReserve` (its bounded window); the Auth-only
    // decision is the occurrence after decoding.
    let decoded = pos(&process, "decode_client_message(", "read_and_process");
    let after_decode = &process[decoded..];
    let reserve_check = pos(
        after_decode,
        "AdmissionClass::AuthReserve",
        "read_and_process after decode_client_message",
    );
    let auth_kind = pos(
        after_decode,
        "RequestKind::Auth",
        "read_and_process after decode_client_message",
    );
    for processing in ["handle_request(", "handle_event("] {
        let at = pos(
            after_decode,
            processing,
            "read_and_process after decode_client_message",
        );
        assert!(
            reserve_check < at && auth_kind < at,
            "the reserve's Auth-only check runs after decoding and before {processing} (spec §4.3)"
        );
    }
    assert!(
        process.contains("DaemonError::Protocol("),
        "a non-Auth message on the reserve is refused as a protocol error (spec §4.3 step 3)"
    );
}

// ---------------------------------------------------------------------------------------
// RSH7 — S-4
// ---------------------------------------------------------------------------------------

/// RSH7 (spec §5): §2h documents lock-screen privacy (Android setting, Chrome per-app
/// setting, `push_previews = "generic"`) and keeps the RAN12 owner check.
#[test]
fn test_rsh_s4_android_lock_screen_privacy_documented() {
    let doc = read(REMOTE_DOC);
    let android = section(&doc, "2h.");
    for needle in [
        "push_previews",
        "\"generic\"",
        "lock screen",
        "Hide sensitive",
        "RAN12",
    ] {
        assert!(
            android.contains(needle),
            "{REMOTE_DOC} §2h must contain `{needle}` (spec §5)"
        );
    }
}

// ---------------------------------------------------------------------------------------
// RSH9 — S-5
// ---------------------------------------------------------------------------------------

/// RSH9 (spec §6.2): `handle_preview_request` keeps the UID-level seat check and evaluates the
/// placement rule after it and before the preview rate limiter.
#[test]
fn test_rsh_s5_preview_placement_is_checked() {
    let dispatcher = code_of(DAEMON_DISPATCHER);
    let preview = body_of(&dispatcher, "async fn handle_preview_request(");
    let seat = pos(
        &preview,
        "has_local_seat_session(",
        "handle_preview_request",
    );
    let placement = pos(
        &preview,
        "authorize_preview_placement(",
        "handle_preview_request",
    );
    let limiter = pos(&preview, "preview_limiter", "handle_preview_request");
    assert!(
        seat < placement && placement < limiter,
        "6c-2 seat check, then 6c-2a placement, then the rate limiter (spec §6.2)"
    );
    let policy = code_of(DAEMON_SESSION_POLICY);
    for needle in [
        "pub enum PeerPlacement",
        "pub fn classify_peer_placement(",
        "pub fn authorize_preview_placement(",
    ] {
        assert!(
            policy.contains(needle),
            "{DAEMON_SESSION_POLICY} defines {needle}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// RSH10 — S-6
// ---------------------------------------------------------------------------------------

/// RSH10 (spec §7): `MANDATORY_HEADERS` (CSP included) byte-identical, the exact
/// `HARDENING_HEADERS` constant, both interpolated by the three head encoders, one CSP.
#[test]
fn test_rsh_s6_hardening_headers_exact_and_csp_unchanged() {
    let http = read(REMOTE_HTTP);
    assert!(
        http.contains(PINNED_MANDATORY_HEADERS),
        "MANDATORY_HEADERS stays byte-identical (RLC-S6)"
    );
    assert!(
        http.contains(PINNED_HARDENING_HEADERS),
        "HARDENING_HEADERS is exactly the spec §7 constant"
    );
    assert_eq!(
        http.matches("Content-Security-Policy:").count(),
        1,
        "exactly one CSP definition"
    );
    let code = code_of(REMOTE_HTTP);
    for encoder in [
        "pub fn encode_response_head(",
        "pub fn encode_sse_head(",
        "pub fn encode_camera_stream_head(",
    ] {
        assert!(
            body_of(&code, encoder).contains("{MANDATORY_HEADERS}{HARDENING_HEADERS}"),
            "{encoder} emits the mandatory then the hardening headers"
        );
    }
}

// ---------------------------------------------------------------------------------------
// RSH12 — S-7
// ---------------------------------------------------------------------------------------

/// RSH12 (spec §8): `main` probes the sandbox right after the root check, before reading
/// `XDG_RUNTIME_DIR` and before binding; the refusal is exit 78 with one constant log line;
/// the home anchor is the binary path relative to the home, equal to the unit's `ExecStart=`;
/// `probe_sandbox` derives it from `current_exe()` with a bounded mount-table read.
#[test]
fn test_rsh_s7_sender_checks_its_sandbox_first() {
    let main = code_of(SENDER_MAIN);
    let body = body_of(&main, "fn main(");
    let root = pos(&body, "check_not_root(", "main");
    let probe = pos(&body, "probe_sandbox()", "main");
    let xdg = pos(&body, "XDG_RUNTIME_DIR", "main");
    let bind = pos(&body, "bind_socket(", "main");
    assert!(
        root < probe && probe < xdg && probe < bind,
        "probe_sandbox() runs after check_not_root and before XDG_RUNTIME_DIR and bind_socket"
    );
    assert!(
        body.trim_start().starts_with("install_logging();"),
        "install_logging(); stays the first statement of main (RMC-S42)"
    );
    let refusal = body_of(&body[probe..], "probe_sandbox()");
    assert!(
        refusal
            .contains("tracing::error!(\"push sender sandbox not detected; refusing to start\")"),
        "one constant error line"
    );
    assert!(
        refusal.contains("exit_code(EXIT_CONFIG)"),
        "the refusal exits 78 (RestartPreventExitStatus=78)"
    );

    let lib = code_of(SENDER_LIB);
    for constant in [
        "pub const SANDBOX_MOUNTINFO_PATH: &str = \"/proc/self/mountinfo\";",
        "pub const MAX_MOUNTINFO_BYTES: usize = 1_048_576;",
        "pub const SANDBOX_HIDDEN_LOG_DIR: &str = \"/var/log\";",
        "pub const SENDER_EXE_HOME_RELATIVE: &str = \".local/bin/soos-push-sender\";",
    ] {
        assert!(
            lib.contains(constant),
            "{SENDER_LIB} must define {constant}"
        );
    }
    let unit = read(SENDER_UNIT);
    let exec: Vec<&str> = unit_lines(&unit)
        .into_iter()
        .filter(|l| l.starts_with("ExecStart="))
        .collect();
    assert_eq!(
        exec,
        vec!["ExecStart=%h/.local/bin/soos-push-sender"],
        "ExecStart= is %h/ followed by SENDER_EXE_HOME_RELATIVE"
    );
    for signature in [
        "pub fn home_from_exe(",
        "pub fn evaluate_sandbox(",
        "pub enum SandboxError",
    ] {
        assert!(lib.contains(signature), "{SENDER_LIB} defines {signature}");
    }
    let probe_body = body_of(&lib, "pub fn probe_sandbox(");
    for needle in [
        "current_exe(",
        "SANDBOX_MOUNTINFO_PATH",
        ".take(",
        "SANDBOX_HIDDEN_LOG_DIR",
        "evaluate_sandbox(",
    ] {
        assert!(
            probe_body.contains(needle),
            "probe_sandbox uses {needle} (spec §8.2)"
        );
    }
}

/// RSH12 (PE-SR1, stronger companion of RMC-S34): the sender's only environment read is the
/// `XDG_RUNTIME_DIR` read of `main.rs`; no home lookup through the environment, the passwd
/// database or `home_dir`; the library reads no environment at all.
#[test]
fn test_rsh_s7_sender_reads_no_other_environment_variable() {
    let files = rust_files(SENDER_SRC);
    assert!(!files.is_empty(), "{SENDER_SRC} has sources");
    let mut reads = Vec::new();
    for (rel, content) in &files {
        let code = strip_comments(production_part(content));
        for reader in ["env::var(", "env::var_os("] {
            for (at, _) in code.match_indices(reader) {
                reads.push((rel.clone(), code[at + reader.len()..].to_string()));
            }
        }
        for forbidden in [
            "env::vars",
            "home_dir",
            "\"HOME\"",
            "getenv",
            "User::from_uid",
            "getpwuid",
        ] {
            assert!(
                !code.contains(forbidden),
                "{rel} must not contain {forbidden} (PE-SR1)"
            );
        }
        if rel.ends_with("/lib.rs") {
            assert!(
                !code.contains("env::var"),
                "{rel} reads no environment variable"
            );
        }
    }
    assert_eq!(reads.len(), 1, "exactly one environment read in the sender");
    let (rel, rest) = &reads[0];
    assert_eq!(rel, SENDER_MAIN, "the only environment read is in main.rs");
    assert!(
        rest.starts_with("\"XDG_RUNTIME_DIR\")"),
        "the only environment read is XDG_RUNTIME_DIR"
    );
}

// ---------------------------------------------------------------------------------------
// RSH13 — S-8a
// ---------------------------------------------------------------------------------------

/// RSH13 (spec §9.1): a symlinked `remote.toml` is refused before any write; the template is
/// created under `set -C` (noclobber, `O_EXCL`); no other `cat >` targets the configuration;
/// the script still parses.
#[test]
fn test_rsh_s8a_installer_refuses_symlinked_config() {
    let script = read(INSTALLER);
    let lines: Vec<&str> = script.lines().map(str::trim).collect();
    let link = lines
        .iter()
        .position(|l| l.starts_with("if [[ -L \"${CONFIG_PATH}\" ]]"))
        .expect("the installer tests `[[ -L \"${CONFIG_PATH}\" ]]` first (spec §9.1)");
    let missing = lines
        .iter()
        .position(|l| l.starts_with("elif [[ ! -e \"${CONFIG_PATH}\" ]]"))
        .expect("the template branch follows as `elif [[ ! -e \"${CONFIG_PATH}\" ]]`");
    assert!(
        link < missing,
        "the symlink check precedes the template branch"
    );
    let refusal = &lines[link + 1..missing];
    assert!(
        refusal
            .iter()
            .any(|l| l.starts_with("exit ") && *l != "exit 0"),
        "the symlink branch exits non-zero"
    );
    for write in [
        "cat ", "chmod", "mkdir", "install ", "tee", "cp ", "mv ", "ln ",
    ] {
        assert!(
            !refusal.iter().any(|l| l.starts_with(write)),
            "the symlink branch writes nothing ({write})"
        );
    }
    assert!(
        refusal.iter().all(|l| !l.contains("> \"${CONFIG_PATH}\"")),
        "the symlink branch never redirects into the configuration"
    );
    let template = lines
        .iter()
        .position(|l| *l == "(set -C; cat > \"${CONFIG_PATH}\") <<'TEMPLATE'")
        .expect("the template is written with `(set -C; cat > \"${CONFIG_PATH}\") <<'TEMPLATE'`");
    assert!(
        missing < template,
        "the template write is inside the elif branch"
    );
    assert_eq!(
        script.matches("cat > \"${CONFIG_PATH}\"").count(),
        1,
        "no other `cat >` writes the configuration"
    );
    assert_eq!(
        script.matches("> \"${CONFIG_PATH}\"").count(),
        1,
        "no other redirection writes the configuration"
    );
    let status = Command::new("bash")
        .arg("-n")
        .arg(workspace_root().join(INSTALLER))
        .status()
        .expect("bash available");
    assert!(status.success(), "bash -n {INSTALLER}");
}

// ---------------------------------------------------------------------------------------
// RSH14 — S-8b
// ---------------------------------------------------------------------------------------

/// RSH14 (spec §9.2, PE-SR5): `camera_stream` re-checks the stop after `mark_streaming` and
/// before the head, and again after the head and before the view counts as shown; the first
/// part is written after both.
#[test]
fn test_rsh_s8b_stop_rechecked_before_head_and_before_first_part() {
    let server = code_of(REMOTE_SERVER);
    let stream = body_of(&server, "async fn camera_stream<");
    let marked = pos(&stream, "mark_streaming(", "camera_stream");
    let after_mark = &stream[marked..];
    let first = pos(after_mark, "stop_requested(", "camera_stream") + marked;
    let head = pos(&stream, "encode_camera_stream_head()", "camera_stream");
    assert!(
        marked < first && first < head,
        "a stop check between mark_streaming and the head (spec §9.2)"
    );
    let second = pos(&stream[head..], "stop_requested(", "camera_stream") + head;
    let shown = pos(&stream, "set_shown()", "camera_stream");
    let part = pos(&stream, "write_part(", "camera_stream");
    assert!(
        head < second && second < shown && second < part,
        "a stop check after the head and before set_shown/write_part (PE-SR5)"
    );
}

// ---------------------------------------------------------------------------------------
// RSH15 — S-8c
// ---------------------------------------------------------------------------------------

/// RSH15 (spec §9.3): the worker's body limit equals `MAX_PUSH_SUBSCRIBE_BODY_BYTES`; the
/// renewal listener sends `renewalPayload(subscription, event.oldSubscription)`;
/// `replacesEndpoint` appears only in `renewalPayload`; one `fetch(` in the worker.
#[test]
fn test_rsh_s8c_worker_names_the_replaced_endpoint() {
    let sw = read(REMOTE_SW);
    let lib = code_of(REMOTE_LIB);
    let limit = const_integer(&lib, "MAX_PUSH_SUBSCRIBE_BODY_BYTES", "usize");
    assert!(
        sw.contains(&format!("const SUBSCRIBE_BODY_LIMIT = {limit};")),
        "{REMOTE_SW}: SUBSCRIBE_BODY_LIMIT = {limit} (MAX_PUSH_SUBSCRIBE_BODY_BYTES)"
    );
    assert_eq!(limit, 2048);
    let listener = body_of(&sw, "self.addEventListener(\"pushsubscriptionchange\"");
    assert!(
        listener.contains("renewalPayload(subscription, event.oldSubscription)"),
        "the listener posts renewalPayload(subscription, event.oldSubscription)"
    );
    let helper_at = pos(&sw, "function renewalPayload(subscription, old)", "sw.js");
    let open = helper_at + sw[helper_at..].find('{').expect("helper body");
    let end = block_end(&sw, open);
    let total = sw.matches("replacesEndpoint").count();
    let inside = sw[open..end].matches("replacesEndpoint").count();
    assert!(inside >= 1, "renewalPayload names replacesEndpoint");
    assert_eq!(total, inside, "replacesEndpoint only inside renewalPayload");
    assert!(
        sw[open..end].contains("SUBSCRIBE_BODY_LIMIT"),
        "renewalPayload checks the body limit"
    );
    assert_eq!(sw.matches("fetch(").count(), 1, "still exactly one fetch(");
}

// ---------------------------------------------------------------------------------------
// RSH16 — S-8d
// ---------------------------------------------------------------------------------------

/// RSH16 (spec §9.4, PE-SR6, PE-SR8): the remote conversion is called only in
/// `handle_preview_request`, after the preview limiter's guard block closed; its IR branch
/// recurses into itself, never into the GUI function; the bounds are 640x480.
#[test]
fn test_rsh_s8d_remote_conversion_off_the_auth_path() {
    let dispatcher = code_of(DAEMON_DISPATCHER);
    let call = "remote_preview_image_for_frame(";
    let total = dispatcher.matches(call).count();
    assert!(total >= 1, "{DAEMON_DISPATCHER} calls {call}");
    let sig = "async fn handle_preview_request(";
    let fn_at = pos(&dispatcher, sig, "dispatcher");
    let open = fn_at + dispatcher[fn_at..].find('{').expect("body");
    let end = block_end(&dispatcher, open);
    let body = &dispatcher[open..end];
    assert_eq!(
        body.matches(call).count(),
        total,
        "{call} is called only inside handle_preview_request"
    );
    let lock = pos(
        body,
        "self.preview_limiter.lock().await",
        "handle_preview_request",
    );
    // The block enclosing the limiter guard: the nearest unmatched `{` before it.
    let mut depth = 0usize;
    let mut guard_open = None;
    for (i, c) in body[..lock].char_indices().rev() {
        match c {
            '}' => depth += 1,
            '{' if depth == 0 => {
                guard_open = Some(i);
                break;
            }
            '{' => depth -= 1,
            _ => {}
        }
    }
    let guard_open = guard_open.expect("the limiter guard lives in a block");
    assert!(guard_open > 0, "the limiter guard has its own block");
    let guard_end = block_end(body, guard_open);
    for (at, _) in body.match_indices(call) {
        assert!(
            at > guard_end,
            "{call} runs after the preview limiter guard was released (PE-SR6)"
        );
    }

    let image = code_of(DAEMON_PREVIEW_IMAGE);
    for constant in [
        "pub const REMOTE_PREVIEW_MAX_WIDTH: u32 = 640;",
        "pub const REMOTE_PREVIEW_MAX_HEIGHT: u32 = 480;",
    ] {
        assert!(
            image.contains(constant),
            "{DAEMON_PREVIEW_IMAGE}: {constant}"
        );
    }
    let remote = body_of(&image, "pub fn remote_preview_image_for_frame(");
    let recursive = remote.matches("remote_preview_image_for_frame(").count();
    assert!(
        recursive >= 1,
        "the IR branch recurses into remote_preview_image_for_frame (PE-SR8)"
    );
    assert_eq!(
        remote.matches("preview_image_for_frame(").count(),
        recursive,
        "remote_preview_image_for_frame never calls the GUI preview_image_for_frame (PE-SR8)"
    );
}

// ---------------------------------------------------------------------------------------
// RSH17 — docs
// ---------------------------------------------------------------------------------------

/// RSH17 (spec §14, §2.3, §4.5, §6.4): the ADR, the architecture §13 (system logs and the
/// auth-reserved connection), walkthrough 194 and IPC decision-table row 6c-2a.
#[test]
fn test_rsh_docs_adr_and_walkthrough() {
    assert!(
        read("AI/DECISIONS.md").contains(ADR_TITLE),
        "AI/DECISIONS.md must contain the ADR {ADR_TITLE}"
    );
    let architecture = read("AI/ARCHITECTURE.md");
    let remote = section(&architecture, "13.");
    for needle in ["system logs", "auth-reserved"] {
        assert!(
            remote.contains(needle),
            "AI/ARCHITECTURE.md §13 must mention {needle}"
        );
    }
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH} must exist");
    assert!(
        read("Docs/IPC_PROTOCOL.md").contains("| 6c-2a |"),
        "Docs/IPC_PROTOCOL.md decision table gains row 6c-2a"
    );
}
