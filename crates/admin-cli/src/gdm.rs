//! Management functions for GDM login PAM integration and disable flag toggling.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::args::GdmAction;
use crate::error::AdminCliError;

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

            // Ensure pam_soos.so is in the PAM file
            if pam_file.is_file() {
                let content = fs::read_to_string(pam_file).map_err(|e| {
                    AdminCliError::GdmConfig(format!(
                        "Failed to read PAM file '{}': {e}",
                        pam_file.display()
                    ))
                })?;

                if !content.contains("pam_soos.so") {
                    let mut new_lines = Vec::new();
                    let mut inserted = false;
                    for line in content.lines() {
                        if !inserted && line.contains("@include common-auth") {
                            new_lines
                                .push("auth  sufficient  pam_soos.so timeout_ms=2500".to_string());
                            inserted = true;
                        }
                        new_lines.push(line.to_string());
                    }
                    if !inserted {
                        new_lines.insert(
                            0,
                            "auth  sufficient  pam_soos.so timeout_ms=2500".to_string(),
                        );
                    }
                    new_lines.push(String::new());
                    fs::write(pam_file, new_lines.join("\n")).map_err(|e| {
                        AdminCliError::GdmConfig(format!(
                            "Failed to write PAM file '{}': {e}",
                            pam_file.display()
                        ))
                    })?;
                }
            } else {
                return Err(AdminCliError::GdmConfig(format!(
                    "PAM file '{}' does not exist",
                    pam_file.display()
                )));
            }

            Ok(get_gdm_status(pam_file, disable_file))
        }
    }
}
