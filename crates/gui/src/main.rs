//! `soos-gui` — Main application entry point.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use eframe::egui;
use soos_biometric_store::{BiometricStore, MasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager};
use soos_enrollment_cli::service::{MODEL_ID_EMBEDDING, MODEL_ID_FACE_DETECTOR, MODEL_ID_PAD};
use soos_gui::app::SoosApp;
use soos_gui::args::GuiArgs;
use soos_gui::camera_source::{CameraSourceBackend, SwitchableCamera, SystemCameraSourceBackend};
use soos_inference_ort::{
    MockEmbeddingExtractor, MockFaceDetector, MockPadDetector, ModelRegistry,
    OrtEmbeddingExtractor, OrtPadDetector, OrtScrfdDetector, RegistryConfig,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // GitHub #155: install the stderr subscriber before anything logs (RUST_LOG honored).
    soos_gui::logging::init();
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
    let mut camera_source: Option<(Arc<SwitchableCamera>, Arc<dyn CameraSourceBackend>)> = None;
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
        // GitHub #150 / #154: the root daemon is the exclusive owner of /dev/video*. The camera
        // source is decided at runtime by the camera-source supervisor (attached below), which
        // follows the daemon state: IPC preview while the daemon runs, direct V4L2 through the
        // shared resolver (GitHub #152) only while it is paused, a blocked notice otherwise.
        let switchable = Arc::new(SwitchableCamera::new());
        let backend: Arc<dyn CameraSourceBackend> = Arc::new(SystemCameraSourceBackend::new(
            PathBuf::from("/run/soos/daemon.sock"),
            args.camera_device.clone(),
            PathBuf::from("/etc/soos/daemon.toml"),
        ));
        tracing::info!(
            "Camera source follows the soos-daemon state (IPC preview while active, direct V4L2 \
             while paused)"
        );
        camera_source = Some((Arc::clone(&switchable), backend));
        let cam: Arc<dyn CameraManager> = switchable;

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
            if let Some((switchable, backend)) = camera_source {
                app.attach_camera_source(switchable, backend);
            }
            Ok(Box::new(app))
        }),
    )?;

    Ok(())
}
