//! Project licence contract (user decision 2026-09-30, matrix rows LIC1–LIC3).
//!
//! The project licence is `AGPL-3.0-or-later`. The `license` key of `[workspace.package]` in the
//! root `Cargo.toml` is the source of truth; every workspace member inherits it, the top-level
//! `LICENSE` file carries the AGPL-3.0 text and the README states the same expression.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contract tests utilize direct assertions and panics"
)]

use std::fs;
use std::path::{Path, PathBuf};

/// SPDX expression of the project licence (user decision 2026-09-30).
const PROJECT_LICENSE: &str = "AGPL-3.0-or-later";

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

/// Workspace member directories listed in the `members = [...]` array of the root manifest.
fn workspace_members(manifest: &str) -> Vec<String> {
    let (_, rest) = manifest
        .split_once("members = [")
        .expect("root Cargo.toml declares workspace members");
    let (list, _) = rest.split_once(']').expect("members array is closed");
    list.split(',')
        .map(|m| m.trim().trim_matches('"').to_string())
        .filter(|m| !m.is_empty() && !m.starts_with('#'))
        .collect()
}

#[test]
fn test_workspace_license_is_agpl_and_inherited_by_every_member() {
    let manifest = read("Cargo.toml");
    let package = manifest
        .split("[workspace.package]")
        .nth(1)
        .expect("root Cargo.toml has [workspace.package]");
    let package = package.split("\n[").next().unwrap_or(package);
    assert!(
        package
            .lines()
            .any(|l| l.trim() == format!("license = \"{PROJECT_LICENSE}\"")),
        "[workspace.package] must declare license = \"{PROJECT_LICENSE}\""
    );
    let members = workspace_members(&manifest);
    assert!(!members.is_empty(), "no workspace member parsed");
    for member in members {
        let text = read(&format!("{member}/Cargo.toml"));
        assert!(
            text.lines().any(|l| l.trim() == "license.workspace = true"),
            "{member}/Cargo.toml must inherit the workspace licence (license.workspace = true)"
        );
        assert!(
            !text
                .lines()
                .any(|l| l.trim_start().starts_with("license =")),
            "{member}/Cargo.toml must not override the workspace licence"
        );
    }
}

#[test]
fn test_top_level_license_file_carries_the_agpl_v3_text() {
    let text = read("LICENSE");
    assert!(
        text.starts_with("GNU AFFERO GENERAL PUBLIC LICENSE"),
        "LICENSE must start with the AGPL title"
    );
    for needle in [
        "Version 3, 19 November 2007",
        "13. Remote Network Interaction; Use with the GNU General Public License.",
        "END OF TERMS AND CONDITIONS",
    ] {
        assert!(text.contains(needle), "LICENSE must contain {needle:?}");
    }
}

#[test]
fn test_readme_and_packages_state_the_project_license() {
    let readme = read("README.md");
    assert!(
        readme.contains("## License") && readme.contains(PROJECT_LICENSE),
        "README.md must have a License section naming {PROJECT_LICENSE}"
    );
    assert!(
        read("packaging/rpm/soos.spec")
            .lines()
            .any(|l| l.starts_with("License:") && l.trim_end().ends_with(PROJECT_LICENSE)),
        "the RPM spec License: must be {PROJECT_LICENSE}"
    );
    assert!(
        read("packaging/arch/PKGBUILD").contains(&format!("license=('{PROJECT_LICENSE}')")),
        "the PKGBUILD license=() must be {PROJECT_LICENSE}"
    );
}
