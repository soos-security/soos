# Architect Spec — Active-Illumination On/Off Differential Liveness Cues

- **Date**: 2026-10-18
- **Issue**: GitHub #431 = PR 2 of epic #353 (split item 2), GitHub-only (no backlog id)
- **Branch**: `feat/ir-active-cues`
- **Base**: `origin/main` at `3f76090`
- **Builds on**: #378 / PR #393 (`soos-camera-v4l` emitter control, strobe classifier, `tests/fixtures` `ir_strobe`)
- **Owner decisions applied**: 2026-10-12 on #353 (the owner's PC has an RGB camera only; IR validation runs on
  another project member's IR PC; mocks and synthetic fixtures until then)

## 0. Goal

Add a zero-I/O module `crates/vision/src/ir_active.rs` that compares a **lit** frame (IR emitter on)
with an **unlit** frame (emitter off) of the same scene and computes four active-illumination cues on
the face crop. The result can only veto: `Ok` means "no active-illumination cue rejects", never "live".
No consumer exists in this PR (no daemon or policy change, no `ir_mode` key). Strict IR mode (split
item 4) will AND this result with the #169 IR gate, the stricter MiniFASNet threshold and the challenge
pattern. The open decision on `StrobeStatus::vetoes_allow(Stable(Absent))` belongs to split item 4 and
is not touched.

## 1. Constraints

- `soos-vision` keeps `#![forbid(unsafe_code)]`; no OpenCV, no new dependency, no new `soos-frame` type
  (the module takes `soos_frame::Frame` and the existing `soos_inference_ort::{BoundingBox, FaceLandmarks}`).
- Zero I/O: no `std::fs`, `std::net`, `std::process`, `std::env`, no logging macro.
- Bounded: the face crop copy is at most `IR_ACTIVE_MAX_CROP_PIXELS` (1024 × 1024) pixels per frame; the
  background ring is summed in place (no allocation); every frame must hold at most
  `IR_ACTIVE_MAX_FRAME_PIXELS` (4096 × 4096) pixels.
- Privacy: the two face-crop copies are `Zeroizing<Vec<u8>>`; results carry aggregates only; rejection
  reasons carry no value.
- Total: no `unwrap`, `expect`, `panic`, unchecked indexing or unchecked arithmetic on input-derived
  values; every non-finite intermediate rejects.

## 2. Public API (`soos_vision::ir_active`, re-exported from `lib.rs`)

```rust
pub const IR_ACTIVE_MAX_FRAME_PIXELS: usize = 4096 * 4096;
pub const IR_ACTIVE_MAX_CROP_PIXELS: usize = 1024 * 1024;
pub const IR_ACTIVE_MIN_FACE_SIDE_PX: usize = 16;
pub const IR_ACTIVE_MIN_EYE_DISTANCE_PX: f32 = 8.0;
pub const IR_ACTIVE_MIN_BACKGROUND_PIXELS: usize = 64;
pub const IR_ACTIVE_BACKGROUND_MARGIN: f32 = 0.5;   // ring = box grown by 50 % per side, minus the box
pub const IR_ACTIVE_EDGE_BAND_FRACTION: f32 = 0.15; // outer band of the face box
pub const IR_ACTIVE_RELIEF_HALF_SIDE: f32 = 0.25;   // × eye distance, square on the nose
pub const IR_ACTIVE_EYE_CORNER_OFFSET: f32 = 0.30;  // × eye distance, outward from each eye
pub const IR_ACTIVE_EYE_CORNER_HALF_SIDE: f32 = 0.08;
pub const IR_ACTIVE_EYE_REGION_HALF_SIDE: f32 = 0.15;
// Provisional, uncalibrated defaults of IrActiveConfig:
pub const IR_ACTIVE_MIN_FACE_RESPONSE: f32 = 20.0;
pub const IR_ACTIVE_MIN_RESPONSE_RATIO: f32 = 1.5;
pub const IR_ACTIVE_SATURATION_LUMA: u8 = 250;
pub const IR_ACTIVE_MAX_SATURATED_FRACTION: f32 = 0.05;
pub const IR_ACTIVE_MIN_REFLECTANCE: f32 = 50.0;
pub const IR_ACTIVE_MAX_REFLECTANCE: f32 = 200.0;
pub const IR_ACTIVE_MIN_RELIEF_RATIO: f32 = 1.2;
pub const IR_ACTIVE_GLINT_MIN_LUMA: u8 = 240;
pub const IR_ACTIVE_GLINT_MIN_DELTA: u8 = 120;
pub const IR_ACTIVE_MIN_GLINT_PIXELS: u32 = 1;
pub const IR_ACTIVE_MAX_GLINT_FRACTION: f32 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrActiveConfig { /* one pub field per default above */ }
impl Default for IrActiveConfig { /* the provisional constants */ }
impl IrActiveConfig { pub fn validate(&self) -> Result<(), IrActiveConfigError>; }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrActiveConfigError { pub field: &'static str }   // Display names the field only

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrActiveRejection {
    InvalidConfig, InvalidFrames, InvalidGeometry, InsufficientBackground,
    StaticPair, NoResponse, Saturated, ReflectanceOutOfBand, FlatRelief, NoGlint,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrActiveStatistics {
    pub face_response: f32,        // mean(lit - unlit) over the face box
    pub background_response: f32,  // mean(lit - unlit) over the background ring
    pub response_ratio: f32,       // face_response / max(background_response, 1)
    pub saturated_fraction: f32,   // lit >= saturation_luma over the face box
    pub lit_face_mean: f32,        // mean lit luma over the face box
    pub relief_ratio: f32,         // nose patch response / max(periphery response, 1)
    pub glint_pixels: [u32; 2],    // per eye region: lit >= glint_min_luma and lit - unlit >= glint_min_delta
    pub eye_region_pixels: [u32; 2],
}

/// Proof that no cue vetoed. It is not a liveness decision and has no `is_live`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrActiveCuesClear { /* private */ }
impl IrActiveCuesClear { pub fn statistics(&self) -> IrActiveStatistics; }

pub fn ir_active_statistics(lit: &Frame, unlit: &Frame, face_box: &BoundingBox,
    landmarks: &FaceLandmarks, config: &IrActiveConfig) -> Result<IrActiveStatistics, IrActiveRejection>;
pub fn evaluate_ir_active_cues(lit: &Frame, unlit: &Frame, face_box: &BoundingBox,
    landmarks: &FaceLandmarks, config: &IrActiveConfig) -> Result<IrActiveCuesClear, IrActiveRejection>;
```

`ir_active_statistics` needs the config for the pixel-classification levels (saturation and glint);
it is exposed for the calibration session (IRA36) and never decides.

## 3. Validation (fail closed, in this order)

1. `config.validate()` else `InvalidConfig`. Ranges: `min_face_response` in (0, 255];
   `min_response_ratio` and `min_relief_ratio` in [1, 255]; `saturation_luma`, `glint_min_luma`,
   `glint_min_delta` ≥ 1; `max_saturated_fraction` in [0, 1]; `0 ≤ min_reflectance < max_reflectance ≤ 255`;
   `min_glint_pixels` in 1..=4096; `max_glint_fraction` in (0, 1]; every float finite. The lower bounds
   mean a valid config can never switch off the response, relief or glint cue.
2. Frames (`InvalidFrames`): both `PixelFormat::Grey` and `SensorType::Infrared`, equal width and
   height, both non-zero, `width × height` ≤ `IR_ACTIVE_MAX_FRAME_PIXELS` (checked arithmetic), and
   `data.len() == width × height` for both.
3. Geometry (`InvalidGeometry`): box and the five landmarks finite; box clamped to the frame, integer
   window `[floor(x1), ceil(x2)) × [floor(y1), ceil(y2))`, each side ≥ `IR_ACTIVE_MIN_FACE_SIDE_PX`,
   area ≤ `IR_ACTIVE_MAX_CROP_PIXELS`; both eyes and the nose inside the window; eye distance finite and
   ≥ `IR_ACTIVE_MIN_EYE_DISTANCE_PX`; every derived patch non-empty after clamping to the window.
4. Background ring (box grown by `IR_ACTIVE_BACKGROUND_MARGIN` × side per side, clamped, minus the box)
   with fewer than `IR_ACTIVE_MIN_BACKGROUND_PIXELS` pixels ⇒ `InsufficientBackground`.

## 4. Cues (in this order; the first failure is reported)

| Order | Cue | Rejects when | Reason |
|---|---|---|---|
| 1 | Static pair | the two data buffers are byte-identical, or `lit.sequence == unlit.sequence` | `StaticPair` |
| 2 | Response ratio | `face_response < min_face_response` or `response_ratio < min_response_ratio` | `NoResponse` |
| 3 | Saturation | `saturated_fraction > max_saturated_fraction` | `Saturated` |
| 4 | Skin NIR reflectance band | `lit_face_mean` outside `[min_reflectance, max_reflectance]` | `ReflectanceOutOfBand` |
| 5 | Inverse-square relief | `relief_ratio < min_relief_ratio` | `FlatRelief` |
| 6 | Corneal glint | an eye region has fewer than `min_glint_pixels` glint pixels, or more than `max_glint_fraction` of its pixels | `NoGlint` |

Any non-finite statistic rejects as `InvalidFrames`. Regions (face-crop coordinates, eye distance `d`):
nose patch = square of half-side `round(0.25 d)` (≥ 1) on the nose; periphery = edge band of the face
box (`max(1, round(0.15 × side))` deep) pooled with two eye-corner squares of half-side `round(0.08 d)`
(≥ 1) at `eye ± 0.30 d` along the inter-eye axis, outward; eye regions = squares of half-side
`max(2, round(0.15 d))` on each eye. The difference is signed (`lit − unlit`): a swapped pair has a
negative response and is `NoResponse`.

## 5. Fixtures (`tests/fixtures` `ir_strobe`, synthetic and non-biometric)

`ActivePresentation::{ShadedHead, FlatPrint, Screen, NoGlint, Saturated, Unlit, StaticInjection}`,
`active_pair(p) -> ActivePair { lit, unlit, face_box: [f32; 4], landmarks: [[f32; 2]; 5] }` on a
160 × 120 `Grey` + `Infrared` frame, face box (50, 25)–(110, 105), landmarks eyes (68, 50) / (92, 50),
nose (80, 68), mouth (70, 85) / (90, 85). The shaded head is a Lambertian ellipsoid (`170 × sqrt(1 − r²)`
over a 20-level ambient), with 2 × 2 saturated glints on the eyes in the lit frame only, background
lit 40 / unlit 10. Expected reasons: flat print → `FlatRelief`, screen → `NoResponse`, no glint →
`NoGlint`, saturated → `Saturated`, unlit → `NoResponse`, static injection → `StaticPair`.

## 6. Known limits (documented, not hidden)

- A print **of an IR capture** carries the captured shading in its reflectance; its difference image can
  show relief and pass cue 5 (PAI species 3 and 4 of #353). The protocol of split item 5 measures it.
- A real-time adaptive USB device can reproduce lit and unlit frames; the challenge pattern of split
  item 4 addresses only passive replays.
- The defaults are not measurements (IRA36, IRA37 pending on an IR module); no FAR, APCER or BPCER is
  claimed.

## 7. Traceability

ADR "[2026-10-18] Active-Illumination Differential Liveness Cues" (amends the 2026-09-30 IR policy
ADR), `Docs/VISION_CRATE.md` §2.4.5 and module tree, matrix IRA29–IRA37 (IRA36, IRA37 ⬜ Pending), BACKLOG
section for #431, walkthrough 232, invariant module `ir_active_contract`.
