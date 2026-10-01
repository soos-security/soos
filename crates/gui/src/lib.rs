//! `soos-gui` — Native Linux Biometric PAM live camera visualizer and guided enrollment manager.
//!
//! Provides:
//! - Real-time V4L2 camera streaming with authentic ONNX model overlays (SCRFD, MiniFASNetV2, ArcFace)
//! - Apple FaceID-style multi-angle guided enrollment flow
//! - Interactive biometric profile consultation and deletion (best-effort in-place
//!   overwrite; the guarantee against recovery is encryption at rest plus master-key
//!   destruction, ADR 2026-09-30 "Biometric Template Erasure Model")

#![forbid(unsafe_code)]

pub mod app;
pub mod args;
pub mod camera_mode;
pub mod camera_source;
pub mod camera_status;
pub mod daemon_control;
pub mod ipc_camera;
pub mod logging;
pub mod privileged;
pub mod state;
pub mod store_mode;
pub mod store_tasks;
pub mod worker;

pub use app::SoosApp;
pub use args::GuiArgs;
pub use camera_mode::{CameraBlockReason, CameraMode};
pub use ipc_camera::{IpcCameraManager, IpcPreviewError};
