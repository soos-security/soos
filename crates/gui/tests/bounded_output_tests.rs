//! Contract tests: the profile-list helper output is bounded while it is read, not after it
//! was buffered whole (candid review finding 7, `MAX_PROFILE_LIST_BYTES`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use soos_gui::privileged::{read_bounded, BoundedReadError, MAX_PROFILE_LIST_BYTES};

/// Reader producing `remaining` bytes of `b'x'` and counting how many were consumed.
struct CountingReader {
    remaining: usize,
    consumed: Arc<AtomicUsize>,
}

impl Read for CountingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.remaining);
        buf[..n].fill(b'x');
        self.remaining -= n;
        self.consumed.fetch_add(n, Ordering::SeqCst);
        Ok(n)
    }
}

#[test]
fn test_read_bounded_accepts_output_up_to_the_limit() {
    let data = vec![b'a'; 1024];
    assert_eq!(read_bounded(&data[..], 1024).unwrap(), data);
    assert!(read_bounded(&b""[..], 1024).unwrap().is_empty());
}

#[test]
fn test_read_bounded_rejects_output_over_the_limit() {
    let data = vec![b'a'; 1025];
    assert!(matches!(
        read_bounded(&data[..], 1024),
        Err(BoundedReadError::Oversized { limit: 1024 })
    ));
}

#[test]
fn test_read_bounded_never_consumes_more_than_limit_plus_one() {
    let consumed = Arc::new(AtomicUsize::new(0));
    let reader = CountingReader {
        remaining: MAX_PROFILE_LIST_BYTES * 4,
        consumed: Arc::clone(&consumed),
    };
    let result = read_bounded(reader, MAX_PROFILE_LIST_BYTES);
    assert!(matches!(result, Err(BoundedReadError::Oversized { .. })));
    assert!(
        consumed.load(Ordering::SeqCst) <= MAX_PROFILE_LIST_BYTES + 1,
        "read {} bytes: the bound must be enforced while reading",
        consumed.load(Ordering::SeqCst)
    );
}

#[test]
fn test_list_profiles_reads_helper_stdout_through_bounded_reader() {
    let src = include_str!("../src/privileged.rs");
    let list_fn = &src[src.find("fn list_profiles(").unwrap()..];
    let list_fn = &list_fn[..list_fn.find("\n}\n").unwrap()];
    assert!(
        !list_fn.contains(".output()"),
        "list_profiles must not buffer the whole helper stdout with Command::output"
    );
    assert!(
        list_fn.contains("read_bounded("),
        "list_profiles must use read_bounded"
    );
}
