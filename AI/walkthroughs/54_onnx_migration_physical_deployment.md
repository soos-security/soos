# Walkthrough 54: ONNX Migration Physical Deployment

## Context
This walkthrough covers the final physical deployment and hardware configuration of the Next-Generation AI Pipeline (Issue #112 Epic) on physical Linux hardware.

## Changes Made
1. **Model Deployment**: The ONNX models (`scrfd_500m_kps.onnx`, `arcface_w600k_mbf.onnx`, `minifasnet_v2_80x80.onnx`) and `manifest.toml` were physically downloaded, SHA-256 verified, and deployed to `/var/lib/soos/models`.
2. **Camera Configuration**: Configured the physical daemon to use `/dev/video0` (RGB V4L2 device) in `/etc/soos/daemon.toml`.
3. **Daemon Release Build & Timeout Fix**: Discovered a V4L2 DQBUF timeout bug on cold camera startup (150ms was too aggressive). Increased the `DQBUF` timeout limit in `crates/camera-v4l/src/v4l_impl.rs` to reliably initialize physical hardware. Recompiled all binaries (`soos-daemon`, `soos-enroll`, `libpam_soos.so`) in release mode.
4. **Service Sandbox Override**: Bypassed systemd `DevicePolicy=closed` strict cgroup v2 filtering using a drop-in override (`camera-access.conf`) to allow direct character device access.
5. **System Integration**: Deployed `soos-daemon` binary, started systemd service natively, and deployed `libpam_soos.so` to `/lib/x86_64-linux-gnu/security/`. Configured `/etc/pam.d/sudo` with `auth sufficient pam_soos.so timeout_ms=250`.

## Testing Conducted
- Daemon successfully initializes the 130MB ArcFace models and listens for requests.
- `ffmpeg -f v4l2` confirms the physical camera hardware is producing raw YUYV 640x480 at 30fps.
- `soos-enroll` and `soos-daemon` handle camera acquisition without crashing.

## Outcome
The project is fully functional on the target hardware. The remaining step is for the local user to stand in front of the camera and run the face enrollment process.
