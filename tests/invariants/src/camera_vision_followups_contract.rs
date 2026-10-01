//! Camera / vision follow-ups of the P3 batch (GitHub #287, matrix rows CVF1-CVF8).
//!
//! - CVF4: every `v4l` metadata call that can panic on malformed driver data
//!   (`query_caps`, `enum_formats`, `set_format`) goes through the panic guard
//!   `crates/camera-v4l/src/v4l_guard.rs`.
//! - CVF5: the capture supervisor builds its hints with the single `supervisor_sensor_hints`.
//! - CVF6: `BiometricEmbedding::to_vec` returns a `Zeroizing` copy.
//! - CVF7-CVF8: `scripts/install.sh` runs `soos-admin camera list` through the informational
//!   helper `scripts/camera_report.sh`, after the install is committed, and the helper never
//!   fails and is bounded.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

const GUARD_RS: &str = "crates/camera-v4l/src/v4l_guard.rs";

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

/// Production part of a Rust source (before the first `#[cfg(test)]`), without `//` comments.
fn production_code(source: &str) -> String {
    let production = source
        .find("#[cfg(test)]")
        .map_or(source, |pos| &source[..pos]);
    production
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// CVF4: the panicking `v4l` 0.14 conversions are reached only through the guard module.
#[test]
fn test_cvf_v4l_metadata_calls_only_through_panic_guard() {
    const RAW_CALLS: [&str; 4] = [
        "query_caps(",
        "enum_formats(",
        "set_format(",
        "Capabilities::from(",
    ];
    let dir = workspace_root().join("crates/camera-v4l/src");
    let mut scanned = 0usize;
    let mut violations = Vec::new();
    for entry in fs::read_dir(&dir).expect("read crates/camera-v4l/src") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let rel = format!(
            "crates/camera-v4l/src/{}",
            path.file_name().unwrap().to_string_lossy()
        );
        if rel == GUARD_RS {
            continue;
        }
        scanned += 1;
        let code = production_code(&read(&rel));
        for call in RAW_CALLS {
            if code.contains(call) {
                violations.push(format!("{rel} calls `{call}` outside {GUARD_RS}"));
            }
        }
    }
    assert!(
        scanned >= 10,
        "the source scan is vacuous ({scanned} files)"
    );
    assert!(
        violations.is_empty(),
        "v4l 0.14 panics on non-UTF-8 driver strings; use the guarded wrappers:\n{}",
        violations.join("\n")
    );

    let guard = production_code(&read(GUARD_RS));
    assert!(
        guard.contains("catch_unwind"),
        "{GUARD_RS} must catch the panic"
    );
    for wrapper in [
        "pub(crate) fn query_caps_guarded",
        "pub(crate) fn enum_formats_guarded",
        "pub(crate) fn set_format_guarded",
    ] {
        assert!(
            guard.contains(wrapper),
            "{GUARD_RS} must define `{wrapper}`"
        );
    }
    for (rel, needle) in [
        ("crates/camera-v4l/src/sensor.rs", "query_caps_guarded"),
        ("crates/camera-v4l/src/sensor.rs", "enum_formats_guarded"),
        ("crates/camera-v4l/src/diagnostics.rs", "query_caps_guarded"),
        ("crates/camera-v4l/src/v4l_impl.rs", "query_caps_guarded"),
        ("crates/camera-v4l/src/v4l_impl.rs", "enum_formats_guarded"),
        ("crates/camera-v4l/src/v4l_impl.rs", "set_format_guarded"),
    ] {
        assert!(
            production_code(&read(rel)).contains(needle),
            "{rel} must use `{needle}`"
        );
    }
}

/// CVF5: the capture open path classifies with the single supervisor hint builder, and the
/// decision to keep the by-id IR token on shared stems is recorded as an ADR.
#[test]
fn test_cvf_supervisor_hints_single_source_and_adr() {
    let v4l_impl = production_code(&read("crates/camera-v4l/src/v4l_impl.rs"));
    assert!(
        v4l_impl.contains("pub fn supervisor_sensor_hints("),
        "v4l_impl.rs must define supervisor_sensor_hints"
    );
    assert!(
        v4l_impl.contains("let hints = supervisor_sensor_hints("),
        "open_and_stream must build its hints with supervisor_sensor_hints"
    );
    assert!(
        !v4l_impl.contains("let hints = SensorHints {"),
        "no second, inline hint construction in the open path"
    );
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("Capture Supervisor Keeps the By-Id IR Token on Shared Stems"),
        "AI/DECISIONS.md must record the supervisor classification decision"
    );
}

/// CVF6: the embedding accessor returns a wipe-on-drop copy.
#[test]
fn test_cvf_embedding_to_vec_returns_zeroizing_copy() {
    let embedding = production_code(&read("crates/inference-ort/src/embedding.rs"));
    assert!(
        embedding.contains("pub fn to_vec(&self) -> Zeroizing<Vec<f32>>"),
        "BiometricEmbedding::to_vec must return Zeroizing<Vec<f32>>"
    );
    assert!(
        !embedding.contains("pub fn to_vec(&self) -> Vec<f32>"),
        "no raw-Vec embedding copy accessor"
    );
}

/// CVF7: `install.sh` previews the camera selection only on a live install, after the
/// install is committed (a failure cannot trigger the rollback), and never fatally.
#[test]
fn test_cvf_install_reports_camera_selection_non_fatally() {
    let install = read("scripts/install.sh");
    let committed = install
        .find("\nCOMMITTED=true")
        .expect("install.sh commits the install");
    let call = install
        .find("scripts/camera_report.sh\" \\")
        .expect("install.sh must invoke scripts/camera_report.sh");
    assert!(
        committed < call,
        "the camera report runs after COMMITTED=true"
    );
    let summary = install
        .find("soos installation and provisioning completed successfully")
        .expect("summary");
    assert!(call < summary, "the report runs before the final summary");
    let block_start = install[..call].rfind("\nif ").expect("guarding if");
    let block = &install[block_start..];
    let block = &block[..block.find("\nfi\n").expect("end of block")];
    assert!(
        block.contains("\"${LIVE_INSTALL}\" = true"),
        "only a live install probes the camera (never a DESTDIR staging build): {block}"
    );
    assert!(
        block.contains("--admin \"${TARGET_BIN_DIR}/soos-admin\"")
            && block.contains("--config \"${SYSCONFDIR}/soos/daemon.toml\""),
        "the installed soos-admin reads the installed daemon.toml: {block}"
    );
    assert!(
        block.contains("|| warn "),
        "a failing report is a warning, never fatal under set -e: {block}"
    );
}

struct ReportFixture {
    work: PathBuf,
    admin: PathBuf,
    admin_log: PathBuf,
    config: PathBuf,
}

fn report_fixture(tag: &str, body: &str) -> ReportFixture {
    let work = std::env::temp_dir().join(format!("soos_cvf_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).expect("scratch dir");
    let admin_log = work.join("admin.log");
    let admin = work.join("soos-admin");
    fs::write(
        &admin,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\n{body}\n",
            admin_log.display()
        ),
    )
    .expect("fake admin");
    fs::set_permissions(&admin, fs::Permissions::from_mode(0o755)).expect("chmod");
    ReportFixture {
        config: work.join("daemon.toml"),
        work,
        admin,
        admin_log,
    }
}

fn run_report(fx: &ReportFixture, timeout: &str) -> (Output, Duration) {
    let start = Instant::now();
    let out = Command::new("bash")
        .arg(workspace_root().join("scripts/camera_report.sh"))
        .args([
            OsStr::new("--admin"),
            fx.admin.as_os_str(),
            OsStr::new("--config"),
            fx.config.as_os_str(),
            OsStr::new("--timeout"),
            OsStr::new(timeout),
        ])
        .output()
        .expect("run camera_report.sh");
    (out, start.elapsed())
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// CVF8: the helper runs `soos-admin camera list --config <cfg>`, prints its output, and
/// exits 0 whatever happens: a failing, missing or hanging `soos-admin` is reported only.
#[test]
fn test_cvf_camera_report_helper_is_informational_and_bounded() {
    // Nominal: the report is printed and the arguments are the documented ones.
    let fx = report_fixture("ok", "echo 'selected:     /dev/video0'\nexit 0");
    let (out, _) = run_report(&fx, "5");
    assert!(out.status.success(), "output:\n{}", combined(&out));
    assert!(combined(&out).contains("selected:     /dev/video0"));
    let args = fs::read_to_string(&fx.admin_log).expect("admin invoked");
    assert_eq!(
        args.trim(),
        format!("camera list --config {}", fx.config.display())
    );
    let _ = fs::remove_dir_all(&fx.work);

    // No camera would be selected (exit status 1): a warning, exit 0.
    let fx = report_fixture("none", "echo 'selected:     none'\nexit 1");
    let (out, _) = run_report(&fx, "5");
    assert!(out.status.success(), "output:\n{}", combined(&out));
    assert!(combined(&out).contains("[WARN]"), "{}", combined(&out));
    let _ = fs::remove_dir_all(&fx.work);

    // Missing binary: a warning, exit 0.
    let fx = report_fixture("missing", "exit 0");
    fs::remove_file(&fx.admin).expect("rm admin");
    let (out, _) = run_report(&fx, "5");
    assert!(out.status.success(), "output:\n{}", combined(&out));
    assert!(combined(&out).contains("[WARN]"), "{}", combined(&out));
    let _ = fs::remove_dir_all(&fx.work);

    // Hanging soos-admin: stopped by the bound, exit 0.
    if Command::new("timeout").arg("--version").output().is_ok() {
        let fx = report_fixture("hang", "exec sleep 30");
        let (out, elapsed) = run_report(&fx, "1");
        assert!(out.status.success(), "output:\n{}", combined(&out));
        assert!(
            elapsed < Duration::from_secs(10),
            "the report is bounded ({elapsed:?})"
        );
        assert!(combined(&out).contains("[WARN]"), "{}", combined(&out));
        let _ = fs::remove_dir_all(&fx.work);
    }
}
