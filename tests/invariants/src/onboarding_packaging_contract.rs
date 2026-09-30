//! Onboarding documentation and packaging metadata contract (GitHub #207 ONB-11, #210 ONB-14).
//!
//! - #207: every `soos-enroll` / `soos-admin` command shown to a user (README, `Docs/`,
//!   `models/README.md`, installer and packaging scripts) names a real subcommand derived from
//!   the clap definitions, template file names are `<uid>.cbor.enc` (never `.bin`), the
//!   installer epilogue prints the real enrollment command, and the README carries a quick start
//!   covering install, enrollment, verification, rescue and uninstall.
//! - #210: the package license and version come from `[workspace.package]` in the root
//!   `Cargo.toml` (through `scripts/lib/pkg_meta.sh` for the build scripts), the RPM and Arch
//!   recipes ship `soos-gui` like the `.deb`, and every packaging cargo invocation is `--locked`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Shared packaging metadata helper sourced by the build scripts.
const PKG_META: &str = "scripts/lib/pkg_meta.sh";

/// Package build scripts that must derive version and license from `Cargo.toml`.
const BUILD_SCRIPTS: [&str; 3] = [
    "scripts/build_deb.sh",
    "scripts/build_rpm.sh",
    "scripts/build_arch.sh",
];

/// Packaging recipes and scripts whose cargo invocations must be reproducible.
const PACKAGING_BUILD_FILES: [&str; 7] = [
    "packaging/debian/rules",
    "packaging/rpm/soos.spec",
    "packaging/arch/PKGBUILD",
    "scripts/build_deb.sh",
    "scripts/build_rpm.sh",
    "scripts/build_arch.sh",
    "scripts/build_packages.sh",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Value of `key` inside the `[workspace.package]` table of a Cargo manifest.
fn workspace_package_value(manifest: &str, key: &str) -> String {
    let mut in_table = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_table = trimmed == "[workspace.package]";
            continue;
        }
        if !in_table {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            if k.trim() == key {
                return v.trim().trim_matches('"').to_string();
            }
        }
    }
    panic!("[workspace.package] has no `{key}`");
}

fn cargo_license() -> String {
    workspace_package_value(&read("Cargo.toml"), "license")
}

fn cargo_version() -> String {
    workspace_package_value(&read("Cargo.toml"), "version")
}

// ---------------------------------------------------------------------------
// #207 — onboarding commands
// ---------------------------------------------------------------------------

fn kebab_variant(variant: &str) -> String {
    let mut out = String::new();
    for (i, c) in variant.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A CLI as shown in documentation: its subcommands and its switch-only global flags.
struct CliShape {
    subcommands: BTreeSet<String>,
    /// Global flags of `Cli` that take no value (`bool` fields).
    switches: BTreeSet<String>,
}

/// Parses `pub enum Commands` variants and the `bool` fields of `pub struct Cli`.
fn cli_shape(args_rs: &str) -> CliShape {
    let source = read(args_rs);
    let mut subcommands = BTreeSet::new();
    let mut switches = BTreeSet::new();
    let mut block: Option<&str> = None;
    for line in source.lines() {
        let t = line.trim();
        if t.starts_with("pub enum Commands") {
            block = Some("enum");
            continue;
        }
        if t.starts_with("pub struct Cli ") || t == "pub struct Cli {" {
            block = Some("cli");
            continue;
        }
        if t == "}" {
            block = None;
            continue;
        }
        match block {
            Some("enum") => {
                if t.starts_with("///") || t.starts_with("#[") || t.is_empty() {
                    continue;
                }
                let name: String = t
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect();
                if !name.is_empty() {
                    subcommands.insert(kebab_variant(&name));
                }
            }
            Some("cli") => {
                if let Some(rest) = t.strip_prefix("pub ") {
                    if let Some((field, ty)) = rest.split_once(':') {
                        if ty.trim().trim_end_matches(',') == "bool" {
                            switches.insert(format!("--{}", field.trim().replace('_', "-")));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    assert!(
        subcommands.len() >= 4,
        "{args_rs}: parsed too few subcommands ({subcommands:?}); parser out of date"
    );
    CliShape {
        subcommands,
        switches,
    }
}

/// User-facing onboarding files (historical `AI/` walkthroughs are excluded on purpose).
fn onboarding_files() -> Vec<String> {
    let root = workspace_root();
    let mut files = vec![
        "README.md".to_string(),
        "models/README.md".to_string(),
        "scripts/install.sh".to_string(),
        "scripts/uninstall.sh".to_string(),
        "packaging/debian/postinst".to_string(),
        "packaging/arch/soos.install".to_string(),
        "packaging/rpm/soos.spec".to_string(),
    ];
    let mut docs: Vec<String> = fs::read_dir(root.join("Docs"))
        .expect("read Docs/")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .map(|p| {
            p.strip_prefix(&root)
                .expect("under root")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    docs.sort();
    files.extend(docs);
    files
}

fn tokenize(rest: &str) -> Vec<String> {
    rest.split_whitespace()
        .map(|t| {
            t.trim_matches(|c| c == '`' || c == '"' || c == '\'')
                .to_string()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// Returns an error text when the tokens after the binary name do not start with a subcommand.
fn check_invocation(shape: &CliShape, tokens: &[String]) -> Option<String> {
    let mut i = 0;
    while let Some(tok) = tokens.get(i) {
        if tok == "-h" || tok == "--help" || tok == "-V" || tok == "--version" {
            return None;
        }
        if tok.starts_with('-') {
            let takes_value = !tok.contains('=') && !shape.switches.contains(tok.as_str());
            i += if takes_value { 2 } else { 1 };
            continue;
        }
        if shape.subcommands.contains(tok.as_str()) {
            return None;
        }
        return Some(format!(
            "`{tok}` is not a subcommand (expected one of {:?})",
            shape.subcommands
        ));
    }
    Some("no subcommand".to_string())
}

/// Finds every command invocation of `bin` in `text`: after `sudo ` / `pkexec `, or at the start
/// of a line inside a fenced code block.
fn invocations<'a>(text: &'a str, bin: &str) -> Vec<(usize, &'a str)> {
    let mut found = Vec::new();
    let mut in_fence = false;
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        for prefix in ["sudo ", "pkexec "] {
            let needle = format!("{prefix}{bin}");
            let mut from = 0;
            while let Some(pos) = line[from..].find(&needle) {
                let start = from + pos + needle.len();
                if line[start..].starts_with(' ') || line[start..].is_empty() {
                    found.push((n + 1, &line[start..]));
                }
                from = start;
            }
        }
        if in_fence {
            if let Some(rest) = trimmed.strip_prefix(bin) {
                if rest.starts_with(' ') || rest.is_empty() {
                    found.push((n + 1, rest));
                }
            }
        }
    }
    found
}

#[test]
fn test_onboarding_cli_invocations_name_real_subcommands() {
    let clis = [
        (
            "soos-enroll",
            cli_shape("crates/enrollment-cli/src/args.rs"),
        ),
        ("soos-admin", cli_shape("crates/admin-cli/src/args.rs")),
    ];
    let mut violations = Vec::new();
    let mut checked = 0usize;
    for rel in onboarding_files() {
        let text = read(&rel);
        for (bin, shape) in &clis {
            for (line, rest) in invocations(&text, bin) {
                checked += 1;
                // The argument list ends at a shell operator or a comment.
                let cut = rest
                    .find(['#', '|', ';', '&', ')'])
                    .map_or(rest, |p| &rest[..p]);
                if let Some(err) = check_invocation(shape, &tokenize(cut)) {
                    violations.push(format!("{rel}:{line}: `{bin}{rest}`: {err}"));
                }
            }
        }
    }
    assert!(
        checked >= 10,
        "the scan found only {checked} invocations; it would pass vacuously"
    );
    assert!(
        violations.is_empty(),
        "onboarding files show commands the CLIs reject (GitHub #207):\n{}",
        violations.join("\n")
    );
}

#[test]
fn test_onboarding_docs_have_no_placeholder_enroll_or_bin_templates() {
    let mut violations = Vec::new();
    for rel in onboarding_files() {
        for (n, line) in read(&rel).lines().enumerate() {
            if line.contains("soos-enroll <") {
                violations.push(format!(
                    "{rel}:{}: `soos-enroll <...>` without a subcommand: {line}",
                    n + 1
                ));
            }
            if let Some(pos) = line.find("biometrics/") {
                let tail: String = line[pos..]
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != '`' && *c != '"')
                    .collect();
                if tail.ends_with(".bin") {
                    violations.push(format!(
                        "{rel}:{}: templates are `<uid>.cbor.enc`, not `{tail}`",
                        n + 1
                    ));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn test_install_epilogue_prints_real_enrollment_command() {
    let install = read("scripts/install.sh");
    let epilogue = install
        .split("Next steps:")
        .nth(1)
        .expect("install.sh prints a 'Next steps:' epilogue");
    assert!(
        epilogue.contains("sudo soos-enroll enroll --username <username>"),
        "the epilogue must print `sudo soos-enroll enroll --username <username>`:\n{epilogue}"
    );
    assert!(
        epilogue.contains("sudo soos-admin add-user <username>"),
        "adding a user to the soos group needs root: `sudo soos-admin add-user <username>`"
    );
    assert!(
        epilogue.contains("Docs/DISTRIBUTION_DEPLOYMENT.md"),
        "the epilogue must point to the explicit PAM activation step"
    );
}

#[test]
fn test_readme_has_quick_start_through_uninstall() {
    let readme = read("README.md");
    let start = readme
        .find("## Quick Start")
        .expect("README.md must have a `## Quick Start` section (GitHub #207)");
    let section = &readme[start..];
    let end = section[3..].find("\n## ").map_or(section.len(), |p| p + 3);
    let section = &section[..end];
    for needle in [
        "scripts/install.sh",
        "soos-admin add-user",
        "soos-enroll enroll --username",
        "soos-admin status",
        "soos-admin test-pam",
        "pam-auth-update --package --enable soos",
        "/etc/soos/disabled",
        "scripts/uninstall.sh",
    ] {
        assert!(
            section.contains(needle),
            "README Quick Start must cover `{needle}`"
        );
    }
}

// ---------------------------------------------------------------------------
// #210 — packaging metadata
// ---------------------------------------------------------------------------

fn run_pkg_meta(args: &[&str]) -> String {
    let out = Command::new("bash")
        .arg(workspace_root().join(PKG_META))
        .args(args)
        .output()
        .expect("run pkg_meta.sh");
    assert!(
        out.status.success(),
        "{PKG_META} {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .trim()
        .to_string()
}

#[test]
fn test_pkg_meta_reads_workspace_package_metadata() {
    assert_eq!(run_pkg_meta(&["version"]), cargo_version());
    assert_eq!(run_pkg_meta(&["license"]), cargo_license());

    // Fixture manifest: the helper reads the table, not a hardcoded value, and ignores
    // same-named keys of other tables.
    let dir = std::env::temp_dir().join(format!("soos_pkg_meta_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch");
    let manifest = dir.join("Cargo.toml");
    fs::write(
        &manifest,
        "[package]\nversion = \"0.0.1\"\n\n[workspace.package]\nversion = \"9.8.7\"\n\
         edition = \"2021\"\nlicense = \"GPL-2.0-only\"\n\n[workspace.dependencies]\n\
         license = \"MIT\"\n",
    )
    .expect("write fixture");
    let m = manifest.to_string_lossy().into_owned();
    assert_eq!(run_pkg_meta(&["--manifest", &m, "version"]), "9.8.7");
    assert_eq!(run_pkg_meta(&["--manifest", &m, "license"]), "GPL-2.0-only");

    // A manifest without the table fails closed.
    fs::write(&manifest, "[package]\nversion = \"1.0.0\"\n").expect("write fixture");
    let out = Command::new("bash")
        .arg(workspace_root().join(PKG_META))
        .args(["--manifest", &m, "version"])
        .output()
        .expect("run pkg_meta.sh");
    assert!(
        !out.status.success(),
        "a manifest without [workspace.package] must fail"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_package_license_matches_cargo_workspace() {
    let license = cargo_license();
    let spec = read("packaging/rpm/soos.spec");
    let spec_license = spec
        .lines()
        .find_map(|l| l.strip_prefix("License:"))
        .expect("soos.spec License:")
        .trim()
        .to_string();
    assert_eq!(
        spec_license, license,
        "soos.spec License must match Cargo.toml"
    );

    let pkgbuild = read("packaging/arch/PKGBUILD");
    let arch_license = pkgbuild
        .lines()
        .find_map(|l| l.strip_prefix("license="))
        .expect("PKGBUILD license=")
        .trim();
    assert_eq!(
        arch_license,
        format!("('{license}')"),
        "PKGBUILD license must match Cargo.toml"
    );

    for rel in BUILD_SCRIPTS {
        let text = read(rel);
        for bad in ["Apache-2.0", "MIT"] {
            assert!(
                !text
                    .lines()
                    .any(|l| l.contains("license") && l.contains(bad)),
                "{rel}: hardcoded license `{bad}`; derive it from {PKG_META}"
            );
        }
    }
    let build_arch = read("scripts/build_arch.sh");
    assert!(
        build_arch.contains("license = ${PKG_LICENSE}"),
        "build_arch.sh .PKGINFO license must come from pkg_meta.sh"
    );
}

#[test]
fn test_package_versions_derive_from_cargo_workspace() {
    let version = cargo_version();
    let spec = read("packaging/rpm/soos.spec");
    let spec_version = spec
        .lines()
        .find_map(|l| l.strip_prefix("Version:"))
        .expect("soos.spec Version:")
        .trim()
        .to_string();
    assert_eq!(
        spec_version, version,
        "soos.spec Version must match Cargo.toml"
    );
    let pkgbuild = read("packaging/arch/PKGBUILD");
    let pkgver = pkgbuild
        .lines()
        .find_map(|l| l.strip_prefix("pkgver="))
        .expect("PKGBUILD pkgver=")
        .trim()
        .to_string();
    assert_eq!(pkgver, version, "PKGBUILD pkgver must match Cargo.toml");

    for rel in BUILD_SCRIPTS {
        let text = read(rel);
        assert!(
            text.contains("scripts/lib/pkg_meta.sh"),
            "{rel} must source {PKG_META}"
        );
        assert!(
            !text.lines().any(|l| {
                let t = l.trim_start();
                t.starts_with("VERSION=\"") && t.chars().nth(9).is_some_and(|c| c.is_ascii_digit())
            }),
            "{rel} hardcodes VERSION; derive it with soos_pkg_version"
        );
        // Behavioural check: the dry-run plan prints the Cargo.toml version.
        let out = Command::new("bash")
            .arg(workspace_root().join(rel))
            .arg("--dry-run")
            .output()
            .expect("run build script");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{rel} --dry-run failed: {stdout}");
        assert!(
            stdout
                .lines()
                .any(|l| l.starts_with("Version:") && l.contains(&version)),
            "{rel} --dry-run must print Version {version}:\n{stdout}"
        );
    }
}

#[test]
fn test_rpm_and_arch_recipes_ship_soos_gui() {
    let spec = read("packaging/rpm/soos.spec");
    assert!(
        spec.contains("target/release/soos-gui %{buildroot}%{_bindir}/soos-gui"),
        "soos.spec %install must install soos-gui"
    );
    let files = spec.split("%files").nth(1).expect("soos.spec %files");
    assert!(
        files.lines().any(|l| l.trim() == "%{_bindir}/soos-gui"),
        "soos.spec %files must list %{{_bindir}}/soos-gui"
    );
    let recommends = spec
        .lines()
        .find_map(|l| l.strip_prefix("Recommends:"))
        .expect("soos.spec must recommend the soos-gui runtime libraries (dlopen'ed)");
    let recommended: BTreeSet<&str> = recommends.split_whitespace().collect();
    for pkg in gui_packages("fedora") {
        assert!(
            recommended.contains(pkg.as_str()),
            "soos.spec Recommends must contain GUI runtime library {pkg}"
        );
    }

    let pkgbuild = read("packaging/arch/PKGBUILD");
    let package = pkgbuild
        .split("package()")
        .nth(1)
        .expect("PKGBUILD package()");
    assert!(
        package.contains("target/release/soos-gui\" \"${pkgdir}/usr/bin/soos-gui\""),
        "PKGBUILD package() must install soos-gui"
    );
    let optdepends = pkgbuild
        .split("optdepends=(")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .expect("PKGBUILD must list the soos-gui runtime libraries as optdepends");
    for pkg in gui_packages("arch") {
        assert!(
            optdepends.contains(&format!("'{pkg}:")),
            "PKGBUILD optdepends must contain GUI runtime library {pkg}"
        );
    }
}

/// GUI runtime packages of a distribution, from the single source of truth.
fn gui_packages(distro: &str) -> Vec<String> {
    let out = Command::new("bash")
        .arg(workspace_root().join("scripts/check_build_deps.sh"))
        .args(["--distro", distro, "--print-packages", "gui"])
        .output()
        .expect("run check_build_deps.sh");
    assert!(out.status.success(), "check_build_deps.sh failed");
    let list: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    assert!(!list.is_empty(), "{distro} gui package list is empty");
    list
}

#[test]
fn test_packaging_cargo_invocations_are_locked() {
    let mut violations = Vec::new();
    for rel in PACKAGING_BUILD_FILES {
        for (n, line) in read(rel).lines().enumerate() {
            let t = line.trim_start();
            let is_cargo = t.starts_with("cargo build") || t.starts_with("cargo test");
            if is_cargo && !t.contains("--locked") {
                violations.push(format!("{rel}:{}: {t}", n + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "packaging cargo invocations must use --locked (GitHub #210):\n{}",
        violations.join("\n")
    );
}

#[test]
fn test_pkgbuild_builds_from_an_explicit_source_tree() {
    let pkgbuild = read("packaging/arch/PKGBUILD");
    for func in ["build()", "package()"] {
        let body = pkgbuild.split(func).nth(1).expect("function present");
        let body = body.split("\n}").next().expect("function body");
        assert!(
            body.contains("cd \"${_soos_srcdir}\""),
            "PKGBUILD {func} must cd into the source tree (`source=()` leaves $srcdir empty)"
        );
    }
}
