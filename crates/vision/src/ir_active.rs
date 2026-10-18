//! Active-illumination on/off differential liveness cues (GitHub #431 = PR 2 of #353; spec
//! `AI/architect_spec_ir_active_cues.md`; ADR 2026-10-18 "Active-Illumination Differential
//! Liveness Cues").
//!
//! A near-infrared module lights the scene with its own emitter. Comparing a frame captured with
//! the emitter **on** (lit) with a frame of the same scene captured with it **off** (unlit)
//! isolates the scene's response to light that `soos` controls:
//!
//! 1. **Static pair**: two identical buffers, or the same capture twice, cannot be a lit/unlit
//!    pair (static injection, the CVE-2021-34466 class).
//! 2. **Response ratio**: the face is much closer to the emitter than the background, so its
//!    response `mean(lit - unlit)` must be strong and larger than the background's. A screen
//!    replay emits almost no near-infrared light and shows no response.
//! 3. **Saturation**: a saturated lit face carries no usable cue.
//! 4. **Skin NIR reflectance band**: the lit face luma must lie in a band (rejects many inks
//!    and papers that are much darker or brighter than skin under the emitter).
//! 5. **Inverse-square relief**: the nose and inner cheeks are closer to the emitter than the
//!    face edges and the eye corners, so their response is larger; a flat surface responds
//!    nearly uniformly.
//! 6. **Corneal glint**: each eye shows a small specular spot on the lit frame that is absent on
//!    the unlit frame.
//!
//! **Veto only.** A pass ([`IrActiveCuesClear`]) only means that no cue rejected the pair: it is
//! never a liveness decision and has no `is_live`. A consumer (strict IR mode, split item 4 of
//! #353) must AND it with the #169 IR gate, the stricter MiniFASNet threshold and its own checks.
//! Every malformed, non-finite, mismatched or degenerate input rejects.
//!
//! **Provisional and uncalibrated.** The default thresholds are sanity values chosen on
//! synthetic fixtures, not measurements; calibration on a real IR module is pending (matrix
//! IRA36, IRA37). No FAR, APCER or BPCER is claimed. A print **of an IR capture** carries the
//! captured shading in its reflectance and can pass the relief cue (PAI species 3 and 4 of #353).
//!
//! The module is zero I/O and never logs. The two face-crop copies are wiped on drop
//! (`Zeroizing`); the background ring is summed in place; results carry aggregates only and
//! rejection reasons carry no value.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "Bounded pixel statistics: every operand is derived from validated frame dimensions (at most 4096 x 4096) or finite, clamped coordinates"
)]

use std::fmt;

use soos_frame::{Frame, PixelFormat, SensorType};
use soos_inference_ort::{BoundingBox, FaceLandmarks, Point2f};
use zeroize::Zeroizing;

/// Maximum number of pixels of each frame of a pair (4096 x 4096).
pub const IR_ACTIVE_MAX_FRAME_PIXELS: usize = 4096 * 4096;

/// Maximum number of pixels of the face crop copied from each frame (1024 x 1024).
pub const IR_ACTIVE_MAX_CROP_PIXELS: usize = 1024 * 1024;

/// Minimum width and height, in pixels, of the face window inside the frame.
pub const IR_ACTIVE_MIN_FACE_SIDE_PX: usize = 16;

/// Minimum distance, in pixels, between the two eye landmarks.
pub const IR_ACTIVE_MIN_EYE_DISTANCE_PX: f32 = 8.0;

/// Minimum number of pixels of the background ring around the face window.
pub const IR_ACTIVE_MIN_BACKGROUND_PIXELS: usize = 64;

/// The background ring is the face window grown by this fraction of its side on each side
/// (clamped to the frame), minus the face window.
pub const IR_ACTIVE_BACKGROUND_MARGIN: f32 = 0.5;

/// Depth of the face-edge band, as a fraction of the face window side.
pub const IR_ACTIVE_EDGE_BAND_FRACTION: f32 = 0.15;

/// Half-side of the nose/inner-cheek square, as a fraction of the eye distance.
pub const IR_ACTIVE_RELIEF_HALF_SIDE: f32 = 0.25;

/// Offset of each eye-corner square from its eye, outward along the inter-eye axis, as a
/// fraction of the eye distance.
pub const IR_ACTIVE_EYE_CORNER_OFFSET: f32 = 0.30;

/// Half-side of each eye-corner square, as a fraction of the eye distance.
pub const IR_ACTIVE_EYE_CORNER_HALF_SIDE: f32 = 0.08;

/// Half-side of each eye region searched for a corneal glint, as a fraction of the eye distance
/// (at least 2 pixels).
pub const IR_ACTIVE_EYE_REGION_HALF_SIDE: f32 = 0.15;

/// Provisional, uncalibrated default: minimum face response `mean(lit - unlit)` (luma levels).
pub const IR_ACTIVE_MIN_FACE_RESPONSE: f32 = 20.0;

/// Provisional, uncalibrated default: minimum face response / background response ratio.
pub const IR_ACTIVE_MIN_RESPONSE_RATIO: f32 = 1.5;

/// Provisional, uncalibrated default: lit luma at or above which a face pixel is saturated.
pub const IR_ACTIVE_SATURATION_LUMA: u8 = 250;

/// Provisional, uncalibrated default: maximum fraction of saturated face pixels on the lit frame.
pub const IR_ACTIVE_MAX_SATURATED_FRACTION: f32 = 0.05;

/// Provisional, uncalibrated default: lower bound of the skin NIR reflectance band (mean lit
/// face luma).
pub const IR_ACTIVE_MIN_REFLECTANCE: f32 = 50.0;

/// Provisional, uncalibrated default: upper bound of the skin NIR reflectance band (mean lit
/// face luma).
pub const IR_ACTIVE_MAX_REFLECTANCE: f32 = 200.0;

/// Provisional, uncalibrated default: minimum nose response / periphery response ratio.
pub const IR_ACTIVE_MIN_RELIEF_RATIO: f32 = 1.2;

/// Provisional, uncalibrated default: lit luma at or above which an eye pixel can be a glint.
pub const IR_ACTIVE_GLINT_MIN_LUMA: u8 = 240;

/// Provisional, uncalibrated default: minimum `lit - unlit` of a glint pixel (absent unlit).
pub const IR_ACTIVE_GLINT_MIN_DELTA: u8 = 120;

/// Provisional, uncalibrated default: minimum glint pixels in each eye region.
pub const IR_ACTIVE_MIN_GLINT_PIXELS: u32 = 1;

/// Provisional, uncalibrated default: maximum fraction of an eye region that may be glint (a
/// larger bright patch is a reflection or a saturated area, not a corneal spot).
pub const IR_ACTIVE_MAX_GLINT_FRACTION: f32 = 0.25;

/// Upper bound accepted for [`IrActiveConfig::min_glint_pixels`].
pub const IR_ACTIVE_MAX_MIN_GLINT_PIXELS: u32 = 4096;

/// Thresholds of the active-illumination cues. **Provisional and uncalibrated**: the defaults
/// are the `IR_ACTIVE_*` constants above, chosen on synthetic fixtures.
///
/// [`IrActiveConfig::validate`] is applied by every evaluation (fail closed). Its lower bounds
/// mean that a valid configuration can never switch off the response, relief or glint cue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrActiveConfig {
    /// Minimum face response `mean(lit - unlit)`, in `(0, 255]`.
    pub min_face_response: f32,
    /// Minimum face / background response ratio, in `[1, 255]`.
    pub min_response_ratio: f32,
    /// Lit luma at or above which a pixel counts as saturated, at least 1.
    pub saturation_luma: u8,
    /// Maximum fraction of saturated face pixels on the lit frame, in `[0, 1]`.
    pub max_saturated_fraction: f32,
    /// Lower bound of the reflectance band (mean lit face luma), in `[0, 255)`.
    pub min_reflectance: f32,
    /// Upper bound of the reflectance band, in `(min_reflectance, 255]`.
    pub max_reflectance: f32,
    /// Minimum nose / periphery response ratio, in `[1, 255]`.
    pub min_relief_ratio: f32,
    /// Lit luma at or above which an eye pixel can be a glint, at least 1.
    pub glint_min_luma: u8,
    /// Minimum `lit - unlit` of a glint pixel, at least 1.
    pub glint_min_delta: u8,
    /// Minimum glint pixels in each eye region, in `1..=IR_ACTIVE_MAX_MIN_GLINT_PIXELS`.
    pub min_glint_pixels: u32,
    /// Maximum glint fraction of each eye region, in `(0, 1]`.
    pub max_glint_fraction: f32,
}

impl Default for IrActiveConfig {
    fn default() -> Self {
        Self {
            min_face_response: IR_ACTIVE_MIN_FACE_RESPONSE,
            min_response_ratio: IR_ACTIVE_MIN_RESPONSE_RATIO,
            saturation_luma: IR_ACTIVE_SATURATION_LUMA,
            max_saturated_fraction: IR_ACTIVE_MAX_SATURATED_FRACTION,
            min_reflectance: IR_ACTIVE_MIN_REFLECTANCE,
            max_reflectance: IR_ACTIVE_MAX_REFLECTANCE,
            min_relief_ratio: IR_ACTIVE_MIN_RELIEF_RATIO,
            glint_min_luma: IR_ACTIVE_GLINT_MIN_LUMA,
            glint_min_delta: IR_ACTIVE_GLINT_MIN_DELTA,
            min_glint_pixels: IR_ACTIVE_MIN_GLINT_PIXELS,
            max_glint_fraction: IR_ACTIVE_MAX_GLINT_FRACTION,
        }
    }
}

/// An [`IrActiveConfig`] field is non-finite or out of its documented range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrActiveConfigError {
    /// Name of the first offending field (never its value).
    pub field: &'static str,
}

impl fmt::Display for IrActiveConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "active-IR cue configuration field `{}` is non-finite or out of range",
            self.field
        )
    }
}

impl std::error::Error for IrActiveConfigError {}

fn finite_in(value: f32, low: f32, high: f32) -> bool {
    value.is_finite() && value >= low && value <= high
}

impl IrActiveConfig {
    /// Checks every field against its documented range; the first offending field is named.
    pub fn validate(&self) -> Result<(), IrActiveConfigError> {
        let fail = |field: &'static str| Err(IrActiveConfigError { field });
        if !finite_in(self.min_face_response, 0.0, 255.0) || self.min_face_response <= 0.0 {
            return fail("min_face_response");
        }
        if !finite_in(self.min_response_ratio, 1.0, 255.0) {
            return fail("min_response_ratio");
        }
        if self.saturation_luma == 0 {
            return fail("saturation_luma");
        }
        if !finite_in(self.max_saturated_fraction, 0.0, 1.0) {
            return fail("max_saturated_fraction");
        }
        if !finite_in(self.min_reflectance, 0.0, 255.0) {
            return fail("min_reflectance");
        }
        if !finite_in(self.max_reflectance, 0.0, 255.0) {
            return fail("max_reflectance");
        }
        if self.min_reflectance >= self.max_reflectance {
            return fail("min_reflectance");
        }
        if !finite_in(self.min_relief_ratio, 1.0, 255.0) {
            return fail("min_relief_ratio");
        }
        if self.glint_min_luma == 0 {
            return fail("glint_min_luma");
        }
        if self.glint_min_delta == 0 {
            return fail("glint_min_delta");
        }
        if self.min_glint_pixels == 0 || self.min_glint_pixels > IR_ACTIVE_MAX_MIN_GLINT_PIXELS {
            return fail("min_glint_pixels");
        }
        if !finite_in(self.max_glint_fraction, 0.0, 1.0) || self.max_glint_fraction <= 0.0 {
            return fail("max_glint_fraction");
        }
        Ok(())
    }
}

/// Why the active-illumination cues rejected a lit/unlit pair. Reasons carry no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrActiveRejection {
    /// The configuration failed [`IrActiveConfig::validate`].
    InvalidConfig,
    /// A frame is not `Grey` from an `Infrared` sensor, the geometries differ, a dimension is
    /// zero or too large, a buffer length is wrong, or a statistic is non-finite.
    InvalidFrames,
    /// The face box or a landmark is non-finite, outside the frame or the box, or too small, or
    /// a derived region is empty.
    InvalidGeometry,
    /// The face fills the frame: too few background pixels to compare against.
    InsufficientBackground,
    /// The two frames are byte-identical or the same capture (static injection).
    StaticPair,
    /// The face does not respond to the emitter, or not more than the background.
    NoResponse,
    /// Too many lit face pixels are saturated.
    Saturated,
    /// The lit face luma is outside the skin reflectance band.
    ReflectanceOutOfBand,
    /// The nose responds no more than the face edges and eye corners (flat surface).
    FlatRelief,
    /// An eye region shows no corneal glint, or a bright patch too large to be one.
    NoGlint,
}

impl fmt::Display for IrActiveRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::InvalidConfig => "active-IR cue configuration is invalid",
            Self::InvalidFrames => "lit/unlit frames are malformed or mismatched",
            Self::InvalidGeometry => "face box or landmarks are malformed",
            Self::InsufficientBackground => "too little background around the face",
            Self::StaticPair => "lit and unlit frames are identical (static pair)",
            Self::NoResponse => "face shows no response to the IR emitter",
            Self::Saturated => "lit face is saturated",
            Self::ReflectanceOutOfBand => "lit face reflectance is outside the skin band",
            Self::FlatRelief => "face response shows no relief (flat surface)",
            Self::NoGlint => "no corneal glint in an eye region",
        };
        f.write_str(text)
    }
}

/// Aggregate statistics of a lit/unlit pair (no pixel data is retained).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrActiveStatistics {
    /// Mean `lit - unlit` over the face window.
    pub face_response: f32,
    /// Mean `lit - unlit` over the background ring.
    pub background_response: f32,
    /// `face_response / max(background_response, 1)`.
    pub response_ratio: f32,
    /// Fraction of face pixels whose lit luma is at or above `saturation_luma`.
    pub saturated_fraction: f32,
    /// Mean lit luma over the face window.
    pub lit_face_mean: f32,
    /// Nose-square response / `max(periphery response, 1)` (periphery: face-edge band pooled
    /// with the two eye-corner squares).
    pub relief_ratio: f32,
    /// Glint pixels in the left and right eye regions.
    pub glint_pixels: [u32; 2],
    /// Pixels of the left and right eye regions.
    pub eye_region_pixels: [u32; 2],
}

/// Proof that no active-illumination cue vetoed a pair. **Not a liveness decision**: it can
/// only be built by [`evaluate_ir_active_cues`] and exposes the statistics, nothing else.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrActiveCuesClear {
    statistics: IrActiveStatistics,
}

impl IrActiveCuesClear {
    /// The statistics the cues were evaluated on.
    pub fn statistics(&self) -> IrActiveStatistics {
        self.statistics
    }
}

/// Half-open pixel window `[x0, x1) x [y0, y1)`.
#[derive(Debug, Clone, Copy)]
struct Window {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Window {
    fn width(&self) -> usize {
        self.x1.saturating_sub(self.x0)
    }

    fn height(&self) -> usize {
        self.y1.saturating_sub(self.y0)
    }

    fn pixels(&self) -> usize {
        self.width().saturating_mul(self.height())
    }
}

/// Sum of `lit - unlit` and number of pixels of a region.
#[derive(Debug, Clone, Copy, Default)]
struct RegionSum {
    sum: i64,
    count: usize,
}

impl RegionSum {
    fn add(self, other: RegionSum) -> RegionSum {
        RegionSum {
            sum: self.sum.saturating_add(other.sum),
            count: self.count.saturating_add(other.count),
        }
    }

    fn mean(self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some(self.sum as f64 / self.count as f64)
        }
    }
}

/// Validated input of one evaluation.
struct Prepared {
    width: usize,
    window: Window,
    background: RegionSum,
    /// Landmarks in face-window coordinates.
    left_eye: Point2f,
    right_eye: Point2f,
    nose: Point2f,
    eye_distance: f32,
}

/// Frame dimensions in pixels, if both frames are a well-formed `Grey` + `Infrared` pair.
fn validate_frames(lit: &Frame, unlit: &Frame) -> Option<(usize, usize)> {
    for frame in [lit, unlit] {
        if frame.format != PixelFormat::Grey || frame.sensor_type != SensorType::Infrared {
            return None;
        }
    }
    if lit.width != unlit.width || lit.height != unlit.height {
        return None;
    }
    if lit.width == 0 || lit.height == 0 {
        return None;
    }
    let width = usize::try_from(lit.width).ok()?;
    let height = usize::try_from(lit.height).ok()?;
    let pixels = width.checked_mul(height)?;
    if pixels > IR_ACTIVE_MAX_FRAME_PIXELS {
        return None;
    }
    if lit.data.len() != pixels || unlit.data.len() != pixels {
        return None;
    }
    Some((width, height))
}

fn finite_point(p: &Point2f) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// Integer face window of a finite, ordered box clamped to the frame.
fn face_window(face_box: &BoundingBox, width: usize, height: usize) -> Option<Window> {
    let BoundingBox { x1, y1, x2, y2 } = *face_box;
    if !(x1.is_finite() && y1.is_finite() && x2.is_finite() && y2.is_finite()) {
        return None;
    }
    if x1 >= x2 || y1 >= y2 {
        return None;
    }
    // Frame sides are at most 4096 * 4096 pixels in total, so they are exact in f32.
    let fx0 = x1.floor().clamp(0.0, width as f32);
    let fx1 = x2.ceil().clamp(0.0, width as f32);
    let fy0 = y1.floor().clamp(0.0, height as f32);
    let fy1 = y2.ceil().clamp(0.0, height as f32);
    let window = Window {
        x0: fx0 as usize,
        y0: fy0 as usize,
        x1: fx1 as usize,
        y1: fy1 as usize,
    };
    if window.x1 > width || window.y1 > height {
        return None;
    }
    if window.width() < IR_ACTIVE_MIN_FACE_SIDE_PX || window.height() < IR_ACTIVE_MIN_FACE_SIDE_PX {
        return None;
    }
    if window.pixels() > IR_ACTIVE_MAX_CROP_PIXELS {
        return None;
    }
    Some(window)
}

/// Converts a landmark to face-window coordinates if it lies inside the window.
fn local_point(p: &Point2f, window: &Window) -> Option<Point2f> {
    let (x0, y0) = (window.x0 as f32, window.y0 as f32);
    let (x1, y1) = (window.x1 as f32, window.y1 as f32);
    if p.x < x0 || p.x >= x1 || p.y < y0 || p.y >= y1 {
        return None;
    }
    Some(Point2f::new(p.x - x0, p.y - y0))
}

/// One frame row restricted to `[x0, x1)`.
fn row_slice(data: &[u8], width: usize, y: usize, x0: usize, x1: usize) -> Option<&[u8]> {
    let start = y.checked_mul(width)?.checked_add(x0)?;
    let end = y.checked_mul(width)?.checked_add(x1)?;
    data.get(start..end)
}

fn diff_sum(lit: &[u8], unlit: &[u8]) -> i64 {
    lit.iter()
        .zip(unlit)
        .map(|(&l, &u)| i64::from(l) - i64::from(u))
        .sum()
}

/// Sum of `lit - unlit` over the background ring around `window` (no allocation).
fn background_ring(
    lit: &[u8],
    unlit: &[u8],
    width: usize,
    height: usize,
    window: &Window,
) -> Option<RegionSum> {
    let margin_x = (window.width() as f32 * IR_ACTIVE_BACKGROUND_MARGIN).round() as usize;
    let margin_y = (window.height() as f32 * IR_ACTIVE_BACKGROUND_MARGIN).round() as usize;
    let outer = Window {
        x0: window.x0.saturating_sub(margin_x),
        y0: window.y0.saturating_sub(margin_y),
        x1: window.x1.saturating_add(margin_x).min(width),
        y1: window.y1.saturating_add(margin_y).min(height),
    };
    let mut total = RegionSum::default();
    for y in outer.y0..outer.y1 {
        let spans: [(usize, usize); 2] = if y >= window.y0 && y < window.y1 {
            [(outer.x0, window.x0), (window.x1, outer.x1)]
        } else {
            [(outer.x0, outer.x1), (0, 0)]
        };
        for (x0, x1) in spans {
            if x0 >= x1 {
                continue;
            }
            let l = row_slice(lit, width, y, x0, x1)?;
            let u = row_slice(unlit, width, y, x0, x1)?;
            total = total.add(RegionSum {
                sum: diff_sum(l, u),
                count: x1 - x0,
            });
        }
    }
    Some(total)
}

fn prepare(
    lit: &Frame,
    unlit: &Frame,
    face_box: &BoundingBox,
    landmarks: &FaceLandmarks,
    config: &IrActiveConfig,
) -> Result<Prepared, IrActiveRejection> {
    config
        .validate()
        .map_err(|_| IrActiveRejection::InvalidConfig)?;
    let (width, height) = validate_frames(lit, unlit).ok_or(IrActiveRejection::InvalidFrames)?;
    if !landmarks.as_array().iter().all(finite_point) {
        return Err(IrActiveRejection::InvalidGeometry);
    }
    let window = face_window(face_box, width, height).ok_or(IrActiveRejection::InvalidGeometry)?;
    let left_eye =
        local_point(&landmarks.left_eye, &window).ok_or(IrActiveRejection::InvalidGeometry)?;
    let right_eye =
        local_point(&landmarks.right_eye, &window).ok_or(IrActiveRejection::InvalidGeometry)?;
    let nose = local_point(&landmarks.nose, &window).ok_or(IrActiveRejection::InvalidGeometry)?;
    let eye_distance = left_eye.distance_to(&right_eye);
    if !eye_distance.is_finite() || eye_distance < IR_ACTIVE_MIN_EYE_DISTANCE_PX {
        return Err(IrActiveRejection::InvalidGeometry);
    }
    let background = background_ring(&lit.data, &unlit.data, width, height, &window)
        .ok_or(IrActiveRejection::InvalidFrames)?;
    if background.count < IR_ACTIVE_MIN_BACKGROUND_PIXELS {
        return Err(IrActiveRejection::InsufficientBackground);
    }
    Ok(Prepared {
        width,
        window,
        background,
        left_eye,
        right_eye,
        nose,
        eye_distance,
    })
}

/// Copies the face window of one frame (bounded by [`IR_ACTIVE_MAX_CROP_PIXELS`]).
fn copy_window(data: &[u8], width: usize, window: &Window) -> Option<Zeroizing<Vec<u8>>> {
    // Exact capacity: the buffer never reallocates, so no unwiped copy is left behind.
    let mut crop: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(window.pixels()));
    for y in window.y0..window.y1 {
        crop.extend_from_slice(row_slice(data, width, y, window.x0, window.x1)?);
    }
    Some(crop)
}

/// Inclusive square `[cx - half, cx + half]` clamped to a `w x h` crop, as a half-open window.
fn square(center: Point2f, half: usize, w: usize, h: usize) -> Option<Window> {
    // Centres are finite and inside (or, for eye corners, near) a window of at most 1024 x 1024
    // pixels, so the rounded values fit in i64.
    let cx = center.x.round() as i64;
    let cy = center.y.round() as i64;
    let half = i64::try_from(half).ok()?;
    let last_x = i64::try_from(w).ok()?.checked_sub(1)?;
    let last_y = i64::try_from(h).ok()?.checked_sub(1)?;
    let x0 = (cx - half).max(0);
    let y0 = (cy - half).max(0);
    let x1 = (cx + half).min(last_x);
    let y1 = (cy + half).min(last_y);
    if x0 > x1 || y0 > y1 {
        return None;
    }
    Some(Window {
        x0: x0 as usize,
        y0: y0 as usize,
        x1: x1 as usize + 1,
        y1: y1 as usize + 1,
    })
}

fn half_side(fraction: f32, eye_distance: f32, minimum: usize) -> usize {
    ((fraction * eye_distance).round() as usize).max(minimum)
}

/// Face-crop pixels of the lit and unlit frames (`w x h`, row-major).
struct Crop<'a> {
    lit: &'a [u8],
    unlit: &'a [u8],
    w: usize,
}

impl Crop<'_> {
    fn rows(&self, region: &Window) -> Option<Vec<(&[u8], &[u8])>> {
        (region.y0..region.y1)
            .map(|y| {
                Some((
                    row_slice(self.lit, self.w, y, region.x0, region.x1)?,
                    row_slice(self.unlit, self.w, y, region.x0, region.x1)?,
                ))
            })
            .collect()
    }

    fn region_sum(&self, region: &Window) -> Option<RegionSum> {
        let mut total = RegionSum::default();
        for (l, u) in self.rows(region)? {
            total = total.add(RegionSum {
                sum: diff_sum(l, u),
                count: l.len(),
            });
        }
        Some(total)
    }

    /// Glint pixels of a region: lit at or above `min_luma` and `lit - unlit >= min_delta`.
    fn glint_count(&self, region: &Window, min_luma: u8, min_delta: u8) -> Option<usize> {
        let mut count = 0usize;
        for (l, u) in self.rows(region)? {
            count += l
                .iter()
                .zip(u)
                .filter(|&(&lv, &uv)| {
                    lv >= min_luma && i32::from(lv) - i32::from(uv) >= i32::from(min_delta)
                })
                .count();
        }
        Some(count)
    }
}

fn to_f32(value: f64) -> Option<f32> {
    let v = value as f32;
    v.is_finite().then_some(v)
}

fn compute(
    lit: &Frame,
    unlit: &Frame,
    prepared: &Prepared,
    config: &IrActiveConfig,
) -> Result<IrActiveStatistics, IrActiveRejection> {
    let invalid_frames = IrActiveRejection::InvalidFrames;
    let invalid_geometry = IrActiveRejection::InvalidGeometry;
    let window = &prepared.window;
    let (w, h) = (window.width(), window.height());
    let lit_crop = copy_window(&lit.data, prepared.width, window).ok_or(invalid_frames)?;
    let unlit_crop = copy_window(&unlit.data, prepared.width, window).ok_or(invalid_frames)?;
    if lit_crop.len() != window.pixels() || unlit_crop.len() != window.pixels() {
        return Err(invalid_frames);
    }
    let crop = Crop {
        lit: &lit_crop,
        unlit: &unlit_crop,
        w,
    };

    // Face window: response, lit mean, saturation.
    let face = RegionSum {
        sum: diff_sum(&lit_crop, &unlit_crop),
        count: lit_crop.len(),
    };
    let lit_sum: u64 = lit_crop.iter().map(|&v| u64::from(v)).sum();
    let saturated = lit_crop
        .iter()
        .filter(|&&v| v >= config.saturation_luma)
        .count();
    let face_response = face.mean().ok_or(invalid_frames)?;
    let background_response = prepared.background.mean().ok_or(invalid_frames)?;
    let response_ratio = face_response / background_response.max(1.0);
    let n = face.count as f64;
    let lit_face_mean = lit_sum as f64 / n;
    let saturated_fraction = saturated as f64 / n;

    // Relief: nose square vs face-edge band pooled with the two eye-corner squares.
    let d = prepared.eye_distance;
    let nose_square = square(
        prepared.nose,
        half_side(IR_ACTIVE_RELIEF_HALF_SIDE, d, 1),
        w,
        h,
    )
    .ok_or(invalid_geometry)?;
    let center = crop.region_sum(&nose_square).ok_or(invalid_frames)?;
    let band_x = ((w as f32 * IR_ACTIVE_EDGE_BAND_FRACTION).round() as usize).max(1);
    let band_y = ((h as f32 * IR_ACTIVE_EDGE_BAND_FRACTION).round() as usize).max(1);
    // The minimum side (16) keeps the inner window non-empty: band < side / 2.
    let inner = Window {
        x0: band_x,
        y0: band_y,
        x1: w.saturating_sub(band_x),
        y1: h.saturating_sub(band_y),
    };
    if inner.x0 >= inner.x1 || inner.y0 >= inner.y1 {
        return Err(invalid_geometry);
    }
    let inner_sum = crop.region_sum(&inner).ok_or(invalid_frames)?;
    let edge = RegionSum {
        sum: face.sum.saturating_sub(inner_sum.sum),
        count: face.count.saturating_sub(inner_sum.count),
    };
    let (le, re) = (prepared.left_eye, prepared.right_eye);
    // Unit vector from the right eye towards the left eye (outward for the left eye).
    let ux = (le.x - re.x) / d;
    let uy = (le.y - re.y) / d;
    let offset = IR_ACTIVE_EYE_CORNER_OFFSET * d;
    let corner_half = half_side(IR_ACTIVE_EYE_CORNER_HALF_SIDE, d, 1);
    let left_corner = square(
        Point2f::new(le.x + ux * offset, le.y + uy * offset),
        corner_half,
        w,
        h,
    )
    .ok_or(invalid_geometry)?;
    let right_corner = square(
        Point2f::new(re.x - ux * offset, re.y - uy * offset),
        corner_half,
        w,
        h,
    )
    .ok_or(invalid_geometry)?;
    let periphery = edge
        .add(crop.region_sum(&left_corner).ok_or(invalid_frames)?)
        .add(crop.region_sum(&right_corner).ok_or(invalid_frames)?);
    let center_response = center.mean().ok_or(invalid_geometry)?;
    let periphery_response = periphery.mean().ok_or(invalid_geometry)?;
    let relief_ratio = center_response / periphery_response.max(1.0);

    // Corneal glint in each eye region.
    let eye_half = half_side(IR_ACTIVE_EYE_REGION_HALF_SIDE, d, 2);
    let mut glint_pixels = [0u32; 2];
    let mut eye_region_pixels = [0u32; 2];
    for (slot, eye) in glint_pixels
        .iter_mut()
        .zip(eye_region_pixels.iter_mut())
        .zip([le, re])
    {
        let region = square(eye, eye_half, w, h).ok_or(invalid_geometry)?;
        let count = crop
            .glint_count(&region, config.glint_min_luma, config.glint_min_delta)
            .ok_or(invalid_frames)?;
        *slot.0 = u32::try_from(count).map_err(|_| invalid_frames)?;
        *slot.1 = u32::try_from(region.pixels()).map_err(|_| invalid_frames)?;
    }

    Ok(IrActiveStatistics {
        face_response: to_f32(face_response).ok_or(invalid_frames)?,
        background_response: to_f32(background_response).ok_or(invalid_frames)?,
        response_ratio: to_f32(response_ratio).ok_or(invalid_frames)?,
        saturated_fraction: to_f32(saturated_fraction).ok_or(invalid_frames)?,
        lit_face_mean: to_f32(lit_face_mean).ok_or(invalid_frames)?,
        relief_ratio: to_f32(relief_ratio).ok_or(invalid_frames)?,
        glint_pixels,
        eye_region_pixels,
    })
}

/// Computes the statistics of a lit/unlit pair without deciding anything (calibration aid).
///
/// Applies the same fail-closed validation as [`evaluate_ir_active_cues`] (configuration,
/// frames, geometry, background), but not the static-pair cue nor any threshold. `config` is
/// needed for the saturation and glint pixel levels.
pub fn ir_active_statistics(
    lit: &Frame,
    unlit: &Frame,
    face_box: &BoundingBox,
    landmarks: &FaceLandmarks,
    config: &IrActiveConfig,
) -> Result<IrActiveStatistics, IrActiveRejection> {
    let prepared = prepare(lit, unlit, face_box, landmarks, config)?;
    compute(lit, unlit, &prepared, config)
}

/// Evaluates the active-illumination cues of a lit/unlit pair; the first rejecting cue is
/// reported (order: static pair, response, saturation, reflectance band, relief, glint).
///
/// `Ok` only means that **no cue vetoed**: it never grants liveness on its own and must be
/// combined (AND) with the other liveness checks by the caller.
pub fn evaluate_ir_active_cues(
    lit: &Frame,
    unlit: &Frame,
    face_box: &BoundingBox,
    landmarks: &FaceLandmarks,
    config: &IrActiveConfig,
) -> Result<IrActiveCuesClear, IrActiveRejection> {
    let prepared = prepare(lit, unlit, face_box, landmarks, config)?;
    if lit.sequence == unlit.sequence || lit.data == unlit.data {
        return Err(IrActiveRejection::StaticPair);
    }
    let s = compute(lit, unlit, &prepared, config)?;
    // `compute` returns finite statistics only, so plain comparisons cannot be fooled by NaN.
    if s.face_response < config.min_face_response || s.response_ratio < config.min_response_ratio {
        return Err(IrActiveRejection::NoResponse);
    }
    if s.saturated_fraction > config.max_saturated_fraction {
        return Err(IrActiveRejection::Saturated);
    }
    if s.lit_face_mean < config.min_reflectance || s.lit_face_mean > config.max_reflectance {
        return Err(IrActiveRejection::ReflectanceOutOfBand);
    }
    if s.relief_ratio < config.min_relief_ratio {
        return Err(IrActiveRejection::FlatRelief);
    }
    for (glints, region) in s.glint_pixels.iter().zip(s.eye_region_pixels.iter()) {
        if *region == 0 || *glints < config.min_glint_pixels {
            return Err(IrActiveRejection::NoGlint);
        }
        let fraction = *glints as f32 / *region as f32;
        if !fraction.is_finite() || fraction > config.max_glint_fraction {
            return Err(IrActiveRejection::NoGlint);
        }
    }
    Ok(IrActiveCuesClear { statistics: s })
}
