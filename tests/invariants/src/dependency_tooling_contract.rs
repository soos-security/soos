//! Dependency hygiene, Docker sandbox toolchain and `save.sh` commit contract
//! (review findings TCI-12 / GitHub #243, TCI-13 / GitHub #244, TCI-15 / GitHub #245).
//!
//! - Every `[bans] skip` entry of `deny.toml` names a crate version that is really locked, and
//!   its reason names at least one package that directly depends on that version (computed from
//!   `Cargo.lock`, the same graph `cargo tree -i` prints). A version used directly by a
//!   workspace crate is never described as "transitive".
//! - Every version of a crate locked in three or more versions is named in `deny.toml`.
//! - A pre-release workspace dependency (e.g. `ort` 2.0.0-rc) is pinned with `=`.
//! - `tests/docker/test_suite.sh` synchronizes the container toolchain with
//!   `rust-toolchain.toml` before it builds, and always rebuilds `libpam_soos.so`.
//! - `save.sh` never stages everything with `git add .`, and an inferred commit subject takes
//!   its type from the branch prefix (never a generic `feat(...)`), with no pseudo-subject body.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

// ---------------------------------------------------------------------------
// Cargo.lock / deny.toml parsing (no external dependencies)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct LockedPackage {
    name: String,
    version: String,
    /// `None` for workspace members (path packages).
    source: Option<String>,
    /// Raw dependency entries: `name`, `name version` or `name version (source)`.
    dependencies: Vec<String>,
}

fn quoted(value: &str) -> Option<String> {
    let start = value.find('"')?;
    let rest = &value[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn parse_lockfile(text: &str) -> Vec<LockedPackage> {
    let mut packages = Vec::new();
    let mut current: Option<LockedPackage> = None;
    let mut in_deps = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            if let Some(pkg) = current.take() {
                packages.push(pkg);
            }
            current = Some(LockedPackage::default());
            in_deps = false;
            continue;
        }
        let Some(pkg) = current.as_mut() else {
            continue;
        };
        if in_deps {
            if trimmed == "]" {
                in_deps = false;
            } else if let Some(dep) = quoted(trimmed) {
                pkg.dependencies.push(dep);
            }
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("name = ") {
            pkg.name = quoted(v).expect("quoted name");
        } else if let Some(v) = trimmed.strip_prefix("version = ") {
            pkg.version = quoted(v).expect("quoted version");
        } else if let Some(v) = trimmed.strip_prefix("source = ") {
            pkg.source = quoted(v);
        } else if trimmed.starts_with("dependencies = [") {
            if trimmed.ends_with(']') {
                // Single-line list (not produced by cargo today, handled for robustness).
                for part in trimmed.split(',') {
                    if let Some(dep) = quoted(part) {
                        pkg.dependencies.push(dep);
                    }
                }
            } else {
                in_deps = true;
            }
        }
    }
    if let Some(pkg) = current.take() {
        packages.push(pkg);
    }
    packages
}

fn locked_packages() -> Vec<LockedPackage> {
    let packages = parse_lockfile(&read("Cargo.lock"));
    assert!(
        packages.len() > 50,
        "Cargo.lock parsing found too few packages"
    );
    packages
}

/// Versions locked for every crate name.
fn versions_by_name(packages: &[LockedPackage]) -> BTreeMap<String, BTreeSet<String>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for pkg in packages {
        map.entry(pkg.name.clone())
            .or_default()
            .insert(pkg.version.clone());
    }
    map
}

/// Resolves a raw `Cargo.lock` dependency entry to `(name, version)`.
fn resolve_dep(entry: &str, versions: &BTreeMap<String, BTreeSet<String>>) -> (String, String) {
    let mut parts = entry.split_whitespace();
    let name = parts.next().expect("dependency name").to_string();
    let version = match parts.next() {
        Some(v) => v.to_string(),
        None => {
            let set = versions
                .get(&name)
                .unwrap_or_else(|| panic!("dependency {name} is not locked"));
            assert_eq!(
                set.len(),
                1,
                "unversioned dependency entry {name} must be unique"
            );
            set.iter().next().expect("one version").clone()
        }
    };
    (name, version)
}

/// Packages that directly depend on `name@version`, as `(dependent name, is workspace member)`.
fn direct_dependents(packages: &[LockedPackage], name: &str, version: &str) -> Vec<(String, bool)> {
    let versions = versions_by_name(packages);
    let mut out = Vec::new();
    for pkg in packages {
        for dep in &pkg.dependencies {
            let (n, v) = resolve_dep(dep, &versions);
            if n == name && v == version {
                out.push((pkg.name.clone(), pkg.source.is_none()));
            }
        }
    }
    out
}

#[derive(Debug)]
struct SkipEntry {
    name: String,
    version: String,
    reason: String,
    line: usize,
}

fn deny_skip_entries(deny: &str) -> Vec<SkipEntry> {
    let mut entries = Vec::new();
    for (n, line) in deny.lines().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("{ crate = ") || !line.contains("reason = ") {
            continue;
        }
        let krate = quoted(&trimmed["{ crate = ".len()..]).expect("quoted crate");
        let reason_at = line.find("reason = ").expect("reason");
        let reason = quoted(&line[reason_at + "reason = ".len()..]).expect("quoted reason");
        let Some((name, version)) = krate.split_once('@') else {
            // `[bans] deny` entries have no version.
            continue;
        };
        entries.push(SkipEntry {
            name: name.to_string(),
            version: version.to_string(),
            reason,
            line: n + 1,
        });
    }
    entries
}

fn reason_tokens(reason: &str) -> BTreeSet<String> {
    reason
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn test_parse_lockfile_resolves_direct_dependents() {
    let lock = "\
[[package]]
name = \"a\"
version = \"1.0.0\"
source = \"registry+x\"

[[package]]
name = \"a\"
version = \"2.0.0\"
source = \"registry+x\"

[[package]]
name = \"b\"
version = \"0.1.0\"
source = \"registry+x\"
dependencies = [
 \"a 1.0.0\",
]

[[package]]
name = \"member\"
version = \"0.1.0\"
dependencies = [
 \"a 2.0.0\",
 \"b\",
]
";
    let packages = parse_lockfile(lock);
    assert_eq!(packages.len(), 4);
    assert_eq!(
        direct_dependents(&packages, "a", "1.0.0"),
        vec![("b".to_string(), false)]
    );
    assert_eq!(
        direct_dependents(&packages, "a", "2.0.0"),
        vec![("member".to_string(), true)]
    );
}

#[test]
fn test_deny_skip_entries_are_locked_versions() {
    let packages = locked_packages();
    let versions = versions_by_name(&packages);
    let entries = deny_skip_entries(&read("deny.toml"));
    assert!(!entries.is_empty(), "deny.toml [bans] skip must be parsed");
    for e in &entries {
        let locked = versions.get(&e.name).cloned().unwrap_or_default();
        assert!(
            locked.contains(&e.version),
            "deny.toml:{}: skip `{}@{}` is not in Cargo.lock (locked: {locked:?}); remove the stale entry",
            e.line,
            e.name,
            e.version
        );
        assert!(
            locked.len() > 1,
            "deny.toml:{}: `{}` is locked in a single version; the skip is unnecessary",
            e.line,
            e.name
        );
    }
}

#[test]
fn test_deny_skip_reasons_name_a_real_direct_dependent() {
    let packages = locked_packages();
    let mut stale = Vec::new();
    for e in deny_skip_entries(&read("deny.toml")) {
        let dependents = direct_dependents(&packages, &e.name, &e.version);
        let tokens = reason_tokens(&e.reason);
        if !dependents.iter().any(|(d, _)| tokens.contains(d)) {
            let names: BTreeSet<&str> = dependents.iter().map(|(d, _)| d.as_str()).collect();
            stale.push(format!(
                "deny.toml:{}: `{}@{}` reason {:?} names none of its direct dependents {names:?}",
                e.line, e.name, e.version, e.reason
            ));
        }
    }
    assert!(
        stale.is_empty(),
        "stale skip reasons (rewrite them from `cargo tree -i <crate>@<version>`):\n{}",
        stale.join("\n")
    );
}

#[test]
fn test_deny_skip_reasons_never_call_a_direct_workspace_dependency_transitive() {
    let packages = locked_packages();
    for e in deny_skip_entries(&read("deny.toml")) {
        let members: Vec<String> = direct_dependents(&packages, &e.name, &e.version)
            .into_iter()
            .filter(|(_, member)| *member)
            .map(|(d, _)| d)
            .collect();
        if members.is_empty() {
            continue;
        }
        let lower = e.reason.to_ascii_lowercase();
        assert!(
            !lower.contains("transitive") && lower.contains("direct"),
            "deny.toml:{}: `{}@{}` is a direct dependency of {members:?}; its reason {:?} must say so",
            e.line,
            e.name,
            e.version,
            e.reason
        );
    }
}

#[test]
fn test_deny_documents_every_version_of_crates_locked_three_times() {
    let deny = read("deny.toml");
    let versions = versions_by_name(&locked_packages());
    for (name, set) in versions.iter().filter(|(_, s)| s.len() >= 3) {
        for v in set {
            let id = format!("{name}@{v}");
            assert!(
                deny.contains(&id),
                "`{id}` is one of {} locked versions of `{name}` but deny.toml never names it; \
                 document its origin (skip entry or comment)",
                set.len()
            );
        }
    }
}

#[test]
fn test_prerelease_workspace_dependencies_are_exact_pinned() {
    let manifest = read("Cargo.toml");
    let section = manifest
        .split("[workspace.dependencies]")
        .nth(1)
        .expect("root Cargo.toml has [workspace.dependencies]");
    let section = section.split("\n[").next().unwrap_or(section);
    let mut checked = 0;
    for line in section.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || !trimmed.contains('=') {
            continue;
        }
        let (name, value) = trimmed.split_once('=').expect("key = value");
        let req = if value.trim_start().starts_with('{') {
            match value.find("version") {
                Some(at) => quoted(&value[at..]),
                None => None,
            }
        } else {
            quoted(value)
        };
        let Some(req) = req else { continue };
        checked += 1;
        let lower = req.to_ascii_lowercase();
        if ["-rc", "-alpha", "-beta", "-pre"]
            .iter()
            .any(|m| lower.contains(m))
        {
            assert!(
                req.starts_with('='),
                "workspace dependency `{}` uses the pre-release requirement {req:?}; pin it with `=` \
                 so pre-release API changes are adopted deliberately",
                name.trim()
            );
        }
    }
    assert!(
        checked > 10,
        "workspace dependency parsing found too few entries"
    );
    assert!(
        section.contains("ort = \"=2.0.0-rc."),
        "ort must stay exact-pinned to its release candidate until 2.0.0 GA"
    );
}

// ---------------------------------------------------------------------------
// Docker sandbox (TCI-13)
// ---------------------------------------------------------------------------

const SUITE: &str = "tests/docker/test_suite.sh";
const PAM_BUILD: &str = "cargo build --locked --release -p soos-pam";

fn first_line_index(text: &str, pred: impl Fn(&str) -> bool) -> Option<usize> {
    text.lines().position(pred)
}

#[test]
fn test_docker_suite_always_rebuilds_the_pam_module() {
    let suite = read(SUITE);
    let lines: Vec<&str> = suite.lines().collect();
    let build = first_line_index(&suite, |l| l.trim() == PAM_BUILD)
        .unwrap_or_else(|| panic!("{SUITE} must run `{PAM_BUILD}`"));
    assert_eq!(
        lines[build],
        PAM_BUILD,
        "{SUITE}:{}: the release build must run unconditionally (top level, never guarded by an \
         existing artifact)",
        build + 1
    );
    let deploy = first_line_index(&suite, |l| l.starts_with("cp \"${SO_PATH}\""))
        .unwrap_or_else(|| panic!("{SUITE} must deploy ${{SO_PATH}}"));
    assert!(
        build < deploy,
        "the module must be rebuilt before it is deployed"
    );
    for l in &lines[..build] {
        assert!(
            !(l.contains("-f \"${SO_PATH}\"") && l.contains("if ")),
            "{SUITE}: the build must not be skipped when an artifact already exists: {l}"
        );
    }
}

#[test]
fn test_docker_suite_syncs_toolchain_with_rust_toolchain_toml() {
    let suite = read(SUITE);
    let sync = first_line_index(&suite, |l| {
        l.contains("rustup toolchain install") && !l.trim_start().starts_with('#')
    })
    .unwrap_or_else(|| {
        panic!("{SUITE} must run `rustup toolchain install` (reads rust-toolchain.toml)")
    });
    let build = first_line_index(&suite, |l| {
        l.trim_start().starts_with("cargo ") && !l.trim_start().starts_with('#')
    })
    .expect("a cargo invocation");
    assert!(
        sync < build,
        "{SUITE}: the toolchain must be synchronized (line {}) before the first cargo call (line {})",
        sync + 1,
        build + 1
    );
    assert!(
        suite.contains("SOOS_REQUIRE_TOOLCHAIN_SYNC"),
        "{SUITE} must fail closed on a failed sync when SOOS_REQUIRE_TOOLCHAIN_SYNC=1"
    );
    assert!(
        suite.contains("rustc --version"),
        "{SUITE} must log the toolchain actually used"
    );
    let toolchain = read("rust-toolchain.toml");
    assert!(
        toolchain.contains("channel = "),
        "rust-toolchain.toml must declare the channel the sandbox installs"
    );
}

#[test]
fn test_ci_pam_integration_requires_toolchain_sync() {
    let ci = read(".github/workflows/ci.yml");
    let job = ci
        .split("\n  pam-integration:")
        .nth(1)
        .expect("ci.yml defines the pam-integration job");
    let job = job.split("\n  # ----").next().unwrap_or(job);
    assert!(
        job.contains("-e SOOS_REQUIRE_TOOLCHAIN_SYNC=1"),
        "the pam-integration job must run the sandbox with SOOS_REQUIRE_TOOLCHAIN_SYNC=1 so a \
         cached image never tests with a stale toolchain"
    );
}

// ---------------------------------------------------------------------------
// save.sh (TCI-15)
// ---------------------------------------------------------------------------

const COMMIT_HELPER: &str = "scripts/commit_message.sh";

#[test]
fn test_save_sh_never_stages_every_untracked_file() {
    let save = read("save.sh");
    for (n, line) in save.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = t.split_whitespace().collect();
        if words.len() >= 3 && words[0] == "git" && words[1] == "add" {
            for w in &words[2..] {
                assert!(
                    !matches!(*w, "." | "-A" | "--all" | "*" | "./"),
                    "save.sh:{}: `{t}` stages every untracked file; stage tracked changes with \
                     `git add -u` and new files only under the source directories",
                    n + 1
                );
            }
        }
    }
    assert!(
        save.lines().any(|l| l.trim() == "git add -u"),
        "save.sh must stage tracked modifications with `git add -u`"
    );
}

#[test]
fn test_save_sh_never_generates_generic_subjects() {
    let save = read("save.sh");
    for banned in [
        "update component implementation",
        "update multiple crate implementations",
        "file(s) modified",
        "PARTS+=(",
    ] {
        assert!(
            !save.contains(banned),
            "save.sh must not generate the misleading message fragment {banned:?}"
        );
    }
    assert!(
        save.contains(COMMIT_HELPER),
        "save.sh must infer subjects with {COMMIT_HELPER}"
    );
}

fn infer_subject(branch: &str, files: &[&str]) -> Output {
    let helper = workspace_root().join(COMMIT_HELPER);
    assert!(helper.is_file(), "{COMMIT_HELPER} must exist");
    let mut child = Command::new("bash")
        .arg("-c")
        .arg("source \"$1\" && soos_infer_commit_subject \"$2\"")
        .arg("bash")
        .arg(&helper)
        .arg(branch)
        .current_dir(workspace_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("bash must be available");
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("stdin");
        // The helper may exit before reading stdin (refused branch): ignore EPIPE.
        for f in files {
            let _ = writeln!(stdin, "{f}");
        }
    }
    child.wait_with_output().expect("wait for bash")
}

fn assert_subject(branch: &str, files: &[&str], expected: &str) {
    let out = infer_subject(branch, files);
    assert!(
        out.status.success(),
        "branch {branch:?} must yield a subject: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout,
        format!("{expected}\n"),
        "branch {branch:?} with {files:?} must yield exactly one subject line"
    );
    assert!(
        expected.len() <= 72,
        "inferred subjects fit the PR title limit"
    );
}

#[test]
fn test_commit_helper_takes_type_from_branch_prefix_and_scope_from_one_crate() {
    assert_subject(
        "fix/pam-timeout",
        &["crates/pam/src/ipc.rs", "crates/pam/tests/ipc_tests.rs"],
        "fix(pam): pam timeout",
    );
    assert_subject(
        "feat/ipc-client",
        &["crates/pam/src/ipc.rs", "crates/protocol/src/codec.rs"],
        "feat: ipc client",
    );
    assert_subject(
        "chore/ci_cache",
        &[".github/workflows/ci.yml", "Docs/CI_CD_AND_SECURITY.md"],
        "chore: ci cache",
    );
    assert_subject(
        "test/Fuzz-Codec",
        &["crates/protocol/tests/fuzz.rs", "AI/walkthroughs/99_x.md"],
        "test(protocol): fuzz codec",
    );
    assert_subject(
        "fix/camera-v4l-eio",
        &["crates/camera-v4l/src/device.rs"],
        "fix(camera-v4l): camera v4l eio",
    );
}

#[test]
fn test_commit_helper_refuses_to_guess_without_a_known_branch_prefix() {
    for branch in [
        "main",
        "detached",
        "refactor/x",
        "docs/readme",
        "fix/",
        "fix/---",
        "feature/pam",
        "",
    ] {
        let out = infer_subject(branch, &["crates/pam/src/lib.rs"]);
        assert!(
            !out.status.success(),
            "branch {branch:?} must not yield an inferred subject"
        );
        assert!(
            out.stdout.is_empty(),
            "branch {branch:?}: no subject on stdout when inference fails"
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("-m"),
            "branch {branch:?}: the error must ask for an explicit -m message"
        );
    }
}

#[test]
fn test_commit_helper_sanitizes_and_bounds_the_subject() {
    assert_subject(
        "fix/pam;rm -rf $HOME`id`",
        &["crates/pam/src/lib.rs"],
        "fix(pam): pam rm rf home id",
    );
    let long = format!("chore/{}", "word-".repeat(20));
    let out = infer_subject(&long, &["Docs/a.md"]);
    assert!(
        !out.status.success() && out.stdout.is_empty(),
        "a subject longer than 72 characters must be refused, not truncated silently"
    );
}
