//! Portable shared crates for the separate `soos-macos` repository (GitHub #360 / #367, ADR
//! "[2026-10-13] macOS Port in a Separate Repository"; spec
//! `AI/architect_spec_macos_port_foundation.md` §3, §4, §9; matrix MAC1–MAC9):
//!
//! - the portable crate set never reaches a Linux-only or application crate, through normal or
//!   build edges, on any target, with every feature (resolved `cargo tree` graph);
//! - the guard itself can fail (a hermetic broken manifest, one witness per forbidden group);
//! - `soos-frame` is pure, forbids `unsafe` and owns the frame types that `soos-camera-v4l`
//!   re-exports; `soos-vision` and the shared fixtures no longer depend on `soos-camera-v4l`;
//! - the macOS CI job is SHA-pinned, secret-free, limited to the portable set, outside the
//!   `CI Success` aggregate, and the only advisory check of `scripts/pr_loop.sh`;
//! - the ADR and `Docs/MACOS_PORT.md` record the bootstrap specification.

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
use std::process::Command;

/// Crates the `soos-macos` repository may consume (ADR 2026-10-13, spec D-3).
const PORTABLE_CRATES: [&str; 7] = [
    "soos-frame",
    "soos-protocol",
    "soos-policy",
    "soos-push-protocol",
    "soos-inference-ort",
    "soos-vision",
    "soos-biometric-store",
];

/// Dev-only crate used by the portable crates' tests; it must be just as portable.
const PORTABLE_DEV_SUPPORT: [&str; 1] = ["soos-test-fixtures"];

/// Crates whose tests the macOS job runs: the portable set without `soos-biometric-store`
/// (compiled with `clippy --all-targets`, tests not yet run on macOS, spec D-4).
const MACOS_TESTED_CRATES: [&str; 6] = [
    "soos-frame",
    "soos-protocol",
    "soos-policy",
    "soos-push-protocol",
    "soos-inference-ort",
    "soos-vision",
];

/// Package names no portable crate may reach (spec §3.1).
///
/// Defensive entries, absent from `Cargo.lock` today: `v4l-sys`, `pam-sys`, `pam`, `udev`,
/// `libudev-sys`, `libsystemd`, `sd-notify`, `inotify`, `inotify-sys`. Every other entry is
/// witnessed by `test_mac_guard_query_sees_linux_only_crates`.
const FORBIDDEN_PORTABLE_DEPENDENCIES: [&str; 31] = [
    // V4L2
    "v4l",
    "v4l2-sys-mit",
    "v4l-sys",
    // Linux-PAM ABI
    "pam-bindings",
    "pam-sys",
    "pam",
    // D-Bus / logind
    "zbus",
    "zbus_macros",
    "zbus_names",
    "zvariant",
    // udev / systemd / inotify
    "udev",
    "libudev-sys",
    "libsystemd",
    "sd-notify",
    "inotify",
    "inotify-sys",
    // Async runtime (policy: shared crates stay runtime-free)
    "tokio",
    "tokio-macros",
    // Linux desktop session stacks
    "wayland-client",
    "wayland-sys",
    "x11rb",
    "x11-dl",
    // Linux-only adapters and application crates of this workspace
    "soos-camera-v4l",
    "soos-pam",
    "soos-daemon",
    "soos-remote",
    "soos-push-sender",
    "soos-gui",
    "soos-admin-cli",
    "soos-enrollment-cli",
    "soos-test-fixtures",
];

/// `name:` of the macOS job in `ci.yml`, also the single advisory check of `pr_loop.sh`.
const MACOS_JOB_NAME: &str = "Portable crates (macOS, non-required)";
const MACOS_JOB_ID: &str = "portable-macos";
/// `name:` of the system ONNX Runtime job, the second advisory check (GitHub #375).
const SYSTEM_ORT_JOB_NAME: &str = "System ONNX Runtime (Arch, offline, advisory on PRs)";
const ADR_TITLE: &str = "macOS Port in a Separate Repository";

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

/// Package names of the resolved normal + build dependency graph of `package` (every target,
/// every feature), without the queried root package itself (spec §3.2, X-1). Fail-closed.
fn dependency_packages(manifest_dir: &Path, package: &str, locked: bool) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| String::from("cargo"));
    let mut cmd = Command::new(cargo);
    cmd.current_dir(manifest_dir).arg("tree");
    cmd.arg(if locked { "--locked" } else { "--offline" });
    cmd.args([
        "-p",
        package,
        "-e",
        "normal,build",
        "--target",
        "all",
        "--all-features",
        "--prefix",
        "none",
        "--format",
        "{p}",
    ]);
    let out = cmd.output().expect("run cargo tree");
    assert!(
        out.status.success(),
        "cargo tree failed for {package} (fail-closed): {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(String::from)
        .collect();
    assert!(
        names.iter().any(|n| n == package),
        "cargo tree output for {package} does not contain the root package (vacuous query): {names:?}"
    );
    strip_root(names, package)
}

/// Sorted, deduplicated `names` without the exact root package name.
fn strip_root(mut names: Vec<String>, root: &str) -> Vec<String> {
    names.retain(|n| n != root);
    names.sort();
    names.dedup();
    names
}

/// The forbidden package names present in `packages`.
fn forbidden_hits(packages: &[String]) -> Vec<String> {
    packages
        .iter()
        .filter(|p| FORBIDDEN_PORTABLE_DEPENDENCIES.contains(&p.as_str()))
        .cloned()
        .collect()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

/// MAC3 (issue-mandated name): no portable crate, nor the dev fixtures they use, reaches a
/// forbidden package.
#[test]
fn test_portable_crates_have_no_linux_only_dependencies() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for package in PORTABLE_CRATES.iter().chain(PORTABLE_DEV_SUPPORT.iter()) {
        let graph = dependency_packages(&root, package, true);
        let hits = forbidden_hits(&graph);
        if !hits.is_empty() {
            violations.push(format!("{package} reaches {hits:?}"));
        }
    }
    // Non-vacuity: the vision graph really is resolved (it must see its inference crate).
    let vision = dependency_packages(&root, "soos-vision", true);
    assert!(
        vision.iter().any(|n| n == "soos-inference-ort")
            && vision.iter().any(|n| n == "soos-frame"),
        "soos-vision graph must contain soos-inference-ort and soos-frame: {vision:?}"
    );
    assert!(
        violations.is_empty(),
        "PORTABILITY VIOLATION (ADR 2026-10-13): portable crates reach Linux-only or application \
         crates:\n{}",
        violations.join("\n")
    );
}

/// MAC4: the pure intersection and root exclusion are exact (X-1).
#[test]
fn test_mac_forbidden_hits_is_exact() {
    assert_eq!(forbidden_hits(&names(&["serde", "v4l"])), ["v4l"]);
    assert!(forbidden_hits(&names(&["serde", "zeroize"])).is_empty());
    assert!(forbidden_hits(&names(&["v4l-utils-free", "tokio-util-free", "pam_x"])).is_empty());
    let graph = names(&["soos-test-fixtures", "soos-frame"]);
    assert!(forbidden_hits(&strip_root(graph.clone(), "soos-test-fixtures")).is_empty());
    assert_eq!(
        forbidden_hits(&strip_root(graph, "soos-vision")),
        ["soos-test-fixtures"],
        "root exclusion must only remove the queried package"
    );
    let unique: std::collections::BTreeSet<&str> =
        FORBIDDEN_PORTABLE_DEPENDENCIES.iter().copied().collect();
    assert_eq!(unique.len(), FORBIDDEN_PORTABLE_DEPENDENCIES.len());
    for p in PORTABLE_CRATES {
        assert!(!FORBIDDEN_PORTABLE_DEPENDENCIES.contains(&p), "{p}");
    }
}

/// MAC4: the same query sees at least one forbidden crate of every group present in
/// `Cargo.lock` through the Linux-only crates (a misspelled name would fail here).
#[test]
fn test_mac_guard_query_sees_linux_only_crates() {
    let root = workspace_root();
    for (package, witnesses) in [
        ("soos-camera-v4l", &["v4l", "v4l2-sys-mit"][..]),
        ("soos-pam", &["pam-bindings"][..]),
        (
            "soos-daemon",
            // zvariant was merged into zbus 6 (#416, owner-approved witness update).
            &["zbus", "zbus_macros", "tokio", "soos-camera-v4l"][..],
        ),
        ("soos-gui", &["wayland-client", "x11rb"][..]),
    ] {
        let hits = forbidden_hits(&dependency_packages(&root, package, true));
        for w in witnesses {
            assert!(
                hits.iter().any(|h| h == w),
                "guard query is vacuous: {w} not seen in {package}'s graph (hits {hits:?})"
            );
        }
    }
}

/// MAC4: a deliberately broken manifest (a hermetic crate outside the repository with a path
/// dependency on a stub named `v4l`) is caught by the same code path.
#[test]
fn test_mac_guard_detects_a_forbidden_crate_in_a_broken_manifest() {
    let dir = std::env::temp_dir().join(format!("soos_mac_guard_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("probe/src")).unwrap();
    fs::create_dir_all(dir.join("stub/src")).unwrap();
    fs::write(
        dir.join("probe/Cargo.toml"),
        "[package]\nname = \"portable-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\
         publish = false\n\n[workspace]\n\n[dependencies]\nv4l = { path = \"../stub\" }\n",
    )
    .unwrap();
    fs::write(dir.join("probe/src/lib.rs"), "").unwrap();
    fs::write(
        dir.join("stub/Cargo.toml"),
        "[package]\nname = \"v4l\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n",
    )
    .unwrap();
    fs::write(dir.join("stub/src/lib.rs"), "").unwrap();
    let graph = dependency_packages(&dir.join("probe"), "portable-probe", false);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(
        forbidden_hits(&graph),
        ["v4l"],
        "broken manifest graph: {graph:?}"
    );
}

/// Dependency keys of the `[dependencies]` / `[dev-dependencies]` / `[build-dependencies]`
/// tables (any target) of a manifest.
fn manifest_dependency_keys(manifest: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            let header = t.trim_matches(|c| c == '[' || c == ']');
            in_deps = header.ends_with("dependencies");
            continue;
        }
        if in_deps && !t.is_empty() && !t.starts_with('#') {
            if let Some((k, _)) = t.split_once('=') {
                keys.push(k.trim().trim_end_matches(".workspace").to_string());
            }
        }
    }
    keys
}

/// MAC2: `soos-frame` is a pure crate: forbids `unsafe`, no I/O module, `zeroize` only.
#[test]
fn test_mac_frame_crate_is_pure_and_forbids_unsafe() {
    let manifest = read("crates/frame/Cargo.toml");
    assert!(manifest.contains("name = \"soos-frame\""), "{manifest}");
    for line in [
        "license.workspace = true",
        "publish.workspace = true",
        "version.workspace = true",
        "edition.workspace = true",
    ] {
        assert!(
            manifest.lines().any(|l| l.trim() == line),
            "crates/frame/Cargo.toml must contain `{line}`"
        );
    }
    assert!(manifest.contains("[lints]\nworkspace = true"), "{manifest}");
    assert!(
        !manifest.contains("[features]"),
        "no feature until an IR-only type needs the non-default `ir` feature (spec D-9)"
    );
    let normal: Vec<String> = {
        let normal_part = manifest
            .split("[dev-dependencies]")
            .next()
            .unwrap_or(&manifest);
        manifest_dependency_keys(normal_part)
    };
    assert_eq!(
        normal,
        ["zeroize"],
        "soos-frame's only normal dependency is zeroize"
    );

    let root_cargo = read("Cargo.toml");
    assert!(root_cargo.contains("\"crates/frame\""), "workspace member");
    assert!(
        root_cargo.contains("soos-frame = { path = \"crates/frame\", version = \"0.1.0\" }"),
        "workspace dependency entry"
    );

    let lib = read("crates/frame/src/lib.rs");
    let first = lib
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("//"))
        .unwrap_or("");
    assert_eq!(first, "#![forbid(unsafe_code)]");

    let src = workspace_root().join("crates/frame/src");
    let mut scanned = 0;
    let mut defined = String::new();
    for entry in fs::read_dir(&src).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            scanned += 1;
            let text = fs::read_to_string(&path).unwrap();
            for io in [
                "std::fs",
                "std::net",
                "std::process",
                "std::os",
                "std::io",
                "std::thread",
                "std::env",
                "unsafe ",
                "unsafe{",
                "println!",
                "eprintln!",
                "tracing",
            ] {
                assert!(
                    !text.contains(io),
                    "{} must stay pure (found `{io}`)",
                    path.display()
                );
            }
            defined.push_str(&text);
        }
    }
    assert!(scanned >= 1, "no soos-frame source scanned");
    for def in [
        "pub struct Frame",
        "pub enum PixelFormat",
        "pub enum SensorType",
        "impl Drop for Frame",
        "impl zeroize::Zeroize for Frame",
    ] {
        assert!(defined.contains(def), "soos-frame must define `{def}`");
    }
}

/// MAC2: `soos-camera-v4l` re-exports the frame types and no longer defines them.
#[test]
fn test_mac_camera_v4l_reexports_the_frame_types() {
    let manifest = read("crates/camera-v4l/Cargo.toml");
    assert!(
        manifest_dependency_keys(&manifest).contains(&"soos-frame".to_string()),
        "soos-camera-v4l depends on soos-frame"
    );
    assert!(read("crates/camera-v4l/src/frame.rs")
        .contains("pub use soos_frame::{Frame, PixelFormat};"));
    assert!(read("crates/camera-v4l/src/sensor.rs").contains("pub use soos_frame::SensorType;"));
    let src = workspace_root().join("crates/camera-v4l/src");
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(&path).unwrap();
                for def in [
                    "pub struct Frame ",
                    "pub struct Frame{",
                    "pub enum PixelFormat",
                    "pub enum SensorType",
                ] {
                    assert!(
                        !text.contains(def),
                        "{} must not redefine `{def}` (single source: soos-frame)",
                        path.display()
                    );
                }
            }
        }
    }
}

/// MAC2: neither `soos-vision` nor the shared fixtures depend on `soos-camera-v4l`.
#[test]
fn test_mac_vision_and_fixtures_do_not_depend_on_camera_v4l() {
    for (manifest, src_dir) in [
        ("crates/vision/Cargo.toml", "crates/vision/src"),
        ("tests/fixtures/Cargo.toml", "tests/fixtures"),
    ] {
        let keys = manifest_dependency_keys(&read(manifest));
        assert!(
            !keys.iter().any(|k| k == "soos-camera-v4l"),
            "{manifest} must not depend on soos-camera-v4l (any table): {keys:?}"
        );
        assert!(
            keys.iter().any(|k| k == "soos-frame"),
            "{manifest} depends on soos-frame: {keys:?}"
        );
        let mut stack = vec![workspace_root().join(src_dir)];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = fs::read_to_string(&path).unwrap();
                    assert!(
                        !text.contains("soos_camera_v4l"),
                        "{} still references soos_camera_v4l",
                        path.display()
                    );
                }
            }
        }
    }
    let vision_tests = workspace_root().join("crates/vision/tests");
    for entry in fs::read_dir(vision_tests).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).unwrap();
            assert!(
                !text.contains("soos_camera_v4l"),
                "{} still references soos_camera_v4l",
                path.display()
            );
        }
    }
}

/// MAC2: `frame` is in both business-crate lists (invariant and candid layer 1).
#[test]
fn test_mac_business_lists_include_frame() {
    let lib = read("tests/invariants/src/lib.rs");
    let start = lib
        .find("fn test_business_crates_forbid_unsafe_code")
        .expect("business crate invariant");
    let list = &lib[start..start + lib[start..].find("];").unwrap()];
    assert!(list.contains("\"frame\""), "business_crates lists frame");
    let candid = read("scripts/candid_review.sh");
    let line = candid
        .lines()
        .find(|l| l.starts_with("BUSINESS_CRATES=("))
        .expect("BUSINESS_CRATES line");
    assert!(
        line.trim_start_matches("BUSINESS_CRATES=(")
            .trim_end_matches(')')
            .split_whitespace()
            .any(|n| n == "frame"),
        "candid_review.sh BUSINESS_CRATES lists frame"
    );
}

/// Body of the top-level job `name` of `ci.yml` (up to the next banner comment).
fn ci_job<'a>(ci: &'a str, name: &str) -> &'a str {
    let start = ci
        .find(&format!("\n  {name}:"))
        .unwrap_or_else(|| panic!("ci.yml must define the {name} job"));
    let job = &ci[start + 1..];
    let end = job.find("\n  # ----").unwrap_or(job.len());
    &job[..end]
}

/// `-p` package arguments of the first line of `job` that contains `marker`.
fn packages_of_command(job: &str, marker: &str) -> Vec<String> {
    let line = job
        .lines()
        .find(|l| l.contains(marker))
        .unwrap_or_else(|| panic!("no `{marker}` command in the macOS job"));
    let words: Vec<&str> = line.split_whitespace().collect();
    let mut packages: Vec<String> = words
        .windows(2)
        .filter(|w| w[0] == "-p")
        .map(|w| w[1].to_string())
        .collect();
    packages.sort();
    packages
}

fn sorted(list: &[&str]) -> Vec<String> {
    let mut v = names(list);
    v.sort();
    v
}

/// MAC5: the macOS job is SHA-pinned, secret-free, read-only, portable-only and not a
/// `CI Success` dependency.
#[test]
fn test_mac_macos_job_is_pinned_secret_free_and_non_required() {
    let ci = read(".github/workflows/ci.yml");
    let job = ci_job(&ci, MACOS_JOB_ID);
    assert!(
        job.contains(&format!("name: \"{MACOS_JOB_NAME}\"")),
        "{job}"
    );
    assert!(job.contains("runs-on: macos-latest"), "arm64 macOS image");
    assert!(job.contains("timeout-minutes:"), "bounded job");
    assert!(job.contains("if: github.event_name != 'schedule'"));
    assert!(!job.contains("secrets."), "no secret in the macOS job");
    assert!(
        !job.contains("permissions:"),
        "workflow default contents: read"
    );
    assert!(
        !job.contains("continue-on-error"),
        "the job reports its real state"
    );
    assert!(job.contains("persist-credentials: false"));
    let mut uses = 0;
    for line in job
        .lines()
        .filter(|l| l.trim_start().starts_with("- uses:") || l.trim_start().starts_with("uses:"))
    {
        uses += 1;
        let spec = line.split("uses:").nth(1).unwrap().trim();
        let (_, rest) = spec.split_once('@').expect("pinned action");
        let sha: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
        assert_eq!(sha.len(), 40, "action pinned by full commit SHA: {line}");
        assert!(
            rest.contains("# v"),
            "pinned action carries its version comment: {line}"
        );
        let linux_jobs = ci.replace(job, "");
        assert!(
            linux_jobs.lines().any(|l| l.trim() == line.trim()),
            "the macOS job reuses a pin already used by the Linux jobs: {line}"
        );
    }
    assert!(uses >= 2, "checkout and cache actions expected");
    assert!(!job.contains("--workspace"), "portable crates only");
    assert_eq!(
        packages_of_command(job, "cargo clippy --locked"),
        sorted(&PORTABLE_CRATES),
        "clippy covers exactly the portable set"
    );
    assert!(job.contains("--all-targets --all-features -- -D warnings"));
    let test_line = job
        .lines()
        .find(|l| l.contains("cargo test --locked") && !l.contains("--no-run"))
        .expect("cargo test step");
    assert_eq!(
        packages_of_command(test_line, "cargo test --locked"),
        sorted(&MACOS_TESTED_CRATES)
    );
    assert_eq!(
        packages_of_command(job, "--no-run"),
        sorted(&MACOS_TESTED_CRATES)
    );
    for forbidden in FORBIDDEN_PORTABLE_DEPENDENCIES {
        assert!(
            !job.contains(&format!("-p {forbidden} "))
                && !job.contains(&format!("-p {forbidden}\n")),
            "the macOS job must not build {forbidden}"
        );
    }
    assert!(job.contains("run: ./scripts/prefetch_onnxruntime.sh"));
    let gate = ci_job(&ci, "ci-success");
    let needs = gate
        .lines()
        .find(|l| l.trim_start().starts_with("needs:"))
        .expect("ci-success needs list");
    assert!(
        !needs.contains(MACOS_JOB_ID),
        "the macOS job is non-required and must not be a CI Success dependency: {needs}"
    );
    assert!(
        ci.contains("portable-macos")
            && ci[..ci.find("\njobs:").unwrap()].contains("portable-macos"),
        "the ci.yml header lists the macOS job"
    );
}

/// MAC5: `scripts/pr_loop.sh` treats exactly the macOS job as advisory through
/// `scripts/pr_checks_gate.sh` and stays fail-closed for every other check (spec §4.1, plan
/// evaluation X-2 and R2-1).
#[test]
fn test_mac_pr_loop_treats_only_the_macos_job_as_advisory() {
    let gate = read(GATE_SCRIPT);
    let line = gate
        .lines()
        .find(|l| l.starts_with("readonly ADVISORY_CHECKS=("))
        .expect("advisory allowlist");
    // Contract amendment (GitHub #375, ADR 2026-10-13 "Distribution Builds: System ONNX
    // Runtime"): the system ONNX Runtime job, which depends on live Arch [extra] state, is the
    // second advisory check on pull requests (it stays required on push to main through
    // `ci-success-main`). The macOS job stays advisory; no other check may be added.
    assert_eq!(
        line,
        format!("readonly ADVISORY_CHECKS=(\"{MACOS_JOB_NAME}\" \"{SYSTEM_ORT_JOB_NAME}\")"),
        "exactly two advisory checks: the macOS job and the system ONNX Runtime job"
    );
    assert_eq!(gate.matches("ADVISORY_CHECKS=(").count(), 1);
    assert!(
        gate.contains(GATE_JQ),
        "the gate fetches the count and the records in one gh call"
    );
    assert!(
        gate.contains("[[ \"$name\" == \"$advisory\" ]]"),
        "advisory names are compared as quoted literals (R3-2)"
    );
    assert!(
        gate.contains("|| [[ -n \"$line\" ]]"),
        "a final record without a trailing newline is still read (R3-1)"
    );
    // The advisory name is a job name exactly once across every workflow (R3-2).
    let workflows = workspace_root().join(".github/workflows");
    let mut occurrences = 0;
    for entry in fs::read_dir(workflows).unwrap().flatten() {
        let text = fs::read_to_string(entry.path()).unwrap();
        occurrences += text
            .lines()
            .filter(|l| l.trim() == format!("name: \"{MACOS_JOB_NAME}\""))
            .count();
    }
    assert_eq!(
        occurrences, 1,
        "the advisory job name must be unique across workflows"
    );

    let script = read("scripts/pr_loop.sh");
    assert!(
        script.contains("--watch --fail-fast"),
        "fail-fast watch kept"
    );
    assert!(
        script.contains("check_name=CI%20Success"),
        "CI Success poll kept"
    );
    assert!(
        script.contains("refusing to merge"),
        "CI Success failure exit kept"
    );
    assert!(
        !script.contains("ADVISORY_CHECKS"),
        "the allowlist lives in the gate helper only"
    );
    let watch = script.find("--watch --fail-fast").unwrap();
    let helper = script
        .find("pr_checks_gate.sh")
        .expect("pr_loop calls the gate helper");
    let poll = script.find("check_name=CI%20Success").unwrap();
    assert!(
        watch < helper && helper < poll,
        "order: watch, gate helper, CI Success poll"
    );
    // Exit status captured explicitly under `set -e` (R3-3), and the gate re-checked after a green
    // CI Success, right before the merge (R3-4).
    assert!(
        script
            .matches("gate_rc=0; \"$GATE_HELPER\" \"$PR_NUMBER\" || gate_rc=$?")
            .count()
            >= 2,
        "pr_loop captures the gate status explicitly, after the watch and before the merge"
    );
    let last_gate = script.rfind("|| gate_rc=$?").unwrap();
    let merge = script.find("gh pr merge").expect("merge command");
    assert!(
        poll < last_gate && last_gate < merge,
        "final gate between CI Success and the merge"
    );
    assert!(
        script.contains("WATCH_START"),
        "the pending loop is bounded"
    );
    let ci = read(".github/workflows/ci.yml");
    assert!(ci.contains(&format!("name: \"{MACOS_JOB_NAME}\"")));
}

const GATE_SCRIPT: &str = "scripts/pr_checks_gate.sh";
/// The single `gh --jq` filter of the gate: the record count, then one TSV record per check.
const GATE_JQ: &str =
    "--json name,bucket --jq '(length|tostring), (.[] | [.name, .bucket] | @tsv)'";

/// A shim fixture: the record count line followed by `records` (each terminated by `\n`).
fn fixture(records: &[String]) -> String {
    let mut out = format!("{}\n", records.len());
    for r in records {
        out.push_str(r);
        out.push('\n');
    }
    out
}

/// Runs the gate helper with a `gh` shim printing `fixture` (TSV after gh's `--jq`) and exiting
/// with `gh_status`; returns the helper's exit code and the arguments the shim received.
fn run_gate(tag: &str, fixture: &str, gh_status: i32, args: &[&str]) -> (i32, String) {
    let dir = std::env::temp_dir().join(format!("soos_mac_gate_{}_{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("fixture.tsv"), fixture).unwrap();
    let shim = dir.join("gh");
    fs::write(
        &shim,
        format!(
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" > \"{d}/args\"\ncat \"{d}/fixture.tsv\"\nexit {gh_status}\n",
            d = dir.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new("bash")
        .arg(workspace_root().join(GATE_SCRIPT))
        .args(args)
        .env("PATH", path)
        .output()
        .expect("run the gate helper");
    let received = fs::read_to_string(dir.join("args")).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    (out.status.code().unwrap_or(-1), received)
}

/// MAC5: behaviour of the advisory gate (plan evaluation R2-1, R2-3).
#[test]
fn test_mac_pr_checks_gate_decisions() {
    let adv = MACOS_JOB_NAME;
    let r = |name: &str, bucket: &str| format!("{name}\t{bucket}");
    let cases: Vec<(&str, String, i32, i32)> = vec![
        (
            "adv_fail_rest_pass",
            fixture(&[r("Lint", "pass"), r("Docs", "skipping"), r(adv, "fail")]),
            0,
            0,
        ),
        (
            "adv_fail_other_pending",
            fixture(&[r("Lint", "pass"), r("PR Title", "pending"), r(adv, "fail")]),
            0,
            3,
        ),
        (
            "blocking_fail",
            fixture(&[r("Lint", "fail"), r(adv, "pass")]),
            0,
            1,
        ),
        (
            "blocking_cancel",
            fixture(&[r("Lint", "cancel"), r("Test", "pass"), r(adv, "fail")]),
            0,
            1,
        ),
        (
            "blocking_fail_beats_pending",
            fixture(&[r("Lint", "fail"), r("Test", "pending")]),
            0,
            1,
        ),
        ("empty_list", fixture(&[]), 0, 1),
        ("no_output", String::new(), 0, 1),
        ("only_advisory", fixture(&[r(adv, "pass")]), 0, 1),
        (
            "malformed_line",
            fixture(&["Lint pass".to_string(), r("Test", "pass")]),
            0,
            1,
        ),
        ("unknown_bucket", fixture(&[r("Lint", "neutral")]), 0, 1),
        (
            "extra_field",
            fixture(&["Lint\tpass\textra".to_string()]),
            0,
            1,
        ),
        ("gh_error", fixture(&[r("Lint", "pass")]), 1, 1),
        (
            "all_pass",
            fixture(&[r("Lint", "pass"), r("Test", "pass"), r(adv, "pass")]),
            0,
            0,
        ),
        (
            "near_miss_name",
            fixture(&[r(&format!("{adv} extra"), "fail"), r("Lint", "pass")]),
            0,
            1,
        ),
        (
            "trailing_space_name",
            fixture(&[r(&format!("{adv} "), "fail"), r("Lint", "pass")]),
            0,
            1,
        ),
        (
            "regex_like_name",
            fixture(&[
                r("Portable crates xmacOS, non-requiredx", "fail"),
                r("Lint", "pass"),
            ]),
            0,
            1,
        ),
        (
            "empty_name",
            fixture(&[r("", "pass"), r("Lint", "pass")]),
            0,
            1,
        ),
        (
            "count_mismatch",
            format!("3\n{}\n{}\n", r("Lint", "pass"), r("Test", "pass")),
            0,
            1,
        ),
        (
            "count_not_a_number",
            format!("two\n{}\n{}\n", r("Lint", "pass"), r("Test", "pass")),
            0,
            1,
        ),
        (
            "last_pending_without_newline",
            format!(
                "3\n{}\n{}\n{}",
                r(adv, "fail"),
                r("other", "pass"),
                r("PR Title", "pending")
            ),
            0,
            3,
        ),
        (
            "last_fail_without_newline",
            format!(
                "3\n{}\n{}\n{}",
                r(adv, "fail"),
                r("other", "pass"),
                r("PR Title", "fail")
            ),
            0,
            1,
        ),
    ];
    for (tag, fixture, gh_status, expected) in &cases {
        let (code, received) = run_gate(tag, fixture, *gh_status, &["42"]);
        assert_eq!(code, *expected, "case {tag}: fixture {fixture:?}");
        assert_eq!(
            received.trim(),
            "pr checks 42 --json name,bucket --jq (length|tostring), (.[] | [.name, .bucket] | @tsv)",
            "case {tag}: gh arguments"
        );
    }
    for (tag, args) in [("no_arg", &[][..]), ("bad_arg", &["4x2"][..])] {
        let (code, received) = run_gate(tag, "Lint\tpass\n", 0, args);
        assert_eq!(code, 1, "case {tag}");
        assert!(received.is_empty(), "case {tag}: gh must not be called");
    }
}

/// MAC1: the ADR records the separate-repository decision and the owner decisions.
#[test]
fn test_mac_adr_records_the_separate_repository_decision() {
    let decisions = read("AI/DECISIONS.md");
    let adr = decisions
        .lines()
        .find(|l| l.contains(ADR_TITLE))
        .unwrap_or_else(|| panic!("AI/DECISIONS.md must record \"{ADR_TITLE}\""));
    for needle in [
        "[2026-10-13]",
        "#360",
        "#367",
        "soos-macos",
        "private",
        "shared-v",
        "never a branch",
        "PAM_IGNORE",
        "RGB-only",
        "convenience factor",
        "sudo_local",
        "ad-hoc",
        "notarization",
        "Portable crates (macOS, non-required)",
        "CI Success",
        "crates.io",
        "deferred",
        "aarch64-apple-darwin",
        "`ir`",
        "AGPL",
    ] {
        assert!(adr.contains(needle), "the ADR must mention `{needle}`");
    }
    for crate_name in PORTABLE_CRATES {
        assert!(
            adr.contains(crate_name),
            "the ADR lists portable crate {crate_name}"
        );
    }
}

/// MAC6: `Docs/MACOS_PORT.md` covers the bootstrap specification and is indexed.
#[test]
fn test_mac_macos_port_doc_covers_the_bootstrap_spec() {
    let doc = read("Docs/MACOS_PORT.md");
    for needle in [
        "sudo_local",
        "OpenPAM",
        "SecurityAgent",
        "FileVault",
        "LocalAuthentication",
        "TCC",
        "capture-avf",
        "xpc-transport",
        "pam-openpam",
        "authz-plugin",
        "menu-bar",
        "RGB-only",
        "convenience factor",
        "password fallback",
        "shared-v",
        "ad-hoc",
        "notarization",
        "Owner checklist",
        "aarch64-apple-darwin",
        "Portable crates (macOS, non-required)",
        "flock",
        "fstatat",
        "unlinkat",
        "geteuid",
        "Deferred",
        "AGPL-3.0-or-later",
        "soos-security",
    ] {
        assert!(
            doc.contains(needle),
            "Docs/MACOS_PORT.md must cover `{needle}`"
        );
    }
    for crate_name in PORTABLE_CRATES {
        assert!(
            doc.contains(crate_name),
            "Docs/MACOS_PORT.md lists {crate_name}"
        );
    }
    for forbidden in FORBIDDEN_PORTABLE_DEPENDENCIES {
        assert!(
            doc.contains(&format!("`{forbidden}`")),
            "Docs/MACOS_PORT.md documents forbidden crate `{forbidden}`"
        );
    }
    let index = read("Docs/README.md");
    assert!(index.contains("(MACOS_PORT.md)") && index.contains("(FRAME_CRATE.md)"));
}
