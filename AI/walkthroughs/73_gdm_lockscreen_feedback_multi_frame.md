# Walkthrough 73: GDM Lock Screen Feedback, Multi-Frame Evaluation & Warmup

## Overview
This walkthrough covers the implementation of Issue #48 (GitHub Issue #136): resolving lock screen / GDM unlock reliability issues by providing real-time user feedback on the lock screen, extending the lock screen evaluation budget, supporting camera warmup frame configuration, enabling explicit `camera_device = "auto"` sentinel resolution, and transitioning the daemon authentication pipeline to a resilient multi-frame evaluation loop.

---

## Key Problem & Root Cause Analysis

1. **Host Configuration Overriding Auto-Selection**:
   In `/etc/soos/daemon.toml`, the configuration key `camera_device = "/dev/video0"` was explicitly setting the RGB camera, completely bypassing auto-selection of the infrared camera node `/dev/video2` (`USB2.0 FHD UVC WebCam: USB2.0 I`). Adding support for `camera_device = "auto"` in `daemon.toml` allows users and defaults to explicitly opt into automated camera resolution with `prefer_ir`.
2. **Single-Frame Premature Abort in Dispatcher**:
   The dispatcher previously evaluated only a single frame. When the user looked at the screen while the camera auto-exposure was settling or their face was slightly turned, `VisionError::NoFaceDetected` or a suboptimal score resulted in an immediate `Verdict::Deny`, terminating authentication before sufficient quality frames could be captured.
3. **Lock Screen Silence & Inadequate Timeout**:
   Lock screens such as GDM had no visual feedback indicating `soos` was attempting face verification. Users were unaware if the camera was active, if a face was detected, or if authentication failed. Furthermore, GDM defaults needed a sufficient timeout (`2500ms`) to account for sensor auto-standby wake and multi-frame processing.

---

## Architectural Changes & Implementation Details

### 1. `soos-pam`: Real-Time User Feedback via PAM Conversation
- Extended `authenticate()` in `crates/pam/src/ipc.rs` to return `Result<(Verdict, ReasonClass), IpcError>`, preserving detailed reason information for user feedback.
- Implemented `send_pam_info` in `crates/pam/src/lib.rs` using `pam-bindings`' `Conv::send(PAM_TEXT_INFO, ...)` with a low-address pointer heuristic (`addr >= 0x10000`). *(Corrected by walkthrough 122, GitHub #220: the heuristic only tolerated the dummy handles of the tests; `send_pam_info` was replaced by the `PamFeedback` trait and the guard is confined to its `PamHandle` adapter.)* *(Further correction, 2026-09-30, commit `4f15282`: the guard was then removed entirely; the `PamHandle` adapter trusts the handle given by libpam and every test uses a real `pam_start` handle, matrix PHS4.)*
- Emits prompt status updates on the lock screen:
  - Initial: `[soos] Looking for face...`
  - Success: `[soos] Face recognized. Unlocking...`
  - Rejection / Timeout: `[soos] Face not recognized.`, `[soos] Biometric spoof detected.`, `[soos] Face verification timed out.`, `[soos] Camera unavailable.`
  - Fail-safe fallback to standard PAM password prompts is strictly preserved.

### 2. `soos-admin-cli`: GDM 2500ms Deadline Configuration
- Updated `GDM_PAM_LINE` in `crates/admin-cli/src/gdm.rs` to include `timeout_ms=2500`.
- Ensured `soos-admin gdm enable` configures `/etc/pam.d/gdm-password` with `timeout_ms=2500`, giving the daemon sufficient headroom for sensor wake and multi-frame evaluation.

### 3. `soos-daemon`: Configurable Warmup & `camera_device = "auto"`
- Added `warmup_frames` to `PipelineConfigFile` in `crates/daemon/src/config.rs`, defaulting to 0 for instant capture while allowing host overrides.
- Supported `camera_device = "auto"` (and `"default"`), leaving `device_path` as the sentinel `/dev/v4l/by-id/default-camera` so that `select_camera_device` automatically picks the IR camera.

### 4. `soos-daemon`: Multi-Frame Evaluation Loop & Dynamic Budget Clamping
- Replaced the single-frame evaluation in `crates/daemon/src/dispatcher.rs` with a bounded multi-frame evaluation loop.
- Dynamic decision budget:
  - Uses client deadline monotonic nanoseconds (`req.deadline_monotonic_ns`), ignoring sentinel `u64::MAX`.
  - Strictly caps the evaluation budget by `self.config.connection_timeout.saturating_sub(50ms)` to ensure complete response transmission before socket timeouts trigger.
  - Loops across incoming frames, skipping duplicate or stale frames, until either `Verdict::Allow` is achieved or the budget expires.
  - Automatically recovers from initial `NoFaceDetected` or intermediate blurry captures.

---

## Contractual Tests & Verification

- **GDM Timeout**: `crates/admin-cli/tests/gdm_tests.rs::test_gdm_pam_line_includes_timeout_ms_2500`.
- **Configuration Parsing**: `crates/daemon/tests/config_tests.rs::test_pipeline_config_warmup_frames_from_toml` and `test_pipeline_config_camera_device_auto_resolution`.
- **PAM Conversation Pointer Safety**: *(corrected by walkthrough 122, GitHub #220: the two tests originally cited here never existed)* the conversation with no handle is exercised by `crates/pam/tests/config_tests.rs::test_authenticate_with_none_handle_returns_ignore_cleanly`; the real conversation by `crates/pam/tests/pam_handle_tests.rs` and `crates/pam/tests/pam_silent_tests.rs` (real `pam_start` handle) and the message selection by `crates/pam/tests/pam_feedback_tests.rs` (recorder).
- **Multi-Frame Evaluation Recovery**: `crates/daemon/tests/pipeline_integration_tests.rs::test_48_multi_frame_evaluation_recovers_from_initial_no_face_to_allow` (verifying empty capture followed by face detection successfully yields `Verdict::Allow`).
- **All Workspace Tests**: Ran `cargo test --workspace` across all crates (100% passing).
- **Candid Review**: Ran `./scripts/candid_review.sh` confirming zero unsafe additions, zero unwrap/expect in PAM, and strict English-only policy compliance.
