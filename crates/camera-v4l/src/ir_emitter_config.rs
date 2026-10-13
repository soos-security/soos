//! `[camera.ir_emitter]` of `/etc/soos/daemon.toml` (GitHub #378, PR 1 of #353; ADR 2026-10-13
//! "UVC Extension-Unit IR Emitter Control and Strobe Classification").
//!
//! ```toml
//! [camera.ir_emitter]
//! mode = "off"            # "off" (default) or "auto"
//! device = "04f2:b6d9"    # USB VID:PID of the IR camera
//! unit = 14               # extension unit id, 1..=255
//! selector = 6            # control selector, 1..=255
//! on = [1, 3, 3, 0]       # SET_CUR payload that lights the emitter, 1..=64 bytes
//! off = [1, 3, 0, 0]      # SET_CUR payload that switches it off, same length
//! ```
//!
//! Parsing is fail-closed: an unknown key, a wrong type, a value out of range, a payload above
//! [`XU_MAX_PAYLOAD_BYTES`], mismatched or identical payloads all make `soos-daemon` refuse to
//! start. Errors name the key, never the value. `soos-daemon` does not drive the emitter yet
//! (split item 4 of #353): `mode = "auto"` is validated and reported at start-up only.

use std::fmt;
use std::fs::File;
use std::io::Read as _;
use std::path::Path;

use crate::daemon_config::{read_daemon_config_text, DaemonConfigError};
use crate::uvc_xu::XU_MAX_PAYLOAD_BYTES;

/// Largest `idVendor` / `idProduct` sysfs file read (4 hex digits and a newline are expected).
pub const MAX_USB_ID_FILE_BYTES: u64 = 16;

/// Longest unknown key name echoed in an error (after sanitizing).
const MAX_ECHOED_KEY_CHARS: usize = 64;

/// Most ignored `[camera]` key names reported by [`ignored_camera_keys`].
pub const MAX_IGNORED_CAMERA_KEYS: usize = 8;

/// The table name used in errors.
const IR_EMITTER_TABLE: &str = "camera.ir_emitter";

/// Keys of `[camera.ir_emitter]`.
const IR_EMITTER_KEYS: [&str; 6] = ["mode", "device", "unit", "selector", "on", "off"];

/// Whether soos may drive the configured emitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IrEmitterMode {
    /// soos never touches the emitter (default; also when the section is absent).
    #[default]
    Off,
    /// soos may switch the configured control on the matching IR node during its own captures
    /// (not acted upon by `soos-daemon` before split item 4 of #353).
    Auto,
}

impl IrEmitterMode {
    /// The TOML spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Auto => "auto",
        }
    }
}

/// A USB vendor and product id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UsbDeviceId {
    /// `idVendor`.
    pub vendor: u16,
    /// `idProduct`.
    pub product: u16,
}

/// Parses exactly four ASCII hex digits.
fn parse_hex4(text: &str) -> Option<u16> {
    if text.len() != 4 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(text, 16).ok()
}

impl UsbDeviceId {
    /// Parses `"vvvv:pppp"` (4 + 4 hex digits, case-insensitive, nothing else).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (vendor, product) = text.split_once(':')?;
        Some(Self {
            vendor: parse_hex4(vendor)?,
            product: parse_hex4(product)?,
        })
    }
}

impl fmt::Display for UsbDeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04x}:{:04x}", self.vendor, self.product)
    }
}

/// Why `[camera.ir_emitter]` was refused. Never carries a configured value (except the USB
/// ids of a mismatch, which are not secret).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IrEmitterConfigError {
    /// A key that is not part of `[camera.ir_emitter]`.
    #[error("unknown key `{key}` in [{table}]")]
    UnknownKey {
        /// Always `camera.ir_emitter`.
        table: &'static str,
        /// The key name, sanitized and truncated to 64 characters.
        key: String,
    },
    /// A key of the wrong TOML type.
    #[error("[camera.ir_emitter] {key} must be a {expected}")]
    WrongType {
        /// The key.
        key: &'static str,
        /// The expected type.
        expected: &'static str,
    },
    /// `mode` is neither `"off"` nor `"auto"`.
    #[error("[camera.ir_emitter] mode must be \"off\" or \"auto\"")]
    InvalidMode,
    /// `mode = "auto"` without this key.
    #[error("[camera.ir_emitter] {key} is required when mode = \"auto\"")]
    MissingKey {
        /// The missing key.
        key: &'static str,
    },
    /// `unit` or `selector` outside `1..=255`.
    #[error("[camera.ir_emitter] {key} must be between 1 and 255")]
    OutOfRange {
        /// The key.
        key: &'static str,
    },
    /// `device` is not `"vvvv:pppp"`.
    #[error("[camera.ir_emitter] device must be a USB id written \"vvvv:pppp\" (hexadecimal)")]
    InvalidDeviceId,
    /// An empty payload.
    #[error("[camera.ir_emitter] {key} must hold at least one byte")]
    EmptyPayload {
        /// `on` or `off`.
        key: &'static str,
    },
    /// A payload above the bound.
    #[error("[camera.ir_emitter] {key} holds more than {max} bytes")]
    PayloadTooLong {
        /// `on` or `off`.
        key: &'static str,
        /// The bound.
        max: usize,
    },
    /// A payload element that is not an integer in `0..=255`.
    #[error("[camera.ir_emitter] {key} must only hold integers between 0 and 255")]
    PayloadByteOutOfRange {
        /// `on` or `off`.
        key: &'static str,
    },
    /// `on` and `off` differ in length.
    #[error("[camera.ir_emitter] on and off must have the same length")]
    PayloadLengthMismatch,
    /// `on` equals `off`: nothing would be switched.
    #[error("[camera.ir_emitter] on and off must differ")]
    IdenticalPayloads,
    /// The USB id of the node could not be read.
    #[error("the USB id of the camera node cannot be read")]
    DeviceIdUnavailable,
    /// The node is another USB device than the configured one.
    #[error("the camera node is USB device {observed}, not the configured {configured}")]
    DeviceMismatch {
        /// From the configuration.
        configured: UsbDeviceId,
        /// From sysfs.
        observed: UsbDeviceId,
    },
}

/// A validated emitter control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XuEmitterDescriptor {
    /// USB id of the IR camera.
    pub device: UsbDeviceId,
    /// Extension unit id (1..=255).
    pub unit: u8,
    /// Control selector (1..=255).
    pub selector: u8,
    /// Payload that lights the emitter.
    pub on: Vec<u8>,
    /// Payload that switches it off.
    pub off: Vec<u8>,
}

/// Checks one payload's length.
fn check_payload(key: &'static str, payload: &[u8]) -> Result<(), IrEmitterConfigError> {
    if payload.is_empty() {
        return Err(IrEmitterConfigError::EmptyPayload { key });
    }
    if payload.len() > XU_MAX_PAYLOAD_BYTES {
        return Err(IrEmitterConfigError::PayloadTooLong {
            key,
            max: XU_MAX_PAYLOAD_BYTES,
        });
    }
    Ok(())
}

/// Checks that the two payloads can toggle anything.
fn check_payload_pair(on: &[u8], off: &[u8]) -> Result<(), IrEmitterConfigError> {
    if on.len() != off.len() {
        return Err(IrEmitterConfigError::PayloadLengthMismatch);
    }
    if on == off {
        return Err(IrEmitterConfigError::IdenticalPayloads);
    }
    Ok(())
}

impl XuEmitterDescriptor {
    /// Builds a descriptor with the rules of the parser.
    ///
    /// # Errors
    ///
    /// `OutOfRange` for a zero unit or selector, payload errors otherwise.
    pub fn new(
        device: UsbDeviceId,
        unit: u8,
        selector: u8,
        on: Vec<u8>,
        off: Vec<u8>,
    ) -> Result<Self, IrEmitterConfigError> {
        if unit == 0 {
            return Err(IrEmitterConfigError::OutOfRange { key: "unit" });
        }
        if selector == 0 {
            return Err(IrEmitterConfigError::OutOfRange { key: "selector" });
        }
        check_payload("on", &on)?;
        check_payload("off", &off)?;
        check_payload_pair(&on, &off)?;
        Ok(Self {
            device,
            unit,
            selector,
            on,
            off,
        })
    }

    /// Fails closed unless `observed` is the configured USB device.
    ///
    /// # Errors
    ///
    /// `DeviceIdUnavailable` for `None`, `DeviceMismatch` for another id.
    pub fn check_device(&self, observed: Option<UsbDeviceId>) -> Result<(), IrEmitterConfigError> {
        match observed {
            None => Err(IrEmitterConfigError::DeviceIdUnavailable),
            Some(id) if id == self.device => Ok(()),
            Some(observed) => Err(IrEmitterConfigError::DeviceMismatch {
                configured: self.device,
                observed,
            }),
        }
    }
}

/// The validated `[camera.ir_emitter]` section.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IrEmitterSettings {
    /// `mode` (default `off`).
    pub mode: IrEmitterMode,
    /// The control, when every descriptor key is present.
    pub descriptor: Option<XuEmitterDescriptor>,
}

/// Formats a payload as a TOML integer array.
fn toml_array(bytes: &[u8]) -> String {
    let items: Vec<String> = bytes.iter().map(u8::to_string).collect();
    format!("[{}]", items.join(", "))
}

impl IrEmitterSettings {
    /// The `[camera.ir_emitter]` table text, parsed back by [`parse_camera_table`] into an
    /// equal value.
    #[must_use]
    pub fn to_toml_section(&self) -> String {
        let mut out = format!("[camera.ir_emitter]\nmode = \"{}\"\n", self.mode.as_str());
        if let Some(d) = &self.descriptor {
            out.push_str(&format!(
                "device = \"{}\"\nunit = {}\nselector = {}\non = {}\noff = {}\n",
                d.device,
                d.unit,
                d.selector,
                toml_array(&d.on),
                toml_array(&d.off)
            ));
        }
        out
    }
}

/// Sanitized, bounded key name for an error.
fn echo_key(key: &str) -> String {
    crate::diagnostics::sanitize_display_text(key)
        .chars()
        .take(MAX_ECHOED_KEY_CHARS)
        .collect()
}

/// Reads `unit` / `selector`.
fn parse_id(key: &'static str, value: &toml::Value) -> Result<u8, IrEmitterConfigError> {
    let toml::Value::Integer(raw) = value else {
        return Err(IrEmitterConfigError::WrongType {
            key,
            expected: "integer",
        });
    };
    u8::try_from(*raw)
        .ok()
        .filter(|id| *id != 0)
        .ok_or(IrEmitterConfigError::OutOfRange { key })
}

/// Reads `on` / `off`.
fn parse_payload(key: &'static str, value: &toml::Value) -> Result<Vec<u8>, IrEmitterConfigError> {
    let toml::Value::Array(items) = value else {
        return Err(IrEmitterConfigError::WrongType {
            key,
            expected: "array of integers",
        });
    };
    if items.is_empty() {
        return Err(IrEmitterConfigError::EmptyPayload { key });
    }
    if items.len() > XU_MAX_PAYLOAD_BYTES {
        return Err(IrEmitterConfigError::PayloadTooLong {
            key,
            max: XU_MAX_PAYLOAD_BYTES,
        });
    }
    items
        .iter()
        .map(|item| match item {
            toml::Value::Integer(raw) => u8::try_from(*raw).ok(),
            _ => None,
        })
        .collect::<Option<Vec<u8>>>()
        .ok_or(IrEmitterConfigError::PayloadByteOutOfRange { key })
}

/// Names of the `[camera]` keys other than `ir_emitter` (ignored by [`parse_camera_table`]),
/// sanitized, truncated to 64 characters and at most [`MAX_IGNORED_CAMERA_KEYS`] of them, so
/// the daemon can warn about them by name, never by value.
#[must_use]
pub fn ignored_camera_keys(camera: &toml::Table) -> Vec<String> {
    camera
        .keys()
        .filter(|k| k.as_str() != "ir_emitter")
        .take(MAX_IGNORED_CAMERA_KEYS)
        .map(|k| echo_key(k))
        .collect()
}

/// Parses the `[camera]` table of `daemon.toml`.
///
/// Only `ir_emitter` is interpreted in `[camera]`: any other key is ignored (reported by
/// [`ignored_camera_keys`]), like unknown keys of the other daemon tables. Inside
/// `[camera.ir_emitter]` only `mode`, `device`, `unit`,
/// `selector`, `on` and `off`. Absent `ir_emitter`: the default (off). With `mode = "off"` the
/// present keys are still validated and missing keys are allowed (`descriptor` is `Some` only
/// when all five are present); with `mode = "auto"` every key is required.
///
/// # Errors
///
/// The first [`IrEmitterConfigError`] found.
pub fn parse_camera_table(camera: &toml::Table) -> Result<IrEmitterSettings, IrEmitterConfigError> {
    // Other `[camera]` keys are ignored like unknown keys of every other daemon table (candid
    // review of #378): a stray key must not stop the daemon. They are reported through
    // [`ignored_camera_keys`].
    let Some(value) = camera.get("ir_emitter") else {
        return Ok(IrEmitterSettings::default());
    };
    let toml::Value::Table(table) = value else {
        return Err(IrEmitterConfigError::WrongType {
            key: "ir_emitter",
            expected: "table",
        });
    };
    if let Some(key) = table
        .keys()
        .find(|k| !IR_EMITTER_KEYS.contains(&k.as_str()))
    {
        return Err(IrEmitterConfigError::UnknownKey {
            table: IR_EMITTER_TABLE,
            key: echo_key(key),
        });
    }

    let mode = match table.get("mode") {
        None => IrEmitterMode::Off,
        Some(toml::Value::String(raw)) => match raw.as_str() {
            "off" => IrEmitterMode::Off,
            "auto" => IrEmitterMode::Auto,
            _ => return Err(IrEmitterConfigError::InvalidMode),
        },
        Some(_) => {
            return Err(IrEmitterConfigError::WrongType {
                key: "mode",
                expected: "string",
            })
        }
    };
    let device = match table.get("device") {
        None => None,
        Some(toml::Value::String(raw)) => {
            Some(UsbDeviceId::parse(raw).ok_or(IrEmitterConfigError::InvalidDeviceId)?)
        }
        Some(_) => {
            return Err(IrEmitterConfigError::WrongType {
                key: "device",
                expected: "string",
            })
        }
    };
    let unit = table.get("unit").map(|v| parse_id("unit", v)).transpose()?;
    let selector = table
        .get("selector")
        .map(|v| parse_id("selector", v))
        .transpose()?;
    let on = table
        .get("on")
        .map(|v| parse_payload("on", v))
        .transpose()?;
    let off = table
        .get("off")
        .map(|v| parse_payload("off", v))
        .transpose()?;
    if let (Some(on), Some(off)) = (&on, &off) {
        check_payload_pair(on, off)?;
    }

    let descriptor = match (device, unit, selector, on, off) {
        (Some(device), Some(unit), Some(selector), Some(on), Some(off)) => {
            Some(XuEmitterDescriptor::new(device, unit, selector, on, off)?)
        }
        (device, unit, selector, on, off) => {
            if mode == IrEmitterMode::Auto {
                let missing = [
                    ("device", device.is_none()),
                    ("unit", unit.is_none()),
                    ("selector", selector.is_none()),
                    ("on", on.is_none()),
                    ("off", off.is_none()),
                ]
                .into_iter()
                .find(|(_, missing)| *missing)
                .map_or("device", |(key, _)| key);
                return Err(IrEmitterConfigError::MissingKey { key: missing });
            }
            None
        }
    };
    Ok(IrEmitterSettings { mode, descriptor })
}

/// Why a client could not read `[camera.ir_emitter]`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IrEmitterReadError {
    /// The shared bounded reader refused the file.
    #[error("the daemon configuration {0}")]
    Read(DaemonConfigError),
    /// Not TOML, or `camera` is not a table.
    #[error("the daemon configuration cannot be parsed")]
    Malformed,
    /// `[camera.ir_emitter]` is invalid (soos-daemon refuses to start with it).
    #[error("{0}")]
    Invalid(IrEmitterConfigError),
}

/// Parses `text` as TOML and returns the `[camera]` table (`None` when absent).
fn camera_table_of(text: &str) -> Result<Option<toml::Table>, IrEmitterReadError> {
    let mut doc: toml::Table = text.parse().map_err(|_| IrEmitterReadError::Malformed)?;
    match doc.remove("camera") {
        None => Ok(None),
        Some(toml::Value::Table(camera)) => Ok(Some(camera)),
        Some(_) => Err(IrEmitterReadError::Malformed),
    }
}

/// Parses the whole text of `daemon.toml` and returns its `[camera.ir_emitter]` settings.
/// For clients (soos-admin) that must not parse TOML themselves.
///
/// # Errors
///
/// `Malformed` for text that is not TOML (or a non-table `camera`), `Invalid` for a refused
/// section.
pub fn parse_ir_emitter_text(text: &str) -> Result<IrEmitterSettings, IrEmitterReadError> {
    match camera_table_of(text)? {
        None => Ok(IrEmitterSettings::default()),
        Some(camera) => parse_camera_table(&camera).map_err(IrEmitterReadError::Invalid),
    }
}

/// `true` when the text of `daemon.toml` defines `camera.ir_emitter` (in any TOML form).
///
/// # Errors
///
/// `Malformed` for text that is not TOML.
pub fn ir_emitter_section_present(text: &str) -> Result<bool, IrEmitterReadError> {
    Ok(camera_table_of(text)?.is_some_and(|camera| camera.contains_key("ir_emitter")))
}

/// Reads `[camera.ir_emitter]` from `path` through the shared bounded reader.
///
/// # Errors
///
/// `Read` when the file cannot be read, otherwise as [`parse_ir_emitter_text`].
pub fn read_daemon_ir_emitter_config(path: &Path) -> Result<IrEmitterSettings, IrEmitterReadError> {
    let text = read_daemon_config_text(path).map_err(IrEmitterReadError::Read)?;
    parse_ir_emitter_text(&text)
}

/// `true` for a `video<N>` node name.
fn is_video_node_name(node: &str) -> bool {
    node.strip_prefix("video")
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Reads one sysfs id file (at most [`MAX_USB_ID_FILE_BYTES`]).
fn read_id_file(path: &Path) -> Option<u16> {
    let file = File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_USB_ID_FILE_BYTES.saturating_add(1))
        .read_to_string(&mut text)
        .ok()?;
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_USB_ID_FILE_BYTES {
        return None;
    }
    parse_hex4(text.strip_suffix('\n').unwrap_or(&text))
}

/// USB id of the V4L2 node `node` (`video<N>`), from `<sysfs_dir>/<node>/device`: the link is
/// canonicalized and `idVendor` / `idProduct` are read from the canonical parent directory (the
/// USB device of the interface), each bounded by [`MAX_USB_ID_FILE_BYTES`].
///
/// `None` for any other node name, a missing link or file, or content that is not exactly four
/// hex digits (with an optional newline).
#[must_use]
pub fn usb_device_id_of_node(sysfs_dir: &Path, node: &str) -> Option<UsbDeviceId> {
    if !is_video_node_name(node) {
        return None;
    }
    let interface = std::fs::canonicalize(sysfs_dir.join(node).join("device")).ok()?;
    let usb_device = interface.parent()?;
    Some(UsbDeviceId {
        vendor: read_id_file(&usb_device.join("idVendor"))?,
        product: read_id_file(&usb_device.join("idProduct"))?,
    })
}
