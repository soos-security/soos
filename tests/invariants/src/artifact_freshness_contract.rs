//! Build-artifact and review-artifact freshness (GitHub #244 / TCI-13, #267 / TCI-14,
//! rows QFX1–QFX5).
//!
//! - No Docker or distribution harness deploys or packages a release artifact only because it
//!   already exists in the bind-mounted `target/release/`: unless the caller explicitly passes
//!   `--skip-build`, the harness always runs `cargo build --locked --release --workspace`
//!   (a no-op when the artifacts are up to date).
//! - The orphan `AI/plan_evaluation_report.md` is gone and nothing points at it.
//! - The candid fingerprint (`scripts/candid_subagent.sh`) excludes both review-report
//!   singletons, `AI/candid_review_report.md` and `AI/plan_evaluator_report.md`, so rewriting
//!   either report never changes the reviewed diff.
//! - The candid fingerprint is portable for binary files: it does not depend on the zlib build
//!   or `core.compression` (a `git diff --binary` patch does), yet still changes with the
//!   binary content (GitHub #339, CI fingerprint mismatch on `apple-touch-icon.png`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

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

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

// ---------------------------------------------------------------------------
// QFX1 — harnesses never reuse an existing release artifact without rebuilding
// ---------------------------------------------------------------------------

/// Harnesses that package or install the release binaries from `target/release/`.
const RELEASE_HARNESSES: &[&str] = &[
    "tests/docker/test_packages.sh",
    "tests/distro/debian_ubuntu_test.sh",
    "tests/distro/fedora_rhel_test.sh",
    "tests/distro/arch_linux_test.sh",
];

const WORKSPACE_BUILD: &str = "cargo build --locked --release --workspace";

fn shell_scripts_under(rel_dir: &str) -> Vec<(String, String)> {
    let dir = workspace_root().join(rel_dir);
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir {rel_dir}: {e}")) {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("sh") {
            let rel = format!(
                "{rel_dir}/{}",
                path.file_name()
                    .and_then(|n| n.to_str())
                    .expect("utf-8 name")
            );
            out.push((rel.clone(), read(&rel)));
        }
    }
    out.sort();
    out
}

/// An `if` whose condition only asks whether a `target/.../release/` artifact is missing:
/// the build it guards is skipped whenever a (possibly stale) artifact exists.
fn is_existence_guard(line: &str) -> bool {
    let t = line.trim_start();
    !is_comment(t)
        && (t.starts_with("if ") || t.starts_with("elif "))
        && t.contains("! -f")
        && t.contains("release/")
}

#[test]
fn test_harnesses_never_skip_the_release_build_because_an_artifact_exists() {
    let mut offenders = Vec::new();
    for dir in ["tests/docker", "tests/distro"] {
        for (rel, text) in shell_scripts_under(dir) {
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if !is_existence_guard(line) {
                    continue;
                }
                let guarded_build = lines
                    .iter()
                    .skip(i + 1)
                    .take_while(|l| l.trim() != "fi")
                    .any(|l| !is_comment(l) && l.contains("cargo build"));
                if guarded_build {
                    offenders.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a harness skips `cargo build` when a release artifact already exists, so a stale \
         host-built binary from the bind mount would be packaged or deployed (GitHub #244):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn test_release_harnesses_always_run_the_locked_workspace_build() {
    for rel in RELEASE_HARNESSES {
        let text = read(rel);
        let builds: Vec<&str> = text
            .lines()
            .filter(|l| !is_comment(l) && l.trim() == WORKSPACE_BUILD)
            .collect();
        assert!(
            !builds.is_empty(),
            "{rel} must run `{WORKSPACE_BUILD}` before it packages the release artifacts"
        );
    }
}

// ---------------------------------------------------------------------------
// QFX2 — orphan plan report removed
// ---------------------------------------------------------------------------

#[test]
fn test_orphan_plan_evaluation_report_is_removed() {
    assert!(
        !workspace_root()
            .join("AI")
            .join("plan_evaluation_report.md")
            .exists(),
        "AI/plan_evaluation_report.md is an orphan (the plan-evaluator skill writes \
         AI/plan_evaluator_report.md); delete it (GitHub #267)"
    );
    for rel in [
        ".agents/skills/plan-evaluator/SKILL.md",
        ".agents/skills/dev-workflow/SKILL.md",
        "Docs/CI_CD_AND_SECURITY.md",
        "Docs/DEVELOPMENT_WORKFLOW.md",
        "scripts/candid_subagent.sh",
    ] {
        assert!(
            !read(rel).contains("plan_evaluation_report.md"),
            "{rel} still names the removed AI/plan_evaluation_report.md"
        );
    }
}

// ---------------------------------------------------------------------------
// QFX3 / QFX4 — candid fingerprint excludes both report singletons
// ---------------------------------------------------------------------------

const CANDID_REPORT: &str = "AI/candid_review_report.md";
const PLAN_REPORT: &str = "AI/plan_evaluator_report.md";

/// A scratch git repository with the real `scripts/candid_subagent.sh`.
struct ScratchRepo {
    dir: PathBuf,
}

impl ScratchRepo {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = workspace_root()
            .join("target")
            .join("artifact_freshness_contract")
            .join(format!(
                "{tag}_{}_{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scripts")).expect("create scratch repo");
        fs::copy(
            workspace_root().join("scripts").join("candid_subagent.sh"),
            dir.join("scripts").join("candid_subagent.sh"),
        )
        .expect("copy candid_subagent.sh");
        let repo = Self { dir };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.write("src/lib.rs", "pub fn f() -> i32 {\n    0\n}\n");
        repo.write(CANDID_REPORT, "# Candid Review Report\n\nbase\n");
        repo.write(PLAN_REPORT, "# Plan Evaluation Report\n\nbase\n");
        repo.commit("chore: base");
        repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo.git(&["switch", "-q", "-c", "topic"]);
        repo
    }

    /// A command isolated from the caller's repository and configuration (the test may run
    /// inside a git hook, where `GIT_DIR` and `GIT_INDEX_FILE` are set).
    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.current_dir(&self.dir);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") || key == "CANDID_BASE_REF" {
                cmd.env_remove(&key);
            }
        }
        cmd.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("HOME", &self.dir)
            .env("GIT_AUTHOR_NAME", "soos test")
            .env("GIT_AUTHOR_EMAIL", "test@invalid")
            .env("GIT_COMMITTER_NAME", "soos test")
            .env("GIT_COMMITTER_EMAIL", "test@invalid");
        cmd
    }

    fn git(&self, args: &[&str]) -> Output {
        let out = self.command("git").args(args).output().expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    fn write(&self, path: &str, content: &str) {
        let full = self.dir.join(path);
        fs::create_dir_all(full.parent().expect("parent")).expect("create parent");
        fs::write(full, content).expect("write file");
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--no-verify",
            "--allow-empty",
            "-m",
            message,
        ]);
    }

    fn candid(&self, args: &[&str]) -> (bool, String) {
        let out = self
            .command("bash")
            .arg("scripts/candid_subagent.sh")
            .args(args)
            .output()
            .expect("run candid_subagent.sh");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    }

    fn fingerprint(&self) -> String {
        self.fingerprint_with_compression(None)
    }

    /// The fingerprint with `core.compression` forced through `GIT_CONFIG_*` (a stand-in for
    /// a different zlib build: zlib-ng on Arch, zlib on the Ubuntu CI runners).
    fn fingerprint_with_compression(&self, level: Option<&str>) -> String {
        let mut cmd = self.command("bash");
        cmd.arg("scripts/candid_subagent.sh").arg("--fingerprint");
        if let Some(level) = level {
            cmd.env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "core.compression")
                .env("GIT_CONFIG_VALUE_0", level);
        }
        let out = cmd.output().expect("run candid_subagent.sh");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let ok = out.status.success();
        assert!(ok, "--fingerprint failed:\n{text}");
        text.lines()
            .rev()
            .find(|l| l.len() == 64 && l.bytes().all(|b| b.is_ascii_hexdigit()))
            .unwrap_or_else(|| panic!("no fingerprint in output:\n{text}"))
            .to_string()
    }
}

impl Drop for ScratchRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn test_candid_fingerprint_ignores_both_review_report_singletons() {
    let repo = ScratchRepo::new("fingerprint");
    repo.write("src/lib.rs", "pub fn f() -> i32 {\n    1\n}\n");
    let code_only = repo.fingerprint();

    repo.write(
        PLAN_REPORT,
        "# Plan Evaluation Report\n\nrewritten for this issue\n",
    );
    assert_eq!(
        repo.fingerprint(),
        code_only,
        "rewriting {PLAN_REPORT} must not change the reviewed-diff fingerprint (GitHub #267)"
    );

    repo.write(CANDID_REPORT, "# Candid Review Report\n\nrewritten\n");
    assert_eq!(
        repo.fingerprint(),
        code_only,
        "rewriting {CANDID_REPORT} must not change the reviewed-diff fingerprint"
    );

    repo.write("src/lib.rs", "pub fn f() -> i32 {\n    2\n}\n");
    assert_ne!(
        repo.fingerprint(),
        code_only,
        "any code change must still change the fingerprint"
    );
}

#[test]
fn test_candid_prepare_lists_neither_review_report() {
    let repo = ScratchRepo::new("prepare");
    repo.write("src/lib.rs", "pub fn f() -> i32 {\n    1\n}\n");
    repo.write(PLAN_REPORT, "# Plan Evaluation Report\n\nrewritten\n");
    let (ok, text) = repo.candid(&["--prepare"]);
    assert!(ok, "--prepare failed:\n{text}");
    assert!(
        text.contains("src/lib.rs"),
        "--prepare must list the changed code:\n{text}"
    );
    assert!(
        !text.contains(PLAN_REPORT),
        "--prepare must not list {PLAN_REPORT} as reviewed code:\n{text}"
    );
    let patch = fs::read_to_string(repo.dir.join("target").join("candid_diff.patch"))
        .expect("read candid_diff.patch");
    assert!(
        !patch.contains(PLAN_REPORT),
        "the review patch must not contain {PLAN_REPORT}"
    );
}

#[test]
fn test_candid_gate_treats_a_report_only_change_as_zero_changes() {
    let repo = ScratchRepo::new("report_only");
    repo.write(PLAN_REPORT, "# Plan Evaluation Report\n\nrewritten\n");
    repo.commit("docs(ai): rewrite plan report");
    let (ok, text) = repo.candid(&["--rev", "HEAD", "--skip-layer1"]);
    assert!(
        ok && text.contains("Zero changes"),
        "a branch that only rewrites {PLAN_REPORT} has no reviewable code change:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// QFX5 — documentation
// ---------------------------------------------------------------------------

#[test]
fn test_candid_report_exclusions_are_documented() {
    for rel in [
        "Docs/CI_CD_AND_SECURITY.md",
        ".agents/skills/candid-reviewer/SKILL.md",
        "scripts/candid_subagent.sh",
    ] {
        assert!(
            read(rel).contains(PLAN_REPORT),
            "{rel} must state that {PLAN_REPORT} is excluded from the fingerprint"
        );
    }
}

/// GitHub #339: a binary file in the diff must give the same fingerprint whatever the zlib
/// build or compression level (`git diff --binary` deflates the blob, so zlib-ng on the
/// owner's Arch host and zlib on the CI runner produced different fingerprints for the same
/// commit), and the fingerprint must still change when the binary content changes.
#[test]
fn test_candid_fingerprint_is_portable_for_binary_files() {
    let repo = ScratchRepo::new("binary");
    let icon = repo.dir.join("assets").join("icon.png");
    fs::create_dir_all(icon.parent().expect("parent")).expect("create assets");
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend((0u16..2048).map(|i| (i.wrapping_mul(31) % 251) as u8));
    fs::write(&icon, &bytes).expect("write binary");

    let default = repo.fingerprint();
    for level in ["0", "1", "9"] {
        assert_eq!(
            repo.fingerprint_with_compression(Some(level)),
            default,
            "core.compression={level} changed the fingerprint of a binary diff"
        );
    }

    bytes[100] ^= 0xff;
    fs::write(&icon, &bytes).expect("rewrite binary");
    assert_ne!(
        repo.fingerprint(),
        default,
        "changing the binary content must change the fingerprint"
    );
}
