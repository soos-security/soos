//! Static contracts of the live camera view of `soos-remote` (ADR 2026-10-07 "Live Camera
//! View in `soos-remote` Through the Daemon Preview Channel", GitHub #345, architect spec
//! `AI/architect_spec_remote_live_camera.md` §13.8 tests 50–59, invariants RLC-S1–RLC-S13,
//! matrix rows RLC2, RLC3, RLC10, RLC13, RLC15):
//!
//! - RLC-S1 dependencies: `soos-protocol` and `jpeg-encoder =0.7.1` only, no decoder, no
//!   camera, vision or inference crate;
//! - RLC-S2 the `IJG` licence is allowed for `jpeg-encoder` only;
//! - RLC-S3 the camera modules never log pixels, tokens or frame properties;
//! - RLC-S4 no recording path;
//! - RLC-S5 secret- and pixel-bearing types are redacted or have no `Debug`;
//! - RLC-S6 the page reads the stream into a canvas, CSP unchanged;
//! - RLC-S7 the daemon `remote_view` gate and seat-session check;
//! - RLC-S8 documentation; RLC-S9 installer template; RLC-S10 no decoder, no device access.
//! - M14 (owner request 2026-10-07): a camera view sends no Web Push.

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
    exists, field_keys, macro_invocations, production_part, read, rust_files, strip_comments,
    toml_table, workspace_root, TRACING_MACROS,
};

/// The four camera modules of spec S-1.
const CAMERA_MODULES: [&str; 4] = [
    "crates/remote/src/camera.rs",
    "crates/remote/src/camera_ipc.rs",
    "crates/remote/src/camera_jpeg.rs",
    "crates/remote/src/camera_slot.rs",
];

/// Reads `rel` after asserting it exists, so a missing file fails with the spec reference.
fn read_required(rel: &str, why: &str) -> String {
    assert!(exists(rel), "{rel} must exist ({why})");
    read(rel)
}

/// Comment-stripped content of every camera module (each must exist).
fn camera_sources() -> Vec<(String, String)> {
    CAMERA_MODULES
        .iter()
        .map(|rel| {
            (
                (*rel).to_string(),
                strip_comments(&read_required(rel, "spec S-1 camera module layout")),
            )
        })
        .collect()
}

/// `cargo tree -p <package> -e normal` package names, or `None` when cargo is unavailable.
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
        "cargo tree -p {package} failed (run `cargo fetch` once after the manifest change): {}",
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

/// Resolved features of every `jpeg-encoder` node in `cargo tree <scope> -e normal
/// --target all`, or `None` when cargo is unavailable. Each entry is the sorted feature set of
/// one `jpeg-encoder` line (`{p}|{f}` format); an empty vector means the crate is absent.
fn jpeg_encoder_features(scope: &[&str]) -> Option<Vec<Vec<String>>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    if Command::new(&cargo).arg("--version").output().is_err() {
        return None;
    }
    let mut args = vec!["tree", "--offline", "--locked"];
    args.extend_from_slice(scope);
    args.extend_from_slice(&[
        "-e", "normal", "--target", "all", "--prefix", "none", "--format", "{p}|{f}",
    ]);
    let output = Command::new(&cargo)
        .current_dir(workspace_root())
        .args(&args)
        .output()
        .expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo {} failed (run `cargo fetch` once after the manifest change): {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut sets: Vec<Vec<String>> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("jpeg-encoder v"))
        .map(|line| {
            let features = line.split_once('|').map_or("", |(_, f)| f);
            let features = features.split_whitespace().next().unwrap_or("");
            let mut set: Vec<String> = features
                .split(',')
                .filter(|f| !f.is_empty())
                .map(str::to_string)
                .collect();
            set.sort();
            set.dedup();
            set
        })
        .collect();
    sets.sort();
    sets.dedup();
    Some(sets)
}

/// `(table header, line)` pairs of a TOML manifest: every non-header line with the header of
/// the table it belongs to (`""` before the first header).
fn lines_with_tables(manifest: &str) -> Vec<(String, String)> {
    let mut table = String::new();
    let mut out = Vec::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            table = trimmed.to_string();
        } else {
            out.push((table.clone(), trimmed.to_string()));
        }
    }
    out
}

/// Every `fn <name>` item of comment-stripped `code` whose name satisfies `select`, with its
/// balanced-brace body. Declarations without a body (`fn f();`) are skipped.
fn fn_bodies(code: &str, select: impl Fn(&str) -> bool) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(pos) = code[start..].find("fn ") {
        let at = start + pos;
        start = at + 3;
        let boundary = at == 0 || {
            let b = code.as_bytes()[at - 1];
            !(b.is_ascii_alphanumeric() || b == b'_')
        };
        if !boundary {
            continue;
        }
        let name: String = code[at + 3..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() || !select(&name) {
            continue;
        }
        let rest = &code[at..];
        let brace = rest.find('{');
        let semi = rest.find(';');
        if let (Some(b), Some(sc)) = (brace, semi) {
            if sc < b {
                continue;
            }
        }
        if brace.is_none() {
            continue;
        }
        out.push((name, block_from(code, at)));
    }
    out
}

/// Logging macros of the camera scans: the RMC tracing macros plus `event!`.
fn log_macros() -> Vec<&'static str> {
    let mut macros: Vec<&'static str> = TRACING_MACROS.to_vec();
    macros.extend_from_slice(&["event", "tracing::event"]);
    macros
}

/// Field names a camera log line must never carry (pixels, frame properties, the token).
const CAMERA_FORBIDDEN_LOG_FIELDS: [&str; 6] =
    ["token", "width", "height", "bytes", "sequence", "len"];

/// Asserts that no logging call in `body` has a format argument or a forbidden field.
fn assert_camera_log_hygiene(rel: &str, name: &str, body: &str) {
    for args in macro_invocations(body, &log_macros()) {
        assert!(
            !args.contains('{'),
            "{rel}::{name}: camera log lines are fixed text, no format argument (RLC-S3): {args}"
        );
        for key in field_keys(&args) {
            for forbidden in CAMERA_FORBIDDEN_LOG_FIELDS {
                assert!(
                    !key.contains(forbidden),
                    "{rel}::{name}: camera log field `{key}` names {forbidden} (RLC-S3): {args}"
                );
            }
        }
    }
}

/// Recording-path needles of RLC-S4 / C33, `use std::{fs, ..}` aliases included.
const RECORDING_NEEDLES: [&str; 15] = [
    "std::fs",
    "tokio::fs",
    "fs::",
    "{fs",
    " fs,",
    ", fs}",
    ", fs ",
    "File::",
    "OpenOptions",
    "create_dir",
    "std::env::temp_dir",
    "tempfile",
    "write_to_path",
    "Command::new",
    "process::Command",
];

/// The recording-path needles found in comment-stripped `code`.
fn recording_hits(code: &str) -> Vec<&'static str> {
    RECORDING_NEEDLES
        .iter()
        .copied()
        .filter(|needle| code.contains(needle))
        .collect()
}

/// The camera functions of the non-camera modules (spec §1.1): the `server.rs` handlers
/// and the `http.rs` part encoders (M14: `push.rs` has no camera notification any more).
fn camera_functions_outside_modules() -> Vec<(String, String, Vec<String>)> {
    let mut out = Vec::new();
    for (rel, required, select) in [
        (
            "crates/remote/src/server.rs",
            vec![
                "camera_options",
                "camera_start",
                "camera_stream",
                "camera_stop",
            ],
            (|n: &str| n.contains("camera")) as fn(&str) -> bool,
        ),
        (
            "crates/remote/src/http.rs",
            vec!["encode_camera_stream_head", "encode_camera_part_head"],
            (|n: &str| n.starts_with("encode_camera")) as fn(&str) -> bool,
        ),
    ] {
        let code = strip_comments(production_part(&read(rel)));
        let bodies = fn_bodies(&code, select);
        let names: Vec<String> = bodies.iter().map(|(n, _)| n.clone()).collect();
        for name in required {
            assert!(
                names.iter().any(|n| n == name),
                "{rel} must define `fn {name}` (spec §1.1, camera code scanned by RLC-S3/RLC-S4)"
            );
        }
        for (name, body) in bodies {
            out.push((rel.to_string(), name, vec![body.to_string()]));
        }
    }
    out
}

/// Attribute lines (`#[...]`) directly above `item` (blank lines skipped) in comment-stripped
/// code, joined; empty when `item` is absent.
fn attributes_above(code: &str, item: &str) -> String {
    let Some(pos) = code.find(item) else {
        return String::new();
    };
    let mut attrs = Vec::new();
    for line in code[..pos].lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
            attrs.push(trimmed.to_string());
        } else {
            break;
        }
    }
    attrs.join(" ")
}

/// Body (balanced braces) of the first block that starts at or after `start`.
fn block_from(code: &str, start: usize) -> &str {
    let Some(open_rel) = code[start..].find('{') else {
        return "";
    };
    let open = start + open_rel;
    let mut depth = 0usize;
    for (i, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &code[open..=open + i];
                }
            }
            _ => {}
        }
    }
    &code[open..]
}

/// Body of `impl ... Debug for <ty>` (any path spelling), if any.
fn manual_debug_impl<'a>(code: &'a str, ty: &str) -> Option<&'a str> {
    let needle = format!("Debug for {ty}");
    let mut start = 0;
    while let Some(pos) = code[start..].find(&needle) {
        let at = start + pos;
        let after = code[at + needle.len()..].chars().next();
        if !matches!(after, Some(c) if c.is_ascii_alphanumeric() || c == '_') {
            return Some(block_from(code, at));
        }
        start = at + needle.len();
    }
    None
}

/// Lines of the markdown section starting with `heading_prefix` up to the next `## `.
fn markdown_section(doc: &str, heading_prefix: &str) -> Option<String> {
    let lines: Vec<&str> = doc.lines().collect();
    let start = lines.iter().position(|l| l.starts_with(heading_prefix))?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("## "))
        .map_or(lines.len(), |p| start + 1 + p);
    Some(lines[start..end].join("\n"))
}

// ---------------------------------------------------------------------------------------
// Test 50 — RLC-S1 dependencies
// ---------------------------------------------------------------------------------------

#[test]
fn test_rlc_s1_remote_camera_dependencies() {
    let manifest = read("crates/remote/Cargo.toml");
    let deps = toml_table(&manifest, "dependencies");
    assert!(
        deps.contains(&"soos-protocol = { workspace = true }"),
        "crates/remote/Cargo.toml must declare exactly `soos-protocol = {{ workspace = true }}` (LC-1)"
    );
    assert!(
        deps.contains(&"jpeg-encoder = { workspace = true }"),
        "crates/remote/Cargo.toml must declare exactly `jpeg-encoder = {{ workspace = true }}` (LC-2)"
    );
    let nix: Vec<&&str> = deps.iter().filter(|l| l.starts_with("nix ")).collect();
    assert_eq!(nix.len(), 1, "exactly one `nix` dependency line");
    let nix_line = nix[0];
    assert!(
        nix_line.contains("workspace = true")
            && nix_line.contains("features")
            && nix_line.contains("\"time\""),
        "the `nix` dependency of soos-remote enables the `time` feature (CLOCK_MONOTONIC, spec §5.2): {nix_line}"
    );

    let root = read("Cargo.toml");
    let workspace_deps = toml_table(&root, "workspace.dependencies");
    let jpeg: Vec<&&str> = workspace_deps
        .iter()
        .filter(|l| l.starts_with("jpeg-encoder ") || l.starts_with("jpeg-encoder="))
        .collect();
    assert_eq!(
        jpeg,
        vec![&"jpeg-encoder = { version = \"=0.7.1\" }"],
        "the workspace pins `jpeg-encoder = {{ version = \"=0.7.1\" }}` (default features only, never `simd`)"
    );
    assert!(
        !root.contains("jpeg-encoder = { version = \"=0.7.1\", features")
            && !root
                .lines()
                .any(|l| l.contains("jpeg-encoder") && l.contains("simd")),
        "jpeg-encoder must never enable features (no `simd`)"
    );

    if let Some(names) = tree_names("soos-remote") {
        for required in ["jpeg-encoder", "soos-protocol"] {
            assert!(
                names.iter().any(|n| n == required),
                "cargo tree -p soos-remote -e normal must contain {required}"
            );
        }
        for forbidden in [
            "wide",
            "bytemuck",
            "jpeg-decoder",
            "zune-jpeg",
            "image",
            "soos-camera-v4l",
            "v4l",
            "ort",
        ] {
            assert!(
                !names.iter().any(|n| n == forbidden),
                "soos-remote must not depend (normal edges) on {forbidden} (RLC-S1)"
            );
        }
    }

    // B1: the resolved feature set of jpeg-encoder is exactly {default, std}, for soos-remote
    // alone and with every workspace member unified (a `simd` enabled anywhere fails).
    for scope in [&["-p", "soos-remote"][..], &["--workspace"][..]] {
        if let Some(sets) = jpeg_encoder_features(scope) {
            assert!(
                !sets.is_empty(),
                "cargo tree {} must resolve jpeg-encoder (LC-2)",
                scope.join(" ")
            );
            for set in &sets {
                assert_eq!(
                    set,
                    &vec!["default".to_string(), "std".to_string()],
                    "cargo tree {}: jpeg-encoder resolved features must be exactly {{default, std}} (never `simd`, LC-2)",
                    scope.join(" ")
                );
            }
        }
    }
    // B1: no feature or target-specific table enables jpeg-encoder differently.
    for rel in ["crates/remote/Cargo.toml", "Cargo.toml"] {
        for (table, line) in lines_with_tables(&read(rel)) {
            if !line.contains("jpeg-encoder") {
                continue;
            }
            assert!(
                !line.contains("simd") && !line.contains("features"),
                "{rel}: the jpeg-encoder line must not enable features (no `simd`): {line}"
            );
            assert!(
                !table.starts_with("[target."),
                "{rel}: jpeg-encoder must not appear in a target-specific table {table}: {line}"
            );
        }
    }
    assert!(
        !read("crates/remote/Cargo.toml").contains("simd"),
        "crates/remote/Cargo.toml must not mention `simd` (LC-2)"
    );
}

// ---------------------------------------------------------------------------------------
// Test 51 — RLC-S2 crate-scoped IJG exception
// ---------------------------------------------------------------------------------------

#[test]
fn test_rlc_s2_ijg_is_allowed_for_jpeg_encoder_only() {
    let deny = read("deny.toml");
    let lines: Vec<&str> = deny.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.trim() == "[licenses]")
        .expect("deny.toml has a [licenses] table");
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |p| start + 1 + p);
    let section = &lines[start..end];

    // The global allow list never contains IJG.
    let allow_start = section
        .iter()
        .position(|l| l.trim_start().starts_with("allow"))
        .expect("[licenses] has an allow list");
    let allow_end = section[allow_start..]
        .iter()
        .position(|l| l.contains(']'))
        .map_or(section.len(), |p| allow_start + p);
    let allow = section[allow_start..=allow_end.min(section.len() - 1)].join("\n");
    assert!(
        !allow.contains("IJG"),
        "deny.toml [licenses] allow must not contain IJG (crate-scoped exception only, LC-2)"
    );

    let exc = section
        .iter()
        .position(|l| l.trim_start().starts_with("exceptions"));
    let Some(exc) = exc else {
        panic!(
            "deny.toml [licenses] must contain `exceptions = [{{ crate = \"jpeg-encoder\", allow = [\"IJG\"] }}]` (LC-2)"
        );
    };
    // The exception value (balanced brackets), whitespace-free.
    let mut value = String::new();
    let mut depth = 0i32;
    let mut started = false;
    'outer: for line in &section[exc..] {
        let code = line.split('#').next().unwrap_or("");
        for c in code.chars() {
            if c.is_whitespace() {
                continue;
            }
            value.push(c);
            if c == '[' {
                depth += 1;
                started = true;
            } else if c == ']' {
                depth -= 1;
                if started && depth == 0 {
                    break 'outer;
                }
            }
        }
    }
    let normalized = value.replace(",}", "}").replace(",]", "]");
    assert_eq!(
        normalized, "exceptions=[{crate=\"jpeg-encoder\",allow=[\"IJG\"]}]",
        "exactly one licence exception: jpeg-encoder / IJG"
    );
    let comment = section[..exc]
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty())
        .copied()
        .unwrap_or("");
    assert!(
        comment.trim_start().starts_with('#')
            && (comment.contains("ADR") || comment.contains("2026-10-07")),
        "the IJG exception is preceded by a comment naming the ADR 2026-10-07: {comment}"
    );
    assert_eq!(
        deny.matches("\"IJG\"").count(),
        1,
        "\"IJG\" appears exactly once in deny.toml (the jpeg-encoder exception)"
    );
}

// ---------------------------------------------------------------------------------------
// Test 52 — RLC-S3 camera modules never log
// ---------------------------------------------------------------------------------------

#[test]
fn test_rlc_s3_camera_modules_never_log() {
    for (rel, code) in camera_sources() {
        let calls = macro_invocations(&code, &log_macros());
        if rel.ends_with("/camera.rs") {
            for args in calls {
                let trimmed = args.trim().trim_end_matches(',').trim();
                assert!(
                    trimmed.starts_with('"')
                        && trimmed.ends_with('"')
                        && trimmed.matches('"').count() == 2
                        && !trimmed.contains('{')
                        && field_keys(&args).is_empty(),
                    "{rel}: every tracing call has exactly one fixed string literal and no field: {args}"
                );
            }
        } else {
            assert!(
                calls.is_empty(),
                "{rel} must contain no tracing macro invocation (RLC-S3): {calls:?}"
            );
        }
    }
    // B4: the camera code of server.rs / http.rs logs fixed text only, without a
    // token, dimension, size or sequence field.
    for (rel, name, bodies) in camera_functions_outside_modules() {
        for body in bodies {
            assert_camera_log_hygiene(&rel, &name, &body);
        }
    }
    let audit = strip_comments(&read("crates/remote/src/audit.rs"));
    for message in [
        "\"camera view started\"",
        "\"camera view ended\"",
        "\"camera view refused\"",
    ] {
        assert!(
            audit.contains(message),
            "crates/remote/src/audit.rs declares the fixed message {message} (spec §10.2)"
        );
    }
    for function in [
        "fn camera_view_started(",
        "fn camera_view_ended(",
        "fn camera_view_refused(",
    ] {
        assert!(
            audit.contains(function),
            "crates/remote/src/audit.rs defines {function} (spec §10.2)"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Test 53 — RLC-S4 no recording path
// ---------------------------------------------------------------------------------------

#[test]
fn test_rlc_s4_no_recording_path() {
    let sources = camera_sources();
    assert_eq!(sources.len(), CAMERA_MODULES.len());
    // Every `camera*.rs` in the crate, including any module added later.
    let all: Vec<(String, String)> = rust_files("crates/remote/src")
        .into_iter()
        .filter(|(rel, _)| {
            rel.rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with("camera"))
        })
        .collect();
    assert!(all.len() >= CAMERA_MODULES.len());
    for (rel, content) in all {
        let code = strip_comments(&content);
        let hits = recording_hits(&code);
        assert!(
            hits.is_empty(),
            "{rel} must not contain {hits:?} (no recording, RLC-S7, C33)"
        );
    }
    // B5: the camera handlers of server.rs (and the other camera functions) add no file,
    // temp-dir or process I/O either.
    for (rel, name, bodies) in camera_functions_outside_modules() {
        for body in bodies {
            let hits = recording_hits(&body);
            assert!(
                hits.is_empty(),
                "{rel}::{name} must not contain {hits:?} (no recording, RLC-S7, C33)"
            );
        }
    }
}

/// Self-test of the B5 scanner: `use std::{fs, io}; fs::write(..)` and a bare `{fs` import
/// are caught; ordinary camera code is not.
#[test]
fn test_rlc_s4_recording_scanner_catches_aliased_fs() {
    let aliased = "use std::{fs, io};\nfn save(b: &[u8]) { fs::write(\"/tmp/x\", b).ok(); }\n";
    assert!(recording_hits(&strip_comments(aliased)).contains(&"fs::"));
    assert!(recording_hits(&strip_comments(aliased)).contains(&"{fs"));
    assert!(!recording_hits("use std::{io, fs};").is_empty());
    let tokio = "use tokio::{fs as f};";
    assert!(!recording_hits(tokio).is_empty());
    let clean = "use std::io::Write;\nfn part(len: usize) -> Vec<u8> { Vec::with_capacity(len) }\n";
    assert!(
        recording_hits(clean).is_empty(),
        "{:?}",
        recording_hits(clean)
    );
    let code = "fn camera_stop() { let _ = std::fs::read(\"x\"); }\nfn other() {}\n";
    let bodies = fn_bodies(code, |n| n.contains("camera"));
    assert_eq!(bodies.len(), 1);
    assert!(!recording_hits(bodies[0].1).is_empty());
    let logs = "fn camera_x() { tracing::event!(Level::INFO, token = %t, \"x\"); }";
    let body = fn_bodies(logs, |n| n.contains("camera"))[0].1;
    let calls = macro_invocations(body, &log_macros());
    assert_eq!(calls.len(), 1, "event! is a logging macro");
    assert!(field_keys(&calls[0]).iter().any(|k| k.contains("token")));
}

// ---------------------------------------------------------------------------------------
// Test 54 — RLC-S5 redacted types
// ---------------------------------------------------------------------------------------

#[test]
fn test_rlc_s5_camera_types_redacted() {
    let slot = strip_comments(&read_required(
        "crates/remote/src/camera_slot.rs",
        "spec §7",
    ));
    for ty in ["ViewToken", "ViewOwner"] {
        let item = format!("pub struct {ty}");
        assert!(slot.contains(&item), "camera_slot.rs defines `{item}`");
        let attrs = attributes_above(&slot, &item);
        assert!(
            !attrs.contains("Debug"),
            "{ty} must not derive Debug (RLC-S4): {attrs}"
        );
        let body = manual_debug_impl(&slot, ty)
            .unwrap_or_else(|| panic!("{ty} has a manual redacted Debug impl (spec §7.1)"));
        assert!(
            body.contains("<redacted>"),
            "the manual Debug of {ty} prints `<redacted>`: {body}"
        );
    }
    let token_attrs = attributes_above(&slot, "pub struct ViewToken");
    assert!(
        slot.contains("impl Drop for ViewToken")
            || slot.contains("Drop for ViewToken {")
            || token_attrs.contains("ZeroizeOnDrop"),
        "ViewToken is zeroized on drop (`impl Drop` or `ZeroizeOnDrop`, spec §7.1)"
    );

    let ipc = strip_comments(&read_required("crates/remote/src/camera_ipc.rs", "spec §5"));
    let jpeg = strip_comments(&read_required(
        "crates/remote/src/camera_jpeg.rs",
        "spec §6",
    ));
    for (code, file, ty) in [
        (&ipc, "camera_ipc.rs", "PreviewFrame"),
        (&jpeg, "camera_jpeg.rs", "JpegFrame"),
        (&jpeg, "camera_jpeg.rs", "JpegSink"),
    ] {
        let item = format!("pub struct {ty}");
        assert!(code.contains(&item), "{file} defines `{item}`");
        let attrs = attributes_above(code, &item);
        assert!(
            !attrs.contains("Debug"),
            "{ty} must not derive Debug (pixel-bearing, RLC-S7): {attrs}"
        );
        assert!(
            manual_debug_impl(code, ty).is_none(),
            "{ty} must have no Debug impl at all (pixel-bearing, RLC-S7)"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Test 55 — RLC-S6 page reads the stream into a canvas, CSP unchanged
// ---------------------------------------------------------------------------------------

/// `MANDATORY_HEADERS` of `crates/remote/src/http.rs` before GitHub #345, byte for byte.
const PINNED_MANDATORY_HEADERS: &str =
    "const MANDATORY_HEADERS: &str = \"Cache-Control: no-store\\r\\n\\
    Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; \\
    img-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; \\
    form-action 'none'; frame-ancestors 'none'\\r\\n\\
    X-Content-Type-Options: nosniff\\r\\n\\
    Referrer-Policy: no-referrer\\r\\n\\
    X-Frame-Options: DENY\\r\\n\\
    Connection: close\\r\\n\";";

#[test]
fn test_rlc_s6_page_reads_the_stream_into_a_canvas() {
    let http = read("crates/remote/src/http.rs");
    assert!(
        http.contains(PINNED_MANDATORY_HEADERS),
        "crates/remote/src/http.rs MANDATORY_HEADERS (CSP included) must stay byte-identical (ADR item 6)"
    );
    assert_eq!(
        http.matches("Content-Security-Policy:").count(),
        1,
        "exactly one CSP definition in http.rs"
    );

    let app = read("crates/remote/assets/app.js");
    for needle in [
        "/api/auth/camera/options",
        "/api/camera/start",
        "/api/camera/stop",
        "\"/api/camera\"",
        "\"X-Soos-Action\": \"camera-view\"",
        "\"camera-stream\"",
        "\"camera-stop\"",
        "getReader(",
        "createImageBitmap(",
        "drawImage(",
        "AbortController",
        "CAMERA_MAX_PART_BYTES = 524288",
        "Start camera view",
        "Stop camera view",
        "pagehide",
    ] {
        assert!(
            app.contains(needle),
            "crates/remote/assets/app.js must contain {needle} (spec §10.3)"
        );
    }
    for forbidden in [
        "createObjectURL",
        "blob:",
        "new Image(",
        "\"img\"",
        ".src =",
    ] {
        assert!(
            !app.contains(forbidden),
            "crates/remote/assets/app.js must not contain {forbidden} (RLC-S12)"
        );
    }
    let index = read("crates/remote/assets/index.html");
    assert!(
        !index.contains("id=\"camera"),
        "index.html gains no camera id (spec S-3, runtime-built card)"
    );
}

// ---------------------------------------------------------------------------------------
// Test 56 — RLC-S7 daemon remote_view gate
// ---------------------------------------------------------------------------------------

const FIRST_FRAME_MESSAGE: &str =
    "\"Remote camera view: first preview frame served to soos-remote on this connection\"";

#[test]
fn test_rlc_s7_daemon_remote_view_gate() {
    let peer = strip_comments(&read_required(
        "crates/daemon/src/preview_peer.rs",
        "spec §9.2",
    ));
    assert!(
        peer.contains("pub const REMOTE_COMPANION_UNIT: &str = \"soos-remote.service\";"),
        "preview_peer.rs defines REMOTE_COMPANION_UNIT = \"soos-remote.service\""
    );
    assert!(
        peer.contains("pub fn classify_preview_peer_cgroup("),
        "preview_peer.rs defines classify_preview_peer_cgroup (spec §9.2)"
    );
    let daemon_lib = strip_comments(&read("crates/daemon/src/lib.rs"));
    assert!(
        daemon_lib.contains("pub mod preview_peer;"),
        "crates/daemon/src/lib.rs declares `pub mod preview_peer;`"
    );

    let dispatcher_full = read("crates/daemon/src/dispatcher.rs");
    let dispatcher = strip_comments(production_part(&dispatcher_full));
    let at = dispatcher
        .find("fn handle_preview_request(")
        .expect("dispatcher.rs has handle_preview_request");
    let body = block_from(&dispatcher, at);
    assert!(
        body.contains("has_local_seat_session("),
        "the preview path calls has_local_seat_session (LC-3a, spec §9.3)"
    );
    assert!(
        !body.contains("is_active_session("),
        "the preview path no longer calls is_active_session (LC-3a)"
    );

    let calls: Vec<String> = macro_invocations(&dispatcher, &TRACING_MACROS)
        .into_iter()
        .filter(|args| args.contains(FIRST_FRAME_MESSAGE))
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "dispatcher.rs logs the first-frame message exactly once (spec §9.3 step 6)"
    );
    assert_eq!(
        field_keys(&calls[0]),
        vec!["peer_uid".to_string()],
        "the first-frame line carries only peer_uid: {}",
        calls[0]
    );
    let info_calls = macro_invocations(&dispatcher, &["info", "tracing::info"]);
    assert!(
        info_calls.iter().any(|a| a.contains(FIRST_FRAME_MESSAGE)),
        "the first-frame line is logged at info level"
    );
}

// ---------------------------------------------------------------------------------------
// Test 57 — RLC-S8 documentation
// ---------------------------------------------------------------------------------------

#[test]
fn test_rlc_s8_camera_is_documented() {
    let doc = read("Docs/REMOTE_COMPANION.md");
    let section = markdown_section(&doc, "## 2f.")
        .expect("Docs/REMOTE_COMPANION.md has a `## 2f.` live camera section (spec §14 D-4)");
    for needle in [
        "camera_view",
        "camera_view_funnel",
        "camera_max_view_s",
        "camera_fps",
        "camera_width",
        "camera_quality",
        "remote_view",
        "allowed_uids",
        "local seat session",
        "Face ID",
        "LED",
        "no recording",
        "multipart/x-mixed-replace",
        "max_connections_per_uid",
        "connection_timeout_ms",
        "not a security boundary",
        "This software is based in part on the work of the Independent JPEG Group",
    ] {
        assert!(
            section.contains(needle),
            "Docs/REMOTE_COMPANION.md §2f must mention {needle}"
        );
    }
    let out_of_scope = markdown_section(&doc, "## 9.")
        .expect("Docs/REMOTE_COMPANION.md keeps its `## 9.` out-of-scope section");
    assert!(
        out_of_scope
            .lines()
            .any(|l| l.contains("live camera") && l.contains("recording")),
        "§9 keeps one line naming `live camera` and `recording` (spec S-4)"
    );

    let guidelines = read("Docs/SECURITY_AND_QUALITY_GUIDELINES.md");
    assert!(
        guidelines
            .lines()
            .any(|l| l.contains("jpeg-encoder") && l.contains("IJG")),
        "Docs/SECURITY_AND_QUALITY_GUIDELINES.md names the jpeg-encoder IJG exception (D-5)"
    );
    let daemon = read("Docs/DAEMON.md");
    assert!(
        daemon.contains("remote_view"),
        "Docs/DAEMON.md documents [preview] remote_view"
    );
    assert!(
        daemon.contains("local seat session"),
        "Docs/DAEMON.md documents the local seat session requirement"
    );
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("Live Camera View in `soos-remote` Through the Daemon Preview Channel"),
        "AI/DECISIONS.md registers the live camera ADR"
    );
}

// ---------------------------------------------------------------------------------------
// Test 58 — RLC-S9 installer template
// ---------------------------------------------------------------------------------------

#[test]
fn test_rlc_s9_installer_template() {
    let script = read("scripts/install_remote.sh");
    for needle in [
        "# camera_view = false",
        "# camera_view_funnel = false",
        "# camera_max_view_s = 120",
        "# camera_fps = 5",
        "# camera_width = 640",
        "# camera_quality = 70",
    ] {
        assert!(
            script.contains(needle),
            "scripts/install_remote.sh template must contain {needle}"
        );
    }
    let remote_view_lines: Vec<&str> = script
        .lines()
        .map(str::trim)
        .filter(|l| l.contains("remote_view = true"))
        .collect();
    assert!(
        !remote_view_lines.is_empty(),
        "scripts/install_remote.sh prints the daemon.toml `remote_view = true` snippet"
    );
    for line in &remote_view_lines {
        assert!(
            line.starts_with("echo"),
            "`remote_view = true` appears only in echo lines (never written): {line}"
        );
    }
    // Self-check of the detector (migration M11): every write form under /etc is caught, and
    // only the exact read-only resolver probe is exempt, so the scan below cannot pass vacuously.
    for sample in [
        "cat x > /etc/soos/daemon.toml",
        "printf '%s' x >> /etc/soos/daemon.toml",
        "echo remote_view = true >/etc/soos/daemon.toml",
        "echo x | tee /etc/soos/daemon.toml",
        "echo x | tee -a /etc/soos/daemon.toml",
        "cp daemon.toml /etc/soos/daemon.toml",
        "install -m 0644 daemon.toml /etc/soos/daemon.toml",
        "mv daemon.toml /etc/soos/daemon.toml",
        "ln -sf daemon.toml /etc/soos/daemon.toml",
        "sed -i 's/a/b/' /etc/soos/daemon.toml",
        "mkdir -p /etc/soos",
        "rm -f /etc/soos/daemon.toml",
        "chmod 0644 /etc/soos/daemon.toml",
        "chown root:root /etc/soos/daemon.toml",
        "target=\"$(readlink -f /etc/hosts 2>/dev/null || true)\"",
        "readlink -f /etc/resolv.conf > /etc/x",
    ] {
        assert!(
            installer_line_touches_etc(sample),
            "the /etc write detector must reject: {sample}"
        );
    }
    assert!(
        !installer_line_touches_etc(INSTALLER_RESOLVER_PROBE),
        "the read-only resolver probe is the single exempt line"
    );
    assert!(
        !installer_line_touches_etc("echo \"     /etc/soos/daemon.toml and restart soos-daemon\""),
        "an echo that only prints an /etc path is not a write"
    );
    for line in script.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        assert!(
            !line.contains("sudo"),
            "scripts/install_remote.sh never runs sudo: {line}"
        );
        assert!(
            !installer_line_touches_etc(line),
            "scripts/install_remote.sh never writes under /etc: {line}"
        );
    }
}

/// The single read-only `/etc` access `scripts/install_remote.sh` may perform
/// (`warn_resolver_stub`, already on `main`). Exempt by exact match only (migration M11).
const INSTALLER_RESOLVER_PROBE: &str =
    "target=\"$(readlink -f /etc/resolv.conf 2>/dev/null || true)\"";

/// True when a trimmed, non-comment installer line may write under `/etc`.
///
/// * Any line (an `echo` included) that redirects (`>`, `>>`) or pipes into `tee` towards an
///   `/etc` path is a write.
/// * Any non-`echo` line naming `/etc` is a write (this covers `cp`, `install`, `mv`, `ln`,
///   `sed -i`, `mkdir`, `rm`, `chmod`, `chown` and any other command), except the exact
///   read-only [`INSTALLER_RESOLVER_PROBE`].
fn installer_line_touches_etc(line: &str) -> bool {
    if !line.contains("/etc") {
        return false;
    }
    let compact: String = line
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect();
    let redirects_to_etc = compact.contains(">/etc");
    let tees_to_etc = line
        .split('|')
        .skip(1)
        .any(|stage| stage.trim_start().starts_with("tee") && stage.contains("/etc"));
    if redirects_to_etc || tees_to_etc {
        return true;
    }
    if line.starts_with("echo") {
        return false;
    }
    line != INSTALLER_RESOLVER_PROBE
}

// ---------------------------------------------------------------------------------------
// Test 59 — RLC-S10 no decoder, no device access
// ---------------------------------------------------------------------------------------

#[test]
fn test_rlc_s10_no_decoder_and_no_device_access_in_remote() {
    let files = rust_files("crates/remote/src");
    assert!(
        files
            .iter()
            .any(|(rel, _)| rel == "crates/remote/src/camera_jpeg.rs"),
        "crates/remote/src/camera_jpeg.rs must exist (spec S-1)"
    );
    for (rel, content) in files {
        let code = strip_comments(&content);
        for forbidden in [
            "jpeg_decoder",
            "zune",
            "image::",
            "/dev/video",
            "v4l",
            "Decoder",
        ] {
            assert!(
                !code.contains(forbidden),
                "{rel} must not contain {forbidden} (RLC-S1, RLC-S8)"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// Test 60 — owner request 2026-10-07: full screen, portrait/landscape, no cooldown (M13)
// ---------------------------------------------------------------------------------------

/// The live view toggles a full-screen stage by a tap (element fullscreen when the browser
/// has it, a fixed overlay otherwise), offers a visible exit control and the Escape key,
/// switches between exactly two modes, landscape and portrait (the image turned by 90°),
/// through a CSS class only (owner correction 2026-10-07: two modes, not four rotations),
/// always leaves full screen when
/// the view ends, and shows no cooldown anywhere (migration M13).
#[test]
fn test_rlc_page_fullscreen_rotate_and_no_cooldown() {
    let app = read("crates/remote/assets/app.js");
    for needle in [
        "requestFullscreen",
        "webkitRequestFullscreen",
        "exitFullscreen",
        "webkitExitFullscreen",
        "fullscreenchange",
        "webkitfullscreenchange",
        "\"Escape\"",
        "\"Exit full screen\"",
        "\"Portrait\"",
        "\"Landscape\"",
        "\"Switch to portrait mode\"",
        "\"Switch to landscape mode\"",
        "\"aria-label\"",
        "\"camera-full\"",
        "\"camera-portrait\"",
        "classList",
        "function enterCameraFullscreen(",
        "function exitCameraFullscreen(",
        "function toggleCameraOrientation(",
    ] {
        assert!(
            app.contains(needle),
            "crates/remote/assets/app.js must contain {needle} (owner request 2026-10-07)"
        );
    }
    for forbidden in [
        "camera-rot-",
        "rotate(180deg)",
        "rotate(270deg)",
        "\"Rotate\"",
    ] {
        assert!(
            !app.contains(forbidden),
            "app.js offers two modes only, never {forbidden} (owner correction 2026-10-07)"
        );
    }
    for forbidden in [".style.", "cssText", "setAttribute(\"style\"", "\"style\""] {
        assert!(
            !app.contains(forbidden),
            "app.js changes the view through classList only, never {forbidden} (CSP unchanged)"
        );
    }
    let lower = app.to_ascii_lowercase();
    for forbidden in ["cooldown", "next view possible", "retry_after_ms"] {
        assert!(
            !lower.contains(forbidden),
            "app.js must not mention {forbidden} (M13: no cooldown between views)"
        );
    }
    let end_at = app
        .find("function endCameraView(")
        .expect("app.js defines endCameraView");
    assert!(
        block_from(&app, end_at).contains("exitCameraFullscreen("),
        "every end of the view (stop, max duration, error, pagehide) leaves full screen"
    );
    let canvas_at = app
        .find("function buildCameraCard(")
        .expect("app.js defines buildCameraCard");
    let card = block_from(&app, canvas_at);
    assert!(
        card.contains("toggleCameraFullscreen"),
        "a tap on the live image toggles full screen"
    );

    let css = read("crates/remote/assets/style.css");
    for needle in [
        ".camera-stage",
        ".camera-full",
        "position: fixed",
        "object-fit: contain",
        ".camera-portrait",
        "rotate(90deg)",
    ] {
        assert!(
            css.contains(needle),
            "crates/remote/assets/style.css must contain {needle}"
        );
    }
    for forbidden in ["camera-rot-", "rotate(180deg)", "rotate(270deg)"] {
        assert!(
            !css.contains(forbidden),
            "style.css offers two modes only, never {forbidden}"
        );
    }
    let full_at = css
        .find("\n.camera-stage.camera-full {")
        .expect("style.css has a top-level `.camera-stage.camera-full` rule");
    let full = block_from(&css, full_at);
    for needle in ["position: fixed", "inset: 0", "env(safe-area-inset-"] {
        assert!(
            full.contains(needle),
            "the full-screen stage rule must contain {needle}: {full}"
        );
    }

    let lib = strip_comments(&read("crates/remote/src/lib.rs"));
    assert!(
        !lib.contains("CAMERA_VIEW_COOLDOWN_MS"),
        "M13: CAMERA_VIEW_COOLDOWN_MS is removed"
    );
    for (rel, code) in camera_sources().into_iter().chain([(
        "crates/remote/src/server.rs".to_string(),
        strip_comments(&read("crates/remote/src/server.rs")),
    )]) {
        let lower = code.to_ascii_lowercase();
        for forbidden in ["cooldown", "retry_after_ms"] {
            assert!(
                !lower.contains(forbidden),
                "{rel} must not contain {forbidden} (M13: no cooldown between views)"
            );
        }
    }

    let doc = read("Docs/REMOTE_COMPANION.md");
    let section = markdown_section(&doc, "## 2f.").expect("Docs/REMOTE_COMPANION.md §2f");
    for needle in ["full screen", "portrait", "landscape", "no cooldown"] {
        assert!(
            section.contains(needle),
            "Docs/REMOTE_COMPANION.md §2f must mention {needle}"
        );
    }
    assert!(
        !doc.contains("camera_cooldown"),
        "Docs/REMOTE_COMPANION.md no longer documents camera_cooldown"
    );
    let contract = read("AI/tester_contract_remote_live_camera.md");
    assert!(
        contract.contains("M13"),
        "AI/tester_contract_remote_live_camera.md records migration M13"
    );
}

/// Needles of the removed camera-view Web Push (M14).
const CAMERA_PUSH_NEEDLES: [&str; 5] = [
    "sooscamera",
    "queue_camera_view",
    "camera_payload",
    "send_camera",
    "PUSH_CAMERA_TOPIC",
];

/// M14 (owner request 2026-10-07, matrix RLC15): starting a live camera view sends no Web
/// Push. No source file of `crates/remote/src` (comments included) names the removed
/// camera notification, the service worker has no camera notification kind, and the tester
/// contract records the migration.
#[test]
fn test_remote_camera_sends_no_push() {
    let files = rust_files("crates/remote/src");
    assert!(
        files.iter().any(|(rel, _)| rel.ends_with("push.rs")),
        "crates/remote/src must be scanned (push.rs found)"
    );
    for (rel, content) in &files {
        for needle in CAMERA_PUSH_NEEDLES {
            assert!(
                !content.contains(needle),
                "{rel} must not contain {needle} (M14: no camera-view Web Push)"
            );
        }
    }
    let push = strip_comments(&read("crates/remote/src/push.rs"));
    assert!(
        !push.contains("kind: \"camera\""),
        "push.rs renders no camera payload kind (M14)"
    );
    let sw = read("crates/remote/assets/sw.js");
    for needle in ["\"camera\"", "soos-camera"] {
        assert!(
            !sw.contains(needle),
            "sw.js has no camera notification ({needle}, M14)"
        );
    }
    let contract = read("AI/tester_contract_remote_live_camera.md");
    assert!(
        contract.contains("M14"),
        "AI/tester_contract_remote_live_camera.md records migration M14"
    );
}
