//! Core business logic and service orchestration for enrollment CLI.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
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
    cosine_similarity, PadInputModality, PipelineOutput, VisionError, VisionPipeline,
    VisionPipelineConfig,
};

use crate::args::{
    resolve_target_uid, validate_camera_device_path, validate_fhs_path, Cli, DebugVisionArgs,
    DeleteArgs, EnrollArgs, ImportArgs, ListArgs, VerifyArgs,
};
use crate::error::EnrollmentCliError;
use crate::html_report::{base64_encode, generate_html_report};
use crate::quality::{select_best_frame, CandidateEvaluation};

/// Default master key path for biometric encryption.
pub const DEFAULT_KEY_PATH: &str = "/var/lib/soos/master.key";

/// Default neural models directory containing `manifest.toml`.
pub const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";

/// Default stable camera device identifier per Criterion C4.
pub const DEFAULT_CAMERA_DEVICE: &str = soos_camera_v4l::AUTO_CAMERA_DEVICE;

/// Attested model registry ID for SCRFD 500M KPS face detector with 5-point landmarks.
pub const MODEL_ID_FACE_DETECTOR: &str = "scrfd_500m_kps";

/// Attested model registry ID for MiniFASNetV2 presentation attack detector.
pub const MODEL_ID_PAD: &str = "minifasnet_v2_pad";

/// Attested model registry ID of the 512D ArcFace embedding extractor (a tf2onnx ResNet34,
/// NHWC input; see `models/README.md`, GitHub #191).
pub const MODEL_ID_EMBEDDING: &str = "arcface_w600k_mbf";

/// Attested `models/manifest.toml` version of the embedding model recorded in enrolled
/// template metadata (GitHub #182 / STO-09). Pinned to the manifest by
/// `test_embedding_model_constants_match_attested_manifest`.
pub const EMBEDDING_MODEL_VERSION: &str = "2.0.0";

/// Set of all 3 neural model IDs required by the biometric vision pipeline.
pub const REQUIRED_MODEL_IDS: [&str; 3] =
    [MODEL_ID_FACE_DETECTOR, MODEL_ID_PAD, MODEL_ID_EMBEDDING];

/// Resolves the camera device path exactly like `soos-daemon` (GitHub #152).
///
/// Explicit sources are considered in order: the CLI argument (`cli_device`), then
/// `[pipeline] camera_device` from the daemon configuration at `config_path`. Sentinels
/// (`""`, `"auto"`, `"default"`, `/dev/v4l/by-id/default-camera`) are ignored. Without an explicit
/// device the shared resolver [`soos_camera_v4l::resolve_camera_device`] auto-detects the capture
/// node matching `[pipeline] sensor_preference` (default `PreferIr`) and returns its stable
/// `/dev/v4l/by-id/` alias (Criterion C4).
pub fn resolve_camera_device_from_config(
    cli_device: Option<PathBuf>,
    config_path: Option<&Path>,
) -> PathBuf {
    resolve_camera_device_from_config_with(
        cli_device,
        config_path,
        &soos_camera_v4l::SystemCameraEnumerator::default(),
    )
}

/// Enumerator-injectable form of [`resolve_camera_device_from_config`] (hermetic tests).
pub fn resolve_camera_device_from_config_with(
    cli_device: Option<PathBuf>,
    config_path: Option<&Path>,
    enumerator: &dyn soos_camera_v4l::CameraEnumerator,
) -> PathBuf {
    let (configured_device, sensor_preference) = read_daemon_camera_settings(config_path);

    let explicit = cli_device
        .filter(|p| !soos_camera_v4l::is_auto_camera_device(p))
        .or_else(|| configured_device.filter(|p| !soos_camera_v4l::is_auto_camera_device(p)));

    soos_camera_v4l::resolve_camera_device(explicit.as_deref(), sensor_preference, enumerator).path
}

/// Reads `[pipeline] camera_device` and `[pipeline] sensor_preference` from the daemon
/// configuration, using the daemon's shared vocabulary. A missing or unreadable file yields
/// `(None, PreferIr)`, the daemon defaults.
fn read_daemon_camera_settings(
    config_path: Option<&Path>,
) -> (Option<PathBuf>, soos_camera_v4l::SensorPreference) {
    let default = (None, soos_camera_v4l::SensorPreference::default());
    let Some(cfg) = config_path.filter(|p| p.is_file()) else {
        return default;
    };
    let Ok(content) = std::fs::read_to_string(cfg) else {
        return default;
    };
    let Ok(value) = content.parse::<toml::Value>() else {
        return default;
    };
    let pipeline = value.get("pipeline");
    let device = pipeline
        .and_then(|p| p.get("camera_device"))
        .and_then(|d| d.as_str())
        .map(PathBuf::from);
    let preference = pipeline
        .and_then(|p| p.get("sensor_preference"))
        .and_then(|s| s.as_str())
        .and_then(soos_camera_v4l::parse_sensor_preference)
        .unwrap_or_default();
    (device, preference)
}

/// Resolves the camera device path from an optional CLI argument only (no daemon configuration),
/// through the shared resolver (see [`resolve_camera_device_from_config`]).
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
    /// `true` when `--model-id`/`--model-version` differ from the loaded embedding
    /// model ([`MODEL_ID_EMBEDDING`] / [`EMBEDDING_MODEL_VERSION`]). The daemon refuses
    /// templates whose model identifier differs from its loaded extractor.
    pub model_overridden: bool,
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
    /// PAD outcome class: `PASSED`, `SPOOF(score=.., threshold=..)`,
    /// `IR_GATE_REJECTED(..)`, `NO_FACE`, `MULTIPLE_FACES` or `LOW_CONFIDENCE(..)`.
    pub pad_result: String,
    /// Liveness score returned by the PAD model, when PAD scored the frame (GitHub #236).
    pub pad_score: Option<f32>,
    /// Effective PAD threshold applied to the frame modality, when PAD ran (GitHub #236).
    pub pad_threshold: Option<f32>,
    pub latency: LatencyBreakdown,
}

impl DiagnosticVerificationReport {
    /// Operator-facing PAD line: the outcome class followed by the PAD score and
    /// threshold when the PAD model scored the frame and the class does not already
    /// carry them (GitHub #216, #236).
    #[must_use]
    pub fn pad_status(&self) -> String {
        match (self.pad_score, self.pad_threshold) {
            (Some(score), Some(threshold)) if !self.pad_result.contains("score=") => format!(
                "{} (score={score:.3}, threshold={threshold:.2})",
                self.pad_result
            ),
            _ => self.pad_result.clone(),
        }
    }
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
            model_overridden: args.model_id != MODEL_ID_EMBEDDING
                || args.model_version != EMBEDDING_MODEL_VERSION,
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
        let pad_threshold = pipeline
            .config()
            .effective_pad_threshold(PadInputModality::for_frame(&frame));

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
                    pad_score: Some(output.pad_result.score),
                    pad_threshold: Some(pad_threshold),
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
                    pad_score: None,
                    pad_threshold: None,
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
                    pad_score: None,
                    pad_threshold: None,
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
                    pad_score: None,
                    pad_threshold: None,
                    latency: LatencyBreakdown {
                        capture_ms,
                        pipeline_ms,
                        matching_ms: 0.0,
                        total_ms,
                    },
                })
            }
            // A presentation attack is a diagnostic Deny verdict with its PAD score,
            // never a generic error (GitHub #216 / PAD-11).
            Err(VisionError::PadFailed {
                score,
                threshold: applied,
            }) => {
                let total_ms = start_total.elapsed().as_secs_f64() * 1000.0;
                Ok(DiagnosticVerificationReport {
                    uid,
                    verdict: Verdict::Deny,
                    match_score: 0.0,
                    match_threshold: threshold,
                    face_count: 1,
                    pad_result: format!("SPOOF(score={score:.3}, threshold={applied:.2})"),
                    pad_score: Some(score),
                    pad_threshold: Some(applied),
                    latency: LatencyBreakdown {
                        capture_ms,
                        pipeline_ms,
                        matching_ms: 0.0,
                        total_ms,
                    },
                })
            }
            // The fail-closed IR gate rejected the crop before the PAD model ran.
            Err(VisionError::IrLivenessGateFailed { reason }) => {
                let total_ms = start_total.elapsed().as_secs_f64() * 1000.0;
                Ok(DiagnosticVerificationReport {
                    uid,
                    verdict: Verdict::Deny,
                    match_score: 0.0,
                    match_threshold: threshold,
                    face_count: 1,
                    pad_result: format!("IR_GATE_REJECTED({reason})"),
                    pad_score: None,
                    pad_threshold: Some(pad_threshold),
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

    /// Imports and encrypts an existing biometric template into the biometric store.
    ///
    /// `--file -` ([`IMPORT_STDIN_PATH`]) reads the template from standard input (the GUI
    /// path, GitHub #156); any other value is read through [`read_import_file`], which
    /// requires the file to be owned by `PKEXEC_UID` when running under `pkexec`.
    pub fn import(&self, args: &ImportArgs) -> Result<EnrollmentOutcome, EnrollmentCliError> {
        if args.file.as_os_str() == IMPORT_STDIN_PATH {
            return self.import_from_reader(args, std::io::stdin().lock());
        }
        check_privileges(self.require_root)?;
        let invoker = parse_pkexec_uid(std::env::var("PKEXEC_UID").ok().as_deref())?;
        let bytes = read_import_file(&args.file, invoker)?;
        self.store_imported(args, &bytes)
    }

    /// Imports a template read from `reader` (at most [`MAX_IMPORT_INPUT_BYTES`] bytes).
    ///
    /// # Errors
    ///
    /// [`EnrollmentCliError::InvalidImport`] when the input exceeds the bound or is not a
    /// 512-value array of finite floats (JSON) or a valid CBOR template; I/O and store errors
    /// otherwise. Nothing is stored on error.
    pub fn import_from_reader<R: Read>(
        &self,
        args: &ImportArgs,
        reader: R,
    ) -> Result<EnrollmentOutcome, EnrollmentCliError> {
        check_privileges(self.require_root)?;
        let bytes = read_import_input(reader)?;
        self.store_imported(args, &bytes)
    }

    fn store_imported(
        &self,
        args: &ImportArgs,
        bytes: &[u8],
    ) -> Result<EnrollmentOutcome, EnrollmentCliError> {
        let uid = resolve_target_uid(args.uid, args.username.as_deref())?;
        let (embedding, model_id, model_version) = parse_import_payload(bytes, args)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let template =
            BiometricTemplate::new(uid, model_id.clone(), model_version.clone(), now, embedding)?;

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

    /// Captures a frame, runs face detection, and writes an HTML debug report.
    ///
    /// The report is created atomically (`O_EXCL | O_NOFOLLOW`, mode `0600`) at the
    /// resolved output path; a pre-existing file or symbolic link is refused. The raw
    /// camera frame is biometric data and is embedded only when `args.embed_frame` is
    /// set; otherwise the report carries detection geometry only. Detector failures are
    /// propagated so that a broken model is never reported as "zero faces".
    pub fn debug_vision(&self, args: &DebugVisionArgs) -> Result<PathBuf, EnrollmentCliError> {
        check_privileges(self.require_root)?;

        let pipeline = self
            .pipeline
            .as_ref()
            .ok_or(EnrollmentCliError::PipelineNotInitialized)?;

        // Validate the destination before touching the camera so that a bad path
        // fails fast and no frame is captured for nothing.
        let out_path = resolve_debug_report_path(args.output.as_deref())?;

        let frame = self.acquire_frame()?;

        let rgb = Zeroizing::new(
            soos_vision::color::convert_to_rgb(
                &frame.data,
                frame.width,
                frame.height,
                frame.format,
            )
            .map_err(|e| EnrollmentCliError::Internal(e.to_string()))?,
        );

        let detections = pipeline
            .detector()
            .detect(&rgb, frame.width, frame.height)?;

        let embedded_frame = args
            .embed_frame
            .then(|| Zeroizing::new(base64_encode(&rgb)));
        let html = generate_html_report(
            frame.width,
            frame.height,
            embedded_frame.as_deref().map(String::as_str),
            &detections,
        );

        write_debug_report(&out_path, &html)?;

        Ok(out_path)
    }
}

/// Upper bound on any `import` input, from standard input or from a file (64 KiB).
///
/// A 512-value JSON float array or CBOR template is well below 16 KiB.
pub const MAX_IMPORT_INPUT_BYTES: usize = 64 * 1024;

/// `--file` value selecting standard input for `import` (GitHub #156).
pub const IMPORT_STDIN_PATH: &str = "-";

/// Embedding dimension accepted by `import` (ArcFace w600k MBF).
pub const IMPORT_EMBEDDING_DIM: usize = 512;

/// Parses the `PKEXEC_UID` environment value set by `pkexec` for the invoking user.
///
/// # Errors
///
/// [`EnrollmentCliError::InvalidImport`] when the value is present but not a decimal UID.
pub fn parse_pkexec_uid(value: Option<&str>) -> Result<Option<u32>, EnrollmentCliError> {
    match value {
        None => Ok(None),
        Some(raw) if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) => raw
            .parse::<u32>()
            .map(Some)
            .map_err(|_| EnrollmentCliError::InvalidImport("PKEXEC_UID is out of range".into())),
        Some(_) => Err(EnrollmentCliError::InvalidImport(
            "PKEXEC_UID is not a decimal user ID".into(),
        )),
    }
}

/// Reads at most [`MAX_IMPORT_INPUT_BYTES`] from `reader` into a zeroizing buffer.
///
/// The bound is enforced while reading (`Read::take`), so an endless input never grows the
/// buffer beyond the cap plus one byte.
fn read_import_input<R: Read>(reader: R) -> Result<Zeroizing<Vec<u8>>, EnrollmentCliError> {
    let cap = u64::try_from(MAX_IMPORT_INPUT_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut buf = Zeroizing::new(Vec::new());
    reader.take(cap).read_to_end(&mut buf)?;
    if buf.len() > MAX_IMPORT_INPUT_BYTES {
        return Err(EnrollmentCliError::InvalidImport(format!(
            "input exceeds {MAX_IMPORT_INPUT_BYTES} bytes"
        )));
    }
    Ok(buf)
}

/// Opens an `import` input file safely and reads it into a zeroizing buffer.
///
/// The file is opened with `O_NOFOLLOW | O_CLOEXEC` and checked on the open descriptor
/// (no TOCTOU window): it must be a regular file of at most [`MAX_IMPORT_INPUT_BYTES`]
/// bytes and, when `expected_owner` is set (the `PKEXEC_UID` of the invoking user), owned
/// by that user, so a `pkexec` caller cannot make root import someone else's file.
///
/// # Errors
///
/// [`EnrollmentCliError::InvalidImport`] on a symlink, a non-regular file, an oversized
/// file or an owner mismatch; [`EnrollmentCliError::Io`] when the file cannot be opened.
pub fn read_import_file(
    path: &Path,
    expected_owner: Option<u32>,
) -> Result<Zeroizing<Vec<u8>>, EnrollmentCliError> {
    use std::os::unix::fs::MetadataExt;

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| {
            if e.raw_os_error() == Some(libc::ELOOP) {
                EnrollmentCliError::InvalidImport("input file is a symbolic link".into())
            } else {
                EnrollmentCliError::Io(e)
            }
        })?;
    let meta = file.metadata()?;
    if !meta.file_type().is_file() {
        return Err(EnrollmentCliError::InvalidImport(
            "input is not a regular file".into(),
        ));
    }
    if meta.len() > u64::try_from(MAX_IMPORT_INPUT_BYTES).unwrap_or(u64::MAX) {
        return Err(EnrollmentCliError::InvalidImport(format!(
            "input file exceeds {MAX_IMPORT_INPUT_BYTES} bytes"
        )));
    }
    if let Some(owner) = expected_owner {
        if meta.uid() != owner {
            return Err(EnrollmentCliError::InvalidImport(
                "input file is not owned by the invoking user (PKEXEC_UID)".into(),
            ));
        }
    }
    read_import_input(file)
}

/// Decodes a JSON float array for `import` into a zeroizing buffer reserved once at
/// [`IMPORT_EMBEDDING_DIM`] values. The buffer never grows (so no reallocation leaves an
/// unzeroized copy): values beyond the dimension are counted but not stored. Returns the
/// stored values and the total number of array elements, or `None` when `bytes` is not
/// exactly one JSON array of numbers.
#[must_use]
pub fn decode_json_embedding(bytes: &[u8]) -> Option<(Zeroizing<Vec<f32>>, usize)> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let decoded = serde::Deserializer::deserialize_seq(&mut de, BoundedEmbeddingVisitor).ok()?;
    de.end().ok()?;
    Some(decoded)
}

/// Visitor of [`decode_json_embedding`].
struct BoundedEmbeddingVisitor;

impl<'de> serde::de::Visitor<'de> for BoundedEmbeddingVisitor {
    type Value = (Zeroizing<Vec<f32>>, usize);

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON array of floats")
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = Zeroizing::new(Vec::new());
        values
            .try_reserve_exact(IMPORT_EMBEDDING_DIM)
            .map_err(|_| <A::Error as serde::de::Error>::custom("embedding allocation failed"))?;
        let mut count = 0_usize;
        while let Some(value) = seq.next_element::<f32>()? {
            if values.len() < IMPORT_EMBEDDING_DIM {
                values.push(value);
            }
            count = count.saturating_add(1);
        }
        Ok((values, count))
    }
}

/// Decodes an `import` payload: a JSON array of finite floats, or a CBOR template.
///
/// Returns the zeroizing embedding with its model identifier and version (from `args` for
/// JSON, from the template for CBOR). Values are never echoed in errors.
fn parse_import_payload(
    bytes: &[u8],
    args: &ImportArgs,
) -> Result<(Zeroizing<Vec<f32>>, String, String), EnrollmentCliError> {
    let (embedding, count, model_id, model_version) =
        if let Some((parsed, count)) = decode_json_embedding(bytes) {
            (
                parsed,
                count,
                args.model_id.clone(),
                args.model_version.clone(),
            )
        } else if let Ok(template) = BiometricTemplate::from_cbor(bytes) {
            (
                template.embedding.clone(),
                template.embedding.len(),
                template.model_id.clone(),
                template.model_version.clone(),
            )
        } else {
            return Err(EnrollmentCliError::InvalidImport(
                "input is neither a JSON float array nor a CBOR BiometricTemplate".into(),
            ));
        };

    if count != IMPORT_EMBEDDING_DIM || embedding.len() != IMPORT_EMBEDDING_DIM {
        return Err(EnrollmentCliError::InvalidImport(format!(
            "invalid embedding dimension: expected {IMPORT_EMBEDDING_DIM}, found {count}"
        )));
    }
    if !embedding.iter().all(|v| v.is_finite()) {
        return Err(EnrollmentCliError::InvalidImport(
            "embedding contains non-finite values".into(),
        ));
    }
    Ok((embedding, model_id, model_version))
}

/// Default root-only directory receiving `debug-vision` reports.
pub const DEFAULT_DEBUG_REPORT_DIR: &str = "/var/lib/soos/debug";

/// Mode applied when `DEFAULT_DEBUG_REPORT_DIR` is created.
pub const DEBUG_REPORT_DIR_MODE: u32 = 0o700;

/// Mode of every report file written by `debug-vision`.
pub const DEBUG_REPORT_FILE_MODE: u32 = 0o600;

/// Ensures the report directory exists as a real directory.
///
/// A missing directory is created with `DEBUG_REPORT_DIR_MODE` (non-recursively, so
/// the parent must already exist). A pre-existing directory is accepted as is: its
/// mode is never rewritten. A symbolic link or a non-directory at `dir` is refused.
pub fn ensure_debug_report_dir(dir: &Path) -> Result<(), EnrollmentCliError> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => Err(EnrollmentCliError::DebugReportRefused {
            path: dir.to_path_buf(),
            reason: "report directory is a symbolic link".to_string(),
        }),
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(EnrollmentCliError::DebugReportRefused {
            path: dir.to_path_buf(),
            reason: "report directory path exists but is not a directory".to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new()
                .mode(DEBUG_REPORT_DIR_MODE)
                .create(dir)?;
            Ok(())
        }
        Err(e) => Err(EnrollmentCliError::Io(e)),
    }
}

/// Resolves the report path from the optional explicit `--output`.
///
/// An explicit path must be absolute, free of `..` components and under a permitted
/// FHS prefix (`validate_fhs_path`). Without an explicit path, a timestamped file name
/// under `DEFAULT_DEBUG_REPORT_DIR` is used and that root-only directory is prepared.
pub fn resolve_debug_report_path(output: Option<&Path>) -> Result<PathBuf, EnrollmentCliError> {
    if let Some(explicit) = output {
        let clean = validate_fhs_path(explicit)?;
        if clean.file_name().is_none() || clean.parent().is_none() {
            return Err(EnrollmentCliError::InvalidPath(format!(
                "Debug report path '{}' must name a file",
                clean.display()
            )));
        }
        return Ok(clean);
    }

    let dir = PathBuf::from(DEFAULT_DEBUG_REPORT_DIR);
    ensure_debug_report_dir(&dir)?;

    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(dir.join(format!(
        "soos-debug-{}-{}.html",
        unix_secs,
        std::process::id()
    )))
}

/// Writes the report to `path` with fail-closed filesystem semantics.
///
/// The parent must be a real directory (a symbolic link is refused), nothing may
/// already exist at `path` (regular file, symbolic link or anything else), and the file
/// is created with `O_CREAT | O_EXCL | O_NOFOLLOW` and mode `DEBUG_REPORT_FILE_MODE`
/// in a single `open(2)` call, so a concurrently planted symbolic link cannot be
/// followed and no pre-existing file is ever truncated.
pub fn write_debug_report(path: &Path, html: &str) -> Result<(), EnrollmentCliError> {
    let parent = path
        .parent()
        .ok_or_else(|| EnrollmentCliError::DebugReportRefused {
            path: path.to_path_buf(),
            reason: "output path has no parent directory".to_string(),
        })?;
    match std::fs::symlink_metadata(parent) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(EnrollmentCliError::DebugReportRefused {
                path: path.to_path_buf(),
                reason: "parent directory is a symbolic link".to_string(),
            });
        }
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return Err(EnrollmentCliError::DebugReportRefused {
                path: path.to_path_buf(),
                reason: "parent path is not a directory".to_string(),
            });
        }
        Err(e) => return Err(EnrollmentCliError::Io(e)),
    }

    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(EnrollmentCliError::DebugReportRefused {
                path: path.to_path_buf(),
                reason: "output path is a symbolic link".to_string(),
            });
        }
        Ok(_) => {
            return Err(EnrollmentCliError::DebugReportRefused {
                path: path.to_path_buf(),
                reason: "output path already exists; it is never overwritten".to_string(),
            });
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(EnrollmentCliError::Io(e)),
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(DEBUG_REPORT_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                EnrollmentCliError::DebugReportRefused {
                    path: path.to_path_buf(),
                    reason: "output path appeared concurrently; it is never overwritten"
                        .to_string(),
                }
            } else {
                EnrollmentCliError::Io(e)
            }
        })?;
    file.write_all(html.as_bytes())?;
    file.sync_all()?;
    Ok(())
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

    // Enrollment captures templates under exactly the authentication thresholds
    // (GitHub #251, #215): every value comes from the shared `VisionPipelineConfig`.
    let pipeline_config = VisionPipelineConfig::default();
    let detector = Arc::new(OrtScrfdDetector::new(
        det_session,
        pipeline_config.min_face_confidence,
        pipeline_config.nms_iou_threshold,
    )?);
    let pad = Arc::new(build_pad_detector(
        pad_session,
        pipeline_config.pad_threshold,
    ));
    let extractor = Arc::new(OrtEmbeddingExtractor::new(emb_session));

    let pipeline = Arc::new(VisionPipeline::new(
        detector,
        pad,
        extractor,
        pipeline_config,
    ));

    Ok(EnrollmentService::new(store, camera, pipeline, true))
}

/// Builds the CLI's production Presentation Attack Detector.
///
/// Sole PAD construction site of `soos-enroll`. The live class index is never overridden
/// here: `soos_inference_ort::pad::DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` is the single source
/// of truth shared with `soos-daemon` and `soos-gui` (GitHub #146, enforced by the
/// `test_no_pad_live_class_index_override_outside_tests` invariant).
pub fn build_pad_detector(
    pad_session: soos_inference_ort::SharedSession,
    liveness_threshold: f32,
) -> OrtPadDetector {
    OrtPadDetector::new(pad_session, liveness_threshold)
}

/// Builds an `EnrollmentService` (full service alias for backward compatibility).
pub fn build_service(cli: &Cli) -> Result<EnrollmentService, EnrollmentCliError> {
    build_full_service(cli)
}
