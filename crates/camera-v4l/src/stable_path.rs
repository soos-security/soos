//! Mapping of kernel-indexed V4L2 nodes (`/dev/videoN`) to persistent `/dev/v4l/by-id/...` links.
//!
//! Kernel indices are assigned in probe order and change across boots, suspend/resume and USB
//! replug; `AI/ARCHITECTURE.md` requires the daemon to address cameras by stable hardware ID.
//!
//! There is a single, bounded by-id scanner in the workspace:
//! [`CameraEnumerator::by_id_aliases`] of [`SystemCameraEnumerator`] (bound
//! [`crate::resolver::MAX_BY_ID_ENTRIES`]). [`stable_device_path`] delegates to it so that the
//! alias chosen here is always the alias the shared resolver would choose (GitHub #152).

use crate::resolver::{CameraEnumerator, SystemCameraEnumerator};
use std::path::{Path, PathBuf};

/// Directory where udev publishes persistent per-device V4L2 symlinks (shared with the resolver).
pub use crate::resolver::DEFAULT_BY_ID_DIR;

/// Returns the persistent by-id symlink in `by_id_dir` that resolves to `node`, or `node` itself
/// when no such link exists (GitHub #151).
///
/// When several links resolve to the same node, the lexicographically smallest name is returned
/// so the choice is deterministic. Unreadable directories and dangling links are ignored.
pub fn stable_device_path(node: &Path, by_id_dir: &Path) -> PathBuf {
    let Ok(target) = std::fs::canonicalize(node) else {
        return node.to_path_buf();
    };
    // `by_id_aliases` is bounded, skips dangling links and is sorted by alias name.
    SystemCameraEnumerator::with_by_id_dir(by_id_dir)
        .by_id_aliases()
        .into_iter()
        .find(|(_, resolved)| *resolved == target)
        .map_or_else(|| node.to_path_buf(), |(alias, _)| alias)
}
