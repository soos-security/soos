//! `daemon.toml` open path, `v4l` hook filter re-installation, 32-bit clock conversions and the
//! decision record (GitHub #291, matrix rows CDF1, CDF3-CDF5).
//!
//! - CDF1: the shared reader opens with `O_PATH` first, checks that handle, and reopens a
//!   regular file through `/proc/self/fd`, re-checking the device and inode numbers.
//! - CDF3: `soos-daemon` re-installs the `v4l` hook filter right after its own panic hook.
//! - CDF4: no `timespec` field is converted with `cast_unsigned` (a `u32` on i686 / armv7).
//! - CDF5: the decisions are recorded as an ADR.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

const READER_RS: &str = "crates/camera-v4l/src/daemon_config.rs";
const DAEMON_MAIN_RS: &str = "crates/daemon/src/main.rs";

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

/// CDF1: `O_PATH` open, handle check, `/proc/self/fd` reopen with an identity re-check.
#[test]
fn test_cdf_reader_opens_with_o_path_then_reopens_through_proc() {
    let code = production_code(&read(READER_RS));
    for needle in [
        "libc::O_PATH | libc::O_CLOEXEC",
        "libc::O_NONBLOCK | libc::O_CLOEXEC",
        "/proc/self/fd/",
        ".dev()",
        ".ino()",
    ] {
        assert!(code.contains(needle), "{READER_RS} must contain `{needle}`");
    }
    let path_open = code.find("libc::O_PATH").expect("O_PATH open");
    let read_open = code.find("libc::O_NONBLOCK").expect("read open");
    assert!(
        path_open < read_open,
        "the O_PATH handle is opened and checked before any open for reading"
    );
}

/// CDF3: the daemon re-installs the hook filter right after installing its own panic hook.
#[test]
fn test_cdf_daemon_reinstalls_the_v4l_hook_filter_after_its_panic_hook() {
    let code = production_code(&read(DAEMON_MAIN_RS));
    let call = "soos_camera_v4l::v4l_guard::install_v4l_panic_hook_filter();";
    assert_eq!(
        code.matches(call).count(),
        1,
        "{DAEMON_MAIN_RS} must call `{call}` once"
    );
    let filter = code.find(call).unwrap();
    for hook in ["install_panic_hook();", "install_panic_hook_with(policy)"] {
        let installed = code.find(hook).unwrap_or_else(|| panic!("{hook} missing"));
        assert!(installed < filter, "the filter goes on top of `{hook}`");
    }
    let pipeline = code
        .find("initialize_pipeline(&")
        .expect("pipeline initialization");
    assert!(
        filter < pipeline,
        "the filter is in place before any camera call"
    );
}

/// CDF4: `timespec` fields are converted with `try_from`, never `cast_unsigned`.
#[test]
fn test_cdf_timespec_fields_are_not_cast_unsigned() {
    let mut offenders = Vec::new();
    let mut stack = vec![workspace_root().join("crates")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                let code = production_code(&fs::read_to_string(&path).unwrap());
                if code.contains("tv_sec.cast_unsigned()")
                    || code.contains("tv_nsec.cast_unsigned()")
                {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "timespec fields cast with cast_unsigned (u32 on 32-bit targets): {offenders:?}"
    );
}

/// CDF5: the decisions are recorded, including the 32-bit check command and the remaining
/// double-panic abort.
#[test]
fn test_cdf_decision_recorded() {
    let decisions = read("AI/DECISIONS.md");
    let title =
        "`O_PATH` Config Open, Re-installable v4l Hook Filter and test-pam Acceptance Fields";
    let start = decisions
        .find(title)
        .unwrap_or_else(|| panic!("AI/DECISIONS.md must record \"{title}\""));
    let entry = &decisions[start..];
    let entry = &entry[..entry.find("\n*").unwrap_or(entry.len())];
    for needle in [
        "ORT_SKIP_DOWNLOAD",
        "i686-unknown-linux-gnu",
        "/proc/self/fd",
        "rejected_reason",
        "aborts",
    ] {
        assert!(entry.contains(needle), "the ADR must mention `{needle}`");
    }
}

/// CDF4: CI type-checks the whole workspace for a 32-bit target on every PR, so a
/// `time_t` / `tv_sec` width assumption (for example passing `stat.st_mtime` where an `i64` is
/// expected) cannot reach `main` again.
#[test]
fn test_cdf_ci_type_checks_a_32_bit_target() {
    let ci = read(".github/workflows/ci.yml");
    let clippy_job = ci
        .split("\n  clippy:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n  test:\n").next())
        .expect("ci.yml must define the clippy job before the test job");
    assert!(
        clippy_job.contains("rustup target add i686-unknown-linux-gnu"),
        "the clippy job must install the i686 target"
    );
    assert!(
        clippy_job.contains(
            "cargo check --locked --workspace --all-targets --all-features --target i686-unknown-linux-gnu"
        ),
        "the clippy job must type-check the whole workspace for i686"
    );
    assert!(
        clippy_job.contains("apt-get install -y --no-install-recommends gcc-multilib"),
        "the i686 check needs the 32-bit libc headers for the v4l2-sys-mit bindgen run"
    );
    assert!(
        clippy_job.contains("ORT_SKIP_DOWNLOAD: \"1\""),
        "the i686 check must skip the ONNX Runtime download (no i686 binaries)"
    );
}
