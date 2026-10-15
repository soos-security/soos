//! # soos-lock-widget
//!
//! Animated soos face-scan widget drawn as the background of `swaylock-plugin`
//! (`swaylock-plugin --command soos-lock-widget`), ADR 2026-10-15 "Lock-Screen Adapters for
//! swaylock-plugin and hyprlock" (GitHub #401, `Docs/LOCKSCREEN_WIDGET.md`).
//!
//! The Wayland `ext-session-lock-v1` protocol gives the locker exclusive ownership of the locked
//! outputs; swaylock-plugin lets a wlr-layer-shell program draw its background. This program draws
//! the brand background, the pink star and a scan ring from the coarse state read by
//! `soos-lockstatus` (Idle, Scanning, Not recognized). It binds no seat: it can neither read nor
//! send input, and it cannot unlock or start anything.
//!
//! Everything except [`wayland`] is pure and unit-tested: [`scene`] (motion), [`layout`]
//! (geometry and memory bounds), [`raster`] and [`font`] (CPU drawing), [`render`].

#![forbid(unsafe_code)]

pub mod args;
pub mod font;
pub mod layout;
pub mod palette;
pub mod raster;
pub mod render;
pub mod scene;
pub mod wayland;
