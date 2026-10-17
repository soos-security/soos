//! Mock soos daemon for the `soos-lockstatus` contract tests: a std `UnixListener` in a temporary
//! directory that serves one scripted behaviour per connection and records what it saw.

#![allow(
    dead_code,
    reason = "Shared test fixtures library used conditionally across test modules"
)]

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use soos_protocol::{
    decode_client_message, encode, ClientMessage, LockStateResponse, LockWidgetState, ReasonClass,
    Request, Response, Verdict, CURRENT_VERSION,
};

/// What the mock does with each connection after reading one request frame.
#[derive(Debug, Clone)]
pub enum Behavior {
    /// Answers this lock state.
    Answer(LockWidgetState, u64),
    /// Answers a lock state with another protocol version.
    WrongVersion,
    /// Answers a standard `Response` refusal with this verdict and reason.
    Refuse(Verdict, ReasonClass),
    /// Writes these raw bytes.
    Raw(Vec<u8>),
    /// Keeps the connection open without answering for this long.
    Stall(Duration),
    /// Closes the connection without answering (a daemon older than #380).
    Close,
    /// Answers after this delay.
    DelayedAnswer(Duration, LockWidgetState, u64),
}

/// One request the mock decoded.
#[derive(Debug, Clone)]
pub struct Seen {
    pub version: u8,
    pub kind: soos_protocol::RequestKind,
    pub uid_hint: u32,
    pub service: String,
    pub deadline_monotonic_ns: u64,
    pub tagged: bool,
}

#[derive(Debug, Default)]
pub struct Stats {
    pub connections: AtomicUsize,
    pub active: AtomicUsize,
    pub max_active: AtomicUsize,
    pub requests: AtomicUsize,
    /// Extra requests sent on an already used connection (must stay 0).
    pub second_requests: AtomicUsize,
    pub seen: Mutex<Vec<Seen>>,
}

pub struct MockDaemon {
    pub dir: tempfile::TempDir,
    pub path: PathBuf,
    pub stats: Arc<Stats>,
    behavior: Arc<Mutex<Behavior>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockDaemon {
    pub fn start(behavior: Behavior) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&path).expect("bind mock socket");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let stats = Arc::new(Stats::default());
        let behavior = Arc::new(Mutex::new(behavior));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let stats = Arc::clone(&stats);
            let behavior = Arc::clone(&behavior);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || accept_loop(&listener, &stats, &behavior, &stop))
        };
        Self {
            dir,
            path,
            stats,
            behavior,
            stop,
            handle: Some(handle),
        }
    }

    pub fn set_behavior(&self, behavior: Behavior) {
        *self.behavior.lock().unwrap() = behavior;
    }

    pub fn requests(&self) -> usize {
        self.stats.requests.load(Ordering::SeqCst)
    }

    pub fn connections(&self) -> usize {
        self.stats.connections.load(Ordering::SeqCst)
    }

    pub fn max_active(&self) -> usize {
        self.stats.max_active.load(Ordering::SeqCst)
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.stats.seen.lock().unwrap().clone()
    }
}

impl Drop for MockDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn accept_loop(
    listener: &UnixListener,
    stats: &Arc<Stats>,
    behavior: &Arc<Mutex<Behavior>>,
    stop: &Arc<AtomicBool>,
) {
    let mut workers = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                stats.connections.fetch_add(1, Ordering::SeqCst);
                let active = stats.active.fetch_add(1, Ordering::SeqCst) + 1;
                stats.max_active.fetch_max(active, Ordering::SeqCst);
                let stats = Arc::clone(stats);
                let current = behavior.lock().unwrap().clone();
                let stop = Arc::clone(stop);
                workers.push(std::thread::spawn(move || {
                    serve(stream, &stats, &current, &stop);
                    stats.active.fetch_sub(1, Ordering::SeqCst);
                }));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => break,
        }
    }
    for w in workers {
        let _ = w.join();
    }
}

fn read_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).ok()?;
    let n = u32::from_be_bytes(len) as usize;
    if n == 0 || n > soos_protocol::MAX_MESSAGE_SIZE {
        return None;
    }
    let mut body = vec![0u8; n];
    stream.read_exact(&mut body).ok()?;
    Some(body)
}

fn serve(mut stream: UnixStream, stats: &Stats, behavior: &Behavior, stop: &AtomicBool) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let Some(body) = read_frame(&mut stream) else {
        return;
    };
    stats.requests.fetch_add(1, Ordering::SeqCst);
    if let Ok((ClientMessage::Request(req), format)) = decode_client_message(&body) {
        stats.seen.lock().unwrap().push(Seen {
            version: req.version,
            kind: req.kind,
            uid_hint: req.uid_hint,
            service: req.service.clone(),
            deadline_monotonic_ns: req.deadline_monotonic_ns,
            tagged: format == soos_protocol::FrameFormat::Tagged,
        });
        respond(&mut stream, &req, behavior, stop);
        if matches!(behavior, Behavior::Close) {
            // An old daemon closes at once (no lingering second-frame probe).
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return;
        }
    }
    // A client following the contract closes after one answer; a second frame is a violation.
    stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    if read_frame(&mut stream).is_some() {
        stats.second_requests.fetch_add(1, Ordering::SeqCst);
    }
}

fn respond(stream: &mut UnixStream, req: &Request, behavior: &Behavior, stop: &AtomicBool) {
    let bytes = match behavior {
        Behavior::Answer(state, sequence) => encode(&LockStateResponse {
            version: CURRENT_VERSION,
            state: *state,
            sequence: *sequence,
        })
        .unwrap(),
        Behavior::WrongVersion => encode(&LockStateResponse {
            version: CURRENT_VERSION + 1,
            state: LockWidgetState::Scanning,
            sequence: 1,
        })
        .unwrap(),
        Behavior::Refuse(verdict, reason) => encode(&Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: *verdict,
            reason_class: *reason,
            issued_monotonic_ns: 1,
            expires_monotonic_ns: u64::MAX,
        })
        .unwrap(),
        Behavior::Raw(raw) => raw.clone(),
        Behavior::Stall(d) => {
            let until = std::time::Instant::now() + *d;
            while std::time::Instant::now() < until && !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            return;
        }
        Behavior::Close => return,
        Behavior::DelayedAnswer(d, state, sequence) => {
            std::thread::sleep(*d);
            encode(&LockStateResponse {
                version: CURRENT_VERSION,
                state: *state,
                sequence: *sequence,
            })
            .unwrap()
        }
    };
    let _ = stream.write_all(&bytes);
}

/// A private runtime directory (mode 0700) like `$XDG_RUNTIME_DIR`.
pub fn runtime_dir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("runtime tempdir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

pub fn my_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

pub fn now_ms() -> Option<u64> {
    soos_lockstatus::client::monotonic_now_ms()
}

pub fn exists(path: &Path) -> bool {
    path.exists()
}
