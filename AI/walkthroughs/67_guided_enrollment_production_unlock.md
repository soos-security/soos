# Walkthrough 67: Guided Enrollment Production Unlock & Camera Arbitration

## Objective
Fix the gap where guided face enrollment worked only in theory (saving to unprivileged `/tmp` storage while the background `soos-daemon` only reads `/var/lib/soos/biometrics`), eliminate camera flapping between RGB (color) and Infrared (black & white), implement seamless camera hardware arbitration between daemon and GUI, and enable physical PAM unlocking with the user's enrolled biometric face template.

---

## Root Causes Identified
1. **Camera Device Flapping (RGB vs IR)**:
   - On dual-sensor laptops (such as the ASUS ROG Zephyrus G14), `/dev/video0` is the RGB color webcam while `/dev/video2` is the greyscale Infrared sensor.
   - Udev created `/dev/v4l/by-id/usb-...-video-index0` pointing to `../../video2` (the IR camera).
   - `soos-daemon` explicitly configured `camera_device = "/dev/video0"` in `/etc/soos/daemon.toml`.
   - `soos-gui` and `soos-enroll` did not inspect `/etc/soos/daemon.toml`, causing them to resolve to `/dev/video2` (IR sensor) unless explicitly overridden on the command line.
   - Furthermore, Linux V4L2 requires exclusive access for video streaming (`VIDIOC_REQBUFS`). When `soos-daemon` was running in the background holding `/dev/video0`, opening `/dev/video0` returned `EBUSY`, crashing the GUI or forcing it to fallback to the secondary sensor.
2. **Template Not Saved to System Store**:
   - `/var/lib/soos/biometrics` is restricted to mode `0700` (`root:soos`), and `/var/lib/soos/master.key` is mode `0600` (`root:soos`).
   - When `soos-gui` is run by an unprivileged desktop user, it silently caught the permission error and fell back to `/tmp/soos-gui-biometrics` and `/tmp/soos-gui-master.key`.
   - Templates saved during guided enrollment never reached `/var/lib/soos/biometrics`, leaving `soos-daemon` unable to find any enrolled template for PAM authentication.

---

## Architectural Changes & Key Implementations

### 1. `soos-camera-v4l`
- Added `enumerate_capture_devices() -> Vec<CameraDeviceInfo>` to query and classify all available video capture devices by format and capabilities.
- Added `SensorType` (`Rgb`, `Infrared`, `Unknown`) and `SensorPreference` (`PreferRgb`, `PreferIr`, `Any`).

### 2. `soos-enrollment-cli`
- Added `soos-enroll import` subcommand with arguments:
  - `--uid <UID>`: Target user ID
  - `--username <NAME>`: Target username
  - `--file <PATH>`: Input embedding file (supporting JSON float array or canonical CBOR template)
  - `--model-id <ID>` (default: `arcface_w600k_mbf`)
  - `--model-version <VER>` (default: `2.0.0`)
- Added `resolve_camera_device_from_config(cli_device, config_path)`:
  - Prioritizes explicit CLI arguments (`--camera-device`).
  - Next reads `camera_device` from `/etc/soos/daemon.toml`.
  - Next falls back to deterministic `/dev/v4l/by-id/` per Criterion C4.
- Added contractual tests in `tests/import_tests.rs` validating template import, dimension enforcement (512D ArcFace), error handling, and config resolution.

### 3. `soos-gui`
- Updated camera initialization to use `resolve_camera_device_from_config`, ensuring both daemon and GUI consistently stream from `/dev/video0`.
- Integrated automated hardware arbitration:
  - If `/dev/video0` is busy on launch because `soos-daemon` is streaming, the GUI detects the active daemon and prompts the user via Polkit (`pkexec systemctl stop soos-daemon.service`) to release the camera.
  - Added live daemon status indicator in the top navigation header (`● Daemon Active` / `○ Daemon Paused`) along with `Pause` and `Resume` controls.
- Production Template Saving:
  - When saving in Guided Enrollment, unprivileged `soos-gui` writes a temporary JSON vector, invokes `pkexec soos-enroll import --uid <uid> --file <tmp>`, and securely shreds the temporary file.
  - The template is encrypted with `/var/lib/soos/master.key` and stored directly into `/var/lib/soos/biometrics/<uid>.bio`.
  - Similarly, deleting a profile in the GUI invokes `pkexec soos-enroll delete --uid <uid> --yes` to shred the production template.

---

## Verification & Test Results
- **Unit & Integration Tests**:
  - `cargo test -p soos-enrollment-cli --test import_tests`: 4 passed, 0 failed.
  - `cargo test -p soos-enrollment-cli`: 38 passed, 0 failed.
  - `cargo test -p soos-camera-v4l`: 27 passed, 0 failed.
  - `cargo test --workspace`: all 100+ tests passed cleanly.
- **Code Quality**:
  - `cargo fmt --all -- --check`: passed with zero diffs.
  - `cargo clippy --workspace --all-targets -- -D warnings`: passed with zero warnings.
- **Invariants**:
  - Zero `unwrap()` or `expect()` in production PAM / daemon pathways.
  - English-only deliverables policy strictly maintained.
