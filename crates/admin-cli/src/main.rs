//! `soos-admin` — Non-biometric diagnostic and administrative CLI.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Administrative CLI diagnostic tool requires stdout/stderr communication"
)]

use std::io;
use std::path::PathBuf;

use clap::Parser;
use soos_admin_cli::args::{CameraAction, Cli, Commands, OutputFormat, DEFAULT_SOCKET_PATH};
use soos_admin_cli::camera::{collect_list_report, probe_report, CameraEnvironment};
use soos_admin_cli::error::AdminCliError;
use soos_admin_cli::logs::fetch_and_filter_logs;
use soos_admin_cli::redact::DefaultRedactionFilter;
use soos_admin_cli::status::query_status;
use soos_admin_cli::test_pam::{effective_timeout_ms, simulate_pam_auth};
use soos_admin_cli::user::add_user_to_soos_group;
use soos_camera_v4l::diagnostics::SystemV4lDeviceProbe;
use soos_protocol::types::Verdict;

fn run() -> Result<(), AdminCliError> {
    let cli = Cli::parse();
    let socket_path: PathBuf = cli
        .socket_path
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_PATH));

    match &cli.command {
        Commands::Status(args) => {
            let report = query_status(&socket_path, &args.unit)?;

            match cli.format {
                OutputFormat::Table => {
                    print!("{}", report.format_table());
                }
                OutputFormat::Json => {
                    println!("{}", report.to_json());
                }
            }

            if !report.is_healthy {
                std::process::exit(1);
            }
        }

        Commands::TestPam(args) => {
            let uid = args.uid.unwrap_or_else(|| nix::unistd::getuid().as_raw());
            let effective_ms = effective_timeout_ms(args.timeout_ms);
            if effective_ms != args.timeout_ms {
                eprintln!(
                    "[WARN] --timeout-ms {} is outside the PAM range; using {effective_ms} ms \
                     (pam_soos.so clamps timeout_ms the same way).",
                    args.timeout_ms
                );
            }
            let report = simulate_pam_auth(&socket_path, uid, &args.service, args.timeout_ms)?;

            match cli.format {
                OutputFormat::Table => {
                    print!("{}", report.format_table());
                }
                OutputFormat::Json => {
                    println!("{}", report.to_json());
                }
            }

            if report.verdict != Verdict::Allow {
                std::process::exit(1);
            }
        }

        Commands::Logs(args) => {
            let filter = DefaultRedactionFilter;
            let mut stdout = io::stdout();
            fetch_and_filter_logs(args, &filter, &mut stdout)?;
        }

        Commands::AddUser(args) => {
            add_user_to_soos_group(&args.username)?;
            println!("[OK] User '{}' added to 'soos' group.", args.username);
        }

        Commands::Camera(args) => {
            let env = CameraEnvironment::default();
            let probe = SystemV4lDeviceProbe;
            let (text, code) = match &args.action {
                CameraAction::List(list) => {
                    let report = collect_list_report(
                        &env,
                        &probe,
                        list.sensor_preference,
                        list.device.as_deref(),
                    );
                    let json = list.json || cli.format == OutputFormat::Json;
                    let text = if json {
                        report.to_json()
                    } else {
                        report.format_table()
                    };
                    (text, report.exit_code())
                }
                CameraAction::Probe(probe_args) => {
                    let report = probe_report(&env, &probe, &probe_args.device);
                    let json = probe_args.json || cli.format == OutputFormat::Json;
                    let text = if json {
                        report.to_json()
                    } else {
                        report.format_table()
                    };
                    (text, report.exit_code())
                }
            };
            println!("{}", text.trim_end());
            if code != 0 {
                std::process::exit(code);
            }
        }

        Commands::Gdm(args) => {
            if args.action == soos_admin_cli::GdmAction::Enable {
                // Never point GDM at a module that is not installed (GitHub #177).
                let dirs: Vec<std::path::PathBuf> = match &args.pam_module_dir {
                    Some(dir) => vec![dir.clone()],
                    None => soos_admin_cli::gdm::DEFAULT_PAM_MODULE_DIRS
                        .iter()
                        .map(std::path::PathBuf::from)
                        .collect(),
                };
                if soos_admin_cli::gdm::find_pam_module(&dirs).is_none() {
                    return Err(AdminCliError::GdmConfig(format!(
                        "{} not found in {}; install soos first or pass --pam-module-dir",
                        soos_admin_cli::gdm::PAM_MODULE_FILE,
                        dirs.iter()
                            .map(|d| d.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
            let status = soos_admin_cli::gdm::configure_gdm(
                &args.action,
                &args.pam_file,
                &args.disable_file,
            )?;
            match cli.format {
                OutputFormat::Table => {
                    println!("GDM Biometric Integration Status:");
                    println!("  PAM Service File:  {}", status.pam_file.display());
                    println!(
                        "  Installed in PAM:  {}",
                        if status.installed { "Yes" } else { "No" }
                    );
                    println!("  Disable Flag File: {}", status.disable_file.display());
                    println!(
                        "  Status:            {}",
                        if status.enabled {
                            "Enabled"
                        } else {
                            "Disabled"
                        }
                    );
                }
                OutputFormat::Json => {
                    let json = serde_json::to_string_pretty(&status).map_err(|e| {
                        AdminCliError::GdmConfig(format!("Failed to serialize JSON: {e}"))
                    })?;
                    println!("{json}");
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
