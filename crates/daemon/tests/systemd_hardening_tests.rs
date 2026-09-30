//! Contractual tests for the least-privilege systemd sandbox of `soos-daemon`
//! (GitHub #203, DMN-14).
//!
//! The daemon parses untrusted IPC, decodes camera frames and runs a native ONNX runtime,
//! so a compromise must not yield full root capabilities, kernel access or network access.
//! The directives are parsed from the active (non-comment) lines of the unit so a comment
//! can never satisfy an assertion.
//!
//! Compatibility constraints enforced here:
//! - the session policy reads `/proc/<pid>/cgroup` of peers owned by other users, so
//!   `ProtectProc=invisible` / `ProcSubset=pid` and `PrivateDevices=yes` are forbidden;
//! - `/dev/video*` stays reachable through the `char-video4linux` device group, because
//!   systemd does not expand globs in `DeviceAllow=` device node paths.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::collections::BTreeSet;
use std::path::PathBuf;

fn unit_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("packaging")
        .join("soos-daemon.service")
}

/// Returns `(section, key, value)` for every active directive of the unit.
fn directives() -> Vec<(String, String, String)> {
    let content = std::fs::read_to_string(unit_path()).expect("read unit");
    let mut section = String::new();
    let mut out = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line.to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.push((section.clone(), k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn values(section: &str, key: &str) -> Vec<String> {
    directives()
        .into_iter()
        .filter(|(s, k, _)| s == section && k == key)
        .map(|(_, _, v)| v)
        .collect()
}

fn single(section: &str, key: &str) -> String {
    let v = values(section, key);
    assert_eq!(
        v.len(),
        1,
        "{section} must set `{key}=` exactly once, found {v:?}"
    );
    v[0].clone()
}

#[test]
fn test_unit_bounds_capabilities_to_daemon_needs() {
    let caps: BTreeSet<String> = single("[Service]", "CapabilityBoundingSet")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let expected: BTreeSet<String> = [
        "CAP_IPC_LOCK",
        "CAP_CHOWN",
        "CAP_FOWNER",
        "CAP_DAC_OVERRIDE",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(
        caps, expected,
        "CapabilityBoundingSet must be exactly the capabilities the daemon uses"
    );
    assert_eq!(
        single("[Service]", "AmbientCapabilities"),
        "",
        "no ambient capabilities may be granted"
    );
}

#[test]
fn test_unit_filters_syscalls_to_system_service() {
    assert_eq!(single("[Service]", "SystemCallFilter"), "@system-service");
    assert_eq!(single("[Service]", "SystemCallErrorNumber"), "EPERM");
    assert_eq!(single("[Service]", "SystemCallArchitectures"), "native");
}

#[test]
fn test_unit_enables_kernel_namespace_and_realtime_protections() {
    for key in [
        "ProtectKernelTunables",
        "ProtectKernelModules",
        "ProtectKernelLogs",
        "ProtectControlGroups",
        "ProtectClock",
        "ProtectHostname",
        "RestrictNamespaces",
        "RestrictRealtime",
        "PrivateNetwork",
        "NoNewPrivileges",
    ] {
        assert_eq!(single("[Service]", key), "yes", "`{key}=yes` is mandatory");
    }
}

#[test]
fn test_unit_denies_all_ip_traffic() {
    assert_eq!(single("[Service]", "IPAddressDeny"), "any");
    assert_eq!(single("[Service]", "RestrictAddressFamilies"), "AF_UNIX");
    assert!(
        values("[Service]", "IPAddressAllow").is_empty(),
        "no IPAddressAllow= exception may exist"
    );
}

#[test]
fn test_unit_keeps_peer_proc_and_camera_access() {
    for key in ["ProtectProc", "ProcSubset"] {
        assert!(
            values("[Service]", key).is_empty(),
            "`{key}=` would hide /proc/<pid>/cgroup of other users' peers from the session policy"
        );
    }
    for v in values("[Service]", "PrivateDevices") {
        assert_ne!(v, "yes", "PrivateDevices=yes would remove /dev/video*");
    }
    assert_eq!(single("[Service]", "DevicePolicy"), "closed");
    let allows = values("[Service]", "DeviceAllow");
    assert!(
        allows.iter().any(|v| v == "char-video4linux rw"),
        "V4L2 devices must be allowed through the char-video4linux device group: {allows:?}"
    );
    for v in &allows {
        let node = v.split_whitespace().next().unwrap_or_default();
        assert!(
            node.starts_with("char-video4linux") || node.starts_with("/dev/video"),
            "unexpected device allowance `{v}`"
        );
    }
}

#[test]
fn test_unit_starts_before_display_manager() {
    let before: Vec<String> = values("[Unit]", "Before")
        .iter()
        .flat_map(|v| v.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .collect();
    assert!(
        before.iter().any(|u| u == "display-manager.service"),
        "the daemon must be ordered Before=display-manager.service, got {before:?}"
    );
    assert_eq!(single("[Install]", "WantedBy"), "multi-user.target");
}

#[test]
fn test_unit_never_grants_dangerous_capabilities() {
    let content = std::fs::read_to_string(unit_path()).expect("read unit");
    let active: String = content
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    for cap in [
        "CAP_SYS_ADMIN",
        "CAP_SYS_MODULE",
        "CAP_SYS_PTRACE",
        "CAP_NET_ADMIN",
        "CAP_NET_RAW",
        "CAP_NET_BIND_SERVICE",
        "CAP_SETUID",
        "CAP_SETGID",
        "CAP_SYS_RAWIO",
        "CAP_MKNOD",
        "CAP_BPF",
    ] {
        assert!(
            !active.contains(cap),
            "soos-daemon.service must not grant {cap}"
        );
    }
}
