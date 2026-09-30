//! Camera documentation drift invariants (GitHub #197, review finding CAM-15).
//!
//! `Docs/CAMERA_V4L_CRATE.md` used to state defaults that the code did not have
//! (`auto_format` false instead of true, `idle_timeout` 60 s instead of 10 s) and cited
//! test names that did not exist. These checks tie the page to the code:
//!
//! - every `(default: ...)` in the `CameraConfig` field list equals the value in
//!   `impl Default for CameraConfig` (`crates/camera-v4l/src/config.rs`), and every field of
//!   that impl except `device_path` (documented by the resolver section) has a documented default;
//! - every `module::test_*` / `module::*` citation of the crate page resolves to a real test
//!   function / test file under `crates/`;
//! - while `soos-daemon` resets `warmup_frames` to 0 for a `daemon.toml` `[pipeline]` section
//!   without the key (matrix CLP2), the page says so next to Criterion C5;
//! - `Docs/ENROLLMENT_CLI.md` documents the shared resolver and the `/dev/videoN` fallback
//!   instead of an unconditional by-id claim.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const CAMERA_DOC: &str = "Docs/CAMERA_V4L_CRATE.md";
const ENROLL_DOC: &str = "Docs/ENROLLMENT_CLI.md";
const CAMERA_CONFIG_RS: &str = "crates/camera-v4l/src/config.rs";
const DAEMON_CONFIG_RS: &str = "crates/daemon/src/config.rs";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Normalizes a Rust default expression into the notation used by the documentation.
fn normalize_code_value(expr: &str) -> String {
    let expr = expr.trim().trim_end_matches(',').trim();
    if let Some(n) = expr
        .strip_prefix("Duration::from_secs(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return format!("{n}s");
    }
    if let Some(n) = expr
        .strip_prefix("Duration::from_millis(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return format!("{n}ms");
    }
    // `PixelFormat::Yuyv` -> `Yuyv`, `SensorPreference::PreferIr` -> `PreferIr`.
    expr.rsplit("::").next().unwrap_or(expr).to_string()
}

/// Parses `field: value,` lines of `impl Default for CameraConfig`.
fn code_defaults(source: &str) -> BTreeMap<String, String> {
    let start = source
        .find("impl Default for CameraConfig")
        .expect("impl Default for CameraConfig");
    let body = &source[start..];
    let open = body
        .find("Self {")
        .expect("Self { in CameraConfig::default")
        + "Self {".len();
    let close = body[open..].find('}').expect("end of Self {") + open;
    body[open..close]
        .lines()
        .filter_map(|line| {
            let (name, value) = line.trim().split_once(':')?;
            let name = name.trim();
            (!name.is_empty() && !name.starts_with("//"))
                .then(|| (name.to_string(), normalize_code_value(value)))
        })
        .collect()
}

/// Parses `- \`a\` & \`b\`: text (default: X)` lines of the documentation.
fn documented_defaults(doc: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in doc.lines() {
        let Some(rest) = line.trim().strip_prefix("- `") else {
            continue;
        };
        let Some(idx) = rest.find("(default: ") else {
            continue;
        };
        let value = rest[idx + "(default: ".len()..]
            .split(')')
            .next()
            .unwrap_or_default()
            .replace('`', "");
        let Some((names, _)) = rest.split_once(':') else {
            continue;
        };
        let names: Vec<String> = names
            .split('&')
            .map(|n| n.trim().trim_matches('`').to_string())
            .collect();
        let values: Vec<String> = if names.len() == 2 {
            value
                .split(['×', 'x'])
                .flat_map(|v| v.split(" to "))
                .map(|v| v.trim().to_string())
                .collect()
        } else {
            vec![value.trim().to_string()]
        };
        assert_eq!(
            names.len(),
            values.len(),
            "cannot pair documented names {names:?} with default {value:?}"
        );
        for (name, value) in names.into_iter().zip(values) {
            out.insert(name, value);
        }
    }
    out
}

/// Invariant CHT7: the documented `CameraConfig` defaults are the code defaults.
#[test]
fn test_camera_doc_defaults_match_camera_config_default() {
    let code = code_defaults(&read(CAMERA_CONFIG_RS));
    assert!(code.len() >= 10, "parsed only {code:?}");
    assert_eq!(code.get("auto_format").map(String::as_str), Some("true"));

    let doc = documented_defaults(&read(CAMERA_DOC));
    let mut failures = Vec::new();
    for (field, value) in &code {
        if field == "device_path" {
            continue;
        }
        match doc.get(field) {
            Some(documented) if documented == value => {}
            Some(documented) => failures.push(format!(
                "`{field}`: documented default `{documented}`, code default `{value}`"
            )),
            None => failures.push(format!("`{field}`: no documented default (code `{value}`)")),
        }
    }
    for field in doc.keys() {
        if !code.contains_key(field) {
            failures.push(format!(
                "`{field}`: documented but not a CameraConfig field"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{CAMERA_DOC} drifted from {CAMERA_CONFIG_RS}:\n{}",
        failures.join("\n")
    );
}

/// Collects every `.rs` file under `dir` (skipping build output).
fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Returns the backticked `module::name` citations of a document.
fn module_citations(doc: &str) -> Vec<(String, String)> {
    doc.split('`')
        .skip(1)
        .step_by(2)
        .filter_map(|span| {
            let (module, name) = span.split_once("::")?;
            let valid_module =
                !module.is_empty() && module.chars().all(|c| c.is_ascii_lowercase() || c == '_');
            (valid_module && (name == "*" || name.starts_with("test_")))
                .then(|| (module.to_string(), name.to_string()))
        })
        .collect()
}

/// Invariant CHT7: every test citation of the camera crate page resolves.
#[test]
fn test_camera_doc_test_citations_resolve() {
    let root = workspace_root();
    let mut files = Vec::new();
    rs_files(&root.join("crates"), &mut files);
    let doc = read(CAMERA_DOC);
    let cited = module_citations(&doc);
    assert!(cited.len() >= 12, "parsed only {cited:?}");

    let mut failures = Vec::new();
    for (module, name) in &cited {
        let candidates: Vec<String> = files
            .iter()
            .filter(|p| p.file_stem().is_some_and(|s| s == module.as_str()))
            .filter_map(|p| fs::read_to_string(p).ok())
            .collect();
        let found = if name == "*" {
            candidates.iter().any(|text| text.contains("#[test]"))
        } else {
            let needle = format!("fn {name}(");
            candidates.iter().any(|text| text.contains(&needle))
        };
        if !found {
            failures.push(format!("`{module}::{name}`"));
        }
    }
    assert!(
        failures.is_empty(),
        "{CAMERA_DOC} cites tests that do not exist: {}",
        failures.join(", ")
    );
}

/// Invariant CHT7: Criterion C5 on the crate page states the daemon.toml warmup behaviour.
#[test]
fn test_camera_doc_states_daemon_toml_warmup_default() {
    let daemon = read(DAEMON_CONFIG_RS);
    let doc = read(CAMERA_DOC);
    if daemon.contains("warmup_frames.unwrap_or(0)") {
        let c5 = doc
            .lines()
            .find(|l| l.starts_with("| **C5**"))
            .expect("C5 row in the crate page");
        assert!(
            c5.contains("daemon.toml") && c5.contains("0"),
            "C5 must state that a daemon.toml [pipeline] section without `warmup_frames` \
             discards 0 frames (matrix CLP2): {c5}"
        );
    }
}

/// Invariant CHT7: the enrollment CLI page documents the shared resolver honestly.
#[test]
fn test_enrollment_doc_describes_shared_camera_resolver() {
    let doc = read(ENROLL_DOC);
    for needle in [
        "resolve_camera_device",
        "sensor_preference",
        "/dev/videoN",
        "--camera-device",
    ] {
        assert!(
            doc.contains(needle),
            "{ENROLL_DOC} must document camera resolution (`{needle}` missing)"
        );
    }
}

#[test]
fn test_camera_doc_parsers_self_test() {
    assert_eq!(normalize_code_value("Duration::from_secs(10),"), "10s");
    assert_eq!(normalize_code_value("Duration::from_millis(100),"), "100ms");
    assert_eq!(
        normalize_code_value("SensorPreference::PreferIr,"),
        "PreferIr"
    );
    let doc = "- `width` & `height`: Frame resolution (default: 640×480)\n\
               - `min_backoff` & `max_backoff`: limits (default: 100ms to 5s)\n\
               - `format`: Pixel format (default: `Yuyv`)\n";
    let parsed = documented_defaults(doc);
    assert_eq!(parsed["width"], "640");
    assert_eq!(parsed["height"], "480");
    assert_eq!(parsed["min_backoff"], "100ms");
    assert_eq!(parsed["max_backoff"], "5s");
    assert_eq!(parsed["format"], "Yuyv");
    assert_eq!(
        module_citations("`warmup_tests::test_a` and `resolver_tests::*` and `Foo::bar`"),
        vec![
            ("warmup_tests".to_string(), "test_a".to_string()),
            ("resolver_tests".to_string(), "*".to_string())
        ]
    );
}
