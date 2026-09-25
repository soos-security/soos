//! Command-line argument parsing for the `soos-gui` diagnostic application.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use clap::Parser;
use soos_biometric_store::DEFAULT_BIOMETRICS_DIR;
use soos_enrollment_cli::service::{DEFAULT_KEY_PATH, DEFAULT_MODELS_DIR};

/// SOOS Linux Biometric PAM — Real-Time Camera Visualizer & Guided Enrollment GUI.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "soos-gui",
    about = "Native Linux Biometric PAM live camera visualizer and guided enrollment manager",
    version
)]
pub struct GuiArgs {
    /// Path to camera device node (default: auto-detected from /dev/v4l/by-id/ or /dev/video0).
    #[arg(short = 'd', long = "camera-device", value_name = "DEVICE")]
    pub camera_device: Option<PathBuf>,

    /// Path to attested ONNX models directory (contains manifest.toml).
    #[arg(
        short = 'm',
        long = "models-dir",
        value_name = "DIR",
        default_value = DEFAULT_MODELS_DIR
    )]
    pub models_dir: PathBuf,

    /// Path to cryptographic master key file.
    #[arg(
        short = 'k',
        long = "key-file",
        value_name = "FILE",
        default_value = DEFAULT_KEY_PATH
    )]
    pub key_file: PathBuf,

    /// Directory storing encrypted biometric templates.
    #[arg(
        short = 'b',
        long = "biometrics-dir",
        value_name = "DIR",
        default_value = DEFAULT_BIOMETRICS_DIR
    )]
    pub biometrics_dir: PathBuf,

    /// Run with software mock camera and mock neural models (for testing without physical camera).
    #[arg(long = "mock")]
    pub mock: bool,
}
