//! Preview peer origin classification (GitHub #345, ADR 2026-10-07 "Live Camera View in
//! `soos-remote` Through the Daemon Preview Channel", owner decision LC-3b).
//!
//! The daemon serves preview frames to an unprivileged peer only when `[preview]` allows its
//! UID. A peer running inside the `soos-remote.service` user unit (the remote companion) is
//! additionally refused unless `[preview] remote_view = true`. The unit is recognised from
//! the peer's `/proc/<pid>/cgroup`, read once per connection.
//!
//! This classification is an administrative opt-in and an audit aid, **not** a security
//! boundary: any process of the owner can start a unit of that name or run the companion
//! outside it. It fails closed: an unreadable, oversized or inconsistent cgroup refuses the
//! preview.
//!
//! The parser is pure (no I/O) and considers only the hierarchies systemd manages (the
//! unified `0::` line and the cgroup v1 `name=systemd` line), like the local-session policy.

use crate::session_policy::{parse_strict_uid, systemd_hierarchy_path};

/// Name of the remote companion's systemd user unit.
pub const REMOTE_COMPANION_UNIT: &str = "soos-remote.service";

/// Where a preview peer runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPeerOrigin {
    /// Any process that is not the remote companion unit (GUI, terminal, other units).
    Local,
    /// A process inside the `soos-remote.service` user unit of the peer's own UID.
    RemoteCompanion,
}

/// Why the origin of a preview peer could not be established (every case refuses).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PreviewPeerError {
    /// The kernel did not provide a usable peer PID.
    #[error("peer pid unavailable")]
    MissingPid,
    /// The peer's cgroup file could not be read (absent, unreadable or oversized).
    #[error("peer cgroup unreadable")]
    Unreadable,
    /// The peer's cgroup content is inconsistent or malformed.
    #[error("peer cgroup malformed")]
    Malformed,
}

/// Classifies one systemd-hierarchy cgroup path.
fn classify_path(path: &str, peer_uid: u32) -> Result<PreviewPeerOrigin, PreviewPeerError> {
    // The root cgroup.
    if path == "/" {
        return Ok(PreviewPeerOrigin::Local);
    }
    let Some(rest) = path.strip_prefix('/') else {
        return Err(PreviewPeerError::Malformed);
    };
    let components: Vec<&str> = rest.split('/').collect();
    // An interior or trailing empty component, `.` or `..` fails closed (never `Local`).
    if components
        .iter()
        .any(|c| c.is_empty() || *c == "." || *c == "..")
    {
        return Err(PreviewPeerError::Malformed);
    }
    let mut iter = components.into_iter();
    if iter.next() != Some("user.slice") {
        return Ok(PreviewPeerOrigin::Local);
    }
    let (Some(slice), Some(service)) = (iter.next(), iter.next()) else {
        return Ok(PreviewPeerOrigin::Local);
    };
    if !service.starts_with("user@") {
        return Ok(PreviewPeerOrigin::Local);
    }
    let slice_uid = slice
        .strip_prefix("user-")
        .and_then(|r| r.strip_suffix(".slice"))
        .and_then(parse_strict_uid)
        .ok_or(PreviewPeerError::Malformed)?;
    let service_uid = service
        .strip_prefix("user@")
        .and_then(|r| r.strip_suffix(".service"))
        .and_then(parse_strict_uid)
        .ok_or(PreviewPeerError::Malformed)?;
    if slice_uid != service_uid || service_uid != peer_uid {
        return Err(PreviewPeerError::Malformed);
    }
    // The first non-`.slice` component below the user manager names the unit.
    match iter.find(|c| !c.ends_with(".slice")) {
        Some(unit) if unit == REMOTE_COMPANION_UNIT => Ok(PreviewPeerOrigin::RemoteCompanion),
        _ => Ok(PreviewPeerOrigin::Local),
    }
}

/// Classifies a preview peer from its `/proc/<pid>/cgroup` content.
///
/// Considers only the systemd hierarchies (`0::` and `name=systemd`). For each such line, a
/// path below `/user.slice/user-<u>.slice/user@<u>.service/` (strict UIDs, equal on both
/// components and equal to `peer_uid`, else `Malformed`) whose first non-`.slice` component
/// after the manager service is exactly [`REMOTE_COMPANION_UNIT`] is `RemoteCompanion`; any
/// other path is `Local`. A path with an empty, `.` or `..` component is `Malformed`.
/// Considered lines that disagree are `Malformed`; no considered line is `Local`.
///
/// # Errors
/// [`PreviewPeerError::Malformed`] as described above.
pub fn classify_preview_peer_cgroup(
    content: &str,
    peer_uid: u32,
) -> Result<PreviewPeerOrigin, PreviewPeerError> {
    let mut found: Option<PreviewPeerOrigin> = None;
    for line in content.lines() {
        let Some(path) = systemd_hierarchy_path(line) else {
            continue;
        };
        let origin = classify_path(path, peer_uid)?;
        match found {
            Some(previous) if previous != origin => return Err(PreviewPeerError::Malformed),
            _ => found = Some(origin),
        }
    }
    Ok(found.unwrap_or(PreviewPeerOrigin::Local))
}
