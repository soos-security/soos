//! Contracts of the release dry run (GitHub #444, part of #361; ADR "[2026-10-18] Release Dry
//! Run Publishes Nothing"; spec `AI/architect_spec_release_dry_run.md`; matrix RDR1 to RDR6 and
//! RDR-S1 to RDR-S8):
//!
//! - `scripts/release_dry_run.sh` and `tests/docker/release_dry_run_test.sh` never tag, push,
//!   sign, upload or publish, and the CI steps that run them upload nothing;
//! - the vendored crate set equals `Cargo.lock`, compared by the `.cargo-checksum.json` package
//!   checksum of every vendored crate (`--check-vendor`, run after every vendoring);
//! - the version comes from `[workspace.package]` of the committed `Cargo.toml` (HEAD), the
//!   timestamps from the commit time;
//! - two runs on the same commit give byte-identical tarballs and `SHA256SUMS` (end to end on a
//!   dependency-free temporary repository; the full repository is proven by the Docker harness);
//! - the Docker harness builds the release workspace offline from the two tarballs only;
//! - ADR and documentation, with the owner-only steps kept out of every script.

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
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const SCRIPT: &str = "scripts/release_dry_run.sh";
const HARNESS: &str = "tests/docker/release_dry_run_test.sh";
const CI: &str = ".github/workflows/ci.yml";
const DECISIONS: &str = "AI/DECISIONS.md";
const RELEASING_DOC: &str = "Docs/RELEASING.md";
const PACKAGING_DOC: &str = "Docs/PACKAGING_AND_PROVISIONING.md";
const CI_DOC: &str = "Docs/CI_CD_AND_SECURITY.md";
const SYSTEM_JOB: &str = "system-onnxruntime";
const ADR_TITLE: &str = "Release Dry Run Publishes Nothing";

/// The `.cargo/config.toml` written into the vendor tarball (the `cargo vendor` snippet).
const VENDOR_CONFIG: &str = "[source.crates-io]\nreplace-with = \"vendored-sources\"\n\n\
                             [source.vendored-sources]\ndirectory = \"vendor\"\n";

const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// Fixed commit time of the temporary repositories (2026-01-02 03:04:05 UTC).
const FIXTURE_EPOCH: u64 = 1_767_323_045;
const FIXTURE_TIME: &str = "2026-01-02 03:04:05";

/// Commands (first token, or first token + git/cargo/... subcommand) that publish, sign,
/// upload, tag, push or reach the network directly.
const FORBIDDEN_COMMANDS: [&str; 11] = [
    "gh", "gpg", "gpg2", "cosign", "minisign", "signify", "curl", "wget", "twine", "scp", "rsync",
];
const FORBIDDEN_SUBCOMMANDS: [(&str, &str); 8] = [
    ("git", "tag"),
    ("git", "push"),
    ("git", "send-email"),
    ("cargo", "publish"),
    ("cargo", "login"),
    ("docker", "push"),
    ("docker", "login"),
    ("npm", "publish"),
];
const FORBIDDEN_WORDS: [&str; 4] = ["--sign", "upload-artifact", "softprops/", "--upload-file"];

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

/// Lines of one CI job (from `  <name>:` to the next section comment).
fn ci_job<'a>(ci: &'a str, name: &str) -> &'a str {
    let start = ci
        .find(&format!("\n  {name}:"))
        .unwrap_or_else(|| panic!("ci.yml must define the {name} job"));
    let job = &ci[start + 1..];
    let end = job.find("\n  # ----").unwrap_or(job.len());
    &job[..end]
}

/// Shell code without comment lines and without trailing ` # ...` comments.
fn code_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| match l.find(" # ") {
            Some(i) => l[..i].to_string(),
            None => l.to_string(),
        })
        .collect()
}

/// Shell words of one line: split on whitespace and shell punctuation.
fn words(line: &str) -> Vec<&str> {
    line.split(|c: char| c.is_whitespace() || ";|&()`\"'{}<>$".contains(c))
        .filter(|w| !w.is_empty())
        .collect()
}

/// Every forbidden command or word found in `text` (shell code).
fn forbidden_hits(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for line in code_lines(text) {
        let ws = words(&line);
        for (i, w) in ws.iter().enumerate() {
            if FORBIDDEN_COMMANDS.contains(w) {
                hits.push(format!("`{w}` in: {line}"));
            }
            for (cmd, sub) in FORBIDDEN_SUBCOMMANDS {
                if *w != cmd {
                    continue;
                }
                // First non-option word after the command (skipping `-c x`/`-C dir` values).
                let mut j = i + 1;
                while j < ws.len() && ws[j].starts_with('-') {
                    j += if ws[j] == "-c" || ws[j] == "-C" { 2 } else { 1 };
                }
                if ws.get(j) == Some(&sub) {
                    hits.push(format!("`{cmd} {sub}` in: {line}"));
                }
            }
        }
        for word in FORBIDDEN_WORDS {
            if line.contains(word) {
                hits.push(format!("`{word}` in: {line}"));
            }
        }
    }
    hits
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A temporary directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "soos-rdr-{tag}-{}-{n}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Runs `bash <script> <args>` from `cwd` with an isolated git configuration.
fn run_script(script: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new("bash")
        .arg(script)
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("TZ", "UTC")
        .output()
        .expect("bash runs")
}

fn git(dir: &Path, args: &[&str]) -> Output {
    let date = format!("{FIXTURE_EPOCH} +0000");
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "soos test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "soos test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        stderr_of(&out)
    );
    out
}

/// A committed, dependency-free Cargo workspace with a copy of the script (version 1.2.3 in
/// `[workspace.package]`, decoy `version` keys in other tables, commit time `FIXTURE_EPOCH`).
fn fixture_repo(workspace_version: Option<&str>) -> TempDir {
    let dir = TempDir::new("repo");
    let root = dir.path();
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::copy(
        workspace_root().join(SCRIPT),
        root.join("scripts/release_dry_run.sh"),
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/a/src")).unwrap();
    let package = workspace_version.map_or(String::new(), |v| {
        format!("[workspace.package]\nversion = \"{v}\"\nedition = \"2021\"\n\n")
    });
    let edition = if workspace_version.is_some() {
        "edition.workspace = true"
    } else {
        "edition = \"2021\""
    };
    let crate_version = workspace_version.unwrap_or("0.0.1");
    fs::write(
        root.join("Cargo.toml"),
        format!(
            "[workspace]\nresolver = \"2\"\nmembers = [\"crates/a\"]\n\n\
             [workspace.metadata.decoy]\nversion = \"9.9.9\"\n\n{package}\
             [workspace.metadata.later]\nversion = \"8.8.8\"\n"
        ),
    )
    .unwrap();
    fs::write(
        root.join("crates/a/Cargo.toml"),
        format!("[package]\nname = \"a\"\nversion = \"{crate_version}\"\n{edition}\n"),
    )
    .unwrap();
    fs::write(root.join("crates/a/src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(
        root.join("Cargo.lock"),
        format!(
            "# This file is automatically @generated by Cargo.\n\
             # It is not intended for manual editing.\nversion = 4\n\n\
             [[package]]\nname = \"a\"\nversion = \"{crate_version}\"\n"
        ),
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "/target/\n").unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["add", "-A"]);
    git(
        root,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "fixture",
        ],
    );
    dir
}

/// `[workspace.package] version` of the repository `Cargo.toml`, parsed independently.
fn workspace_package_version() -> String {
    let cargo = read("Cargo.toml");
    let mut inside = false;
    for line in cargo.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            inside = t == "[workspace.package]";
            continue;
        }
        if inside {
            if let Some(v) = t.strip_prefix("version = \"") {
                return v.trim_end_matches('"').to_string();
            }
        }
    }
    panic!("Cargo.toml must declare [workspace.package] version");
}

fn hex64(seed: u8) -> String {
    format!("{seed:02x}").repeat(32)
}

/// A vendor directory and a lock file, one registry package per `(name, version, checksum)`.
struct VendorFixture {
    dir: TempDir,
}

impl VendorFixture {
    fn new(packages: &[(&str, &str, &str)]) -> Self {
        let dir = TempDir::new("vendor");
        let vendor = dir.path().join("vendor");
        fs::create_dir_all(&vendor).unwrap();
        let mut lock = String::from(
            "version = 4\n\n[[package]]\nname = \"local\"\nversion = \"0.1.0\"\n\
             dependencies = [\n \"a\",\n]\n",
        );
        for (name, version, sum) in packages {
            lock.push_str(&format!(
                "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n\
                 source = \"{REGISTRY}\"\nchecksum = \"{sum}\"\n"
            ));
            let crate_dir = vendor.join(name);
            fs::create_dir_all(&crate_dir).unwrap();
            fs::write(
                crate_dir.join(".cargo-checksum.json"),
                format!(
                    "{{\"$comment\":\"x\",\"files\":{{\"src/lib.rs\":\"{}\"}},\"package\":\"{sum}\"}}",
                    hex64(0xee)
                ),
            )
            .unwrap();
        }
        fs::write(dir.path().join("Cargo.lock"), lock).unwrap();
        Self { dir }
    }

    fn vendor(&self) -> PathBuf {
        self.dir.path().join("vendor")
    }

    fn lock(&self) -> PathBuf {
        self.dir.path().join("Cargo.lock")
    }

    fn check(&self) -> Output {
        run_script(
            &workspace_root().join(SCRIPT),
            self.dir.path(),
            &[
                "--check-vendor",
                self.vendor().to_str().unwrap(),
                self.lock().to_str().unwrap(),
            ],
        )
    }
}

fn base_packages() -> Vec<(&'static str, &'static str, String)> {
    vec![
        ("a", "1.0.0", hex64(0x0a)),
        ("b", "2.0.0", hex64(0x0b)),
        ("c", "3.0.0", hex64(0x0c)),
    ]
}

fn fixture(packages: &[(&str, &str, String)]) -> VendorFixture {
    let borrowed: Vec<(&str, &str, &str)> = packages
        .iter()
        .map(|(n, v, s)| (*n, *v, s.as_str()))
        .collect();
    VendorFixture::new(&borrowed)
}

fn assert_rejected(out: &Output, why: &str) {
    assert_eq!(
        out.status.code(),
        Some(1),
        "--check-vendor must fail closed (exit 1) for {why}; stdout: {} stderr: {}",
        stdout_of(out),
        stderr_of(out)
    );
}

/// RDR-S1 self-test: the scanner finds every forbidden form and ignores comments and prose.
#[test]
fn test_rdr_s1_forbidden_command_scanner_self_test() {
    for bad in [
        "gh release create v1",
        "  git tag -s v1",
        "git -c user.name=x push origin",
        "git -C dir tag v1",
        "gpg --detach-sign x",
        "cosign sign-blob x",
        "cargo publish --dry-run",
        "x | curl -T y",
        "wget https://example.invalid",
        "uses: actions/upload-artifact@deadbeef",
        "sha256sum x --sign",
        "docker push img",
    ] {
        assert!(!forbidden_hits(bad).is_empty(), "scanner must catch: {bad}");
    }
    for good in [
        "# gh release, git tag, git push and gpg are owner-only",
        "git -c tar.umask=0022 archive --format=tar HEAD",
        "git log -1 --format=%ct HEAD",
        "info \"this run publishes nothing\"",
        "docker run --rm --network none img",
        "cargo vendor --locked --offline vendor",
        "tar --sort=name -cf - x # never signed",
    ] {
        assert!(
            forbidden_hits(good).is_empty(),
            "scanner must accept: {good}: {:?}",
            forbidden_hits(good)
        );
    }
}

/// RDR-S1: the script, the harness and the CI job never tag, push, sign, upload or publish.
#[test]
fn test_rdr_s1_script_and_harness_never_publish_sign_or_upload() {
    for rel in [SCRIPT, HARNESS] {
        let hits = forbidden_hits(&read(rel));
        assert!(
            hits.is_empty(),
            "{rel} must never publish, sign, tag, push or upload:\n{}",
            hits.join("\n")
        );
    }
    let ci = read(CI);
    let job = ci_job(&ci, SYSTEM_JOB);
    let run_lines: String = job
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("run: "))
        .collect::<Vec<_>>()
        .join("\n");
    let mut hits = forbidden_hits(&run_lines);
    hits.extend(forbidden_hits(job));
    assert!(
        hits.is_empty(),
        "the {SYSTEM_JOB} job must never publish or upload:\n{}",
        hits.join("\n")
    );
}

/// RDR-S2: a vendor directory that matches `Cargo.lock` package by package is accepted, and
/// the check counts the crates.
#[test]
fn test_rdr_s2_check_vendor_accepts_matching_set() {
    let fx = fixture(&base_packages());
    let out = fx.check();
    assert!(
        out.status.success(),
        "matching vendor set must pass: {} {}",
        stdout_of(&out),
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).contains("3 vendored crates match Cargo.lock"),
        "the check reports the matched count: {}",
        stdout_of(&out)
    );
    // Same set in a different directory order and with versioned directory names.
    let fx = fixture(&base_packages());
    fs::rename(fx.vendor().join("a"), fx.vendor().join("a-1.0.0")).unwrap();
    assert!(
        fx.check().status.success(),
        "directory names are not trusted, only checksums"
    );
}

/// RDR-S2: every mismatch between the vendor directory and `Cargo.lock` fails closed.
#[test]
fn test_rdr_s2_check_vendor_rejects_mismatches() {
    // Altered package checksum of one vendored crate.
    let fx = fixture(&base_packages());
    let path = fx.vendor().join("b/.cargo-checksum.json");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace(&hex64(0x0b), &hex64(0x1b))).unwrap();
    assert_rejected(&fx.check(), "an altered package checksum");

    // A crate of Cargo.lock missing from the vendor directory.
    let fx = fixture(&base_packages());
    fs::remove_dir_all(fx.vendor().join("c")).unwrap();
    assert_rejected(&fx.check(), "a missing crate");

    // An extra vendored crate absent from Cargo.lock.
    let fx = fixture(&base_packages());
    fs::create_dir_all(fx.vendor().join("d")).unwrap();
    fs::write(
        fx.vendor().join("d/.cargo-checksum.json"),
        format!("{{\"files\":{{}},\"package\":\"{}\"}}", hex64(0x0d)),
    )
    .unwrap();
    assert_rejected(&fx.check(), "an extra crate");

    // A duplicate: two directories with the same checksum, one lock crate missing.
    let fx = fixture(&base_packages());
    fs::remove_dir_all(fx.vendor().join("c")).unwrap();
    fs::create_dir_all(fx.vendor().join("c")).unwrap();
    fs::copy(
        fx.vendor().join("a/.cargo-checksum.json"),
        fx.vendor().join("c/.cargo-checksum.json"),
    )
    .unwrap();
    assert_rejected(&fx.check(), "a duplicated checksum");

    // A vendored directory without .cargo-checksum.json.
    let fx = fixture(&base_packages());
    fs::remove_file(fx.vendor().join("a/.cargo-checksum.json")).unwrap();
    assert_rejected(&fx.check(), "a missing .cargo-checksum.json");

    // A .cargo-checksum.json without a package checksum (or with two).
    let fx = fixture(&base_packages());
    fs::write(fx.vendor().join("a/.cargo-checksum.json"), "{\"files\":{}}").unwrap();
    assert_rejected(&fx.check(), "a checksum file without a package checksum");
    let fx = fixture(&base_packages());
    fs::write(
        fx.vendor().join("a/.cargo-checksum.json"),
        format!(
            "{{\"package\":\"{}\",\"package\":\"{}\"}}",
            hex64(0x0a),
            hex64(0x0a)
        ),
    )
    .unwrap();
    assert_rejected(&fx.check(), "a checksum file with two package checksums");

    // A symlink or a plain file at the top of the vendor directory.
    let fx = fixture(&base_packages());
    std::os::unix::fs::symlink(fx.vendor().join("a"), fx.vendor().join("link")).unwrap();
    assert_rejected(&fx.check(), "a symlink in the vendor directory");
    let fx = fixture(&base_packages());
    fs::write(fx.vendor().join("stray.txt"), "x").unwrap();
    assert_rejected(&fx.check(), "a plain file in the vendor directory");

    // A git source in Cargo.lock (not a registry crate).
    let fx = fixture(&base_packages());
    let lock = fs::read_to_string(fx.lock()).unwrap();
    fs::write(
        fx.lock(),
        format!(
            "{lock}\n[[package]]\nname = \"g\"\nversion = \"0.1.0\"\n\
             source = \"git+https://example.invalid/g#0123456\"\n"
        ),
    )
    .unwrap();
    assert_rejected(&fx.check(), "a git source");

    // Another registry.
    let fx = fixture(&base_packages());
    let lock = fs::read_to_string(fx.lock()).unwrap();
    fs::write(
        fx.lock(),
        lock.replacen(REGISTRY, "registry+https://example.invalid/index", 1),
    )
    .unwrap();
    assert_rejected(&fx.check(), "a registry other than crates.io");

    // A registry package without a checksum line.
    let fx = fixture(&base_packages());
    let lock = fs::read_to_string(fx.lock()).unwrap();
    fs::write(
        fx.lock(),
        lock.replacen(&format!("checksum = \"{}\"\n", hex64(0x0c)), "", 1),
    )
    .unwrap();
    assert_rejected(&fx.check(), "a registry package without checksum");

    // A malformed (non 64-hex) checksum in Cargo.lock.
    let fx = fixture(&base_packages());
    let lock = fs::read_to_string(fx.lock()).unwrap();
    fs::write(fx.lock(), lock.replacen(&hex64(0x0a), "abc", 1)).unwrap();
    assert_rejected(&fx.check(), "a malformed lock checksum");

    // An empty vendor directory against a non-empty lock file.
    let fx = fixture(&base_packages());
    for name in ["a", "b", "c"] {
        fs::remove_dir_all(fx.vendor().join(name)).unwrap();
    }
    assert_rejected(&fx.check(), "an empty vendor directory");
}

/// RDR-S3: the `--dry-run` plan of the repository names the `[workspace.package]` version and
/// the HEAD commit time, and writes nothing.
#[test]
fn test_rdr_s3_dry_run_plan_uses_workspace_package_version_and_commit_time() {
    let root = workspace_root();
    let version = workspace_package_version();
    let epoch = Command::new("git")
        .args(["log", "-1", "--format=%ct", "HEAD"])
        .current_dir(&root)
        .output()
        .expect("git log");
    let epoch = stdout_of(&epoch).trim().to_string();
    assert!(epoch.parse::<u64>().is_ok(), "commit time: {epoch}");
    let tmp = TempDir::new("plan");
    let out_dir = tmp.path().join("out");
    let out = run_script(
        &root.join(SCRIPT),
        &root,
        &["--dry-run", "--out-dir", out_dir.to_str().unwrap()],
    );
    let text = stdout_of(&out);
    assert!(
        out.status.success(),
        "--dry-run: {text} {}",
        stderr_of(&out)
    );
    for needle in [
        format!("version: {version} ([workspace.package] of HEAD:Cargo.toml)"),
        format!("SOURCE_DATE_EPOCH={epoch}"),
        format!("soos-{version}.tar.gz"),
        format!("soos-{version}-vendor.tar.zst"),
        "SHA256SUMS".to_string(),
        format!("output: {}", out_dir.display()),
        "cargo fetch --locked".to_string(),
        "cargo vendor --locked --offline".to_string(),
        "nothing is tagged, signed, uploaded or published".to_string(),
    ] {
        assert!(
            text.contains(&needle),
            "plan must contain `{needle}`:\n{text}"
        );
    }
    assert!(
        !out_dir.exists(),
        "--dry-run must not create the output directory"
    );
    // Default output directory: target/release-dry-run of the repository.
    let out = run_script(&root.join(SCRIPT), &root, &["--dry-run"]);
    assert!(
        stdout_of(&out).contains(&format!(
            "output: {}",
            root.join("target/release-dry-run").display()
        )),
        "default output directory: {}",
        stdout_of(&out)
    );
    // Static: the version is read from the committed manifest, in [workspace.package].
    let script = read(SCRIPT);
    assert!(script.contains("git -C \"${ROOT}\" show HEAD:Cargo.toml"));
    assert!(script.contains("[workspace.package]"));
    assert!(script.contains("git -C \"${ROOT}\" log -1 --format=%ct HEAD"));
}

/// RDR-S3: the version comes from `[workspace.package]` of the committed `Cargo.toml` only:
/// decoy `version` keys in other tables and an uncommitted edit are ignored, and a manifest
/// without it fails.
#[test]
fn test_rdr_s3_version_comes_from_head_workspace_package() {
    let repo = fixture_repo(Some("1.2.3"));
    let root = repo.path();
    // Uncommitted edit of the version: HEAD still says 1.2.3.
    let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        cargo.replace("version = \"1.2.3\"", "version = \"7.7.7\""),
    )
    .unwrap();
    let out = run_script(
        &root.join("scripts/release_dry_run.sh"),
        root,
        &["--dry-run"],
    );
    let text = stdout_of(&out);
    assert!(out.status.success(), "{text} {}", stderr_of(&out));
    assert!(text.contains("soos-1.2.3.tar.gz"), "{text}");
    assert!(
        text.contains(&format!("SOURCE_DATE_EPOCH={FIXTURE_EPOCH}")),
        "{text}"
    );
    for decoy in ["9.9.9", "8.8.8", "7.7.7"] {
        assert!(
            !text.contains(&format!("soos-{decoy}")),
            "decoy {decoy}: {text}"
        );
    }
    assert!(
        stderr_of(&out).contains("uncommitted"),
        "a dirty work tree is reported: {}",
        stderr_of(&out)
    );

    let repo = fixture_repo(None);
    let out = run_script(
        &repo.path().join("scripts/release_dry_run.sh"),
        repo.path(),
        &["--dry-run"],
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "no [workspace.package] version must fail: {}",
        stdout_of(&out)
    );
    assert!(stderr_of(&out).contains("[workspace.package]"));
}

/// RDR-S4: bounded options; every usage error exits 2 and does nothing.
#[test]
fn test_rdr_s4_usage_errors_exit_2() {
    let root = workspace_root();
    let script = root.join(SCRIPT);
    let tmp = TempDir::new("usage");
    let d = tmp.path().to_str().unwrap().to_string();
    for args in [
        vec!["--bogus"],
        vec!["--dry-run", "--dry-run"],
        vec!["--offline", "--offline"],
        vec!["--out-dir"],
        vec!["--out-dir", ""],
        vec!["--out-dir", "/"],
        vec!["--out-dir", "--dry-run"],
        vec!["--out-dir", &d, "--out-dir", &d],
        vec!["--check-vendor"],
        vec!["--check-vendor", &d],
        vec!["--check-vendor", &d, &d, &d],
        vec!["--check-vendor", &d, "/nonexistent/Cargo.lock"],
        vec!["--dry-run", "--check-vendor", &d, &d],
        vec!["extra"],
    ] {
        let out = run_script(&script, &root, &args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?} must be a usage error (exit 2): {} {}",
            stdout_of(&out),
            stderr_of(&out)
        );
    }
    let out = run_script(&script, &root, &["--help"]);
    assert!(out.status.success());
    for needle in ["--dry-run", "--offline", "--out-dir", "--check-vendor"] {
        assert!(stdout_of(&out).contains(needle), "--help lists {needle}");
    }
}

/// `tar -tv` listing lines (UTC, numeric owners, full time) of an archive.
fn tar_listing(archive: &Path, decompress: &str) -> Vec<String> {
    let out = Command::new("tar")
        .args([
            decompress,
            "-tv",
            "--numeric-owner",
            "--full-time",
            "-f",
            archive.to_str().unwrap(),
        ])
        .env("TZ", "UTC")
        .output()
        .expect("tar runs");
    assert!(out.status.success(), "tar -t: {}", stderr_of(&out));
    stdout_of(&out).lines().map(str::to_string).collect()
}

/// Entry name of a `tar -tv --full-time` line (sixth field onwards).
fn entry_name(line: &str) -> String {
    line.split_whitespace()
        .skip(5)
        .collect::<Vec<_>>()
        .join(" ")
}

fn assert_normalized(listing: &[String], what: &str) {
    assert!(!listing.is_empty(), "{what} must not be empty");
    for line in listing {
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(fields[1], "0/0", "{what}: owner must be 0/0: {line}");
        assert_eq!(
            format!("{} {}", fields[3], fields[4]),
            FIXTURE_TIME,
            "{what}: mtime must be the commit time: {line}"
        );
        let mode = fields[0].as_bytes();
        assert!(
            mode[5] != b'w' && mode[8] != b'w',
            "{what}: no group/other write bit: {line}"
        );
        assert!(
            entry_name(line).starts_with("soos-1.2.3/"),
            "{what}: every entry is under soos-1.2.3/: {line}"
        );
    }
}

/// RDR1, RDR2, RDR3: end to end on a dependency-free repository, two runs (one with the
/// fetch, one `--offline`) are byte-identical; the archives are normalized (owner 0/0, mtime =
/// commit time, gzip without name or time, sorted vendor tar) and `SHA256SUMS` verifies.
#[test]
fn test_rdr1_two_runs_are_byte_identical_and_normalized() {
    let repo = fixture_repo(Some("1.2.3"));
    let root = repo.path();
    fs::write(root.join("UNTRACKED.txt"), "not committed\n").unwrap();
    let script = root.join("scripts/release_dry_run.sh");
    let out1 = root.join("target/run1");
    let out2 = root.join("target/run2");
    let r1 = run_script(&script, root, &["--out-dir", out1.to_str().unwrap()]);
    assert!(
        r1.status.success(),
        "run 1: {} {}",
        stdout_of(&r1),
        stderr_of(&r1)
    );
    let r2 = run_script(
        &script,
        root,
        &["--offline", "--out-dir", out2.to_str().unwrap()],
    );
    assert!(
        r2.status.success(),
        "run 2: {} {}",
        stdout_of(&r2),
        stderr_of(&r2)
    );
    assert!(stdout_of(&r1).contains("0 vendored crates match Cargo.lock"));

    let names = [
        "SHA256SUMS",
        "soos-1.2.3-vendor.tar.zst",
        "soos-1.2.3.tar.gz",
    ];
    for dir in [&out1, &out2] {
        let mut listed: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        listed.sort();
        assert_eq!(listed, names, "exactly the three outputs, no staging left");
    }
    for name in names {
        assert_eq!(
            fs::read(out1.join(name)).unwrap(),
            fs::read(out2.join(name)).unwrap(),
            "{name} must be byte-identical across runs"
        );
    }

    // SHA256SUMS: bare names, both files, verifies.
    let sums = fs::read_to_string(out1.join("SHA256SUMS")).unwrap();
    let lines: Vec<&str> = sums.lines().collect();
    assert_eq!(lines.len(), 2, "{sums}");
    assert!(lines[0].ends_with("  soos-1.2.3.tar.gz"), "{sums}");
    assert!(lines[1].ends_with("  soos-1.2.3-vendor.tar.zst"), "{sums}");
    let check = Command::new("sha256sum")
        .args(["-c", "--strict", "SHA256SUMS"])
        .current_dir(&out1)
        .output()
        .expect("sha256sum");
    assert!(
        check.status.success(),
        "sha256sum -c: {}",
        stdout_of(&check)
    );

    // gzip header: no file name flag, MTIME zero.
    let gz = fs::read(out1.join("soos-1.2.3.tar.gz")).unwrap();
    assert_eq!(&gz[..3], &[0x1f, 0x8b, 0x08], "gzip deflate member");
    assert_eq!(gz[3] & 0x08, 0, "gzip -n: no original file name");
    assert_eq!(&gz[4..8], &[0, 0, 0, 0], "gzip -n: MTIME zero");

    // Source tarball: HEAD only, normalized, no vendor or cargo configuration.
    let src = tar_listing(&out1.join("soos-1.2.3.tar.gz"), "-z");
    assert_normalized(&src, "source tarball");
    let src_names: Vec<String> = src.iter().map(|l| entry_name(l)).collect();
    for needed in [
        "soos-1.2.3/Cargo.toml",
        "soos-1.2.3/Cargo.lock",
        "soos-1.2.3/scripts/release_dry_run.sh",
    ] {
        assert!(
            src_names.iter().any(|n| n == needed),
            "{needed} in {src_names:?}"
        );
    }
    for absent in [
        "UNTRACKED.txt",
        "/target/",
        "/.cargo/",
        "/vendor/",
        "/.git/",
    ] {
        assert!(
            !src_names.iter().any(|n| n.contains(absent)),
            "{absent} must not be in the source tarball: {src_names:?}"
        );
    }

    // Vendor tarball: exactly the cargo configuration and the vendor directory, sorted.
    let vendor = tar_listing(&out1.join("soos-1.2.3-vendor.tar.zst"), "--zstd");
    assert_normalized(&vendor, "vendor tarball");
    let vendor_names: Vec<String> = vendor.iter().map(|l| entry_name(l)).collect();
    assert_eq!(
        vendor_names,
        ["soos-1.2.3/.cargo/config.toml", "soos-1.2.3/vendor/"],
        "vendor tarball entries"
    );
    let config = Command::new("tar")
        .args([
            "--zstd",
            "-xOf",
            out1.join("soos-1.2.3-vendor.tar.zst").to_str().unwrap(),
            "soos-1.2.3/.cargo/config.toml",
        ])
        .output()
        .expect("tar -xO");
    assert_eq!(
        stdout_of(&config),
        VENDOR_CONFIG,
        "vendored-sources snippet"
    );

    // A second run into the same directory replaces the outputs and leaves no staging.
    let r3 = run_script(
        &script,
        root,
        &["--offline", "--out-dir", out1.to_str().unwrap()],
    );
    assert!(r3.status.success(), "{}", stderr_of(&r3));
    assert_eq!(fs::read_dir(&out1).unwrap().count(), 3);
}

/// RDR-S5: determinism flags, environment hygiene and the step order of the script.
#[test]
fn test_rdr_s5_script_pins_determinism_and_step_order() {
    let script = read(SCRIPT);
    let code = code_lines(&script).join("\n");
    for needle in [
        "set -euo pipefail",
        "umask 022",
        "export LC_ALL=C",
        "export TZ=UTC",
        "unset TAR_OPTIONS GZIP ZSTD_CLEVEL ZSTD_NBTHREADS",
        "readonly ZSTD_LEVEL=12",
        "export SOURCE_DATE_EPOCH",
        "git -C \"${ROOT}\" -c tar.umask=0022 archive --format=tar --prefix=\"${PREFIX}/\" HEAD",
        "gzip -n -9",
        "--sort=name",
        "--format=gnu",
        "--owner=0 --group=0 --numeric-owner",
        "--mtime=\"@${SOURCE_DATE_EPOCH}\"",
        "--mode=u+rwX,go+rX,go-w",
        "--hard-dereference",
        "zstd \"-${ZSTD_LEVEL}\" -T1 -q -c",
        "sha256sum \"${SOURCE_TARBALL}\" \"${VENDOR_TARBALL}\"",
        "trap cleanup EXIT",
    ] {
        assert!(code.contains(needle), "{SCRIPT} must contain `{needle}`");
    }
    let pos = |needle: &str| {
        code.find(needle)
            .unwrap_or_else(|| panic!("{SCRIPT} must contain `{needle}`"))
    };
    let main = pos("\nmain() {");
    let after = |needle: &str| {
        main + code[main..]
            .find(needle)
            .unwrap_or_else(|| panic!("main() must call `{needle}`"))
    };
    let archive = after("build_source_tarball");
    let fetch = after("fetch_crates");
    let vendor = after("vendor_crates");
    let check = after("check_vendor ");
    let pack = after("build_vendor_tarball");
    let sums = after("write_checksums");
    let publish = after("move_outputs");
    assert!(
        archive < fetch
            && fetch < vendor
            && vendor < check
            && check < pack
            && pack < sums
            && sums < publish,
        "order: source tarball, fetch, vendor, vendor check, vendor tarball, checksums, move"
    );
    assert!(code.contains("cargo fetch --locked"));
    assert!(code.contains("CARGO_NET_OFFLINE=true cargo vendor --locked --offline"));
    // The only online step is the fetch, and --offline skips it.
    assert_eq!(code.matches("cargo fetch").count(), 1, "one fetch call");
    assert!(
        !code.contains("cargo vendor --locked vendor"),
        "vendor runs offline"
    );
}

/// RDR-S6: the Docker harness proves reproducibility and the offline build from the tarballs.
#[test]
fn test_rdr_s6_harness_builds_offline_from_tarballs_only() {
    let harness = read(HARNESS);
    let code = code_lines(&harness).join("\n");
    for needle in [
        "set -euo pipefail",
        "readonly DOCKERFILE=\"tests/docker/Dockerfile.system-ort\"",
        "scripts/release_dry_run.sh\" --out-dir \"${work}/run1\"",
        "scripts/release_dry_run.sh\" --offline --out-dir \"${work}/run2\"",
        "cmp ",
        "docker run --rm --network none",
        "-v \"${work}/run1:/release:ro\"",
        "-e ORT_SKIP_DOWNLOAD=1",
        "-e CARGO_NET_OFFLINE=true",
        "-e CARGO_HOME=/build/cargo-home",
        "sha256sum -c --strict SHA256SUMS",
        "cargo build --locked --offline --release --workspace --no-default-features --features system-onnxruntime",
        "/usr/local/cargo/registry",
        "-name .git",
        "NEEDED.*\\[libonnxruntime\\.so\\.1\\]",
        "--in-container",
    ] {
        assert!(code.contains(needle), "{HARNESS} must contain `{needle}`");
    }
    // The repository and any cargo registry are never mounted into the offline container.
    for forbidden in [
        "${root}:/workspace",
        "/usr/local/cargo/registry\"",
        "REGISTRY_VOLUME",
    ] {
        assert!(
            !code.contains(forbidden),
            "{HARNESS} must not mount `{forbidden}` into the offline build"
        );
    }
    assert!(
        code.contains("SOOS_SORT_CONTAINER") && code.contains("/.dockerenv"),
        "--in-container runs only inside the image"
    );
}

/// RDR-S7: CI runs the harness inside the advisory `system-onnxruntime` job (same check name,
/// required on `main` through `ci-success-main`) and uploads or releases nothing.
#[test]
fn test_rdr_s7_ci_runs_dry_run_in_advisory_system_job_without_upload() {
    let ci = read(CI);
    let job = ci_job(&ci, SYSTEM_JOB);
    let system = job
        .find("run: ./tests/docker/system_onnxruntime_test.sh")
        .expect("system harness step");
    let toolchain = job
        .find("run: rustup toolchain install --profile minimal")
        .expect("host toolchain step for cargo vendor");
    let dry_run = job
        .find(&format!("run: ./{HARNESS}"))
        .expect("release dry run step");
    assert!(
        system < toolchain && toolchain < dry_run,
        "dry run after the system harness"
    );
    let step = &job[job[..dry_run].rfind("- name:").unwrap()..];
    assert!(
        step.contains("timeout-minutes: 40"),
        "dry run step timeout:\n{step}"
    );
    for forbidden in ["upload-artifact", "gh release", "softprops/"] {
        assert!(
            !ci.contains(forbidden),
            "ci.yml must not contain `{forbidden}`"
        );
    }
    assert!(
        !job.contains("actions/cache/save"),
        "the {SYSTEM_JOB} job saves no cache (nothing produced leaves the runner)"
    );
    let env_end = ci.find("\njobs:").expect("jobs");
    assert!(
        ci[..env_end].contains("permissions:\n  contents: read\n"),
        "workflow permissions stay read-only"
    );
    assert!(!job.contains("permissions:"), "the job adds no permission");
    assert_eq!(
        ci.matches(&format!("./{HARNESS}")).count(),
        1,
        "the dry run runs in exactly one job"
    );
}

/// RDR-S8: ADR and documentation.
#[test]
fn test_rdr_s8_adr_and_docs() {
    let decisions = read(DECISIONS);
    let adr = decisions
        .lines()
        .find(|l| l.contains(ADR_TITLE))
        .unwrap_or_else(|| panic!("ADR \"{ADR_TITLE}\""));
    for needle in [
        "#444",
        "#361",
        SCRIPT,
        HARNESS,
        "SOURCE_DATE_EPOCH",
        "[workspace.package]",
        ".cargo-checksum.json",
        "SHA256SUMS",
        "--network none",
        "upload-artifact",
        "ADVISORY_CHECKS",
        "OpenPGP",
        "owner",
    ] {
        assert!(adr.contains(needle), "the ADR must mention `{needle}`");
    }
    let releasing = read(RELEASING_DOC);
    for needle in [
        "scripts/release_dry_run.sh",
        "--dry-run",
        "SHA256SUMS",
        "Owner-only",
        "OpenPGP",
        "signed tag",
        "GitHub Release",
        "AUR",
        "never",
    ] {
        assert!(
            releasing.contains(needle),
            "{RELEASING_DOC} must mention `{needle}`"
        );
    }
    assert!(
        read(PACKAGING_DOC).contains("(RELEASING.md)"),
        "{PACKAGING_DOC} points to RELEASING.md"
    );
    assert!(
        read(CI_DOC).contains(HARNESS),
        "{CI_DOC} documents the release dry run harness"
    );
    assert!(read("Docs/README.md").contains("](RELEASING.md)"));
}
