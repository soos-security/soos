//! Mapping of kernel-indexed V4L2 nodes (`/dev/videoN`) to persistent `/dev/v4l/by-id/...` links.
//!
//! Kernel indices are assigned in probe order and change across boots, suspend/resume and USB
//! replug; `AI/ARCHITECTURE.md` requires the daemon to address cameras by stable hardware ID.

use std::path::{Path, PathBuf};

/// Directory where udev publishes persistent per-device V4L2 symlinks.
pub const DEFAULT_BY_ID_DIR: &str = "/dev/v4l/by-id";

/// Upper bound on the number of by-id entries inspected (bounded I/O).
const MAX_BY_ID_ENTRIES: usize = 256;

/// Returns the persistent by-id symlink in `by_id_dir` that resolves to `node`, or `node` itself
/// when no such link exists (GitHub #151).
///
/// When several links resolve to the same node, the lexicographically smallest name is returned
/// so the choice is deterministic. Unreadable directories and dangling links are ignored.
pub fn stable_device_path(node: &Path, by_id_dir: &Path) -> PathBuf {
    let Ok(target) = std::fs::canonicalize(node) else {
        return node.to_path_buf();
    };
    let Ok(entries) = std::fs::read_dir(by_id_dir) else {
        return node.to_path_buf();
    };

    let mut matches: Vec<PathBuf> = entries
        .take(MAX_BY_ID_ENTRIES)
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|link| {
            std::fs::canonicalize(link)
                .map(|resolved| resolved == target)
                .unwrap_or(false)
        })
        .collect();
    matches.sort();

    matches
        .into_iter()
        .next()
        .unwrap_or_else(|| node.to_path_buf())
}
