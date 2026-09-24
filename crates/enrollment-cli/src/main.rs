//! `soos-enroll` — Root enrollment and diagnostic tool CLI entry point.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI administrative tool requires stdout/stderr communication with the administrator"
)]

use std::io::{self, Write};

use clap::Parser;

use soos_enrollment_cli::args::{Cli, Commands, OutputFormat};
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::{build_full_service, build_store_only, check_privileges};
use soos_protocol::Verdict;

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
    check_privileges(true)?;
    let cli = Cli::parse();

    match &cli.command {
        Commands::Enroll(args) => {
            let service = build_full_service(&cli)?;
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
            let service = build_full_service(&cli)?;
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
            let service = build_store_only(&cli)?;
            service.delete(args, |uid| {
                let prompt = format!(
                    "Are you sure you want to securely shred and delete template for UID {uid}? [y/N]: "
                );
                prompt_stdin(&prompt)
            })?;

            println!("[OK] Biometric template securely shredded and removed.");
        }

        Commands::List(args) => {
            let service = build_store_only(&cli)?;
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

        Commands::DebugVision => {
            let service = build_full_service(&cli)?;
            let path = service.debug_vision()?;
            println!("\n[OK] Visual debugging report generated successfully.");
            println!("Open the following file in your web browser to visualize the detection:");
            println!("  file://{}\n", path);
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
