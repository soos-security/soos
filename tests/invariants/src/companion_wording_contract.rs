//! Companion off-switch and opt-in wording contract (GitHub #423, part of #362; follow-up to
//! the #409 candid review S-1, S-2 and S-4; architect spec
//! `AI/architect_spec_companion_wording.md` §3; matrix CWD1–CWD5):
//!
//! - `scripts/setup.sh` never says every companion feature can be turned off in `soos-gui`:
//!   `soos-gui` has no switch for the failed-password alerts, so the help and the closing
//!   summary name `--password-alerts off` and the `remote.toml` keys instead;
//! - the premise holds: `RemoteSwitch::key` has no `password_alerts` arm;
//! - `Docs/REMOTE_COMPANION.md` and `AI/ARCHITECTURE.md` no longer call `allow_funnel`,
//!   `allow_unlock` or the push sender opt-in, since `scripts/setup.sh` turns them on, and
//!   `Docs/PACKAGING_AND_PROVISIONING.md` names the failed-password alerts exception;
//! - the README security section names the push and offline-alert switches of `soos-gui`.
//!
//! std only.

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

const SETUP: &str = "scripts/setup.sh";
const FEATURE_CONTROL: &str = "crates/gui/src/feature_control.rs";
const REMOTE_DOC: &str = "Docs/REMOTE_COMPANION.md";
const PACKAGING: &str = "Docs/PACKAGING_AND_PROVISIONING.md";
const ARCHITECTURE: &str = "AI/ARCHITECTURE.md";
const README: &str = "README.md";

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

/// Whitespace collapsed to single spaces.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Sentence-like units of collapsed text: split on ". " and ";".
fn units(text: &str) -> Vec<String> {
    collapse(text)
        .split(". ")
        .flat_map(|s| s.split(';'))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Whether a unit says every companion feature can be turned off in `soos-gui`
/// ("every feature" and "soos-gui" in one unit, without an exception).
fn claims_every_feature_has_gui_switch(unit: &str) -> bool {
    let lower = unit.to_ascii_lowercase();
    lower.contains("every feature")
        && lower.contains("soos-gui")
        && !lower.contains("except")
        && !lower.contains("no switch")
}

/// The `usage()` heredoc of `setup.sh`.
fn usage_block(setup: &str) -> String {
    let start = setup
        .find("usage() {")
        .expect("setup.sh must define usage()");
    let rest = &setup[start..];
    let end = rest
        .find("\nEOF\n")
        .expect("usage() must end its heredoc with EOF");
    rest[..end].to_string()
}

/// `setup.sh` without full-line comments.
fn setup_code(setup: &str) -> String {
    setup
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// CWD1: `setup.sh` never says every feature can be turned off in `soos-gui`; its help and
/// closing summary name the password-alerts path.
#[test]
fn test_cwd_setup_names_the_password_alerts_off_path() {
    let setup = read(SETUP);
    let code = setup_code(&setup);
    let claims: Vec<String> = units(&code)
        .into_iter()
        .filter(|u| claims_every_feature_has_gui_switch(u))
        .collect();
    assert!(
        claims.is_empty(),
        "{SETUP} must not say every feature can be turned off in soos-gui (no password_alerts switch): {claims:#?}"
    );
    let usage = collapse(&usage_block(&setup));
    for needle in [
        "soos-gui (Features)",
        "--password-alerts off",
        "password_alerts, push_notifications and offline_alerts",
        "remote.toml",
    ] {
        assert!(
            usage.contains(needle),
            "{SETUP} --help must mention `{needle}` where it explains turning features off"
        );
    }
    assert!(
        code.lines().any(|l| l.contains("info ")
            && l.contains("soos-gui (Features)")
            && l.contains("--password-alerts off")
            && l.contains("install_remote.sh --uninstall")),
        "the closing companion summary of {SETUP} must name the soos-gui switches, --password-alerts off and the uninstall command"
    );
}

/// CWD2: the premise of CWD1: `soos-gui` has no `password_alerts` switch. Adding one must
/// update the setup help and this row together.
#[test]
fn test_cwd_gui_has_no_password_alerts_switch() {
    let source = read(FEATURE_CONTROL);
    let start = source
        .find("pub fn key(self) -> &'static str {")
        .expect("RemoteSwitch::key must exist");
    let body = &source[start..];
    let end = body.find("\n    }\n").expect("end of RemoteSwitch::key");
    let body = &body[..end];
    assert!(
        body.contains("\"allow_funnel\"") && body.contains("\"offline_alerts\""),
        "RemoteSwitch::key was not found where expected"
    );
    assert!(
        !body.contains("password_alerts"),
        "RemoteSwitch now has a password_alerts switch: update the {SETUP} help, the closing summary and CWD1"
    );
}

/// CWD3: the companion docs no longer call setup-enabled keys opt-in, and the packaging doc
/// names the failed-password alerts exception to the soos-gui switches.
#[test]
fn test_cwd_docs_do_not_call_setup_enabled_keys_opt_in() {
    let doc = read(REMOTE_DOC);
    let examples: Vec<&str> = doc
        .lines()
        .filter(|l| l.trim_start().starts_with("allow_funnel = true"))
        .collect();
    assert!(
        !examples.is_empty(),
        "{REMOTE_DOC} must keep its `allow_funnel = true` example"
    );
    for line in &examples {
        assert!(
            !line.to_ascii_lowercase().contains("opt-in"),
            "{REMOTE_DOC}: setup writes allow_funnel = true, so the example must not call it opt-in: {line}"
        );
        if line.contains('#') {
            assert!(
                line.contains("scripts/setup.sh") && line.contains("403"),
                "{REMOTE_DOC}: the commented example must state the code default (403) and that scripts/setup.sh writes true: {line}"
            );
        }
    }
    let packaging = collapse(&read(PACKAGING));
    assert!(
        !packaging.contains("each switchable off later in soos-gui"),
        "{PACKAGING}: soos-gui has no password_alerts switch, so not every companion feature is switchable off there"
    );
    assert!(
        packaging.contains("except the failed-password alerts")
            && packaging.contains("`--password-alerts off`"),
        "{PACKAGING}: the setup defaults must name the failed-password alerts exception and `--password-alerts off`"
    );
    let arch = read(ARCHITECTURE);
    for header in ["Push sender", "Unlock"] {
        let rows: Vec<&str> = arch
            .lines()
            .filter(|l| l.starts_with(&format!("| {header}")))
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "{ARCHITECTURE} must have exactly one `{header}` row in the companion table"
        );
        let cell = rows[0].split('|').nth(1).expect("header cell");
        assert!(
            !cell.to_ascii_lowercase().contains("opt-in"),
            "{ARCHITECTURE}: the `{header}` row header must not say opt-in: {cell}"
        );
        assert!(
            cell.contains("scripts/setup.sh") && cell.contains("off by default in code"),
            "{ARCHITECTURE}: the `{header}` row header must say off by default in code and on with scripts/setup.sh: {cell}"
        );
    }
    let unlock = arch
        .lines()
        .find(|l| l.starts_with("| Unlock"))
        .expect("Unlock row");
    assert!(
        unlock.contains("(default `false`"),
        "{ARCHITECTURE}: the Unlock row must keep its default `false` statement"
    );
}

/// CWD4: the README security section names the push and offline-alert switches.
#[test]
fn test_cwd_readme_names_push_and_offline_switches() {
    let readme = read(README);
    let start = readme
        .find("\n## Security")
        .expect("README.md must have a Security section");
    let rest = &readme[start + 1..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    let section = &rest[..end];
    let switches: Vec<String> = units(section)
        .into_iter()
        .filter(|u| u.contains("off switch"))
        .collect();
    assert!(
        !switches.is_empty(),
        "the README security section must name the soos-gui off switches"
    );
    for unit in &switches {
        let lower = unit.to_ascii_lowercase();
        assert!(
            lower.contains("push") && lower.contains("offline alerts"),
            "the README off-switch sentence must also name push and offline alerts: {unit}"
        );
    }
}

/// CWD5: the CWD1 detector and unit splitter on synthetic input.
#[test]
fn test_cwd_detector_helpers() {
    assert!(claims_every_feature_has_gui_switch(
        "Every feature can be turned off later in soos-gui (Features)"
    ));
    assert!(claims_every_feature_has_gui_switch(
        "Every feature below is on by default (owner decision) and can be turned off later in soos-gui (Features)"
    ));
    assert!(!claims_every_feature_has_gui_switch(
        "Every feature except the failed-password alerts has a switch in soos-gui"
    ));
    assert!(!claims_every_feature_has_gui_switch(
        "the failed-password alerts have no switch in soos-gui; every feature is on"
    ));
    assert!(!claims_every_feature_has_gui_switch(
        "Every feature below is on by default"
    ));
    let split = units("Every feature is on.  Funnel has a switch in\nsoos-gui; done");
    assert_eq!(
        split,
        vec![
            "Every feature is on".to_string(),
            "Funnel has a switch in soos-gui".to_string(),
            "done".to_string()
        ]
    );
    let old = "Every feature below is on\nby default (owner decision) and can be turned off later in soos-gui (Features); a\nfeature already turned off";
    assert!(units(old)
        .iter()
        .any(|u| claims_every_feature_has_gui_switch(u)));
}
