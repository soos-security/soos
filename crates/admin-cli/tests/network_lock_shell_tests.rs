//! Runs the network lock shell test against a temporary root (GitHub #365, matrix NLK2,
//! NLK3): the real `soos-admin` binary and `scripts/uninstall.sh --destdir`.

#![allow(
    clippy::expect_used,
    reason = "Contractual test suite utilizes direct assertions and expect"
)]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn test_nlk_shell_install_status_uninstall_in_temp_root() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();
    let output = Command::new("bash")
        .arg(manifest.join("tests/network_lock_install_test.sh"))
        .arg(env!("CARGO_BIN_EXE_soos-admin"))
        .arg(&repo)
        .output()
        .expect("run bash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "network_lock_install_test.sh failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("all checks passed"), "{stdout}");
}
