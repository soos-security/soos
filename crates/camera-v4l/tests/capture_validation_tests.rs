//! Contract tests for the V4L2 capture-path validation (GitHub #192, #193, #194).
//!
//! - #192 (CAM-10): the driver-returned fourcc, `bytesperline` and `V4L2_BUF_FLAG_ERROR` are
//!   honoured and every captured buffer is size-validated before it is published.
//! - #193 (CAM-11): `fps` is requested from the hardware (`VIDIOC_S_PARM`) and `idle_fps` is a
//!   publication throttle shared by the V4L2 manager and the mock.
//! - #194 (CAM-12): the DQBUF poll timeout is bounded to 150-250 ms so `Drop` completes well
//!   within the 500 ms Criterion C9 budget; a sustained stall escalates to `Starved`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions and unwraps"
)]

use soos_camera_v4l::capture::{
    apply_frame_rate, dqbuf_poll_timeout, granted_fps, max_consecutive_poll_timeouts, publish_due,
    requested_frame_interval, validate_captured_buffer, validate_negotiated_format, BufferMeta,
    FormatValidationError, FrameRateOutcome, FrameRejection, NegotiatedFormat, C9_SHUTDOWN_BUDGET,
    MAX_CONSECUTIVE_REJECTED_FRAMES, MAX_DQBUF_POLL_TIMEOUT, MAX_STREAM_STALL,
    MIN_DQBUF_POLL_TIMEOUT,
};
use soos_camera_v4l::{
    resolve_camera_device, CameraConfigBuilder, CameraManager, CameraResolutionSource, PixelFormat,
    SensorPreference, SystemCameraEnumerator, V4lCameraManager,
};
use std::cell::Cell;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

mod common;

use common::{wait_until, SETTLE_TIMEOUT};

fn meta(bytesused: u32) -> BufferMeta {
    BufferMeta {
        bytesused,
        error_flag: false,
    }
}

fn layout(format: PixelFormat, width: u32, height: u32, bytes_per_line: u32) -> NegotiatedFormat {
    validate_negotiated_format(
        format,
        soos_camera_v4l::pixel_format_to_fourcc(format).repr,
        width,
        height,
        bytes_per_line,
    )
    .expect("canonical layout must validate")
}

// ---------------------------------------------------------------------------------------------
// #192: driver-returned format validation
// ---------------------------------------------------------------------------------------------

#[test]
fn test_validate_negotiated_format_accepts_exact_fourcc() {
    let negotiated = validate_negotiated_format(PixelFormat::Yuyv, *b"YUYV", 640, 480, 1280)
        .expect("exact YUYV must validate");
    assert_eq!(negotiated.format, PixelFormat::Yuyv);
    assert_eq!((negotiated.width, negotiated.height), (640, 480));
    assert_eq!(negotiated.bytes_per_line, 1280);
    assert!(!negotiated.substituted);
}

#[test]
fn test_validate_negotiated_format_labels_known_substitution() {
    // enum_formats failed and YUYV was requested blindly; the driver substituted MJPEG.
    let negotiated = validate_negotiated_format(PixelFormat::Yuyv, *b"MJPG", 640, 480, 0)
        .expect("a known substituted layout must be adopted and labelled");
    assert_eq!(
        negotiated.format,
        PixelFormat::Mjpeg,
        "frames must be labelled with the driver-returned format, never the requested one"
    );
    assert!(negotiated.substituted);
}

#[test]
fn test_validate_negotiated_format_rejects_unknown_substitution() {
    let err = validate_negotiated_format(PixelFormat::Yuyv, *b"H264", 640, 480, 0)
        .expect_err("an unsupported driver fourcc must fail closed");
    assert!(matches!(
        err,
        FormatValidationError::UnsupportedFourcc { .. }
    ));
    let text = err.to_string();
    assert!(text.contains("H264") && text.contains("YUYV"), "{text}");
}

#[test]
fn test_validate_negotiated_format_rejects_bgr_and_rgb32_substitution() {
    // BGR3 has swapped channels and RGB4 has four bytes per pixel: labelling them Rgb24 would
    // feed wrong colours or a wrong buffer size to detection.
    for fourcc in [*b"BGR3", *b"RGB4"] {
        let err = validate_negotiated_format(PixelFormat::Rgb24, fourcc, 640, 480, 0)
            .expect_err("non-RGB24 layouts must never be labelled Rgb24");
        assert!(matches!(
            err,
            FormatValidationError::UnsupportedFourcc { .. }
        ));
    }
}

#[test]
fn test_validate_negotiated_format_accepts_grey_aliases() {
    for fourcc in [*b"GREY", *b"Y800", *b"Y8  "] {
        let negotiated = validate_negotiated_format(PixelFormat::Grey, fourcc, 640, 480, 640)
            .expect("8-bit grey aliases share one layout");
        assert_eq!(negotiated.format, PixelFormat::Grey);
    }
}

#[test]
fn test_validate_negotiated_format_rejects_stride_smaller_than_row() {
    let err = validate_negotiated_format(PixelFormat::Yuyv, *b"YUYV", 640, 480, 1000)
        .expect_err("a stride below width*2 cannot hold a YUYV row");
    assert_eq!(
        err,
        FormatValidationError::StrideTooSmall {
            format: PixelFormat::Yuyv,
            bytes_per_line: 1000,
            row_bytes: 1280,
        }
    );
}

#[test]
fn test_validate_negotiated_format_zero_stride_means_tight_rows() {
    let negotiated = validate_negotiated_format(PixelFormat::Rgb24, *b"RGB3", 4, 2, 0).unwrap();
    assert_eq!(negotiated.bytes_per_line, 12);
}

#[test]
fn test_validate_negotiated_format_rejects_invalid_dimensions() {
    for (w, h) in [(0, 480), (640, 0)] {
        let err = validate_negotiated_format(PixelFormat::Yuyv, *b"YUYV", w, h, 0).unwrap_err();
        assert!(matches!(
            err,
            FormatValidationError::InvalidDimensions { .. }
        ));
    }
    // NV12 chroma is subsampled 2x2: odd dimensions have no well-defined tight layout.
    let err = validate_negotiated_format(PixelFormat::Nv12, *b"NV12", 641, 480, 0).unwrap_err();
    assert!(matches!(
        err,
        FormatValidationError::InvalidDimensions { .. }
    ));
    let err = validate_negotiated_format(PixelFormat::Nv12, *b"NV12", 640, 481, 0).unwrap_err();
    assert!(matches!(
        err,
        FormatValidationError::InvalidDimensions { .. }
    ));
}

// ---------------------------------------------------------------------------------------------
// #192: captured buffer validation
// ---------------------------------------------------------------------------------------------

#[test]
fn test_validate_captured_buffer_rejects_short_yuyv() {
    let fmt = layout(PixelFormat::Yuyv, 640, 480, 0);
    let buf = vec![0u8; 640 * 480 * 2 - 1];
    let err = validate_captured_buffer(&buf, meta(buf.len() as u32), &fmt)
        .expect_err("a truncated YUYV buffer must never be published");
    assert_eq!(
        err,
        FrameRejection::ShortBuffer {
            expected: 640 * 480 * 2,
            actual: 640 * 480 * 2 - 1,
        }
    );
}

#[test]
fn test_validate_captured_buffer_accepts_exact_yuyv() {
    let fmt = layout(PixelFormat::Yuyv, 640, 480, 0);
    let buf = vec![7u8; 640 * 480 * 2];
    let data = validate_captured_buffer(&buf, meta(buf.len() as u32), &fmt).unwrap();
    assert_eq!(
        Some(data.len()),
        PixelFormat::Yuyv.expected_buffer_size(640, 480)
    );
}

#[test]
fn test_validate_captured_buffer_rejects_error_flag() {
    let fmt = layout(PixelFormat::Yuyv, 4, 2, 0);
    let buf = vec![0u8; 16];
    let err = validate_captured_buffer(
        &buf,
        BufferMeta {
            bytesused: 16,
            error_flag: true,
        },
        &fmt,
    )
    .expect_err("V4L2_BUF_FLAG_ERROR frames carry corrupted data and must be skipped");
    assert_eq!(err, FrameRejection::ErrorFlag);
}

#[test]
fn test_validate_captured_buffer_destrides_padded_rows() {
    // 4x2 YUYV (8 bytes per row) padded to a 12-byte stride.
    let fmt = layout(PixelFormat::Yuyv, 4, 2, 12);
    let mut buf = Vec::new();
    buf.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 0xEE, 0xEE, 0xEE, 0xEE]);
    buf.extend_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16, 0xEE, 0xEE, 0xEE, 0xEE]);
    let data = validate_captured_buffer(&buf, meta(24), &fmt).unwrap();
    assert_eq!(
        data,
        (1..=16).collect::<Vec<u8>>(),
        "padding bytes must be stripped so the buffer matches width*height*2"
    );
}

#[test]
fn test_validate_captured_buffer_destrides_nv12_planes() {
    // 4x2 NV12: two luma rows + one interleaved chroma row, each padded from 4 to 6 bytes.
    let fmt = layout(PixelFormat::Nv12, 4, 2, 6);
    let buf = vec![
        1, 2, 3, 4, 0xEE, 0xEE, // luma row 0
        5, 6, 7, 8, 0xEE, 0xEE, // luma row 1
        9, 10, 11, 12, 0xEE, 0xEE, // chroma row
    ];
    let data = validate_captured_buffer(&buf, meta(18), &fmt).unwrap();
    assert_eq!(data, (1..=12).collect::<Vec<u8>>());
    assert_eq!(
        Some(data.len()),
        PixelFormat::Nv12.expected_buffer_size(4, 2)
    );
}

#[test]
fn test_validate_captured_buffer_short_padded_frame_rejected() {
    // With a 12-byte stride the frame needs 2 full strides (the last row's padding may be
    // omitted by the driver, but the pixels may not).
    let fmt = layout(PixelFormat::Yuyv, 4, 2, 12);
    let buf = vec![0u8; 19];
    let err = validate_captured_buffer(&buf, meta(19), &fmt).unwrap_err();
    assert_eq!(
        err,
        FrameRejection::ShortBuffer {
            expected: 20,
            actual: 19
        }
    );
    // Exactly (rows - 1) * stride + row bytes is accepted.
    let buf = vec![0u8; 20];
    assert_eq!(
        validate_captured_buffer(&buf, meta(20), &fmt)
            .unwrap()
            .len(),
        16
    );
}

#[test]
fn test_validate_captured_buffer_bytesused_handling() {
    let fmt = layout(PixelFormat::Grey, 4, 2, 0);
    // bytesused shorter than the mapping: only the used prefix counts.
    let buf = vec![3u8; 64];
    let err = validate_captured_buffer(&buf, meta(7), &fmt).unwrap_err();
    assert_eq!(
        err,
        FrameRejection::ShortBuffer {
            expected: 8,
            actual: 7
        }
    );
    assert_eq!(
        validate_captured_buffer(&buf, meta(8), &fmt).unwrap().len(),
        8
    );
    // bytesused == 0 on an uncompressed format: the driver did not report it, the mapping is used.
    assert_eq!(
        validate_captured_buffer(&buf, meta(0), &fmt).unwrap().len(),
        8
    );
    // bytesused beyond the mapping is a driver bug: reject, never read past it.
    let err = validate_captured_buffer(&buf, meta(65), &fmt).unwrap_err();
    assert_eq!(
        err,
        FrameRejection::BytesUsedExceedsBuffer {
            bytesused: 65,
            capacity: 64
        }
    );
}

#[test]
fn test_validate_captured_buffer_mjpeg_keeps_payload_and_rejects_empty() {
    let fmt = layout(PixelFormat::Mjpeg, 640, 480, 0);
    let buf = vec![0xFFu8; 4096];
    let data = validate_captured_buffer(&buf, meta(100), &fmt).unwrap();
    assert_eq!(data.len(), 100, "compressed payload is exactly bytesused");
    let err = validate_captured_buffer(&buf, meta(0), &fmt).unwrap_err();
    assert_eq!(err, FrameRejection::EmptyPayload);
}

#[test]
fn test_rejected_frame_streak_is_bounded() {
    // About one second of corrupted frames at 30 fps, then the device is reopened.
    assert_eq!(
        MAX_CONSECUTIVE_REJECTED_FRAMES, 30,
        "a stream of corrupted frames must reopen the device within about one second"
    );
}

// ---------------------------------------------------------------------------------------------
// #194: bounded DQBUF poll timeout (Criterion C9)
// ---------------------------------------------------------------------------------------------

#[test]
fn test_dqbuf_poll_timeout_bounded_for_every_fps() {
    assert_eq!(MIN_DQBUF_POLL_TIMEOUT, Duration::from_millis(150));
    assert_eq!(MAX_DQBUF_POLL_TIMEOUT, Duration::from_millis(250));
    for fps in [0, 1, 2, 5, 10, 15, 20, 25, 30, 60, 120, 1000, u32::MAX] {
        let timeout = dqbuf_poll_timeout(fps);
        assert!(
            timeout >= MIN_DQBUF_POLL_TIMEOUT && timeout <= MAX_DQBUF_POLL_TIMEOUT,
            "fps {fps}: poll timeout {timeout:?} outside 150-250 ms"
        );
        // The supervisor re-checks `running` between polls: Drop waits at most one poll.
        assert!(
            timeout.saturating_mul(2) <= C9_SHUTDOWN_BUDGET,
            "fps {fps}: a poll of {timeout:?} leaves no margin in the C9 budget"
        );
    }
    assert_eq!(C9_SHUTDOWN_BUDGET, Duration::from_millis(500));
}

#[test]
fn test_dqbuf_poll_timeout_is_three_frame_intervals_when_in_range() {
    assert_eq!(dqbuf_poll_timeout(15), Duration::from_millis(200));
    assert_eq!(dqbuf_poll_timeout(30), MIN_DQBUF_POLL_TIMEOUT);
    assert_eq!(dqbuf_poll_timeout(5), MAX_DQBUF_POLL_TIMEOUT);
}

#[test]
fn test_stall_budget_preserves_two_second_tolerance() {
    assert_eq!(MAX_STREAM_STALL, Duration::from_millis(2000));
    for timeout in [
        MIN_DQBUF_POLL_TIMEOUT,
        Duration::from_millis(200),
        MAX_DQBUF_POLL_TIMEOUT,
    ] {
        let limit = max_consecutive_poll_timeouts(timeout);
        assert!(limit >= 1);
        assert!(
            timeout.saturating_mul(limit) >= MAX_STREAM_STALL,
            "{limit} x {timeout:?} escalates to Starved before the 2 s stall tolerance"
        );
        assert!(
            timeout.saturating_mul(limit - 1) < MAX_STREAM_STALL,
            "{limit} x {timeout:?} tolerates a stall longer than necessary"
        );
    }
    assert_eq!(max_consecutive_poll_timeouts(MAX_DQBUF_POLL_TIMEOUT), 8);
    assert_eq!(max_consecutive_poll_timeouts(Duration::ZERO), 1);
}

// ---------------------------------------------------------------------------------------------
// #193: frame rate requested from the hardware; idle_fps publication throttle parity
// ---------------------------------------------------------------------------------------------

#[test]
fn test_requested_frame_interval_uses_configured_fps() {
    assert_eq!(requested_frame_interval(30), (1, 30));
    assert_eq!(requested_frame_interval(15), (1, 15));
    // A zero rate (struct literal bypassing the builder clamp) never requests a 1/0 interval:
    // it falls back to the 30 fps default.
    assert_eq!(requested_frame_interval(0), (1, 30));
}

#[test]
fn test_granted_fps_from_driver_interval() {
    assert_eq!(granted_fps(1, 30), Some(30));
    assert_eq!(granted_fps(1001, 30000), Some(29));
    assert_eq!(granted_fps(0, 30), None);
    assert_eq!(granted_fps(1, 0), None);
}

// ---------------------------------------------------------------------------------------------
// VIDIOC_S_PARM seam (GitHub #193): `apply_frame_rate` is the only code path open_and_stream uses
// to request the frame rate; the ioctl is injected as a closure.
// ---------------------------------------------------------------------------------------------

const SPARM_PATH: &str = "/dev/video-sparm";

#[test]
fn test_apply_frame_rate_accepted_request() {
    let seen = Cell::new(None);
    let outcome = apply_frame_rate(Path::new(SPARM_PATH), 30, |num, den| {
        seen.set(Some((num, den)));
        Ok((num, den))
    });
    assert_eq!(
        seen.get(),
        Some((1, 30)),
        "the ioctl must receive a 1/fps interval"
    );
    assert_eq!(
        outcome,
        FrameRateOutcome::Granted {
            requested_fps: 30,
            numerator: 1,
            denominator: 30,
        }
    );
    assert_eq!(outcome.granted_fps(), Some(30));
}

#[test]
fn test_apply_frame_rate_adjusted_request() {
    // The driver rounds a 60 fps request to its closest supported 1/15 s interval.
    let seen = Cell::new(None);
    let outcome = apply_frame_rate(Path::new(SPARM_PATH), 60, |num, den| {
        seen.set(Some((num, den)));
        Ok((1, 15))
    });
    assert_eq!(seen.get(), Some((1, 60)));
    assert_eq!(
        outcome,
        FrameRateOutcome::Granted {
            requested_fps: 60,
            numerator: 1,
            denominator: 15,
        }
    );
    assert_eq!(
        outcome.granted_fps(),
        Some(15),
        "the granted rate, not the requested one, drives the DQBUF poll"
    );
}

#[test]
fn test_apply_frame_rate_unusable_granted_interval() {
    // A driver answering with a 0/0 interval keeps the configured rate for the poll timeout.
    let outcome = apply_frame_rate(Path::new(SPARM_PATH), 30, |_, _| Ok((0, 0)));
    assert!(matches!(outcome, FrameRateOutcome::Granted { .. }));
    assert_eq!(outcome.granted_fps(), None);
}

#[test]
fn test_apply_frame_rate_refusal_is_non_fatal() {
    // A driver without frame-interval control refuses VIDIOC_S_PARM: the refusal is reported as
    // an outcome (a warning is logged), never as an error that would abort stream setup.
    let calls = Cell::new(0u32);
    let outcome = apply_frame_rate(Path::new(SPARM_PATH), 30, |_, _| {
        calls.set(calls.get() + 1);
        Err(io::Error::from_raw_os_error(25)) // ENOTTY
    });
    assert_eq!(calls.get(), 1, "the ioctl is attempted exactly once");
    assert_eq!(outcome, FrameRateOutcome::Refused { requested_fps: 30 });
    assert_eq!(outcome.granted_fps(), None);
}

#[test]
fn test_apply_frame_rate_zero_fps_requests_default() {
    let seen = Cell::new(None);
    let outcome = apply_frame_rate(Path::new(SPARM_PATH), 0, |num, den| {
        seen.set(Some((num, den)));
        Err(io::Error::from_raw_os_error(16)) // EBUSY
    });
    assert_eq!(
        seen.get(),
        Some((1, 30)),
        "a zero rate never requests a 1/0 interval"
    );
    assert_eq!(outcome, FrameRateOutcome::Refused { requested_fps: 30 });
}

#[test]
fn test_publish_fps_idle_throttle_parity() {
    let config = CameraConfigBuilder::new()
        .fps(30)
        .idle_fps(5)
        .idle_timeout(Duration::from_secs(10))
        .build();
    assert_eq!(config.publish_fps(Duration::from_secs(1)), 30);
    assert_eq!(config.publish_fps(Duration::from_secs(5)), 30);
    assert_eq!(config.publish_fps(Duration::from_secs(6)), 5);

    // A disabled idle timeout (the GUI direct-camera mode) never throttles.
    let config = CameraConfigBuilder::new()
        .fps(30)
        .idle_fps(5)
        .idle_timeout(Duration::ZERO)
        .build();
    assert_eq!(config.publish_fps(Duration::from_secs(3600)), 30);
}

#[test]
fn test_publish_due_throttles_only_below_full_rate() {
    // Full rate: every hardware frame is published.
    assert!(publish_due(Some(Duration::from_millis(1)), 30, 30));
    // First frame after (re)start is always published.
    assert!(publish_due(None, 5, 30));
    // Throttled to 5 fps from a 30 fps stream: one frame every ~200 ms.
    assert!(!publish_due(Some(Duration::from_millis(100)), 5, 30));
    assert!(!publish_due(Some(Duration::from_millis(166)), 5, 30));
    // Six 33 ms hardware frames (~200 ms minus jitter) are enough.
    assert!(publish_due(Some(Duration::from_millis(190)), 5, 30));
    assert!(publish_due(Some(Duration::from_millis(250)), 5, 30));
}

// ---------------------------------------------------------------------------------------------
// Hardware evidence (opt-in): production Drop latency while streaming a real camera.
// ---------------------------------------------------------------------------------------------

/// Environment switch enabling hardware tests (same gate as `hardware_smoke_tests.rs`, ADR
/// "Hermetic V4L2 Enumeration" (3)).
const HW_TESTS_ENV: &str = "SOOS_HW_TESTS";

/// Measures the production `V4lCameraManager` Drop latency while it streams the auto-resolved
/// camera (Criterion C9, GitHub #194). Never runs in CI: `#[ignore]`d and gated by
/// `SOOS_HW_TESTS=1`. Manual evidence only; it does not verify `VIDIOC_S_PARM`.
#[test]
#[ignore = "requires a V4L2 camera; run with SOOS_HW_TESTS=1 and --ignored"]
fn test_v4l_streaming_drop_completes_within_budget_on_hardware() {
    if !std::env::var(HW_TESTS_ENV).is_ok_and(|v| v == "1") {
        return;
    }
    let resolution = resolve_camera_device(
        None,
        SensorPreference::default(),
        &SystemCameraEnumerator::default(),
    );
    assert_eq!(
        resolution.source,
        CameraResolutionSource::AutoDetected,
        "auto-detection found no capture node"
    );
    let config = CameraConfigBuilder::new()
        .device_path(&resolution.path)
        .warmup_frames(2)
        .idle_timeout(Duration::ZERO)
        .build();
    let camera = V4lCameraManager::spawn(config).expect("spawn must succeed");
    assert!(
        wait_until(SETTLE_TIMEOUT, || camera.is_ready()),
        "hardware camera never became ready"
    );
    let start = Instant::now();
    drop(camera);
    let elapsed = start.elapsed();
    assert!(
        elapsed < C9_SHUTDOWN_BUDGET,
        "streaming V4lCameraManager drop took {elapsed:?} (Criterion C9: < 500 ms)"
    );
}
