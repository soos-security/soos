//! `soos-frame` — pure, I/O-free camera frame types shared by the capture backends
//! (`soos-camera-v4l` on Linux) and the vision pipeline (`soos-vision`).
//!
//! The crate is part of the portable set that the separate `soos-macos` repository may consume
//! (ADR 2026-10-13 "macOS Port in a Separate Repository"): it has no I/O, no platform-specific
//! dependency and no `unsafe` code. IR-only types added later (IR anti-spoofing) go behind a
//! non-default `ir` feature so an RGB-only consumer never compiles them.

#![forbid(unsafe_code)]

mod frame;
mod sensor;

pub use frame::{Frame, PixelFormat};
pub use sensor::SensorType;
