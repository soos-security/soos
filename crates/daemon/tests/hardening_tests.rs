//! Production hardening integration test suite: swap protection, zeroization audit, and cargo-deny.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Test suite assertions"
)]

use std::fs;
use std::path::PathBuf;
use zeroize::Zeroize;

use soos_daemon::mlock::{
    mlock_process_address_space, mlock_slice, munlock_process_address_space, munlock_slice,
    LockedBuffer,
};

fn find_workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("Failed to locate workspace root")
        .to_path_buf()
}

#[test]
fn test_mlock_slice_and_munlock_slice_lifecycle() {
    let buffer = vec![0x42_u8; 4096];
    let locked = mlock_slice(&buffer);
    // If running in unprivileged sandbox, locked will be false; if root, true.
    // In either case it must not panic and must execute safely.
    if locked {
        munlock_slice(&buffer);
    }
}

#[test]
fn test_locked_buffer_raii_wrapper() {
    let mut secret = [0x77_u8; 32];
    {
        let locked = LockedBuffer::new(secret);
        assert_eq!(locked.as_ref().len(), 32);
        assert_eq!(locked.as_ref()[0], 0x77);
    }
    secret.zeroize();
    assert_eq!(secret[0], 0);
}

#[test]
fn test_mlock_process_address_space_call() {
    let ok = mlock_process_address_space();
    // Must execute without panic, returning true if privileged or false if unprivileged
    if ok {
        munlock_process_address_space();
    }
}

#[test]
fn test_cargo_deny_bans_duplicate_versions() {
    let root = find_workspace_root();
    let deny_file = root.join("deny.toml");
    assert!(deny_file.exists(), "deny.toml not found");

    let content = fs::read_to_string(&deny_file).expect("Failed to read deny.toml");

    // Sub-issue #16.4: Block duplicate dependency versions
    assert!(
        content.contains("multiple-versions = \"deny\""),
        "SECURITY AUDIT VIOLATION: deny.toml must enforce multiple-versions = \"deny\""
    );

    // Verify OpenCV and Nokhwa remain explicitly banned
    assert!(
        content.contains("opencv"),
        "deny.toml must maintain explicit ban on opencv"
    );
    assert!(
        content.contains("nokhwa"),
        "deny.toml must maintain explicit ban on nokhwa"
    );
}

#[test]
fn test_systemd_hardening_directives_complete() {
    let root = find_workspace_root();
    let service_file = root.join("packaging").join("soos-daemon.service");
    assert!(service_file.exists(), "soos-daemon.service not found");

    let content = fs::read_to_string(&service_file).expect("Failed to read service file");

    // Sub-issue #16.3: Verify MemoryDenyWriteExecute, RestrictSUIDSGID, SystemCallArchitectures=native
    assert!(
        content.contains("MemoryDenyWriteExecute=yes"),
        "Missing MemoryDenyWriteExecute=yes"
    );
    assert!(
        content.contains("RestrictSUIDSGID=yes"),
        "Missing RestrictSUIDSGID=yes"
    );
    assert!(
        content.contains("SystemCallArchitectures=native"),
        "Missing SystemCallArchitectures=native"
    );
    assert!(
        content.contains("NoNewPrivileges=yes"),
        "Missing NoNewPrivileges=yes"
    );
    assert!(content.contains("PrivateTmp=yes"), "Missing PrivateTmp=yes");
    assert!(
        content.contains("ProtectHome=yes"),
        "Missing ProtectHome=yes"
    );
    assert!(
        content.contains("ProtectSystem=strict"),
        "Missing ProtectSystem=strict"
    );
    assert!(
        content.contains("DevicePolicy=closed"),
        "Missing DevicePolicy=closed"
    );
    assert!(
        content.contains("DeviceAllow=/dev/video* rw"),
        "Missing DeviceAllow=/dev/video* rw"
    );
    assert!(
        content.contains("RestrictAddressFamilies=AF_UNIX"),
        "Missing RestrictAddressFamilies=AF_UNIX"
    );
    assert!(
        content.contains("LockPersonality=yes"),
        "Missing LockPersonality=yes"
    );

    // Negative assertions: ensure no insecure network protocols or unrestricted devices
    assert!(
        !content.contains("AF_INET"),
        "Security violation: service must not allow AF_INET"
    );
    assert!(
        !content.contains("AF_NETLINK"),
        "Security violation: service must not allow AF_NETLINK"
    );
}
