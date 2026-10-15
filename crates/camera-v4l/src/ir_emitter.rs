//! IR emitter control (GitHub #378, PR 1 of #353).
//!
//! [`IrEmitter`] is what the capture side will drive (split item 4 of #353): switch the
//! near-infrared emitter on or off and read its state back. [`UvcXuEmitter`] implements it on a
//! UVC extension-unit control described by `[camera.ir_emitter]`; it reads the original value
//! when it is opened and writes it back on [`UvcXuEmitter::restore`] and on drop, so the device
//! is left as it was found. The mock implementation lives in [`crate::ir_mock`]
//! (`mock-camera` feature).

use crate::ir_emitter_config::XuEmitterDescriptor;
use crate::uvc_xu::{xu_get_cur, xu_get_info, xu_get_len, xu_set_cur, XuControl, XuError};

/// Emitter state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmitterState {
    /// Emitter lit.
    On,
    /// Emitter dark.
    Off,
}

/// What the emitter control supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmitterCapability {
    /// The state can be read back.
    pub can_read: bool,
    /// The state can be commanded.
    pub can_write: bool,
    /// Control payload length in bytes.
    pub payload_len: u16,
}

/// Why an emitter operation failed. Carries no payload byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IrEmitterError {
    /// The extension-unit request failed.
    #[error("emitter control request failed: {0}")]
    Control(#[from] XuError),
    /// `GET_LEN` of the control differs from the configured payload length.
    #[error("the control holds {control} bytes but the configured payloads hold {configured}")]
    PayloadLengthMismatch {
        /// `GET_LEN`.
        control: u16,
        /// Configured payload length.
        configured: usize,
    },
    /// `GET_INFO` says the control cannot be written.
    #[error("the control cannot be written")]
    NotWritable,
    /// `GET_INFO` says the control cannot be read.
    #[error("the control cannot be read")]
    NotReadable,
    /// Writing the original value back failed; the device may stay in the last written state
    /// until it is unplugged or the host reboots.
    #[error("the original control value could not be restored ({0}); unplug and replug the camera or reboot")]
    RestoreFailed(XuError),
    /// The mock refused the command (fault injection).
    #[error("the emitter refused the command")]
    Rejected,
}

/// An IR emitter the capture side can switch. Object safe.
pub trait IrEmitter {
    /// What the emitter supports.
    ///
    /// # Errors
    ///
    /// The control error.
    fn capability(&mut self) -> Result<EmitterCapability, IrEmitterError>;

    /// Switches the emitter.
    ///
    /// # Errors
    ///
    /// The control error; [`IrEmitter::commanded`] is unchanged then.
    fn set(&mut self, state: EmitterState) -> Result<(), IrEmitterError>;

    /// Reads the current state from the device: `Some(On)` / `Some(Off)` when it matches one of
    /// the configured states, `None` for anything else.
    ///
    /// # Errors
    ///
    /// The control error (a failed read is never guessed).
    fn get(&mut self) -> Result<Option<EmitterState>, IrEmitterError>;

    /// Last state successfully commanded through [`IrEmitter::set`] (`None` before).
    fn commanded(&self) -> Option<EmitterState>;
}

/// Emitter on a UVC extension-unit control.
pub struct UvcXuEmitter<C: XuControl> {
    control: C,
    descriptor: XuEmitterDescriptor,
    info_get: bool,
    info_set: bool,
    original: Vec<u8>,
    commanded: Option<EmitterState>,
    /// A write was attempted since the last restore.
    dirty: bool,
}

impl<C: XuControl> UvcXuEmitter<C> {
    /// Checks the control (`GET_INFO`: GET and SET supported; `GET_LEN`: equal to the payload
    /// length) and reads its original value (`GET_CUR`). Nothing is written.
    ///
    /// # Errors
    ///
    /// The control error, [`IrEmitterError::NotReadable`], [`IrEmitterError::NotWritable`] or
    /// [`IrEmitterError::PayloadLengthMismatch`].
    pub fn open(mut control: C, descriptor: XuEmitterDescriptor) -> Result<Self, IrEmitterError> {
        let (unit, selector) = (descriptor.unit, descriptor.selector);
        let info = xu_get_info(&mut control, unit, selector)?;
        if !info.supports_get() {
            return Err(IrEmitterError::NotReadable);
        }
        if !info.supports_set() {
            return Err(IrEmitterError::NotWritable);
        }
        let len = xu_get_len(&mut control, unit, selector)?;
        if usize::from(len) != descriptor.on.len() {
            return Err(IrEmitterError::PayloadLengthMismatch {
                control: len,
                configured: descriptor.on.len(),
            });
        }
        let original = xu_get_cur(&mut control, unit, selector, descriptor.on.len())?;
        Ok(Self {
            control,
            descriptor,
            info_get: info.supports_get(),
            info_set: info.supports_set(),
            original,
            commanded: None,
            dirty: false,
        })
    }

    /// Writes the original value back; a no-op when nothing was written since the last
    /// restore. Idempotent.
    ///
    /// # Errors
    ///
    /// [`IrEmitterError::RestoreFailed`] (the emitter stays dirty, so drop tries again).
    pub fn restore(&mut self) -> Result<(), IrEmitterError> {
        if !self.dirty {
            return Ok(());
        }
        xu_set_cur(
            &mut self.control,
            self.descriptor.unit,
            self.descriptor.selector,
            &self.original,
        )
        .map_err(IrEmitterError::RestoreFailed)?;
        self.dirty = false;
        Ok(())
    }

    /// The configured control.
    #[must_use]
    pub fn descriptor(&self) -> &XuEmitterDescriptor {
        &self.descriptor
    }
}

impl<C: XuControl> IrEmitter for UvcXuEmitter<C> {
    fn capability(&mut self) -> Result<EmitterCapability, IrEmitterError> {
        Ok(EmitterCapability {
            can_read: self.info_get,
            can_write: self.info_set,
            payload_len: u16::try_from(self.descriptor.on.len()).unwrap_or(u16::MAX),
        })
    }

    fn set(&mut self, state: EmitterState) -> Result<(), IrEmitterError> {
        let payload = match state {
            EmitterState::On => &self.descriptor.on,
            EmitterState::Off => &self.descriptor.off,
        };
        // Marked before the write: a failed write leaves the device state unknown.
        self.dirty = true;
        xu_set_cur(
            &mut self.control,
            self.descriptor.unit,
            self.descriptor.selector,
            payload,
        )?;
        self.commanded = Some(state);
        Ok(())
    }

    fn get(&mut self) -> Result<Option<EmitterState>, IrEmitterError> {
        let current = xu_get_cur(
            &mut self.control,
            self.descriptor.unit,
            self.descriptor.selector,
            self.descriptor.on.len(),
        )?;
        Ok(if current == self.descriptor.on {
            Some(EmitterState::On)
        } else if current == self.descriptor.off {
            Some(EmitterState::Off)
        } else {
            None
        })
    }

    fn commanded(&self) -> Option<EmitterState> {
        self.commanded
    }
}

impl<C: XuControl> Drop for UvcXuEmitter<C> {
    fn drop(&mut self) {
        if self.restore().is_err() {
            tracing::warn!(
                unit = self.descriptor.unit,
                selector = self.descriptor.selector,
                "IR emitter control could not be restored; unplug and replug the camera or reboot"
            );
        }
    }
}
