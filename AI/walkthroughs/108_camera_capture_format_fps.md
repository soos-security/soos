# Walkthrough 108 — V4L2 Capture Honours the Driver Format, Frame Rate and a Bounded DQBUF Wait

- **Date**: 2026-09-30
- **Issues**: GitHub #192 (CAM-10), #193 (CAM-11), #194 (CAM-12)
- **Branch**: `fix/p2-camera-capture-format-fps`
- **Matrix criteria**: CFP1–CFP7 (new)
- **ADR**: 2026-09-30 "V4L2 Capture Trusts the Driver Result, Not the Request"
- **Scope**: `soos-camera-v4l` (new `capture` module, `v4l_impl`, `config`, `mock`), docs.

---

## 1. Findings

| Issue | Finding | Consequence |
|---|---|---|
| #192 | Only `actual_format.width/height` were read after `VIDIOC_S_FMT`; the returned fourcc and `bytesperline` were ignored, frames were labelled with the requested format, `V4L2_BUF_FLAG_ERROR` was never checked and `Frame::new` accepted any size | Padded or substituted formats became silent per-frame drops in `soos-vision` (`InvalidBufferSize`); an error-flagged UVC frame reached detection |
| #193 | `fps` / `idle_fps` were never applied by `V4lCameraManager` (no `VIDIOC_S_PARM`); only the mock honoured them | The real camera ran at the driver default (for example 10 fps), raising `MAX_FRAME_AGE_NS` staleness; mock and production diverged |
| #194 | `stream_timeout_ms = 2000` under a comment promising 150-250 ms and Criterion C9 (< 500 ms `Drop`) | GUI close and daemon stop could hang for 2 s on a stalled camera; C9 was only measured on the mock |

## 2. Design (`crates/camera-v4l/src/capture.rs`)

Pure, hermetically testable functions:

- `validate_negotiated_format(requested, fourcc, width, height, bytesperline) -> NegotiatedFormat`:
  adopts a substituted fourcc only when its layout is known (strict mapping: `BGR3` and `RGB4`
  are never labelled `Rgb24`), rejects zero or odd-NV12 dimensions and a stride below one row;
  `bytesperline == 0` means tight rows.
- `validate_captured_buffer(buf, BufferMeta, &NegotiatedFormat) -> Vec<u8>`: rejects
  error-flagged buffers, `bytesused` beyond the mapping, short uncompressed buffers
  (`(rows - 1) * stride + row_bytes`) and empty MJPEG payloads, and de-strides padded rows. The
  returned payload always equals `PixelFormat::expected_buffer_size`. Nothing is copied for a
  rejected buffer.
- `requested_frame_interval(fps)`, `granted_fps(num, den)`: `VIDIOC_S_PARM` request and logging.
- `apply_frame_rate(path, fps, set_interval) -> FrameRateOutcome` (rework): the only path
  `open_and_stream` uses for `VIDIOC_S_PARM`; the ioctl is a closure, so accepted, adjusted and
  refused requests are tested hermetically and a refusal cannot become fatal.
- `CameraConfig::publish_fps(idle_elapsed)` and `publish_due(...)`: the `idle_fps` publication
  throttle, now shared by the mock (refactored to call it) and the V4L2 loop.
- `dqbuf_poll_timeout(fps)` (3 frame intervals, clamped 150-250 ms) and
  `max_consecutive_poll_timeouts(poll)` (stall budget `MAX_STREAM_STALL` = 2 s).

The streaming loop moved from `open_and_stream` to `run_capture_loop` over a crate-private
`CaptureSource` trait (`wait_ready`, `next_buffer`, `resync`), implemented by the V4L2 MMAP
stream and by a scripted test double.

### `v4l` 0.14 re-queue hazard

`Stream::next()` re-queues the previous buffer, then polls. When that poll times out, the
re-queued buffer is still owned by the driver, and a second `next()` would queue it twice
(`EINVAL`). The loop therefore:

1. polls the device handle itself (`Handle::poll`, bounded) before each `next()` once streaming
   started, so an ordinary stall never reaches `next()`;
2. after a timeout inside `next()` (slow first frame after `STREAMON`), calls
   `CaptureStream::dequeue` directly (`resync`) and discards that one buffer before `next()` is
   called again.

### Frame rate

`VIDIOC_S_PARM` with `1/fps` is sent after `VIDIOC_S_FMT` and before `REQBUFS`. A refusal is a
warning: drivers without frame-interval control keep streaming at their default rate. The poll
timeout is derived from the granted rate when the driver reports one. `idle_fps` is not
re-programmed into the hardware mid-stream: `uvcvideo` refuses `VIDIOC_S_PARM` while streaming
(`EBUSY`), so it would need a stream restart; it throttles publication instead, and the device is
released entirely at `idle_timeout` (auto-standby, unchanged).

## 3. Red Evidence

`crates/camera-v4l/tests/capture_validation_tests.rs` was written first. Before the
implementation it did not compile:

```
error[E0432]: unresolved import `soos_camera_v4l::capture`
error[E0599]: no method named `publish_fps` found for struct `CameraConfig` in the current scope (x4)
```

The unit tests of the loop (`capture::tests`) were written together with the `CaptureSource`
seam they need; they fail on any loop that re-calls `next()` after a timeout, publishes an
error-flagged frame, or blocks longer than one poll after a stop request.

## 4. Green Evidence

- `cargo test --locked -p soos-camera-v4l --all-features --all-targets`: all suites pass,
  including 25 `capture_validation_tests` and 11 `capture::tests` unit tests.
- Manual hardware evidence (not CI, not reproducible from the repository): the `#[ignore]`d
  `test_v4l_streaming_drop_completes_within_budget_on_hardware` (gate `SOOS_HW_TESTS=1`, as in
  `hardware_smoke_tests.rs`; `SOOS_HW_TESTS=1 cargo test -p soos-camera-v4l --all-features --test
  capture_validation_tests test_v4l_streaming_drop -- --ignored`) passed on an "Integrated
  Camera" (uvcvideo) on 2026-09-30; it asserts `Drop` latency only and is not evidence for
  `VIDIOC_S_PARM`.

### Rework after candid review

- CFP4 evidence: `VIDIOC_S_PARM` moved behind `capture::apply_frame_rate`; new hermetic tests
  `capture_validation_tests::test_apply_frame_rate_accepted_request`,
  `test_apply_frame_rate_adjusted_request`, `test_apply_frame_rate_unusable_granted_interval`,
  `test_apply_frame_rate_refusal_is_non_fatal` and `test_apply_frame_rate_zero_fps_requests_default`
  (red: unresolved import before the seam existed).
- The hardware drop test is `#[ignore]`d and gated on `SOOS_HW_TESTS=1` (ADR "Hermetic V4L2
  Enumeration" (3)) instead of silently passing in CI.
- `EINTR` from `next_buffer` now resyncs like a timeout instead of reopening the device
  (`capture::test_dequeue_interrupted_resyncs_instead_of_reopening`, red: `BufferDequeue`).

## 5. Security and Robustness Audit

- No `unwrap`/`expect`/indexing in production code; every size computation is checked
  (`checked_mul`, `checked_add`, `try_from`), and allocations are bounded by the mapped buffer.
- No new `unsafe`: `Handle::poll` and `CaptureStream::dequeue` are safe `v4l` APIs.
- Logs carry only the device path, formats, sizes and rates, never frame bytes.
- Rejected buffers are never copied; published payloads stay in the zeroizing `Frame`.
- Fail-closed: an unknown driver layout is a `SetFormat` error, corrupted frames are never
  published, and a sustained stall or corruption streak reopens the device.

## 6. Documentation

- `Docs/CAMERA_V4L_CRATE.md`: new "Capture-Path Validation" section; `fps`, `idle_fps` and
  `idle_timeout` descriptions corrected (the documented `auto_format` default `false` and
  `idle_timeout` default `60s` were drift: the code defaults are `true` and `10s`).
- `AI/VERIFICATION_MATRIX.md`: component `camera-capture-format-fps`, rows CFP1–CFP7.

## 7. Follow-ups

- Stride-padded and substituted-format drivers were only validated with synthetic buffers; a
  device that pads YUYV rows should be exercised on real hardware.
- `fourcc_to_pixel_format` (used for enumeration) still maps `BGR3` and `RGB4` to `Rgb24`; a
  device offering only those now fails with a `SetFormat` diagnostic instead of streaming wrong
  colours. Supporting them needs dedicated `PixelFormat` variants.
- GARP2 (`bytesused` slicing) remains `⬜ Pending` as written; CFP2 now covers that behaviour with
  automated tests and the row can be re-pointed in a traceability pass.
