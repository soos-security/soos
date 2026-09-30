//! Management functions for GDM login PAM integration and disable flag toggling.

use serde::Serialize;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::args::GdmAction;
use crate::error::AdminCliError;

/// PAM line inserted into the GDM service file by `gdm enable`.
pub const GDM_PAM_LINE: &str = "auth  sufficient  pam_soos.so timeout_ms=2500";

/// Suffix of the pristine copy kept next to an edited PAM file. `scripts/uninstall.sh`
/// restores every `*.soos-backup` found in `/etc/pam.d` (GitHub #166).
pub const PAM_BACKUP_SUFFIX: &str = ".soos-backup";

/// Status summary of GDM PAM integration and disable flag.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GdmStatus {
    /// True if `pam_soos.so` is configured in the PAM service file.
    pub installed: bool,
    /// True if GDM biometric authentication is active (not disabled).
    pub enabled: bool,
    /// Path to the inspected PAM service file.
    pub pam_file: PathBuf,
    /// Path to the inspected disable flag file.
    pub disable_file: PathBuf,
}

/// Inspects current GDM integration and disable state.
pub fn get_gdm_status(pam_file: &Path, disable_file: &Path) -> GdmStatus {
    let installed = if pam_file.is_file() {
        fs::read_to_string(pam_file)
            .map(|content| content.contains("pam_soos.so"))
            .unwrap_or(false)
    } else {
        false
    };

    let disabled = disable_file.exists() || Path::new("/etc/soos/disabled").exists();
    let enabled = installed && !disabled;

    GdmStatus {
        installed,
        enabled,
        pam_file: pam_file.to_path_buf(),
        disable_file: disable_file.to_path_buf(),
    }
}

/// Executes GDM configuration action (Status, Enable, Disable).
pub fn configure_gdm(
    action: &GdmAction,
    pam_file: &Path,
    disable_file: &Path,
) -> Result<GdmStatus, AdminCliError> {
    match action {
        GdmAction::Status => Ok(get_gdm_status(pam_file, disable_file)),
        GdmAction::Disable => {
            if let Some(parent) = disable_file.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).map_err(|e| {
                        AdminCliError::GdmConfig(format!(
                            "Failed to create directory '{}': {e}",
                            parent.display()
                        ))
                    })?;
                }
            }
            fs::write(disable_file, "disabled\n").map_err(|e| {
                AdminCliError::GdmConfig(format!(
                    "Failed to write disable flag '{}': {e}",
                    disable_file.display()
                ))
            })?;
            Ok(get_gdm_status(pam_file, disable_file))
        }
        GdmAction::Enable => {
            // Remove disable flag if present
            if disable_file.exists() {
                fs::remove_file(disable_file).map_err(|e| {
                    AdminCliError::GdmConfig(format!(
                        "Failed to remove disable flag '{}': {e}",
                        disable_file.display()
                    ))
                })?;
            }

            // Ensure pam_soos.so is in the PAM file (backup + atomic replacement).
            ensure_gdm_pam_line(pam_file)?;

            Ok(get_gdm_status(pam_file, disable_file))
        }
    }
}

/// Returns the path of the pristine backup kept next to `pam_file`.
#[must_use]
pub fn pam_backup_path(pam_file: &Path) -> PathBuf {
    let mut name = pam_file
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(PAM_BACKUP_SUFFIX);
    pam_file.with_file_name(name)
}

fn gdm_error(what: &str, path: &Path, err: &std::io::Error) -> AdminCliError {
    AdminCliError::GdmConfig(format!("{what} '{}': {err}", path.display()))
}

/// Inserts [`GDM_PAM_LINE`] into `pam_file` unless a `pam_soos.so` line is present.
///
/// The original bytes are first saved to [`pam_backup_path`] (never overwritten when a
/// backup already exists: it holds the pristine pre-soos state), then the new content
/// is written to a temporary file in the same directory, fsynced and renamed over the
/// original, so a crash never leaves a truncated PAM file. Symlinks and non-regular
/// files are refused without any modification.
fn ensure_gdm_pam_line(pam_file: &Path) -> Result<(), AdminCliError> {
    let metadata = match fs::symlink_metadata(pam_file) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AdminCliError::GdmConfig(format!(
                "PAM file '{}' does not exist",
                pam_file.display()
            )));
        }
        Err(e) => return Err(gdm_error("Failed to inspect PAM file", pam_file, &e)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AdminCliError::GdmConfig(format!(
            "Refusing to edit '{}': not a regular file (symlink or special file)",
            pam_file.display()
        )));
    }

    let original =
        fs::read(pam_file).map_err(|e| gdm_error("Failed to read PAM file", pam_file, &e))?;
    let content = String::from_utf8_lossy(&original);
    if content.contains("pam_soos.so") {
        return Ok(());
    }

    // Never propagate group/world write permission to the rewritten file or its backup.
    let mode = metadata.permissions().mode() & 0o7755;
    let owner = (metadata.uid(), metadata.gid());

    let backup = pam_backup_path(pam_file);
    match fs::symlink_metadata(&backup) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            write_atomic(&backup, &original, mode, owner)?;
        }
        Err(e) => return Err(gdm_error("Failed to inspect PAM backup", &backup, &e)),
    }

    let mut new_lines = Vec::new();
    let mut inserted = false;
    for line in content.lines() {
        if !inserted && line.contains("@include common-auth") {
            new_lines.push(GDM_PAM_LINE.to_string());
            inserted = true;
        }
        new_lines.push(line.to_string());
    }
    if !inserted {
        new_lines.insert(0, GDM_PAM_LINE.to_string());
    }
    new_lines.push(String::new());
    write_atomic(pam_file, new_lines.join("\n").as_bytes(), mode, owner)
}

/// Writes `bytes` to `target` atomically: exclusive temporary file in the same
/// directory (created with `mode`, owned like the original), `fsync`, `rename`,
/// then `fsync` of the directory. The temporary file is removed on failure.
fn write_atomic(
    target: &Path,
    bytes: &[u8],
    mode: u32,
    owner: (u32, u32),
) -> Result<(), AdminCliError> {
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{file_name}.soos-tmp-{}", std::process::id()));

    write_and_rename(&tmp, target, dir, bytes, mode, owner).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        gdm_error("Failed to write PAM file atomically", target, &e)
    })
}

fn write_and_rename(
    tmp: &Path,
    target: &Path,
    dir: &Path,
    bytes: &[u8],
    mode: u32,
    owner: (u32, u32),
) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(tmp)?;
    file.write_all(bytes)?;
    std::os::unix::fs::fchown(&file, Some(owner.0), Some(owner.1))?;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, target)?;
    fs::File::open(dir)?.sync_all()
}
