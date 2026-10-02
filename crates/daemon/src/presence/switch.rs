//! Presence kill switch (GitHub #323, D9).
//!
//! `<dir>/disabled` (the existing global PAM flag) or `<dir>/presence.disable` stops presence
//! at the next tick and again right before any `UnlockSession`; `gdm.disable` and the other
//! per-service PAM flags do not.

use std::io::ErrorKind;
use std::path::PathBuf;

use super::{GLOBAL_DISABLE_FLAG, PRESENCE_DISABLE_FLAG};

/// Kill-switch flag files of one directory.
#[derive(Debug, Clone)]
pub struct PresenceSwitch {
    dir: PathBuf,
}

impl PresenceSwitch {
    /// Watches the flag files of `dir` (production: `DEFAULT_KILL_SWITCH_DIR`).
    #[must_use]
    pub const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `true` when `<dir>/disabled` or `<dir>/presence.disable` exists as any entry (a
    /// dangling symlink counts), and on any stat error other than `NotFound` (fail closed).
    /// Re-evaluated on every call.
    #[must_use]
    pub fn is_engaged(&self) -> bool {
        [GLOBAL_DISABLE_FLAG, PRESENCE_DISABLE_FLAG]
            .iter()
            .any(
                |flag| match std::fs::symlink_metadata(self.dir.join(flag)) {
                    Ok(_) => true,
                    Err(err) => err.kind() != ErrorKind::NotFound,
                },
            )
    }
}
