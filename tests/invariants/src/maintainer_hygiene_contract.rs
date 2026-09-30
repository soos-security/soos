//! Repository hygiene and documentation-drift contract (GitHub #238 TCI-07, #239 TCI-08,
//! #240 TCI-09).
//!
//! - #238: no one-off maintainer scripts at the repository root, root-level scratch files are
//!   ignored so `git add .` in `save.sh` cannot commit them, and no tooling file hard-codes a
//!   personal `/home/<user>/` path.
//! - #239: every branch registered in `scripts/sync_issue.py` uses an approved prefix
//!   (`feat/`, `fix/`, `test/`, `chore/`) unless it is one of the frozen, merged legacy
//!   branches documented in `AGENTS.md`; both workspace maps list every Cargo member and the
//!   real `tests/` directories; `AI/ARCHITECTURE.md` carries no stale status claims.
//! - #240: the Docker PAM harness is `pam_test_runner` (not `pamtester`), the PAM matrix range
//!   quoted by the tooling matches `tests/docker/test_suite.sh`, `run_tests.sh` declares no
//!   unused constants, `Docs/README.md` indexes every page, and the mock/memory documents
//!   describe the current fixtures and 512D embeddings.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

// ---------------------------------------------------------------------------
// #238 — one-off maintainer scripts and personal paths
// ---------------------------------------------------------------------------

#[test]
fn test_no_one_off_maintainer_scripts_at_repository_root() {
    for name in [
        "create_issues.py",
        "fix_main.py",
        "patch_sync.py",
        "temp_body.md",
    ] {
        assert!(
            !root().join(name).exists(),
            "one-off maintainer file '{name}' must not live at the repository root (GitHub #238)"
        );
    }
}

#[test]
fn test_root_level_scratch_files_are_gitignored() {
    let gitignore = read(".gitignore");
    let lines: Vec<&str> = gitignore.lines().map(str::trim).collect();
    for pattern in ["/*.py", "/temp_*.md"] {
        assert!(
            lines.contains(&pattern),
            ".gitignore must ignore root-level scratch files with '{pattern}' so that \
             `git add .` in save.sh cannot stage them (GitHub #238)"
        );
    }
}

/// Collects every regular file below `dir` (recursively).
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// Returns the first `/home/<name>` occurrence (a literal user home directory), if any.
fn personal_home_path(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let needle = b"/home/";
    let mut i = 0;
    while i + needle.len() < bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let next = bytes[i + needle.len()];
            if next.is_ascii_alphanumeric() || next == b'_' {
                let end = text[i..]
                    .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                    .map_or(text.len(), |n| i + n);
                return Some(text[i..end].to_string());
            }
        }
        i += 1;
    }
    None
}

#[test]
fn test_personal_home_path_detector_matches_literal_home_dirs() {
    assert_eq!(
        personal_home_path("gh = \"/home/alice/.local/bin/gh\"").as_deref(),
        Some("/home/alice/.local/bin/gh")
    );
    assert_eq!(personal_home_path("cd \"$HOME\" && ls /home/"), None);
    assert_eq!(personal_home_path("ProtectHome=yes"), None);
}

#[test]
fn test_tooling_has_no_personal_home_paths() {
    let root = root();
    let mut files = Vec::new();
    for dir in [
        "scripts",
        ".githooks",
        ".github",
        ".agents",
        "packaging",
        "tests/docker",
        "tests/distro",
        "tests/physical",
    ] {
        collect_files(&root.join(dir), &mut files);
    }
    for entry in fs::read_dir(&root).expect("read root").flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_file()
            && (name.ends_with(".sh") || name.ends_with(".py") || name == "Dockerfile")
        {
            files.push(path);
        }
    }
    let mut offenders = Vec::new();
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        if let Some(hit) = personal_home_path(&text) {
            offenders.push(format!(
                "{}: {hit}",
                file.strip_prefix(&root).unwrap_or(&file).display()
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "tooling must use repository-relative paths or PATH lookups, never a personal \
         /home/<user>/ path (GitHub #238):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn test_sync_issue_resolves_gh_through_path_only() {
    let script = read("scripts/sync_issue.py");
    assert!(
        script.contains("shutil.which(\"gh\")"),
        "scripts/sync_issue.py must resolve gh through PATH (shutil.which)"
    );
    assert!(
        !script.contains(".local/bin/gh"),
        "scripts/sync_issue.py must not fall back to a hard-coded per-user gh binary"
    );
}

// ---------------------------------------------------------------------------
// #239 — branch prefixes, workspace maps, ARCHITECTURE.md status claims
// ---------------------------------------------------------------------------

const APPROVED_BRANCH_PREFIXES: [&str; 4] = ["feat/", "fix/", "test/", "chore/"];

/// Branch names registered in `BRANCH_TO_ISSUE` of `scripts/sync_issue.py`.
fn registered_branches() -> Vec<String> {
    let script = read("scripts/sync_issue.py");
    let start = script
        .find("BRANCH_TO_ISSUE = {")
        .expect("BRANCH_TO_ISSUE table");
    let body = &script[start..];
    let end = body.find("\n}").expect("end of BRANCH_TO_ISSUE table");
    body[..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix('"')?;
            let close = rest.find('"')?;
            rest[close + 1..]
                .trim_start()
                .starts_with(':')
                .then(|| rest[..close].to_string())
        })
        .collect()
}

#[test]
fn test_registered_branches_use_approved_prefixes_or_documented_legacy_names() {
    let branches = registered_branches();
    assert!(branches.len() > 10, "BRANCH_TO_ISSUE must be parsed");
    let agents = read("AGENTS.md");
    let facts = read(".agents/skills/dev-workflow/references/project-facts.md");
    let mut offenders = Vec::new();
    for branch in &branches {
        if APPROVED_BRANCH_PREFIXES
            .iter()
            .any(|p| branch.starts_with(p))
        {
            continue;
        }
        let quoted = format!("`{branch}`");
        if !agents.contains(&quoted) || !facts.contains(&quoted) {
            offenders.push(branch.clone());
        }
    }
    assert!(
        offenders.is_empty(),
        "branches registered with a non-approved prefix must be frozen legacy branches named \
         in AGENTS.md and project-facts.md (GitHub #239): {offenders:?}"
    );
}

fn tampered_sync_issue(tag: &str, injection: &str) -> PathBuf {
    let source = read("scripts/sync_issue.py");
    let at = source
        .find("def run_gh_cmd")
        .expect("anchor def run_gh_cmd");
    let dir = root()
        .join("target")
        .join("maintainer_hygiene_contract")
        .join(format!("{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    let copy = dir.join("sync_issue.py");
    fs::write(
        &copy,
        format!("{}{injection}\n{}", &source[..at], &source[at..]),
    )
    .expect("write tampered copy");
    copy
}

fn run_check(script: &Path) -> Output {
    let backlog = root().join("AI").join("BACKLOG.md");
    Command::new("python3")
        .arg(script)
        .args(["--check", "--backlog"])
        .arg(backlog)
        .current_dir(root())
        .output()
        .expect("python3 must be available")
}

#[test]
fn test_sync_issue_check_rejects_unapproved_branch_prefix() {
    for (tag, branch) in [
        ("refactor_prefix", "refactor/new-cleanup"),
        ("docs_prefix", "docs/new-guide"),
    ] {
        let copy = tampered_sync_issue(tag, &format!("BRANCH_TO_ISSUE[\"{branch}\"] = 1\n"));
        let output = run_check(&copy);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success(),
            "--check must reject the new branch '{branch}' with a non-approved prefix:\n{text}"
        );
        assert!(
            text.contains(&format!("branch '{branch}' uses a non-approved prefix")),
            "the failure must name the offending branch '{branch}':\n{text}"
        );
    }
}

#[test]
fn test_sync_issue_check_accepts_frozen_legacy_branch() {
    let copy = tampered_sync_issue("legacy_ok", "");
    let output = run_check(&copy);
    assert!(
        output.status.success(),
        "an untampered copy (with the frozen legacy refactor/ and docs/ branches) must pass:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Workspace members declared in the root `Cargo.toml`.
fn workspace_members() -> Vec<String> {
    let cargo = read("Cargo.toml");
    let start = cargo.find("members = [").expect("members");
    let body = &cargo[start..];
    let end = body.find(']').expect("end of members");
    body[..end]
        .lines()
        .filter_map(|l| {
            let l = l.trim().trim_end_matches(',');
            l.strip_prefix('"')?.strip_suffix('"').map(str::to_string)
        })
        .collect()
}

/// The first fenced code block of `doc` that contains `marker`.
fn tree_block(doc: &str, marker: &str) -> String {
    let mut in_block = false;
    let mut block = String::new();
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            if in_block {
                if block.contains(marker) {
                    return block;
                }
                block.clear();
            }
            in_block = !in_block;
            continue;
        }
        if in_block {
            block.push_str(line);
            block.push('\n');
        }
    }
    panic!("no code block containing {marker:?}");
}

fn tests_subdirs() -> Vec<String> {
    let mut dirs: Vec<String> = fs::read_dir(root().join("tests"))
        .expect("read tests/")
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn test_workspace_maps_list_every_member_and_real_tests_dirs() {
    let members = workspace_members();
    assert!(members.contains(&"crates/gui".to_string()));
    for doc_path in ["AGENTS.md", "AI/ARCHITECTURE.md"] {
        let tree = tree_block(&read(doc_path), "crates/");
        for member in &members {
            let leaf = member.rsplit('/').next().expect("leaf");
            assert!(
                tree.contains(&format!("── {leaf}/")),
                "{doc_path} workspace tree must list Cargo member '{member}' (GitHub #239)"
            );
        }
        for dir in tests_subdirs() {
            assert!(
                tree.contains(&format!("── {dir}/")),
                "{doc_path} workspace tree must list tests/{dir}/ (GitHub #239)"
            );
        }
        for stale in ["integration-pam", "integration/"] {
            assert!(
                !tree.contains(stale),
                "{doc_path} workspace tree lists non-existent tests/{stale} (GitHub #239)"
            );
        }
    }
}

#[test]
fn test_architecture_has_no_stale_status_claims() {
    let arch = read("AI/ARCHITECTURE.md");
    for stale in [
        "Current Skeleton Phase",
        "Planned Target",
        "`nokhwa` 0.10.11",
        "(`protocol`, `policy`, `vision`).",
        "Dockerized `pamtester`",
    ] {
        assert!(
            !arch.contains(stale),
            "AI/ARCHITECTURE.md still contains the stale claim {stale:?} (GitHub #239/#240)"
        );
    }
    assert!(
        arch.contains("test_business_crates_forbid_unsafe_code"),
        "AI/ARCHITECTURE.md must point to the invariant test that owns the forbid(unsafe_code) list"
    );
}

// ---------------------------------------------------------------------------
// #240 — Docker harness, PAM matrix range, run_tests.sh, docs index, mock docs
// ---------------------------------------------------------------------------

#[test]
fn test_docker_pam_harness_does_not_claim_pamtester() {
    for path in [
        "Dockerfile",
        "tests/docker/Dockerfile.ubuntu",
        "tests/docker/test_suite.sh",
        "run_tests.sh",
        "AI/ROLES_AND_WORKFLOW.md",
    ] {
        assert!(
            !read(path).contains("pamtester"),
            "{path} must not install or claim pamtester: the Docker matrix runs \
             tests/docker/pam_test_runner.c (GitHub #240)"
        );
    }
    assert!(
        read("tests/docker/test_suite.sh").contains("pam_test_runner"),
        "the Docker suite must use pam_test_runner"
    );
    assert!(
        read("tests/physical/pam_integration_test.sh").contains("pamtester"),
        "the physical suite keeps pamtester as its documented fallback runner"
    );
}

/// Highest `T<n>` case executed by `tests/docker/test_suite.sh`.
fn highest_pam_case() -> u32 {
    read("tests/docker/test_suite.sh")
        .lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("info \"T")?;
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .max()
        .expect("test_suite.sh runs T cases")
}

#[test]
fn test_pam_matrix_range_claims_match_the_suite() {
    let max = highest_pam_case();
    assert!(max >= 15, "test_suite.sh runs T1..T{max}");
    let full = format!("T1–T{max}");
    for path in [
        "run_tests.sh",
        ".agents/skills/dev-workflow/references/project-facts.md",
        ".github/workflows/ci.yml",
        "Docs/CI_CD_AND_SECURITY.md",
    ] {
        let text = read(path);
        assert!(
            text.contains(&full),
            "{path} must quote the full PAM matrix range {full} (GitHub #240)"
        );
        for n in 1..max {
            let stale = format!("T1–T{n})");
            let stale_space = format!("T1–T{n} ");
            assert!(
                !text.contains(&stale) && !text.contains(&stale_space),
                "{path} quotes the stale PAM matrix range T1–T{n}; the suite runs {full}"
            );
        }
    }
    assert!(
        !read("Docs/PAM_DOCKER_TEST_MATRIX.md").contains("artifact used by T1–T9"),
        "Docs/PAM_DOCKER_TEST_MATRIX.md must not imply that only T1–T9 use the production artifact"
    );
}

#[test]
fn test_run_tests_declares_no_unused_readonly_constants() {
    let script = read("run_tests.sh");
    for line in script.lines() {
        let Some(rest) = line.trim().strip_prefix("readonly ") else {
            continue;
        };
        let Some((name, _)) = rest.split_once('=') else {
            continue;
        };
        let uses = script.matches(&format!("${{{name}}}")).count()
            + script.matches(&format!("${name}")).count();
        assert!(
            uses > 0,
            "run_tests.sh declares readonly {name} but never uses it (GitHub #240)"
        );
    }
}

#[test]
fn test_docs_index_lists_every_docs_page() {
    let index = read("Docs/README.md");
    let mut missing = Vec::new();
    for entry in fs::read_dir(root().join("Docs")).expect("Docs/").flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".md") && name != "README.md" && !index.contains(&format!("]({name})")) {
            missing.push(name);
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "Docs/README.md must index every page under Docs/ (GitHub #240); missing: {missing:?}"
    );
}

#[test]
fn test_mock_and_memory_docs_describe_current_fixtures_and_512d() {
    let mock = read("AI/MOCK_STRATEGY.md");
    assert!(
        !mock.contains("JPEG") && !mock.contains("serialized tensor arrays"),
        "AI/MOCK_STRATEGY.md must not describe image/tensor fixture files that do not exist"
    );
    assert!(
        mock.contains("tests/fixtures/mod.rs"),
        "AI/MOCK_STRATEGY.md must point to the synthetic fixture module tests/fixtures/mod.rs"
    );
    assert!(
        !read("Docs/MEMORY_PROTECTION_AND_SWAP.md").contains("128D/512D"),
        "Docs/MEMORY_PROTECTION_AND_SWAP.md must state the 512D embedding only"
    );
    let decisions = read("AI/DECISIONS.md");
    let class_zero = decisions
        .lines()
        .find(|l| l.contains("MiniFASNetV2 Class Ordering"))
        .expect("2026-09-20 class-ordering ADR");
    assert!(
        class_zero.contains("Superseded"),
        "the class-0 ADR must be marked superseded by the live class index 1 decision"
    );
}
