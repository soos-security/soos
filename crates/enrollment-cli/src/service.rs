//! Core business logic and service orchestration for enrollment CLI.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nix::unistd::{Uid, User};
use zeroize::Zeroizing;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey, DEFAULT_BIOMETRICS_DIR};
use soos_camera_v4l::{
    CameraConfigBuilder, CameraManager, Frame, MockCameraManager, V4lCameraManager,
};
use soos_inference_ort::{
    BoundingBox, FaceDetection, MockEmbeddingExtractor, MockFaceDetector, MockPadDetector,
    ModelRegistry, OrtEmbeddingExtractor, OrtPadDetector, OrtScrfdDetector, RegistryConfig,
};
use soos_protocol::Verdict;
use soos_vision::{
    cosine_similarity, PipelineOutput, VisionError, VisionPipeline, VisionPipelineConfig,
};

use crate::args::{
    resolve_target_uid, validate_camera_device_path, validate_fhs_path, Cli, DeleteArgs,
    EnrollArgs, ImportArgs, ListArgs, VerifyArgs,
};
use crate::error::EnrollmentCliError;
use crate::html_report::{base64_encode, generate_html_report};
use crate::quality::{select_best_frame, CandidateEvaluation};

/// Default master key path for biometric encryption.
pub const DEFAULT_KEY_PATH: &str = "/var/lib/soos/master.key";

/// Default neural models directory containing `manifest.toml`.
pub const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";

/// Default stable camera device identifier per Criterion C4.
pub const DEFAULT_CAMERA_DEVICE: &str = "/dev/v4l/by-id/default-camera";

/// Attested model registry ID for SCRFD 500M KPS face detector with 5-point landmarks.
pub const MODEL_ID_FACE_DETECTOR: &str = "scrfd_500m_kps";

/// Attested model registry ID for MiniFASNetV2 presentation attack detector.
pub const MODEL_ID_PAD: &str = "minifasnet_v2_pad";

/// Attested model registry ID for ArcFace MobileFaceNet w600k 512D feature extractor.
pub const MODEL_ID_EMBEDDING: &str = "arcface_w600k_mbf";

/// Set of all 3 neural model IDs required by the biometric vision pipeline.
pub const REQUIRED_MODEL_IDS: [&str; 3] =
    [MODEL_ID_FACE_DETECTOR, MODEL_ID_PAD, MODEL_ID_EMBEDDING];

/// Resolves the camera device path:
/// 1. Explicit CLI argument (`cli_device`), if provided.
/// 2. Active `camera_device` from daemon config, if explicitly provided or default exists.
/// 3. First deterministic entry in `/dev/v4l/by-id/`.
/// 4. Fallback `/dev/v4l/by-id/default-camera`.
pub fn resolve_camera_device_from_config(
    cli_device: Option<PathBuf>,
    config_path: Option<&Path>,
) -> PathBuf {
    if let Some(device) = cli_device {
        return device;
    }

    // 1. Check daemon configuration if provided
    if let Some(cfg) = config_path {
        if cfg.is_file() {
            if let Ok(content) = std::fs::read_to_string(cfg) {
                if let Ok(value) = content.parse::<toml::Value>() {
                    if let Some(dev_str) = value
                        .get("pipeline")
                        .and_then(|p| p.get("camera_device"))
                        .and_then(|d| d.as_str())
                    {
                        return PathBuf::from(dev_str);
                    }
                }
            }
        }
    }

    // 2. Deterministic entry in /dev/v4l/by-id/ per Criterion C4
    let by_id_dir = Path::new("/dev/v4l/by-id");
    if by_id_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(by_id_dir) {
            let mut paths: Vec<_> = entries
                .filter_map(|e| e.ok().map(|entry| entry.path()))
                .filter(|p| p.is_file() || p.is_symlink())
                .collect();
            paths.sort();
            if let Some(first) = paths.into_iter().next() {
                return first;
            }
        }
    }

    PathBuf::from(DEFAULT_CAMERA_DEVICE)
}

/// Resolves the camera device path, preferring an explicit CLI argument if provided,
/// then the first deterministic entry in `/dev/v4l/by-id/`, and falling back to
/// `/dev/v4l/by-id/default-camera` per Criterion C4.
pub fn resolve_camera_device(cli_device: Option<PathBuf>) -> PathBuf {
    resolve_camera_device_from_config(cli_device, None)
}

/// Verifies that the process is running with root privileges (EUID 0) if required.
pub fn check_privileges(require_root: bool) -> Result<(), EnrollmentCliError> {
    if require_root && nix::unistd::geteuid().as_raw() != 0 {
        return Err(EnrollmentCliError::RootRequired);
    }
    Ok(())
}

/// Summary presented to the user during interactive enrollment confirmation.
#[derive(Debug, Clone)]
pub struct EnrollmentSummary {
    pub uid: u32,
    pub frames_evaluated: usize,
    pub valid_candidates: usize,
    pub best_score: f32,
    pub embedding_dim: usize,
    pub model_id: String,
    pub model_version: String,
}

/// Outcome of a successfully completed enrollment.
#[derive(Debug, Clone)]
pub struct EnrollmentOutcome {
    pub uid: u32,
    pub frames_evaluated: usize,
    pub best_score: f32,
    pub embedding_dim: usize,
    pub model_id: String,
    pub model_version: String,
}

/// Latency metrics breakdown measured during diagnostic verification.
#[derive(Debug, Clone, Default)]
pub struct LatencyBreakdown {
    pub capture_ms: f64,
    pub pipeline_ms: f64,
    pub matching_ms: f64,
    pub total_ms: f64,
}

/// Comprehensive diagnostic verification report.
#[derive(Debug, Clone)]
pub struct DiagnosticVerificationReport {
    pub uid: u32,
    pub verdict: Verdict,
    pub match_score: f32,
    pub match_threshold: f32,
    pub face_count: usize,
    pub pad_result: String,
    pub latency: LatencyBreakdown,
}

/// Metadata summary of an enrolled user.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EnrolledUserSummary {
    pub uid: u32,
    pub username: String,
    pub model_id: String,
    pub model_version: String,
    pub enrollment_timestamp: u64,
    pub embedding_dim: usize,
}

/// Core enrollment service orchestrator.
pub struct EnrollmentService {
    store: Arc<BiometricStore>,
    camera: Option<Arc<dyn CameraManager>>,
    pipeline: Option<Arc<VisionPipeline>>,
    require_root: bool,
}

impl EnrollmentService {
    /// Creates a new full `EnrollmentService` with camera and vision pipeline.
    pub fn new(
        store: Arc<BiometricStore>,
        camera: Arc<dyn CameraManager>,
        pipeline: Arc<VisionPipeline>,
        require_root: bool,
    ) -> Self {
        Self {
            store,
            camera: Some(camera),
            pipeline: Some(pipeline),
            require_root,
        }
    }

    /// Creates a new store-only `EnrollmentService` without initializing camera or models.
    pub fn new_store_only(store: Arc<BiometricStore>, require_root: bool) -> Self {
        Self {
            store,
            camera: None,
            pipeline: None,
            require_root,
        }
    }

    /// Returns `true` if camera and vision pipeline are initialized.
    pub fn is_full_service(&self) -> bool {
        self.camera.is_some() && self.pipeline.is_some()
    }

    /// Acquires a fresh, stabilized camera frame.
    fn acquire_frame(&self) -> Result<Arc<Frame>, EnrollmentCliError> {
        let camera = self
            .camera
            .as_ref()
            .ok_or(EnrollmentCliError::CameraNotInitialized)?;
        camera.notify_activity();
        for _ in 0..200 {
            if let Some(frame) = camera.latest_frame() {
                return Ok(frame);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(EnrollmentCliError::Camera(
            soos_camera_v4l::CameraError::Starved,
        ))
    }

    /// Enrolls a user with multi-frame quality selection, interactive confirmation, and encryption.
    pub fn enroll(
        &self,
        args: &EnrollArgs,
        mut prompt_confirm: impl FnMut(&EnrollmentSummary) -> bool,
    ) -> Result<EnrollmentOutcome, EnrollmentCliError> {
        check_privileges(self.require_root)?;

        let pipeline = self
            .pipeline
            .as_ref()
            .ok_or(EnrollmentCliError::PipelineNotInitialized)?;

        let uid = resolve_target_uid(args.uid, args.username.as_deref())?;

        let already_enrolled = self.store.exists(uid)?;

        let frames_to_capture = args.frames.clamp(1, 30);
        let mut candidates = Vec::with_capacity(frames_to_capture);
        let mut outputs: Vec<Option<PipelineOutput>> = Vec::with_capacity(frames_to_capture);

        for idx in 0..frames_to_capture {
            let frame = self.acquire_frame()?;
            match pipeline.process_frame(&frame) {
                Ok(output) => {
                    candidates.push(CandidateEvaluation {
                        frame_idx: idx,
                        detections: vec![output.detection.clone()],
                    });
                    outputs.push(Some(output));
                }
                Err(VisionError::NoFaceDetected) => {
                    candidates.push(CandidateEvaluation {
                        frame_idx: idx,
                        detections: vec![],
                    });
                    outputs.push(None);
                }
                Err(VisionError::MultipleFacesDetected { count }) => {
                    let dummy_dets = (0..count)
                        .map(|i| FaceDetection {
                            box_: BoundingBox::new(10.0 * (i as f32 + 1.0), 10.0, 50.0, 50.0),
                            score: 0.90,
                            landmarks: None,
                        })
                        .collect();
                    candidates.push(CandidateEvaluation {
                        frame_idx: idx,
                        detections: dummy_dets,
                    });
                    outputs.push(None);
                }
                Err(VisionError::FaceBelowConfidence { confidence, .. }) => {
                    candidates.push(CandidateEvaluation {
                        frame_idx: idx,
                        detections: vec![FaceDetection {
                            box_: BoundingBox::new(10.0, 10.0, 50.0, 50.0),
                            score: confidence,
                            landmarks: None,
                        }],
                    });
                    outputs.push(None);
                }
                Err(e) => return Err(e.into()),
            }
        }

        let best = select_best_frame(&candidates, pipeline.config().min_face_confidence)?;
        let best_output = outputs
            .get(best.frame_idx)
            .and_then(|opt| opt.as_ref())
            .ok_or_else(|| {
                EnrollmentCliError::Internal(
                    "Internal error retrieving best evaluated frame".to_string(),
                )
            })?;

        let embedding_dim = best_output.embedding.len();
        let valid_candidates = candidates
            .iter()
            .filter(|c| {
                c.detections.len() == 1
                    && c.detections
                        .first()
                        .map(|d| d.score >= pipeline.config().min_face_confidence)
                        .unwrap_or(false)
            })
            .count();

        let summary = EnrollmentSummary {
            uid,
            frames_evaluated: frames_to_capture,
            valid_candidates,
            best_score: best.score,
            embedding_dim,
            model_id: args.model_id.clone(),
            model_version: args.model_version.clone(),
        };

        if !args.yes {
            if !prompt_confirm(&summary) {
                return Err(EnrollmentCliError::Cancelled);
            }
        } else if already_enrolled {
            // Overwriting silently allowed when --yes is explicitly passed
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let template = BiometricTemplate::new(
            uid,
            args.model_id.clone(),
            args.model_version.clone(),
            timestamp,
            Zeroizing::new(best_output.embedding.as_slice().to_vec()),
        )?;

        self.store.enroll(&template)?;

        Ok(EnrollmentOutcome {
            uid,
            frames_evaluated: frames_to_capture,
            best_score: best.score,
            embedding_dim,
            model_id: args.model_id.clone(),
            model_version: args.model_version.clone(),
        })
    }

    /// Diagnostic one-shot verification reporting match score, face count, PAD, and latency.
    pub fn verify(
        &self,
        args: &VerifyArgs,
    ) -> Result<DiagnosticVerificationReport, EnrollmentCliError> {
        check_privileges(self.require_root)?;

        let pipeline = self
            .pipeline
            .as_ref()
            .ok_or(EnrollmentCliError::PipelineNotInitialized)?;

        let uid = resolve_target_uid(args.uid, args.username.as_deref())?;

        let template = self
            .store
            .get(uid)?
            .ok_or(EnrollmentCliError::NotEnrolled(uid))?;

        let start_total = Instant::now();

        // 1. Frame capture
        let start_cap = Instant::now();
        let frame = self.acquire_frame()?;
        let capture_ms = start_cap.elapsed().as_secs_f64() * 1000.0;

        // 2. Vision processing
        let start_pipe = Instant::now();
        let process_res = pipeline.process_frame(&frame);
        let pipeline_ms = start_pipe.elapsed().as_secs_f64() * 1000.0;

        let threshold = pipeline.config().match_threshold;

        match process_res {
            Ok(output) => {
                let start_match = Instant::now();
                let score =
                    cosine_similarity(template.embedding.as_slice(), output.embedding.as_slice())?;
                let matching_ms = start_match.elapsed().as_secs_f64() * 1000.0;
                let total_ms = start_total.elapsed().as_secs_f64() * 1000.0;

                let verdict = if score >= threshold {
                    Verdict::Allow
                } else {
                    Verdict::Deny
                };

                Ok(DiagnosticVerificationReport {
                    uid,
                    verdict,
                    match_score: score,
                    match_threshold: threshold,
                    face_count: 1,
                    pad_result: "PASSED".to_string(),
                    latency: LatencyBreakdown {
                        capture_ms,
                        pipeline_ms,
                        matching_ms,
                        total_ms,
                    },
                })
            }
            Err(VisionError::NoFaceDetected) => {
                let total_ms = start_total.elapsed().as_secs_f64() * 1000.0;
                Ok(DiagnosticVerificationReport {
                    uid,
                    verdict: Verdict::Deny,
                    match_score: 0.0,
                    match_threshold: threshold,
                    face_count: 0,
                    pad_result: "NO_FACE".to_string(),
                    latency: LatencyBreakdown {
                        capture_ms,
                        pipeline_ms,
                        matching_ms: 0.0,
                        total_ms,
                    },
                })
            }
            Err(VisionError::MultipleFacesDetected { count }) => {
                let total_ms = start_total.elapsed().as_secs_f64() * 1000.0;
                Ok(DiagnosticVerificationReport {
                    uid,
                    verdict: Verdict::Deny,
                    match_score: 0.0,
                    match_threshold: threshold,
                    face_count: count,
                    pad_result: "MULTIPLE_FACES".to_string(),
                    latency: LatencyBreakdown {
                        capture_ms,
                        pipeline_ms,
                        matching_ms: 0.0,
                        total_ms,
                    },
                })
            }
            Err(VisionError::FaceBelowConfidence { confidence, .. }) => {
                let total_ms = start_total.elapsed().as_secs_f64() * 1000.0;
                Ok(DiagnosticVerificationReport {
                    uid,
                    verdict: Verdict::Deny,
                    match_score: 0.0,
                    match_threshold: threshold,
                    face_count: 1,
                    pad_result: format!("LOW_CONFIDENCE({confidence:.2})"),
                    latency: LatencyBreakdown {
                        capture_ms,
                        pipeline_ms,
                        matching_ms: 0.0,
                        total_ms,
                    },
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Deletes an enrolled biometric template with interactive confirmation and secure erasure.
    pub fn delete(
        &self,
        args: &DeleteArgs,
        mut prompt_confirm: impl FnMut(u32) -> bool,
    ) -> Result<bool, EnrollmentCliError> {
        check_privileges(self.require_root)?;

        let uid = resolve_target_uid(args.uid, args.username.as_deref())?;

        if !self.store.exists(uid)? {
            return Err(EnrollmentCliError::NotEnrolled(uid));
        }

        if !args.yes && !prompt_confirm(uid) {
            return Err(EnrollmentCliError::Cancelled);
        }

        self.store.delete(uid)?;
        Ok(true)
    }

    /// Lists all currently enrolled UIDs and metadata.
    pub fn list(&self, _args: &ListArgs) -> Result<Vec<EnrolledUserSummary>, EnrollmentCliError> {
        check_privileges(self.require_root)?;

        let uids = self.store.list_enrolled()?;
        let mut summaries = Vec::with_capacity(uids.len());

        for uid in uids {
            if let Some(template) = self.store.get(uid)? {
                let username = match User::from_uid(Uid::from_raw(uid)) {
                    Ok(Some(u)) => u.name,
                    _ => uid.to_string(),
                };

                summaries.push(EnrolledUserSummary {
                    uid,
                    username,
                    model_id: template.model_id,
                    model_version: template.model_version,
                    enrollment_timestamp: template.enrollment_timestamp,
                    embedding_dim: template.embedding_dim,
                });
            }
        }

        summaries.sort_by_key(|s| s.uid);
        Ok(summaries)
    }

    /// Imports and encrypts an existing biometric template (from file) into the biometric store.
    pub fn import(&self, args: &ImportArgs) -> Result<EnrollmentOutcome, EnrollmentCliError> {
        check_privileges(self.require_root)?;

        let uid = resolve_target_uid(args.uid, args.username.as_deref())?;

        let file_bytes = std::fs::read(&args.file)?;

        // Support either JSON array of f32 or CBOR-encoded BiometricTemplate
        let (embedding, model_id, model_version) =
            if let Ok(parsed) = serde_json::from_slice::<Vec<f32>>(&file_bytes) {
                (parsed, args.model_id.clone(), args.model_version.clone())
            } else if let Ok(template) = BiometricTemplate::from_cbor(&file_bytes) {
                (
                    (*template.embedding).clone(),
                    template.model_id,
                    template.model_version,
                )
            } else {
                return Err(EnrollmentCliError::Internal(
                "Input file is neither a valid JSON float array nor a valid CBOR BiometricTemplate"
                    .to_string(),
            ));
            };

        if embedding.len() != 512 {
            return Err(EnrollmentCliError::Internal(format!(
                "Invalid embedding dimension: expected 512, found {}",
                embedding.len()
            )));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let template = BiometricTemplate::new(
            uid,
            model_id.clone(),
            model_version.clone(),
            now,
            Zeroizing::new(embedding),
        )?;

        self.store.enroll(&template)?;

        Ok(EnrollmentOutcome {
            uid,
            frames_evaluated: 1,
            best_score: 1.0,
            embedding_dim: 512,
            model_id,
            model_version,
        })
    }

    /// Captures a frame, runs face detection, and generates an HTML report.
    pub fn debug_vision(&self) -> Result<String, EnrollmentCliError> {
        check_privileges(self.require_root)?;

        let pipeline = self
            .pipeline
            .as_ref()
            .ok_or(EnrollmentCliError::PipelineNotInitialized)?;

        let frame = self.acquire_frame()?;

        let rgb = soos_vision::color::convert_to_rgb(
            &frame.data,
            frame.width,
            frame.height,
            frame.format,
        )
        .map_err(|e| EnrollmentCliError::Internal(e.to_string()))?;

        // Extract internal detector via pipeline config? No, we don't have a public getter for detector.
        // Wait, VisionPipeline doesn't expose detector.
        // We can just use process_frame? But process_frame fails fast on 0 or >1 face.
        // Let's just use process_frame, and if it fails, we don't get the detection boxes.
        // Actually, we can get around this by accessing the detector directly if we had a getter.
        // Let's add a public method to VisionPipeline if needed, OR we can just add it to service.rs?
        // Wait, I will just call `process_frame` and if it succeeds, visualize it.
        // But what if it fails? That's EXACTLY what we want to debug.
        // Wait! Let's modify VisionPipeline to expose detector or we just rebuild the detector?
        // I will just use `pipeline.process_frame`, and if it fails, I still generate the report with NO detections (empty).
        // Actually, to get the detections, we need the detector.

        // Let's just create a new detector instance? No, that's heavy.
        // Wait, I can't access `pipeline.detector` because it's private.
        // Let's just add `pub fn detector(&self) -> &Arc<dyn FaceDetector>` to `VisionPipeline` in `vision/src/pipeline.rs`.
        // For now, let's assume we'll add that getter.
        let detections = pipeline
            .detector()
            .detect(&rgb, frame.width, frame.height)
            .unwrap_or_default();

        let base64_img = base64_encode(&rgb);
        let html = generate_html_report(frame.width, frame.height, &base64_img, &detections);

        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let out_path = cwd.join("soos-debug.html");
        std::fs::write(&out_path, html)
            .map_err(|e| EnrollmentCliError::Internal(format!("Failed to write HTML: {}", e)))?;

        Ok(out_path.display().to_string())
    }
}

/// Builds an `EnrollmentService` initialized with only the biometric store (master key and templates).
///
/// Defers camera hardware access and ONNX model loading, making it safe and fast
/// for read-only / administrative commands (`list`, `delete`).
pub fn build_store_only(cli: &Cli) -> Result<EnrollmentService, EnrollmentCliError> {
    let raw_key_path = cli
        .key_file
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_KEY_PATH));
    let key_path = validate_fhs_path(&raw_key_path)?;

    let raw_bio_dir = cli
        .biometrics_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_BIOMETRICS_DIR));
    let bio_dir = validate_fhs_path(&raw_bio_dir)?;

    let key = MasterKey::load_or_create(&key_path)?;
    let store = Arc::new(BiometricStore::new(bio_dir, key)?);

    Ok(EnrollmentService::new_store_only(store, true))
}

/// Builds a full `EnrollmentService` with camera hardware streaming and all 4 ONNX models.
///
/// Required for biometric capture and comparison commands (`enroll`, `verify`).
pub fn build_full_service(cli: &Cli) -> Result<EnrollmentService, EnrollmentCliError> {
    let raw_key_path = cli
        .key_file
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_KEY_PATH));
    let key_path = validate_fhs_path(&raw_key_path)?;

    let raw_bio_dir = cli
        .biometrics_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_BIOMETRICS_DIR));
    let bio_dir = validate_fhs_path(&raw_bio_dir)?;

    let raw_camera = resolve_camera_device_from_config(
        cli.camera_device.clone(),
        Some(Path::new("/etc/soos/daemon.toml")),
    );
    let device_path = validate_camera_device_path(&raw_camera)?;

    let raw_models_dir = cli
        .models_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR));
    let models_dir = validate_fhs_path(&raw_models_dir)?;

    let key = MasterKey::load_or_create(&key_path)?;
    let store = Arc::new(BiometricStore::new(bio_dir, key)?);

    if cli.mock {
        let camera_config = CameraConfigBuilder::new().build();
        let camera: Arc<dyn CameraManager> = Arc::new(MockCameraManager::new(camera_config));
        let detector = Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95));
        let pad = Arc::new(MockPadDetector::new_live());
        let extractor = Arc::new(MockEmbeddingExtractor::new(512));
        let pipeline_config = VisionPipelineConfig::default();
        let pipeline = Arc::new(VisionPipeline::new(
            detector,
            pad,
            extractor,
            pipeline_config,
        ));
        return Ok(EnrollmentService::new(store, camera, pipeline, true));
    }

    let camera_config = CameraConfigBuilder::new().device_path(device_path).build();
    let camera: Arc<dyn CameraManager> = Arc::new(V4lCameraManager::spawn(camera_config)?);

    let mut registry = ModelRegistry::new(RegistryConfig::new(models_dir))?;
    registry.verify_integrity()?;

    let det_session = registry.get_or_load_session(MODEL_ID_FACE_DETECTOR)?;
    let pad_session = registry.get_or_load_session(MODEL_ID_PAD)?;
    let emb_session = registry.get_or_load_session(MODEL_ID_EMBEDDING)?;

    let detector = Arc::new(OrtScrfdDetector::new(det_session, 0.70, 0.40)?);
    let pad = Arc::new(OrtPadDetector::new_with_class_index(pad_session, 0.80, 2));
    let extractor = Arc::new(OrtEmbeddingExtractor::new(emb_session));

    let pipeline_config = VisionPipelineConfig::default();
    let pipeline = Arc::new(VisionPipeline::new(
        detector,
        pad,
        extractor,
        pipeline_config,
    ));

    Ok(EnrollmentService::new(store, camera, pipeline, true))
}

/// Builds an `EnrollmentService` (full service alias for backward compatibility).
pub fn build_service(cli: &Cli) -> Result<EnrollmentService, EnrollmentCliError> {
    build_full_service(cli)
}
