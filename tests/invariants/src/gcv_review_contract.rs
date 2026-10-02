//! Static contracts of the 2026-10-02 GUI / camera / vision review fixes (GitHub #313, #314;
//! walkthrough 168, matrix rows GCV10, GCV11, GCV14, GCV19).
//!
//! - every entry of the shipped `models/manifest.toml` declares its `input_layout` (VIS-NEW-4);
//! - the face pixel buffers named by the review are zeroizing containers (VIS-NEW-5);
//! - `v4l::Device::with_path` is only called by the guarded opener (CAM-NEW-4);
//! - `soos-gui` runs `pkexec`, `systemctl` and `soos-enroll` by absolute path, and the
//!   `soos-enroll` path is the one every installer uses (CAM-NEW-7 b).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contract test suite uses assertions"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// GCV10 (VIS-NEW-4): each `[models.<id>]` table of the shipped manifest declares
/// `input_layout`, so the registry asserts the layout of every model and never emits the
/// "manifest predates input layout attestation" warning on a fresh install.
#[test]
fn test_gcv_every_shipped_manifest_entry_declares_input_layout() {
    let manifest = read("models/manifest.toml");
    let mut tables = 0usize;
    let mut current: Option<String> = None;
    let mut declared = false;
    let mut missing: Vec<String> = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            if let Some(id) = current.take() {
                if !declared {
                    missing.push(id);
                }
            }
            if let Some(id) = line
                .strip_prefix("[models.")
                .and_then(|rest| rest.strip_suffix(']'))
            {
                tables += 1;
                current = Some(id.to_string());
                declared = false;
            }
        } else if current.is_some() && line.starts_with("input_layout") {
            declared = true;
        }
    }
    if let Some(id) = current {
        if !declared {
            missing.push(id);
        }
    }
    assert!(tables >= 3, "the manifest lists the shipped models");
    assert!(
        missing.is_empty(),
        "models/manifest.toml entries without input_layout: {missing:?}"
    );
}

/// GCV11 (VIS-NEW-5): the face buffers named by the review are zeroizing containers.
#[test]
fn test_gcv_face_pixel_buffers_are_zeroizing() {
    for (rel, needle) in [
        (
            "crates/vision/src/ir_liveness.rs",
            "let luma: Zeroizing<Vec<f64>> = Zeroizing::new(",
        ),
        (
            "crates/vision/src/color.rs",
            "let mut decoded_bytes = Zeroizing::new(",
        ),
        (
            "crates/inference-ort/src/pad.rs",
            "let probs = Zeroizing::new(Self::softmax(&logits));",
        ),
        (
            "crates/inference-ort/src/pad.rs",
            "let exps: Zeroizing<Vec<f32>> =",
        ),
    ] {
        assert!(read(rel).contains(needle), "{rel} must contain `{needle}`");
    }
}

/// GCV14 (CAM-NEW-4): `v4l` unwraps `CString::new` on the device path, so the crate opens
/// nodes only through `open_device_guarded` (NUL check, panic guard, guarded close).
#[test]
fn test_gcv_v4l_devices_are_opened_only_through_the_guard() {
    let dir = workspace_root().join("crates/camera-v4l/src");
    let mut offenders = Vec::new();
    let mut stack = vec![dir];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") || path.ends_with("v4l_guard.rs") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            if path.ends_with("v4l_impl.rs") {
                // `V4lBackend::open_device` (the single opener CCB11 requires) checks the path
                // and runs the call inside the guard.
                assert!(
                    text.contains(
                        "reject_nul_device_path(path)?;\n        guard_v4l_call(|| v4l::Device::with_path(path))"
                    ),
                    "V4lBackend::open_device must reject NUL bytes and open inside the guard"
                );
                assert!(text.contains("let opened = DropGuarded::new("));
                continue;
            }
            if text.contains("Device::with_path(") || text.contains("Device::new(") {
                offenders.push(path.display().to_string());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "open V4L2 nodes with v4l_guard::open_device_guarded: {offenders:?}"
    );
    let guard = read("crates/camera-v4l/src/v4l_guard.rs");
    assert!(guard.contains("pub(crate) fn open_device_guarded"));
    assert!(guard.contains("impl<T> Drop for DropGuarded<T>"));
}

/// GCV19 (CAM-NEW-7 b): privileged programs run by absolute path, and the `soos-enroll` path
/// is where every installer puts it.
#[test]
fn test_gcv_gui_privileged_programs_use_absolute_paths() {
    let privileged = read("crates/gui/src/privileged.rs");
    for constant in [
        "pub const PKEXEC_PROGRAM: &str = \"/usr/bin/pkexec\";",
        "pub const SYSTEMCTL_PROGRAM: &str = \"/usr/bin/systemctl\";",
        "pub const SOOS_ENROLL_PROGRAM: &str = \"/usr/bin/soos-enroll\";",
    ] {
        assert!(privileged.contains(constant), "missing `{constant}`");
    }
    for relative in [
        "Command::new(\"pkexec\")",
        "&[\"systemctl\",",
        "&[\"soos-enroll\",",
        ".args([\"soos-enroll\",",
    ] {
        assert!(
            !privileged.contains(relative),
            "privileged.rs still runs a program through PATH: `{relative}`"
        );
    }
    let monitor = read("crates/gui/src/daemon_control.rs");
    assert!(monitor.contains("Command::new(crate::privileged::SYSTEMCTL_PROGRAM)"));
    assert!(!monitor.contains("Command::new(\"systemctl\")"));

    // Installed location of soos-enroll: install.sh (default prefix /usr, also used by the
    // Debian package), the Arch PKGBUILD and the RPM spec (%{_bindir} = /usr/bin).
    let install = read("scripts/install.sh");
    assert!(install.contains("PREFIX=\"/usr\""));
    assert!(install.contains("install_artifact soos-enroll 0755 \"${TARGET_BIN_DIR}/soos-enroll\""));
    assert!(install.contains("TARGET_BIN_DIR=\"${DESTDIR}${PREFIX}/bin\""));
    assert!(read("packaging/arch/PKGBUILD").contains("\"${pkgdir}/usr/bin/soos-enroll\""));
    assert!(read("packaging/rpm/soos.spec").contains("%{buildroot}%{_bindir}/soos-enroll"));
}
