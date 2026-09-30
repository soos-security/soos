# Walkthrough 109 — Sensor Classification Hints, Deep-Greyscale IR Formats and Preview Downscaling

- **Date**: 2026-09-30
- **Issues**: GitHub #195 (CAM-13, IR/RGB classification relies on truncated names and format
  heuristics), GitHub #196 (CAM-14, preview frames above 2 MiB make the daemon drop the connection)
- **Branch**: `fix/p2-camera-classification-preview-size`
- **Matrix criteria**: CCP1–CCP5 (new)
- **ADRs**: "Sensor Classification Hints and Deep-Greyscale IR Capture", "Preview Frames Are
  Downscaled Daemon-Side" (`AI/DECISIONS.md`, 2026-09-30)

---

## 1. Context

**CAM-13.** `classify_sensor` looked only at the V4L2 card name (capped at 31 bytes) and at the
format list: "any colour fourcc means RGB". On the development host both nodes report the same
name, `Integrated Camera: Integrated C`. An IR module that also advertises YUYV was classified RGB,
and `PreferIr` then silently selected the RGB node. `fourcc_to_pixel_format` mapped only
`GREY`/`Y800`/`Y8`, so an IR node exposing only `Y8I`, `Y10`, `Y12` or `Y16` had no supported
format and was dropped by `enumerate_capture_devices`.

**CAM-14.** `encode_preview` returns `CodecError::MessageTooLarge` above
`MAX_PREVIEW_MESSAGE_SIZE` (2 MiB). The dispatcher propagated it with `?`, which closed the
connection. A 1920x1080 YUYV frame (4,147,200 bytes) therefore made every preview request fail,
and the GUI reconnected every 100 ms while the daemon logged an error each time.

## 2. Specification

`soos-camera-v4l`:

- `SensorHints { by_id_name: Option<String>, frame_sizes: Vec<(u32, u32)> }` and the ordered
  scorer `classify_sensor_with_hints` (by-id `IR` token > card-name marker > greyscale-only >
  frame-size signature `is_ir_frame_size_signature` (every size at most 640x400) > colour means
  RGB > Unknown). `classify_sensor` is the scorer with empty hints (same results as before).
- `CameraDeviceInfo::sensor_type_with_hints`, `select_camera_device_with(devices, preference,
  classify)` (same fallback order as `select_camera_device`).
- `CameraEnumerator::frame_sizes` (provided method, default: none); `SystemCameraEnumerator`
  enumerates `VIDIOC_ENUM_FRAMESIZES` (bounded by `MAX_FRAME_SIZE_HINTS` = 64; a stepwise range
  contributes its maximum only). `resolve_camera_device` classifies every node once with its
  by-id alias name and frame sizes.
- `plan_capture_with_hints`; the capture supervisor builds the same hints for the opened node
  (by-id name of `device_path`, frame sizes of the open device).
- `deep_grey.rs`: `DeepGreyFormat { Y8i, Y10, Y12, Y16 }`, `delivered_formats` (deep formats
  count as `Grey`), `select_wire_format` / `CaptureWireFormat` (deep format streamed only when
  `Grey` is negotiated and no native 8-bit greyscale fourcc exists; priority `Y16 > Y12 > Y10 >
  Y8I`), `DeepGreyFormat::to_grey8(data, width, height, stride)`; `capture_device_from_probe`
  is the pure part of enumeration.

`soos-daemon`:

- `preview_image.rs` (re-exported from `preview`): `MAX_PREVIEW_WIDTH` = 640,
  `PREVIEW_FORMAT_EMPTY` = 255, `MAX_PREVIEW_PIXEL_BYTES` = `MAX_PREVIEW_MESSAGE_SIZE - 1024`,
  `PreviewImage` (zeroized on drop), `preview_image_for_frame(&Frame)`.
- The dispatcher builds the `PreviewResponse` from `preview_image_for_frame` and maps a
  `MessageTooLarge` from `encode_preview` to the empty preview.

Why no new `CameraDeviceInfo` fields or `PixelFormat` variants: existing contract tests build
`CameraDeviceInfo` with struct literals (new fields would not compile without editing them), and a
new `PixelFormat` variant would reach vision, PAD policy, evidence and the preview wire format.
Normalising deep greyscale to `Grey` in the capture thread keeps a single greyscale format.

## 3. Tests first (red evidence)

New files: `crates/camera-v4l/tests/sensor_hint_classification_tests.rs` (17 tests) and
`crates/daemon/tests/preview_downscale_tests.rs` (9 tests).

1. Before any production change, both files failed to compile (unresolved
   `classify_sensor_with_hints`, `SensorHints`, `capture_device_from_probe`, `delivered_formats`,
   `select_wire_format`, `DeepGreyFormat`, `plan_capture_with_hints`, missing trait method
   `CameraEnumerator::frame_sizes`, missing `sensor_type_with_hints`; unresolved
   `preview_image_for_frame`, `MAX_PREVIEW_WIDTH`, `MAX_PREVIEW_PIXEL_BYTES`,
   `PREVIEW_FORMAT_EMPTY`).
2. With `preview_image.rs` added but the dispatcher unchanged, the two end-to-end tests failed
   at runtime: `test_preview_1080p_yuyv_frame_is_downscaled_not_disconnect` and
   `test_preview_oversize_frame_returns_empty_response_not_disconnect` panicked at
   `The daemon must answer instead of closing the connection` (the daemon closed the socket on
   the first request). The six pure conversion tests passed at that point.

## 4. Implementation

- `sensor.rs`: scorer, hints, `select_camera_device_with`, `capture_device_from_probe`,
  bounded `device_frame_sizes` / `frame_sizes_at`; `enumerate_capture_devices` uses the probe
  function, so deep-greyscale nodes are kept.
- `resolver.rs`: `frame_sizes` provided method, `alias_for`, `sensor_hints_for`,
  `by_id_name_of_path`; one classification pass per node, then `select_camera_device_with`.
- `v4l_impl.rs`: `plan_capture` delegates to `plan_capture_with_hints`; `open_and_stream`
  enumerates fourccs once, builds the hints, requests the wire fourcc and normalises deep-grey
  buffers (a truncated buffer is dropped at debug level, never published).
- `preview_image.rs`: pass-through for frames at most 640 px wide (or compressed MJPEG) within
  the budget; otherwise the decimation step is `ceil(width / 640)`, increased until the output
  fits the budget; YUYV/NV12 are sampled directly (same fixed-point coefficients as
  `soos_vision`), MJPEG is decoded with the bounded `soos_vision::convert_to_rgb`; every read
  uses `get` and checked arithmetic, and intermediate buffers are `Zeroizing`.
- `dispatcher.rs`: uses `preview_image_for_frame`, `PREVIEW_FORMAT_EMPTY`, and the
  `MessageTooLarge` fallback.

## 5. Audit

- No `unwrap`/`expect`/indexing in the new production code; all allocations are bounded by the
  validated frame size, the 2 MiB preview budget or `MAX_FRAME_SIZE_HINTS`.
- No pixel data, frame content or biometric data is logged (the oversize warning logs only the
  peer UID). The preview authorization, session and rate-limit checks run before, unchanged.
- No new `unsafe`; no PAM code touched; no Tokio or OpenCV added; socket permissions unchanged.
- `soos-gui` already treats an empty preview (`width = 0`) as "no frame yet" and keeps polling.

## 6. Gate

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features --
-D warnings`, `cargo test --locked --workspace --all-targets --all-features --no-fail-fast` and
`./scripts/candid_review.sh` were run on the final diff (results in the branch report).

## 7. Follow-ups

- Validate the frame-size signature and the deep-greyscale wire path on real IR modules
  (Y16 / Y8I RealSense-style nodes, Windows-Hello modules advertising YUYV).
- `bus_info`, `driver` and `device_caps` were not added: both nodes of one USB camera share them,
  so they do not separate RGB from IR; revisit if a vendor-specific signal is found.
- Packed deep-greyscale variants (`Y10P`, `Y10B`, `Y12P`) are not mapped.
- Candid review finding 3 (open): udev builds `/dev/v4l/by-id/` names from the USB device
  product string, so every interface of a composite RGB+IR module shares the by-id stem. If that
  product string carries an `IR` token, both nodes are classified Infrared and `PreferIr` may pick
  the RGB node (fail-closed: stricter IR PAD policy, but the wrong camera). Planned fix: when
  several capture nodes share a by-id stem apart from `-index<N>`, do not treat the by-id token as
  decisive and fall through to the next rule, with a fixture row for this case.
- Candid review finding 6 (open): the IR frame-size signature (every size at most 640x400) is
  checked before the colour-format rule, so a low-resolution RGB webcam without VGA is classified
  IR (fail-closed). Log at `info` when this rule alone decides the classification, and validate
  the signature on real hardware.
- Capture-side hints (candid review finding 4, documented, not changed): the capture supervisor
  looks up no by-id alias for an explicit `/dev/videoN` `device_path`; see
  `Docs/CAMERA_V4L_CRATE.md`.
