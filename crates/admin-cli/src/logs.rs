//! Filtered journal log retriever with automatic sensitive data redaction.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use crate::args::LogsArgs;
use crate::error::AdminCliError;
use crate::redact::RedactionFilter;

/// Upper bound of the tail `logs --file` keeps (GitHub #312, STO-NEW-8); `-n 0` and larger
/// requests are clamped to it.
pub const MAX_LOG_TAIL_LINES: usize = 10_000;

/// Upper bound of one line read by `logs --file`; the rest of a longer line is dropped.
pub const MAX_LOG_LINE_BYTES: usize = 16 * 1024;

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
///
/// Bounded (GitHub #312, STO-NEW-8): the file is opened `O_NONBLOCK | O_CLOEXEC` and must be
/// a regular file on the open descriptor (a FIFO or device is refused without blocking); the
/// tail is kept in a ring buffer of at most [`MAX_LOG_TAIL_LINES`] lines (`limit == 0` and
/// larger limits are clamped to it); each line keeps at most [`MAX_LOG_LINE_BYTES`] bytes and
/// invalid UTF-8 is replaced, never an error.
fn read_and_filter_file(
    path: &std::path::Path,
    limit: usize,
    filter: &dyn RedactionFilter,
    out: &mut dyn Write,
) -> Result<(), AdminCliError> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| {
            AdminCliError::Logs(format!("failed to open log file {}: {e}", path.display()))
        })?;
    let is_regular = file.metadata().map(|m| m.is_file()).map_err(|e| {
        AdminCliError::Logs(format!(
            "failed to inspect log file {}: {e}",
            path.display()
        ))
    })?;
    if !is_regular {
        return Err(AdminCliError::Logs(format!(
            "log file {} is not a regular file",
            path.display()
        )));
    }

    let capacity = if limit == 0 {
        MAX_LOG_TAIL_LINES
    } else {
        limit.min(MAX_LOG_TAIL_LINES)
    };
    let mut tail: VecDeque<String> = VecDeque::with_capacity(capacity);
    let mut reader = BufReader::new(file);
    while let Some(line) = read_bounded_line(&mut reader).map_err(|e| {
        AdminCliError::Logs(format!("failed to read lines from {}: {e}", path.display()))
    })? {
        if tail.len() == capacity {
            tail.pop_front();
        }
        tail.push_back(line);
    }

    for line in &tail {
        let redacted = filter.redact(line);
        writeln!(out, "{redacted}").map_err(AdminCliError::SocketIo)?;
    }

    out.flush().map_err(AdminCliError::SocketIo)?;
    Ok(())
}

/// Reads one line of at most [`MAX_LOG_LINE_BYTES`] bytes (without its `\n`), dropping the
/// rest of a longer line; `None` at end of file.
fn read_bounded_line(reader: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let bound = u64::try_from(MAX_LOG_LINE_BYTES).unwrap_or(u64::MAX);
    let read = reader.by_ref().take(bound).read_until(b'\n', &mut buf)?;
    if read == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    } else if buf.len() >= MAX_LOG_LINE_BYTES {
        // Discard the remainder of the oversized line, chunk by chunk.
        loop {
            let chunk = reader.fill_buf()?;
            if chunk.is_empty() {
                break;
            }
            match chunk.iter().position(|&b| b == b'\n') {
                Some(pos) => {
                    reader.consume(pos.saturating_add(1));
                    break;
                }
                None => {
                    let len = chunk.len();
                    reader.consume(len);
                }
            }
        }
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
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
