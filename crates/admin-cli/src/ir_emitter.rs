//! `soos-admin camera ir-emitter probe|configure` (GitHub #378, PR 1 of #353; ADR 2026-10-13
//! "UVC Extension-Unit IR Emitter Control and Strobe Classification").
//!
//! - `probe` is read-only: the controls of an Infrared uvcvideo node are scanned with `GET_*`
//!   requests only, through [`XuReadOnly`], within the bounds of
//!   [`soos_camera_v4l::uvc_xu::probe_xu_controls`].
//! - `configure` validates a descriptor, checks the control length read-only and writes the
//!   managed `[camera.ir_emitter]` block of the daemon configuration. The device is written only
//!   with `--test-on-device` ([`test_emitter_on_device`]), which restores the original value on
//!   every path with the termination signals blocked; `mode = "auto"` is accepted only after
//!   such a test succeeded in the same run.
//!
//! The command never captures a frame and never parses TOML itself (the camera crate does).
//! It runs on a single thread, so the blocked signal mask is the process mask.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::ops::RangeInclusive;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use nix::sys::signal::{pthread_sigmask, SigSet, SigmaskHow, Signal};
use serde::Serialize;
use soos_camera_v4l::diagnostics::{probe_camera_node, sanitize_display_text, V4lDeviceProbe};
use soos_camera_v4l::ir_emitter::{EmitterState, IrEmitter as _, UvcXuEmitter};
use soos_camera_v4l::ir_emitter_config::{
    ir_emitter_section_present, parse_ir_emitter_text, read_daemon_ir_emitter_config,
    usb_device_id_of_node, IrEmitterMode, IrEmitterSettings, UsbDeviceId, XuEmitterDescriptor,
};
use soos_camera_v4l::uvc_xu::{
    probe_xu_controls, xu_get_info, xu_get_len, XuBudget, XuControl, XuReadOnly,
};
use soos_camera_v4l::SensorType;

use crate::camera::CameraEnvironment;
use crate::daemon_config::MAX_DAEMON_CONFIG_BYTES;
use crate::error::AdminCliError;

/// First line of the block written by `configure`.
pub const IR_EMITTER_BLOCK_BEGIN: &str = "# BEGIN soos-admin camera.ir_emitter";
/// Last line of the block written by `configure`.
pub const IR_EMITTER_BLOCK_END: &str = "# END soos-admin camera.ir_emitter";
/// Hard cap on the UVC requests of one `configure --test-on-device` run, enforced with
/// [`XuBudget`]: 2 read-only pre-checks (`GET_INFO`, `GET_LEN`), 3 to open the emitter
/// (`GET_INFO`, `GET_LEN`, `GET_CUR`), on, read back, off, read back, the restore and its
/// drop-time retry. This is the real maximum (candid review of #378).
pub const MAX_DEVICE_TEST_QUERIES: usize = 11;
/// Requests of the read-only pre-checks of `configure`.
const CONFIGURE_PRECHECK_QUERIES: usize = 2;
/// Driver whose nodes expose UVC extension units.
const UVC_DRIVER: &str = "uvcvideo";
/// Attempts at a fresh temporary file name.
const MAX_TEMP_ATTEMPTS: u32 = 16;

/// Displayable path: control characters and bidirectional overrides replaced.
fn sanitize_path_text(path: &Path) -> String {
    sanitize_display_text(&path.to_string_lossy())
}

fn ir_error(message: impl Into<String>) -> AdminCliError {
    AdminCliError::IrEmitter(message.into())
}

/// Environment of the command (injected in tests).
pub struct IrEmitterEnv<'a> {
    /// sysfs, `/dev` and by-id directories.
    pub camera: &'a CameraEnvironment,
    /// Node metadata and classification.
    pub probe: &'a dyn V4lDeviceProbe,
    /// Effective uid of the caller.
    pub euid: u32,
}

/// An IR node accepted by [`check_ir_node`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrNode {
    /// Kernel node path.
    pub node: PathBuf,
    /// USB id from sysfs (`None` when it cannot be read).
    pub usb_id: Option<UsbDeviceId>,
    /// Device number (`st_rdev`) of the classified node, read before and after the
    /// classification and equal both times (`None` when it cannot be read or changed).
    pub rdev: Option<u64>,
}

/// Device number of `path` (following links), `None` when it cannot be read.
fn device_number(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|meta| meta.rdev())
}

/// Proves that an opened descriptor (device number `opened`) is the node classified by
/// [`check_ir_node`] (`expected`).
///
/// # Errors
///
/// [`AdminCliError::IrEmitter`] when the classified device number is unknown or differs.
pub fn ensure_same_node(expected: Option<u64>, opened: u64) -> Result<(), AdminCliError> {
    match expected {
        Some(rdev) if rdev == opened => Ok(()),
        Some(_) => Err(ir_error(
            "the camera node changed between its classification and its opening; nothing was done",
        )),
        None => Err(ir_error(
            "the device number of the classified camera node cannot be read; nothing was done",
        )),
    }
}

/// Accepts only root, and an Infrared capture node of the uvcvideo driver.
///
/// # Errors
///
/// [`AdminCliError::IrEmitter`] naming the refused condition.
pub fn check_ir_node(env: &IrEmitterEnv<'_>, device: &Path) -> Result<IrNode, AdminCliError> {
    if env.euid != 0 {
        return Err(ir_error(
            "this command needs root (use sudo): it opens the camera node for UVC requests",
        ));
    }
    let rdev_before = device_number(device);
    let node = probe_camera_node(device, &env.camera.aliases(), env.probe);
    let shown = sanitize_path_text(device);
    let Some(details) = node.details.as_ref() else {
        return Err(ir_error(format!("{shown} could not be probed")));
    };
    if node.sensor_type != Some(SensorType::Infrared) {
        return Err(ir_error(format!(
            "{shown} is not a capture node classified as an Infrared sensor; pick the IR node \
             listed by `soos-admin camera list`"
        )));
    }
    if details.driver != UVC_DRIVER {
        return Err(ir_error(format!(
            "{shown} is not driven by uvcvideo; extension-unit controls are UVC only"
        )));
    }
    let usb_id = node
        .node
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| usb_device_id_of_node(&env.camera.sysfs_dir, name));
    let rdev = match (rdev_before, device_number(&node.node)) {
        (Some(before), Some(after)) if before == after => Some(before),
        _ => None,
    };
    Ok(IrNode {
        node: node.node,
        usb_id,
        rdev,
    })
}

/// Lowercase hexadecimal without separators.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One control of a probe report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IrControlReport {
    /// Extension unit id.
    pub unit: u8,
    /// Control selector.
    pub selector: u8,
    /// Length in bytes.
    pub len: u16,
    /// GET supported.
    pub get: bool,
    /// SET supported.
    pub set: bool,
    /// Current value in hexadecimal (`None` when not read).
    pub current: Option<String>,
}

/// Result of `camera ir-emitter probe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IrProbeReport {
    /// Kernel node path (sanitized).
    pub node: String,
    /// USB id of the node.
    pub usb_id: Option<String>,
    /// Controls found.
    pub controls: Vec<IrControlReport>,
    /// Requests issued.
    pub queries_issued: usize,
    /// The request budget stopped the scan.
    pub truncated: bool,
    /// Error that stopped the scan.
    pub aborted: Option<String>,
    /// The configured `[camera.ir_emitter]` control, when one is configured.
    pub configured: Option<String>,
    /// The configured control is a control of this node, on this USB device, of the right
    /// length.
    pub configured_matches_node: bool,
}

impl IrProbeReport {
    /// Pretty JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Human-readable table.
    #[must_use]
    pub fn format_table(&self) -> String {
        let mut out = format!(
            "IR node:        {}\nUSB device:     {}\nRequests:       {}{}\n",
            self.node,
            self.usb_id.as_deref().unwrap_or("unknown"),
            self.queries_issued,
            if self.truncated {
                " (budget reached)"
            } else {
                ""
            }
        );
        if let Some(error) = &self.aborted {
            out.push_str(&format!("Scan stopped:   {error}\n"));
        }
        out.push_str(&format!(
            "Configured:     {}{}\n\nUNIT  SELECTOR  LEN  GET  SET  CURRENT\n",
            self.configured.as_deref().unwrap_or("none"),
            if self.configured_matches_node {
                " (matches this node)"
            } else {
                ""
            }
        ));
        for c in &self.controls {
            out.push_str(&format!(
                "{:<5} {:<9} {:<4} {:<4} {:<4} {}\n",
                c.unit,
                c.selector,
                c.len,
                if c.get { "yes" } else { "no" },
                if c.set { "yes" } else { "no" },
                c.current.as_deref().unwrap_or("-")
            ));
        }
        out
    }

    /// 1 when the scan stopped on an error before finding any control, 0 otherwise.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        i32::from(self.aborted.is_some() && self.controls.is_empty())
    }
}

/// Read-only probe of `device`: scans `units` with `GET_*` requests only and compares the
/// configured descriptor of `config_path` (if any) with this node.
///
/// # Errors
///
/// [`check_ir_node`] refusals.
pub fn probe_ir_emitter(
    env: &IrEmitterEnv<'_>,
    device: &Path,
    units: RangeInclusive<u8>,
    config_path: &Path,
    ctrl: &mut dyn XuControl,
) -> Result<IrProbeReport, AdminCliError> {
    let node = check_ir_node(env, device)?;
    let mut read_only = XuReadOnly::new(ctrl);
    let scan = probe_xu_controls(&mut read_only, units);
    let descriptor = read_daemon_ir_emitter_config(config_path)
        .ok()
        .and_then(|settings| settings.descriptor);
    let configured_matches_node = descriptor.as_ref().is_some_and(|d| {
        d.check_device(node.usb_id).is_ok()
            && scan.controls.iter().any(|c| {
                c.unit == d.unit && c.selector == d.selector && usize::from(c.len) == d.on.len()
            })
    });
    Ok(IrProbeReport {
        node: sanitize_path_text(&node.node),
        usb_id: node.usb_id.map(|id| id.to_string()),
        controls: scan
            .controls
            .iter()
            .map(|c| IrControlReport {
                unit: c.unit,
                selector: c.selector,
                len: c.len,
                get: c.info.supports_get(),
                set: c.info.supports_set(),
                current: c.current.as_deref().map(hex),
            })
            .collect(),
        queries_issued: scan.queries_issued,
        truncated: scan.truncated,
        aborted: scan.aborted.map(|e| e.to_string()),
        configured: descriptor.map(|d| {
            format!(
                "device {} unit {} selector {} ({} bytes)",
                d.device,
                d.unit,
                d.selector,
                d.on.len()
            )
        }),
        configured_matches_node,
    })
}

/// A `configure` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigureRequest {
    /// IR device.
    pub device: PathBuf,
    /// `mode` to write.
    pub mode: IrEmitterMode,
    /// Extension unit id.
    pub unit: u8,
    /// Control selector.
    pub selector: u8,
    /// Payload that lights the emitter.
    pub on: Vec<u8>,
    /// Payload that switches it off.
    pub off: Vec<u8>,
    /// Daemon configuration to update.
    pub config_path: PathBuf,
    /// Render only.
    pub dry_run: bool,
    /// Run [`test_emitter_on_device`] first.
    pub test_on_device: bool,
}

/// How the configuration file was changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// The file did not exist and was created (mode 0644).
    Created,
    /// The managed block was added or replaced.
    Updated,
}

/// Result of `--test-on-device`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceTestReport {
    /// Reading back after writing `on` returned `on`.
    pub on_readback_ok: bool,
    /// Reading back after writing `off` returned `off`.
    pub off_readback_ok: bool,
    /// The original value was written back.
    pub restored: bool,
}

/// Result of `configure`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigureReport {
    /// The `[camera.ir_emitter]` text.
    pub section: String,
    /// `None` for `--dry-run`.
    pub written: Option<WriteOutcome>,
    /// `Some` with `--test-on-device`.
    pub device_test: Option<DeviceTestReport>,
}

/// Validates the request, checks the control read-only, optionally tests it on the device, and
/// writes the managed block (unless `dry_run`).
///
/// # Errors
///
/// [`AdminCliError::IrEmitter`] for any refusal; nothing is written then.
pub fn configure_ir_emitter(
    env: &IrEmitterEnv<'_>,
    request: &ConfigureRequest,
    ctrl: &mut dyn XuControl,
) -> Result<ConfigureReport, AdminCliError> {
    let node = check_ir_node(env, &request.device)?;
    let usb_id = node.usb_id.ok_or_else(|| {
        ir_error("the USB id of this node cannot be read from sysfs; refusing to configure it")
    })?;
    let descriptor = XuEmitterDescriptor::new(
        usb_id,
        request.unit,
        request.selector,
        request.on.clone(),
        request.off.clone(),
    )
    .map_err(|e| ir_error(e.to_string()))?;
    if request.mode == IrEmitterMode::Auto && !request.test_on_device {
        return Err(ir_error(
            "--mode auto needs --test-on-device: an untested control is never enabled",
        ));
    }
    {
        let mut budget = XuBudget::new(ctrl, CONFIGURE_PRECHECK_QUERIES);
        let mut read_only = XuReadOnly::new(&mut budget);
        let info = xu_get_info(&mut read_only, request.unit, request.selector)
            .map_err(|e| ir_error(format!("GET_INFO of the control failed: {e}")))?;
        if !info.supports_get() || !info.supports_set() {
            return Err(ir_error(
                "the control cannot be both read and written (GET_INFO)",
            ));
        }
        let len = xu_get_len(&mut read_only, request.unit, request.selector)
            .map_err(|e| ir_error(format!("GET_LEN of the control failed: {e}")))?;
        if usize::from(len) != descriptor.on.len() {
            return Err(ir_error(format!(
                "the control holds {len} bytes but the payloads hold {}",
                descriptor.on.len()
            )));
        }
    }
    let device_test = if request.test_on_device {
        Some(test_emitter_on_device(ctrl, &descriptor, &mut || {})?)
    } else {
        None
    };
    let settings = IrEmitterSettings {
        mode: request.mode,
        descriptor: Some(descriptor),
    };
    let section = settings.to_toml_section();
    let written = if request.dry_run {
        None
    } else {
        Some(write_ir_emitter_section(&request.config_path, &settings)?)
    };
    Ok(ConfigureReport {
        section,
        written,
        device_test,
    })
}

/// Blocks the termination signals of the calling thread until dropped.
struct SignalBlock {
    previous: SigSet,
}

impl SignalBlock {
    fn new() -> Result<Self, AdminCliError> {
        let mut block = SigSet::empty();
        for signal in [
            Signal::SIGINT,
            Signal::SIGTERM,
            Signal::SIGHUP,
            Signal::SIGQUIT,
            Signal::SIGTSTP,
        ] {
            block.add(signal);
        }
        let mut previous = SigSet::empty();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&block), Some(&mut previous))
            .map_err(|e| ir_error(format!("cannot block termination signals: {e}")))?;
        Ok(Self { previous })
    }
}

impl Drop for SignalBlock {
    fn drop(&mut self) {
        // A pending Ctrl-C is delivered here, after the original value was written back.
        let _ = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&self.previous), None);
    }
}

/// The only device write of the command: with SIGINT, SIGTERM, SIGHUP, SIGQUIT and SIGTSTP
/// blocked, opens the emitter (reads the original value), writes `on` and reads it back, writes
/// `off` and reads it back, then restores the original value, on every path (the emitter is
/// dropped, and restores, before the signal mask is). At most [`MAX_DEVICE_TEST_QUERIES`]
/// requests. `hook` runs inside the blocked window (a test seam; production passes a no-op).
/// SIGKILL and SIGSTOP cannot be blocked: a killed run may leave the last written value.
///
/// # Errors
///
/// [`AdminCliError::IrEmitter`] when a request fails, a read-back differs, or the restore fails.
pub fn test_emitter_on_device(
    ctrl: &mut dyn XuControl,
    descriptor: &XuEmitterDescriptor,
    hook: &mut dyn FnMut(),
) -> Result<DeviceTestReport, AdminCliError> {
    let _signals = SignalBlock::new()?;
    let mut budget = XuBudget::new(
        ctrl,
        MAX_DEVICE_TEST_QUERIES.saturating_sub(CONFIGURE_PRECHECK_QUERIES),
    );
    let mut emitter = UvcXuEmitter::open(&mut budget, descriptor.clone())
        .map_err(|e| ir_error(format!("cannot open the control: {e}")))?;
    hook();
    let outcome = (|| {
        emitter.set(EmitterState::On)?;
        let on_ok = emitter.get()? == Some(EmitterState::On);
        emitter.set(EmitterState::Off)?;
        let off_ok = emitter.get()? == Some(EmitterState::Off);
        Ok::<_, soos_camera_v4l::IrEmitterError>((on_ok, off_ok))
    })();
    let restored = emitter.restore();
    drop(emitter);
    restored.map_err(|e| ir_error(e.to_string()))?;
    let (on_readback_ok, off_readback_ok) = outcome
        .map_err(|e| ir_error(format!("device test failed (original value restored): {e}")))?;
    if !on_readback_ok || !off_readback_ok {
        return Err(ir_error(
            "the control did not read back the written payloads (original value restored)",
        ));
    }
    Ok(DeviceTestReport {
        on_readback_ok,
        off_readback_ok,
        restored: true,
    })
}

/// The original file as found: its text and identity.
struct Original {
    text: String,
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
    mode: u32,
    owner: (u32, u32),
}

/// Reads `path` without following a symbolic link, bounded by [`MAX_DAEMON_CONFIG_BYTES`].
fn read_original(path: &Path) -> Result<Option<Original>, AdminCliError> {
    let shown = sanitize_path_text(path);
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            return Err(ir_error(format!(
                "{shown} is a symbolic link; edit its target instead"
            )))
        }
        Err(e) => return Err(ir_error(format!("cannot open {shown}: {e}"))),
    };
    let meta = file
        .metadata()
        .map_err(|e| ir_error(format!("cannot examine {shown}: {e}")))?;
    if !meta.is_file() {
        return Err(ir_error(format!("{shown} is not a regular file")));
    }
    if meta.len() > MAX_DAEMON_CONFIG_BYTES {
        return Err(ir_error(format!(
            "{shown} is larger than {MAX_DAEMON_CONFIG_BYTES} bytes"
        )));
    }
    let mut bytes = Vec::new();
    file.take(MAX_DAEMON_CONFIG_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| ir_error(format!("cannot read {shown}: {e}")))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_DAEMON_CONFIG_BYTES {
        return Err(ir_error(format!(
            "{shown} is larger than {MAX_DAEMON_CONFIG_BYTES} bytes"
        )));
    }
    let text =
        String::from_utf8(bytes).map_err(|_| ir_error(format!("{shown} is not UTF-8 text")))?;
    Ok(Some(Original {
        text,
        dev: meta.dev(),
        ino: meta.ino(),
        len: meta.len(),
        mtime: (meta.mtime(), meta.mtime_nsec()),
        mode: meta.mode(),
        owner: (meta.uid(), meta.gid()),
    }))
}

/// Removes the managed block from `text` (refusing unbalanced or duplicated markers).
fn strip_managed_block(text: &str) -> Result<String, AdminCliError> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let is = |line: &str, marker: &str| line.trim_end_matches(['\n', '\r']) == marker;
    let begins: Vec<usize> = (0..lines.len())
        .filter(|i| lines.get(*i).is_some_and(|l| is(l, IR_EMITTER_BLOCK_BEGIN)))
        .collect();
    let ends: Vec<usize> = (0..lines.len())
        .filter(|i| lines.get(*i).is_some_and(|l| is(l, IR_EMITTER_BLOCK_END)))
        .collect();
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => Ok(text.to_string()),
        ([begin], [end]) if begin < end => Ok(lines
            .iter()
            .enumerate()
            .filter(|(i, _)| i < begin || i > end)
            .map(|(_, l)| *l)
            .collect()),
        _ => Err(ir_error(
            "the managed [camera.ir_emitter] block markers are unbalanced or duplicated; fix \
             the file by hand",
        )),
    }
}

/// Builds the new configuration text, validated like soos-daemon would load it.
fn render_config(original: &str, settings: &IrEmitterSettings) -> Result<String, AdminCliError> {
    let mut text = strip_managed_block(original)?;
    match ir_emitter_section_present(&text) {
        Ok(false) => {}
        Ok(true) => {
            return Err(ir_error(
                "[camera.ir_emitter] is defined outside the managed block; edit it by hand or \
                 remove it first",
            ))
        }
        Err(_) => {
            return Err(ir_error(
                "the configuration does not parse as TOML; fix it before configuring the emitter",
            ))
        }
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(IR_EMITTER_BLOCK_BEGIN);
    text.push('\n');
    text.push_str(&settings.to_toml_section());
    text.push_str(IR_EMITTER_BLOCK_END);
    text.push('\n');
    match parse_ir_emitter_text(&text) {
        Ok(parsed) if parsed == *settings => Ok(text),
        _ => Err(ir_error(
            "the updated configuration would not load in soos-daemon; nothing was written",
        )),
    }
}

/// `true` when `path` still is the file read (or still absent).
fn unchanged(path: &Path, original: Option<&Original>) -> bool {
    match (std::fs::symlink_metadata(path), original) {
        (Err(e), None) => e.kind() == std::io::ErrorKind::NotFound,
        (Ok(meta), Some(o)) => {
            meta.is_file()
                && meta.dev() == o.dev
                && meta.ino() == o.ino
                && meta.len() == o.len
                && (meta.mtime(), meta.mtime_nsec()) == o.mtime
        }
        _ => false,
    }
}

/// Creates a fresh temporary sibling of `path`.
fn create_temp(dir: &Path, name: &str) -> Result<(PathBuf, File), AdminCliError> {
    let pid = std::process::id();
    for attempt in 0..MAX_TEMP_ATTEMPTS {
        let tmp = dir.join(format!(".{name}.soos-admin.{pid}.{attempt}.tmp"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(ir_error(format!(
                    "cannot create a temporary file in {}: {e}",
                    sanitize_path_text(dir)
                )))
            }
        }
    }
    Err(ir_error(format!(
        "no free temporary file name in {}",
        sanitize_path_text(dir)
    )))
}

/// Writes the managed `[camera.ir_emitter]` block into `config_path` atomically.
///
/// The original is read without following a symbolic link (a link is refused), bounded by
/// [`MAX_DAEMON_CONFIG_BYTES`]; the result must load like soos-daemon loads it; the new text
/// goes to a fresh temporary sibling (mode of the original masked to 0644, or 0644 for a new
/// file; owner of the original kept when running as root), is synced, and replaces the target
/// only if the target is still the file read (same device, inode, size and mtime). The
/// temporary file is removed on every failure.
///
/// # Errors
///
/// [`AdminCliError::IrEmitter`]; the target is then unchanged.
pub fn write_ir_emitter_section(
    config_path: &Path,
    settings: &IrEmitterSettings,
) -> Result<WriteOutcome, AdminCliError> {
    let original = read_original(config_path)?;
    let text = render_config(original.as_ref().map_or("", |o| o.text.as_str()), settings)?;
    let dir = config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = config_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| ir_error("the configuration path has no file name"))?;
    let (tmp, file) = create_temp(dir, &name)?;
    let result = publish(file, &tmp, config_path, dir, &text, original.as_ref());
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    Ok(if original.is_some() {
        WriteOutcome::Updated
    } else {
        WriteOutcome::Created
    })
}

/// Writes, syncs, re-checks and renames the temporary file.
fn publish(
    mut file: File,
    tmp: &Path,
    target: &Path,
    dir: &Path,
    text: &str,
    original: Option<&Original>,
) -> Result<(), AdminCliError> {
    let shown = sanitize_path_text(target);
    let io = |e: std::io::Error| ir_error(format!("cannot write {shown}: {e}"));
    file.write_all(text.as_bytes()).map_err(io)?;
    let mode = original.map_or(0o644, |o| o.mode & 0o644);
    file.set_permissions(std::fs::Permissions::from_mode(mode))
        .map_err(io)?;
    if let Some(o) = original {
        if nix::unistd::geteuid().is_root() {
            std::os::unix::fs::fchown(&file, Some(o.owner.0), Some(o.owner.1)).map_err(io)?;
        }
    }
    file.sync_all().map_err(io)?;
    drop(file);
    if !unchanged(target, original) {
        return Err(ir_error(format!(
            "{shown} changed while it was being updated; nothing was written"
        )));
    }
    std::fs::rename(tmp, target).map_err(io)?;
    File::open(dir).and_then(|d| d.sync_all()).map_err(io)?;
    Ok(())
}
