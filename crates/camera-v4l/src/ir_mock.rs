//! Hardware-free IR emitter simulation (`mock-camera` feature; GitHub #378, PR 1 of #353).
//!
//! - [`MockXuControl`]: in-memory extension-unit controls with a request spy and fault
//!   injection, standing in for [`crate::uvc_xu::UvcXuDevice`];
//! - [`MockIrEmitter`]: an [`IrEmitter`] whose [`MockIrScene`] tells
//!   [`crate::MockCameraManager::new_ir`] which synthetic frames are lit;
//! - [`synthetic_ir_frame`]: deterministic, non-biometric Grey frames stamped Infrared.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use soos_frame::{Frame, PixelFormat, SensorType};

use crate::ir_emitter::{EmitterCapability, EmitterState, IrEmitter, IrEmitterError};
use crate::uvc_xu::{check_request_length, XuControl, XuError, XuQuery};

/// How the simulated module responds to commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockEmitterBehavior {
    /// The scene follows the commanded state.
    Controllable,
    /// Even sequence numbers are lit, odd ones dark, whatever is commanded.
    NativeAlternating,
    /// Always lit.
    Steady,
    /// Never lit.
    Absent,
}

const STATE_OFF: u8 = 0;
const STATE_ON: u8 = 1;

/// Shared illumination state of a [`MockIrEmitter`].
#[derive(Debug, Clone)]
pub struct MockIrScene {
    behavior: MockEmitterBehavior,
    commanded: Arc<AtomicU8>,
}

impl MockIrScene {
    /// Whether the frame with this sequence number is lit.
    #[must_use]
    pub fn is_lit(&self, sequence: u64) -> bool {
        match self.behavior {
            MockEmitterBehavior::Controllable => self.commanded.load(Ordering::Acquire) == STATE_ON,
            MockEmitterBehavior::NativeAlternating => sequence.is_multiple_of(2),
            MockEmitterBehavior::Steady => true,
            MockEmitterBehavior::Absent => false,
        }
    }
}

/// Simulated emitter.
#[derive(Debug)]
pub struct MockIrEmitter {
    scene: MockIrScene,
    commanded: Option<EmitterState>,
    fail_next_set: bool,
    set_calls: usize,
}

impl MockIrEmitter {
    /// A simulated emitter, initially off.
    #[must_use]
    pub fn new(behavior: MockEmitterBehavior) -> Self {
        Self {
            scene: MockIrScene {
                behavior,
                commanded: Arc::new(AtomicU8::new(STATE_OFF)),
            },
            commanded: None,
            fail_next_set: false,
            set_calls: 0,
        }
    }

    /// The scene handle for [`crate::MockCameraManager::new_ir`].
    #[must_use]
    pub fn scene(&self) -> MockIrScene {
        self.scene.clone()
    }

    /// The next [`IrEmitter::set`] fails with [`IrEmitterError::Rejected`].
    pub fn fail_next_set(&mut self) {
        self.fail_next_set = true;
    }

    /// Number of `set` calls (failed ones included).
    #[must_use]
    pub fn set_calls(&self) -> usize {
        self.set_calls
    }
}

impl IrEmitter for MockIrEmitter {
    fn capability(&mut self) -> Result<EmitterCapability, IrEmitterError> {
        Ok(EmitterCapability {
            can_read: true,
            can_write: self.scene.behavior == MockEmitterBehavior::Controllable,
            payload_len: 1,
        })
    }

    fn set(&mut self, state: EmitterState) -> Result<(), IrEmitterError> {
        self.set_calls = self.set_calls.saturating_add(1);
        if std::mem::take(&mut self.fail_next_set) {
            return Err(IrEmitterError::Rejected);
        }
        let raw = match state {
            EmitterState::On => STATE_ON,
            EmitterState::Off => STATE_OFF,
        };
        self.scene.commanded.store(raw, Ordering::Release);
        self.commanded = Some(state);
        Ok(())
    }

    fn get(&mut self) -> Result<Option<EmitterState>, IrEmitterError> {
        Ok(Some(if self.scene.is_lit(0) {
            EmitterState::On
        } else {
            EmitterState::Off
        }))
    }

    fn commanded(&self) -> Option<EmitterState> {
        self.commanded
    }
}

/// Disc level of a lit synthetic frame.
pub const MOCK_IR_LIT_LUMA: u8 = 150;
/// Background level of a lit synthetic frame.
const MOCK_IR_LIT_BACKGROUND: u8 = 60;
/// Disc level of an unlit synthetic frame.
pub const MOCK_IR_UNLIT_LUMA: u8 = 12;
/// Background level of an unlit synthetic frame.
const MOCK_IR_UNLIT_BACKGROUND: u8 = 8;

/// A deterministic Grey frame stamped Infrared: a bright disc over a dim background when lit,
/// near-black when unlit, with a small texture. Synthetic and non-biometric.
#[must_use]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "coordinates bounded by the frame geometry; the texture term is below 4"
)]
pub fn synthetic_ir_frame(
    width: u32,
    height: u32,
    sequence: u64,
    timestamp_mono_ns: u64,
    lit: bool,
) -> Frame {
    let (disc, background) = if lit {
        (MOCK_IR_LIT_LUMA, MOCK_IR_LIT_BACKGROUND)
    } else {
        (MOCK_IR_UNLIT_LUMA, MOCK_IR_UNLIT_BACKGROUND)
    };
    let (w, h) = (u64::from(width), u64::from(height));
    let (cx, cy) = (w / 2, h / 2);
    let radius = (w.min(h) / 3).max(1);
    let mut data = Vec::with_capacity((width as usize).saturating_mul(height as usize));
    for y in 0..h {
        for x in 0..w {
            let dx = x.abs_diff(cx);
            let dy = y.abs_diff(cy);
            let inside = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
                <= radius.saturating_mul(radius);
            let base = if inside { disc } else { background };
            data.push(base.saturating_add(((x + y) % 4) as u8));
        }
    }
    Frame::new(
        data,
        width,
        height,
        timestamp_mono_ns,
        PixelFormat::Grey,
        sequence,
    )
    .with_sensor_type(SensorType::Infrared)
}

/// Largest request log kept by [`MockXuControl`].
const MAX_MOCK_REQUEST_LOG: usize = 8192;

/// One simulated control.
#[derive(Debug, Clone)]
struct MockXuEntry {
    unit: u8,
    selector: u8,
    info: u8,
    value: Vec<u8>,
}

/// In-memory extension-unit controls with a request spy and fault injection.
#[derive(Debug, Clone, Default)]
pub struct MockXuControl {
    controls: Vec<MockXuEntry>,
    requests: Vec<(u8, u8, XuQuery)>,
    failing: Vec<(XuQuery, XuError)>,
    sets_before_failure: Option<usize>,
    get_cur_before_failure: Option<(usize, XuError)>,
}

impl MockXuControl {
    /// No control at all (every request answers `NotFound`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) a control with `GET_INFO` bits `info` and current `value`.
    #[must_use]
    pub fn with_control(mut self, unit: u8, selector: u8, info: u8, value: Vec<u8>) -> Self {
        self.controls
            .retain(|c| !(c.unit == unit && c.selector == selector));
        self.controls.push(MockXuEntry {
            unit,
            selector,
            info,
            value,
        });
        self
    }

    /// Every later request of kind `query` fails with `error`.
    pub fn fail_query(&mut self, query: XuQuery, error: XuError) {
        self.failing.push((query, error));
    }

    /// After `successful_sets` more successful `SET_CUR`, every `SET_CUR` fails with `Io(EIO)`.
    pub fn fail_set_after(&mut self, successful_sets: usize) {
        self.sets_before_failure = Some(successful_sets);
    }

    /// After `successful` more successful `GET_CUR`, every `GET_CUR` fails with `error`.
    pub fn fail_get_cur_after(&mut self, successful: usize, error: XuError) {
        self.get_cur_before_failure = Some((successful, error));
    }

    /// Every request received (bounded log).
    #[must_use]
    pub fn requests(&self) -> &[(u8, u8, XuQuery)] {
        &self.requests
    }

    /// Current value of a control.
    #[must_use]
    pub fn value(&self, unit: u8, selector: u8) -> Option<Vec<u8>> {
        self.controls
            .iter()
            .find(|c| c.unit == unit && c.selector == selector)
            .map(|c| c.value.clone())
    }
}

impl XuControl for MockXuControl {
    fn query(
        &mut self,
        unit: u8,
        selector: u8,
        query: XuQuery,
        data: &mut [u8],
    ) -> Result<(), XuError> {
        check_request_length(query, data.len())?;
        if self.requests.len() < MAX_MOCK_REQUEST_LOG {
            self.requests.push((unit, selector, query));
        }
        if let Some((_, error)) = self.failing.iter().find(|(q, _)| *q == query) {
            return Err(*error);
        }
        if query == XuQuery::SetCur {
            if let Some(remaining) = self.sets_before_failure.as_mut() {
                if *remaining == 0 {
                    return Err(XuError::Io(libc::EIO));
                }
                *remaining = remaining.saturating_sub(1);
            }
        }
        if query == XuQuery::GetCur {
            if let Some((remaining, error)) = self.get_cur_before_failure.as_mut() {
                if *remaining == 0 {
                    return Err(*error);
                }
                *remaining = remaining.saturating_sub(1);
            }
        }
        let entry = self
            .controls
            .iter_mut()
            .find(|c| c.unit == unit && c.selector == selector)
            .ok_or(XuError::NotFound)?;
        match query {
            XuQuery::GetLen => {
                let len = u16::try_from(entry.value.len()).map_err(|_| XuError::Io(libc::EIO))?;
                data.copy_from_slice(&len.to_le_bytes());
            }
            XuQuery::GetInfo => data.fill(entry.info),
            XuQuery::SetCur => {
                if entry.info & 0x02 == 0 {
                    return Err(XuError::Unsupported);
                }
                if data.len() != entry.value.len() {
                    return Err(XuError::Io(libc::ENOBUFS));
                }
                entry.value.copy_from_slice(data);
            }
            _ => {
                if entry.info & 0x01 == 0 {
                    return Err(XuError::Unsupported);
                }
                if data.len() != entry.value.len() {
                    return Err(XuError::Io(libc::ENOBUFS));
                }
                data.copy_from_slice(&entry.value);
            }
        }
        Ok(())
    }
}
