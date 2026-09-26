//! `soos-gui` — Native Linux Biometric PAM live camera visualizer and guided enrollment manager.
//!
//! Provides:
//! - Real-time V4L2 camera streaming with authentic ONNX model overlays (SCRFD, MiniFASNetV2, ArcFace)
//! - Apple FaceID-style multi-angle guided enrollment flow
//! - Interactive biometric profile consultation and secure anti-forensic deletion

#![forbid(unsafe_code)]

pub mod app;
pub mod args;
pub mod ipc_camera;
pub mod state;
pub mod worker;

pub use app::SoosApp;
pub use args::GuiArgs;
pub use ipc_camera::IpcCameraManager;
