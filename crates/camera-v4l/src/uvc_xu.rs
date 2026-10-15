//! UVC extension-unit (XU) control access through `UVCIOC_CTRL_QUERY` (GitHub #378, PR 1 of
//! #353; ADR 2026-10-13 "UVC Extension-Unit IR Emitter Control and Strobe Classification").
//!
//! Many Windows Hello IR modules switch their near-infrared emitter only through a vendor
//! extension-unit control that the stock `uvcvideo` driver never sets. This module gives
//! bounded, typed access to such controls:
//!
//! - [`XuControl`] is the raw request seam (production: [`UvcXuDevice`]; tests: the
//!   `mock-camera` [`crate::ir_mock::MockXuControl`]);
//! - [`XuReadOnly`] refuses every write without forwarding it, so the read-only probe
//!   ([`probe_xu_controls`]) cannot change the device even by mistake;
//! - every buffer is at most [`XU_MAX_PAYLOAD_BYTES`] and every scan is bounded.
//!
//! The ioctl in [`UvcXuDevice::query`] is the only `unsafe` block added for the IR emitter work.

use std::fs::{File, OpenOptions};
use std::ops::RangeInclusive;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

/// UVC class-specific request codes (USB Video Class 1.5, table A-8; `linux/usb/video.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum XuQuery {
    /// `SET_CUR`: writes the current value (the only write).
    SetCur = 0x01,
    /// `GET_CUR`: reads the current value.
    GetCur = 0x81,
    /// `GET_MIN`.
    GetMin = 0x82,
    /// `GET_MAX`.
    GetMax = 0x83,
    /// `GET_RES`.
    GetRes = 0x84,
    /// `GET_LEN`: the control length in bytes (2-byte little-endian answer).
    GetLen = 0x85,
    /// `GET_INFO`: the capability bits (1-byte answer).
    GetInfo = 0x86,
    /// `GET_DEF`.
    GetDef = 0x87,
}

impl XuQuery {
    /// The request code sent to the device.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// `true` only for `SET_CUR`.
    #[must_use]
    pub const fn is_write(self) -> bool {
        matches!(self, Self::SetCur)
    }
}

/// Largest control payload soos reads or writes (also the `[camera.ir_emitter]` bound).
pub const XU_MAX_PAYLOAD_BYTES: usize = 64;
/// Units scanned by default by the read-only probe (`1..=PROBE_MAX_UNIT_ID`).
pub const PROBE_MAX_UNIT_ID: u8 = 32;
/// Selectors scanned per unit (`1..=PROBE_MAX_SELECTOR`).
pub const PROBE_MAX_SELECTOR: u8 = 32;
/// Hard cap on UVC requests issued by one probe (failed requests included).
pub const MAX_PROBE_QUERIES: usize = 4096;
/// Transient per-control errors (`Busy`, `Io`) tolerated by one probe before it aborts.
pub const MAX_PROBE_CONTROL_ERRORS: usize = 16;

/// Why an extension-unit request failed. Carries no payload byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum XuError {
    /// `ENOENT`: no such unit or selector.
    #[error("no such extension-unit control")]
    NotFound,
    /// `EINVAL`, `ENOTTY`, `EOPNOTSUPP`, `ENOSYS`, `EBADRQC`: not a UVC node, or request refused.
    #[error("request not supported by this node or control")]
    Unsupported,
    /// `EACCES`, `EPERM`.
    #[error("permission denied")]
    PermissionDenied,
    /// `EBUSY`.
    #[error("device busy")]
    Busy,
    /// `ENODEV`, `ENXIO`: the device is gone.
    #[error("device gone")]
    Gone,
    /// Any other errno.
    #[error("I/O error (errno {0})")]
    Io(i32),
    /// The path is not a V4L2 character device (refused before or right after `open(2)`).
    #[error("not a V4L2 character device")]
    NotV4l2Device,
    /// The request buffer length is not allowed for this request.
    #[error("invalid request length {len}")]
    InvalidLength {
        /// The refused length.
        len: usize,
    },
    /// `SET_CUR` through the read-only wrapper.
    #[error("write refused by the read-only probe")]
    WriteRefused,
    /// The probe reached its request budget.
    #[error("probe request budget exhausted")]
    QueryBudgetExhausted,
}

impl XuError {
    /// Classifies an errno.
    #[must_use]
    pub fn from_errno(errno: i32) -> Self {
        match errno {
            libc::ENOENT => Self::NotFound,
            libc::EINVAL | libc::ENOTTY | libc::EOPNOTSUPP | libc::ENOSYS | libc::EBADRQC => {
                Self::Unsupported
            }
            libc::EACCES | libc::EPERM => Self::PermissionDenied,
            libc::EBUSY => Self::Busy,
            libc::ENODEV | libc::ENXIO => Self::Gone,
            other => Self::Io(other),
        }
    }

    fn from_io(error: &std::io::Error) -> Self {
        error.raw_os_error().map_or(Self::Io(0), Self::from_errno)
    }
}

/// Checks the request buffer length the kernel expects for `query`.
///
/// # Errors
///
/// [`XuError::InvalidLength`] for `GET_LEN` ≠ 2, `GET_INFO` ≠ 1, or any other request with
/// 0 or more than [`XU_MAX_PAYLOAD_BYTES`] bytes.
pub fn check_request_length(query: XuQuery, len: usize) -> Result<(), XuError> {
    let ok = match query {
        XuQuery::GetLen => len == 2,
        XuQuery::GetInfo => len == 1,
        _ => (1..=XU_MAX_PAYLOAD_BYTES).contains(&len),
    };
    if ok {
        Ok(())
    } else {
        Err(XuError::InvalidLength { len })
    }
}

/// Raw access to UVC extension-unit controls (`UVCIOC_CTRL_QUERY`).
pub trait XuControl {
    /// Issues one request; `data.len()` is the request size (`GET_LEN`: 2, `GET_INFO`: 1,
    /// otherwise 1..=[`XU_MAX_PAYLOAD_BYTES`]). Implementations validate the length first.
    ///
    /// # Errors
    ///
    /// An [`XuError`] for a refused length or a failed request.
    fn query(
        &mut self,
        unit: u8,
        selector: u8,
        query: XuQuery,
        data: &mut [u8],
    ) -> Result<(), XuError>;
}

impl<T: XuControl + ?Sized> XuControl for &mut T {
    fn query(
        &mut self,
        unit: u8,
        selector: u8,
        query: XuQuery,
        data: &mut [u8],
    ) -> Result<(), XuError> {
        (**self).query(unit, selector, query, data)
    }
}

/// `GET_INFO` capability bits (UVC 1.5 §4.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XuInfo {
    /// The raw bitmap.
    pub raw: u8,
}

impl XuInfo {
    /// Bit 0: `GET_*` supported.
    #[must_use]
    pub const fn supports_get(self) -> bool {
        self.raw & 0x01 != 0
    }

    /// Bit 1: `SET_CUR` supported.
    #[must_use]
    pub const fn supports_set(self) -> bool {
        self.raw & 0x02 != 0
    }
}

/// `GET_LEN` of one control.
///
/// # Errors
///
/// The request error.
pub fn xu_get_len(ctrl: &mut dyn XuControl, unit: u8, selector: u8) -> Result<u16, XuError> {
    let mut buf = [0u8; 2];
    ctrl.query(unit, selector, XuQuery::GetLen, &mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

/// `GET_INFO` of one control.
///
/// # Errors
///
/// The request error.
pub fn xu_get_info(ctrl: &mut dyn XuControl, unit: u8, selector: u8) -> Result<XuInfo, XuError> {
    let mut buf = [0u8; 1];
    ctrl.query(unit, selector, XuQuery::GetInfo, &mut buf)?;
    Ok(XuInfo { raw: buf[0] })
}

/// `GET_CUR` of `len` bytes (1..=[`XU_MAX_PAYLOAD_BYTES`], checked before any request).
///
/// # Errors
///
/// [`XuError::InvalidLength`] or the request error.
pub fn xu_get_cur(
    ctrl: &mut dyn XuControl,
    unit: u8,
    selector: u8,
    len: usize,
) -> Result<Vec<u8>, XuError> {
    check_request_length(XuQuery::GetCur, len)?;
    let mut buf = vec![0u8; len];
    ctrl.query(unit, selector, XuQuery::GetCur, &mut buf)?;
    Ok(buf)
}

/// `SET_CUR` of `payload` (1..=[`XU_MAX_PAYLOAD_BYTES`] bytes, checked before any request).
///
/// # Errors
///
/// [`XuError::InvalidLength`] or the request error.
pub fn xu_set_cur(
    ctrl: &mut dyn XuControl,
    unit: u8,
    selector: u8,
    payload: &[u8],
) -> Result<(), XuError> {
    check_request_length(XuQuery::SetCur, payload.len())?;
    let mut buf = [0u8; XU_MAX_PAYLOAD_BYTES];
    let slot = buf
        .get_mut(..payload.len())
        .ok_or(XuError::InvalidLength { len: payload.len() })?;
    slot.copy_from_slice(payload);
    ctrl.query(unit, selector, XuQuery::SetCur, slot)
}

/// Read-only view of a control: every `GET_*` is forwarded, `SET_CUR` is refused with
/// [`XuError::WriteRefused`] and never reaches the inner control.
pub struct XuReadOnly<'a> {
    inner: &'a mut dyn XuControl,
}

impl<'a> XuReadOnly<'a> {
    /// Wraps `inner`.
    pub fn new(inner: &'a mut dyn XuControl) -> Self {
        Self { inner }
    }
}

impl XuControl for XuReadOnly<'_> {
    fn query(
        &mut self,
        unit: u8,
        selector: u8,
        query: XuQuery,
        data: &mut [u8],
    ) -> Result<(), XuError> {
        if query.is_write() {
            return Err(XuError::WriteRefused);
        }
        self.inner.query(unit, selector, query, data)
    }
}

/// Request budget: forwards at most `limit` requests to the inner control and refuses every
/// later one with [`XuError::QueryBudgetExhausted`] without forwarding it (GitHub #378,
/// candid review: the device-test bound is enforced in code, not only documented).
pub struct XuBudget<'a> {
    inner: &'a mut dyn XuControl,
    limit: usize,
    issued: usize,
}

impl<'a> XuBudget<'a> {
    /// Wraps `inner` with a budget of `limit` requests.
    pub fn new(inner: &'a mut dyn XuControl, limit: usize) -> Self {
        Self {
            inner,
            limit,
            issued: 0,
        }
    }

    /// Requests forwarded so far.
    #[must_use]
    pub fn issued(&self) -> usize {
        self.issued
    }
}

impl XuControl for XuBudget<'_> {
    fn query(
        &mut self,
        unit: u8,
        selector: u8,
        query: XuQuery,
        data: &mut [u8],
    ) -> Result<(), XuError> {
        if self.issued >= self.limit {
            return Err(XuError::QueryBudgetExhausted);
        }
        self.issued = self.issued.saturating_add(1);
        self.inner.query(unit, selector, query, data)
    }
}

/// One control found by the read-only scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XuControlReport {
    /// Extension unit id.
    pub unit: u8,
    /// Control selector.
    pub selector: u8,
    /// `GET_LEN`.
    pub len: u16,
    /// `GET_INFO`.
    pub info: XuInfo,
    /// `GET_CUR` value; `None` when GET is unsupported, the length is 0 or above
    /// [`XU_MAX_PAYLOAD_BYTES`], or the read failed.
    pub current: Option<Vec<u8>>,
}

/// Result of [`probe_xu_controls`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct XuProbeReport {
    /// Controls found, in unit then selector order.
    pub controls: Vec<XuControlReport>,
    /// Requests issued (failed ones included).
    pub queries_issued: usize,
    /// `true` when [`MAX_PROBE_QUERIES`] stopped the scan.
    pub truncated: bool,
    /// Error that stopped the scan (`PermissionDenied`, `Gone`, or the transient error that
    /// exceeded [`MAX_PROBE_CONTROL_ERRORS`]).
    pub aborted: Option<XuError>,
}

/// What one probe step decided.
enum Step<T> {
    Value(T),
    Skip,
    Stop,
}

/// Request counter and error policy of one probe.
struct ProbeState<'a> {
    ctrl: XuReadOnly<'a>,
    report: XuProbeReport,
    transient_errors: usize,
}

impl ProbeState<'_> {
    /// Issues one request under the budget and classifies its failure.
    fn run<T>(
        &mut self,
        request: impl FnOnce(&mut dyn XuControl) -> Result<T, XuError>,
    ) -> Step<T> {
        if self.report.queries_issued >= MAX_PROBE_QUERIES {
            self.report.truncated = true;
            return Step::Stop;
        }
        self.report.queries_issued = self.report.queries_issued.saturating_add(1);
        match request(&mut self.ctrl) {
            Ok(value) => Step::Value(value),
            Err(XuError::NotFound | XuError::Unsupported | XuError::InvalidLength { .. }) => {
                Step::Skip
            }
            Err(error @ (XuError::Busy | XuError::Io(_))) => {
                self.transient_errors = self.transient_errors.saturating_add(1);
                if self.transient_errors >= MAX_PROBE_CONTROL_ERRORS {
                    self.report.aborted = Some(error);
                    Step::Stop
                } else {
                    Step::Skip
                }
            }
            Err(error) => {
                self.report.aborted = Some(error);
                Step::Stop
            }
        }
    }
}

/// Read-only scan of `units` × `1..=PROBE_MAX_SELECTOR`: `GET_LEN`, then `GET_INFO`, then
/// `GET_CUR` when GET is supported and the length is within [`XU_MAX_PAYLOAD_BYTES`].
///
/// `ctrl` is wrapped in [`XuReadOnly`], so no write can reach it. The scan stops at
/// [`MAX_PROBE_QUERIES`] requests (`truncated`), at the first `PermissionDenied` / `Gone`, or
/// after [`MAX_PROBE_CONTROL_ERRORS`] transient errors (`aborted`).
pub fn probe_xu_controls(ctrl: &mut dyn XuControl, units: RangeInclusive<u8>) -> XuProbeReport {
    let mut state = ProbeState {
        ctrl: XuReadOnly::new(ctrl),
        report: XuProbeReport::default(),
        transient_errors: 0,
    };
    'scan: for unit in units {
        for selector in 1..=PROBE_MAX_SELECTOR {
            let len = match state.run(|c| xu_get_len(c, unit, selector)) {
                Step::Value(len) => len,
                Step::Skip => continue,
                Step::Stop => break 'scan,
            };
            let info = match state.run(|c| xu_get_info(c, unit, selector)) {
                Step::Value(info) => info,
                Step::Skip => continue,
                Step::Stop => break 'scan,
            };
            let len_usize = usize::from(len);
            let mut current = None;
            if info.supports_get() && (1..=XU_MAX_PAYLOAD_BYTES).contains(&len_usize) {
                match state.run(|c| xu_get_cur(c, unit, selector, len_usize)) {
                    Step::Value(value) => current = Some(value),
                    Step::Skip => {}
                    Step::Stop => {
                        state.report.controls.push(XuControlReport {
                            unit,
                            selector,
                            len,
                            info,
                            current,
                        });
                        break 'scan;
                    }
                }
            }
            state.report.controls.push(XuControlReport {
                unit,
                selector,
                len,
                info,
                current,
            });
        }
    }
    state.report
}

/// Kernel ABI of `struct uvc_xu_control_query` (`include/uapi/linux/uvcvideo.h`).
#[repr(C)]
struct UvcXuControlQuery {
    unit: u8,
    selector: u8,
    query: u8,
    size: u16,
    data: *mut u8,
}

/// `_IOWR(type, nr, size)` of `asm-generic/ioctl.h`.
const fn iowr(kind: u8, nr: u8, size: usize) -> u64 {
    // The size field is 14 bits wide; the struct is 12 or 16 bytes.
    (3u64 << 30) | (((size as u64) & 0x3fff) << 16) | ((kind as u64) << 8) | (nr as u64)
}

/// `UVCIOC_CTRL_QUERY = _IOWR('u', 0x21, struct uvc_xu_control_query)` (`0xC010_7521` on
/// 64-bit targets).
pub const UVCIOC_CTRL_QUERY: u64 = iowr(b'u', 0x21, size_of::<UvcXuControlQuery>());

/// Production control on an open V4L2 node (UVC driver).
#[derive(Debug)]
pub struct UvcXuDevice {
    file: File,
    rdev: u64,
}

impl UvcXuDevice {
    /// Opens `path` for extension-unit requests.
    ///
    /// Anything but a V4L2 character device (major 81) is refused before `open(2)`; the opened
    /// descriptor is checked again (`fstat`) before it is used, so a node swapped between the
    /// check and the open is refused as well. Opened read-write with
    /// `O_CLOEXEC | O_NONBLOCK | O_NOCTTY`.
    ///
    /// # Errors
    ///
    /// [`XuError::NotV4l2Device`], or the `open(2)` error.
    pub fn open(path: &Path) -> Result<Self, XuError> {
        crate::diagnostics::ensure_v4l2_char_device(path).map_err(|failure| match failure {
            crate::diagnostics::ProbeFailure::NotFound => XuError::NotFound,
            crate::diagnostics::ProbeFailure::PermissionDenied => XuError::PermissionDenied,
            crate::diagnostics::ProbeFailure::Busy => XuError::Busy,
            crate::diagnostics::ProbeFailure::Other(_) => XuError::NotV4l2Device,
        })?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(path)
            .map_err(|e| XuError::from_io(&e))?;
        let metadata = file.metadata().map_err(|e| XuError::from_io(&e))?;
        if !crate::diagnostics::is_v4l2_char_device(&metadata) {
            return Err(XuError::NotV4l2Device);
        }
        Ok(Self {
            file,
            rdev: std::os::unix::fs::MetadataExt::rdev(&metadata),
        })
    }

    /// Device number (`st_rdev`) of the opened descriptor, so a caller can prove it is the node
    /// it classified before opening it.
    #[must_use]
    pub fn rdev(&self) -> u64 {
        self.rdev
    }
}

impl XuControl for UvcXuDevice {
    fn query(
        &mut self,
        unit: u8,
        selector: u8,
        query: XuQuery,
        data: &mut [u8],
    ) -> Result<(), XuError> {
        check_request_length(query, data.len())?;
        let size =
            u16::try_from(data.len()).map_err(|_| XuError::InvalidLength { len: data.len() })?;
        let mut request = UvcXuControlQuery {
            unit,
            selector,
            query: query.code(),
            size,
            data: data.as_mut_ptr(),
        };
        let fd = self.file.as_raw_fd();
        let code = v4l::v4l2::vidioc::_IOC_TYPE::try_from(UVCIOC_CTRL_QUERY)
            .map_err(|_| XuError::Unsupported)?;
        // SAFETY: `fd` belongs to `self.file`, open for the whole call. `request` is a
        // `#[repr(C)]` mirror of `struct uvc_xu_control_query` (layout asserted by the unit
        // test below) and is exclusively borrowed for the call; its `data` pointer refers to
        // the caller's exclusively borrowed slice of exactly `size` bytes (1..=64, or 2 for
        // GET_LEN and 1 for GET_INFO, checked above), and the kernel copies at most `size`
        // bytes from or to it and keeps no pointer after returning.
        unsafe {
            v4l::v4l2::ioctl(
                fd,
                code,
                std::ptr::from_mut(&mut request).cast::<std::os::raw::c_void>(),
            )
        }
        .map_err(|e| XuError::from_io(&e))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "Unit tests use direct assertions")]
mod tests {
    use super::*;

    /// IRA5: the mirror struct and the request code match the kernel ABI.
    #[test]
    fn test_ira_uvc_query_struct_layout_and_request_code() {
        assert_eq!(std::mem::offset_of!(UvcXuControlQuery, unit), 0);
        assert_eq!(std::mem::offset_of!(UvcXuControlQuery, selector), 1);
        assert_eq!(std::mem::offset_of!(UvcXuControlQuery, query), 2);
        assert_eq!(std::mem::offset_of!(UvcXuControlQuery, size), 4);
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(std::mem::offset_of!(UvcXuControlQuery, data), 8);
            assert_eq!(size_of::<UvcXuControlQuery>(), 16);
            assert_eq!(UVCIOC_CTRL_QUERY, 0xC010_7521);
        }
        #[cfg(target_pointer_width = "32")]
        {
            assert_eq!(size_of::<UvcXuControlQuery>(), 12);
            assert_eq!(UVCIOC_CTRL_QUERY, 0xC00C_7521);
        }
    }

    /// IRA5: request lengths follow the kernel rules.
    #[test]
    fn test_ira_request_length_rules() {
        assert!(check_request_length(XuQuery::GetLen, 2).is_ok());
        assert!(check_request_length(XuQuery::GetLen, 1).is_err());
        assert!(check_request_length(XuQuery::GetInfo, 1).is_ok());
        assert!(check_request_length(XuQuery::GetInfo, 2).is_err());
        assert!(check_request_length(XuQuery::SetCur, 64).is_ok());
        assert!(check_request_length(XuQuery::SetCur, 65).is_err());
        assert!(check_request_length(XuQuery::GetCur, 0).is_err());
        assert_eq!(XuError::from_errno(libc::ENOENT), XuError::NotFound);
        assert_eq!(XuError::from_errno(libc::EBADRQC), XuError::Unsupported);
        assert_eq!(
            XuError::from_errno(libc::ENOBUFS),
            XuError::Io(libc::ENOBUFS)
        );
    }
}
