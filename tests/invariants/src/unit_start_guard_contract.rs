//! First-start guards of `soos-daemon.service` evaluated by systemd itself (GitHub #211,
//! ONB-15; matrix rows IRP6–IRP7).
//!
//! systemd ignores a malformed `Condition*=` line with a warning and then starts the unit
//! anyway, so a textual check alone cannot prove the guard works. These tests hand every
//! `ConditionPathExists=` of the shipped unit to `systemd-analyze condition` (with the path
//! re-rooted into a scratch directory) and assert "condition failed" without the file and
//! success with it. They also pin the design decision that only the models manifest gates the
//! start: the configuration file is optional and `master.key` is created on first start.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stderr,
    reason = "Contract tests utilize direct assertions, panics, indexing and a skip notice"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::installer_contract::scratch_dir;

const UNIT: &str = "packaging/soos-daemon.service";
const MODELS_MANIFEST: &str = "/var/lib/soos/models/manifest.toml";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// `key=value` lines of the `[Unit]` section, comments excluded.
fn unit_section_entries() -> Vec<(String, String)> {
    let text = fs::read_to_string(workspace_root().join(UNIT)).expect("read unit");
    let mut in_unit = false;
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_unit = line == "[Unit]";
            continue;
        }
        if !in_unit || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            entries.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    entries
}

fn analyze_condition(condition: &str) -> Option<Output> {
    Command::new("systemd-analyze")
        .arg("condition")
        .arg(condition)
        .output()
        .ok()
}

/// True when `systemd-analyze condition` evaluates conditions in this environment.
fn systemd_analyze_usable() -> bool {
    analyze_condition("ConditionPathExists=/").is_some_and(|o| o.status.success())
        && analyze_condition("ConditionPathExists=/nonexistent/soos/probe")
            .is_some_and(|o| !o.status.success())
}

/// IRP6: the models manifest is the only path that gates the start; the optional
/// configuration file and the auto-created master key never do.
#[test]
fn test_daemon_unit_path_conditions_gate_only_on_models_manifest() {
    let paths: Vec<String> = unit_section_entries()
        .into_iter()
        .filter(|(k, _)| k.starts_with("Condition") || k.starts_with("Assert"))
        .filter(|(k, _)| k.contains("Path") || k.contains("File") || k.contains("Directory"))
        .map(|(_, v)| v)
        .collect();
    assert_eq!(
        paths,
        vec![MODELS_MANIFEST.to_string()],
        "only ConditionPathExists={MODELS_MANIFEST} may gate the start (daemon.toml is optional, \
         master.key is created on first start)"
    );
}

/// IRP7: systemd itself evaluates every `ConditionPathExists=` of the unit as failed when the
/// file is absent and as satisfied once it exists (skips when `systemd-analyze` is unusable).
#[test]
fn test_daemon_unit_conditions_evaluate_as_condition_failed_without_models() {
    if !systemd_analyze_usable() {
        eprintln!("skipping: systemd-analyze condition is not usable in this environment");
        return;
    }
    let conditions: Vec<String> = unit_section_entries()
        .into_iter()
        .filter(|(k, _)| k == "ConditionPathExists")
        .map(|(_, v)| v)
        .collect();
    assert!(
        !conditions.is_empty(),
        "{UNIT} must carry ConditionPathExists="
    );
    let root = scratch_dir("unit_conditions");
    for value in conditions {
        let rel = value
            .strip_prefix('/')
            .unwrap_or_else(|| panic!("ConditionPathExists={value} must be an absolute path"));
        let rerooted = root.join(rel);
        let condition = format!("ConditionPathExists={}", rerooted.display());

        let absent = analyze_condition(&condition).expect("systemd-analyze");
        assert!(
            !absent.status.success(),
            "{condition} must fail without the file; output:\n{}",
            String::from_utf8_lossy(&absent.stderr)
        );

        fs::create_dir_all(rerooted.parent().expect("parent")).expect("mkdir");
        fs::write(&rerooted, b"").expect("create file");
        let present = analyze_condition(&condition).expect("systemd-analyze");
        assert!(
            present.status.success(),
            "{condition} must succeed with the file; output:\n{}",
            String::from_utf8_lossy(&present.stderr)
        );
    }
    let _ = fs::remove_dir_all(&root);
}
