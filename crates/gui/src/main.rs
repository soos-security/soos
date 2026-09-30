//! `soos-gui` — Main application entry point.

#![forbid(unsafe_code)]

use std::sync::Arc;

use clap::Parser;
use eframe::egui;
use soos_biometric_store::{BiometricStore, MasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager, V4lCameraManager};
use soos_enrollment_cli::service::{
    resolve_camera_device_from_config, MODEL_ID_EMBEDDING, MODEL_ID_FACE_DETECTOR, MODEL_ID_PAD,
};
use soos_gui::app::SoosApp;
use soos_gui::args::GuiArgs;
use soos_gui::camera_mode::{
    decide_camera_mode, probe_daemon_socket, CameraMode, DaemonSocketProbe,
    UnavailableCameraManager,
};
use soos_inference_ort::{
    MockEmbeddingExtractor, MockFaceDetector, MockPadDetector, ModelRegistry,
    OrtEmbeddingExtractor, OrtPadDetector, OrtScrfdDetector, RegistryConfig,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = GuiArgs::parse();

    // 1. Initialize Biometric Store
    let (key, is_system_key) = match MasterKey::load_or_create(&args.key_file) {
        Ok(k) => (k, true),
        Err(_) => {
            // Fallback for non-root testing if /var/lib/soos/master.key is root-only
            let fallback_key = std::env::temp_dir().join("soos-gui-master.key");
            (MasterKey::load_or_create(&fallback_key)?, false)
        }
    };

    let (store, is_system_store) = match BiometricStore::new(&args.biometrics_dir, key.clone()) {
        Ok(s) => (Arc::new(s), is_system_key),
        Err(_) => {
            let fallback_bio = std::env::temp_dir().join("soos-gui-biometrics");
            std::fs::create_dir_all(&fallback_bio)?;
            (Arc::new(BiometricStore::new(&fallback_bio, key)?), false)
        }
    };

    // 2. Initialize Camera and Neural Models
    let mut camera_notice: Option<String> = None;
    let (camera, pipeline): (Arc<dyn CameraManager>, Arc<VisionPipeline>) = if args.mock {
        let camera_config = CameraConfigBuilder::new().build();
        let cam: Arc<dyn CameraManager> = Arc::new(MockCameraManager::new(camera_config));
        let detector = Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95));
        let pad = Arc::new(MockPadDetector::new_live());
        let extractor = Arc::new(MockEmbeddingExtractor::new(512));
        let pipe = Arc::new(VisionPipeline::new(
            detector,
            pad,
            extractor,
            VisionPipelineConfig::default(),
        ));
        (cam, pipe)
    } else {
        let daemon_sock = std::path::Path::new("/run/soos/daemon.sock");
        // GitHub #150: the root daemon is the exclusive owner of /dev/video*. Direct V4L2 access
        // is only allowed when the daemon is provably not running; otherwise stream through the
        // daemon IPC (GitHub #143 preview authorization) or show an actionable blocked state.
        let socket_probe = probe_daemon_socket(daemon_sock);
        let preview_probe = (socket_probe == DaemonSocketProbe::Reachable)
            .then(|| soos_gui::IpcCameraManager::probe_preview(daemon_sock));
        let mode = decide_camera_mode(socket_probe, preview_probe, SoosApp::is_daemon_active());
        let cam: Arc<dyn CameraManager> = match mode {
            CameraMode::DaemonIpc => {
                tracing::info!(
                    "Connected to 'soos-daemon' video proxy at '{}'. Streaming via daemon IPC.",
                    daemon_sock.display()
                );
                Arc::new(soos_gui::IpcCameraManager::spawn(daemon_sock))
            }
            CameraMode::Blocked(reason) => {
                let message = reason.user_message();
                tracing::warn!(reason = ?reason, "{message}");
                camera_notice = Some(message);
                Arc::new(UnavailableCameraManager)
            }
            CameraMode::DirectV4l => {
                // Same shared resolver as soos-daemon and soos-enroll (GitHub #152).
                let device_path = resolve_camera_device_from_config(
                    args.camera_device,
                    Some(std::path::Path::new("/etc/soos/daemon.toml")),
                );
                let camera_config = CameraConfigBuilder::new()
                    .device_path(device_path.clone())
                    .warmup_frames(0)
                    .idle_timeout(std::time::Duration::ZERO)
                    .build();
                tracing::info!(
                    "soos-daemon is not running; opening direct V4L2 camera device '{}'",
                    device_path.display()
                );
                Arc::new(V4lCameraManager::spawn(camera_config)?)
            }
        };

        let mut registry = ModelRegistry::new(RegistryConfig::new(&args.models_dir))?;
        registry.verify_integrity()?;

        let det_session = registry.get_or_load_session(MODEL_ID_FACE_DETECTOR)?;
        let pad_session = registry.get_or_load_session(MODEL_ID_PAD)?;
        let emb_session = registry.get_or_load_session(MODEL_ID_EMBEDDING)?;

        let detector = Arc::new(OrtScrfdDetector::new(det_session, 0.60, 0.40)?);
        let pad = Arc::new(OrtPadDetector::new(pad_session, 0.80));
        let extractor = Arc::new(OrtEmbeddingExtractor::new(emb_session));

        let pipe = Arc::new(VisionPipeline::new(
            detector,
            pad,
            extractor,
            VisionPipelineConfig::default(),
        ));
        (cam, pipe)
    };

    // 3. Launch Native Desktop Window
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 780.0])
            .with_min_inner_size([900.0, 600.0])
            .with_active(true)
            .with_title("SOOS — Linux Biometric PAM"),
        ..Default::default()
    };

    eframe::run_native(
        "SOOS — Biometric Management & Live Analysis",
        native_options,
        Box::new(move |cc| {
            let mut app = SoosApp::new(cc, store, camera, pipeline, is_system_store);
            app.set_camera_notice(camera_notice);
            Ok(Box::new(app))
        }),
    )?;

    Ok(())
}
