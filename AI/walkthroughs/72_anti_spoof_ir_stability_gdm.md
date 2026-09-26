# Walkthrough 72: Anti-Spoofing Class Alignment, Hardware IR Default, Latency Calibration, and Safe GDM Integration

## Objective

Resolve four interrelated security and system integration issues identified during physical laptop and desktop testing:
1. **Anti-Spoofing Screen Replay Bypass**: Fix vulnerability where phone screen replays were falsely identified as genuine live faces.
2. **Hardware Infrared (IR) Sensor Auto-Selection**: Ensure laptops with dual RGB/IR sensors (e.g., Asus ROG Zephyrus G14) automatically route to the IR camera node even when the V4L2 device name is truncated.
3. **Authentication Stability & Latency Calibration**: Eliminate intermittent broken pipes and cold-start timeouts when waking the camera from auto-standby.
4. **GDM Integration with Safe Disable Toggle**: Integrate facial verification into the GDM display manager (`/etc/pam.d/gdm-password`) with an immediate disable toggle to prevent any risk of user lockout during testing.

---

## Root Causes Identified

1. **MiniFASNet Class Mapping Inversion**:
   In MiniFASNet (`Silent-Face-Anti-Spoofing`), the model outputs 3 classes:
   - Index 0: `PrintPhoto`
   - Index 1: `Genuine Live`
   - Index 2: `ScreenReplay`
   The codebase had previously set `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX = 2`. When a phone screen or monitor replay was displayed, the model output high probability for Class 2 (Screen Replay). Because index 2 was treated as genuine live, the system assigned high live confidence to screen replays, enabling attacks.
2. **Distorted PAD Crop Aspect Ratio**:
   `expand_bbox_for_pad` in `crates/vision/src/crop.rs` expanded bounding boxes with hard edge clamping, which altered the aspect ratio when faces were close to image borders. MiniFASNet expects a centered face with context cropped according to the Minivision shifting algorithm (`_get_new_box`), which translates the crop box away from image boundaries to preserve the scale and aspect ratio.
3. **Dual-Sensor IR Priority and V4L2 Name Truncation**:
   Laptops with dual RGB/IR sensors presented `/dev/video0` (RGB) and `/dev/video2` (IR). The default sensor preference was `PreferRgb`, and the hardware card name for the IR sensor (`USB2.0 FHD UVC WebCam: USB2.0 I`) was truncated to 31 characters by the kernel V4L2 driver, stripping the terminal `R` in `IR`.
4. **Auto-Standby Decision Budget Starvation**:
   When the camera was suspended in auto-standby, waking required re-opening the device and streaming 20 warmup frames (~667ms at 30 FPS). However, `dispatcher.rs` only waited up to 600ms before returning `CameraUnavailable`, and `DECISION_BUDGET_MS` in `pipeline.rs` was 220ms. Total latency exceeded the 220ms budget on first attempt, causing intermittent timeouts and broken pipes.
5. **GDM Lockout Risk**:
   GDM runs as unprivileged user `gdm` during display manager authentication. Without an isolated killswitch or module-level disable flag, any integration flaw could leave the graphical login screen locked out.

---

## Architectural Changes & Key Implementations

### 1. `soos-inference-ort` (`crates/inference-ort/src/pad.rs`)
- Aligned `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` to `1`:
  ```rust
  pub const DEFAULT_MINIFASNET_LIVE_CLASS_INDEX: usize = 1;
  ```
- Mapped classes:
  - Index 0: `PrintPhoto`
  - Index 1: `Genuine Live`
  - Index 2: `ScreenReplay`
- Verified live faces score index 1 and screen replays score index 2, reliably returning `SpoofDetected`.

### 2. `soos-vision` (`crates/vision/src/crop.rs`)
- Implemented the Minivision `CropImage::_get_new_box` shifting algorithm:
  - Computes scale factor `(real_w / w + real_h / h) / 2.0`.
  - Determines new box dimensions: `w * scale` and `h * scale`.
  - Translates `(left, top)` if the box exceeds boundaries: shifts right when `left < 0`, shifts down when `top < 0`, shifts left when `right > img_w`, shifts up when `bottom > img_h`.
  - Preserves exact aspect ratio and context geometry for MiniFASNet.

### 3. `soos-camera-v4l` (`crates/camera-v4l/src/config.rs`, `sensor.rs`)
- Defaulted `SensorPreference` and `CameraConfig::default()` to `SensorPreference::PreferIr`.
- Added heuristic detection in `classify_sensor()`:
  - Recognizes 31-character truncated card names ending in `: USB2.0 I` or containing `WebCam: USB2.0 I` as Infrared.
- Filtered out empty or invalid format nodes in `enumerate_capture_devices()`.

### 4. `soos-daemon` (`crates/daemon/src/pipeline.rs`, `dispatcher.rs`)
- Increased `DECISION_BUDGET_MS` from 220ms to 900ms, staying strictly within the PAM 1000ms deadline while providing ample room for camera cold-start and warmup.
- Increased camera auto-standby wake timeout in `dispatcher.rs` from 600ms to 800ms.
- Retained Criterion C5 invariant: `CameraConfig::default().warmup_frames` remains 20 frames for physical sensor AGC/AEC convergence.

### 5. `soos-pam` (`crates/pam/src/config.rs`, `lib.rs`, `syslog.rs`)
- Added `PamConfig::is_disabled()` method checking:
  - `config.disabled == true` (passed as PAM argument `disabled`)
  - Global disable flag file: `/etc/soos/disabled`
  - Service-specific disable flag file: `/etc/soos/<service>.disable` (e.g. `/etc/soos/gdm.disable` when `service == "gdm-password"`)
- Immediately returns `PAM_IGNORE` if disabled, logging to syslog via `syslog::log_info`.

### 6. `soos-admin-cli` (`crates/admin-cli/src/args.rs`, `gdm.rs`, `main.rs`)
- Added `soos-admin gdm` subcommand:
  - `soos-admin gdm status`: inspects `/etc/pam.d/gdm-password` and `/etc/soos/gdm.disable`, reporting `Configured & Enabled`, `Configured & Disabled`, or `Not Configured`.
  - `soos-admin gdm enable`: removes `/etc/soos/gdm.disable` and ensures `/etc/pam.d/gdm-password` contains `auth sufficient pam_soos.so timeout_ms=1000`.
  - `soos-admin gdm disable`: atomically touches `/etc/soos/gdm.disable`, immediately disabling PAM verification in GDM without editing PAM files.

---

## Verification & Testing Summary

1. **Unit & Integration Test Suite**:
   - `crates/inference-ort/tests/pad_tests.rs`: verified `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX == 1` and screen replay rejection.
   - `crates/vision/tests/pad_tests.rs`: verified aspect-ratio preserving bounding box translation.
   - `crates/camera-v4l/tests/dual_sensor_tests.rs`: verified `SensorPreference::PreferIr` default and 31-char truncated device classification.
   - `crates/daemon/tests/config_tests.rs`: verified 900ms decision budget and IR camera config.
   - `crates/pam/tests/config_tests.rs`: verified `is_disabled()` handling of arguments and flag files.
   - `crates/admin-cli/tests/gdm_tests.rs`: verified full `status`, `enable`, `disable` command lifecycle.
2. **Quality Gates**:
   - `cargo fmt --all -- --check`: PASS.
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`: PASS with 0 warnings.
   - `cargo test --workspace`: 100% PASS across all crates and invariant tests.
   - `./scripts/candid_review.sh`: PASSED with all 7 architectural invariant audits green.
