//! GitHub #329 — install readiness race, foreign-owned build directory, presence wake settle
//! (spec `AI/architect_spec_install_presence_warmup.md`, rows IWP1–IWP7, IWP11, IWP13).
//!
//! - `scripts/wait_daemon_ready.sh` polls `soos-admin status` until it reports healthy within
//!   one bounded wait, prints only the final report, and bounds every attempt.
//! - `scripts/install.sh --build` refuses, in its read-only preflight, a cargo target directory
//!   the build user cannot write (foreign-owned files, non-writable directories) and prints the
//!   exact fix, before any build or change.
//! - The presence wake settle never reaches the PAM path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::print_stderr,
    reason = "Contract tests utilize direct assertions, panics, indexing and skip notices"
)]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use crate::installer_contract::scratch_dir;
use crate::packaging_ownership_contract::{combined, root_shims};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read_repo(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn write_exec(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod script");
}

fn have(tool: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {tool} >/dev/null 2>&1"))
        .status()
        .is_ok_and(|s| s.success())
}

// ---------------------------------------------------------------------------
// IWP1–IWP4 — bounded health poll of scripts/wait_daemon_ready.sh
// ---------------------------------------------------------------------------

struct Ready {
    work: PathBuf,
    manifest: PathBuf,
    socket: PathBuf,
    admin: PathBuf,
    counter: PathBuf,
    _listener: UnixListener,
}

impl Drop for Ready {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.work);
    }
}

/// A deployed manifest, a bound socket and a fake `soos-admin` that answers attempt `n`
/// with `{"attempt":n,"is_healthy":...}` and exits 0 from attempt `healthy_at` on.
fn ready_fixture(tag: &str, healthy_at: u32) -> Ready {
    let work = scratch_dir(&format!("iwp_ready_{tag}"));
    let manifest = work.join("manifest.toml");
    fs::write(&manifest, "[manifest]\nversion = \"2.0.0\"\n").expect("manifest");
    let socket = work.join("daemon.sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let counter = work.join("attempts");
    let admin = work.join("soos-admin");
    write_exec(
        &admin,
        &format!(
            "#!/bin/sh\n\
             n=$(cat '{c}' 2>/dev/null || echo 0)\n\
             n=$((n + 1))\n\
             echo \"$n\" > '{c}'\n\
             if [ \"$n\" -ge {healthy_at} ]; then\n\
             \x20 printf '{{\"attempt\":%s,\"is_healthy\":true}}\\n' \"$n\"\n\
             \x20 exit 0\n\
             fi\n\
             printf '{{\"attempt\":%s,\"is_healthy\":false}}\\n' \"$n\"\n\
             exit 1\n",
            c = counter.display()
        ),
    );
    Ready {
        work,
        manifest,
        socket,
        admin,
        counter,
        _listener: listener,
    }
}

fn attempts(fx: &Ready) -> u32 {
    fs::read_to_string(&fx.counter)
        .map(|s| s.trim().parse().expect("counter"))
        .unwrap_or(0)
}

fn run_ready(fx: &Ready, timeout: &str) -> (Output, Duration) {
    let start = Instant::now();
    let out = Command::new("bash")
        .arg(workspace_root().join("scripts/wait_daemon_ready.sh"))
        .args([
            OsStr::new("--socket"),
            fx.socket.as_os_str(),
            OsStr::new("--manifest"),
            fx.manifest.as_os_str(),
            OsStr::new("--admin"),
            fx.admin.as_os_str(),
            OsStr::new("--timeout"),
            OsStr::new(timeout),
        ])
        .output()
        .expect("run wait_daemon_ready.sh");
    (out, start.elapsed())
}

/// IWP1 — an unhealthy first answer (camera still starting) is not a failure: the helper
/// queries again until `soos-admin status` exits 0, within its `--timeout`.
#[test]
fn test_iwp_readiness_polls_until_status_is_healthy() {
    let fx = ready_fixture("poll", 3);
    let (out, elapsed) = run_ready(&fx, "10");
    assert!(
        out.status.success(),
        "a daemon that turns healthy on the third status query is ready (attempts: {}); output:\n{}",
        attempts(&fx),
        combined(&out)
    );
    assert_eq!(
        attempts(&fx),
        3,
        "the helper stops polling at the first healthy answer"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "readiness is reported as soon as the status is healthy ({elapsed:?})"
    );
}

/// IWP2 — stdout carries exactly one JSON document: the healthy report, never an earlier
/// unhealthy one (callers capture it with `$(...)` and grep `"is_healthy": true`).
#[test]
fn test_iwp_readiness_prints_only_the_final_status() {
    let fx = ready_fixture("final", 3);
    let (out, _) = run_ready(&fx, "10");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("{\"attempt\":3,\"is_healthy\":true}"),
        "the healthy report is printed; output:\n{}",
        combined(&out)
    );
    for earlier in ["\"attempt\":1,", "\"attempt\":2,"] {
        assert!(
            !stdout.contains(earlier),
            "an earlier unhealthy report ({earlier}) must not be printed; stdout:\n{stdout}"
        );
    }
    assert_eq!(
        stdout.matches("\"attempt\":").count(),
        1,
        "exactly one report on stdout:\n{stdout}"
    );
}

/// IWP2 — a daemon that never turns healthy: the helper keeps polling for the whole bound,
/// prints the last report once, then fails (exit 1) naming the bound and the journal.
#[test]
fn test_iwp_readiness_times_out_after_polling_with_last_status() {
    let fx = ready_fixture("never", 1_000_000);
    let (out, elapsed) = run_ready(&fx, "2");
    assert_eq!(
        out.status.code(),
        Some(1),
        "a daemon that never reports healthy is not ready; output:\n{}",
        combined(&out)
    );
    let n = attempts(&fx);
    assert!(
        n >= 3,
        "the status must be polled during the whole --timeout (only {n} attempt(s)); output:\n{}",
        combined(&out)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.matches(&format!("{{\"attempt\":{n},")).count(),
        1,
        "the last report is printed once; stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("\"attempt\":1,"),
        "earlier reports are not printed; stdout:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("did not report healthy within 2 s") && stderr.contains("journalctl"),
        "the failure names the bound and the journal; stderr:\n{stderr}"
    );
    assert!(
        elapsed >= Duration::from_secs(2) && elapsed < Duration::from_secs(15),
        "the poll lasts the bound and no longer ({elapsed:?})"
    );
}

/// IWP3 — a hanging `soos-admin status` is killed per attempt: the wait stays bounded and a
/// killed attempt's partial output is never printed.
#[test]
fn test_iwp_readiness_attempt_is_bounded() {
    if !have("timeout") {
        eprintln!("skipped: coreutils timeout is not on PATH");
        return;
    }
    let fx = ready_fixture("hang", 1_000_000);
    write_exec(&fx.admin, "#!/bin/sh\nprintf '{\"partial\":'\nsleep 30\n");
    let (out, elapsed) = run_ready(&fx, "1");
    assert_eq!(
        out.status.code(),
        Some(1),
        "a hanging status query is not readiness; output:\n{}",
        combined(&out)
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "--timeout 1 + one 5 s attempt bound must end the wait well before the hang ({elapsed:?})"
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("partial"),
        "a killed attempt contributes no stdout; output:\n{}",
        combined(&out)
    );
}

/// IWP4 — the installer keeps exit 70 for a daemon that is not ready, and the helper keeps
/// its documented exit codes.
#[test]
fn test_iwp_install_keeps_exit_70_on_readiness_failure() {
    let install = read_repo("scripts/install.sh");
    let wait = install
        .find("scripts/wait_daemon_ready.sh\" \\")
        .expect("install.sh invokes the readiness helper");
    let tail = &install[wait..];
    let not_ready = tail
        .find("the daemon is not ready")
        .expect("readiness failure message");
    assert!(
        tail[not_ready..].trim_start().contains("exit 70"),
        "a readiness failure must still exit 70"
    );
    let helper = read_repo("scripts/wait_daemon_ready.sh");
    assert!(
        helper.contains("Exit codes: 0 ready, 1 not ready, 2 usage error."),
        "the helper keeps its exit-code contract"
    );
}

// ---------------------------------------------------------------------------
// IWP5–IWP7 — build target directory preflight of scripts/install.sh --build
// ---------------------------------------------------------------------------

struct BuildRun {
    out: Output,
    calls: String,
    staged: Vec<String>,
}

fn entries(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Runs `install.sh --build --destdir <stage>` with `CARGO_TARGET_DIR=<target>`. With
/// `fake_root`, `id -u` reports 0 and `runuser`/`cargo` only log (root_shims); otherwise
/// only `cargo` and `runuser` are replaced by logging shims.
fn run_build(
    tag: &str,
    target: &Path,
    fake_root: bool,
    sudo_user: Option<&str>,
    dry_run: bool,
) -> BuildRun {
    let shims = scratch_dir(&format!("iwp_{tag}_shims"));
    let log = shims.join("calls.log");
    if fake_root {
        root_shims(&shims, &log);
    } else {
        for tool in ["cargo", "runuser"] {
            write_exec(
                &shims.join(tool),
                &format!(
                    "#!/bin/bash\necho \"{tool} $*\" >> \"{}\"\nexit 0\n",
                    log.display()
                ),
            );
        }
    }
    let stage = scratch_dir(&format!("iwp_{tag}_stage"));
    let path = format!(
        "{}:{}",
        shims.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = Command::new("bash");
    cmd.arg(workspace_root().join("scripts/install.sh"))
        .args([
            "--build",
            "--skip-models",
            "--skip-systemd",
            "--distro",
            "none",
        ])
        .arg("--destdir")
        .arg(&stage)
        .env("PATH", path)
        .env("CARGO_TARGET_DIR", target)
        .env_remove("SUDO_USER")
        .env_remove("DOAS_USER")
        .env_remove("PKEXEC_UID");
    if dry_run {
        cmd.arg("--dry-run");
    }
    if let Some(user) = sudo_user {
        cmd.env("SUDO_USER", user);
    }
    let out = cmd.output().expect("run install.sh --build");
    let calls = fs::read_to_string(&log).unwrap_or_default();
    let staged = entries(&stage);
    let _ = fs::remove_dir_all(&shims);
    let _ = fs::remove_dir_all(&stage);
    BuildRun { out, calls, staged }
}

fn uid_of(user: &str) -> Option<u32> {
    let out = Command::new("id").args(["-u", user]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn current_uid() -> u32 {
    fs::metadata("/proc/self").expect("proc self").uid()
}

fn current_user_name() -> String {
    let out = Command::new("id").arg("-un").output().expect("id -un");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A target directory left by an earlier build of another account (the runner, seen from
/// the build user `nobody`): `<target>/release/.cargo-build-lock` and a `deps` directory.
fn foreign_target(tag: &str) -> PathBuf {
    let target = scratch_dir(&format!("iwp_{tag}_target"));
    fs::create_dir_all(target.join("release/deps")).expect("release dir");
    fs::write(target.join("release/.cargo-build-lock"), b"").expect("lock file");
    target
}

fn nobody_is_a_foreign_user() -> bool {
    match uid_of("nobody") {
        Some(uid) => uid != current_uid(),
        None => false,
    }
}

/// IWP5 — `install.sh --build` as root for the invoking user `nobody`, over a target
/// directory whose files belong to another account: the preflight fails (exit 2, nothing
/// modified) with the exact `chown` fix, before `runuser` or `cargo` and before any change.
#[test]
fn test_iwp_build_refuses_foreign_owned_target_dir() {
    if !nobody_is_a_foreign_user() {
        eprintln!("skipped: no 'nobody' account distinct from the test runner");
        return;
    }
    let target = foreign_target("foreign");
    let run = run_build("foreign", &target, true, Some("nobody"), false);
    let text = combined(&run.out);
    let _ = fs::remove_dir_all(&target);
    assert_eq!(
        run.out.status.code(),
        Some(2),
        "a foreign-owned target directory is a preflight failure; output:\n{text}"
    );
    assert!(
        text.contains(&format!("sudo chown -R nobody: {}", target.display())),
        "the failure must print the exact fix 'sudo chown -R nobody: {}'; output:\n{text}",
        target.display()
    );
    assert!(
        text.contains("not owned by the build user 'nobody'"),
        "the failure must name the build user; output:\n{text}"
    );
    assert!(
        text.contains("nothing was modified"),
        "the refusal happens in the read-only preflight; output:\n{text}"
    );
    assert!(
        run.calls.is_empty(),
        "neither runuser nor cargo may run before the check (calls: {:?})",
        run.calls
    );
    assert!(
        run.staged.is_empty(),
        "nothing may be installed (staged: {:?})",
        run.staged
    );
}

/// IWP5 — `--dry-run --build` reports the same foreign-owned target directory.
#[test]
fn test_iwp_dry_run_build_reports_foreign_owned_target_dir() {
    if !nobody_is_a_foreign_user() {
        eprintln!("skipped: no 'nobody' account distinct from the test runner");
        return;
    }
    let target = foreign_target("dryforeign");
    let run = run_build("dryforeign", &target, true, Some("nobody"), true);
    let text = combined(&run.out);
    let _ = fs::remove_dir_all(&target);
    assert_eq!(
        run.out.status.code(),
        Some(2),
        "the dry run must fail its preflight; output:\n{text}"
    );
    assert!(
        text.contains(&format!("sudo chown -R nobody: {}", target.display())),
        "the dry run must print the exact fix; output:\n{text}"
    );
    assert!(
        !run.calls.contains("runuser") && !run.calls.contains("cargo "),
        "a dry run never builds (calls: {:?})",
        run.calls
    );
}

/// IWP6 — a target directory owned by the build user but holding a non-writable directory
/// is refused with `chmod -R u+w <target>` (non-root runner: root writes everything).
#[test]
fn test_iwp_build_refuses_non_writable_target_dir() {
    if current_uid() == 0 {
        eprintln!("skipped: root can write a 0555 directory");
        return;
    }
    let target = scratch_dir("iwp_ro_target");
    let release = target.join("release");
    fs::create_dir_all(&release).expect("release dir");
    fs::write(release.join("old-artifact"), b"x").expect("file");
    fs::set_permissions(&release, fs::Permissions::from_mode(0o555)).expect("chmod 0555");
    let run = run_build("ro", &target, false, None, false);
    let text = combined(&run.out);
    fs::set_permissions(&release, fs::Permissions::from_mode(0o755)).expect("chmod back");
    let _ = fs::remove_dir_all(&target);
    assert_eq!(
        run.out.status.code(),
        Some(2),
        "a non-writable target directory is a preflight failure; output:\n{text}"
    );
    assert!(
        text.contains(&format!("chmod -R u+w {}", target.display())),
        "the failure must print the exact fix 'chmod -R u+w {}'; output:\n{text}",
        target.display()
    );
    assert!(
        !run.calls.contains("cargo "),
        "cargo must not run (calls: {:?})",
        run.calls
    );
    assert!(
        run.staged.is_empty(),
        "nothing may be installed (staged: {:?})",
        run.staged
    );
}

/// IWP7 — no false positive: a target directory owned by the build user, or an absent one,
/// lets the build run as that user.
#[test]
fn test_iwp_build_proceeds_with_an_owned_target_dir() {
    if current_uid() == 0 {
        eprintln!("skipped: root is never the invoking user");
        return;
    }
    let me = current_user_name();

    let target = scratch_dir("iwp_owned_target");
    fs::create_dir_all(target.join("release/deps")).expect("release dir");
    fs::write(target.join("release/.cargo-build-lock"), b"").expect("lock file");
    let run = run_build("owned", &target, true, Some(&me), false);
    let _ = fs::remove_dir_all(&target);
    assert!(
        run.calls.contains(&format!("runuser -u {me} --")),
        "an owned target directory must not block the build (calls: {:?}):\n{}",
        run.calls,
        combined(&run.out)
    );
    assert!(
        !combined(&run.out).contains("sudo chown -R"),
        "no ownership hint for an owned target directory:\n{}",
        combined(&run.out)
    );

    let absent = scratch_dir("iwp_absent_parent").join("target");
    let run = run_build("absent", &absent, true, Some(&me), false);
    let _ = fs::remove_dir_all(absent.parent().expect("parent"));
    assert!(
        run.calls.contains(&format!("runuser -u {me} --")),
        "an absent target directory must not block the build (calls: {:?}):\n{}",
        run.calls,
        combined(&run.out)
    );
}

// ---------------------------------------------------------------------------
// IWP11 — the presence wake settle never reaches the PAM path
// ---------------------------------------------------------------------------

/// IWP11 — the settle is a presence-only constant used by the worker; the dispatcher keeps
/// the plain consensus and the daemon keeps the instant-wake `warmup_frames` default.
#[test]
fn test_iwp_settle_is_presence_only() {
    let presence = read_repo("crates/daemon/src/presence/mod.rs");
    assert!(
        presence.contains("pub const PRESENCE_WAKE_SETTLE_MS: u64 = 1000;"),
        "presence/mod.rs must define PRESENCE_WAKE_SETTLE_MS = 1000 (single source)"
    );
    let worker = read_repo("crates/daemon/src/presence/worker.rs");
    assert!(
        worker.contains("with_not_before(") && worker.contains("PRESENCE_WAKE_SETTLE_MS"),
        "the presence worker must bound its consensus window after a wake"
    );
    let dispatcher = read_repo("crates/daemon/src/dispatcher.rs");
    assert!(
        dispatcher.contains("run_face_consensus("),
        "the PAM path keeps the shared consensus"
    );
    for forbidden in ["with_not_before", "PRESENCE_WAKE_SETTLE"] {
        assert!(
            !dispatcher.contains(forbidden),
            "the PAM path must never apply the presence settle ({forbidden})"
        );
    }
    let consensus = read_repo("crates/daemon/src/consensus.rs");
    assert!(
        consensus.contains("not_before_ns()"),
        "the shared consensus reads the window's lower bound"
    );
    assert!(
        !consensus.contains("PRESENCE_WAKE_SETTLE"),
        "the shared consensus takes the bound from the deadline, never the presence constant"
    );
    let config = read_repo("crates/daemon/src/config.rs");
    assert!(
        config.contains("pub const DAEMON_DEFAULT_WARMUP_FRAMES: usize = 0;"),
        "the PAM instant-wake contract (warmup_frames = 0) is unchanged"
    );
}

// ---------------------------------------------------------------------------
// IWP13 — documentation and ADR
// ---------------------------------------------------------------------------

/// IWP13 — the operator documentation and the ADR register describe the three changes.
#[test]
fn test_iwp_docs_and_adr_describe_the_changes() {
    let packaging = read_repo("Docs/PACKAGING_AND_PROVISIONING.md");
    for needle in ["is_healthy", "sudo chown -R"] {
        assert!(
            packaging.contains(needle),
            "Docs/PACKAGING_AND_PROVISIONING.md must document `{needle}` (health poll, build directory fix)"
        );
    }
    let daemon = read_repo("Docs/DAEMON.md");
    assert!(
        daemon.contains("PRESENCE_WAKE_SETTLE_MS"),
        "Docs/DAEMON.md must document the presence wake settle"
    );
    let decisions = read_repo("AI/DECISIONS.md");
    assert!(
        decisions.contains("Presence Wake Settle") && decisions.contains("GitHub #329"),
        "AI/DECISIONS.md must record the ADR of GitHub #329"
    );
}
