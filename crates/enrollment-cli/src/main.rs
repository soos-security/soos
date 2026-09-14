//! `soos-enroll` — Root enrollment and diagnostic tool CLI entry point.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI administrative tool requires stdout/stderr communication with the administrator"
)]

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use soos_biometric_store::{BiometricStore, MasterKey, DEFAULT_BIOMETRICS_DIR};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, V4lCameraManager};
use soos_enrollment_cli::args::{Cli, Commands, OutputFormat};
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::EnrollmentService;
use soos_inference_ort::{
    ModelRegistry, OrtEmbeddingExtractor, OrtFaceDetector, OrtLandmarkDetector, RegistryConfig,
};
use soos_protocol::Verdict;
use soos_vision::{VisionPipeline, VisionPipelineConfig};

const DEFAULT_KEY_PATH: &str = "/var/lib/soos/master.key";
const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
const DEFAULT_CAMERA_DEVICE: &str = "/dev/video0";

fn build_service(cli: &Cli) -> Result<EnrollmentService, EnrollmentCliError> {
    let key_path = cli
        .key_file
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_KEY_PATH));
    let key = MasterKey::load_or_create(&key_path)?;

    let bio_dir = cli
        .biometrics_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_BIOMETRICS_DIR));
    let store = Arc::new(BiometricStore::new(bio_dir, key)?);

    let device_path = cli
        .camera_device
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CAMERA_DEVICE));
    let camera_config = CameraConfigBuilder::new().device_path(device_path).build();
    let camera: Arc<dyn CameraManager> = Arc::new(V4lCameraManager::spawn(camera_config)?);

    let models_dir = cli
        .models_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR));
    let mut registry = ModelRegistry::new(RegistryConfig::new(models_dir))?;
    registry.verify_integrity()?;

    let det_session = registry.get_or_load_session("face_detector")?;
    let lm_session = registry.get_or_load_session("facial_landmarks")?;
    let emb_session = registry.get_or_load_session("face_embedding")?;

    let detector = Arc::new(OrtFaceDetector::new(det_session, 0.70, 0.40));
    let landmarks = Arc::new(OrtLandmarkDetector::new(lm_session));
    let extractor = Arc::new(OrtEmbeddingExtractor::new(emb_session));

    let pipeline_config = VisionPipelineConfig::default();
    let pipeline = Arc::new(VisionPipeline::new(
        detector,
        landmarks,
        extractor,
        pipeline_config,
    ));

    Ok(EnrollmentService::new(
        store,
        camera,
        pipeline,
        !cli.skip_root_check,
    ))
}

fn prompt_stdin(prompt: &str) -> bool {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        let trimmed = input.trim().to_lowercase();
        trimmed == "y" || trimmed == "yes"
    } else {
        false
    }
}

fn run() -> Result<(), EnrollmentCliError> {
    let cli = Cli::parse();
    let service = build_service(&cli)?;

    match &cli.command {
        Commands::Enroll(args) => {
            let outcome = service.enroll(args, |summary| {
                println!("\n=== Biometric Enrollment Summary ===");
                println!("Target UID:           {}", summary.uid);
                println!("Frames evaluated:     {}", summary.frames_evaluated);
                println!("Valid face frames:    {}", summary.valid_candidates);
                println!("Selected score:       {:.2}", summary.best_score);
                println!("Embedding dimension:  {}", summary.embedding_dim);
                println!("Model ID:             {}", summary.model_id);
                println!("Model Version:        {}", summary.model_version);
                println!("====================================\n");

                prompt_stdin("Save and encrypt this biometric template? [y/N]: ")
            })?;

            println!(
                "[OK] User {} enrolled successfully ({} frames evaluated, quality score: {:.2}).",
                outcome.uid, outcome.frames_evaluated, outcome.best_score
            );
        }

        Commands::Verify(args) => {
            let report = service.verify(args)?;

            println!("\n====================================================");
            println!("            SOOS BIOMETRIC VERIFICATION REPORT      ");
            println!("====================================================");
            println!("Target UID:          {}", report.uid);
            println!("Verdict:             {:?}", report.verdict);
            println!(
                "Match Score:         {:.4} (threshold: {:.4})",
                report.match_score, report.match_threshold
            );
            println!("Face Count:          {}", report.face_count);
            println!("PAD Anti-Spoof:      {}", report.pad_result);
            println!("----------------------------------------------------");
            println!("Latency Breakdown:");
            println!("  Camera Capture:    {:.2} ms", report.latency.capture_ms);
            println!("  Vision Pipeline:   {:.2} ms", report.latency.pipeline_ms);
            println!("  Cosine Match:      {:.2} ms", report.latency.matching_ms);
            println!("  Total Roundtrip:   {:.2} ms", report.latency.total_ms);
            println!("====================================================\n");

            if report.verdict != Verdict::Allow {
                std::process::exit(1);
            }
        }

        Commands::Delete(args) => {
            service.delete(args, |uid| {
                let prompt = format!(
                    "Are you sure you want to securely shred and delete template for UID {uid}? [y/N]: "
                );
                prompt_stdin(&prompt)
            })?;

            println!("[OK] Biometric template securely shredded and removed.");
        }

        Commands::List(args) => {
            let summaries = service.list(args)?;

            match args.format {
                OutputFormat::Table => {
                    if summaries.is_empty() {
                        println!("No biometric templates currently enrolled.");
                    } else {
                        println!(
                            "{:<8} {:<16} {:<16} {:<10} {:<12} {:<6}",
                            "UID", "USERNAME", "MODEL", "VERSION", "TIMESTAMP", "DIM"
                        );
                        println!("{:-<72}", "");
                        for s in summaries {
                            println!(
                                "{:<8} {:<16} {:<16} {:<10} {:<12} {:<6}",
                                s.uid,
                                s.username,
                                s.model_id,
                                s.model_version,
                                s.enrollment_timestamp,
                                s.embedding_dim
                            );
                        }
                    }
                }
                OutputFormat::Json => {
                    println!("[");
                    for (i, s) in summaries.iter().enumerate() {
                        let comma = if i.saturating_add(1) < summaries.len() {
                            ","
                        } else {
                            ""
                        };
                        println!(
                            "  {{\"uid\": {}, \"username\": \"{}\", \"model_id\": \"{}\", \"model_version\": \"{}\", \"enrollment_timestamp\": {}, \"embedding_dim\": {}}}{}",
                            s.uid, s.username, s.model_id, s.model_version, s.enrollment_timestamp, s.embedding_dim, comma
                        );
                    }
                    println!("]");
                }
            }
        }
    }

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("[ERROR] {err}");
        std::process::exit(1);
    }
}
