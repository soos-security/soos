//! Traceability tooling contract tests for `scripts/sync_issue.py` (GitHub #188, TCI-05).
//!
//! - `--check` is an offline self-check: it passes on the repository and rejects duplicate
//!   GitHub targets, unknown backlog ids and duplicate backlog headings.
//! - `--auto` never rewrites `AI/BACKLOG.md` (it only mirrors already-committed checkboxes to
//!   GitHub), so nothing is left dirty after a push.
//! - A failing or unreachable `gh` is visible (non-zero exit, explicit message) and
//!   non-destructive (the local backlog is untouched).
//! - `save.sh` runs the self-check before staging, and neither `save.sh` nor
//!   `scripts/pr_loop.sh` silences a sync failure with `|| true`.

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

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Fresh scratch directory under the workspace `target/` directory (never a repository clone).
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = workspace_root()
        .join("target")
        .join("sync_issue_contract")
        .join(format!("{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn script_path() -> PathBuf {
    workspace_root().join("scripts").join("sync_issue.py")
}

fn backlog_path() -> PathBuf {
    workspace_root().join("AI").join("BACKLOG.md")
}

fn run_python(script: &Path, args: &[&str], path_prefix: Option<&Path>) -> Output {
    let mut cmd = Command::new("python3");
    cmd.arg(script).args(args).current_dir(workspace_root());
    if let Some(prefix) = path_prefix {
        let path = std::env::var("PATH").unwrap_or_default();
        cmd.env("PATH", format!("{}:{path}", prefix.display()));
    }
    cmd.output()
        .expect("python3 must be available to run scripts/sync_issue.py")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Copies `scripts/sync_issue.py` into a scratch directory, inserting `injection` right before
/// the line that starts with `anchor`.
fn tampered_script(tag: &str, anchor: &str, injection: &str) -> PathBuf {
    let source = fs::read_to_string(script_path()).expect("read scripts/sync_issue.py");
    let at = source
        .find(anchor)
        .unwrap_or_else(|| panic!("anchor {anchor:?} not found in scripts/sync_issue.py"));
    let tampered = format!("{}{injection}\n{}", &source[..at], &source[at..]);
    let dir = scratch_dir(tag);
    let copy = dir.join("sync_issue.py");
    fs::write(&copy, tampered).expect("write tampered copy");
    copy
}

#[test]
fn test_sync_issue_check_passes_on_repository() {
    let output = run_python(&script_path(), &["--check"], None);
    assert!(
        output.status.success(),
        "`python3 scripts/sync_issue.py --check` must pass on the repository:\n{}",
        combined(&output)
    );
}

#[test]
fn test_sync_issue_check_rejects_duplicate_github_target() {
    let copy = tampered_script(
        "dup_target",
        "BRANCH_TO_ISSUE = {",
        "BACKLOG_TO_GITHUB[23] = BACKLOG_TO_GITHUB[22]\n",
    );
    let backlog = backlog_path();
    let output = run_python(
        &copy,
        &["--check", "--backlog", backlog.to_str().unwrap()],
        None,
    );
    let text = combined(&output);
    assert!(
        !output.status.success(),
        "a duplicate GitHub target must fail --check:\n{text}"
    );
    assert!(
        text.contains("duplicate GitHub target"),
        "the failure must name the duplicate GitHub target:\n{text}"
    );
}

#[test]
fn test_sync_issue_check_rejects_unknown_backlog_id() {
    let copy = tampered_script(
        "unknown_id",
        "def run_gh_cmd",
        "BRANCH_TO_ISSUE[\"feat/does-not-exist\"] = 999\n",
    );
    let backlog = backlog_path();
    let output = run_python(
        &copy,
        &["--check", "--backlog", backlog.to_str().unwrap()],
        None,
    );
    let text = combined(&output);
    assert!(
        !output.status.success(),
        "a branch mapped to an unknown backlog id must fail --check:\n{text}"
    );
    assert!(
        text.contains("unknown backlog issue #999"),
        "the failure must name the unknown backlog id:\n{text}"
    );
}

#[test]
fn test_sync_issue_check_rejects_duplicate_backlog_heading() {
    let dir = scratch_dir("dup_heading");
    let original = fs::read_to_string(backlog_path()).expect("read AI/BACKLOG.md");
    let copy = dir.join("BACKLOG.md");
    fs::write(
        &copy,
        format!("{original}\n### Issue #1: duplicated heading\n\n- [ ] **#1.99** — dup\n"),
    )
    .expect("write backlog copy");
    let output = run_python(
        &script_path(),
        &["--check", "--backlog", copy.to_str().unwrap()],
        None,
    );
    let text = combined(&output);
    assert!(
        !output.status.success(),
        "a duplicated backlog heading must fail --check:\n{text}"
    );
    assert!(
        text.contains("duplicate backlog heading #1"),
        "the failure must name the duplicated heading:\n{text}"
    );
}

/// Backlog copy where sub-issue #22.1 is still open, so `--auto` has something it could
/// (wrongly) tick locally.
fn backlog_copy_with_open_subissue(tag: &str) -> (PathBuf, String) {
    let dir = scratch_dir(tag);
    let original = fs::read_to_string(backlog_path()).expect("read AI/BACKLOG.md");
    let reopened = original.replacen("- [x] **#22.1**", "- [ ] **#22.1**", 1);
    assert_ne!(
        original, reopened,
        "fixture: #22.1 must exist and be checked"
    );
    let copy = dir.join("BACKLOG.md");
    fs::write(&copy, &reopened).expect("write backlog copy");
    (copy, reopened)
}

#[test]
fn test_sync_issue_auto_local_only_never_rewrites_backlog() {
    let (copy, before) = backlog_copy_with_open_subissue("auto_local");
    let output = run_python(
        &script_path(),
        &[
            "--auto",
            "--local-only",
            "--branch",
            "feat/camera-format-negotiation",
            "--backlog",
            copy.to_str().unwrap(),
        ],
        None,
    );
    let text = combined(&output);
    assert!(
        output.status.success(),
        "--auto --local-only must succeed:\n{text}"
    );
    let after = fs::read_to_string(&copy).expect("read backlog copy");
    assert_eq!(before, after, "--auto must never rewrite AI/BACKLOG.md");
    assert!(
        text.contains("#22.1"),
        "--auto must report the still-open sub-issue #22.1:\n{text}"
    );
}

#[test]
fn test_sync_issue_auto_unregistered_branch_is_a_visible_noop() {
    let (copy, before) = backlog_copy_with_open_subissue("auto_unregistered");
    let output = run_python(
        &script_path(),
        &[
            "--auto",
            "--local-only",
            "--branch",
            "fix/some-review-finding",
            "--backlog",
            copy.to_str().unwrap(),
        ],
        None,
    );
    let text = combined(&output);
    assert!(
        output.status.success(),
        "an unregistered (GitHub-only) branch is not an error:\n{text}"
    );
    assert!(
        text.contains("not registered"),
        "the no-op must be announced:\n{text}"
    );
    assert_eq!(
        before,
        fs::read_to_string(&copy).expect("read backlog copy")
    );
}

#[test]
fn test_sync_issue_gh_failure_is_visible_and_non_destructive() {
    let (copy, before) = backlog_copy_with_open_subissue("gh_failure");
    // Fake `gh` that fails every call, as an offline or unauthenticated CLI would.
    let bin = scratch_dir("gh_failure_bin");
    let fake_gh = bin.join("gh");
    fs::write(
        &fake_gh,
        "#!/bin/sh\necho 'error connecting to api.github.com' >&2\nexit 1\n",
    )
    .expect("write fake gh");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755)).expect("chmod fake gh");
    }
    let output = run_python(
        &script_path(),
        &[
            "--auto",
            "--branch",
            "feat/camera-format-negotiation",
            "--backlog",
            copy.to_str().unwrap(),
        ],
        Some(&bin),
    );
    let text = combined(&output);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a failed GitHub sync must exit with status 2:\n{text}"
    );
    assert!(
        text.contains("GitHub sync failed"),
        "a failed GitHub sync must be reported explicitly:\n{text}"
    );
    assert_eq!(
        before,
        fs::read_to_string(&copy).expect("read backlog copy"),
        "a failed GitHub sync must leave AI/BACKLOG.md untouched"
    );
}

#[test]
fn test_save_sh_runs_sync_check_before_staging() {
    let save = fs::read_to_string(workspace_root().join("save.sh")).expect("read save.sh");
    let check = save
        .find("sync_issue.py --check")
        .expect("save.sh must run `sync_issue.py --check`");
    let stage = save
        .find("\ngit add .")
        .expect("save.sh stages with `git add .`");
    assert!(
        check < stage,
        "save.sh must run the sync self-check before staging the commit"
    );
}

#[test]
fn test_release_scripts_never_silence_sync_failures() {
    for rel in ["save.sh", "scripts/pr_loop.sh"] {
        let text = fs::read_to_string(workspace_root().join(rel)).expect("read release script");
        for (n, line) in text.lines().enumerate() {
            if line.contains("sync_issue.py") {
                assert!(
                    !line.contains("|| true"),
                    "{rel}:{}: a sync_issue.py failure must be visible, not `|| true`",
                    n + 1
                );
            }
        }
    }
}
