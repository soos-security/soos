//! Wipe guard for ONNX Runtime session outputs (GitHub #255, review finding VIS-13).
//!
//! The raw embedding, SCRFD landmark tensors and PAD logits are written by ONNX Runtime into
//! output buffers it owns (`SessionOutputs`). Copying a value into a `Zeroizing` container does
//! not wipe the ORT allocation it came from. [`ZeroizingOutputs`] owns the outputs of one
//! `Session::run`, lends them to the decoder, and overwrites every `f32` output tensor in place
//! (`try_extract_tensor_mut`) before the allocation is released, on every path including early
//! error returns.
//!
//! Limitation: ORT-internal intermediate activation buffers (arena memory) are not reachable
//! through the public API and are not wiped (ADR 2026-09-30 "ORT Output Tensors Wiped In
//! Place", `AI/DECISIONS.md`).

use std::ops::Deref;

use ort::session::SessionOutputs;
use zeroize::Zeroize;

/// Owns the outputs of one ORT `Session::run` and wipes every `f32` output tensor on drop.
pub struct ZeroizingOutputs<'r> {
    outputs: SessionOutputs<'r>,
}

impl<'r> ZeroizingOutputs<'r> {
    /// Takes ownership of the outputs of a session run.
    pub fn new(outputs: SessionOutputs<'r>) -> Self {
        Self { outputs }
    }

    /// Overwrites every `f32` output tensor with zeros in place and returns how many tensors
    /// were wiped. Non-`f32` or non-tensor outputs are skipped (the production models emit
    /// only `f32` tensors). Idempotent; also called on drop.
    pub fn wipe(&mut self) -> usize {
        let mut wiped = 0usize;
        for mut value in self.outputs.values_mut() {
            if let Ok((_, data)) = value.try_extract_tensor_mut::<f32>() {
                data.zeroize();
                wiped = wiped.saturating_add(1);
            }
        }
        wiped
    }
}

impl<'r> Deref for ZeroizingOutputs<'r> {
    type Target = SessionOutputs<'r>;

    fn deref(&self) -> &Self::Target {
        &self.outputs
    }
}

impl Drop for ZeroizingOutputs<'_> {
    fn drop(&mut self) {
        let _wiped = self.wipe();
    }
}
