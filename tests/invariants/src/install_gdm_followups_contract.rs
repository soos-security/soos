//! GitHub #331 — install follow-ups, physical procedure fixes, GDM shared soos rule
//! (spec `AI/architect_spec_install_gdm_followups.md`, rows IGF1–IGF5, IGF9, IGF10, IGF12,
//! IGF13, IGF19).
//!
//! - `scripts/install.sh --build` also refuses read-only files and an unreadable subtree of
//!   the cargo target directory, with `chmod` advice (never the wrong `chown` advice).
//! - `scripts/wait_daemon_ready.sh` documents its real worst case, reports the last
//!   `soos-admin status` stderr on timeout and fails fast when the binary cannot run.
//! - A relative `CARGO_TARGET_DIR` is resolved once against the checkout.
//! - The presence settle keyed on the stream start never reaches the PAM path.
//! - The physical procedure and the documentation describe the changes.

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

use crate::installer_contract::{scratch_dir, stage_fixture_artifacts};
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

fn current_uid() -> u32 {
    fs::metadata("/proc/self").expect("proc self").uid()
}

fn current_user_name() -> String {
    let out = Command::new("id").arg("-un").output().expect("id -un");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn uid_of(user: &str) -> Option<u32> {
    let out = Command::new("id").args(["-u", user]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn nobody_is_a_foreign_user() -> bool {
    match uid_of("nobody") {
        Some(uid) => uid != current_uid(),
        None => false,
    }
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

/// The text of a Markdown section: from the heading starting with `start` to the next
/// heading starting with `next` (or the end of the document).
fn section<'a>(doc: &'a str, start: &str, next: &str) -> &'a str {
    let from = doc
        .find(start)
        .unwrap_or_else(|| panic!("section '{start}' not found"));
    let rest = &doc[from..];
    let to = rest[start.len()..]
        .find(next)
        .map_or(rest.len(), |i| i + start.len());
    &rest[..to]
}

// ---------------------------------------------------------------------------
// IGF1, IGF2 — build target directory preflight: read-only files, unreadable subtree
// ---------------------------------------------------------------------------

struct BuildRun {
    out: Output,
    calls: String,
    staged: Vec<String>,
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
    let shims = scratch_dir(&format!("igf_{tag}_shims"));
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
    let stage = scratch_dir(&format!("igf_{tag}_stage"));
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

/// A target directory owned by the runner, every directory writable, holding one 0444 file.
fn read_only_file_target(tag: &str) -> (PathBuf, PathBuf) {
    let target = scratch_dir(&format!("igf_{tag}_target"));
    let release = target.join("release");
    fs::create_dir_all(release.join("deps")).expect("release dir");
    let file = release.join("old-artifact");
    fs::write(&file, b"x").expect("file");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).expect("chmod 0444");
    (target, file)
}

fn assert_read_only_file_refused(run: &BuildRun, target: &Path, file: &Path, me: &str) {
    let text = combined(&run.out);
    assert_eq!(
        run.out.status.code(),
        Some(2),
        "a read-only file in the target directory is a preflight failure; output:\n{text}"
    );
    assert!(
        text.contains(&format!(
            "The cargo target directory {} holds a file the build user '{me}' cannot write (first: {}).",
            target.display(),
            file.display()
        )),
        "the failure must name the read-only file; output:\n{text}"
    );
    assert!(
        text.contains(&format!("Fix it with: chmod -R u+w {}", target.display())),
        "the failure must print the exact fix 'chmod -R u+w {}'; output:\n{text}",
        target.display()
    );
    assert!(
        !run.calls.contains("cargo ") && !run.calls.contains("runuser"),
        "neither cargo nor runuser may run (calls: {:?})",
        run.calls
    );
    assert!(
        run.staged.is_empty(),
        "nothing may be installed (staged: {:?})",
        run.staged
    );
}

/// IGF1 — a target directory owned by the build user with writable directories but one
/// read-only file is refused before the build, with `chmod -R u+w <target>`.
#[test]
fn test_igf_build_refuses_read_only_file_in_target_dir() {
    if current_uid() == 0 {
        eprintln!("skipped: root can write a 0444 file");
        return;
    }
    let me = current_user_name();
    let (target, file) = read_only_file_target("rofile");
    let run = run_build("rofile", &target, false, None, false);
    let _ = fs::remove_dir_all(&target);
    assert_read_only_file_refused(&run, &target, &file, &me);
}

/// IGF1 — `--dry-run --build` reports the same read-only file.
#[test]
fn test_igf_dry_run_build_reports_read_only_file_in_target_dir() {
    if current_uid() == 0 {
        eprintln!("skipped: root can write a 0444 file");
        return;
    }
    let me = current_user_name();
    let (target, file) = read_only_file_target("dryrofile");
    let run = run_build("dryrofile", &target, false, None, true);
    let _ = fs::remove_dir_all(&target);
    assert_read_only_file_refused(&run, &target, &file, &me);
}

fn assert_unreadable_dir_refused(run: &BuildRun, target: &Path, dir: &Path, me: &str) {
    let text = combined(&run.out);
    assert_eq!(
        run.out.status.code(),
        Some(2),
        "an unreadable directory in the target directory is a preflight failure; output:\n{text}"
    );
    assert!(
        text.contains(&format!(
            "The cargo target directory {} holds an entry the build user '{me}' cannot read (first: {}).",
            target.display(),
            dir.display()
        )),
        "the failure must name the unreadable entry; output:\n{text}"
    );
    assert!(
        text.contains(&format!("Fix it with: chmod -R u+rwX {}", target.display())),
        "the failure must print the exact fix 'chmod -R u+rwX {}'; output:\n{text}",
        target.display()
    );
    assert!(
        !text.contains("sudo chown -R"),
        "a user-owned unreadable directory must not get the 'sudo chown -R' advice; output:\n{text}"
    );
    assert!(
        !run.calls.contains("cargo ") && !run.calls.contains("runuser"),
        "neither cargo nor runuser may run (calls: {:?})",
        run.calls
    );
    assert!(
        run.staged.is_empty(),
        "nothing may be installed (staged: {:?})",
        run.staged
    );
}

fn run_unreadable(tag: &str, mode: u32, fake_root: bool) {
    let me = current_user_name();
    let target = scratch_dir(&format!("igf_{tag}_target"));
    let deps = target.join("release/deps");
    fs::create_dir_all(&deps).expect("deps dir");
    fs::write(deps.join("libold.rlib"), b"x").expect("file");
    fs::set_permissions(&deps, fs::Permissions::from_mode(mode)).expect("chmod deps");
    let sudo_user = fake_root.then_some(me.as_str());
    let run = run_build(tag, &target, fake_root, sudo_user, false);
    fs::set_permissions(&deps, fs::Permissions::from_mode(0o755)).expect("chmod back");
    let _ = fs::remove_dir_all(&target);
    assert_unreadable_dir_refused(&run, &target, &deps, &me);
}

/// IGF2 — a user-owned 0000 directory, non-root runner: `chmod -R u+rwX`, never `chown`.
#[test]
fn test_igf_build_refuses_unreadable_dir_mode_0000_with_chmod_advice() {
    if current_uid() == 0 {
        eprintln!("skipped: root reads a 0000 directory");
        return;
    }
    run_unreadable("unread0000", 0o000, false);
}

/// IGF2 — a user-owned 0300 directory (writable, not readable), non-root runner.
#[test]
fn test_igf_build_refuses_unreadable_dir_mode_0300_with_chmod_advice() {
    if current_uid() == 0 {
        eprintln!("skipped: root reads a 0300 directory");
        return;
    }
    run_unreadable("unread0300", 0o300, false);
}

/// IGF2 — the same 0300 directory under fake root (`id -u` = 0, build user = the runner):
/// the same `chmod -R u+rwX` advice.
#[test]
fn test_igf_fake_root_build_refuses_unreadable_dir_with_chmod_advice() {
    if current_uid() == 0 {
        eprintln!("skipped: needs a real non-root uid behind the fake root");
        return;
    }
    run_unreadable("unreadroot", 0o300, true);
}

// ---------------------------------------------------------------------------
// IGF3–IGF5 — scripts/wait_daemon_ready.sh
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

/// A deployed manifest, a bound socket and a fake `soos-admin` with the given body.
fn ready_fixture(tag: &str) -> Ready {
    let work = scratch_dir(&format!("igf_ready_{tag}"));
    let manifest = work.join("manifest.toml");
    fs::write(&manifest, "[manifest]\nversion = \"2.0.0\"\n").expect("manifest");
    let socket = work.join("daemon.sock");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let counter = work.join("attempts");
    let admin = work.join("soos-admin");
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

fn run_ready_with_admin(fx: &Ready, admin: &Path, timeout: &str) -> (Output, Duration) {
    let start = Instant::now();
    let out = Command::new("bash")
        .arg(workspace_root().join("scripts/wait_daemon_ready.sh"))
        .args([
            OsStr::new("--socket"),
            fx.socket.as_os_str(),
            OsStr::new("--manifest"),
            fx.manifest.as_os_str(),
            OsStr::new("--admin"),
            admin.as_os_str(),
            OsStr::new("--timeout"),
            OsStr::new(timeout),
        ])
        .output()
        .expect("run wait_daemon_ready.sh");
    (out, start.elapsed())
}

/// A fake admin that counts attempts, prints an unhealthy report on stdout, runs
/// `stderr_cmd` (shell, with `$n` = attempt number) and exits 1.
fn failing_admin(fx: &Ready, stderr_cmd: &str) {
    write_exec(
        &fx.admin,
        &format!(
            "#!/bin/sh\n\
             n=$(cat '{c}' 2>/dev/null || echo 0)\n\
             n=$((n + 1))\n\
             echo \"$n\" > '{c}'\n\
             printf '{{\"attempt\":%s,\"is_healthy\":false}}\\n' \"$n\"\n\
             {stderr_cmd}\n\
             exit 1\n",
            c = fx.counter.display()
        ),
    );
}

/// IGF3 — the header, `--help`, README and packaging doc state the real worst case
/// (`--timeout` + 7.5 s; 37.5 s with the default), and README no longer says "at most 30 s".
#[test]
fn test_igf_readiness_documents_real_worst_case_bound() {
    let helper = read_repo("scripts/wait_daemon_ready.sh");
    let header_end = helper.find("set -euo pipefail").expect("strict mode line");
    let header = &helper[..header_end];
    assert!(
        header.contains("+ 7.5 s") && header.contains("start of the last status attempt"),
        "the script header must state that --timeout bounds the start of the last status attempt and the worst case is about --timeout + 7.5 s"
    );
    let help = Command::new("bash")
        .arg(workspace_root().join("scripts/wait_daemon_ready.sh"))
        .arg("--help")
        .output()
        .expect("run --help");
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(
        help.status.success() && help_text.contains("+ 7.5 s") && help_text.contains("kill-after"),
        "usage() must state the worst case (--timeout + 7.5 s) and the unbounded attempt without timeout --kill-after:\n{help_text}"
    );
    let readme = read_repo("README.md");
    assert!(
        !readme.contains("waits at most 30 s"),
        "README.md must no longer claim the helper waits at most 30 s"
    );
    assert!(
        readme.contains("worst case about 37.5 s"),
        "README.md must state the worst case (about 37.5 s with the default --timeout)"
    );
    let packaging = read_repo("Docs/PACKAGING_AND_PROVISIONING.md");
    assert!(
        packaging.contains("7.5 s") && packaging.contains("37.5 s"),
        "Docs/PACKAGING_AND_PROVISIONING.md must state the worst case (--timeout + 7.5 s, 37.5 s by default)"
    );
}

/// IGF3 — behavioural guard of the documented bound: `--timeout 1` with a hanging admin
/// ends within 1 + 7.5 s plus a 2 s scheduling margin.
#[test]
fn test_igf_readiness_hanging_admin_respects_documented_bound() {
    if !have("timeout") {
        eprintln!("skipped: coreutils timeout is not on PATH");
        return;
    }
    let fx = ready_fixture("hang");
    write_exec(&fx.admin, "#!/bin/sh\nsleep 30\n");
    let (out, elapsed) = run_ready_with_admin(&fx, &fx.admin, "1");
    assert_eq!(
        out.status.code(),
        Some(1),
        "a hanging admin is not readiness; output:\n{}",
        combined(&out)
    );
    assert!(
        elapsed < Duration::from_millis(10_500),
        "--timeout 1 must end within the documented worst case (1 + 7.5 s, +2 s margin): {elapsed:?}"
    );
}

/// IGF4 — on timeout, the error reports the last attempt's stderr (and only the last).
#[test]
fn test_igf_readiness_timeout_reports_last_status_stderr() {
    let fx = ready_fixture("stderr");
    failing_admin(&fx, "printf 'boom-%s-end\\n' \"$n\" >&2");
    let (out, _) = run_ready_with_admin(&fx, &fx.admin, "2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "output:\n{}", combined(&out));
    let n = attempts(&fx);
    assert!(n >= 2, "the status must be polled more than once ({n})");
    let failed = stderr
        .find("did not report healthy within 2 s")
        .unwrap_or_else(|| panic!("timeout line kept verbatim; stderr:\n{stderr}"));
    assert!(
        stderr.contains(
            "        Inspect: soos-admin status; journalctl -u soos-daemon.service -n 50"
        ),
        "the Inspect line is kept verbatim; stderr:\n{stderr}"
    );
    let last = stderr
        .find(&format!(
            "        Last '{} status' error:",
            fx.admin.display()
        ))
        .unwrap_or_else(|| {
            panic!("the timeout error must report the last status stderr; stderr:\n{stderr}")
        });
    assert!(
        last > failed,
        "the last error follows the timeout line; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains(&format!("        boom-{n}-end")),
        "the last attempt's stderr (boom-{n}-end) is printed, indented by 8 spaces; stderr:\n{stderr}"
    );
    for earlier in 1..n {
        assert!(
            !stderr.contains(&format!("boom-{earlier}-end")),
            "only the last attempt's stderr is kept (found boom-{earlier}-end); stderr:\n{stderr}"
        );
    }
    assert_eq!(
        stdout.matches("\"attempt\":").count(),
        1,
        "stdout keeps exactly one (the last) report:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("\"attempt\":{n},")),
        "stdout keeps the last report:\n{stdout}"
    );
}

/// IGF4 — control characters of the stderr tail are printed as `?`.
#[test]
fn test_igf_readiness_stderr_tail_replaces_control_characters() {
    let fx = ready_fixture("ctrl");
    failing_admin(&fx, "printf 'esc\\033[31mred\\007bell\\n' >&2");
    let (out, _) = run_ready_with_admin(&fx, &fx.admin, "1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "output:\n{}", combined(&out));
    assert!(
        stderr.contains("esc?[31mred?bell"),
        "control characters must be replaced by '?'; stderr:\n{stderr:?}"
    );
    assert!(
        !stderr.contains('\u{1b}') && !stderr.contains('\u{7}'),
        "no raw control character may reach the terminal; stderr:\n{stderr:?}"
    );
}

/// IGF4 — the stderr tail keeps at most the last 1024 characters.
#[test]
fn test_igf_readiness_stderr_tail_is_bounded() {
    let fx = ready_fixture("tail");
    failing_admin(
        &fx,
        "{ printf 'START'; head -c 5000 /dev/zero | tr '\\0' x; printf 'END\\n'; } >&2",
    );
    let (out, _) = run_ready_with_admin(&fx, &fx.admin, "1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "output:\n{}", combined(&out));
    assert!(
        stderr.contains("xEND"),
        "the tail keeps the end of the last stderr; stderr length {}",
        stderr.len()
    );
    assert!(
        !stderr.contains("START"),
        "the tail drops the beginning of a long stderr"
    );
    let xs = stderr.matches('x').count();
    assert!(
        (900..=1024).contains(&xs),
        "the tail keeps at most the last 1024 characters ({xs} filler characters printed)"
    );
}

fn assert_fails_fast(fx: &Ready, admin: &Path, code: u32, reason: &str) {
    let (out, elapsed) = run_ready_with_admin(fx, admin, "30");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "an admin that cannot run exits 1 (install.sh keeps exit 70); output:\n{}",
        combined(&out)
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "exit {code} must fail at once, not after --timeout 30 ({elapsed:?})"
    );
    assert!(
        stderr.contains(&format!(
            "[ERROR] Cannot run '{}' (exit {code}: {reason}); readiness cannot be checked.",
            admin.display()
        )),
        "the failure must name the exit code; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("        Pass --admin <PATH> with the installed soos-admin binary."),
        "the failure must tell how to fix it; stderr:\n{stderr}"
    );
    assert!(
        out.stdout.is_empty(),
        "no report on stdout; stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// IGF5 — `--admin` pointing to a non-executable file (exit 126) fails at once.
#[test]
fn test_igf_readiness_fails_fast_on_non_executable_admin() {
    let fx = ready_fixture("noexec");
    fs::write(&fx.admin, "#!/bin/sh\nexit 0\n").expect("write admin");
    fs::set_permissions(&fx.admin, fs::Permissions::from_mode(0o644)).expect("chmod 0644");
    assert_fails_fast(&fx, &fx.admin, 126, "found but not executable");
}

/// IGF5 — `--admin` pointing to a missing path (exit 127) fails at once.
#[test]
fn test_igf_readiness_fails_fast_on_missing_admin() {
    let fx = ready_fixture("missing");
    let missing = fx.work.join("no-such-soos-admin");
    assert_fails_fast(&fx, &missing, 127, "not found");
}

// ---------------------------------------------------------------------------
// IGF9 — the stream-start settle is presence-only
// ---------------------------------------------------------------------------

/// IGF9 — only the presence worker reads `stream_started_mono_ns`; the dispatcher never
/// applies the settle; consensus and the warmup default are untouched.
#[test]
fn test_igf_stream_start_settle_is_presence_only() {
    let worker = read_repo("crates/daemon/src/presence/worker.rs");
    assert!(
        worker.contains("stream_started_mono_ns()") && worker.contains("presence_settle_window("),
        "the presence worker must key its settle on the camera stream start"
    );
    let dispatcher = read_repo("crates/daemon/src/dispatcher.rs");
    for forbidden in [
        "stream_started_mono_ns",
        "with_not_before",
        "PRESENCE_WAKE_SETTLE",
        "presence_settle_window",
    ] {
        assert!(
            !dispatcher.contains(forbidden),
            "the PAM path must never apply the presence settle ({forbidden})"
        );
    }
    let consensus = read_repo("crates/daemon/src/consensus.rs");
    for forbidden in ["stream_started_mono_ns", "PRESENCE_WAKE_SETTLE"] {
        assert!(
            !consensus.contains(forbidden),
            "the shared consensus takes its bound from the deadline only ({forbidden})"
        );
    }
    let config = read_repo("crates/daemon/src/config.rs");
    assert!(
        config.contains("pub const DAEMON_DEFAULT_WARMUP_FRAMES: usize = 0;"),
        "the PAM instant-wake contract (warmup_frames = 0) is unchanged"
    );
    for dir in ["crates/daemon/src", "crates/pam/src"] {
        let out = Command::new("grep")
            .args(["-rl", "stream_started_mono_ns"])
            .arg(workspace_root().join(dir))
            .output()
            .expect("grep");
        for file in String::from_utf8_lossy(&out.stdout).lines() {
            assert!(
                file.contains("/crates/daemon/src/presence/"),
                "only the presence worker may read the stream start: {file}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// IGF10 — relative CARGO_TARGET_DIR resolved against the checkout
// ---------------------------------------------------------------------------

/// A relative target directory under the checkout's (gitignored) `target/`, removed on drop.
struct RelTarget {
    rel: String,
    abs: PathBuf,
}

impl RelTarget {
    fn new(tag: &str) -> Self {
        let rel = format!("target/igf_rel_{tag}_{}", std::process::id());
        let abs = workspace_root().join(&rel);
        let _ = fs::remove_dir_all(&abs);
        Self { rel, abs }
    }
}

impl Drop for RelTarget {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.abs);
    }
}

fn install_from_elsewhere(
    tag: &str,
    rel: &str,
    args: &[&str],
    shims: Option<&Path>,
    env: &[(&str, &str)],
) -> Output {
    let cwd = scratch_dir(&format!("igf_{tag}_cwd"));
    let mut cmd = Command::new("bash");
    cmd.current_dir(&cwd)
        .arg(workspace_root().join("scripts/install.sh"))
        .args(args)
        .env("CARGO_TARGET_DIR", rel)
        .env_remove("SUDO_USER")
        .env_remove("DOAS_USER")
        .env_remove("PKEXEC_UID");
    if let Some(dir) = shims {
        cmd.env(
            "PATH",
            format!(
                "{}:{}",
                dir.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run install.sh");
    let _ = fs::remove_dir_all(&cwd);
    out
}

/// IGF10 — the default artifact directory of a relative `CARGO_TARGET_DIR` is
/// `<checkout>/<rel>/release`, whatever the working directory.
#[test]
fn test_igf_relative_cargo_target_dir_artifact_default_uses_checkout() {
    let target = RelTarget::new("artifacts");
    stage_fixture_artifacts(&target.abs.join("release"));
    let stage = scratch_dir("igf_relart_stage");
    let stage_s = stage.display().to_string();
    let out = install_from_elsewhere(
        "relart",
        &target.rel,
        &[
            "--dry-run",
            "--skip-models",
            "--skip-systemd",
            "--distro",
            "none",
            "--pam-dir",
            "/usr/lib/security",
            "--destdir",
            &stage_s,
        ],
        None,
        &[],
    );
    let _ = fs::remove_dir_all(&stage);
    let text = combined(&out);
    assert!(
        !text.contains("Artifact directory not found"),
        "a relative CARGO_TARGET_DIR must be resolved against the checkout, not the working directory; output:\n{text}"
    );
    assert!(
        out.status.success(),
        "the dry run must find the artifacts in {}/release; output:\n{text}",
        target.abs.display()
    );
}

/// IGF10 — `--build --artifact-dir` names the resolved build directory, and `--help`
/// documents the resolution.
#[test]
fn test_igf_relative_cargo_target_dir_named_resolved_in_messages() {
    let target = RelTarget::new("msg");
    let out = install_from_elsewhere(
        "relmsg",
        &target.rel,
        &["--build", "--artifact-dir", "/nonexistent"],
        None,
        &[],
    );
    let text = combined(&out);
    assert!(
        text.contains(&format!(
            "--build always produces {}/release",
            target.abs.display()
        )),
        "the error must print the resolved build directory; output:\n{text}"
    );
    let help = install_from_elsewhere("relhelp", &target.rel, &["--help"], None, &[]);
    assert!(
        String::from_utf8_lossy(&help.stdout).contains("resolved against the checkout"),
        "--help must say that a relative CARGO_TARGET_DIR is resolved against the checkout:\n{}",
        combined(&help)
    );
}

/// IGF10 — the build preflight inspects `<checkout>/<rel>`: files foreign to the build
/// user `nobody` there are refused even when the installer runs from another directory.
#[test]
fn test_igf_relative_cargo_target_dir_ownership_check_uses_checkout() {
    if !nobody_is_a_foreign_user() {
        eprintln!("skipped: no 'nobody' account distinct from the test runner");
        return;
    }
    let target = RelTarget::new("own");
    fs::create_dir_all(target.abs.join("release/deps")).expect("release dir");
    fs::write(target.abs.join("release/.cargo-build-lock"), b"").expect("lock file");
    let shims = scratch_dir("igf_relown_shims");
    let log = shims.join("calls.log");
    root_shims(&shims, &log);
    let stage = scratch_dir("igf_relown_stage");
    let stage_s = stage.display().to_string();
    let out = install_from_elsewhere(
        "relown",
        &target.rel,
        &[
            "--build",
            "--skip-models",
            "--skip-systemd",
            "--distro",
            "none",
            "--destdir",
            &stage_s,
        ],
        Some(&shims),
        &[("SUDO_USER", "nobody")],
    );
    let calls = fs::read_to_string(&log).unwrap_or_default();
    let _ = fs::remove_dir_all(&shims);
    let _ = fs::remove_dir_all(&stage);
    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(2),
        "the foreign-owned <checkout>/<rel> must fail the preflight; output:\n{text}"
    );
    assert!(
        text.contains(&format!("sudo chown -R nobody: {}", target.abs.display())),
        "the ownership check must inspect the resolved {}; output:\n{text}",
        target.abs.display()
    );
    assert!(
        !calls.contains("runuser") && !calls.contains("cargo "),
        "no build may run (calls: {calls:?})"
    );
}

/// IGF10 — the build itself receives the resolved absolute directory: `runuser` gets
/// `SOOS_CARGO_TARGET_DIR`, the non-root path `CARGO_TARGET_DIR`; AFC7 literals intact.
#[test]
fn test_igf_relative_cargo_target_dir_passed_to_the_build() {
    let install = read_repo("scripts/install.sh");
    assert!(
        install.contains(
            "CARGO_TARGET_DIR=\"${BUILD_TARGET_DIR}\" SOOS_BINDIR=\"${PREFIX}/bin\" \"${BUILD_CMD[@]}\""
        ),
        "the non-root build must receive CARGO_TARGET_DIR=\"${{BUILD_TARGET_DIR}}\""
    );
    assert!(
        install.contains("CARGO_TARGET_DIR=\"${SOOS_CARGO_TARGET_DIR}\" SOOS_BINDIR=\"$2\" cargo build --release --locked --workspace"),
        "the runuser build must re-assert CARGO_TARGET_DIR from SOOS_CARGO_TARGET_DIR"
    );

    let me = current_user_name();
    let abs_expected;
    // Root path (fake root, build user = the runner).
    {
        let target = RelTarget::new("runuser");
        abs_expected = target.abs.display().to_string();
        let shims = scratch_dir("igf_relrun_shims");
        let log = shims.join("calls.log");
        root_shims(&shims, &log);
        write_exec(
            &shims.join("runuser"),
            &format!(
                "#!/bin/bash\necho \"runuser $* SCTD=${{SOOS_CARGO_TARGET_DIR:-unset}}\" >> \"{}\"\nexit 0\n",
                log.display()
            ),
        );
        let stage = scratch_dir("igf_relrun_stage");
        let stage_s = stage.display().to_string();
        let out = install_from_elsewhere(
            "relrun",
            &target.rel,
            &[
                "--build",
                "--skip-models",
                "--skip-systemd",
                "--distro",
                "none",
                "--destdir",
                &stage_s,
            ],
            Some(&shims),
            &[("SUDO_USER", me.as_str())],
        );
        let calls = fs::read_to_string(&log).unwrap_or_default();
        let _ = fs::remove_dir_all(&shims);
        let _ = fs::remove_dir_all(&stage);
        if current_uid() != 0 {
            let line = calls
                .lines()
                .find(|l| l.starts_with(&format!("runuser -u {me} --")))
                .unwrap_or_else(|| {
                    panic!(
                        "install.sh must build through runuser: {calls:?}\n{}",
                        combined(&out)
                    )
                });
            assert!(
                line.ends_with(&format!(" SCTD={abs_expected}")),
                "runuser must receive SOOS_CARGO_TARGET_DIR={abs_expected}: {line}"
            );
            assert!(
                line.contains("SOOS_BINDIR=\"$2\" cargo build --release --locked --workspace"),
                "AFC7 literal kept: {line}"
            );
        }
    }
    // Non-root path: the cargo shim sees the resolved CARGO_TARGET_DIR.
    if current_uid() != 0 {
        let target = RelTarget::new("nonroot");
        let shims = scratch_dir("igf_relnr_shims");
        let log = shims.join("calls.log");
        write_exec(
            &shims.join("cargo"),
            &format!(
                "#!/bin/bash\necho \"cargo $* CTD=${{CARGO_TARGET_DIR:-unset}}\" >> \"{}\"\nexit 0\n",
                log.display()
            ),
        );
        let stage = scratch_dir("igf_relnr_stage");
        let stage_s = stage.display().to_string();
        let out = install_from_elsewhere(
            "relnr",
            &target.rel,
            &[
                "--build",
                "--skip-models",
                "--skip-systemd",
                "--distro",
                "none",
                "--destdir",
                &stage_s,
            ],
            Some(&shims),
            &[],
        );
        let calls = fs::read_to_string(&log).unwrap_or_default();
        let _ = fs::remove_dir_all(&shims);
        let _ = fs::remove_dir_all(&stage);
        if calls.is_empty() {
            eprintln!(
                "skipped non-root build check: build dependencies missing\n{}",
                combined(&out)
            );
            return;
        }
        let line = calls
            .lines()
            .find(|l| l.starts_with("cargo build"))
            .unwrap_or_else(|| panic!("cargo build must run: {calls:?}"));
        assert!(
            line.ends_with(&format!(" CTD={}", target.abs.display())),
            "the non-root build must receive CARGO_TARGET_DIR={}: {line}",
            target.abs.display()
        );
    }
}

// ---------------------------------------------------------------------------
// IGF12, IGF13 — physical procedure
// ---------------------------------------------------------------------------

/// IGF12 — the §6 rollback edits only the base stacks, following symlinks.
#[test]
fn test_igf_physical_rollback_edits_only_base_stacks() {
    let doc = read_repo("tests/physical/screensaver_test.md");
    let s6 = section(&doc, "## 6.", "\n## ");
    assert!(
        s6.contains("--follow-symlinks"),
        "the §6 sed must follow symlinks (authselect ships system-auth/password-auth as symlinks)"
    );
    for stack in ["common-auth", "system-auth", "password-auth"] {
        assert!(s6.contains(stack), "§6 must name the base stack {stack}");
    }
    for gone in [
        "/etc/pam.d/swaylock",
        "/etc/pam.d/hyprlock",
        "/etc/pam.d/sudo",
    ] {
        assert!(
            !s6.contains(gone),
            "§6 must no longer edit {gone}: it reaches soos only through the base stacks"
        );
    }
    let mut in_code = false;
    let mut seen_sed = false;
    for line in s6.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if !in_code {
            continue;
        }
        if line.contains("sed ") {
            seen_sed = true;
        }
        for (i, _) in line.match_indices("/etc/pam.d/") {
            let name: String = line[i + "/etc/pam.d/".len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            assert!(
                ["common-auth", "system-auth", "password-auth"].contains(&name.as_str()),
                "the §6 rollback commands may only name the base stacks (found /etc/pam.d/{name}): {line}"
            );
        }
    }
    assert!(seen_sed, "§6 keeps a sed rollback command");
}

/// IGF13 — §3.3 Test Case 2 tells how to find the transient GDM worker.
#[test]
fn test_igf_physical_gdm_worker_pgrep_hint() {
    let doc = read_repo("tests/physical/screensaver_test.md");
    let s33 = section(&doc, "### 3.3", "### 3.4");
    assert!(
        s33.contains("pgrep -af 'gdm-session-worker \\[pam/gdm-password\\]'"),
        "§3.3 must give the pgrep command for the transient gdm-session-worker"
    );
    assert!(
        s33.contains("/proc/<pid>/cgroup"),
        "§3.3 must tell to read /proc/<pid>/cgroup of the worker"
    );
}

// ---------------------------------------------------------------------------
// IGF19 — documentation and ADR
// ---------------------------------------------------------------------------

/// IGF19 — the ADR register, the architecture, the deployment guide, the physical
/// procedure and the shared project facts describe the shared GDM rule.
#[test]
fn test_igf_docs_and_adr_describe_the_changes() {
    let decisions = read_repo("AI/DECISIONS.md");
    assert!(
        decisions.contains("[2026-10-05] GDM Reuses a Shared Primary soos Rule"),
        "AI/DECISIONS.md must record the 2026-10-05 ADR 'GDM Reuses a Shared Primary soos Rule'"
    );
    let placement = decisions
        .lines()
        .find(|l| l.contains("[2026-09-30] GDM PAM Stack Placement"))
        .expect("ADR 2026-09-30 GDM PAM Stack Placement");
    assert!(
        placement.contains("Amended by ADR 2026-10-05 \"GDM Reuses a Shared Primary soos Rule"),
        "the 2026-09-30 GDM PAM Stack Placement entry must carry the amendment note"
    );

    let arch = read_repo("AI/ARCHITECTURE.md");
    assert!(
        arch.contains("GDM uses `timeout_ms=2500` only when"),
        "AI/ARCHITECTURE.md must limit timeout_ms=2500 to the managed block"
    );
    assert!(
        !arch.contains("GDM uses `timeout_ms=2500`."),
        "AI/ARCHITECTURE.md must drop the unconditional 'GDM uses `timeout_ms=2500`.'"
    );
    assert!(
        arch.contains("shared_stack"),
        "AI/ARCHITECTURE.md must describe the shared-rule case (shared_stack)"
    );

    let deploy = read_repo("Docs/DISTRIBUTION_DEPLOYMENT.md");
    assert!(
        deploy.contains("Shared soos Rule"),
        "Docs/DISTRIBUTION_DEPLOYMENT.md must document the shared-rule case (Shared soos Rule)"
    );
    for needle in [
        "timeout_ms=2500",
        "connection_timeout_ms",
        "2500 ms",
        "1000 ms",
    ] {
        assert!(
            deploy.contains(needle),
            "Docs/DISTRIBUTION_DEPLOYMENT.md must keep/document `{needle}`"
        );
    }
    assert!(
        !deploy.contains(
            "What `gdm enable` writes, e.g. on Fedora 40 (`authselect ... with-faillock`):"
        ),
        "the Fedora example must be relabelled as a stack without a soos rule"
    );

    let physical = read_repo("tests/physical/screensaver_test.md");
    let s6 = section(&physical, "## 6.", "\n## ");
    assert!(
        s6.contains("Shared soos Rule"),
        "screensaver §6 must cover the shared-rule GDM case (Shared soos Rule)"
    );
    assert!(
        s6.contains("may be no backup"),
        "screensaver §6 must say there may be no backup in the shared-rule case (plan evaluation R2-1)"
    );

    let facts = read_repo(".claude/skills/dev-workflow/references/project-facts.md");
    let gdm_row = facts
        .lines()
        .find(|l| l.starts_with("| `GDM_PAM_LINE`"))
        .expect("GDM_PAM_LINE row");
    assert!(
        gdm_row.contains("GitHub #331"),
        "project-facts GDM_PAM_LINE row must mention the shared-rule omission (GitHub #331)"
    );
    for constant in ["PRESENCE_WAKE_SETTLE_MS", "SYSTEMCTL_SHOW_TIMEOUT_MS"] {
        assert!(
            facts
                .lines()
                .any(|l| l.starts_with(&format!("| `{constant}`"))),
            "project-facts must list `{constant}`"
        );
    }

    let camera = read_repo("Docs/CAMERA_V4L_CRATE.md");
    assert!(
        camera.contains("stream_started_mono_ns"),
        "Docs/CAMERA_V4L_CRATE.md must document CameraManager::stream_started_mono_ns"
    );
}
