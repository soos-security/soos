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
use soos_admin_cli::args::{Cli, Commands, OutputFormat, DEFAULT_SOCKET_PATH};
use soos_admin_cli::error::AdminCliError;
use soos_admin_cli::logs::fetch_and_filter_logs;
use soos_admin_cli::redact::DefaultRedactionFilter;
use soos_admin_cli::status::query_status;
use soos_admin_cli::test_pam::simulate_pam_auth;
use soos_admin_cli::user::add_user_to_soos_group;
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
    }

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("[ERROR] {err}");
        std::process::exit(1);
    }
}
