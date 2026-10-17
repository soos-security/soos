//! Wayland layer-shell loop: the only module touching the compositor.
//!
//! swaylock-plugin runs this program with a private Wayland connection (`WAYLAND_SOCKET`) and
//! expects "a full-size background layer surface for each Wayland output" (`swaylock-plugin(1)`,
//! `--command`); it restarts the program whenever it closes that connection. The widget therefore:
//!
//! - binds `wl_compositor`, `wl_shm` and `zwlr_layer_shell_v1` only, and **no seat** (no keyboard,
//!   pointer or touch object: it can neither read nor send input);
//! - creates one background layer surface per output (at most `MAX_OUTPUTS`), anchored to every
//!   edge, `KeyboardInteractivity::None`;
//! - draws with one shared-memory buffer per surface, and a second one only while the compositor
//!   still holds the first ([`crate::layout::needs_extra_buffer`], at most
//!   [`crate::layout::MAX_BUFFERS_PER_SURFACE`]), within the pixel budgets of [`crate::layout`];
//!   each buffer has its own pool of exactly [`crate::layout::slot_len`] bytes (no pool ever
//!   grows, so a failed allocation leaves nothing behind and is retried like a busy buffer);
//!   a buffer is painted fully once and then only the widget box;
//! - redraws every `AMBIENT_REPAINT_MS` only while the scene animates (idle draws nothing);
//! - reads the lock state on one poller thread through `soos-lockstatus` (coordinated per user).

use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::TrySendError;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{
    self, Event as ChannelEvent, SyncSender,
};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    registry_handlers,
};
use soos_lockstatus::motion::{MotionPrefs, AMBIENT_REPAINT_MS};
use soos_lockstatus::poll::Poller;
use soos_lockstatus::view::LockView;

use crate::layout::{
    buffer_len, needs_extra_buffer, slot_len, Layout, Rect, MAX_OUTPUTS, MAX_TOTAL_PIXELS,
};
use crate::raster::Canvas;
use crate::render::{paint_full, paint_widget};
use crate::scene::Scene;

/// Delay before every exit 1, so that swaylock-plugin (which restarts its background program
/// whenever it closes the connection) restarts a broken setup at most about once per second.
pub const SETUP_FAILURE_EXIT_DELAY_MS: u64 = 1000;

/// Layer-shell namespace of the widget surfaces.
pub const LAYER_NAMESPACE: &str = "soos-lock-widget";

/// Views queued from the poller thread at most (the thread keeps the newest unsent one).
const CHANNEL_BOUND: usize = 4;

/// Why the widget stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The compositor closed every surface: a normal end (exit 0 after
    /// [`SETUP_FAILURE_EXIT_DELAY_MS`], like [`Exit::Disconnected`]).
    Closed,
    /// The Wayland connection failed or was closed (the lock normally ended): exit 0, after
    /// [`SETUP_FAILURE_EXIT_DELAY_MS`] so that a connection broken for another reason cannot make
    /// swaylock-plugin restart the widget in a tight loop.
    Disconnected,
    /// The Wayland setup failed (exit 1 after [`SETUP_FAILURE_EXIT_DELAY_MS`]).
    Setup(&'static str),
    /// The lock-state poller stopped (exit 1 after the same delay, so that swaylock-plugin
    /// restarts the widget with a fresh poller).
    PollerGone,
}

/// Configuration of one run.
#[derive(Debug, Clone)]
pub struct WidgetConfig {
    /// Daemon socket.
    pub socket: PathBuf,
    /// `$XDG_RUNTIME_DIR` (coordination of the lock-status clients of the user).
    pub runtime_dir: Option<PathBuf>,
    /// Caller UID.
    pub uid: u32,
    /// Requested poll interval (clamped by the poller).
    pub interval_ms: u64,
    /// Resolved motion preference.
    pub prefs: MotionPrefs,
}

/// One shared-memory buffer of the configured size in its own pool.
struct ShmBuffer {
    /// Declared before `pool`: dropped first.
    buffer: Buffer,
    pool: SlotPool,
    /// Whether the buffer was painted fully once.
    painted: bool,
}

impl ShmBuffer {
    /// One ARGB8888 buffer of `width` × `height` in a pool of exactly [`slot_len`] bytes; `None`
    /// on any failure (nothing is kept).
    fn new(shm: &Shm, width: u32, height: u32) -> Option<Self> {
        let len = slot_len(width, height)?;
        let w = i32::try_from(width).ok()?;
        let h = i32::try_from(height).ok()?;
        let stride = i32::try_from(u64::from(width).saturating_mul(4)).ok()?;
        let mut pool = SlotPool::new(len, shm).ok()?;
        let (buffer, _) = pool
            .create_buffer(w, h, stride, wl_shm::Format::Argb8888)
            .ok()?;
        Some(Self {
            buffer,
            pool,
            painted: false,
        })
    }

    /// Whether the compositor no longer holds the buffer (it can be drawn).
    fn is_free(&mut self) -> bool {
        self.buffer.canvas(&mut self.pool).is_some()
    }
}

/// One output's layer surface and its buffers.
struct OutputSurface {
    output: wl_output::WlOutput,
    layer: LayerSurface,
    /// Buffers of the configured size (one after configure, at most `MAX_BUFFERS_PER_SURFACE`).
    buffers: Vec<ShmBuffer>,
    layout: Option<Layout>,
    pixels: u64,
}

struct Widget {
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    surfaces: Vec<OutputSurface>,
    scene: Scene,
    started: Instant,
    handle: LoopHandle<'static, Widget>,
    timer_armed: bool,
    draw_requested: bool,
    outputs_reported: bool,
    skip_reported: bool,
    exit: Option<Exit>,
}

/// Runs the widget until the compositor closes it or a fatal error occurs.
#[must_use]
pub fn run(config: &WidgetConfig) -> Exit {
    match setup_and_run(config) {
        Ok(exit) | Err(exit) => exit,
    }
}

/// Whether `WAYLAND_SOCKET`, when set, names an open socket descriptor of this process.
///
/// `Connection::connect_to_env` adopts that descriptor as is; a stale number would make it own
/// (and later close) a descriptor that does not exist, which the standard library treats as a
/// fatal I/O-safety violation. Checked through `/proc/self/fd` with the standard library only.
fn wayland_socket_is_valid() -> bool {
    use std::os::unix::fs::FileTypeExt;
    let Some(value) = std::env::var_os("WAYLAND_SOCKET") else {
        return true;
    };
    let Some(fd) = value.to_str().and_then(|v| v.parse::<u32>().ok()) else {
        return false;
    };
    std::fs::metadata(format!("/proc/self/fd/{fd}")).is_ok_and(|meta| meta.file_type().is_socket())
}

fn setup_and_run(config: &WidgetConfig) -> Result<Exit, Exit> {
    if !wayland_socket_is_valid() {
        return Err(Exit::Setup("WAYLAND_SOCKET is not an open socket"));
    }
    let conn = Connection::connect_to_env()
        .map_err(|_| Exit::Setup("cannot connect to the Wayland compositor"))?;
    let (globals, queue) =
        registry_queue_init::<Widget>(&conn).map_err(|_| Exit::Setup("Wayland registry failed"))?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|_| Exit::Setup("wl_compositor is not available"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|_| Exit::Setup("wlr-layer-shell is not available"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|_| Exit::Setup("wl_shm is not available"))?;
    let mut event_loop: EventLoop<'static, Widget> =
        EventLoop::try_new().map_err(|_| Exit::Setup("event loop creation failed"))?;
    let handle = event_loop.handle();
    WaylandSource::new(conn, queue)
        .insert(handle.clone())
        .map_err(|_| Exit::Setup("Wayland event source failed"))?;

    let (sender, receiver) = channel::sync_channel::<LockView>(CHANNEL_BOUND);
    handle
        .insert_source(receiver, |event, (), widget: &mut Widget| match event {
            ChannelEvent::Msg(view) => {
                let now = widget.now_ms();
                widget.scene.update(view, now);
                widget.draw_requested = true;
            }
            ChannelEvent::Closed => {
                widget.scene.update(LockView::UNAVAILABLE, widget.now_ms());
                widget.draw_requested = true;
                widget.exit = Some(Exit::PollerGone);
            }
        })
        .map_err(|_| Exit::Setup("poller channel failed"))?;
    spawn_poller(config, sender).map_err(|()| Exit::PollerGone)?;

    let started = Instant::now();
    let mut widget = Widget {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        surfaces: Vec::new(),
        scene: Scene::new(config.prefs, 0),
        started,
        handle,
        timer_armed: false,
        draw_requested: false,
        outputs_reported: false,
        skip_reported: false,
        exit: None,
    };
    loop {
        if event_loop.dispatch(None, &mut widget).is_err() {
            // Normally swaylock-plugin went away because the lock ended.
            return Ok(Exit::Disconnected);
        }
        if widget.draw_requested {
            widget.draw_requested = false;
            widget.redraw();
        }
        if let Some(exit) = widget.exit {
            return Ok(exit);
        }
    }
}

/// Starts the poller thread: it sends only views that differ from the last one delivered, never
/// blocks on the channel (a full channel retries with the newest poll, so a stale view is never
/// delivered late); it ends when the widget drops the receiver.
fn spawn_poller(config: &WidgetConfig, sender: SyncSender<LockView>) -> Result<(), ()> {
    let mut poller = Poller::new(
        config.socket.clone(),
        config.runtime_dir.clone(),
        config.uid,
        config.interval_ms,
    );
    std::thread::Builder::new()
        .name("soos-lock-poll".to_string())
        .spawn(move || {
            let mut sent: Option<LockView> = None;
            loop {
                let (view, delay) = poller.poll();
                if sent != Some(view) {
                    match sender.try_send(view) {
                        Ok(()) => sent = Some(view),
                        Err(TrySendError::Full(_)) => {}
                        Err(TrySendError::Disconnected(_)) => return,
                    }
                }
                std::thread::sleep(delay);
            }
        })
        .map(|_| ())
        .map_err(|_| ())
}

impl Widget {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn create_surface(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if self.surfaces.len() >= MAX_OUTPUTS {
            if !self.outputs_reported {
                // Once per run, never the state.
                self.outputs_reported = true;
                let _ignored = writeln!(
                    std::io::stderr(),
                    "soos-lock-widget: more than {MAX_OUTPUTS} outputs; the others show no widget"
                );
            }
            return;
        }
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Background,
            Some(LAYER_NAMESPACE),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        self.surfaces.push(OutputSurface {
            output,
            layer,
            buffers: Vec::new(),
            layout: None,
            pixels: 0,
        });
    }

    /// (Re)allocates the first buffer of surface `index` for `width` × `height` within the
    /// budgets (a second one is allocated by [`Self::draw_all`] only when needed); a surface over a
    /// budget (or whose shared memory cannot be allocated) keeps no buffer and is not drawn.
    /// Returns whether the surface can be drawn.
    fn configure_surface(&mut self, index: usize, width: u32, height: u32) -> bool {
        let others = self
            .surfaces
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, s)| s.pixels)
            .fold(0u64, u64::saturating_add);
        let shm = &self.shm;
        let Some(surface) = self.surfaces.get_mut(index) else {
            return true;
        };
        if surface
            .layout
            .is_some_and(|l| l.width == width && l.height == height)
        {
            return true;
        }
        surface.buffers.clear();
        surface.layout = None;
        surface.pixels = 0;
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        if buffer_len(width, height).is_none() {
            return false;
        }
        if others.saturating_add(pixels) > MAX_TOTAL_PIXELS {
            return false;
        }
        let Some(first) = ShmBuffer::new(shm, width, height) else {
            return false;
        };
        surface.buffers.push(first);
        surface.layout = Layout::for_surface(width, height);
        surface.pixels = pixels;
        surface.layout.is_some()
    }

    /// Draws the current frame on every surface; returns whether a surface had no free buffer
    /// (a retry is needed). A released buffer is reused; when the compositor still holds every
    /// buffer and the policy allows, a second one is allocated (painted fully on its first use); a
    /// failed allocation is treated like a busy buffer.
    fn draw_all(&mut self) -> bool {
        let frame = self.scene.frame(self.now_ms());
        let mut retry = false;
        let shm = &self.shm;
        for surface in &mut self.surfaces {
            let Some(layout) = surface.layout else {
                continue;
            };
            let mut free = surface.buffers.iter_mut().position(ShmBuffer::is_free);
            if needs_extra_buffer(surface.buffers.len(), free.is_some()) {
                if let Some(extra) = ShmBuffer::new(shm, layout.width, layout.height) {
                    surface.buffers.push(extra);
                    free = surface.buffers.len().checked_sub(1);
                }
            }
            let Some(ShmBuffer {
                buffer,
                pool,
                painted,
            }) = free.and_then(|index| surface.buffers.get_mut(index))
            else {
                retry = true;
                continue;
            };
            let Some(mut canvas) = buffer
                .canvas(pool)
                .and_then(|bytes| Canvas::new(bytes, layout.width, layout.height))
            else {
                retry = true;
                continue;
            };
            let wl_surface = surface.layer.wl_surface();
            let damage = if *painted {
                paint_widget(&mut canvas, &layout, &frame);
                layout.widget
            } else {
                paint_full(&mut canvas, &layout, &frame);
                Rect {
                    x: 0,
                    y: 0,
                    w: layout.width,
                    h: layout.height,
                }
            };
            *painted = true;
            let coord = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
            wl_surface.damage_buffer(
                coord(damage.x),
                coord(damage.y),
                coord(damage.w),
                coord(damage.h),
            );
            if buffer.attach_to(wl_surface).is_err() {
                retry = true;
                continue;
            }
            surface.layer.commit();
        }
        retry
    }

    /// Draws now and arms the frame timer while the scene animates or a buffer was busy; once
    /// settled no timer runs (idle = zero frames).
    fn redraw(&mut self) {
        let retry = self.draw_all();
        let animating = self.scene.frame(self.now_ms()).animating;
        if (animating || retry) && !self.timer_armed {
            let timer = Timer::from_duration(Duration::from_millis(AMBIENT_REPAINT_MS));
            let inserted = self
                .handle
                .insert_source(timer, |_, (), widget: &mut Widget| {
                    let retry = widget.draw_all();
                    if retry || widget.scene.frame(widget.now_ms()).animating {
                        TimeoutAction::ToDuration(Duration::from_millis(AMBIENT_REPAINT_MS))
                    } else {
                        widget.timer_armed = false;
                        TimeoutAction::Drop
                    }
                });
            self.timer_armed = inserted.is_ok();
        }
    }
}

impl CompositorHandler for Widget {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
        // Buffers stay at scale 1 (logical size); a HiDPI compositor upscales them.
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        // Frames are paced by the calloop timer, never by frame callbacks.
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Widget {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.create_surface(qh, output);
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.surfaces.retain(|s| s.output != output);
    }
}

impl LayerShellHandler for Widget {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.surfaces.retain(|s| &s.layer != layer);
        if self.surfaces.is_empty() {
            self.exit = Some(Exit::Closed);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(index) = self.surfaces.iter().position(|s| &s.layer == layer) else {
            return;
        };
        let (width, height) = configure.new_size;
        if !self.configure_surface(index, width, height) && !self.skip_reported {
            // Once per run, never the state: the output keeps swaylock-plugin's background.
            self.skip_reported = true;
            let _ignored = writeln!(
                std::io::stderr(),
                "soos-lock-widget: an output of {width}x{height} cannot be drawn (zero size, \
                 over the pixel budget or out of shared memory); it shows no widget"
            );
        }
        // The surface needs a buffer before it is shown: drawn right after this dispatch.
        self.draw_requested = true;
    }
}

impl ShmHandler for Widget {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Widget {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_compositor!(Widget);
delegate_output!(Widget);
delegate_shm!(Widget);
delegate_layer!(Widget);
delegate_registry!(Widget);
