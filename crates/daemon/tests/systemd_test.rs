//! Contractual integration test for systemd sandboxing directives.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::fs;
use std::path::PathBuf;

fn find_workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("Failed to locate workspace root")
        .to_path_buf()
}

#[test]
fn test_systemd_unit_file_sandboxing_directives() {
    let root = find_workspace_root();
    let service_file = root.join("packaging").join("soos-daemon.service");

    assert!(
        service_file.exists(),
        "Required systemd unit file '{}' not found",
        service_file.display()
    );

    let content = fs::read_to_string(&service_file).expect("Failed to read unit file");

    // Acceptance D3 & Issue #35.3: Verify all systemd sandbox directives are explicitly enabled
    let mandatory_directives = [
        "User=root",
        "Group=soos",
        "ExecStart=/usr/libexec/soos/soos-daemon",
        "Restart=on-failure",
        "RestartSec=2",
        "UMask=0077",
        "RuntimeDirectory=soos",
        "RuntimeDirectoryMode=0750",
        "StateDirectory=soos",
        "ReadWritePaths=/var/lib/soos /run/soos",
        "NoNewPrivileges=yes",
        "PrivateTmp=yes",
        "ProtectHome=yes",
        "ProtectSystem=strict",
        "DevicePolicy=closed",
        "DeviceAllow=/dev/video* rw",
        "RestrictAddressFamilies=AF_UNIX", // Core Acceptance D3
        "LockPersonality=yes",
        "MemoryDenyWriteExecute=yes",
        "RestrictSUIDSGID=yes",
        "SystemCallArchitectures=native",
    ];

    for directive in &mandatory_directives {
        assert!(
            content.contains(directive),
            "SECURITY INVARIANT VIOLATION: systemd service missing mandatory sandbox directive '{}' in {}",
            directive,
            service_file.display()
        );
    }

    assert!(
        !content.contains("Group=root"),
        "SECURITY INVARIANT VIOLATION: soos-daemon.service must run under Group=soos, not Group=root"
    );
}
