//! Static contracts of the distribution build against the system ONNX Runtime (GitHub #375,
//! PR 1 of #361; ADR "[2026-10-13] Distribution Builds: System ONNX Runtime"; spec
//! `AI/architect_spec_system_onnxruntime.md`; matrix SORT-S1 to SORT-S7):
//!
//! - the workspace `ort` entry carries only the features both builds need, every edge to an
//!   ONNX Runtime crate disables default features, and the five ONNX Runtime crates declare
//!   `download-binaries` (default) and `system-onnxruntime` exactly; `ort` stays CPU only;
//! - `rust-version` (MSRV) is declared, inherited by every member and not newer than the
//!   pinned toolchain;
//! - the resolved feature graph: the default build is unchanged (the downloaded, SHA-256
//!   verified archive), the system build is `pkg-config` + `api-17` with no download code;
//! - the daemon runs the CPU execution provider check before any other start-up step;
//! - the offline Arch CI job, its harness and its image;
//! - `pam_soos.so` never depends on ONNX Runtime;
//! - ADR and documentation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CI: &str = ".github/workflows/ci.yml";
const HARNESS: &str = "tests/docker/system_onnxruntime_test.sh";
const DOCKERFILE: &str = "tests/docker/Dockerfile.system-ort";
const ARCH_DOCKERFILE: &str = "tests/docker/Dockerfile.arch";
const PIPELINE: &str = "crates/daemon/src/pipeline.rs";
const RUNTIME: &str = "crates/inference-ort/src/runtime.rs";
const ENROLL_SERVICE: &str = "crates/enrollment-cli/src/service.rs";
const GUI_MAIN: &str = "crates/gui/src/main.rs";
const DECISIONS: &str = "AI/DECISIONS.md";
const PACKAGING_DOC: &str = "Docs/PACKAGING_AND_PROVISIONING.md";
const INFERENCE_DOC: &str = "Docs/INFERENCE_ORT_CRATE.md";
const CI_DOC: &str = "Docs/CI_CD_AND_SECURITY.md";
const GATE_SCRIPT: &str = "scripts/pr_checks_gate.sh";

/// `name:` of the system ONNX Runtime job: advisory on pull requests (listed in
/// `scripts/pr_checks_gate.sh`), required on push to `main` through [`MAIN_GATE_NAME`].
const SORT_JOB_NAME: &str = "System ONNX Runtime (Arch, offline, advisory on PRs)";

/// `name:` of the push-only aggregate that requires the system ONNX Runtime job on `main`.
const MAIN_GATE_NAME: &str = "CI Success (main, system ONNX Runtime)";

/// Declared minimum supported Rust version (spec §0: highest `rust-version` of the locked
/// dependencies, egui 0.35; proven by the CI job building with exactly 1.92.0).
const MSRV: &str = "1.92";

/// The exact workspace `ort` entry: both builds share only these features.
const WORKSPACE_ORT_LINE: &str = "ort = { version = \"=2.0.0-rc.13\", default-features = false, \
                                  features = [\"std\", \"ndarray\", \"tracing\"] }";

/// The five crates whose build graph contains `ort` (`cargo tree -i ort`).
const ORT_CRATES: [(&str, &str); 5] = [
    ("crates/inference-ort/Cargo.toml", "soos-inference-ort"),
    ("crates/vision/Cargo.toml", "soos-vision"),
    ("crates/daemon/Cargo.toml", "soos-daemon"),
    ("crates/enrollment-cli/Cargo.toml", "soos-enrollment-cli"),
    ("crates/gui/Cargo.toml", "soos-gui"),
];

/// `ort-sys` features of the default (downloaded archive) build: `ort`'s own defaults.
const DEFAULT_ORT_SYS_FEATURES: [&str; 16] = [
    "__tls",
    "api-17",
    "api-18",
    "api-19",
    "api-20",
    "api-21",
    "api-22",
    "api-23",
    "api-24",
    "api-25",
    "api-26",
    "api-27",
    "copy-dylibs",
    "download-binaries",
    "std",
    "tls-native",
];

/// `ort-sys` features of the system build.
const SYSTEM_ORT_SYS_FEATURES: [&str; 3] = ["api-17", "pkg-config", "std"];

/// `ort` features that would leave the CPU-only, linked-at-build-time design.
const FORBIDDEN_ORT_FEATURES: [&str; 22] = [
    "load-dynamic",
    "preload-dylibs",
    "alternative-backend",
    "cuda",
    "tensorrt",
    "rocm",
    "migraphx",
    "directml",
    "coreml",
    "openvino",
    "onednn",
    "nnapi",
    "qnn",
    "cann",
    "vitis",
    "webgpu",
    "xnnpack",
    "armnn",
    "acl",
    "tvm",
    "rknpu",
    "nvrtx",
];

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

/// Lines of the TOML table `[name]` (up to the next table header).
fn toml_table<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("[{name}]");
    let mut lines = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == header;
            continue;
        }
        if inside && !trimmed.is_empty() && !trimmed.starts_with('#') {
            lines.push(trimmed);
        }
    }
    lines
}

/// Workspace member directories from the root `[workspace] members` array.
fn workspace_members() -> Vec<String> {
    let root = read("Cargo.toml");
    let start = root.find("members = [").expect("members array");
    let rest = &root[start..];
    let end = rest.find(']').expect("members end");
    rest[..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim().trim_end_matches(',');
            line.strip_prefix('"')
                .and_then(|l| l.strip_suffix('"'))
                .map(str::to_string)
        })
        .collect()
}

/// Lines of one CI job (from `  <name>:` to the next section comment).
fn ci_job<'a>(ci: &'a str, name: &str) -> &'a str {
    let start = ci
        .find(&format!("\n  {name}:"))
        .unwrap_or_else(|| panic!("ci.yml must define the {name} job"));
    let job = &ci[start + 1..];
    let end = job.find("\n  # ----").unwrap_or(job.len());
    &job[..end]
}

/// Shell code without comment lines.
fn code_lines(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Resolved `ort-sys` feature sets in `cargo tree <scope> -e normal,build,dev`, one per
/// `ort-sys` line, or `None` when cargo is unavailable.
fn ort_sys_features(scope: &[&str]) -> Option<Vec<BTreeSet<String>>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    if Command::new(&cargo).arg("--version").output().is_err() {
        return None;
    }
    let mut args = vec!["tree", "--offline", "--locked"];
    args.extend_from_slice(scope);
    args.extend_from_slice(&[
        "-e",
        "normal,build,dev",
        "--prefix",
        "none",
        "--format",
        "{p}|{f}",
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
    let mut sets: Vec<BTreeSet<String>> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("ort-sys v"))
        .map(|line| {
            let features = line.split_once('|').map_or("", |(_, f)| f);
            let features = features.split_whitespace().next().unwrap_or("");
            features
                .split(',')
                .filter(|f| !f.is_empty())
                .map(str::to_string)
                .collect()
        })
        .collect();
    sets.dedup();
    Some(sets)
}

/// Package names in `cargo tree <scope> -e normal,build`, or `None` without cargo.
fn tree_package_names(scope: &[&str]) -> Option<BTreeSet<String>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    if Command::new(&cargo).arg("--version").output().is_err() {
        return None;
    }
    let mut args = vec!["tree", "--offline", "--locked"];
    args.extend_from_slice(scope);
    args.extend_from_slice(&["-e", "normal,build", "--prefix", "none", "--format", "{p}"]);
    let output = Command::new(&cargo)
        .current_dir(workspace_root())
        .args(&args)
        .output()
        .expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo {} failed: {}",
        args.join(" "),
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

fn as_set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

/// SORT-S1: the workspace `ort` entry, the `default-features = false` edges and the exact
/// feature tables of the five ONNX Runtime crates.
#[test]
fn test_sort_s1_workspace_ort_entry_and_feature_tables() {
    let root = read("Cargo.toml");
    let deps = toml_table(&root, "workspace.dependencies");
    let ort: Vec<&&str> = deps.iter().filter(|l| l.starts_with("ort ")).collect();
    assert_eq!(ort.len(), 1, "one workspace ort entry: {ort:?}");
    assert_eq!(
        ort[0].split_whitespace().collect::<Vec<_>>().join(" "),
        WORKSPACE_ORT_LINE
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        "the workspace ort entry carries only the features both builds need"
    );
    for name in ["soos-inference-ort", "soos-vision", "soos-enrollment-cli"] {
        let line = deps
            .iter()
            .find(|l| l.starts_with(&format!("{name} ")))
            .unwrap_or_else(|| panic!("workspace entry {name}"));
        assert!(
            line.contains("default-features = false"),
            "{name} must not re-enable the download through its default features: {line}"
        );
    }

    let forward = |crates: &[&str], feature: &str| -> String {
        let items: Vec<String> = crates
            .iter()
            .map(|c| format!("\"{c}/{feature}\""))
            .collect();
        format!("{feature} = [{}]", items.join(", "))
    };
    let expected: [(&str, [String; 3]); 5] = [
        (
            "crates/inference-ort/Cargo.toml",
            [
                "default = [\"download-binaries\"]".to_string(),
                "download-binaries = [\"ort/default\"]".to_string(),
                "system-onnxruntime = [\"ort/pkg-config\", \"ort/api-17\"]".to_string(),
            ],
        ),
        (
            "crates/vision/Cargo.toml",
            [
                "default = [\"download-binaries\"]".to_string(),
                forward(&["soos-inference-ort"], "download-binaries"),
                forward(&["soos-inference-ort"], "system-onnxruntime"),
            ],
        ),
        (
            "crates/daemon/Cargo.toml",
            [
                "default = [\"download-binaries\"]".to_string(),
                forward(&["soos-inference-ort", "soos-vision"], "download-binaries"),
                forward(&["soos-inference-ort", "soos-vision"], "system-onnxruntime"),
            ],
        ),
        (
            "crates/enrollment-cli/Cargo.toml",
            [
                "default = [\"download-binaries\"]".to_string(),
                forward(&["soos-inference-ort", "soos-vision"], "download-binaries"),
                forward(&["soos-inference-ort", "soos-vision"], "system-onnxruntime"),
            ],
        ),
        (
            "crates/gui/Cargo.toml",
            [
                "default = [\"download-binaries\"]".to_string(),
                forward(
                    &["soos-inference-ort", "soos-vision", "soos-enrollment-cli"],
                    "download-binaries",
                ),
                forward(
                    &["soos-inference-ort", "soos-vision", "soos-enrollment-cli"],
                    "system-onnxruntime",
                ),
            ],
        ),
    ];
    for (path, lines) in &expected {
        let text = read(path);
        let features = toml_table(&text, "features");
        for line in lines {
            assert!(
                features.contains(&line.as_str()),
                "{path} [features] must contain `{line}`; found {features:?}"
            );
        }
        for key in ["default", "download-binaries", "system-onnxruntime"] {
            let count = features
                .iter()
                .filter(|l| l.starts_with(&format!("{key} =")))
                .count();
            assert_eq!(count, 1, "{path} declares `{key}` exactly once");
        }
        // Every edge to an ONNX Runtime crate goes through the workspace entry (which
        // disables default features); a local `default-features = true` would re-enable it.
        for dep in [
            "soos-inference-ort",
            "soos-vision",
            "soos-enrollment-cli",
            "ort ",
        ] {
            for line in text.lines().filter(|l| l.trim_start().starts_with(dep)) {
                assert!(
                    line.contains("workspace = true") && !line.contains("default-features"),
                    "{path}: `{}` must be a plain workspace dependency",
                    line.trim()
                );
            }
        }
    }
    assert_eq!(ORT_CRATES.len(), expected.len());
}

/// SORT-S1: `ort` stays CPU only and linked at build time: no manifest enables dynamic
/// loading, an alternative backend or any GPU/NPU execution provider.
#[test]
fn test_sort_s1_ort_stays_cpu_only() {
    let mut manifests = vec!["Cargo.toml".to_string()];
    manifests.extend(
        workspace_members()
            .iter()
            .map(|m| format!("{m}/Cargo.toml")),
    );
    for path in &manifests {
        let text = read(path);
        for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
            let mentions_ort = line.contains("ort/") || line.trim_start().starts_with("ort ");
            if !mentions_ort {
                continue;
            }
            for feature in FORBIDDEN_ORT_FEATURES {
                assert!(
                    !line.contains(&format!("\"ort/{feature}\""))
                        && !line.contains(&format!("\"{feature}\"")),
                    "{path}: ort feature `{feature}` is forbidden (ort CPU only, no dlopen): {line}"
                );
            }
        }
    }
}

/// SORT-S1: CPU only in source: no crate registers an execution provider and the `ort::ep`
/// module is used only by the start-up probe.
#[test]
fn test_sort_s1_no_execution_provider_registration_in_source() {
    let root = workspace_root();
    let mut files = Vec::new();
    for member in workspace_members()
        .into_iter()
        .filter(|m| m.starts_with("crates/"))
    {
        let src = root.join(&member).join("src");
        if src.is_dir() {
            collect_rs(&src, &mut files);
        }
    }
    assert!(!files.is_empty());
    for file in files {
        let rel = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("with_execution_providers"),
            "{rel}: soos never registers an execution provider (CPU only)"
        );
        if rel != RUNTIME {
            for token in ["ep::", "ExecutionProvider", "ort::ep"] {
                assert!(
                    !contains_token(&text, token),
                    "{rel}: `{token}` (execution providers) is used only by {RUNTIME}"
                );
            }
        }
    }
}

/// `token` occurs at an identifier boundary (not preceded by an identifier character), so
/// `ep::` matches `use ort::{ep::CUDA}` and `ort::ep::CPU` but not `EnrollmentStep::Frontal`.
fn contains_token(text: &str, token: &str) -> bool {
    text.match_indices(token).any(|(at, _)| {
        text[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
    })
}

#[test]
fn test_sort_s1_token_scan_is_identifier_aware() {
    assert!(contains_token("use ort::{ep::CUDA, session};", "ep::"));
    assert!(contains_token("ort::ep::CPU::default()", "ep::"));
    assert!(!contains_token("EnrollmentStep::Frontal", "ep::"));
    assert!(contains_token(
        "impl ExecutionProvider for X",
        "ExecutionProvider"
    ));
    assert!(!contains_token("MyExecutionProvider", "ExecutionProvider"));
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// SORT-S2: `rust-version` is declared once, inherited by every member and not newer than the
/// exact release pinned by `rust-toolchain.toml`.
#[test]
fn test_sort_s2_msrv_declared_inherited_and_not_newer_than_toolchain() {
    let root = read("Cargo.toml");
    let package = toml_table(&root, "workspace.package");
    assert!(
        package.contains(&format!("rust-version = \"{MSRV}\"").as_str()),
        "[workspace.package] must declare rust-version = \"{MSRV}\": {package:?}"
    );
    let members = workspace_members();
    assert!(members.len() >= 17, "workspace members: {members:?}");
    for member in &members {
        let text = read(&format!("{member}/Cargo.toml"));
        let pkg = toml_table(&text, "package");
        assert!(
            pkg.contains(&"rust-version.workspace = true"),
            "{member}/Cargo.toml must inherit rust-version.workspace = true"
        );
    }

    let toolchain = read("rust-toolchain.toml");
    let channel = toolchain
        .lines()
        .find_map(|l| l.trim().strip_prefix("channel = "))
        .map(|v| v.trim_matches('"').to_string())
        .expect("rust-toolchain.toml channel");
    let parse = |v: &str| -> Vec<u32> {
        v.split('.')
            .map(|p| p.parse::<u32>().expect("numeric version component"))
            .collect()
    };
    let mut msrv = parse(MSRV);
    msrv.resize(3, 0);
    let pinned = parse(&channel);
    assert!(
        msrv <= pinned,
        "rust-version {MSRV} must not be newer than the pinned toolchain {channel}"
    );
}

/// SORT-S3: the default build resolves `ort-sys` exactly as before this change (the
/// downloaded, SHA-256 verified archive at API 27).
#[test]
fn test_sort_s3_default_feature_graph_unchanged() {
    let Some(sets) = ort_sys_features(&["--workspace"]) else {
        return;
    };
    assert_eq!(
        sets,
        vec![as_set(&DEFAULT_ORT_SYS_FEATURES)],
        "default workspace build must keep ort's own defaults"
    );
    for (_, package) in ORT_CRATES {
        let sets = ort_sys_features(&["-p", package]).expect("cargo available");
        assert_eq!(
            sets,
            vec![as_set(&DEFAULT_ORT_SYS_FEATURES)],
            "default build of {package} must download the verified archive"
        );
    }
}

/// SORT-S3: the system build links through pkg-config at API 17 and compiles no download
/// code, for the whole workspace and for each ONNX Runtime crate built alone.
#[test]
fn test_sort_s3_system_feature_graph_is_pkg_config_api17_only() {
    let system = ["--no-default-features", "--features", "system-onnxruntime"];
    let mut scope = vec!["--workspace"];
    scope.extend_from_slice(&system);
    let Some(sets) = ort_sys_features(&scope) else {
        return;
    };
    assert_eq!(sets, vec![as_set(&SYSTEM_ORT_SYS_FEATURES)]);
    for (_, package) in ORT_CRATES {
        let mut scope = vec!["-p", package];
        scope.extend_from_slice(&system);
        let sets = ort_sys_features(&scope).expect("cargo available");
        assert_eq!(
            sets,
            vec![as_set(&SYSTEM_ORT_SYS_FEATURES)],
            "system build of {package} must not compile the download"
        );
    }
}

/// SORT-S4: `initialize_pipeline` runs the CPU execution provider check before the camera,
/// keys, stores and models.
#[test]
fn test_sort_s4_daemon_checks_cpu_provider_first() {
    let src = read(PIPELINE);
    let start = src
        .find("pub fn initialize_pipeline(")
        .expect("initialize_pipeline");
    let body = &src[start..];
    let check = body
        .find("soos_inference_ort::verify_cpu_execution_provider()?")
        .expect("initialize_pipeline must call verify_cpu_execution_provider()?");
    for later in [
        "resolve_pipeline_camera(",
        "MasterKey::load_or_create(",
        "ModelRegistry::new(",
        "verify_integrity()",
    ] {
        let at = body
            .find(later)
            .unwrap_or_else(|| panic!("initialize_pipeline calls {later}"));
        assert!(
            check < at,
            "the CPU execution provider check must run before {later}"
        );
    }
}

/// SORT-S4: the probe asks the library by name: a never-registered `ExecutionProvider` that
/// keeps the default `is_available` (`GetAvailableProviders`), never `ep::CPU` (whose
/// `is_available` answers `true` without asking), and `verify_cpu_execution_provider` goes
/// through it with `CPU_EXECUTION_PROVIDER`.
#[test]
fn test_sort_s4_probe_asks_the_library_by_name() {
    let src = read(RUNTIME);
    assert!(src.contains("impl ExecutionProvider for ListedProviderProbe"));
    assert!(
        !src.contains("fn is_available"),
        "the probe must keep the default is_available (GetAvailableProviders)"
    );
    assert!(
        !src.contains("ep::CPU"),
        "ep::CPU::is_available never asks the library"
    );
    let verify = src
        .find("pub fn verify_cpu_execution_provider()")
        .expect("verify_cpu_execution_provider");
    let body = &src[verify..];
    let body = &body[..body.find("\n}").expect("end of verify")];
    assert!(
        body.contains("execution_provider_listed(CPU_EXECUTION_PROVIDER)"),
        "verify_cpu_execution_provider must probe CPU_EXECUTION_PROVIDER by name:\n{body}"
    );
    assert!(
        src.contains("catch_unwind"),
        "the handshake panic is caught"
    );
}

/// SORT-S4: `soos-enroll` (root) and `soos-gui` run the same check before they open the
/// camera or build any ORT session.
#[test]
fn test_sort_s4_enroll_and_gui_check_cpu_provider_before_sessions() {
    let enroll = read(ENROLL_SERVICE);
    let start = enroll
        .find("pub fn build_full_service_with_notes(")
        .expect("build_full_service_with_notes");
    let body = &enroll[start..];
    let check = body
        .find("soos_inference_ort::verify_cpu_execution_provider()?")
        .expect("soos-enroll must run verify_cpu_execution_provider()?");
    for later in [
        "MasterKey::load_or_create(",
        "BiometricStore::new(",
        "V4lCameraManager::spawn(",
        "ModelRegistry::new(",
    ] {
        let at = body
            .find(later)
            .unwrap_or_else(|| panic!("enroll calls {later}"));
        assert!(check < at, "soos-enroll checks the provider before {later}");
    }
    let gui = read(GUI_MAIN);
    let check = gui
        .find("soos_inference_ort::verify_cpu_execution_provider()?")
        .expect("soos-gui must run verify_cpu_execution_provider()?");
    let registry = gui.find("ModelRegistry::new(").expect("gui registry");
    assert!(
        check < registry,
        "soos-gui checks the provider before ModelRegistry::new"
    );
}

/// SORT-S5: the CI job runs the offline Arch harness on pull requests and gates CI Success.
#[test]
fn test_sort_s5_ci_job_runs_offline_arch_harness() {
    let ci = read(CI);
    let job = ci_job(&ci, "system-onnxruntime");
    for needle in [
        "if: github.event_name != 'schedule'",
        "needs: lint",
        "timeout-minutes: 75",
        "persist-credentials: false",
        "run: ./tests/docker/system_onnxruntime_test.sh",
    ] {
        assert!(
            job.contains(needle),
            "system-onnxruntime job needs `{needle}`:\n{job}"
        );
    }
    assert!(
        job.contains(&format!("name: \"{SORT_JOB_NAME}\"")),
        "system-onnxruntime job name: {SORT_JOB_NAME}"
    );
    assert!(
        !job.contains("continue-on-error"),
        "a failure must stay visible (red check), never be swallowed"
    );
    assert!(
        !job.contains("${{"),
        "system-onnxruntime must not interpolate expressions into its steps"
    );
    for line in job
        .lines()
        .filter(|l| l.trim_start().starts_with("- uses:"))
    {
        let reference = line.split('@').nth(1).unwrap_or("");
        let sha: String = reference
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect();
        assert_eq!(
            sha.len(),
            40,
            "actions must be pinned by full commit SHA: {line}"
        );
    }
    // Every --all-features job resolves the verified download only (no mixed link with an
    // older distro library found through pkg-config).
    let env_block =
        &ci[ci.find("\nenv:\n").expect("workflow env")..ci.find("\njobs:").expect("jobs")];
    assert!(
        env_block.contains("LIBONNXRUNTIME_NO_PKG_CONFIG: \"1\""),
        "workflow env must set LIBONNXRUNTIME_NO_PKG_CONFIG: \"1\""
    );
    // save.sh runs the same --all-features gates locally: same environment, before its first
    // cargo gate (auditor constraint 17).
    let save = read("save.sh");
    let export = save
        .find("export LIBONNXRUNTIME_NO_PKG_CONFIG=\"${LIBONNXRUNTIME_NO_PKG_CONFIG:-1}\"")
        .expect("save.sh must export LIBONNXRUNTIME_NO_PKG_CONFIG");
    let first_gate = save
        .find("cargo clippy --locked")
        .expect("save.sh clippy gate");
    assert!(
        export < first_gate,
        "save.sh exports the variable before its cargo gates"
    );
    // Advisory on pull requests: not a dependency of the PR-time CI Success aggregate, and
    // listed as an advisory check of the merge gate (it depends on live Arch [extra] state).
    let gate = ci_job(&ci, "ci-success");
    let needs = gate
        .lines()
        .find(|l| l.trim_start().starts_with("needs:"))
        .expect("ci-success needs list");
    assert!(
        !needs.contains("system-onnxruntime"),
        "system-onnxruntime is advisory on pull requests, not a CI Success dependency: {needs}"
    );
    let advisory = read(GATE_SCRIPT);
    let line = advisory
        .lines()
        .find(|l| l.starts_with("readonly ADVISORY_CHECKS=("))
        .expect("advisory allowlist");
    assert!(
        line.contains(&format!("\"{SORT_JOB_NAME}\"")),
        "{GATE_SCRIPT} lists the system ONNX Runtime job as advisory: {line}"
    );
    // Required on push to main: a push-only aggregate needs CI Success and the job, fails on
    // anything but success, and is skipped (never blocking) on pull requests.
    let main_gate = ci_job(&ci, "ci-success-main");
    for needle in [
        format!("name: \"{MAIN_GATE_NAME}\""),
        "if: always() && (github.event_name == 'push' || github.event_name == 'workflow_dispatch')"
            .to_string(),
        "needs: [ci-success, system-onnxruntime]".to_string(),
        "RESULTS: ${{ join(needs.*.result, ' ') }}".to_string(),
        "\"$r\" != \"success\"".to_string(),
        "exit 1".to_string(),
    ] {
        assert!(
            main_gate.contains(&needle),
            "ci-success-main needs `{needle}`:\n{main_gate}"
        );
    }
    assert!(!main_gate.contains("continue-on-error"));
    // Both names are unique job names across the workflows (the gate compares literals).
    let workflows = workspace_root().join(".github/workflows");
    for name in [SORT_JOB_NAME, MAIN_GATE_NAME] {
        let mut occurrences = 0;
        for entry in fs::read_dir(&workflows).unwrap().flatten() {
            let text = fs::read_to_string(entry.path()).unwrap();
            occurrences += text
                .lines()
                .filter(|l| l.trim() == format!("name: \"{name}\""))
                .count();
        }
        assert_eq!(
            occurrences, 1,
            "job name `{name}` must be unique across workflows"
        );
    }
}

/// SORT-S5: the harness fetches online in a separate step, then builds and tests with the
/// network disabled, offline and locked, against the system library and the real models.
#[test]
fn test_sort_s5_harness_builds_offline_with_system_library() {
    let text = read(HARNESS);
    assert!(text.starts_with("#!/usr/bin/env bash"), "bash shebang");
    assert!(text.contains("set -euo pipefail"), "strict mode");
    let code = code_lines(&text);
    for needle in [
        "--network none",
        "ORT_SKIP_DOWNLOAD=1",
        "CARGO_NET_OFFLINE=true",
        "cargo fetch --locked",
        "scripts/download_models.sh",
        "cargo build --locked --offline --release --workspace --no-default-features --features system-onnxruntime",
        "SOOS_REQUIRE_REAL_MODELS=1",
        "--test pad_real_model_tests",
        "--test embedding_real_model_tests",
        "--test scrfd_real_model_tests",
        "--test pad_crop_real_model_tests",
        "--test pad_nonface_pipeline_real_model_tests",
        "--test system_onnxruntime_startup_tests",
        "--test system_onnxruntime_tests",
        "--test scrfd_hot_path_real_model_tests",
        "--test pad_output_contract_tests",
        "--test pad_input_range_tests",
        "cargo check --locked --offline --workspace --all-targets --all-features",
        "LIBONNXRUNTIME_NO_PKG_CONFIG",
        "RUSTUP_TOOLCHAIN",
        "rust-version",
        "ort.pyke.io",
        "/sys/class/net",
        "readelf -d",
        "libonnxruntime.so",
        "libpam_soos.so",
        "pacman -Qo",
        "pacman -Rdd --noconfirm onnxruntime-cpu",
        "--version",
        "-e features -i ort-sys",
        "SOOS_SORT_BUILD_ATTEMPTS",
        "download-binaries",
        DOCKERFILE,
    ] {
        assert!(code.contains(needle), "{HARNESS} must contain `{needle}`");
    }
    // Every offline cargo invocation is locked and offline.
    for line in code.lines() {
        let t = line.trim();
        if t.contains("cargo build")
            || t.contains("cargo test")
            || t.contains("cargo tree")
            || t.contains("cargo check")
        {
            assert!(
                t.contains("--locked") && t.contains("--offline"),
                "offline cargo commands use --locked --offline: {t}"
            );
        }
    }
    // The fetch runs before the first network-less container, the build after it.
    let fetch = code.find("cargo fetch --locked").expect("fetch");
    let offline = code.find("--network none").expect("offline run");
    let build = code.find("cargo build --locked --offline").expect("build");
    assert!(
        fetch < offline && offline < build,
        "fetch, then offline build"
    );
    assert!(
        !code.contains("ORT_LIB_LOCATION=") && !code.contains("ORT_LIB_PATH="),
        "the system build links through pkg-config only (no ORT_LIB_LOCATION override)"
    );
    assert!(
        !code.contains("curl ") || !code.contains("| sh"),
        "no curl | sh in the harness"
    );
}

/// SORT-S5: the image is the digest-pinned Arch base of `Dockerfile.arch`, installs
/// `onnxruntime-cpu` from the repositories and Rust only through the verified installer at the
/// MSRV passed as a build argument.
#[test]
fn test_sort_s5_dockerfile_pinned_arch_with_onnxruntime_cpu() {
    let text = read(DOCKERFILE);
    let arch = read(ARCH_DOCKERFILE);
    let from_of = |t: &str| -> Vec<String> {
        t.lines()
            .filter(|l| l.starts_with("FROM "))
            .map(str::to_string)
            .collect()
    };
    let from = from_of(&text);
    assert_eq!(from.len(), 1, "single-stage image: {from:?}");
    assert_eq!(
        from,
        from_of(&arch),
        "the system ONNX Runtime image uses the same archlinux digest as Dockerfile.arch"
    );
    let digest = from[0].split("@sha256:").nth(1).expect("digest pin");
    assert!(
        digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()),
        "64-hex digest: {digest}"
    );
    let code = code_lines(&text);
    for needle in [
        "onnxruntime-cpu",
        "pkgconf",
        "ARG SOOS_MSRV",
        "install_rustup.sh --default-toolchain \"${SOOS_MSRV}\"",
    ] {
        assert!(
            code.contains(needle),
            "{DOCKERFILE} must contain `{needle}`"
        );
    }
    assert!(
        !code.contains("ARG SOOS_MSRV="),
        "the MSRV has no default: the harness passes it from Cargo.toml"
    );
    assert!(
        !code.contains("| sh") && !code.contains("|sh"),
        "no curl | sh"
    );
    assert!(
        !code.contains("onnxruntime-opt") && !code.contains("onnxruntime-cuda"),
        "CPU build of ONNX Runtime only"
    );
}

/// SORT-S6: `pam_soos.so` never links or loads ONNX Runtime, in the default, all-features and
/// system builds.
#[test]
fn test_sort_s6_pam_graph_has_no_onnx_runtime() {
    let scopes: [&[&str]; 3] = [
        &["-p", "soos-pam"],
        &["-p", "soos-pam", "--all-features"],
        &["-p", "soos-pam", "--no-default-features"],
    ];
    for scope in scopes {
        let Some(names) = tree_package_names(scope) else {
            return;
        };
        assert!(names.contains("soos-pam"), "{scope:?}: {names:?}");
        for forbidden in ["ort", "ort-sys", "soos-inference-ort", "tokio"] {
            assert!(
                !names.contains(forbidden),
                "soos-pam must not depend on {forbidden} ({scope:?})"
            );
        }
    }
}

/// SORT-S7: the ADR records the decision, the evidence, the trust change and the rejected
/// alternatives; the docs give the build command and the startup check.
#[test]
fn test_sort_s7_adr_and_docs() {
    let decisions = read(DECISIONS);
    let adr = decisions
        .lines()
        .find(|l| l.contains("Distribution Builds: System ONNX Runtime"))
        .expect("ADR \"Distribution Builds: System ONNX Runtime\"");
    for needle in [
        "#375",
        "system-onnxruntime",
        "download-binaries",
        "pkg-config",
        "api-17",
        "rust-version",
        "1.92",
        "--no-default-features",
        "--all-features",
        "verify_cpu_execution_provider",
        "load-dynamic",
        "dlopen",
        "prebuilt",
        "post-install",
        "SHA-256",
        "distro-signed",
        "onnxruntime-cpu",
        "--network none",
    ] {
        assert!(adr.contains(needle), "the ADR must mention `{needle}`");
    }
    let packaging = read(PACKAGING_DOC);
    assert!(
        packaging.contains(
            "ORT_SKIP_DOWNLOAD=1 cargo build --locked --offline --release --workspace \
             --no-default-features --features system-onnxruntime"
        ),
        "{PACKAGING_DOC} gives the exact distribution build command"
    );
    let inference = read(INFERENCE_DOC);
    for needle in [
        "verify_cpu_execution_provider",
        "check_cpu_execution_provider",
        "CpuExecutionProviderUnavailable",
        "system-onnxruntime",
    ] {
        assert!(
            inference.contains(needle),
            "{INFERENCE_DOC} documents `{needle}`"
        );
    }
    let ci_doc = read(CI_DOC);
    assert!(
        ci_doc.contains("system-onnxruntime") && ci_doc.contains(HARNESS),
        "{CI_DOC} documents the system ONNX Runtime job"
    );
}
