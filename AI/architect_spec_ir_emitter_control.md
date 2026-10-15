# Architect Spec — GitHub #378: UVC extension-unit IR emitter control and stream strobe classification

- **Date**: 2026-10-13
- **Issue**: GitHub-only #378 (PR 1 of epic #353, planned split item 1); no `AI/BACKLOG.md` id.
- **Branch**: `feat/ir-antispoofing-mock` (GitHub-only issue: not registered in `scripts/sync_issue.py`; the commit carries `Closes #378`, `Refs #353`).
- **Base commit**: `6f32ef8` (`origin/main`).
- **Owner decisions** (#353 comment of 2026-10-12, restated for this run):
  - OD-1 The owner's PC has an RGB `Integrated Camera` only. IR hardware validation runs on another project member's PC with an IR camera. Everything in this PR is testable with mocks and synthetic fixtures; the physical rows stay `⬜ Pending` for that member.
  - OD-2 `soos-frame` owns `Frame` / `PixelFormat` / `SensorType`; any IR-only type added to it goes behind a non-default `ir` feature (ADR 2026-10-13). **This PR adds nothing to `soos-frame`** (every new type lives in the adapter crate `soos-camera-v4l` or in the dev-only fixtures crate), so no `ir` feature is introduced yet.
- **Assumed decisions (owner to confirm)** — open questions of #353 that touch this scope, resolved with the most conservative option:
  - AD-1 (#353 Q5, blind probing vs known-device database): **no blind writes at all.** `soos-admin camera ir-emitter probe` is strictly read-only (`GET_*` UVC requests only, enforced by a type-level read-only wrapper). The only device write in this PR is `configure --test-on-device`, an explicit flag that writes exactly the operator-supplied `on`/`off` payloads to one operator-supplied control and restores the original value. The probing approach is inspired by linux-enable-ir-emitter (MIT), reimplemented without copying code (attribution in `Docs/CAMERA_V4L_CRATE.md`). No known-device database is shipped: no entry could be verified on real hardware (OD-1); the database is deferred to the docs/supported-modules PR (#353 split item 6).
  - AD-2 (#353 Q2, default mode): `[camera.ir_emitter] mode` defaults to `"off"` (section absent ⇒ off ⇒ behaviour unchanged). `"auto"` is accepted and validated but **not yet acted upon by `soos-daemon`** in this PR (a start-up warning says so); driving the emitter during capture lands with strict IR mode (#353 split item 4).
  - AD-3 The daemon-side "VID:PID differs from the resolved node ⇒ refuse at start-up" check needs the resolved IR node, which the daemon resolves lazily in the capture thread. This PR ships the check (`IrEmitterDescriptor::check_device`) and the bounded sysfs reader (`usb_device_id_of_node`), used by `soos-admin camera ir-emitter probe|configure`; the daemon calls them when it starts driving the emitter (split item 4). Until then a mismatched descriptor can never reach a device, because the daemon never opens the XU control.

## 1. Scope & Blast Radius

| Area | Change |
|---|---|
| `crates/camera-v4l/src/uvc_xu.rs` | **new**: UVC request codes, `XuControl` trait, `UvcXuDevice` (the only new `unsafe`: `UVCIOC_CTRL_QUERY`), `XuReadOnly` wrapper, bounded read-only scan `probe_xu_controls` |
| `crates/camera-v4l/src/ir_emitter.rs` | **new**: `EmitterState`, `EmitterCapability`, `IrEmitter` trait, `UvcXuEmitter<C>` (restore-on-drop), `IrEmitterError` |
| `crates/camera-v4l/src/ir_emitter_config.rs` | **new**: `[camera.ir_emitter]` types, fail-closed parser, TOML rendering, `UsbDeviceId`, bounded sysfs `idVendor`/`idProduct` reader, shared-reader entry point for clients |
| `crates/camera-v4l/src/strobe.rs` | **new**: frame mean luma, `LumaSample`, `StrobeClass`, `classify_strobe`, `StrobeMonitor` |
| `crates/camera-v4l/src/ir_mock.rs` | **new**, `#[cfg(feature = "mock-camera")]`: `MockXuControl` (spy + fault injection), `MockIrEmitter`, `MockEmitterBehavior`, `MockIrScene`, `synthetic_ir_frame` |
| `crates/camera-v4l/src/mock.rs` | private `frame_source` hook; `MockCameraManager::new_ir` (`mock-camera` only) produces Grey frames stamped `Infrared` whose luma follows the mock emitter |
| `crates/camera-v4l/src/diagnostics.rs` | `ensure_v4l2_char_device` becomes `pub(crate)` (reused by `UvcXuDevice::open`) |
| `crates/camera-v4l/src/lib.rs`, `Cargo.toml` | module declarations and re-exports; dev-deps `proptest = "1"` (already in `Cargo.lock`), `soos-test-fixtures` |
| `crates/daemon/src/config.rs` | `[camera]` table parsed strictly through the camera crate; `PipelineConfig::ir_emitter`; warning when `mode = "auto"` |
| `crates/admin-cli/src/ir_emitter.rs` | **new**: `camera ir-emitter probe|configure` logic (reports, managed-block config writer, `--test-on-device` with signal blocking and restore) |
| `crates/admin-cli/src/args.rs`, `main.rs`, `lib.rs`, `error.rs` | `CameraAction::IrEmitter`, dispatch, `AdminCliError::IrEmitter(String)` |
| `crates/admin-cli/Cargo.toml` | dev-dependency `soos-camera-v4l` with `features = ["mock-camera"]` (no new crate) |
| `tests/fixtures/mod.rs` | **new** module `ir_strobe` (synthetic, non-biometric Grey IR sequences) |
| `tests/invariants/src/ir_emitter_contract.rs` + `lib.rs` | **new** static contracts (unsafe confinement, read-only probe, no payload in logs, `soos-frame` untouched) |
| Docs / traceability | `Docs/CAMERA_V4L_CRATE.md` (emitter section rewritten), `Docs/DAEMON.md` (`[camera.ir_emitter]`), ADR, matrix `IRA*`, walkthrough 206 |

Untouched: `crates/pam`, the IPC protocol, `soos-frame`, `soos-vision`, `soos-policy`, the auth pipeline of the daemon (no emitter is driven, no frame is classified at runtime), packaging (no new binary, no new installed file; the key is optional).

Consumers of changed public items: `PipelineConfig` gains a field (every literal in the tree uses `..Default::default()`: `crates/daemon/tests/inference_threads_config_tests.rs`); `CameraAction` gains a variant (only `main.rs` matches it exhaustively; tests use `_ =>` arms or `matches!`); `MockCameraManager` gains a constructor (no signature change).

## 2. Types & Signatures

### 2.1 `uvc_xu.rs`

```rust
/// UVC class-specific request codes (USB Video Class 1.5, table A-8; `linux/usb/video.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum XuQuery { SetCur = 0x01, GetCur = 0x81, GetMin = 0x82, GetMax = 0x83,
                   GetRes = 0x84, GetLen = 0x85, GetInfo = 0x86, GetDef = 0x87 }
impl XuQuery { pub const fn code(self) -> u8; pub const fn is_write(self) -> bool /* SetCur only */; }

/// Largest control payload soos reads or writes (also the `[camera.ir_emitter]` bound).
pub const XU_MAX_PAYLOAD_BYTES: usize = 64;
/// Units scanned by default by the read-only probe (`1..=PROBE_MAX_UNIT_ID`).
pub const PROBE_MAX_UNIT_ID: u8 = 32;
/// Selectors scanned per unit (`1..=PROBE_MAX_SELECTOR`).
pub const PROBE_MAX_SELECTOR: u8 = 32;
/// Hard cap on UVC requests issued by one probe (every request counts, failed ones included).
pub const MAX_PROBE_QUERIES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum XuError {
    NotFound,                       // ENOENT: no such unit / selector
    Unsupported,                    // EINVAL, ENOTTY, EOPNOTSUPP, ENOSYS, EBADRQC: not a UVC node or request refused
    PermissionDenied,               // EACCES, EPERM
    Busy,                           // EBUSY
    Gone,                           // ENODEV, ENXIO
    Io(i32),                        // any other errno (number only)
    NotV4l2Device,                  // open refused: not a V4L2 character device (major 81)
    InvalidLength { len: usize },   // 0 or > XU_MAX_PAYLOAD_BYTES (GET_LEN: != 2, GET_INFO: != 1)
    WriteRefused,                   // SET_CUR through XuReadOnly
    QueryBudgetExhausted,           // probe cap reached
}

/// Raw access to UVC extension-unit controls (`UVCIOC_CTRL_QUERY`). Tests inject `MockXuControl`.
pub trait XuControl {
    /// Issues one request; `data.len()` is the request size (`GET_LEN`: 2, `GET_INFO`: 1,
    /// otherwise 1..=XU_MAX_PAYLOAD_BYTES). Implementations validate the length first.
    fn query(&mut self, unit: u8, selector: u8, query: XuQuery, data: &mut [u8]) -> Result<(), XuError>;
}

/// `GET_INFO` capability bits (UVC 1.5 §4.1.2): bit 0 GET supported, bit 1 SET supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XuInfo { pub raw: u8 }
impl XuInfo { pub const fn supports_get(self) -> bool; pub const fn supports_set(self) -> bool; }

pub fn xu_get_len(ctrl: &mut dyn XuControl, unit: u8, selector: u8) -> Result<u16, XuError>; // little-endian
pub fn xu_get_info(ctrl: &mut dyn XuControl, unit: u8, selector: u8) -> Result<XuInfo, XuError>;
/// `len` must be 1..=XU_MAX_PAYLOAD_BYTES, else InvalidLength without any request.
pub fn xu_get_cur(ctrl: &mut dyn XuControl, unit: u8, selector: u8, len: usize) -> Result<Vec<u8>, XuError>;
pub fn xu_set_cur(ctrl: &mut dyn XuControl, unit: u8, selector: u8, payload: &[u8]) -> Result<(), XuError>;

/// Read-only view: forwards every GET_* request, refuses SET_CUR with WriteRefused without
/// forwarding it (the inner control never sees a write).
pub struct XuReadOnly<'a> { inner: &'a mut dyn XuControl }
impl<'a> XuReadOnly<'a> { pub fn new(inner: &'a mut dyn XuControl) -> Self; }
impl XuControl for XuReadOnly<'_> { /* ... */ }

/// One control found by the read-only scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XuControlReport { pub unit: u8, pub selector: u8, pub len: u16, pub info: XuInfo,
                             /// GET_CUR value, `None` when GET is unsupported, the length is
                             /// 0 or > XU_MAX_PAYLOAD_BYTES, or GET_CUR failed.
                             pub current: Option<Vec<u8>> }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XuProbeReport { pub controls: Vec<XuControlReport>, pub queries_issued: usize,
                           /// true when MAX_PROBE_QUERIES stopped the scan
                           pub truncated: bool,
                           /// first error that is neither NotFound nor Unsupported (stops the scan:
                           /// PermissionDenied, Busy, Gone, Io)
                           pub aborted: Option<XuError> }

/// Read-only scan of `units` × `1..=PROBE_MAX_SELECTOR`: GET_LEN, then GET_INFO, then GET_CUR
/// when supported. Wraps `ctrl` in `XuReadOnly` itself. Bounded by MAX_PROBE_QUERIES.
pub fn probe_xu_controls(ctrl: &mut dyn XuControl, units: std::ops::RangeInclusive<u8>) -> XuProbeReport;

/// Production control on an open V4L2 node.
pub struct UvcXuDevice { file: std::fs::File }
impl UvcXuDevice {
    /// Refuses anything but a V4L2 character device (`ensure_v4l2_char_device`, before
    /// `open(2)`), then opens read-write with O_CLOEXEC | O_NONBLOCK | O_NOCTTY.
    pub fn open(path: &Path) -> Result<Self, XuError>;
}
impl XuControl for UvcXuDevice { /* the only new unsafe block */ }
```

Kernel ABI (`include/uapi/linux/uvcvideo.h`), mirrored privately:

```rust
#[repr(C)]
struct UvcXuControlQuery { unit: u8, selector: u8, query: u8, size: u16, data: *mut u8 }
const UVCIOC_CTRL_QUERY: c_ulong /* v4l::v4l2::vidioc::_IOC_TYPE */ =
    _IOWR('u', 0x21, size_of::<UvcXuControlQuery>());   // 0xC010_7521 on 64-bit targets
```

`UvcXuDevice::query` validates the length (`InvalidLength`), builds the struct with `size = len as u16`
(len ≤ 64 proven before the cast) and `data = data.as_mut_ptr()`, then calls `v4l::v4l2::ioctl`
(the same wrapper `v4l_guard::enum_formats_guarded` uses). Errno mapping as listed in `XuError`.

### 2.2 `ir_emitter.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmitterState { On, Off }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmitterCapability { pub can_read: bool, pub can_write: bool, pub payload_len: u16 }

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IrEmitterError {
    Control(#[from] XuError),
    /// GET_LEN of the control differs from the configured payload length.
    PayloadLengthMismatch { control: u16, configured: usize },
    /// GET_INFO says the control cannot be written (or read).
    NotWritable, NotReadable,
    /// Restoring the original value failed; the device may stay in the last written state.
    RestoreFailed(XuError),
    /// The mock refused the command (fault injection).
    Rejected,
}

/// An IR emitter the capture side can switch. Object safe.
pub trait IrEmitter {
    fn capability(&mut self) -> Result<EmitterCapability, IrEmitterError>;
    fn set(&mut self, state: EmitterState) -> Result<(), IrEmitterError>;
    /// Last state successfully commanded through `set` (None before the first `set`).
    fn commanded(&self) -> Option<EmitterState>;
}

/// Production emitter on a UVC extension-unit control.
pub struct UvcXuEmitter<C: XuControl> { control: C, descriptor: XuEmitterDescriptor,
                                        original: Option<Vec<u8>>, commanded: Option<EmitterState>, dirty: bool }
impl<C: XuControl> UvcXuEmitter<C> {
    /// GET_INFO (must support GET and SET), GET_LEN (must equal the payload length), then
    /// GET_CUR of the original value. Nothing is written.
    pub fn open(control: C, descriptor: XuEmitterDescriptor) -> Result<Self, IrEmitterError>;
    /// Writes the original value back (no-op when nothing was written). Idempotent.
    pub fn restore(&mut self) -> Result<(), IrEmitterError>;
    pub fn descriptor(&self) -> &XuEmitterDescriptor;
}
impl<C: XuControl> Drop for UvcXuEmitter<C> { /* best-effort restore when dirty; errors ignored (logged without payload) */ }
```

`set(On)` writes `descriptor.on`, `set(Off)` writes `descriptor.off`; a failed `set` leaves
`commanded` unchanged but marks the emitter dirty (the device state is unknown, so `restore` /
`Drop` still writes the original).

### 2.3 `ir_emitter_config.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IrEmitterMode { #[default] Off, Auto }       // TOML: "off" | "auto" (exact, lowercase)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UsbDeviceId { pub vendor: u16, pub product: u16 }
impl UsbDeviceId { pub fn parse(text: &str) -> Option<Self>; /* "vvvv:pppp", 4+4 hex digits, case-insensitive */ }
impl Display for UsbDeviceId { /* lowercase "vvvv:pppp" */ }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XuEmitterDescriptor { pub device: UsbDeviceId, pub unit: u8, pub selector: u8,
                                 pub on: Vec<u8>, pub off: Vec<u8> }
impl XuEmitterDescriptor {
    /// Validates: unit, selector in 1..=255; on/off 1..=XU_MAX_PAYLOAD_BYTES bytes, equal
    /// lengths, different contents.
    pub fn new(device: UsbDeviceId, unit: u8, selector: u8, on: Vec<u8>, off: Vec<u8>) -> Result<Self, IrEmitterConfigError>;
    /// Fails closed: None => DeviceIdUnavailable, different => DeviceMismatch.
    pub fn check_device(&self, observed: Option<UsbDeviceId>) -> Result<(), IrEmitterConfigError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IrEmitterSettings { pub mode: IrEmitterMode, pub descriptor: Option<XuEmitterDescriptor> }
impl IrEmitterSettings {
    /// The `[camera.ir_emitter]` table text (`[camera.ir_emitter]` header + keys), parseable
    /// by `parse_camera_table` into an equal value (round trip).
    pub fn to_toml_section(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IrEmitterConfigError {
    UnknownKey { table: &'static str, key: String },   // key sanitized + truncated to 64 chars
    WrongType { key: &'static str, expected: &'static str },
    InvalidMode,                                         // value never echoed
    MissingKey { key: &'static str },                    // mode = "auto" needs every key
    OutOfRange { key: &'static str },                    // unit / selector outside 1..=255
    InvalidDeviceId,                                     // not "vvvv:pppp"
    EmptyPayload { key: &'static str },
    PayloadTooLong { key: &'static str, max: usize },
    PayloadByteOutOfRange { key: &'static str },         // element outside 0..=255
    PayloadLengthMismatch,                               // on.len() != off.len()
    IdenticalPayloads,                                   // on == off cannot toggle anything
    DeviceIdUnavailable,
    DeviceMismatch { configured: UsbDeviceId, observed: UsbDeviceId },
}

/// Parses the `[camera]` table of daemon.toml. Only `ir_emitter` is accepted in `[camera]`
/// (any other key: UnknownKey). Inside `[camera.ir_emitter]` the keys are `mode`, `device`,
/// `unit`, `selector`, `on`, `off`; any other key: UnknownKey. Absent `ir_emitter`: default.
/// With mode = "off", present keys are still validated (each one; the cross-key checks when
/// both payloads are present), missing keys are allowed, `descriptor` is Some only when all five
/// descriptor keys are present. With mode = "auto" every key is required.
pub fn parse_camera_table(camera: &toml::Table) -> Result<IrEmitterSettings, IrEmitterConfigError>;

/// Client entry point (soos-admin): shared bounded reader (`read_daemon_config_text`), then
/// `parse_camera_table` on `[camera]` (absent: default). Never parses `[pipeline]`.
pub fn read_daemon_ir_emitter_config(path: &Path) -> Result<IrEmitterSettings, IrEmitterReadError>;
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IrEmitterReadError { Read(DaemonConfigError), Malformed, Invalid(IrEmitterConfigError) }

/// Reads `<sysfs_dir>/<node>/device/../idVendor` and `idProduct` (each at most
/// MAX_USB_ID_FILE_BYTES = 16 bytes, 4 hex digits + optional newline). `node` must be a
/// `video<N>` name (anything else, missing files or bad content: None).
pub fn usb_device_id_of_node(sysfs_dir: &Path, node: &str) -> Option<UsbDeviceId>;
pub const MAX_USB_ID_FILE_BYTES: u64 = 16;
```

### 2.4 `strobe.rs`

```rust
pub const STROBE_MIN_SAMPLES: usize = 8;     // fewer: TooFewSamples
pub const STROBE_MAX_SAMPLES: usize = 64;    // more: TooManySamples
pub const STROBE_MIN_PER_GROUP: usize = 2;   // lit/unlit (or even/odd) group minimum
pub const STROBE_MIN_LUMA_DELTA: f32 = 12.0; // min(lit) - max(unlit) gap, in 8-bit luma levels
pub const STROBE_DARK_LUMA: f32 = 20.0;      // every sample below: no illumination at all

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LumaSample { pub sequence: u64, pub mean_luma: f32, pub commanded: Option<EmitterState> }
impl LumaSample {
    /// Infrared frames only (SensorType::Infrared), else None.
    pub fn from_ir_frame(frame: &Frame, commanded: Option<EmitterState>) -> Option<Self>;
}
/// Mean 8-bit luma of a frame: Grey (all bytes), Yuyv (even bytes), Nv12 (Y plane), Rgb24 (all
/// bytes). None for Mjpeg, zero-size frames, or a buffer whose length differs from
/// PixelFormat::expected_buffer_size (mismatched geometry).
pub fn frame_mean_luma(frame: &Frame) -> Option<f32>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrobeClass {
    /// Lit, stable stream; the emitter (if any) cannot be shown to respond.
    Steady,
    /// Frames alternate lit/unlit by sequence parity on their own.
    NativeAlternating,
    /// Commanded on/off states are followed by the scene.
    Controllable,
    /// No illumination at all (every sample darker than STROBE_DARK_LUMA).
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StrobeError {
    TooFewSamples { got: usize, min: usize },
    TooManySamples { got: usize, max: usize },
    /// NaN, infinite, negative or > 255 mean luma.
    InvalidLuma,
    /// Sequence numbers not strictly increasing.
    NonMonotonicSequence,
    /// The commanded on/off pattern is aligned with sequence parity, so a natively alternating
    /// stream would satisfy it too: the pattern proves nothing.
    AmbiguousPattern,
    /// None of the four classes fits (irregular flicker): fail closed.
    Irregular,
}

/// Order (first match wins): validate → Controllable (both states commanded ≥ MIN_PER_GROUP,
/// not parity-aligned, min(on) − max(off) ≥ DELTA) → NativeAlternating (each parity ≥
/// MIN_PER_GROUP and one parity's min − other parity's max ≥ DELTA) → Absent (max < DARK) →
/// Steady (max − min < DELTA) → Irregular.
pub fn classify_strobe(samples: &[LumaSample]) -> Result<StrobeClass, StrobeError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrobeStatus {
    Learning,
    Unclassified(StrobeError),          // a block failed before any class was established
    Stable(StrobeClass),
    Changed { from: StrobeClass, to: Option<StrobeClass> },   // sticky
}
impl StrobeStatus { /// true for everything but Stable.
                    pub const fn vetoes_allow(self) -> bool; }

/// Classifies consecutive, non-overlapping blocks of `block_len` samples
/// (clamped to STROBE_MIN_SAMPLES..=STROBE_MAX_SAMPLES). The first classified block
/// establishes the class; any later block of another class, or failing classification, turns
/// the status into the sticky `Changed`. Memory: at most STROBE_MAX_SAMPLES samples.
pub struct StrobeMonitor { /* block_len, buffer, status */ }
impl StrobeMonitor {
    pub fn new(block_len: usize) -> Self;
    pub fn push(&mut self, sample: LumaSample) -> StrobeStatus;
    pub fn status(&self) -> StrobeStatus;
}
```

### 2.5 `ir_mock.rs` (`#[cfg(feature = "mock-camera")]`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockEmitterBehavior { Controllable, NativeAlternating, Steady, Absent }

/// Shared illumination state read by MockCameraManager::new_ir.
#[derive(Debug, Clone)]
pub struct MockIrScene { /* Arc<AtomicU8> behaviour + commanded */ }
impl MockIrScene { pub fn is_lit(&self, sequence: u64) -> bool; }

pub struct MockIrEmitter { /* scene, commanded, fail_next_set: bool, set_calls: usize */ }
impl MockIrEmitter {
    pub fn new(behavior: MockEmitterBehavior) -> Self;   // initial commanded state: Off
    pub fn scene(&self) -> MockIrScene;
    pub fn fail_next_set(&mut self);
    pub fn set_calls(&self) -> usize;
}
impl IrEmitter for MockIrEmitter { /* capability: controllable only for Controllable */ }

/// Lit frames: face disc mean ≈ 150 over a 60 background; unlit: ≈ 18 / 12; deterministic
/// texture, non-biometric. Grey, stamped SensorType::Infrared.
pub const MOCK_IR_LIT_LUMA: u8 = 150; pub const MOCK_IR_UNLIT_LUMA: u8 = 12;
pub fn synthetic_ir_frame(width: u32, height: u32, sequence: u64, timestamp_mono_ns: u64, lit: bool) -> Frame;

/// In-memory XU controls with a request spy and fault injection.
pub struct MockXuControl { /* controls: Vec<MockXuEntry>, log (≤ 8192 entries), faults */ }
impl MockXuControl {
    pub fn new() -> Self;
    pub fn with_control(self, unit: u8, selector: u8, info: u8, value: Vec<u8>) -> Self;
    pub fn fail_query(&mut self, query: XuQuery, error: XuError);   // every such request fails
    pub fn fail_set_after(&mut self, successful_sets: usize);        // later SET_CUR fail with Io(EIO)
    pub fn requests(&self) -> &[(u8, u8, XuQuery)];
    pub fn value(&self, unit: u8, selector: u8) -> Option<Vec<u8>>;
}
impl XuControl for MockXuControl { /* ENOENT for unknown unit/selector, InvalidLength like the kernel */ }
impl XuControl for &mut MockXuControl { /* forwards; lets tests keep ownership */ }
```

`MockCameraManager::new_ir(config: CameraConfig, scene: MockIrScene) -> Self` (`mock-camera`
only): identical lifecycle to `new`, but every generated frame is `synthetic_ir_frame(.., scene.is_lit(seq))`
(Grey, `Infrared`), whatever `config.format`. Implemented with a private, unconditional
`frame_source: Option<Arc<dyn Fn(u32, u32, u64, u64) -> Frame + Send + Sync>>` field (`None` for `new`).

### 2.6 `soos-admin camera ir-emitter` (`crates/admin-cli/src/ir_emitter.rs`)

```rust
pub struct IrEmitterEnv<'a> {
    pub camera: &'a CameraEnvironment,      // sysfs, /dev, by-id
    pub probe: &'a dyn V4lDeviceProbe,      // node metadata + classification
    pub euid: u32,
}
/// Node gate shared by probe and configure: euid 0, `probe_camera_node` reports a Candidate
/// with sensor Infrared and driver "uvcvideo"; returns the kernel node path and its USB id
/// (`usb_device_id_of_node`, None allowed for probe, refused by configure).
pub fn check_ir_node(env: &IrEmitterEnv, device: &Path) -> Result<IrNode, AdminCliError>;
pub struct IrNode { pub node: PathBuf, pub usb_id: Option<UsbDeviceId> }

/// Read-only: scan (`probe_xu_controls`, all GET_*), plus the configured descriptor of
/// `config_path` and whether it matches this node. Output never contains anything but control
/// metadata and values.
pub fn probe_ir_emitter(env: &IrEmitterEnv, device: &Path, units: RangeInclusive<u8>,
                        config_path: &Path, ctrl: &mut dyn XuControl) -> Result<IrProbeReport, AdminCliError>;

pub struct ConfigureRequest { pub device: PathBuf, pub mode: IrEmitterMode, pub unit: u8, pub selector: u8,
                              pub on: Vec<u8>, pub off: Vec<u8>, pub config_path: PathBuf,
                              pub dry_run: bool, pub test_on_device: bool }
/// 1. check_ir_node (USB id required); 2. XuEmitterDescriptor::new; 3. read-only GET_INFO +
/// GET_LEN through XuReadOnly (length must equal the payload); 4. if test_on_device:
/// `test_emitter_on_device` (below); 5. render; 6. unless dry_run: `write_ir_emitter_section`.
pub fn configure_ir_emitter(env: &IrEmitterEnv, request: &ConfigureRequest, ctrl: &mut dyn XuControl)
    -> Result<ConfigureReport, AdminCliError>;

/// The only device write: with SIGINT, SIGTERM, SIGHUP, SIGQUIT and SIGTSTP blocked on the
/// calling thread, UvcXuEmitter::open, set(On), GET_CUR == on, set(Off), GET_CUR == off, then
/// restore (also on every error path). At most MAX_DEVICE_TEST_QUERIES requests (11 after R22). The
/// previous signal mask is restored afterwards, so a Ctrl-C pressed meanwhile terminates the
/// process only once the original value is back. `hook` is a test seam called inside the
/// window (production: no-op).
pub fn test_emitter_on_device(ctrl: &mut dyn XuControl, descriptor: &XuEmitterDescriptor,
                              hook: &mut dyn FnMut()) -> Result<DeviceTestReport, AdminCliError>;

/// Managed block markers.
pub const IR_EMITTER_BLOCK_BEGIN: &str = "# BEGIN soos-admin camera.ir_emitter";
pub const IR_EMITTER_BLOCK_END: &str = "# END soos-admin camera.ir_emitter";
/// Replaces (or appends) the managed block in `config_path`:
/// - refuses a symbolic link, a non-regular file, a file above MAX_DAEMON_CONFIG_BYTES, an
///   unbalanced/duplicated marker pair, and a `[camera.ir_emitter]` defined outside the block;
/// - the final text must parse as TOML and through `parse_camera_table`, else nothing is
///   written;
/// - missing file: created 0644 (parent directory must exist);
/// - atomic: exclusive temporary sibling (`create_new`, O_NOFOLLOW, mode of the original or
///   0644), fsync, rename, directory fsync; owner of the original kept when running as root.
pub fn write_ir_emitter_section(config_path: &Path, settings: &IrEmitterSettings) -> Result<WriteOutcome, AdminCliError>;
```

CLI (`args.rs`):

```text
soos-admin camera ir-emitter probe <DEVICE> [--unit <1..=255>] [--config PATH] [--json]
soos-admin camera ir-emitter configure <DEVICE> --unit <1..=255> --selector <1..=255>
        --on <HEX> --off <HEX> [--mode auto|off] [--config PATH] [--dry-run] [--test-on-device]
```

`--on`/`--off`: even number of hex digits, 2..=128 (1..=64 bytes), optional `0x` prefix; anything
else is a clap error. `--mode` default `auto`. `--config` default `/etc/soos/daemon.toml`.
Both actions require euid 0 (checked in `check_ir_node`, injected `euid` in tests). `main.rs` opens
`UvcXuDevice::open(&node)` only after `check_ir_node` succeeded (the open itself is the first
device access beyond the metadata ioctls of `camera probe`).

## 3. Constants & Config

| Name | Location | Value | Sentinel semantics |
|---|---|---|---|
| `XU_MAX_PAYLOAD_BYTES` | `uvc_xu.rs` | 64 | 0 or > 64 ⇒ `InvalidLength` / `EmptyPayload` / `PayloadTooLong` |
| `PROBE_MAX_UNIT_ID` / `PROBE_MAX_SELECTOR` | `uvc_xu.rs` | 32 / 32 | `--unit N` narrows the scan to one unit (1..=255); 0 refused by clap |
| `MAX_PROBE_QUERIES` | `uvc_xu.rs` | 4096 | reached ⇒ `truncated = true`, scan stops |
| `MAX_DEVICE_TEST_QUERIES` | `admin-cli/src/ir_emitter.rs` | 10 | hard cap on `--test-on-device` |
| `STROBE_*` | `strobe.rs` | 8 / 64 / 2 / 12.0 / 20.0 | uncalibrated, provisional; calibration is an IR hardware row (IRA-HW) |
| `MAX_USB_ID_FILE_BYTES` | `ir_emitter_config.rs` | 16 | longer ⇒ `None` |
| `[camera.ir_emitter] mode` | daemon.toml | `"off"` | absent section ⇒ off; anything but `"off"`/`"auto"` ⇒ start-up refused |
| `device` | | `"vvvv:pppp"` | required for auto |
| `unit`, `selector` | | 1..=255 | 0, negative, > 255 ⇒ refused |
| `on`, `off` | | 1..=64 integers 0..=255, equal length, different | empty, longer, out-of-range element, length mismatch, identical ⇒ refused |

## 4. Error Taxonomy

| Error | Where | Result |
|---|---|---|
| `IrEmitterConfigError` | daemon start-up | `DaemonError::Config("Invalid [camera.ir_emitter]: …")`: the daemon refuses to start (no value echoed) |
| `IrEmitterConfigError` | `soos-admin … configure` | `AdminCliError::IrEmitter`, exit 1, nothing written |
| `XuError` during probe | admin | scan stops at the first non-ENOENT/EINVAL error, reported; exit 1 when no control was listed because of it |
| `IrEmitterError::RestoreFailed` | `--test-on-device` | loud error: "original value could not be restored; unplug/replug or reboot" |
| `StrobeError` / `StrobeStatus` ≠ `Stable` | library (consumed by split item 4) | `vetoes_allow() == true`: the future strict path maps it to `Deny`/`Unavailable` → `PAM_IGNORE` |

No path in this PR reaches the auth pipeline, so no `Verdict` changes and `pam_soos.so` is untouched.

## 5. Latency Budget

Not affected: the daemon parses one more optional table at start-up and never drives the emitter
or classifies frames at runtime in this PR. The strobe classifier is O(n) over ≤ 64 samples and
`frame_mean_luma` is one pass over the buffer (≤ 640×480 for IR modules); their budget is
accounted for in split item 4.

## 6. Invariants Touched

- `unsafe` stays in adapter crates: one new `unsafe` block in `crates/camera-v4l/src/uvc_xu.rs`
  with `// SAFETY:`; `#![deny(clippy::undocumented_unsafe_blocks)]` already applies; no new
  `unsafe` anywhere else (`test_business_crates_forbid_unsafe_code` unchanged).
- Root daemon remains the sole owner of `/dev/video*` at runtime: `soos-admin` touches the node
  only on explicit root commands, like `camera probe`; the daemon is not involved.
- No frame, payload or difference data is logged; `ir_mock` frames are synthetic.
- Fail-closed config: invalid `[camera.ir_emitter]` refuses start-up.
- Portable set unaffected (`soos-frame` untouched; `portable_crates_contract` unchanged).
- Camera diagnostics invariants (`camera_diagnostics_contract`): the new admin module streams no
  frame and does not call the `v4l` crate directly; the XU access goes through `soos_camera_v4l`.

## 7. Test Hooks for Tester

- `MockXuControl` (requests spy, faults), `XuReadOnly`, `MockIrEmitter`, `MockIrScene`,
  `MockCameraManager::new_ir`, `synthetic_ir_frame`.
- `soos-test-fixtures::ir_strobe`: `ir_frame(w, h, seq, lit)`, `steady_sequence`,
  `native_alternating_sequence`, `controllable_sequence(pattern)`, `absent_sequence`,
  `class_change_sequence` (all Grey, `Infrared`, synthetic).
- `IrEmitterEnv { euid, probe, camera }` with a fixture `V4lDeviceProbe` and a temporary
  sysfs tree; `test_emitter_on_device(.., hook)` hook observes the thread signal mask.
- `UvcXuDevice::open` on `/dev/null` and on a regular file (refused before `open(2)` of a device).
- Request code and struct layout asserted at compile/test time (`size_of`, `offset_of!`,
  `0xC010_7521` on 64-bit).

Planned test files: `crates/camera-v4l/tests/ir_emitter_config_tests.rs`, `uvc_xu_tests.rs`,
`ir_emitter_tests.rs`, `strobe_classification_tests.rs`, `ir_mock_camera_tests.rs`;
`crates/daemon/tests/ir_emitter_daemon_config_tests.rs`;
`crates/admin-cli/tests/ir_emitter_command_tests.rs`; `tests/invariants/src/ir_emitter_contract.rs`.

## 8. Documentation Drift / ADR

- `Docs/CAMERA_V4L_CRATE.md` "IR Sensors and Emitter Requirements" states "`soos` does **not**
  drive the IR emitter". After this PR: the daemon still does not drive it; `soos-admin` can
  probe read-only and, on an explicit flag, test one control. Rewrite the section accordingly.
- New ADR `[2026-10-13] UVC Extension-Unit IR Emitter Control and Strobe Classification (GitHub
  #378, PR 1 of #353)`: the new `unsafe` site and its invariants, read-only probe, explicit-flag
  device write with restore and signal blocking, `[camera.ir_emitter]` fail-closed with default
  `off`, AD-1..AD-3 marked "assumed — owner to confirm", no Windows Hello equivalence claimed. The
  umbrella ADR "Active-IR Liveness, Emitter Control and Strict IR Mode" remains for split item 4/6.

## 9. Revision 2 (plan evaluation round 1: REVISION_REQUIRED, findings F1–F16)

This section amends §1–§8; where they disagree, this section wins.

- **R1 (F1, MAJOR) — `IrEmitter::get`.** The trait gains
  `fn get(&mut self) -> Result<Option<EmitterState>, IrEmitterError>`: `UvcXuEmitter` issues one
  `GET_CUR` of the payload length and returns `Some(On)` / `Some(Off)` when the value equals the
  `on` / `off` payload, `None` for any other value; a failed read is `Err(Control(..))`.
  `MockIrEmitter::get` returns the state of its scene (`Some(On)` lit, `Some(Off)` unlit, for
  sequence 0 of the scene). `commanded()` stays (last successful command).
- **R2 (F2, MAJOR) — no untested `auto`.** `configure --mode` defaults to **`off`**. `--mode auto`
  is refused unless `--test-on-device` is given in the same run and succeeds (on, off and restore
  all verified); otherwise nothing is written. The ADR states that `mode = "auto"` written by this
  PR has no runtime effect yet and is validated again (and the device identity checked) when split
  item 4 starts driving the emitter.
- **R3 (F3) — signal window.** `test_emitter_on_device` blocks SIGINT, SIGTERM, SIGHUP, SIGQUIT and
  SIGTSTP with `pthread_sigmask` through an RAII `SignalBlock` guard created **before** the
  `UvcXuEmitter` (drop order: emitter restore first, then the mask), so the mask is restored after
  the restore on every path, panics included. `soos-admin` is single-threaded on this path (no
  thread is spawned by `main.rs` or `ir_emitter.rs`; invariant IRA22 asserts no `thread::spawn` /
  `std::thread` in `crates/admin-cli/src/ir_emitter.rs`), so the thread mask is the process mask.
  SIGKILL and SIGSTOP cannot be blocked; the docs say a killed run may leave the control in its
  last written state (recovery: replug or reboot).
- **R4 (F4) — no TOML in soos-admin.** `soos-camera-v4l::ir_emitter_config` adds
  `pub fn parse_ir_emitter_text(text: &str) -> Result<IrEmitterSettings, IrEmitterReadError>`
  (whole daemon.toml text → `[camera]` settings; `Malformed` when not TOML) and
  `pub fn ir_emitter_section_present(text: &str) -> Result<bool, IrEmitterReadError>` (true when
  `camera.ir_emitter` exists). `read_daemon_ir_emitter_config` = shared bounded reader +
  `parse_ir_emitter_text`. `crates/admin-cli/src/ir_emitter.rs` calls only these; invariant IRA23
  forbids `toml::` in it.
- **R5 (F5) — comparison rules.** Controllable: `min(on) − max(off) >= STROBE_MIN_LUMA_DELTA`.
  NativeAlternating: `min(lit parity) − max(other parity) >= DELTA`. Absent: `max < STROBE_DARK_LUMA`
  (strict). Steady: `max − min < DELTA` (strict). Luma valid iff finite and `0.0 <= l <= 255.0`.
  Hand-built boundary tests (`test_ira_threshold_boundaries`, `test_ira_invalid_inputs_fail_closed`).
- **R6 (F6) — ambiguity over short periods.** A commanded pattern is *periodic-aligned* when, for
  some period `p` in `2..=4`, the command of every commanded sample is a function of
  `sequence % p` (inverted patterns included, since any function counts). Such a pattern yields
  `AmbiguousPattern` before any separation test. Test: a native period-4 strobe with the in-phase
  pattern on,on,off,off,… is ambiguous, never `Controllable`.
- **R7 (F7) — sticky `Unclassified`.** Once a block fails before any class was established, the
  monitor stays `Unclassified` (sticky, vetoes) for its lifetime. Test added.
- **R8 (F8) — config writer.** `write_ir_emitter_section` opens the target with
  `O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK` (a symlink fails with `ELOOP` and is refused),
  `fstat` on the descriptor (regular file, ≤ `MAX_DAEMON_CONFIG_BYTES`), bounded read; the
  temporary sibling `.<name>.soos-admin.<pid>.<n>.tmp` is created with `create_new` +
  `O_NOFOLLOW`, mode `original & 0o644` (`0o644` for a new file), owner of the original set on the
  temporary (`fchown`, only when euid 0) before the rename; right before `rename(2)` the target is
  re-checked (same device, inode, size and mtime, or still absent for a new file) and the write is
  aborted otherwise; the temporary is removed on every failure; the directory is fsynced.
- **R9 (F9)** — every new camera test file using mocks starts with `#![cfg(feature = "mock-camera")]`.
- **R10 (F10)** — `Docs/DAEMON.md` documents `[camera]` / `[camera.ir_emitter]` and the upgrade
  note *(superseded by §11 R21: other `[camera]` keys are ignored with a warning)*.
- **R11 (F11)** — `UvcXuDevice::open` re-checks the opened descriptor (`fstat`: character device,
  major 81) before any ioctl. `usb_device_id_of_node` canonicalizes `<sysfs>/<node>/device` and
  reads `idVendor` / `idProduct` from the canonical parent directory (never a lexical `..`).
- **R12 (F12)** — mock timing: tests freeze the mock camera (`set_frozen(true)`) and generate each
  frame with `notify_activity()` after the command, so the frame/command order is deterministic;
  the scene is read at generation time. No sequence log is needed.
- **R13 (F13)** — the epic acceptance line "a VID:PID that differs from the resolved node makes the
  daemon refuse at start-up" stays **unchecked**; the matrix row is `⬜ Pending (split item 4)`.
- **R14 (F14)** — the request code is computed locally (`_IOWR` arithmetic) and asserted
  (`0xC010_7521` on 64-bit) with the struct layout (`size_of`, `offset_of!`) in a unit test of
  `uvc_xu.rs`. `XuControl` has a blanket `impl<T: XuControl + ?Sized> XuControl for &mut T`.
- **R15 (F15)** — walkthrough number 206 is assigned by the orchestrator (other numbers are taken
  by parallel branches). `soos-test-fixtures::ir_strobe::unresponsive_sequence(pattern)` (commands
  issued, scene stays lit) is part of the fixture API; the module is rustfmt-clean.
- **R16 (F16)** — probe robustness: `NotFound` / `Unsupported` skip the selector silently; `Busy`
  or `Io(_)` on one selector skip it and count; after `MAX_PROBE_CONTROL_ERRORS = 16` such errors
  the scan aborts (reported); `PermissionDenied` and `Gone` abort at once.

## 10. Revision 3 (plan evaluation round 2: REVISION_REQUIRED, findings N1–N4)

- **R17 (N1, MAJOR) — deterministic mock frames.** `MockCameraManager` gains
  `#[cfg(feature = "mock-camera")] pub fn step_frame(&self) -> Option<Arc<Frame>>`: it always
  generates exactly one frame (next sequence number, current monotonic time) through the
  `frame_source` hook (the IR scene read at call time) or `generate_synthetic_frame`, stores it
  with the monotonic rule and returns it, even while frozen; it does not touch readiness, warm-up
  or activity. `set_frozen` / `notify_activity` keep their semantics; the wake frame of
  `notify_activity` is now also produced through `frame_source` (identical output for `new`). IR
  mock tests freeze the camera and use `step_frame()` after each command; R12 is replaced.
- **R18 (N2)** — the canonical non-periodic test pattern is `[on, on, off, off, on, off, off, on]`
  (not a function of `sequence % p` for p = 2, 3, 4; checked by hand in the tester contract). A
  parity-periodic pattern on a truly controllable scene returns `AmbiguousPattern`
  (`test_ira_parity_aligned_pattern_is_ambiguous`).
- **R19 (N3)** — the no-thread invariant covers `crates/admin-cli/src/main.rs` too.
- **R20 (N4)** — worst-case probe duration is documented: at most `MAX_PROBE_CONTROL_ERRORS` (16)
  timed-out requests at the uvcvideo control timeout (`UVC_CTRL_CONTROL_TIMEOUT`, 5 s in current
  kernels) before the scan aborts, i.e. about 80 s in the pathological case; normal ENOENT answers
  are immediate. No wall-clock cap is added (the probe is an interactive root diagnostic).

## 11. Revision 4 (candid review CHANGES_REQUESTED, `AI/candid_review_report.md`)

- **R21 (MAJOR)** — only `[camera.ir_emitter]` is strict. Any other key of `[camera]` is ignored,
  like unknown keys of every other daemon table, and reported by
  `ir_emitter_config::ignored_camera_keys` (sanitized, ≤ 64 characters, at most
  `MAX_IGNORED_CAMERA_KEYS` = 8 names); the daemon pushes one start-up warning per name, never the
  value. `tests/physical/screensaver_test.md` now says `[pipeline] idle_timeout_secs`.
- **R22 (MINOR)** — `uvc_xu::XuBudget` forwards at most `limit` requests and refuses later ones
  with `QueryBudgetExhausted` without forwarding them. `MAX_DEVICE_TEST_QUERIES` becomes the real
  maximum, 11: `configure` pre-checks run under a budget of 2, `test_emitter_on_device` under a
  budget of 9 (open 3, on, read, off, read, restore, drop-time retry).
- **R23 (MINOR)** — `IrNode::rdev` holds the device number of the classified node (read before
  and after the classification, equal both times); `UvcXuDevice::rdev()` is the device number of
  the opened descriptor; `ensure_same_node` refuses an unknown or different number before any
  request (called by `soos-admin` right after the open).
- **R24 (SUGGESTION)** — `StrobeStatus::vetoes_allow` stays `false` for every `Stable` class,
  `Stable(Absent)` included; recorded in the ADR as an open decision for split item 4.
