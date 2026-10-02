//! Filtered journal log retriever with automatic sensitive data redaction.

#[cfg(test)]
use std::io::Read;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::FileExt;
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
/// tail holds at most [`MAX_LOG_TAIL_LINES`] lines (`limit == 0` and larger limits are clamped
/// to it); each line keeps at most [`MAX_LOG_LINE_BYTES`] bytes and invalid UTF-8 is replaced,
/// never an error. The tail is read backwards from the size seen by `fstat` in bounded chunks
/// (GitHub #318), so the time spent follows the tail, not the size of the file.
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
    let len = file.metadata().map(|m| m.len()).map_err(|e| {
        AdminCliError::Logs(format!(
            "failed to inspect log file {}: {e}",
            path.display()
        ))
    })?;
    let tail = tail_lines(&file, len, capacity).map_err(|e| {
        AdminCliError::Logs(format!("failed to read lines from {}: {e}", path.display()))
    })?;

    for line in &tail {
        let redacted = filter.redact(line);
        writeln!(out, "{redacted}").map_err(AdminCliError::SocketIo)?;
    }

    out.flush().map_err(AdminCliError::SocketIo)?;
    Ok(())
}

/// Positional reads of a log file (a test seam: production reads the opened `File`).
trait PositionalRead {
    /// Reads at most `buf.len()` bytes at `offset`; `0` at end of file.
    fn read_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<usize>;
}

impl PositionalRead for std::fs::File {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
        FileExt::read_at(self, buf, offset)
    }
}

/// Size of one backward read of `logs --file` (GitHub #318).
const TAIL_CHUNK_BYTES: usize = 64 * 1024;

/// Fills `buf` from `offset`; a file that shrank under the read is an `UnexpectedEof` error.
fn read_exact_at(source: &impl PositionalRead, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    let mut done = 0_usize;
    while let Some(rest) = buf.get_mut(done..).filter(|rest| !rest.is_empty()) {
        let at = offset.saturating_add(u64::try_from(done).unwrap_or(u64::MAX));
        match source.read_at(rest, at) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "log file shrank while it was read",
                ))
            }
            Ok(n) => done = done.saturating_add(n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The last `capacity` lines of the first `len` bytes of `source`, oldest first, each bounded
/// to [`MAX_LOG_LINE_BYTES`] and decoded lossily (GitHub #312).
///
/// GitHub #318: the file is scanned backwards from `len` in chunks of [`TAIL_CHUNK_BYTES`]
/// until `capacity` line boundaries are found, so the cost follows the tail, not the size of
/// the file. Only the byte ranges of those lines are kept during the scan; each line is then
/// read from its start, at most [`MAX_LOG_LINE_BYTES`] bytes. Line semantics match the
/// forward reader: lines are separated by `\n`, a final `\n` ends the last line (it does not
/// start an empty one), and a final line without `\n` counts.
fn tail_lines(
    source: &impl PositionalRead,
    len: u64,
    capacity: usize,
) -> std::io::Result<Vec<String>> {
    if len == 0 || capacity == 0 {
        return Ok(Vec::new());
    }
    let mut last = [0_u8; 1];
    let mut pos = len.saturating_sub(1);
    read_exact_at(source, &mut last, pos)?;
    if last != *b"\n" {
        pos = len;
    }
    // Byte ranges `[start, end)` of the lines found, newest first, without their `\n`.
    let mut ranges: Vec<(u64, u64)> = Vec::with_capacity(capacity.min(1024));
    let mut line_end = pos;
    let mut chunk = vec![0_u8; TAIL_CHUNK_BYTES];
    'scan: while pos > 0 {
        let size = pos.min(u64::try_from(TAIL_CHUNK_BYTES).unwrap_or(u64::MAX));
        let start = pos.saturating_sub(size);
        let Some(buf) = chunk.get_mut(..usize::try_from(size).unwrap_or(TAIL_CHUNK_BYTES)) else {
            break;
        };
        read_exact_at(source, buf, start)?;
        for (index, byte) in buf.iter().enumerate().rev() {
            if *byte != b'\n' {
                continue;
            }
            let newline = start.saturating_add(u64::try_from(index).unwrap_or(u64::MAX));
            ranges.push((newline.saturating_add(1), line_end));
            line_end = newline;
            if ranges.len() >= capacity {
                break 'scan;
            }
        }
        pos = start;
    }
    if pos == 0 && ranges.len() < capacity {
        ranges.push((0, line_end));
    }

    let cap = u64::try_from(MAX_LOG_LINE_BYTES).unwrap_or(u64::MAX);
    let mut lines = Vec::with_capacity(ranges.len());
    for (start, end) in ranges.into_iter().rev() {
        let take = end.saturating_sub(start).min(cap);
        let mut bytes = vec![0_u8; usize::try_from(take).unwrap_or(MAX_LOG_LINE_BYTES)];
        read_exact_at(source, &mut bytes, start)?;
        lines.push(String::from_utf8_lossy(&bytes).into_owned());
    }
    Ok(lines)
}

/// Forward reader of GitHub #312, kept as the oracle of the backward tail tests.
///
/// Reads one line of at most [`MAX_LOG_LINE_BYTES`] bytes (without its `\n`), dropping the
/// rest of a longer line; `None` at end of file.
#[cfg(test)]
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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Unit tests of the tail reader seam use direct assertions and arithmetic"
)]
mod tail_tests {
    //! GitHub #318 (row VCO5): the default `logs --file` tail reads backwards from the end in
    //! bounded chunks, so its cost follows the tail, not the size of the file, and it returns
    //! exactly what the forward reader of GitHub #312 returned.

    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

    /// A virtual log of `lines` lines of 100 bytes (`"<10-digit index>xxx...x\n"`) that counts
    /// the bytes it serves, so no large file is ever written.
    struct VirtualLog {
        lines: u64,
        served: Cell<u64>,
    }

    impl VirtualLog {
        const LINE: u64 = 100;

        fn len(&self) -> u64 {
            self.lines * Self::LINE
        }

        fn byte(offset: u64) -> u8 {
            let (index, column) = (offset / Self::LINE, offset % Self::LINE);
            if column == Self::LINE - 1 {
                b'\n'
            } else if column < 10 {
                let digit = (index / 10_u64.pow(9 - column as u32)) % 10;
                b'0' + digit as u8
            } else {
                b'x'
            }
        }
    }

    impl PositionalRead for VirtualLog {
        fn read_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
            let left = self.len().saturating_sub(offset);
            let n = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            for (i, b) in buf[..n].iter_mut().enumerate() {
                *b = Self::byte(offset + i as u64);
            }
            self.served.set(self.served.get() + n as u64);
            Ok(n)
        }
    }

    struct Bytes(Vec<u8>);

    impl PositionalRead for Bytes {
        fn read_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
            let start = usize::try_from(offset)
                .unwrap_or(usize::MAX)
                .min(self.0.len());
            let n = buf.len().min(self.0.len() - start);
            buf[..n].copy_from_slice(&self.0[start..start + n]);
            Ok(n)
        }
    }

    /// The forward reader of GitHub #312, kept as the oracle.
    fn forward_reference(bytes: &[u8], capacity: usize) -> Vec<String> {
        let mut tail = VecDeque::new();
        let mut reader = BufReader::new(bytes);
        while let Some(line) = read_bounded_line(&mut reader).unwrap() {
            if tail.len() == capacity {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        tail.into()
    }

    /// VCO5: on a 64 MiB log, the default tail and the largest tail read only the tail region.
    #[test]
    fn test_vco_logs_tail_reads_only_the_tail_region() {
        let log = VirtualLog {
            lines: 64 * 1024 * 1024 / VirtualLog::LINE,
            served: Cell::new(0),
        };
        let last = log.lines - 1;

        let tail = tail_lines(&log, log.len(), 10).unwrap();
        assert_eq!(tail.len(), 10);
        assert!(tail[9].starts_with(&format!("{last:010}x")), "{}", tail[9]);
        assert!(
            tail[0].starts_with(&format!("{:010}x", last - 9)),
            "{}",
            tail[0]
        );
        assert!(
            log.served.get() <= 1024 * 1024,
            "the default tail must not read the whole file: {} bytes read",
            log.served.get()
        );

        log.served.set(0);
        let tail = tail_lines(&log, log.len(), MAX_LOG_TAIL_LINES).unwrap();
        assert_eq!(tail.len(), MAX_LOG_TAIL_LINES);
        let region = MAX_LOG_TAIL_LINES as u64 * VirtualLog::LINE;
        assert!(
            log.served.get() <= 2 * region + 2 * 1024 * 1024,
            "the largest tail reads about its own region: {} bytes read for {region}",
            log.served.get()
        );
    }

    fn lcg(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *state >> 33
    }

    /// Pseudo-random log bytes: short, empty, invalid UTF-8, multi-byte and over-long lines
    /// (beyond `MAX_LOG_LINE_BYTES` and across chunk boundaries).
    fn random_log(seed: u64, lines: usize) -> Vec<u8> {
        let mut state = seed;
        let mut out = Vec::new();
        for _ in 0..lines {
            let len = match lcg(&mut state) % 10 {
                0 => 0,
                1 => MAX_LOG_LINE_BYTES - 1 + (lcg(&mut state) % 3) as usize,
                2 => MAX_LOG_LINE_BYTES * 5 + (lcg(&mut state) % 70_000) as usize,
                _ => (lcg(&mut state) % 200) as usize,
            };
            for _ in 0..len {
                out.push(match lcg(&mut state) % 50 {
                    0 => 0xff,
                    1 => 0xc3,
                    2 => 0xa9,
                    _ => b'a' + (lcg(&mut state) % 26) as u8,
                });
            }
            out.push(b'\n');
        }
        if seed.is_multiple_of(2) {
            out.extend_from_slice(b"unterminated last line");
        }
        out
    }

    /// VCO5: the backward tail returns exactly the forward reader's lines (line cap, lossy
    /// UTF-8, empty lines, a final line without `\n`) for every requested size.
    #[test]
    fn test_vco_logs_backward_tail_matches_the_forward_reader() {
        let mut cases: Vec<Vec<u8>> = [
            &b""[..],
            b"\n",
            b"\n\n",
            b"a",
            b"a\n",
            b"a\n\nb",
            b"a\nb\n",
            b"\xffbad\n\xc3",
        ]
        .iter()
        .map(|c| c.to_vec())
        .collect();
        let mut exact = vec![b'y'; MAX_LOG_LINE_BYTES];
        exact.push(b'\n');
        exact.extend(vec![b'z'; MAX_LOG_LINE_BYTES + 1]);
        cases.push(exact);
        for seed in 1..=8 {
            cases.push(random_log(seed, 60));
        }
        for bytes in &cases {
            let source = Bytes(bytes.clone());
            for capacity in [1, 2, 3, 7, 40, MAX_LOG_TAIL_LINES] {
                assert_eq!(
                    tail_lines(&source, bytes.len() as u64, capacity).unwrap(),
                    forward_reference(bytes, capacity),
                    "capacity {capacity}, {} bytes",
                    bytes.len()
                );
            }
        }
    }
}
