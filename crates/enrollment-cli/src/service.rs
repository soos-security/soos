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

use soos_evidence_store::{
    EvidenceConfig, EvidenceStore, EvidenceStoreError, DEFAULT_EVIDENCE_DIR,
    DEFAULT_KEY_PATH as DEFAULT_EVIDENCE_KEY_PATH,
};

use crate::args::{
    resolve_target_uid, validate_camera_device_path, validate_fhs_path, Cli, DebugVisionArgs,
    DeleteArgs, EnrollArgs, ImportArgs, ListArgs, MigrateArgs, VerifyArgs,
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

/// Maximum wait, in milliseconds, for the camera to publish a frame newer than the previous
/// enrollment candidate (GitHub #228). A camera that stalls longer fails the enrollment
/// with [`soos_camera_v4l::CameraError::Starved`] instead of re-evaluating a stale frame.
pub const ENROLL_FRESH_FRAME_TIMEOUT_MS: u64 = 500;

/// Maximum wait, in milliseconds, for the first cached camera frame of an operation.
const FIRST_FRAME_TIMEOUT_MS: u64 = 2000;

/// Poll interval, in milliseconds, while waiting for a camera frame.
const FRAME_POLL_INTERVAL_MS: u64 = 10;

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
/// `/dev/v4l/by-id/` alias (Criterion C4). The notes about the configuration are dropped here;
/// use [`resolve_camera_device_from_config_reported`] to obtain them.
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
    resolve_camera_device_from_config_reported(cli_device, config_path, enumerator).path
}

/// Camera device chosen by [`resolve_camera_device_from_config_reported`] and the notes about
/// the daemon configuration it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDeviceChoice {
    /// The resolved device path.
    pub path: PathBuf,
    /// Why some or all of `daemon.toml` was not applied (unusable file, or a key ignored
    /// because of its type or value). Names keys, never their values. Empty when the
    /// configuration was applied as written or no configuration path was given.
    pub notes: Vec<String>,
}

/// Resolves the camera device like [`resolve_camera_device_from_config_with`] and also returns
/// the notes about the configuration (`soos-gui` logs them; `soos-enroll` prints
/// [`camera_config_notes`] before it opens the camera).
///
/// `daemon.toml` is read by the shared reader [`soos_camera_v4l::daemon_config`] (GitHub #289):
/// symbolic links are followed like `soos-daemon` follows them; a missing, unreadable, oversized,
/// malformed or non-regular file yields the soos-daemon defaults with one note; a key of the wrong type falls back to its own default
/// with a note naming it, and the other key still applies.
pub fn resolve_camera_device_from_config_reported(
    cli_device: Option<PathBuf>,
    config_path: Option<&Path>,
    enumerator: &dyn soos_camera_v4l::CameraEnumerator,
) -> CameraDeviceChoice {
    let (settings, notes) = config_path
        .map(read_camera_settings_with_notes)
        .unwrap_or_default();

    let explicit = cli_device
        .filter(|p| !soos_camera_v4l::is_auto_camera_device(p))
        .or_else(|| {
            settings
                .camera_device
                .filter(|p| !soos_camera_v4l::is_auto_camera_device(p))
        });
    let sensor_preference = settings.sensor_preference.unwrap_or_default();

    CameraDeviceChoice {
        path: soos_camera_v4l::resolve_camera_device(
            explicit.as_deref(),
            sensor_preference,
            enumerator,
        )
        .path,
        notes,
    }
}

/// Notes about the daemon configuration at `config_path`, as
/// [`resolve_camera_device_from_config_reported`] reports them: empty when the file applies as
/// written, otherwise one line per problem naming the file (sanitized for display) and the key,
/// never a value.
pub fn camera_config_notes(config_path: &Path) -> Vec<String> {
    read_camera_settings_with_notes(config_path).1
}

/// Reads the camera settings through the shared reader and turns its outcome into notes.
fn read_camera_settings_with_notes(
    path: &Path,
) -> (
    soos_camera_v4l::daemon_config::DaemonCameraSettings,
    Vec<String>,
) {
    // Same sanitizer as the `soos-admin camera list` note: control characters and
    // bidirectional overrides in the operator-supplied path become `?` (GitHub #289).
    let shown = soos_camera_v4l::diagnostics::sanitize_display_text(&path.to_string_lossy());
    match soos_camera_v4l::daemon_config::read_daemon_camera_config(path) {
        Ok(config) => {
            let notes = config
                .warnings()
                .into_iter()
                .map(|warning| format!("camera settings: {shown}: {warning}"))
                .collect();
            (config.settings, notes)
        }
        Err(err) => (
            soos_camera_v4l::daemon_config::DaemonCameraSettings::default(),
            vec![format!(
                "camera settings: {shown} {err}; using the soos-daemon defaults"
            )],
        ),
    }
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
    /// Candidates rejected by presentation attack detection (counted as invalid, GitHub #228).
    pub pad_rejections: usize,
    pub best_score: f32,
    pub embedding_dim: usize,
    pub model_id: String,
    pub model_version: String,
    /// `true` when `--model-id`/`--model-version` differ from the loaded embedding
    /// model ([`MODEL_ID_EMBEDDING`] / [`EMBEDDING_MODEL_VERSION`]). The daemon refuses
    /// templates whose model identifier differs from its loaded extractor.
    pub model_overridden: bool,
    /// `true` when the target user already has a template that confirming will replace
    /// (GitHub #233): the confirmation prompt must say so.
    pub already_enrolled: bool,
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
    /// `true` when an existing template of the same user was replaced.
    pub replaced_existing: bool,
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

/// Formats `soos-enroll list --format json` output with `serde_json` (GitHub #232).
///
/// Every string field (username, model identifier and version) is escaped, so the output is
/// valid JSON whatever the stored metadata contains.
#[must_use]
pub fn format_enrolled_json(summaries: &[EnrolledUserSummary]) -> String {
    serde_json::to_string_pretty(summaries)
        .unwrap_or_else(|_| "{\"error\": \"list serialization failed\"}".to_string())
}

/// Reason reported when no evidence key exists (evidence was never enabled on the host).
const EVIDENCE_SKIPPED_NO_KEY: &str =
    "no evidence key found: evidence storage was never enabled, nothing to migrate";

/// A file (or a whole store) the migration could not process; it was left untouched.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MigrationFailureSummary {
    /// Template UID, snapshot path, or the store name when the store itself failed.
    pub item: String,
    /// Error message (UIDs and paths only, never embedding values, frames or key material).
    pub error: String,
}

/// Migration counts of one store for `soos-enroll migrate` (GitHub #287).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct StoreMigrationSummary {
    /// Why the store was not examined (`None` when it was).
    pub skipped: Option<String>,
    /// Legacy files re-encrypted (in a dry run: that would be re-encrypted).
    pub migrated: usize,
    /// Files already in the AAD-bound v2 envelope, left untouched.
    pub already_current: usize,
    /// Number of files (or stores) that could not be processed.
    pub failed: usize,
    /// One entry per failure.
    pub failures: Vec<MigrationFailureSummary>,
}

impl StoreMigrationSummary {
    fn skipped(reason: &str) -> Self {
        Self {
            skipped: Some(reason.to_string()),
            ..Self::default()
        }
    }

    fn store_failed(item: &str, error: String) -> Self {
        Self {
            failed: 1,
            failures: vec![MigrationFailureSummary {
                item: item.to_string(),
                error,
            }],
            ..Self::default()
        }
    }
}

impl From<soos_biometric_store::TemplateMigrationReport> for StoreMigrationSummary {
    fn from(report: soos_biometric_store::TemplateMigrationReport) -> Self {
        let failures: Vec<MigrationFailureSummary> = report
            .failed
            .into_iter()
            .map(|f| MigrationFailureSummary {
                item: f.uid.to_string(),
                error: f.error,
            })
            .collect();
        Self {
            skipped: None,
            migrated: report.migrated.len(),
            already_current: report.already_current.len(),
            failed: failures.len(),
            failures,
        }
    }
}

impl From<soos_evidence_store::SnapshotMigrationReport> for StoreMigrationSummary {
    fn from(report: soos_evidence_store::SnapshotMigrationReport) -> Self {
        let failures: Vec<MigrationFailureSummary> = report
            .failed
            .into_iter()
            .map(|f| MigrationFailureSummary {
                item: f.path.display().to_string(),
                error: f.error,
            })
            .collect();
        Self {
            skipped: None,
            migrated: report.migrated.len(),
            already_current: report.already_current.len(),
            failed: failures.len(),
            failures,
        }
    }
}

/// Summary of `soos-enroll migrate` over both encrypted stores (GitHub #287).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MigrationSummary {
    /// `true` when nothing was written (`--dry-run`).
    pub dry_run: bool,
    /// Biometric templates.
    pub templates: StoreMigrationSummary,
    /// Evidence snapshots.
    pub evidence: StoreMigrationSummary,
}

impl MigrationSummary {
    /// `true` when at least one file or store could not be processed.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.templates.failed > 0 || self.evidence.failed > 0
    }
}

/// Formats `soos-enroll migrate --format json` output with `serde_json`.
#[must_use]
pub fn format_migration_json(summary: &MigrationSummary) -> String {
    serde_json::to_string_pretty(summary)
        .unwrap_or_else(|_| "{\"error\": \"migration summary serialization failed\"}".to_string())
}

/// Opens the evidence store for `soos-enroll migrate` without ever creating a key.
///
/// Returns `Ok(None)` when `key_path` does not exist (evidence was never enabled on this
/// host). An existing key is validated like the daemon validates it (regular file, no
/// symlink, owned by root or the effective UID, mode `0600`, 32 bytes).
pub fn open_evidence_store_for_migration(
    evidence_dir: &Path,
    key_path: &Path,
) -> Result<Option<EvidenceStore>, EnrollmentCliError> {
    match soos_evidence_store::MasterKey::load_existing(key_path) {
        Ok(key) => Ok(Some(EvidenceStore::new(
            EvidenceConfig::enabled_with_dir(evidence_dir.to_path_buf(), key_path.to_path_buf()),
            key,
        ))),
        Err(EvidenceStoreError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Reason reported when the template store cannot hold anything to migrate.
const TEMPLATES_SKIPPED_NO_KEY: &str =
    "no master key found: no template can exist, nothing to migrate (no key is created)";

/// Reason reported when the biometrics directory does not exist.
const TEMPLATES_SKIPPED_NO_DIR: &str =
    "no biometrics directory found: nothing to migrate (no directory is created)";

/// Opens the template store for `soos-enroll migrate` without ever creating the master key
/// or the biometrics directory (GitHub #287).
///
/// Returns `Ok(None)` when `key_path` or `bio_dir` does not exist: nothing can be migrated
/// and nothing is created (unlike `list` / `delete`, which open the key with
/// `load_or_create`). An existing key is validated by
/// [`soos_biometric_store::MasterKey::load_existing`] and an existing directory by
/// [`BiometricStore::open_existing`], which never creates it (GitHub #289).
pub fn open_template_store_for_migration(
    bio_dir: &Path,
    key_path: &Path,
) -> Result<Option<BiometricStore>, EnrollmentCliError> {
    Ok(open_template_store_checked(bio_dir, key_path)?.ok())
}

/// Like [`open_template_store_for_migration`], reporting why no store was opened.
fn open_template_store_checked(
    bio_dir: &Path,
    key_path: &Path,
) -> Result<Result<BiometricStore, &'static str>, EnrollmentCliError> {
    let key = match MasterKey::load_existing(key_path) {
        Ok(key) => key,
        Err(soos_biometric_store::BiometricStoreError::Io(e))
            if e.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(Err(TEMPLATES_SKIPPED_NO_KEY));
        }
        Err(e) => return Err(e.into()),
    };
    // Never `BiometricStore::new`, which would create a directory that vanished after a
    // separate existence check (GitHub #289): `open_existing` checks and opens in one step.
    match BiometricStore::open_existing(bio_dir, key) {
        Ok(store) => Ok(Ok(store)),
        Err(soos_biometric_store::BiometricStoreError::Io(e))
            if e.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(Err(TEMPLATES_SKIPPED_NO_DIR))
        }
        Err(e) => Err(e.into()),
    }
}

/// Resolves the template paths of `soos-enroll migrate` (`--biometrics-dir`, `--key-file`,
/// FHS-validated) and opens the store through [`open_template_store_for_migration`]; the
/// error side carries the skip reason. Never creates a key or a directory.
pub fn build_templates_for_migration(
    cli: &Cli,
) -> Result<Result<BiometricStore, &'static str>, EnrollmentCliError> {
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
    open_template_store_checked(&bio_dir, &key_path)
}

/// Runs `soos-enroll migrate` over the template store and the evidence store (GitHub #287).
///
/// Requires root when `require_root` is set. A `None` store is reported as skipped (no key
/// or no directory: nothing exists to migrate). See [`EnrollmentService::migrate`].
pub fn run_migration(
    args: &MigrateArgs,
    templates: Option<&BiometricStore>,
    evidence: Option<&EvidenceStore>,
    require_root: bool,
) -> Result<MigrationSummary, EnrollmentCliError> {
    run_migration_with_reasons(
        args,
        templates.ok_or(TEMPLATES_SKIPPED_NO_KEY),
        evidence,
        require_root,
    )
}

/// [`run_migration`] with an explicit skip reason for the template store.
pub fn run_migration_with_reasons(
    args: &MigrateArgs,
    templates: Result<&BiometricStore, &str>,
    evidence: Option<&EvidenceStore>,
    require_root: bool,
) -> Result<MigrationSummary, EnrollmentCliError> {
    check_privileges(require_root)?;

    let templates = match templates {
        Ok(store) => match store.migrate_legacy_templates(args.dry_run) {
            Ok(report) => StoreMigrationSummary::from(report),
            Err(e) => StoreMigrationSummary::store_failed("biometric store", e.to_string()),
        },
        Err(reason) => StoreMigrationSummary::skipped(reason),
    };
    let evidence = match evidence {
        Some(store) => match store.migrate_legacy_snapshots(args.dry_run) {
            Ok(report) => StoreMigrationSummary::from(report),
            Err(e) => StoreMigrationSummary::store_failed("evidence store", e.to_string()),
        },
        None => StoreMigrationSummary::skipped(EVIDENCE_SKIPPED_NO_KEY),
    };

    Ok(MigrationSummary {
        dry_run: args.dry_run,
        templates,
        evidence,
    })
}

/// Resolves the evidence paths of `soos-enroll migrate` (defaults
/// `/var/lib/soos/evidence` and `/var/lib/soos/evidence.key`, FHS-validated) and opens the
/// evidence store through [`open_evidence_store_for_migration`].
pub fn build_evidence_for_migration(
    args: &MigrateArgs,
) -> Result<Option<EvidenceStore>, EnrollmentCliError> {
    let raw_dir = args
        .evidence_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_EVIDENCE_DIR));
    let raw_key = args
        .evidence_key_file
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_EVIDENCE_KEY_PATH));
    let dir = validate_fhs_path(&raw_dir)?;
    let key = validate_fhs_path(&raw_key)?;
    open_evidence_store_for_migration(&dir, &key)
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
        self.acquire_frame_after(None)
    }

    /// Acquires a camera frame, newer than `previous_sequence` when one is given.
    ///
    /// Without a previous sequence the first cached frame is returned (waiting up to 2 s for
    /// the camera to publish one). With a previous sequence, cached frames whose `sequence`
    /// is not strictly greater are skipped for at most [`ENROLL_FRESH_FRAME_TIMEOUT_MS`], so
    /// that every multi-frame enrollment candidate is a distinct capture (GitHub #228).
    ///
    /// # Errors
    ///
    /// [`soos_camera_v4l::CameraError::Starved`] when no (fresh) frame arrives in time.
    fn acquire_frame_after(
        &self,
        previous_sequence: Option<u64>,
    ) -> Result<Arc<Frame>, EnrollmentCliError> {
        let camera = self
            .camera
            .as_ref()
            .ok_or(EnrollmentCliError::CameraNotInitialized)?;
        camera.notify_activity();
        let budget = match previous_sequence {
            None => Duration::from_millis(FIRST_FRAME_TIMEOUT_MS),
            Some(_) => Duration::from_millis(ENROLL_FRESH_FRAME_TIMEOUT_MS),
        };
        let start = Instant::now();
        loop {
            if let Some(frame) = camera.latest_frame() {
                if previous_sequence.is_none_or(|prev| frame.sequence > prev) {
                    return Ok(frame);
                }
            }
            if start.elapsed() >= budget {
                return Err(EnrollmentCliError::Camera(
                    soos_camera_v4l::CameraError::Starved,
                ));
            }
            std::thread::sleep(Duration::from_millis(FRAME_POLL_INTERVAL_MS));
        }
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
        let mut last_sequence: Option<u64> = None;
        let mut pad_rejections = 0usize;
        let mut last_pad_error: Option<VisionError> = None;

        for idx in 0..frames_to_capture {
            let frame = self.acquire_frame_after(last_sequence)?;
            last_sequence = Some(frame.sequence);
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
                // Pre-PAD quality gate (GitHub #218): an unusable capture is an invalid
                // candidate, like a frame without a face, never an abort (GitHub #285).
                Err(VisionError::FaceTooSmall { .. } | VisionError::FaceBlurred { .. }) => {
                    candidates.push(CandidateEvaluation {
                        frame_idx: idx,
                        detections: vec![],
                    });
                    outputs.push(None);
                }
                Err(
                    e @ (VisionError::PadFailed { .. } | VisionError::IrLivenessGateFailed { .. }),
                ) => {
                    // A presentation-attack rejection is an invalid candidate, not an abort
                    // (GitHub #228); it can never become the enrolled frame.
                    pad_rejections = pad_rejections.saturating_add(1);
                    last_pad_error = Some(e);
                    candidates.push(CandidateEvaluation {
                        frame_idx: idx,
                        detections: vec![],
                    });
                    outputs.push(None);
                }
                Err(e) => return Err(e.into()),
            }
        }

        let best = match select_best_frame(&candidates, pipeline.config().min_face_confidence) {
            Ok(best) => best,
            // Every candidate failed and at least one was a presentation attack: report the
            // PAD rejection rather than a generic quality failure (fail closed, nothing stored).
            Err(e) => return Err(last_pad_error.map_or(e, EnrollmentCliError::from)),
        };
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
            pad_rejections,
            best_score: best.score,
            embedding_dim,
            model_id: args.model_id.clone(),
            model_version: args.model_version.clone(),
            model_overridden: args.model_id != MODEL_ID_EMBEDDING
                || args.model_version != EMBEDDING_MODEL_VERSION,
            already_enrolled,
        };

        // `--yes` is the explicit consent to replace an existing template; otherwise the
        // prompt shows `summary.already_enrolled` before the administrator confirms.
        if !args.yes && !prompt_confirm(&summary) {
            return Err(EnrollmentCliError::Cancelled);
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
            replaced_existing: already_enrolled,
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
            // The pre-PAD quality gate (GitHub #218) rejected the face before PAD ran: a
            // diagnostic Deny naming the gate, never a generic error (GitHub #285).
            Err(VisionError::FaceTooSmall {
                width_px,
                min_width_px,
            }) => {
                let total_ms = start_total.elapsed().as_secs_f64() * 1000.0;
                Ok(DiagnosticVerificationReport {
                    uid,
                    verdict: Verdict::Deny,
                    match_score: 0.0,
                    match_threshold: threshold,
                    face_count: 1,
                    pad_result: format!(
                        "FACE_TOO_SMALL(width={width_px:.1}px, min={min_width_px:.1}px)"
                    ),
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
            Err(VisionError::FaceBlurred {
                sharpness,
                min_sharpness,
            }) => {
                let total_ms = start_total.elapsed().as_secs_f64() * 1000.0;
                Ok(DiagnosticVerificationReport {
                    uid,
                    verdict: Verdict::Deny,
                    match_score: 0.0,
                    match_threshold: threshold,
                    face_count: 1,
                    pad_result: format!(
                        "FACE_BLURRED(sharpness={sharpness:.1}, min={min_sharpness:.1})"
                    ),
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
            if let Some(template) = self.store.get_metadata(uid)? {
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

    /// Re-encrypts every legacy (v1) template and, when `evidence` is given, every legacy
    /// evidence snapshot with the AAD-bound v2 envelope (GitHub #287, owner decision
    /// 2026-10-01). With `--dry-run` nothing is written and the counts report what would be
    /// migrated.
    ///
    /// Requires root. The writes go through the stores' own atomic write paths; v2 files are
    /// never rewritten, so a second run reports zero migrated files. A per-file failure is
    /// counted and listed, never aborts the other files and leaves the file untouched; a
    /// failure of a whole store (for example an unlistable or symlinked directory) is reported
    /// the same way so the other store is still migrated. The summary carries counts, UIDs,
    /// paths and error messages only.
    pub fn migrate(
        &self,
        args: &MigrateArgs,
        evidence: Option<&EvidenceStore>,
    ) -> Result<MigrationSummary, EnrollmentCliError> {
        run_migration(args, Some(&self.store), evidence, self.require_root)
    }

    /// Imports and encrypts an existing biometric template into the biometric store.
    ///
    /// Equivalent to [`Self::import_with_overwrite`] without `--yes`: a file import onto an
    /// already enrolled user fails with [`EnrollmentCliError::AlreadyEnrolled`].
    pub fn import(&self, args: &ImportArgs) -> Result<EnrollmentOutcome, EnrollmentCliError> {
        self.import_with_overwrite(args, false)
    }

    /// Imports a template, replacing an existing one only when `allow_overwrite` (`--yes`).
    ///
    /// `--file -` ([`IMPORT_STDIN_PATH`]) reads the template from standard input (the GUI
    /// path, GitHub #156) through [`Self::import_with_overwrite_from_reader`]; any other value
    /// is read through [`read_import_file`], which requires the file to be owned by
    /// `PKEXEC_UID` when running under `pkexec`.
    ///
    /// # Errors
    ///
    /// [`EnrollmentCliError::AlreadyEnrolled`] when the import (file or stdin) targets an
    /// enrolled user and `allow_overwrite` is `false`; the input is not read and nothing is
    /// replaced (GitHub #237).
    pub fn import_with_overwrite(
        &self,
        args: &ImportArgs,
        allow_overwrite: bool,
    ) -> Result<EnrollmentOutcome, EnrollmentCliError> {
        if args.file.as_os_str() == IMPORT_STDIN_PATH {
            return self.import_with_overwrite_from_reader(
                args,
                allow_overwrite,
                std::io::stdin().lock(),
            );
        }
        self.refuse_unconfirmed_overwrite(args, allow_overwrite)?;
        let invoker = parse_pkexec_uid(std::env::var("PKEXEC_UID").ok().as_deref())?;
        let bytes = read_import_file(&args.file, invoker)?;
        self.store_imported(args, &bytes, allow_overwrite)
    }

    /// Imports a template read from `reader`, replacing an existing one only when
    /// `allow_overwrite` (`--yes`).
    ///
    /// The overwrite check runs before `reader` is touched, so a refused stdin import never
    /// consumes its input (GitHub #237).
    ///
    /// # Errors
    ///
    /// [`EnrollmentCliError::AlreadyEnrolled`] when the target user is enrolled and
    /// `allow_overwrite` is `false`; otherwise as [`Self::import_from_reader`].
    pub fn import_with_overwrite_from_reader<R: Read>(
        &self,
        args: &ImportArgs,
        allow_overwrite: bool,
        reader: R,
    ) -> Result<EnrollmentOutcome, EnrollmentCliError> {
        self.refuse_unconfirmed_overwrite(args, allow_overwrite)?;
        let bytes = read_import_input(reader)?;
        self.store_imported(args, &bytes, allow_overwrite)
    }

    /// Checks privileges and refuses to replace an enrolled template without `--yes`.
    fn refuse_unconfirmed_overwrite(
        &self,
        args: &ImportArgs,
        allow_overwrite: bool,
    ) -> Result<(), EnrollmentCliError> {
        check_privileges(self.require_root)?;
        let uid = resolve_target_uid(args.uid, args.username.as_deref())?;
        if !allow_overwrite && self.store.exists(uid)? {
            return Err(EnrollmentCliError::AlreadyEnrolled(uid));
        }
        Ok(())
    }

    /// Imports a template read from `reader` (at most [`MAX_IMPORT_INPUT_BYTES`] bytes).
    ///
    /// Library entry point without overwrite gating: an existing template is replaced and the
    /// replacement is reported in [`EnrollmentOutcome::replaced_existing`]. The CLI never calls
    /// it directly; `soos-enroll import --file -` goes through
    /// [`Self::import_with_overwrite_from_reader`], which requires `--yes` to replace.
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
        self.store_imported(args, &bytes, true)
    }

    /// Parses and stores an imported template. Without `allow_overwrite` the write goes
    /// through [`BiometricStore::enroll_if_absent`] (GitHub #291): the duplicate check and
    /// the write run under one store lock, so a template enrolled after the early
    /// [`Self::refuse_unconfirmed_overwrite`] check is never replaced and the import fails
    /// with [`EnrollmentCliError::AlreadyEnrolled`] instead.
    fn store_imported(
        &self,
        args: &ImportArgs,
        bytes: &[u8],
        allow_overwrite: bool,
    ) -> Result<EnrollmentOutcome, EnrollmentCliError> {
        let uid = resolve_target_uid(args.uid, args.username.as_deref())?;
        let (embedding, model_id, model_version) = parse_import_payload(bytes, args)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let template =
            BiometricTemplate::new(uid, model_id.clone(), model_version.clone(), now, embedding)?;

        let replaced_existing = if allow_overwrite {
            let existed = self.store.exists(uid)?;
            self.store.enroll(&template)?;
            existed
        } else {
            match self.store.enroll_if_absent(&template) {
                Ok(()) => false,
                Err(soos_biometric_store::BiometricStoreError::AlreadyEnrolled(uid)) => {
                    return Err(EnrollmentCliError::AlreadyEnrolled(uid));
                }
                Err(e) => return Err(e.into()),
            }
        };

        Ok(EnrollmentOutcome {
            uid,
            frames_evaluated: 1,
            best_score: 1.0,
            embedding_dim: 512,
            model_id,
            model_version,
            replaced_existing,
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
/// Required for biometric capture and comparison commands (`enroll`, `verify`). The notes
/// about `daemon.toml` are dropped; `soos-enroll` uses [`build_full_service_with_notes`].
pub fn build_full_service(cli: &Cli) -> Result<EnrollmentService, EnrollmentCliError> {
    build_full_service_with_notes(
        cli,
        Path::new(soos_camera_v4l::daemon_config::DEFAULT_DAEMON_CONFIG_PATH),
        &mut |_| {},
    )
}

/// [`build_full_service`] with the camera settings read from `config_path`, handing the notes
/// of that single read to `report_notes` (GitHub #291).
///
/// The camera device is resolved once through [`resolve_camera_device_from_config_reported`];
/// `report_notes` is called exactly once with that call's notes (an empty slice when the file
/// applies as written), before the device path is validated and before the camera or any
/// model is opened. The printed notes therefore always describe the configuration that is
/// applied, never a second read of a file that may have changed in between.
pub fn build_full_service_with_notes(
    cli: &Cli,
    config_path: &Path,
    report_notes: &mut dyn FnMut(&[String]),
) -> Result<EnrollmentService, EnrollmentCliError> {
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

    let choice = resolve_camera_device_from_config_reported(
        cli.camera_device.clone(),
        Some(config_path),
        &soos_camera_v4l::SystemCameraEnumerator::default(),
    );
    report_notes(&choice.notes);
    let device_path = validate_camera_device_path(&choice.path)?;

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
