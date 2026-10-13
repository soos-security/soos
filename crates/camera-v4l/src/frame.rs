//! Frame structures and pixel formats for camera capture.
//!
//! The types live in the pure crate `soos-frame` (GitHub #367, ADR 2026-10-13 "macOS Port in a
//! Separate Repository") and are re-exported here, so every `soos_camera_v4l::Frame`,
//! `soos_camera_v4l::frame::PixelFormat` and `crate::frame::*` path keeps naming the same types.

pub use soos_frame::{Frame, PixelFormat};
