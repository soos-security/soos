//! Hermetic tests of the V4L2 capture supervisor through a fake [`CaptureBackend`]
//! (GitHub #198, CAM-16; matrix rows CCB1–CCB9, walkthrough 159).
//!
//! The supervisor (`supervise` / `open_and_stream`) runs unchanged on its own
//! `soos-v4l-capture` thread; only the device operations below it (open, `VIDIOC_QUERYCAP`,
//! `VIDIOC_ENUM_FMT`, `VIDIOC_S_FMT`, `VIDIOC_S_PARM`, MMAP stream, DQBUF, teardown) are
//! scripted. Device paths live under a directory that never exists, so the by-id alias lookup of
//! the open path finds nothing and the tests never touch `/dev`. Frames are synthetic byte
//! patterns; nothing is written or logged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual supervisor test suite uses direct assertions on a scripted backend"
)]

use super::*;
use crate::capture::{BufferMeta, CaptureSource, C9_SHUTDOWN_BUDGET, MAX_STREAM_STALL};
use crate::config::CameraConfigBuilder;
use crate::status::CameraErrorKind;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::{Mutex, MutexGuard};

/// Parent of every fake device path; it never exists, so the alias lookup stays hermetic.
const FAKE_DIR: &str = "/nonexistent-soos-ccb";
/// Simulated frame interval of a streaming fake node.
const FAKE_FRAME_INTERVAL: Duration = Duration::from_millis(5);
/// Upper bound for every wait on a supervisor transition (generous for slow CI runners).
const TRANSITION_TIMEOUT: Duration = Duration::from_secs(10);

fn fake_path(name: &str) -> PathBuf {
    Path::new(FAKE_DIR).join(name)
}

/// Behaviour of one MMAP stream started on a fake node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamScript {
    /// Valid frames filled with `fill`, forever.
    Frames { fill: u8 },
    /// `count` valid frames, then the node disappears (`ENODEV` on DQBUF, later opens `ENOENT`).
    VanishAfter { fill: u8, count: u32 },
    /// No frame ever: every DQBUF / poll waits its full timeout and times out.
    Stall,
}

/// Scripted capture node.
#[derive(Debug, Clone)]
struct FakeNode {
    card: String,
    video_capture: bool,
    fourccs: Vec<FourCC>,
    /// `bytesperline` granted by `VIDIOC_S_FMT`.
    stride: u32,
    /// Length of every delivered buffer.
    frame_len: usize,
    /// errno of the next `open(2)` attempts, one consumed per attempt.
    open_errors: VecDeque<i32>,
    /// errno of the next `VIDIOC_S_FMT` calls, one consumed per call.
    format_errors: VecDeque<i32>,
    /// Scripts of the next streams, one consumed per stream; the last one repeats.
    streams: VecDeque<StreamScript>,
    /// Number of upcoming stream teardowns that panic like `Drop for v4l::io::mmap::Stream`.
    teardown_panics: u32,
}

impl FakeNode {
    /// RGB webcam streaming 4x2 YUYV.
    fn rgb(fill: u8) -> Self {
        Self {
            card: "Integrated Camera".into(),
            video_capture: true,
            fourccs: vec![FourCC::new(b"YUYV")],
            stride: 8,
            frame_len: 16,
            open_errors: VecDeque::new(),
            format_errors: VecDeque::new(),
            streams: VecDeque::from([StreamScript::Frames { fill }]),
            teardown_panics: 0,
        }
    }

    /// IR node advertising 8-bit greyscale only.
    fn ir_grey(fill: u8) -> Self {
        Self {
            card: "Integrated IR Camera".into(),
            fourccs: vec![FourCC::new(b"GREY")],
            stride: 4,
            frame_len: 8,
            ..Self::rgb(fill)
        }
    }

    /// IR node advertising only the 16-bit `Y16 ` deep-greyscale format.
    fn ir_y16(fill: u8) -> Self {
        Self {
            card: "Depth Module".into(),
            fourccs: vec![FourCC::new(b"Y16 ")],
            stride: 8,
            frame_len: 16,
            ..Self::rgb(fill)
        }
    }

    fn with_streams(mut self, streams: &[StreamScript]) -> Self {
        self.streams = streams.iter().copied().collect();
        self
    }

    fn with_open_errors(mut self, errors: &[i32]) -> Self {
        self.open_errors = errors.iter().copied().collect();
        self
    }

    fn with_format_errors(mut self, errors: &[i32]) -> Self {
        self.format_errors = errors.iter().copied().collect();
        self
    }
}

/// What the fake observed (never frame content).
#[derive(Debug, Default)]
struct FakeLog {
    /// Every `open(2)` attempt with its path and time.
    opens: Vec<(PathBuf, Instant)>,
    /// Devices currently open (incremented on a successful open, decremented on close).
    open_handles: usize,
    /// Streams started (`VIDIOC_REQBUFS` + mmap succeeded).
    streams_started: usize,
    /// Streams torn down (dropped through the guard).
    teardowns: usize,
    /// `VIDIOC_S_FMT` requests.
    format_requests: Vec<v4l::Format>,
    /// Calls of the test resolver.
    resolves: usize,
}

#[derive(Debug, Default)]
struct FakeWorld {
    nodes: HashMap<PathBuf, FakeNode>,
    log: FakeLog,
}

/// Locks the world; a poisoned lock (a panicking teardown never holds it) is still readable.
fn lock(world: &Mutex<FakeWorld>) -> MutexGuard<'_, FakeWorld> {
    world.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Default)]
struct FakeBackend {
    world: Arc<Mutex<FakeWorld>>,
}

impl FakeBackend {
    fn with_node(path: &Path, node: FakeNode) -> Self {
        let backend = Self::default();
        backend.insert(path, node);
        backend
    }

    fn insert(&self, path: &Path, node: FakeNode) {
        lock(&self.world).nodes.insert(path.to_path_buf(), node);
    }

    fn log<T>(&self, read: impl FnOnce(&FakeLog) -> T) -> T {
        read(&lock(&self.world).log)
    }

    fn opens_of(&self, path: &Path) -> usize {
        self.log(|log| log.opens.iter().filter(|(p, _)| p == path).count())
    }

    /// A resolver that counts its calls and returns `target`.
    fn resolver(&self, target: PathBuf) -> Arc<dyn DevicePathResolver> {
        let world = Arc::clone(&self.world);
        Arc::new(move || {
            lock(&world).log.resolves += 1;
            Some(target.clone())
        })
    }
}

impl CaptureBackend for FakeBackend {
    type Device = FakeDevice;

    fn open_device(&self, path: &Path) -> io::Result<FakeDevice> {
        let mut world = lock(&self.world);
        world.log.opens.push((path.to_path_buf(), Instant::now()));
        let Some(node) = world.nodes.get_mut(path) else {
            return Err(io::Error::from_raw_os_error(libc::ENOENT));
        };
        if let Some(errno) = node.open_errors.pop_front() {
            return Err(io::Error::from_raw_os_error(errno));
        }
        world.log.open_handles += 1;
        Ok(FakeDevice {
            world: Arc::clone(&self.world),
            path: path.to_path_buf(),
        })
    }
}

struct FakeDevice {
    world: Arc<Mutex<FakeWorld>>,
    path: PathBuf,
}

impl FakeDevice {
    fn node<T>(&self, read: impl FnOnce(&mut FakeNode) -> T) -> Option<T> {
        lock(&self.world).nodes.get_mut(&self.path).map(read)
    }

    fn gone() -> io::Error {
        io::Error::from_raw_os_error(libc::ENODEV)
    }
}

impl Drop for FakeDevice {
    fn drop(&mut self) {
        let mut world = lock(&self.world);
        world.log.open_handles = world.log.open_handles.saturating_sub(1);
    }
}

impl CaptureDevice for FakeDevice {
    type Stream<'a> = FakeStream<'a>;

    fn capabilities(&self) -> io::Result<NodeCapabilities> {
        self.node(|node| NodeCapabilities {
            card: node.card.clone(),
            video_capture: node.video_capture,
        })
        .ok_or_else(Self::gone)
    }

    fn pixel_formats(&self) -> Vec<FourCC> {
        self.node(|node| node.fourccs.clone()).unwrap_or_default()
    }

    fn frame_sizes(&self, _path: &Path, _fourccs: &[FourCC]) -> Vec<(u32, u32)> {
        Vec::new()
    }

    fn apply_format(&self, requested: &v4l::Format) -> io::Result<v4l::Format> {
        lock(&self.world).log.format_requests.push(*requested);
        self.node(|node| match node.format_errors.pop_front() {
            Some(errno) => Err(io::Error::from_raw_os_error(errno)),
            None => {
                let mut granted = *requested;
                granted.stride = node.stride;
                Ok(granted)
            }
        })
        .ok_or_else(Self::gone)?
    }

    fn apply_frame_interval(&self, numerator: u32, denominator: u32) -> io::Result<(u32, u32)> {
        Ok((numerator, denominator))
    }

    fn start_stream(
        &self,
        _buffer_count: u32,
        poll_timeout: Duration,
    ) -> io::Result<FakeStream<'_>> {
        let (script, frame_len, panic_on_teardown) = self
            .node(|node| {
                let script = if node.streams.len() > 1 {
                    node.streams.pop_front()
                } else {
                    node.streams.front().copied()
                };
                let panic_on_teardown = node.teardown_panics > 0;
                node.teardown_panics = node.teardown_panics.saturating_sub(1);
                (script, node.frame_len, panic_on_teardown)
            })
            .ok_or_else(Self::gone)?;
        let script = script.unwrap_or(StreamScript::Stall);
        lock(&self.world).log.streams_started += 1;
        Ok(FakeStream {
            device: self,
            script,
            poll_timeout,
            delivered: 0,
            frame_len,
            buffer: Vec::new(),
            panic_on_teardown,
        })
    }
}

struct FakeStream<'a> {
    device: &'a FakeDevice,
    script: StreamScript,
    poll_timeout: Duration,
    delivered: u32,
    frame_len: usize,
    buffer: Vec<u8>,
    panic_on_teardown: bool,
}

impl FakeStream<'_> {
    fn timed_out(&self) -> io::Error {
        thread::sleep(self.poll_timeout);
        io::Error::new(io::ErrorKind::TimedOut, "VIDIOC_DQBUF")
    }

    /// The node disappears: DQBUF fails with `ENODEV` and later opens with `ENOENT`.
    fn vanish(&self) -> io::Error {
        lock(&self.device.world).nodes.remove(&self.device.path);
        io::Error::from_raw_os_error(libc::ENODEV)
    }
}

impl CaptureSource for FakeStream<'_> {
    fn wait_ready(&mut self, timeout: Duration) -> io::Result<bool> {
        if self.script == StreamScript::Stall {
            thread::sleep(timeout);
            return Ok(false);
        }
        thread::sleep(FAKE_FRAME_INTERVAL);
        Ok(true)
    }

    fn next_buffer(&mut self) -> io::Result<(&[u8], BufferMeta)> {
        let fill = match self.script {
            StreamScript::Stall => return Err(self.timed_out()),
            StreamScript::VanishAfter { count, .. } if self.delivered >= count => {
                return Err(self.vanish())
            }
            StreamScript::Frames { fill } | StreamScript::VanishAfter { fill, .. } => fill,
        };
        self.delivered = self.delivered.saturating_add(1);
        self.buffer = vec![fill; self.frame_len];
        let meta = BufferMeta {
            bytesused: u32::try_from(self.frame_len).unwrap(),
            error_flag: false,
        };
        Ok((self.buffer.as_slice(), meta))
    }

    fn resync(&mut self) -> io::Result<()> {
        match self.script {
            StreamScript::Stall => Err(self.timed_out()),
            _ => Ok(()),
        }
    }
}

impl Drop for FakeStream<'_> {
    fn drop(&mut self) {
        lock(&self.device.world).log.teardowns += 1;
        // The lock is released before panicking, like the failing `VIDIOC_STREAMOFF` of v4l 0.14.
        if self.panic_on_teardown {
            panic!("VIDIOC_STREAMOFF failed (simulated v4l 0.14 teardown panic)");
        }
    }
}

/// 4x2 capture, no warm-up, auto format, given idle timeout and backoff limits.
fn config(path: &Path, idle_timeout: Duration, min_backoff: Duration) -> CameraConfig {
    CameraConfigBuilder::new()
        .device_path(path)
        .resolution(4, 2)
        .fps(30)
        .warmup_frames(0)
        .idle_timeout(idle_timeout)
        .backoff_limits(min_backoff, min_backoff.saturating_mul(4))
        .build()
}

fn spawn(
    backend: &FakeBackend,
    config: CameraConfig,
    resolver: Option<Arc<dyn DevicePathResolver>>,
) -> V4lCameraManager {
    V4lCameraManager::spawn_inner(config, resolver, backend.clone()).expect("spawn supervisor")
}

/// Polls `condition` every 2 ms until it holds or `TRANSITION_TIMEOUT` elapses.
fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + TRANSITION_TIMEOUT;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(2));
    }
    condition()
}

/// Waits for a published frame, signalling activity on every poll (as the daemon does).
fn wait_for_frame(camera: &V4lCameraManager) -> Arc<Frame> {
    let mut frame = None;
    assert!(
        wait_until(|| {
            camera.notify_activity();
            frame = camera.latest_frame();
            frame.is_some()
        }),
        "no frame within {TRANSITION_TIMEOUT:?}; health {:?}, status {:?}",
        camera.health(),
        camera.status()
    );
    frame.unwrap()
}

fn is_error_of_kind(status: CameraStatus, expected: CameraErrorKind) -> bool {
    matches!(status, CameraStatus::Error { kind, .. } if kind == expected)
}

/// CCB1: a failing open is retried with exponential backoff (`Recovering`, typed error status,
/// no frame), and the first successful open streams and reports `Ready`.
#[test]
fn test_ccb_open_failure_backs_off_then_recovers() {
    let path = fake_path("video0");
    let backend = FakeBackend::with_node(
        &path,
        FakeNode::rgb(0x40).with_open_errors(&[libc::EACCES, libc::EACCES, libc::EIO]),
    );
    let min_backoff = Duration::from_millis(30);
    let camera = spawn(&backend, config(&path, Duration::ZERO, min_backoff), None);

    assert!(
        wait_until(|| is_error_of_kind(camera.status(), CameraErrorKind::PermissionDenied)),
        "an EACCES open must surface as a PermissionDenied error status, got {:?}",
        camera.status()
    );
    assert_eq!(camera.health(), CameraHealth::Recovering);
    assert!(
        camera.latest_frame().is_none(),
        "no frame before a successful open"
    );

    let frame = wait_for_frame(&camera);
    assert_eq!((frame.width, frame.height), (4, 2));
    assert_eq!(camera.health(), CameraHealth::Streaming);
    assert_eq!(camera.status(), CameraStatus::Ready);

    let opens: Vec<Instant> = backend.log(|log| log.opens.iter().map(|(_, at)| *at).collect());
    assert!(opens.len() >= 4, "three failed opens, then one success");
    // Exponential backoff: min, 2 x min, 4 x min (capped at max = 4 x min).
    for (i, factor) in [1u32, 2, 4].into_iter().enumerate() {
        let gap = opens[i + 1].duration_since(opens[i]);
        assert!(
            gap >= min_backoff * factor,
            "open {} followed open {} after {gap:?}, expected at least {:?}",
            i + 2,
            i + 1,
            min_backoff * factor
        );
    }
    assert_eq!(backend.log(|log| log.resolves), 0);
    camera.stop();
}

/// CCB2: a node streamed by another process fails `VIDIOC_S_FMT` with `EBUSY`: the status is
/// `DeviceBusy`, the device is closed between attempts, the path is never re-resolved, and the
/// camera streams once the other owner releases it.
#[test]
fn test_ccb_busy_device_is_retried_on_the_same_path() {
    let path = fake_path("video0");
    let other = fake_path("video2");
    let backend = FakeBackend::with_node(
        &path,
        FakeNode::rgb(0x41).with_format_errors(&[libc::EBUSY, libc::EBUSY]),
    );
    backend.insert(&other, FakeNode::rgb(0x99));
    let resolver = backend.resolver(other.clone());
    let camera = spawn(
        &backend,
        config(&path, Duration::ZERO, Duration::from_millis(30)),
        Some(resolver),
    );

    assert!(
        wait_until(|| is_error_of_kind(camera.status(), CameraErrorKind::DeviceBusy)),
        "EBUSY at VIDIOC_S_FMT must be classified DeviceBusy, got {:?}",
        camera.status()
    );
    let frame = wait_for_frame(&camera);
    assert_eq!(frame.data[0], 0x41, "frames come from the configured node");
    assert_eq!(camera.current_device_path(), path);
    assert_eq!(
        backend.log(|log| log.resolves),
        0,
        "EBUSY never re-resolves"
    );
    assert_eq!(backend.opens_of(&other), 0);
    assert!(backend.opens_of(&path) >= 3);
    assert_eq!(
        backend.log(|log| log.open_handles),
        1,
        "each failed attempt closed its device handle"
    );
    camera.stop();
}

/// CCB3: frames flow with the negotiated geometry and format and are stamped with the sensor
/// type of the opened node: RGB (YUYV), IR (GREY by card name) and IR deep greyscale (`Y16 `
/// normalised to 8-bit Grey).
#[test]
fn test_ccb_frames_flow_stamped_with_the_sensor_type() {
    let cases = [
        (
            FakeNode::rgb(0x42),
            SensorType::Rgb,
            PixelFormat::Yuyv,
            *b"YUYV",
            16,
        ),
        (
            FakeNode::ir_grey(0x43),
            SensorType::Infrared,
            PixelFormat::Grey,
            *b"GREY",
            8,
        ),
        (
            FakeNode::ir_y16(0x44),
            SensorType::Infrared,
            PixelFormat::Grey,
            *b"Y16 ",
            8,
        ),
    ];
    for (node, sensor, format, wire, len) in cases {
        let path = fake_path("video0");
        let backend = FakeBackend::with_node(&path, node);
        let camera = spawn(
            &backend,
            config(&path, Duration::ZERO, Duration::from_millis(30)),
            None,
        );

        let first = wait_for_frame(&camera);
        assert_eq!(first.sensor_type, sensor, "sensor stamp for {wire:?}");
        assert_eq!(first.format, format);
        assert_eq!((first.width, first.height), (4, 2));
        assert_eq!(first.data.len(), len, "tightly packed {format:?} frame");
        assert!(
            first.timestamp_mono_ns > 0,
            "frames carry the monotonic clock"
        );

        assert!(
            wait_until(|| camera
                .latest_frame()
                .is_some_and(|f| f.sequence > first.sequence)),
            "frames keep flowing"
        );
        assert_eq!(camera.health(), CameraHealth::Streaming);
        assert_eq!(camera.status(), CameraStatus::Ready);
        let requested = backend.log(|log| log.format_requests[0]);
        assert_eq!(requested.fourcc.repr, wire, "wire fourcc requested");
        assert_eq!((requested.width, requested.height), (4, 2));
        camera.stop();
    }
}

/// CCB4: a stream that delivers nothing escalates to `Starved` after `MAX_STREAM_STALL`
/// (frames withdrawn, `Recovering`), the device is torn down and reopened, and a healthy second
/// stream recovers.
#[test]
fn test_ccb_stall_escalates_to_starved_then_reopens() {
    let path = fake_path("video0");
    let backend = FakeBackend::with_node(
        &path,
        FakeNode::rgb(0x45)
            .with_streams(&[StreamScript::Stall, StreamScript::Frames { fill: 0x46 }]),
    );
    let started = Instant::now();
    let camera = spawn(
        &backend,
        config(&path, Duration::ZERO, Duration::from_millis(200)),
        None,
    );

    assert!(
        wait_until(|| is_error_of_kind(camera.status(), CameraErrorKind::Starved)),
        "a stalled stream must report Starved, got {:?}",
        camera.status()
    );
    assert!(
        started.elapsed() >= MAX_STREAM_STALL,
        "Starved only after the stall budget"
    );
    assert_eq!(camera.health(), CameraHealth::Recovering);
    assert!(camera.latest_frame().is_none());
    assert_eq!(
        backend.log(|log| log.teardowns),
        1,
        "stalled stream torn down"
    );
    assert_eq!(
        backend.log(|log| log.open_handles),
        0,
        "device closed while backing off"
    );

    let frame = wait_for_frame(&camera);
    assert_eq!(frame.data[0], 0x46, "second stream delivers");
    assert_eq!(backend.log(|log| log.streams_started), 2);
    assert_eq!(camera.health(), CameraHealth::Streaming);
    assert_eq!(
        backend.log(|log| log.resolves),
        0,
        "Starved never re-resolves"
    );
    camera.stop();
}

/// CCB5: a node that disappears mid-stream (`ENODEV` on DQBUF) reports `DeviceNotFound`, the
/// resolver is consulted, and the supervisor reopens and streams the re-enumerated node.
#[test]
fn test_ccb_vanished_device_is_reresolved_to_the_new_node() {
    let old = fake_path("video0");
    let new = fake_path("video4");
    let backend = FakeBackend::with_node(
        &old,
        FakeNode::rgb(0x47).with_streams(&[StreamScript::VanishAfter {
            fill: 0x47,
            count: 3,
        }]),
    );
    backend.insert(&new, FakeNode::rgb(0x48));
    let resolver = backend.resolver(new.clone());
    let camera = spawn(
        &backend,
        config(&old, Duration::ZERO, Duration::from_millis(30)),
        Some(resolver),
    );

    assert!(
        wait_until(|| camera.latest_frame().is_some_and(|f| f.data[0] == 0x48)),
        "the re-resolved node streams; status {:?}",
        camera.status()
    );
    assert_eq!(camera.current_device_path(), new);
    assert_eq!(camera.health(), CameraHealth::Streaming);
    assert!(backend.log(|log| log.resolves) >= 1);
    assert_eq!(
        backend.opens_of(&old),
        1,
        "the vanished node is not reopened"
    );
    camera.stop();
}

/// CCB6: without a resolver, a vanished node is retried on its configured path forever
/// (`DeviceNotFound`, bounded backoff), never substituted, and streams again when it returns.
#[test]
fn test_ccb_vanished_device_without_resolver_keeps_its_path() {
    let path = fake_path("video0");
    let backend = FakeBackend::with_node(
        &path,
        FakeNode::rgb(0x49).with_streams(&[StreamScript::VanishAfter {
            fill: 0x49,
            count: 2,
        }]),
    );
    let camera = spawn(
        &backend,
        config(&path, Duration::ZERO, Duration::from_millis(20)),
        None,
    );

    assert!(
        wait_until(
            || is_error_of_kind(camera.status(), CameraErrorKind::DeviceNotFound)
                && backend.opens_of(&path) >= 3
        ),
        "a vanished node is retried as DeviceNotFound, status {:?}",
        camera.status()
    );
    assert_eq!(camera.health(), CameraHealth::Recovering);
    assert!(
        camera.latest_frame().is_none(),
        "frames withdrawn after loss"
    );
    assert_eq!(camera.current_device_path(), path);

    backend.insert(&path, FakeNode::rgb(0x4a));
    assert!(
        wait_until(|| camera.latest_frame().is_some_and(|f| f.data[0] == 0x4a)),
        "the returning node streams again"
    );
    camera.stop();
}

/// CCB7: a node without `V4L2_CAP_VIDEO_CAPTURE` is `UnsupportedDevice` and consults the
/// resolver, which moves the supervisor to a real capture node.
#[test]
fn test_ccb_unsupported_node_is_reresolved() {
    let metadata = fake_path("video1");
    let capture = fake_path("video0");
    let backend = FakeBackend::with_node(
        &metadata,
        FakeNode {
            video_capture: false,
            ..FakeNode::rgb(0x4b)
        },
    );
    backend.insert(&capture, FakeNode::rgb(0x4c));
    let resolver = backend.resolver(capture.clone());
    let camera = spawn(
        &backend,
        config(&metadata, Duration::ZERO, Duration::from_millis(20)),
        Some(resolver),
    );

    let frame = wait_for_frame(&camera);
    assert_eq!(frame.data[0], 0x4c);
    assert_eq!(camera.current_device_path(), capture);
    assert_eq!(
        backend.log(|log| log.streams_started),
        1,
        "no stream on the metadata node"
    );
    camera.stop();
}

/// CCB8: past the idle timeout the stream is torn down and the device closed (`Standby`,
/// `Suspended`, no frame); activity reopens the same node and streaming resumes.
#[test]
fn test_ccb_idle_suspend_releases_the_device_and_resumes_on_activity() {
    let path = fake_path("video0");
    let backend = FakeBackend::with_node(&path, FakeNode::rgb(0x4d));
    let camera = spawn(
        &backend,
        config(&path, Duration::from_millis(300), Duration::from_millis(20)),
        None,
    );
    camera.notify_activity();
    wait_for_frame(&camera);

    assert!(
        wait_until(|| camera.status() == CameraStatus::Suspended),
        "no auto-standby, status {:?}",
        camera.status()
    );
    assert_eq!(camera.health(), CameraHealth::Standby);
    assert!(camera.latest_frame().is_none());
    assert!(
        wait_until(|| backend.log(|log| log.open_handles) == 0),
        "the device handle is released in standby"
    );
    assert_eq!(
        backend.log(|log| (log.streams_started, log.teardowns)),
        (1, 1)
    );

    camera.notify_activity();
    assert!(
        wait_until(|| backend.log(|log| log.streams_started) == 2),
        "activity reopens the device"
    );
    camera.notify_activity();
    wait_for_frame(&camera);
    assert_eq!(camera.health(), CameraHealth::Streaming);
    assert_eq!(backend.opens_of(&path), 2);
    camera.stop();
}

/// CCB9: a teardown that panics like `v4l` 0.14 after an idle suspend becomes a recoverable
/// `StreamTeardown` error (`Recovering`, `Io`), never `Dead`; the supervisor reopens, settles in
/// standby and streams again on activity.
#[test]
fn test_ccb_teardown_failure_recovers_instead_of_dying() {
    let path = fake_path("video0");
    let backend = FakeBackend::with_node(
        &path,
        FakeNode {
            teardown_panics: 1,
            ..FakeNode::rgb(0x4e)
        },
    );
    let camera = spawn(
        &backend,
        config(
            &path,
            Duration::from_millis(300),
            Duration::from_millis(300),
        ),
        None,
    );
    camera.notify_activity();
    wait_for_frame(&camera);

    assert!(
        wait_until(|| is_error_of_kind(camera.status(), CameraErrorKind::Io)),
        "a teardown panic must surface as an Io error, status {:?}",
        camera.status()
    );
    assert_eq!(camera.health(), CameraHealth::Recovering);
    assert!(camera.latest_frame().is_none());

    let mut dead = false;
    assert!(
        wait_until(|| {
            dead |= camera.health() == CameraHealth::Dead;
            camera.status() == CameraStatus::Suspended
        }),
        "the supervisor reopens and settles in standby, status {:?}",
        camera.status()
    );
    assert!(!dead, "a teardown failure never marks the camera Dead");
    assert_eq!(
        backend.opens_of(&path),
        2,
        "reopened after the failed teardown"
    );

    camera.notify_activity();
    wait_for_frame(&camera);
    assert_eq!(camera.health(), CameraHealth::Streaming);
    camera.stop();
}

/// CCB10: shutdown of a stalled stream, of a supervisor in a long backoff, and of a streaming
/// session whose teardown panics all complete within the C9 budget, release the device and leave
/// the manager `Stopped` and frame-free.
#[test]
fn test_ccb_shutdown_releases_the_device_within_budget() {
    struct Case {
        node: FakeNode,
        min_backoff: Duration,
        settle: fn(&V4lCameraManager, &FakeBackend) -> bool,
    }
    let cases = [
        Case {
            node: FakeNode::rgb(0x50).with_streams(&[StreamScript::Stall]),
            min_backoff: Duration::from_millis(20),
            settle: |_, backend| backend.log(|log| log.streams_started) == 1,
        },
        Case {
            node: FakeNode::rgb(0x51).with_open_errors(&[libc::EIO; 4]),
            min_backoff: Duration::from_secs(5),
            settle: |camera, _| camera.health() == CameraHealth::Recovering,
        },
        Case {
            node: FakeNode {
                teardown_panics: 1,
                ..FakeNode::rgb(0x52)
            },
            min_backoff: Duration::from_millis(20),
            settle: |camera, _| camera.latest_frame().is_some(),
        },
    ];
    for (index, case) in cases.into_iter().enumerate() {
        let path = fake_path("video0");
        let backend = FakeBackend::with_node(&path, case.node);
        let camera = spawn(
            &backend,
            config(&path, Duration::ZERO, case.min_backoff),
            None,
        );
        assert!(
            wait_until(|| (case.settle)(&camera, &backend)),
            "case {index} never reached its pre-shutdown state"
        );

        let stop = Instant::now();
        camera.stop();
        assert!(
            wait_until(|| camera.status() == CameraStatus::Stopped),
            "case {index}: supervisor did not stop"
        );
        let elapsed = stop.elapsed();
        assert!(
            elapsed <= C9_SHUTDOWN_BUDGET,
            "case {index}: shutdown took {elapsed:?}, budget {C9_SHUTDOWN_BUDGET:?}"
        );
        assert!(camera.latest_frame().is_none());
        assert_ne!(camera.health(), CameraHealth::Dead, "case {index}");
        assert_eq!(
            backend.log(|log| log.open_handles),
            0,
            "case {index}: device released"
        );
        let (started, torn_down) = backend.log(|log| (log.streams_started, log.teardowns));
        assert_eq!(started, torn_down, "case {index}: every stream torn down");
        drop(camera);
    }
}
