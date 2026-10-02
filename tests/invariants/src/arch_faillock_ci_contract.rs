//! Follow-ups of the 2026-10-02 review batch (GitHub #318; walkthrough 170, matrix rows
//! AFC1–AFC8):
//!
//! - on Arch, a face match lands on the success path of the stock pambase stack
//!   (`pam_permit`, `pam_env`, `pam_faillock authsucc`) instead of ending the stack, and the
//!   Docker harness proves the tally reset and the kept lock (AFC1, AFC2);
//! - `tests/docker/Dockerfile.systemd` is pinned by digest like the other sandbox images (AFC3);
//! - the `ort-sys` ONNX Runtime download used by the required CI checks is cached and retried
//!   a bounded number of times (AFC4–AFC6);
//! - `scripts/install.sh --build` compiles `soos-gui` for its `--prefix`, packages keep
//!   `/usr/bin` (AFC7, AFC8; the GUI side is in `crates/gui/tests/program_path_tests.rs`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract test suite uses assertions"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::packaging_ownership_contract::{
    auth_rules, combined, root_shims, scratch, success_jump,
};

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

/// Exact primary rule of the Arch stack: jumps over `pam_systemd_home`, `pam_unix`, the event
/// line and `pam_faillock authfail` onto `pam_permit.so`.
const ARCH_FACE_LINE: &str = "auth  [success=4 default=ignore]  pam_soos.so";

// ---------------------------------------------------------------------------
// AFC1 / AFC2 — Arch face login runs pam_faillock authsucc and pam_env
// ---------------------------------------------------------------------------

/// AFC1 — the Arch primary rule lands on `pam_permit.so`, followed by `pam_env.so` and
/// `pam_faillock.so authsucc`, after `pam_faillock.so preauth`; the snippet, the deployment
/// guide, ARCHITECTURE §5 and the ADR register describe that path.
#[test]
fn test_arch_face_match_lands_on_faillock_authsucc_and_pam_env() {
    let full = read("packaging/pam/arch/system-auth");
    let rules = auth_rules(&full);
    let soos = rules
        .iter()
        .position(|r| r.module == "pam_soos.so" && !r.args.iter().any(|a| a.starts_with("event=")))
        .expect("primary pam_soos.so rule");
    let preauth = rules
        .iter()
        .position(|r| {
            r.module == "pam_faillock.so" && r.args.first().map(String::as_str) == Some("preauth")
        })
        .expect("pam_faillock.so preauth");
    assert!(
        preauth < soos,
        "faillock preauth must run before pam_soos (a locked account fails)"
    );
    assert_eq!(
        rules[soos].control, "[success=4 default=ignore]",
        "the Arch face rule must jump to pam_permit.so, never end the stack (success=done)"
    );
    let jump = success_jump(&rules[soos].control).expect("numeric success jump");
    let landing = soos + 1 + jump;
    let tail: Vec<(&str, Option<&str>)> = rules[landing..]
        .iter()
        .map(|r| (r.module.as_str(), r.args.first().map(String::as_str)))
        .collect();
    assert_eq!(
        tail,
        vec![
            ("pam_permit.so", None),
            ("pam_env.so", None),
            ("pam_faillock.so", Some("authsucc")),
        ],
        "a face match must run exactly the stock success path"
    );
    for skipped in &rules[soos + 1..landing] {
        assert!(
            skipped.module != "pam_env.so"
                && !(skipped.module == "pam_faillock.so"
                    && skipped.args.first().map(String::as_str) == Some("authsucc")),
            "the face jump must not skip {}",
            skipped.line
        );
    }
    assert!(
        full.lines().any(|l| l == ARCH_FACE_LINE),
        "packaging/pam/arch/system-auth must carry the exact line {ARCH_FACE_LINE:?}"
    );

    let snippet = read("packaging/pam/arch/system-auth.snippet");
    assert!(
        snippet.lines().any(|l| l == ARCH_FACE_LINE),
        "the snippet must carry {ARCH_FACE_LINE:?}"
    );
    assert!(
        snippet.contains("pam_faillock.so authsucc") && snippet.contains("pam_env.so"),
        "the snippet must say where the face jump lands"
    );

    let doc = read("Docs/DISTRIBUTION_DEPLOYMENT.md");
    let section = doc
        .split("### 5.2 ")
        .nth(1)
        .and_then(|s| s.split("### 5.3 ").next())
        .expect("Docs/DISTRIBUTION_DEPLOYMENT.md §5.2");
    assert!(
        section.contains(&format!("insert `{ARCH_FACE_LINE}`")),
        "§5.2 must document the success=4 primary rule"
    );
    assert!(
        !section.contains("A face match ends the auth stack"),
        "§5.2 still documents the former success=done behaviour"
    );
    assert!(
        section.contains("resets the faillock tally"),
        "§5.2 must state that a face login resets the faillock tally"
    );

    let architecture = read("AI/ARCHITECTURE.md");
    let pam = architecture
        .split("## 5. PAM Module Implementation")
        .nth(1)
        .and_then(|s| s.split("\n## 6.").next())
        .expect("ARCHITECTURE §5");
    assert!(
        pam.contains("success=4") && pam.contains("Arch Linux only"),
        "ARCHITECTURE §5 must record the Arch-only exception to success=done"
    );
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.lines().any(|l| l.contains("GitHub #318")
            && l.contains("[success=4 default=ignore]")
            && l.contains("authsucc")),
        "AI/DECISIONS.md must carry the ADR amending ARCHITECTURE §5 for Arch"
    );
}

/// AFC2 — the Arch harness, on the edited stock stack, resets the tally with a face login
/// after deny-1 wrong passwords and keeps a locked account locked under a face Allow.
#[test]
fn test_arch_harness_face_login_resets_tally_and_lock_still_fails() {
    let harness = read("tests/distro/arch_linux_test.sh");
    for needle in [
        "faillock_deny_limit()",
        "/etc/security/faillock.conf",
        "wrong_passwords \"${BELOW_DENY}\"",
        "assert_tally 0 \"face Allow after ${BELOW_DENY} wrong password(s)\"",
        "wrong_passwords \"${FAILLOCK_DENY}\"",
        "a face Allow unlocked an account locked by pam_faillock",
        "assert_tally \"${FAILLOCK_DENY}\" \"face Allow on a locked account\"",
        "print \"auth  [success=4 default=ignore]  pam_soos.so\"",
    ] {
        assert!(
            harness.contains(needle),
            "tests/distro/arch_linux_test.sh must contain {needle:?}"
        );
    }
    // The new scenarios run after the stock stack is installed and before it is restored.
    let installed = harness
        .find("install -m 0644 \"${EDITED_SYSTEM_AUTH}\" /etc/pam.d/system-auth")
        .expect("edited stack installed");
    let restored = harness
        .find("install -m 0644 \"${SYNTHETIC_SYSTEM_AUTH_BACKUP}\" /etc/pam.d/system-auth")
        .expect("synthetic stack restored");
    let scenario = harness
        .find("wrong_passwords \"${FAILLOCK_DENY}\"")
        .expect("locked scenario");
    assert!(installed < scenario && scenario < restored);
}

// ---------------------------------------------------------------------------
// AFC3 — Dockerfile.systemd pinned by digest
// ---------------------------------------------------------------------------

fn from_lines(rel: &str) -> Vec<String> {
    read(rel)
        .lines()
        .filter(|l| l.starts_with("FROM "))
        .map(str::to_string)
        .collect()
}

/// AFC3 — the systemd runtime image uses the same `ubuntu:24.04@sha256:` digest as the
/// release build image `tests/docker/Dockerfile.ubuntu` (same glibc).
#[test]
fn test_systemd_runtime_image_uses_the_build_image_digest() {
    let systemd = from_lines("tests/docker/Dockerfile.systemd");
    let ubuntu = from_lines("tests/docker/Dockerfile.ubuntu");
    assert_eq!(systemd.len(), 1, "Dockerfile.systemd has one FROM line");
    let digest = systemd[0]
        .strip_prefix("FROM ubuntu:24.04@sha256:")
        .expect("Dockerfile.systemd must pin ubuntu:24.04 by digest");
    assert!(
        digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()),
        "64-hex digest expected: {}",
        systemd[0]
    );
    assert_eq!(
        systemd, ubuntu,
        "the systemd runtime image and the release build image must share one base digest"
    );
}

// ---------------------------------------------------------------------------
// AFC4 – AFC6 — ONNX Runtime download cache and bounded retry
// ---------------------------------------------------------------------------

const CACHE_RESTORE: &str =
    "uses: actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0";
const CACHE_SAVE: &str =
    "uses: actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0";
const ORT_CACHE_PATH: &str = "path: ~/.cache/ort.pyke.io";
const ORT_CACHE_KEY: &str = "key: ort-${{ runner.os }}-${{ hashFiles('Cargo.lock') }}";

/// Body of the top-level job `name` of `ci.yml` (up to the next banner comment).
fn ci_job<'a>(ci: &'a str, name: &str) -> &'a str {
    let start = ci
        .find(&format!("\n  {name}:"))
        .unwrap_or_else(|| panic!("ci.yml must define the {name} job"));
    let job = &ci[start + 1..];
    let end = job.find("\n  # ----").unwrap_or(job.len());
    &job[..end]
}

/// AFC4 — every required job that builds ONNX Runtime restores the SHA-pinned ORT cache keyed on
/// `Cargo.lock` before its first build, prefetches with the bounded retry (host jobs) or hands
/// the cache to the Docker harness, and only `main` saves it; permissions stay read-only.
#[test]
fn test_ci_caches_and_retries_the_onnxruntime_download() {
    let ci = read(".github/workflows/ci.yml");
    for (name, first_build, host) in [
        ("clippy", "run: cargo clippy", true),
        ("test", "run: cargo test", true),
        (
            "package-deploy",
            "run: ./tests/distro/run_distro_validation.sh ubuntu",
            false,
        ),
        (
            "systemd-unit",
            "run: ./tests/docker/systemd_unit_acceptance_test.sh --models download",
            false,
        ),
    ] {
        let job = ci_job(&ci, name);
        let restore = job
            .find(CACHE_RESTORE)
            .unwrap_or_else(|| panic!("{name}: must restore the ORT cache ({CACHE_RESTORE})"));
        let build = job
            .find(first_build)
            .unwrap_or_else(|| panic!("{name}: `{first_build}` not found"));
        assert!(
            restore < build,
            "{name}: the ORT cache must be restored before {first_build}"
        );
        let save = job
            .find(CACHE_SAVE)
            .unwrap_or_else(|| panic!("{name}: must save the ORT cache ({CACHE_SAVE})"));
        assert_eq!(
            job.matches(ORT_CACHE_PATH).count(),
            2,
            "{name}: restore and save use {ORT_CACHE_PATH}"
        );
        assert_eq!(
            job.matches(ORT_CACHE_KEY).count(),
            2,
            "{name}: restore and save use {ORT_CACHE_KEY}"
        );
        let save_step = &job[job[..save].rfind("- name:").expect("save step")..save];
        assert!(
            save_step.contains("if: github.ref == 'refs/heads/main'")
                && save_step.contains("steps.ort-cache.outputs.cache-hit != 'true'"),
            "{name}: only main writes the ORT cache (no cache poisoning from pull requests)"
        );
        assert!(job.contains("id: ort-cache"), "{name}: restore step id");
        if host {
            assert!(
                job.contains("ORT_CACHE_DIR=${HOME}/.cache/ort.pyke.io"),
                "{name}: ort-sys must use the cached directory"
            );
            let prefetch = job
                .find("run: ./scripts/prefetch_onnxruntime.sh")
                .unwrap_or_else(|| panic!("{name}: must prefetch with the bounded retry"));
            assert!(restore < prefetch && prefetch < build);
        } else {
            assert!(
                job.contains("SOOS_ORT_CACHE_DIR=${HOME}/.cache/ort.pyke.io"),
                "{name}: the Docker harness must receive the host ORT cache"
            );
        }
        assert!(
            !job.contains("permissions:"),
            "{name}: no job-level permissions (workflow default contents: read)"
        );
    }
    assert!(ci.contains("\npermissions:\n  contents: read\n"));
    assert!(
        !ci.contains(": write"),
        "ci.yml must not grant any write permission"
    );
}

/// Runs `scripts/prefetch_onnxruntime.sh` with a `cargo` shim that fails `failures` times.
fn run_prefetch(tag: &str, failures: u32, env: &[(&str, &str)]) -> (std::process::Output, usize) {
    let shims = scratch(&format!("afc_{tag}"));
    let counter = shims.join("calls");
    let cargo = shims.join("cargo");
    fs::write(
        &cargo,
        format!(
            "#!/bin/bash\necho \"$*\" >> \"{c}\"\nn=$(wc -l < \"{c}\")\n[[ \"$*\" == \"check --locked -p ort\" ]] || exit 99\n(( n > {failures} ))\n",
            c = counter.display()
        ),
    )
    .expect("write cargo shim");
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    let path = format!(
        "{}:{}",
        shims.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = Command::new("bash");
    cmd.arg(workspace_root().join("scripts/prefetch_onnxruntime.sh"))
        .env("PATH", path)
        .env("SOOS_ORT_FETCH_DELAY_S", "0")
        .env_remove("SOOS_ORT_FETCH_ATTEMPTS");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run prefetch_onnxruntime.sh");
    let calls = fs::read_to_string(&counter)
        .unwrap_or_default()
        .lines()
        .count();
    let _ = fs::remove_dir_all(&shims);
    (out, calls)
}

/// AFC5 — the prefetch retries `cargo check --locked -p ort` at most 3 times by default, stops
/// at the first success, fails after the last attempt, and rejects an unbounded attempt count.
#[test]
fn test_onnxruntime_prefetch_retries_a_bounded_number_of_times() {
    let (out, calls) = run_prefetch("transient", 2, &[]);
    assert!(
        out.status.success(),
        "2 failures then success:\n{}",
        combined(&out)
    );
    assert_eq!(calls, 3, "stops at the first success");

    let (out, calls) = run_prefetch("first", 0, &[]);
    assert!(out.status.success(), "{}", combined(&out));
    assert_eq!(calls, 1, "no retry when the first attempt succeeds");

    let (out, calls) = run_prefetch("down", 100, &[]);
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    assert_eq!(calls, 3, "default bound is 3 attempts");
    assert!(combined(&out).contains("failed after 3 attempt(s)"));

    for bad in ["0", "6", "1000", "x"] {
        let (out, calls) = run_prefetch(
            &format!("bad_{bad}"),
            100,
            &[("SOOS_ORT_FETCH_ATTEMPTS", bad)],
        );
        assert_eq!(
            out.status.code(),
            Some(2),
            "attempts={bad}: {}",
            combined(&out)
        );
        assert_eq!(calls, 0, "attempts={bad}: nothing may run");
    }
    let (out, _) = run_prefetch("bad_delay", 0, &[("SOOS_ORT_FETCH_DELAY_S", "3600")]);
    assert_eq!(out.status.code(), Some(2), "an unbounded delay is rejected");
}

/// AFC6 — the Docker harnesses that compile ONNX Runtime prefetch it with the bounded retry and
/// mount the host ORT cache only when `SOOS_ORT_CACHE_DIR` names an existing absolute directory.
#[test]
fn test_docker_harnesses_prefetch_and_share_the_onnxruntime_cache() {
    for script in [
        "tests/distro/debian_ubuntu_test.sh",
        "tests/distro/fedora_rhel_test.sh",
        "tests/distro/arch_linux_test.sh",
    ] {
        let text = read(script);
        let prefetch = text
            .find("bash scripts/prefetch_onnxruntime.sh")
            .unwrap_or_else(|| panic!("{script} must prefetch ONNX Runtime"));
        let build = text
            .find("    cargo build --locked --release --workspace")
            .unwrap_or_else(|| panic!("{script} release build"));
        assert!(
            prefetch < build,
            "{script}: prefetch before the release build"
        );
    }
    let distro = read("tests/distro/run_distro_validation.sh");
    let sua = read("tests/docker/systemd_unit_acceptance_test.sh");
    for (name, text) in [
        ("run_distro_validation.sh", &distro),
        ("systemd_unit_acceptance_test.sh", &sua),
    ] {
        for needle in [
            "if [[ -n \"${SOOS_ORT_CACHE_DIR:-}\" ]]; then",
            "\"${SOOS_ORT_CACHE_DIR}\" != /* || ! -d \"${SOOS_ORT_CACHE_DIR}\"",
            ":/ort-cache\" -e \"ORT_CACHE_DIR=/ort-cache\")",
        ] {
            assert!(text.contains(needle), "{name} must contain {needle:?}");
        }
    }
    assert!(
        sua.contains("bash /workspace/scripts/prefetch_onnxruntime.sh"),
        "the systemd harness must prefetch ONNX Runtime before its release build"
    );
}

// ---------------------------------------------------------------------------
// AFC7 / AFC8 — soos-gui program paths follow the install prefix
// ---------------------------------------------------------------------------

/// AFC7 — `install.sh --build --prefix /opt/soos` builds with `SOOS_BINDIR=/opt/soos/bin`, also
/// when it drops to the invoking user; installing prebuilt artifacts under another prefix warns.
#[test]
fn test_install_build_exports_the_prefix_bindir() {
    let shims = scratch("afc_bindir_shims");
    let log = shims.join("calls.log");
    root_shims(&shims, &log);
    let stage = scratch("afc_bindir_stage");
    let target = scratch("afc_bindir_target");
    let path = format!(
        "{}:{}",
        shims.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new("bash")
        .arg(workspace_root().join("scripts/install.sh"))
        .args([
            "--build",
            "--skip-models",
            "--skip-systemd",
            "--distro",
            "none",
        ])
        .args(["--prefix", "/opt/soos"])
        .arg("--destdir")
        .arg(&stage)
        .env("PATH", path)
        .env("CARGO_TARGET_DIR", &target)
        .env("SUDO_USER", "sudouser")
        .env_remove("DOAS_USER")
        .env_remove("PKEXEC_UID")
        .env_remove("SOOS_BINDIR")
        .output()
        .expect("run install.sh --build");
    let calls = fs::read_to_string(&log).unwrap_or_default();
    let _ = fs::remove_dir_all(&shims);
    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&target);
    let runuser = calls
        .lines()
        .find(|l| l.starts_with("runuser -u sudouser --"))
        .unwrap_or_else(|| {
            panic!(
                "install.sh must build through runuser: {calls:?}\n{}",
                combined(&out)
            )
        });
    assert!(
        runuser.contains("SOOS_BINDIR=\"$2\" cargo build --release --locked --workspace")
            && runuser.trim_end().ends_with(" /opt/soos/bin"),
        "the runuser build must export SOOS_BINDIR=<prefix>/bin: {runuser}"
    );

    let install = read("scripts/install.sh");
    assert!(
        install.contains("SOOS_BINDIR=\"${PREFIX}/bin\" \"${BUILD_CMD[@]}\""),
        "the non-root --build path must export SOOS_BINDIR=<prefix>/bin"
    );
    assert!(
        install.contains("soos-gui runs ${SOOS_GUI_ENROLL_DEFAULT}"),
        "install.sh must warn when prebuilt artifacts are installed under another prefix"
    );
}

/// AFC8 — every package builder compiles with `SOOS_BINDIR=/usr/bin` (packages install
/// `soos-enroll` there), so a stray environment value never reaches a package.
#[test]
fn test_package_builders_pin_soos_bindir_to_usr_bin() {
    for (file, needle) in [
        ("scripts/build_deb.sh", "export SOOS_BINDIR=/usr/bin"),
        ("scripts/build_arch.sh", "export SOOS_BINDIR=/usr/bin"),
        ("scripts/build_rpm.sh", "export SOOS_BINDIR=/usr/bin"),
        ("scripts/build_packages.sh", "export SOOS_BINDIR=/usr/bin"),
        ("packaging/arch/PKGBUILD", "export SOOS_BINDIR=/usr/bin"),
        ("packaging/rpm/soos.spec", "export SOOS_BINDIR=%{_bindir}"),
        ("packaging/debian/rules", "export SOOS_BINDIR = /usr/bin"),
    ] {
        let text = read(file);
        let at = text
            .find(needle)
            .unwrap_or_else(|| panic!("{file} must contain `{needle}`"));
        let build = text
            .find("cargo build --locked --release --workspace")
            .unwrap_or_else(|| panic!("{file} release build"));
        assert!(
            at < build,
            "{file}: SOOS_BINDIR must be set before the release build"
        );
    }
}
