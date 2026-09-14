//! Filtered journal log retriever with automatic sensitive data redaction.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use crate::args::LogsArgs;
use crate::error::AdminCliError;
use crate::redact::RedactionFilter;

/// Fetches logs from systemd journal or file, applies redaction filtering, and writes to `out`.
///
/// # Errors
///
/// Returns `AdminCliError::Logs` or `AdminCliError::SocketIo` if reading fails.
pub fn fetch_and_filter_logs(
    args: &LogsArgs,
    filter: &dyn RedactionFilter,
    out: &mut dyn Write,
) -> Result<(), AdminCliError> {
    if let Some(file_path) = &args.file {
        read_and_filter_file(file_path, args.lines, filter, out)
    } else {
        read_and_filter_journalctl(args, filter, out)
    }
}

/// Reads lines from a file, retains the last `lines` entries, applies redaction, and writes to `out`.
fn read_and_filter_file(
    path: &std::path::Path,
    limit: usize,
    filter: &dyn RedactionFilter,
    out: &mut dyn Write,
) -> Result<(), AdminCliError> {
    let file = File::open(path).map_err(|e| {
        AdminCliError::Logs(format!("failed to open log file {}: {e}", path.display()))
    })?;
    let reader = BufReader::new(file);

    let all_lines: Result<Vec<String>, std::io::Error> = reader.lines().collect();
    let lines = all_lines.map_err(|e| {
        AdminCliError::Logs(format!("failed to read lines from {}: {e}", path.display()))
    })?;

    let start_idx = if limit > 0 && lines.len() > limit {
        lines.len().saturating_sub(limit)
    } else {
        0
    };

    if let Some(slice) = lines.get(start_idx..) {
        for line in slice {
            let redacted = filter.redact(line);
            writeln!(out, "{redacted}").map_err(AdminCliError::SocketIo)?;
        }
    }

    out.flush().map_err(AdminCliError::SocketIo)?;
    Ok(())
}

/// Invokes `journalctl` as a child process and streams redacted lines to `out`.
fn read_and_filter_journalctl(
    args: &LogsArgs,
    filter: &dyn RedactionFilter,
    out: &mut dyn Write,
) -> Result<(), AdminCliError> {
    let mut cmd = Command::new("journalctl");
    cmd.arg("-u").arg(&args.unit);
    cmd.arg("-n").arg(args.lines.to_string());
    cmd.arg("--no-pager");

    if args.follow {
        cmd.arg("-f");
    }

    if let Some(prio) = &args.priority {
        cmd.arg("-p").arg(prio);
    }

    if let Some(since) = &args.since {
        cmd.arg("--since").arg(since);
    }

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        AdminCliError::Logs(format!(
            "failed to invoke journalctl: {e}. Is systemd installed on this system?"
        ))
    })?;

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line_res in reader.lines() {
            let line = line_res.map_err(|e| {
                AdminCliError::Logs(format!("error reading journalctl output: {e}"))
            })?;
            let redacted = filter.redact(&line);
            writeln!(out, "{redacted}").map_err(AdminCliError::SocketIo)?;
        }
    }

    let status = child
        .wait()
        .map_err(|e| AdminCliError::Logs(format!("failed to wait for journalctl child: {e}")))?;

    if !status.success() {
        return Err(AdminCliError::Logs(format!(
            "journalctl exited with non-zero status: {status}"
        )));
    }

    out.flush().map_err(AdminCliError::SocketIo)?;
    Ok(())
}
