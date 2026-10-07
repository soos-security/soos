//! Live camera view runtime and view loop (ADR 2026-10-07 "Live Camera View in `soos-remote`
//! Through the Daemon Preview Channel", architect spec §7, §8.6, §8.7).
//!
//! [`CameraRuntime`] holds the single global [`ViewSlot`] behind a `std::sync::Mutex` (never
//! held across an `.await`, a poisoned lock fails closed), the durable stop epoch and the rate
//! gate of the `camera view refused` audit line. [`ViewGuard`] frees the slot on every exit
//! path (error, cancellation, task abort, panic) and emits `camera view ended` for a view that
//! showed pixels.
//!
//! The view loop pins one frame step (sleep until the next tick, fetch one preview frame,
//! convert and encode it on the blocking pool) and polls it by `&mut` from a biased
//! `select!`: non-terminal arms (a Funnel session check that passes, stray bytes on the read
//! half) never cancel an in-flight exchange or encode, and every part is written whole in the
//! arm body. The stop signal is the slot's durable flag plus a `watch` epoch, checked at the
//! top of every iteration and after every arm body. Nothing is recorded, nothing is logged
//! except one fixed-text `debug!` per end reason.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::unix::{ReadHalf, WriteHalf};
use tokio::sync::watch;
use tokio::time::{sleep_until, Instant};
use tracing::debug;
use zeroize::Zeroizing;

use crate::audit;
use crate::camera_ipc::{PreviewError, PreviewSource, PreviewSourceFactory};
use crate::camera_jpeg::{encode_preview_jpeg, FrameError, JpegFrame, JpegSettings};
use crate::camera_slot::{SlotError, SlotPhase, ViewOwner, ViewSlot, ViewTicket, ViewToken};
use crate::config::{CameraConfig, CameraWidth};
use crate::http::{encode_camera_part_head, write_bounded, CAMERA_STREAM_TRAILER};
use crate::{
    CAMERA_DAEMON_BACKOFF_MAX_MS, CAMERA_DAEMON_BACKOFF_MIN_MS, CAMERA_FULL_WIDTH,
    CAMERA_HALF_WIDTH, CAMERA_MAX_BAD_FRAMES, CAMERA_RATE_LIMITED_BACKOFF_MS,
    CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS, CAMERA_SESSION_CHECK_MS, CAMERA_STALL_TIMEOUT_MS,
};
use soos_protocol::types::{
    PREVIEW_FORMAT_EMPTY, PREVIEW_FORMAT_GREY, PREVIEW_FORMAT_RGB24, PREVIEW_FORMAT_YUYV,
};

/// Fixed buffer of the read half of a view: bytes are discarded, EOF ends the view.
const VIEW_SINK_BYTES: usize = 64;

/// View settings resolved from the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraSettings {
    /// Encoder settings.
    pub jpeg: JpegSettings,
    /// Frames per second.
    pub fps: u32,
    /// Maximum view duration.
    pub max_view: Duration,
}

impl CameraSettings {
    /// The settings of a validated [`CameraConfig`].
    #[must_use]
    pub fn from_config(config: &CameraConfig) -> Self {
        Self {
            jpeg: JpegSettings {
                width: config.width,
                quality: config.quality,
            },
            fps: config.fps,
            max_view: Duration::from_secs(u64::from(config.max_view_s)),
        }
    }

    /// The output width in pixels (`GET /api/camera`, `view_ready`).
    #[must_use]
    pub fn width_px(&self) -> u32 {
        match self.jpeg.width {
            CameraWidth::Full => CAMERA_FULL_WIDTH,
            CameraWidth::Half => CAMERA_HALF_WIDTH,
        }
    }

    /// The maximum view duration in whole seconds.
    #[must_use]
    pub fn max_view_s(&self) -> u64 {
        self.max_view.as_secs()
    }

    /// The interval between two frame requests.
    fn frame_interval(&self) -> Duration {
        Duration::from_micros(
            1_000_000_u64
                .checked_div(u64::from(self.fps))
                .unwrap_or(1_000_000),
        )
    }
}

/// Why a view ended (one fixed-text `debug!` each; also the outcome of the view loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewEnd {
    /// `POST /api/camera/stop`.
    Stopped,
    /// `camera_max_view_s` reached.
    MaxDuration,
    /// The client closed its side.
    ClientClosed,
    /// A part write failed or timed out.
    WriteFailed,
    /// The Funnel web session is gone.
    SessionExpired,
    /// The server shuts down.
    Shutdown,
    /// The daemon refused the preview.
    DaemonRefused,
    /// No new frame within the stall or first-frame bound.
    CameraUnavailable,
    /// An unsupported format, or too many consecutive bad frames.
    UnsupportedFrame,
    /// The encoding task failed.
    Internal,
}

/// One fixed-text debug line per end reason (never a field).
fn log_end(end: ViewEnd) {
    match end {
        ViewEnd::Stopped => debug!("live view closed: stop requested"),
        ViewEnd::MaxDuration => debug!("live view closed: maximum duration"),
        ViewEnd::ClientClosed => debug!("live view closed: client gone"),
        ViewEnd::WriteFailed => debug!("live view closed: part write failed"),
        ViewEnd::SessionExpired => debug!("live view closed: web session gone"),
        ViewEnd::Shutdown => debug!("live view closed: server shutdown"),
        ViewEnd::DaemonRefused => debug!("live view closed: daemon refusal"),
        ViewEnd::CameraUnavailable => debug!("live view closed: no frame"),
        ViewEnd::UnsupportedFrame => debug!("live view closed: unsupported frame"),
        ViewEnd::Internal => debug!("live view closed: internal failure"),
    }
}

/// Shared camera state of the server.
pub struct CameraRuntime {
    settings: CameraSettings,
    factory: Option<PreviewSourceFactory>,
    slot: Mutex<ViewSlot>,
    stop_epoch: watch::Sender<u64>,
    /// Origin of the refused-audit gate.
    origin: Instant,
    /// 1 + milliseconds since `origin` of the last `camera view refused` line; 0 = never.
    refused_at: AtomicU64,
}

impl CameraRuntime {
    /// A runtime with `settings`; `factory` is `None` when no preview source is wired.
    #[must_use]
    pub fn new(settings: CameraSettings, factory: Option<PreviewSourceFactory>) -> Self {
        Self {
            settings,
            factory,
            slot: Mutex::new(ViewSlot::new()),
            stop_epoch: watch::Sender::new(0),
            origin: Instant::now(),
            refused_at: AtomicU64::new(0),
        }
    }

    /// The resolved settings.
    #[must_use]
    pub fn settings(&self) -> &CameraSettings {
        &self.settings
    }

    /// A new preview source, `None` when no factory is wired.
    #[must_use]
    pub fn new_source(&self) -> Option<Box<dyn PreviewSource>> {
        self.factory.as_ref().map(|factory| factory())
    }

    /// Pre-check of the slot (a poisoned lock is `Busy`).
    ///
    /// # Errors
    ///
    /// [`SlotError`].
    pub fn check(&self, now: Instant) -> Result<(), SlotError> {
        match self.slot.lock() {
            Ok(mut slot) => slot.check(now),
            Err(_) => Err(SlotError::Busy),
        }
    }

    /// Reserves the slot with `token` for `owner`.
    ///
    /// # Errors
    ///
    /// [`SlotError`].
    pub fn reserve(
        &self,
        now: Instant,
        owner: ViewOwner,
        token: ViewToken,
    ) -> Result<(), SlotError> {
        match self.slot.lock() {
            Ok(mut slot) => slot.reserve(now, owner, token),
            Err(_) => Err(SlotError::Busy),
        }
    }

    /// Consumes the token and subscribes to the stop epoch inside the same critical section.
    ///
    /// # Errors
    ///
    /// [`SlotError`].
    pub fn begin(
        &self,
        now: Instant,
        owner: ViewOwner,
        presented: &ViewToken,
    ) -> Result<(ViewTicket, watch::Receiver<u64>), SlotError> {
        match self.slot.lock() {
            Ok(mut slot) => {
                let ticket = slot.begin(now, owner, presented, self.settings.max_view)?;
                Ok((ticket, self.stop_epoch.subscribe()))
            }
            Err(_) => Err(SlotError::Busy),
        }
    }

    /// Marks `view` as streaming.
    pub fn mark_streaming(&self, view: u64) {
        if let Ok(mut slot) = self.slot.lock() {
            slot.mark_streaming(view);
        }
    }

    /// Stops what the slot holds; bumps the stop epoch when something was stopped.
    pub fn stop(&self) -> bool {
        let stopped = match self.slot.lock() {
            Ok(mut slot) => slot.stop(),
            Err(_) => false,
        };
        if stopped {
            self.stop_epoch
                .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
        }
        stopped
    }

    /// Whether `view` must stop (a poisoned lock reads as stopped).
    #[must_use]
    pub fn stop_requested(&self, view: u64) -> bool {
        self.slot
            .lock()
            .map_or(true, |slot| slot.stop_requested(view))
    }

    /// Phase and remaining cooldown (a poisoned lock reads as idle).
    #[must_use]
    pub fn phase(&self, now: Instant) -> (SlotPhase, Option<u64>) {
        self.slot
            .lock()
            .map_or((SlotPhase::Idle, None), |slot| slot.phase(now))
    }

    /// Ends `view`.
    fn end(&self, now: Instant, view: u64, shown: bool) {
        if let Ok(mut slot) = self.slot.lock() {
            slot.end(now, view, shown);
        }
    }

    /// Emits `camera view refused`, at most once per `CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS`.
    pub fn audit_refused(&self) {
        let now_ms = u64::try_from(
            Instant::now()
                .saturating_duration_since(self.origin)
                .as_millis(),
        )
        .unwrap_or(u64::MAX)
        .saturating_add(1);
        let last = self.refused_at.load(Ordering::SeqCst);
        if last != 0 && now_ms.saturating_sub(last) < CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS {
            return;
        }
        if self
            .refused_at
            .compare_exchange(last, now_ms, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            audit::camera_view_refused();
        }
    }
}

/// Frees the slot of one started view on every exit path.
pub struct ViewGuard {
    runtime: Arc<CameraRuntime>,
    view: u64,
    shown: bool,
}

impl ViewGuard {
    /// The guard of `view`.
    #[must_use]
    pub fn new(runtime: Arc<CameraRuntime>, view: u64) -> Self {
        Self {
            runtime,
            view,
            shown: false,
        }
    }

    /// Records that pixels were shown (cooldown and `camera view ended` at the end).
    pub fn set_shown(&mut self) {
        self.shown = true;
    }
}

impl Drop for ViewGuard {
    fn drop(&mut self) {
        self.runtime.end(Instant::now(), self.view, self.shown);
        if self.shown {
            audit::camera_view_ended();
        }
    }
}

/// Result of one frame step.
pub(crate) enum StepOutcome {
    /// A JPEG ready to be written, with its frame sequence.
    Frame(JpegFrame, u64),
    /// The explicit empty preview (camera waking).
    Waiting,
    /// The same sequence as the last sent frame (never re-encoded).
    Skip,
    /// A geometry or encode failure of one frame.
    Bad,
    /// A format that is never converted.
    Unsupported,
    /// The daemon answered with an error.
    Failed(PreviewError),
    /// The encoding task failed.
    Internal,
}

/// A frame step future: it owns the source and hands it back with its outcome.
type FrameStep = Pin<Box<dyn Future<Output = (Box<dyn PreviewSource>, StepOutcome)> + Send>>;

/// Sleeps until `at`, fetches one frame and encodes it on the blocking pool.
fn frame_step(
    mut source: Box<dyn PreviewSource>,
    at: Instant,
    last_sent: Option<u64>,
    jpeg: JpegSettings,
) -> FrameStep {
    Box::pin(async move {
        sleep_until(at).await;
        let fetched = source.next_frame().await;
        let outcome = match fetched {
            Err(err) => StepOutcome::Failed(err),
            Ok(frame) => {
                if frame.format == PREVIEW_FORMAT_EMPTY || frame.data.is_empty() {
                    StepOutcome::Waiting
                } else if last_sent == Some(frame.sequence) {
                    StepOutcome::Skip
                } else if ![
                    PREVIEW_FORMAT_GREY,
                    PREVIEW_FORMAT_YUYV,
                    PREVIEW_FORMAT_RGB24,
                ]
                .contains(&frame.format)
                {
                    StepOutcome::Unsupported
                } else {
                    let sequence = frame.sequence;
                    let encoded = tokio::task::spawn_blocking(move || {
                        encode_preview_jpeg(
                            frame.format,
                            frame.width,
                            frame.height,
                            &frame.data,
                            jpeg,
                        )
                    })
                    .await;
                    match encoded {
                        Ok(Ok(jpeg)) => StepOutcome::Frame(jpeg, sequence),
                        Ok(Err(FrameError::UnsupportedFormat)) => StepOutcome::Unsupported,
                        Ok(Err(_)) => StepOutcome::Bad,
                        Err(_) => StepOutcome::Internal,
                    }
                }
            }
        };
        (source, outcome)
    })
}

/// Cadence and back-off of the frame requests.
pub(crate) struct Pacer {
    interval: Duration,
    tick: Instant,
    backoff_ms: u64,
}

impl Pacer {
    fn new(settings: &CameraSettings, now: Instant) -> Self {
        Self {
            interval: settings.frame_interval(),
            tick: now,
            backoff_ms: CAMERA_DAEMON_BACKOFF_MIN_MS,
        }
    }

    /// The next tick after a completed step: `max(tick + interval, now)` (no burst
    /// catch-up), later after a failure.
    fn next(&mut self, outcome: &StepOutcome, now: Instant) -> Instant {
        let regular = self.tick.checked_add(self.interval).unwrap_or(now).max(now);
        let at = match outcome {
            StepOutcome::Frame(..) => {
                self.backoff_ms = CAMERA_DAEMON_BACKOFF_MIN_MS;
                regular
            }
            StepOutcome::Failed(PreviewError::RateLimited) => regular.max(
                now.checked_add(Duration::from_millis(CAMERA_RATE_LIMITED_BACKOFF_MS))
                    .unwrap_or(now),
            ),
            StepOutcome::Failed(_) => {
                let delay = self.backoff_ms;
                self.backoff_ms = self
                    .backoff_ms
                    .saturating_mul(2)
                    .min(CAMERA_DAEMON_BACKOFF_MAX_MS);
                regular.max(now.checked_add(Duration::from_millis(delay)).unwrap_or(now))
            }
            _ => regular,
        };
        self.tick = at;
        at
    }
}

/// Re-validation of a Funnel web session (revocation first, `Touch::Keep`).
pub(crate) type SessionCheck =
    Box<dyn Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// Everything a view loop needs besides the socket halves.
pub(crate) struct ViewContext<'a> {
    /// The camera runtime.
    pub(crate) runtime: &'a CameraRuntime,
    /// The started view.
    pub(crate) ticket: ViewTicket,
    /// Stop epoch receiver subscribed at `begin`.
    pub(crate) stop_rx: watch::Receiver<u64>,
    /// Shutdown signal of the server.
    pub(crate) closing: watch::Receiver<bool>,
    /// `Some` on the Funnel: re-validates the web session.
    pub(crate) session_check: Option<SessionCheck>,
}

/// Outcome of the first-frame phase.
pub(crate) enum FirstFrame {
    /// The first JPEG and the source to keep using.
    Ready(JpegFrame, u64, Box<dyn PreviewSource>, Pacer),
    /// The view ended before any head byte.
    Ended(ViewEnd),
}

/// A select arm result of a view loop.
enum Arm {
    End(ViewEnd),
    Continue,
    Step(Box<dyn PreviewSource>, StepOutcome),
    SessionChecked(bool),
}

impl ViewContext<'_> {
    /// The outcome of a stop epoch change: `End(Stopped)` when the view must stop,
    /// `End(Shutdown)` when the sender is gone (plan O3).
    fn on_stop(&self, changed: Result<(), watch::error::RecvError>) -> Arm {
        match changed {
            Err(_) => Arm::End(ViewEnd::Shutdown),
            Ok(()) if self.runtime.stop_requested(self.ticket.view) => Arm::End(ViewEnd::Stopped),
            Ok(()) => Arm::Continue,
        }
    }

    /// Fetches frames until the first JPEG, bounded by `deadline`, without writing a byte.
    pub(crate) async fn first_frame(
        &mut self,
        read_half: &mut ReadHalf<'_>,
        source: Box<dyn PreviewSource>,
        deadline: Instant,
    ) -> FirstFrame {
        let settings = *self.runtime.settings();
        let mut pacer = Pacer::new(&settings, Instant::now());
        let mut step = frame_step(source, pacer.tick, None, settings.jpeg);
        let mut sink = [0u8; VIEW_SINK_BYTES];
        let mut bad = 0u32;
        let mut session_at = Instant::now()
            .checked_add(Duration::from_millis(CAMERA_SESSION_CHECK_MS))
            .unwrap_or_else(Instant::now);
        loop {
            if self.runtime.stop_requested(self.ticket.view) {
                return FirstFrame::Ended(ViewEnd::Stopped);
            }
            let arm = tokio::select! {
                biased;
                closed = self.closing.changed() => {
                    if closed.is_err() || *self.closing.borrow_and_update() {
                        Arm::End(ViewEnd::Shutdown)
                    } else {
                        Arm::Continue
                    }
                }
                changed = self.stop_rx.changed() => self.on_stop(changed),
                read = read_half.read(&mut sink) => match read {
                    Ok(0) | Err(_) => Arm::End(ViewEnd::ClientClosed),
                    Ok(_) => Arm::Continue,
                },
                () = sleep_until(deadline) => Arm::End(ViewEnd::CameraUnavailable),
                () = sleep_until(session_at), if self.session_check.is_some() => {
                    match self.session_check.as_ref() {
                        Some(check) => Arm::SessionChecked(check().await),
                        None => Arm::Continue,
                    }
                }
                (back, outcome) = &mut step => Arm::Step(back, outcome),
            };
            match arm {
                Arm::End(end) => return FirstFrame::Ended(end),
                Arm::Continue => {}
                Arm::SessionChecked(true) => {
                    session_at = Instant::now()
                        .checked_add(Duration::from_millis(CAMERA_SESSION_CHECK_MS))
                        .unwrap_or_else(Instant::now);
                }
                Arm::SessionChecked(false) => return FirstFrame::Ended(ViewEnd::SessionExpired),
                Arm::Step(back, outcome) => {
                    let at = pacer.next(&outcome, Instant::now());
                    match outcome {
                        StepOutcome::Frame(jpeg, sequence) => {
                            return FirstFrame::Ready(jpeg, sequence, back, pacer);
                        }
                        StepOutcome::Failed(PreviewError::Refused) => {
                            return FirstFrame::Ended(ViewEnd::DaemonRefused);
                        }
                        StepOutcome::Unsupported => {
                            return FirstFrame::Ended(ViewEnd::UnsupportedFrame);
                        }
                        StepOutcome::Internal => return FirstFrame::Ended(ViewEnd::Internal),
                        StepOutcome::Bad => {
                            bad = bad.saturating_add(1);
                            if bad >= CAMERA_MAX_BAD_FRAMES {
                                return FirstFrame::Ended(ViewEnd::UnsupportedFrame);
                            }
                        }
                        StepOutcome::Failed(_) | StepOutcome::Waiting | StepOutcome::Skip => {}
                    }
                    step = frame_step(back, at, None, settings.jpeg);
                }
            }
        }
    }

    /// Streams parts until an end condition; the first part was already written at
    /// `last_sent_at`. Every part is one bounded write of the whole part.
    pub(crate) async fn stream(
        &mut self,
        read_half: &mut ReadHalf<'_>,
        write_half: &mut WriteHalf<'_>,
        start: (Box<dyn PreviewSource>, Pacer, u64),
        last_sent_at: Instant,
    ) -> ViewEnd {
        let settings = *self.runtime.settings();
        let (source, mut pacer, first_sequence) = start;
        let mut last_sent = Some(first_sequence);
        let mut last_sent_at = last_sent_at;
        let mut step = frame_step(source, pacer.tick, last_sent, settings.jpeg);
        let mut sink = [0u8; VIEW_SINK_BYTES];
        let mut bad = 0u32;
        let stall = Duration::from_millis(CAMERA_STALL_TIMEOUT_MS);
        let mut session_at = Instant::now()
            .checked_add(Duration::from_millis(CAMERA_SESSION_CHECK_MS))
            .unwrap_or_else(Instant::now);
        loop {
            if self.runtime.stop_requested(self.ticket.view) {
                return ViewEnd::Stopped;
            }
            let stall_at = last_sent_at.checked_add(stall).unwrap_or(last_sent_at);
            let arm = tokio::select! {
                biased;
                closed = self.closing.changed() => {
                    if closed.is_err() || *self.closing.borrow_and_update() {
                        Arm::End(ViewEnd::Shutdown)
                    } else {
                        Arm::Continue
                    }
                }
                changed = self.stop_rx.changed() => self.on_stop(changed),
                read = read_half.read(&mut sink) => match read {
                    Ok(0) | Err(_) => Arm::End(ViewEnd::ClientClosed),
                    Ok(_) => Arm::Continue,
                },
                () = sleep_until(self.ticket.ends_at) => Arm::End(ViewEnd::MaxDuration),
                () = sleep_until(session_at), if self.session_check.is_some() => {
                    match self.session_check.as_ref() {
                        Some(check) => Arm::SessionChecked(check().await),
                        None => Arm::Continue,
                    }
                }
                () = sleep_until(stall_at) => Arm::End(ViewEnd::CameraUnavailable),
                (back, outcome) = &mut step => Arm::Step(back, outcome),
            };
            match arm {
                Arm::End(end) => return end,
                Arm::Continue => {}
                Arm::SessionChecked(true) => {
                    session_at = Instant::now()
                        .checked_add(Duration::from_millis(CAMERA_SESSION_CHECK_MS))
                        .unwrap_or_else(Instant::now);
                }
                Arm::SessionChecked(false) => return ViewEnd::SessionExpired,
                Arm::Step(back, outcome) => {
                    match &outcome {
                        StepOutcome::Frame(jpeg, sequence) => {
                            if write_part(write_half, jpeg).await.is_err() {
                                return ViewEnd::WriteFailed;
                            }
                            last_sent = Some(*sequence);
                            last_sent_at = Instant::now();
                            bad = 0;
                        }
                        StepOutcome::Failed(PreviewError::Refused) => {
                            return ViewEnd::DaemonRefused;
                        }
                        StepOutcome::Unsupported => return ViewEnd::UnsupportedFrame,
                        StepOutcome::Internal => return ViewEnd::Internal,
                        StepOutcome::Bad => {
                            bad = bad.saturating_add(1);
                            if bad >= CAMERA_MAX_BAD_FRAMES {
                                return ViewEnd::UnsupportedFrame;
                            }
                        }
                        StepOutcome::Failed(_) | StepOutcome::Waiting | StepOutcome::Skip => {}
                    }
                    let at = pacer.next(&outcome, Instant::now());
                    step = frame_step(back, at, last_sent, settings.jpeg);
                }
            }
        }
    }
}

/// Writes one whole part (`--soosframe` head, the JPEG, CRLF) as one bounded write of a
/// zeroized buffer allocated at its final size.
///
/// # Errors
///
/// The write failure.
pub(crate) async fn write_part(
    write_half: &mut WriteHalf<'_>,
    jpeg: &JpegFrame,
) -> Result<(), crate::http::WriteError> {
    let head = encode_camera_part_head(jpeg.bytes.len());
    let total = head
        .len()
        .saturating_add(jpeg.bytes.len())
        .saturating_add(2);
    let mut part = Zeroizing::new(Vec::with_capacity(total));
    part.extend_from_slice(&head);
    part.extend_from_slice(&jpeg.bytes);
    part.extend_from_slice(b"\r\n");
    write_bounded(write_half, &part).await
}

/// Ends a view: one debug line, then the best-effort trailer when the client may still read.
pub(crate) async fn finish_view(write_half: &mut WriteHalf<'_>, end: ViewEnd) {
    log_end(end);
    if !matches!(
        end,
        ViewEnd::ClientClosed | ViewEnd::WriteFailed | ViewEnd::Shutdown
    ) {
        let _ = write_bounded(write_half, CAMERA_STREAM_TRAILER).await;
    }
}

/// Logs the end of a view that never showed pixels.
pub(crate) fn log_unshown_end(end: ViewEnd) {
    log_end(end);
}
