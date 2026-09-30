//! End-to-end contract for the deterministic gate `scripts/candid_review.sh`
//! (GitHub #242, review finding TCI-11).
//!
//! Every test builds a throw-away git repository under the workspace `target/` directory,
//! copies the real script into it, fakes `origin/main` with `git update-ref`, and runs the
//! script exactly as the pre-commit hook does. The contract:
//!
//! - an added PAM production line is audited even when it contains the substring `tests`
//!   (no `grep -v tests`), while code added inside a `#[cfg(test)]` module is not;
//! - `#[cfg(test)] mod tests;` never hides the production code that follows it;
//! - the English words `pour` and `attention` are not French markers, real French still is;
//! - the audited diff starts at `git merge-base origin/main HEAD`, so commits that land on
//!   `main` after the branch point never show up as (reversed) branch changes.

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

/// Minimal PAM crate source: production code plus an inline test module.
const PAM_LIB: &str = "\
pub fn authenticate() -> i32 {
    0
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_ok() {
        assert_eq!(super::authenticate(), 0);
    }
}
";

/// A scratch git repository with the real `scripts/candid_review.sh`.
struct ScratchRepo {
    dir: PathBuf,
}

impl ScratchRepo {
    /// Creates the repository, commits `files` on `main` and points `origin/main` at it.
    fn new(tag: &str, files: &[(&str, &str)]) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = workspace_root()
            .join("target")
            .join("candid_review_contract")
            .join(format!(
                "{tag}_{}_{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scripts")).expect("create scratch repo");
        fs::copy(
            workspace_root().join("scripts").join("candid_review.sh"),
            dir.join("scripts").join("candid_review.sh"),
        )
        .expect("copy candid_review.sh");
        let repo = Self { dir };
        repo.git(&["init", "-q", "-b", "main"]);
        for (path, content) in files {
            repo.write(path, content);
        }
        repo.commit("chore: base");
        repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo
    }

    /// A `git` command isolated from the caller's repository and configuration (the test may
    /// run inside a pre-commit hook, where `GIT_DIR` and `GIT_INDEX_FILE` are set).
    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.current_dir(&self.dir);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
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
            "-m",
            message,
        ]);
    }

    /// Runs the script from the repository root, as `.githooks/pre-commit` does.
    fn candid_review(&self) -> (bool, String) {
        let out = self
            .command("bash")
            .arg("scripts/candid_review.sh")
            .output()
            .expect("run candid_review.sh");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    }
}

impl Drop for ScratchRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn test_fil_candid_flags_pam_unwrap_on_line_containing_tests_substring() {
    let repo = ScratchRepo::new("tests_substring", &[("crates/pam/src/lib.rs", PAM_LIB)]);
    repo.git(&["switch", "-q", "-c", "topic"]);
    let changed = PAM_LIB.replace(
        "pub fn authenticate() -> i32 {\n    0\n}",
        "pub fn authenticate() -> i32 {\n    let attests = lookup().unwrap();\n    attests\n}",
    );
    repo.write("crates/pam/src/lib.rs", &changed);
    repo.commit("feat(pam): add lookup");

    let (ok, output) = repo.candid_review();
    assert!(
        !ok,
        "a production unwrap on a line containing 'tests' must fail the gate:\n{output}"
    );
    assert!(
        output.contains("attests"),
        "the offending line must be reported:\n{output}"
    );
}

#[test]
fn test_fil_candid_allows_unwrap_inside_cfg_test_module() {
    let repo = ScratchRepo::new("cfg_test_module", &[("crates/pam/src/lib.rs", PAM_LIB)]);
    repo.git(&["switch", "-q", "-c", "topic"]);
    let changed = PAM_LIB.replace(
        "        assert_eq!(super::authenticate(), 0);",
        "        let value = Some(super::authenticate()).unwrap();\n        println!(\"{value}\");",
    );
    repo.write("crates/pam/src/lib.rs", &changed);
    repo.commit("test(pam): extend unit test");

    let (ok, output) = repo.candid_review();
    assert!(
        ok,
        "unwrap/println inside a #[cfg(test)] module is test code and must pass:\n{output}"
    );
}

#[test]
fn test_fil_candid_cfg_test_mod_declaration_does_not_hide_production() {
    let base = "#[cfg(test)]\nmod tests;\n\npub fn authenticate() -> i32 {\n    0\n}\n";
    let repo = ScratchRepo::new("cfg_test_decl", &[("crates/pam/src/lib.rs", base)]);
    repo.git(&["switch", "-q", "-c", "topic"]);
    repo.write(
        "crates/pam/src/lib.rs",
        &base.replace("    0\n", "    lookup().expect(\"present\")\n"),
    );
    repo.commit("feat(pam): add lookup");

    let (ok, output) = repo.candid_review();
    assert!(
        !ok,
        "production code after `#[cfg(test)] mod tests;` must still be audited:\n{output}"
    );
    assert!(output.contains("expect("), "{output}");
}

#[test]
fn test_fil_candid_accepts_english_attention_and_pour() {
    let repo = ScratchRepo::new("english_words", &[("crates/protocol/src/lib.rs", "")]);
    repo.git(&["switch", "-q", "-c", "topic"]);
    repo.write(
        "crates/protocol/src/lib.rs",
        "// Attention: pour the decoded bytes into the bounded buffer.\npub fn noop() {}\n",
    );
    repo.commit("docs(protocol): comment");

    let (ok, output) = repo.candid_review();
    assert!(ok, "'attention' and 'pour' are English words:\n{output}");
}

#[test]
fn test_fil_candid_still_flags_french_comments() {
    let repo = ScratchRepo::new("french_words", &[("crates/protocol/src/lib.rs", "")]);
    repo.git(&["switch", "-q", "-c", "topic"]);
    // Assembled at run time so this source file itself carries no French marker.
    let comment = format!("// {} le {}\n", concat!("da", "ns"), concat!("fich", "ier"));
    repo.write("crates/protocol/src/lib.rs", &comment);
    repo.commit("docs(protocol): comment");

    let (ok, output) = repo.candid_review();
    assert!(
        !ok,
        "a French comment must still fail the language audit:\n{output}"
    );
}

#[test]
fn test_fil_candid_ignores_upstream_commits_after_branch_point() {
    let base = "pub fn authenticate() -> i32 {\n    lookup().unwrap()\n}\n";
    let repo = ScratchRepo::new(
        "moving_main",
        &[("crates/pam/src/lib.rs", base), ("README.md", "readme\n")],
    );
    repo.git(&["switch", "-q", "-c", "topic"]);
    repo.write("README.md", "readme, updated\n");
    repo.commit("docs: update readme");

    // `main` moves on after the branch point and removes the legacy unwrap.
    repo.git(&["switch", "-q", "main"]);
    repo.write(
        "crates/pam/src/lib.rs",
        "pub fn authenticate() -> i32 {\n    lookup().unwrap_or(0)\n}\n",
    );
    repo.commit("fix(pam): drop unwrap");
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    repo.git(&["switch", "-q", "topic"]);

    let (ok, output) = repo.candid_review();
    assert!(
        ok,
        "upstream changes after the branch point must not be audited as branch changes:\n{output}"
    );
}
