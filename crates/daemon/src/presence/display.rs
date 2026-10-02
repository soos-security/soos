//! Screen gating input from the DRM sysfs class directory (GitHub #323, D8).
//!
//! `Off` gates the scan; `On` and `Unknown` allow it (owner: "when detectable").

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::{MAX_DRM_CONNECTORS, MAX_SYSFS_ATTR_BYTES};

/// Aggregate DPMS state of the connected displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayState {
    /// At least one connected connector is DPMS-on.
    On,
    /// At least one connector is connected and none is DPMS-on.
    Off,
    /// No connected connector, or the state could not be read.
    Unknown,
}

impl DisplayState {
    /// Stable, value-free code for logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Unknown => "unknown",
        }
    }
}

/// Source of the display state (mockable).
pub trait DisplayProbe: Send + Sync + 'static {
    /// Current display state.
    fn display_state(&self) -> DisplayState;
}

/// Pure classifier over `(status, dpms)` pairs (values already trimmed).
#[must_use]
pub fn classify_connectors(connectors: &[(&str, &str)]) -> DisplayState {
    let mut any_connected = false;
    for (status, dpms) in connectors {
        if *status == "connected" {
            any_connected = true;
            if *dpms == "On" {
                return DisplayState::On;
            }
        }
    }
    if any_connected {
        DisplayState::Off
    } else {
        DisplayState::Unknown
    }
}

/// Whether `name` matches `card[0-9]+-[A-Za-z0-9-]+`.
fn is_connector_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("card") else {
        return false;
    };
    let Some((index, connector)) = rest.split_once('-') else {
        return false;
    };
    !index.is_empty()
        && index.bytes().all(|b| b.is_ascii_digit())
        && !connector.is_empty()
        && connector
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Reads one sysfs attribute (at most [`MAX_SYSFS_ATTR_BYTES`]); `None` when unreadable or
/// longer. One trailing newline is removed.
fn read_attribute(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let limit = u64::try_from(MAX_SYSFS_ATTR_BYTES).ok()?.checked_add(1)?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).ok()?;
    if bytes.len() > MAX_SYSFS_ATTR_BYTES {
        return None;
    }
    let content = String::from_utf8(bytes).ok()?;
    let trimmed = content.strip_suffix('\n').unwrap_or(&content);
    Some(trimmed.to_string())
}

/// Reads `<dir>/card<N>-<connector>/{status,dpms}`.
#[derive(Debug, Clone)]
pub struct SysfsDisplayProbe {
    dir: PathBuf,
}

impl SysfsDisplayProbe {
    /// Probe over `dir` (production: `DEFAULT_DRM_SYSFS_DIR`).
    #[must_use]
    pub const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `(status, dpms)` of every readable connector, or `None` when the directory is
    /// unreadable or holds more than [`MAX_DRM_CONNECTORS`] entries.
    fn connectors(&self) -> Option<Vec<(String, String)>> {
        let entries = std::fs::read_dir(&self.dir).ok()?;
        let mut names = Vec::new();
        for (count, entry) in entries.enumerate() {
            if count >= MAX_DRM_CONNECTORS {
                return None;
            }
            let entry = entry.ok()?;
            if let Some(name) = entry.file_name().to_str() {
                if is_connector_name(name) {
                    names.push(name.to_string());
                }
            }
        }
        let mut connectors = Vec::new();
        for name in names {
            // Class entries are symlinks into the device tree; `join` + `open` follows them.
            let base = self.dir.join(&name);
            if let (Some(status), Some(dpms)) = (
                read_attribute(&base.join("status")),
                read_attribute(&base.join("dpms")),
            ) {
                connectors.push((status, dpms));
            }
        }
        Some(connectors)
    }
}

impl DisplayProbe for SysfsDisplayProbe {
    fn display_state(&self) -> DisplayState {
        match self.connectors() {
            Some(connectors) => {
                let pairs: Vec<(&str, &str)> = connectors
                    .iter()
                    .map(|(status, dpms)| (status.as_str(), dpms.as_str()))
                    .collect();
                classify_connectors(&pairs)
            }
            None => DisplayState::Unknown,
        }
    }
}
