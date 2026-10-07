//! Contract tests of the journal reader's process side (ADR 2026-10-06 "Failed-Password
//! Alerts in `soos-remote` From the System Journal", architect spec
//! `AI/architect_spec_remote_auth_alerts.md` §2.5, A-14, tests 42–43; matrix RMC46, RMC52).
//!
//! Test 42 drives the bounded line reader over an in-memory `tokio::io::duplex` pipe.
//! Test 43 spawns the real `/usr/bin/journalctl` (read-only, `--follow` from now) only when it
//! exists, and checks that dropping the lines object kills and reaps the child; it never
//! changes any journal configuration and returns early (with a note) without `journalctl`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    clippy::print_stderr,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::AsyncWriteExt;

use soos_remote::journal::{
    BoundedLineReader, FollowStart, JournalSource, JournalctlSource, LineRead, JOURNALCTL_PATH,
};
use soos_remote::{JOURNAL_READ_CHUNK_BYTES, MAX_JOURNAL_LINE_BYTES};

/// What a reader yields, comparable.
#[derive(Debug, PartialEq, Eq)]
enum Got {
    Line(Vec<u8>),
    Overlong,
    End,
}

async fn next(reader: &mut BoundedLineReader<tokio::io::DuplexStream>) -> Got {
    match tokio::time::timeout(Duration::from_secs(10), reader.next_line())
        .await
        .expect("the reader answers")
    {
        LineRead::Line(bytes) => Got::Line(bytes.to_vec()),
        LineRead::Overlong => Got::Overlong,
        LineRead::End => Got::End,
    }
}

/// A short description (kind and length) for diagnostics.
fn short(item: &Got) -> String {
    match item {
        Got::Line(bytes) => format!("Line({} bytes)", bytes.len()),
        Got::Overlong => "Overlong".to_string(),
        Got::End => "End".to_string(),
    }
}

/// Writes `data` in pieces of the given sizes (cycled), then closes the writer.
fn feed(mut writer: tokio::io::DuplexStream, data: Vec<u8>, pieces: Vec<usize>) {
    tokio::spawn(async move {
        let mut at = 0;
        let mut i = 0;
        while at < data.len() {
            let size = pieces[i % pieces.len()].max(1);
            let end = (at + size).min(data.len());
            writer.write_all(&data[at..end]).await.unwrap();
            writer.flush().await.unwrap();
            tokio::task::yield_now().await;
            at = end;
            i += 1;
        }
        drop(writer);
    });
}

/// Test 42 (RMC46, A-14, F-10): exact lines whatever the write boundaries; a line of
/// exactly `MAX_JOURNAL_LINE_BYTES` is accepted, one byte more is `Overlong` and the next
/// line is read intact; a partial last line at EOF is `End`; the buffer capacity is
/// reserved once and never changes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rmc_alerts_bounded_line_reader() {
    let at_bound = vec![b'a'; MAX_JOURNAL_LINE_BYTES];
    let over = vec![b'b'; MAX_JOURNAL_LINE_BYTES + 1];
    let far_over = vec![b'c'; 3 * MAX_JOURNAL_LINE_BYTES + 17];
    let mut data = Vec::new();
    for part in [
        &b"{\"MESSAGE\":\"first\"}"[..],
        b"",
        b"second line",
        &at_bound,
        &over,
        b"after over",
        &far_over,
        b"after far over",
    ] {
        data.extend_from_slice(part);
        data.push(b'\n');
    }
    data.extend_from_slice(b"partial line without newline");
    let expected = vec![
        Got::Line(b"{\"MESSAGE\":\"first\"}".to_vec()),
        Got::Line(Vec::new()),
        Got::Line(b"second line".to_vec()),
        Got::Line(at_bound.clone()),
        Got::Overlong,
        Got::Line(b"after over".to_vec()),
        Got::Overlong,
        Got::Line(b"after far over".to_vec()),
        Got::End,
    ];
    for pieces in [
        vec![1],
        vec![7, 3, 4096, 1],
        vec![JOURNAL_READ_CHUNK_BYTES],
        vec![JOURNAL_READ_CHUNK_BYTES + 1],
        vec![JOURNAL_READ_CHUNK_BYTES - 1, 2],
        vec![65_536],
        vec![MAX_JOURNAL_LINE_BYTES, 5],
    ] {
        // Small duplex buffers force many short reads.
        let (writer, reader) = tokio::io::duplex(997);
        feed(writer, data.clone(), pieces.clone());
        let mut reader = BoundedLineReader::new(reader);
        let capacity = reader.capacity();
        assert!(
            capacity >= MAX_JOURNAL_LINE_BYTES + JOURNAL_READ_CHUNK_BYTES,
            "reserved once at construction: {capacity}"
        );
        let mut got = Vec::new();
        loop {
            let item = next(&mut reader).await;
            assert_eq!(
                reader.capacity(),
                capacity,
                "never reallocated ({pieces:?})"
            );
            let end = item == Got::End;
            got.push(item);
            if end {
                break;
            }
            assert!(
                got.len() <= expected.len(),
                "too many items with {pieces:?}"
            );
        }
        assert_eq!(got.len(), expected.len(), "{pieces:?}");
        for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
            assert!(
                g == e,
                "item {i} with {pieces:?}: got {}, expected {}",
                short(g),
                short(e)
            );
        }
        // After End, End again.
        assert_eq!(next(&mut reader).await, Got::End);
    }
    // An empty stream is End at once.
    let (writer, reader) = tokio::io::duplex(64);
    drop(writer);
    let mut reader = BoundedLineReader::new(reader);
    assert_eq!(next(&mut reader).await, Got::End);
}

fn parent_pid_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `pid (comm) state ppid …`; comm may contain spaces and parentheses.
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
}

/// Test 43 (RMC52, A-1, F-8): dropping the lines object of a real `journalctl --follow`
/// child kills it and the runtime reaps it within 2 s (the PID is gone, or reused by a
/// process that is not our child).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rmc_alerts_journalctl_source_kills_child_on_drop() {
    if !Path::new(JOURNALCTL_PATH).exists() {
        eprintln!("note: {JOURNALCTL_PATH} not present; test 43 skipped on this host");
        return;
    }
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let source = JournalctlSource;
    let mut lines = match tokio::time::timeout(
        Duration::from_secs(5),
        source.follow(FollowStart::Since { unix_s: now_s }),
    )
    .await
    .expect("follow returns within the bound")
    {
        Ok(lines) => lines,
        Err(e) => panic!("journalctl present but the follower did not start: {e:?}"),
    };
    let pid = lines
        .child_pid()
        .expect("the production lines expose the child PID");
    assert!(pid > 1);
    // At most one line within 2 s (none is needed).
    let _ = tokio::time::timeout(Duration::from_secs(2), lines.next_line()).await;
    let me = std::process::id();
    drop(lines);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match parent_pid_of(pid) {
            None => break,
            Some(parent) if parent != me => break,
            Some(_) => {}
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the journalctl child {pid} is still ours 2 s after the drop"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
