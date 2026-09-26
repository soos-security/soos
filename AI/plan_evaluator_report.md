# Plan Evaluator Report: Issue #46 — Biometric Reliability, Camera Power Lifecycle, IR Prioritization, and Daemon-Proxied GUI Preview

- **Date**: 2026-09-26
- **Target Issue**: Issue #46 (Backlog #46, GitHub #132)
- **Target Branch**: `feat/biometric-reliability-and-camera-lifecycle`
- **Evaluator**: Plan Evaluator Sub-Agent (Dev-Workflow Phase 1.5)

---

## 1. Executive Summary

The proposed implementation plan addresses four physical hardware integration findings:
1. **Biometric Reliability & Separability**: Fixes ArcFace BGR channel layout mapping in `soos-inference-ort` and synchronizes default verification thresholds across the workspace to `0.70` (matching `soos_policy::ThresholdConfig::DEFAULT_MATCH_THRESHOLD`) and PAD to `0.85`.
2. **Infrared Sensor Prioritization**: Exposes `sensor_preference = "prefer_ir"` in daemon configuration, integrates `select_camera_device(&candidates, SensorPreference::PreferIr)`, and ensures clean `PixelFormat::Grey` pipeline ingestion.
3. **Camera Power Management & Auto-Standby**: Implements a 3-state worker lifecycle (`Active` -> `Idle` -> `Suspended`) in `soos-camera-v4l`, releasing the V4L2 device file descriptor and extinguishing the hardware privacy LED after 10s of inactivity. Accommodates cold-start wakeups with an updated 1000ms default PAM execution timeout budget.
4. **Daemon Video Proxy for GUI**: Eliminates the V4L2 device lock conflict (`EBUSY`) and removes Polkit `systemctl stop soos-daemon` calls in `soos-gui`. Extends `soos-protocol` with `RequestKind::PreviewFrame`, `PreviewResponse`, and bounded preview codecs up to 2 MiB, allowing `soos-gui` to stream preview frames directly from `soos-daemon` without stopping the service.

The plan has been rigorously audited against `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/BACKLOG.md`, and `AI/VERIFICATION_MATRIX.md`.

---

## 2. Six-Pillar Architectural Audit

### Pillar 1: Architectural Alignment & Threat Model
- **Boundary Preservation**: `soos-daemon` retains exclusive root ownership of `/dev/video*` and the Unix domain socket `/run/soos/daemon.sock` (`0660`, `root:soos`).
- **Unprivileged GUI Streaming**: `soos-gui` no longer attempts to acquire exclusive V4L2 locks or execute Polkit commands (`pkexec systemctl stop soos-daemon`). It acts as a standard IPC client over `/run/soos/daemon.sock`.
- **SO_PEERCRED & Session Checks**: Peer verification and active session checks remain strictly enforced. `RequestKind::PreviewFrame` provides frame snapshots for diagnostic rendering without bypassing authentication policy.

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Zero Async in PAM**: `crates/pam` continues to rely strictly on blocking `std::os::unix::net::UnixStream` with synchronous polling.
- **Latency Budget Accommodation**: Raising `DEFAULT_TIMEOUT_MS` from 250ms to 1000ms provides sufficient headroom for camera cold-start initialization and auto-exposure convergence from `Suspended` state.
- **Dispatcher Wakeup**: When an authentication request arrives while the camera is suspended, the dispatcher triggers `notify_activity()` and awaits readiness with a bounded deadline (< 600ms) before rendering verdicts.

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Zero Panics in Production**: Interfaces avoid `unwrap()` / `expect()`.
- **Fallback Integrity**: Any failure during cold start, format negotiation, or socket communication systematically degrades to `Verdict::Unavailable` or `PAM_IGNORE`.
- **Bound Checking**: Frame dimension and buffer length validation in `convert_to_rgb`, `crop_and_resize`, and `expand_bbox_for_pad` fail safely if dimensions are zero or invalid.

### Pillar 4: Dependency Isolation & Banned Crates
- **Banned Crates**: Absolute prohibition against `opencv` and `nokhwa` is strictly maintained.
- **Language & Safety**: `#![forbid(unsafe_code)]` remains strictly enforced in `protocol`, `vision`, and `policy`.
- **Adapter Isolation**: `unsafe` remains confined to `v4l` MMAP handling and POSIX `clock_gettime`.

### Pillar 5: Data Confidentiality & Zeroization
- **Strict Size Boundaries**: PAM authentication messages remain strictly bounded by `MAX_MESSAGE_SIZE` (4,096 bytes).
- **Preview Size Ceiling**: `MAX_PREVIEW_MESSAGE_SIZE` is capped at 2 MiB, preventing memory exhaustion while comfortably accommodating downscaled or 640x480 video frames.
- **Zero Wire Secrets**: Passwords and biometric templates are strictly excluded from IPC messages.

### Pillar 6: Test Integrity & TDD Contracts
- **Red Phase Precondition**: Unit and integration tests for all 4 sub-issues will be written and verified failing before modifying production code.
- **Contractual Immutability**: Existing test contracts are preserved with zero test weakening.
- **Coverage**: Covers ArcFace BGR channel order, threshold synchronization, camera auto-standby, IR device selection, and IPC preview request/response handling.

---

## 3. Evaluation Verdict

**VALIDATION_VERDICT: APPROVED**

The implementation plan satisfies all zero-trust architectural invariants and quality criteria. The dev-workflow orchestrator may proceed directly to Phase 2 (Tester Sub-Agent).
