//! Contract tests of the pure JPEG path of the live camera view of `soos-remote` (GitHub #345,
//! ADR 2026-10-07 "Live Camera View in `soos-remote` Through the Daemon Preview Channel",
//! architect spec `AI/architect_spec_remote_live_camera.md` §6, tests 1–9, matrix RLC9,
//! RLC11, RLC13).
//!
//! Pure: synthetic Grey / YUYV / RGB24 frames built in memory, no I/O, no clock. The JPEG
//! output is checked by walking its marker segments (SOI, SOF0, DQT, SOS, EOI); nothing
//! decodes the image (`soos-remote` ships no decoder, RLC-S8).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::manual_div_ceil,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::io::{ErrorKind, Write};

use proptest::prelude::*;

use soos_remote::camera_jpeg::{
    convert_frame, encode_preview_jpeg, encode_preview_jpeg_into, FrameError, JpegSettings,
    JpegSink, PixelKind,
};
use soos_remote::config::CameraWidth;
use soos_remote::{
    CAMERA_FULL_WIDTH, CAMERA_HALF_WIDTH, CAMERA_MAX_SCRATCH_BYTES, CAMERA_MAX_SOURCE_HEIGHT,
    CAMERA_MAX_SOURCE_WIDTH, CAMERA_MIN_SOURCE_DIM, MAX_CAMERA_JPEG_BYTES,
};

/// Wire format codes (`soos_protocol::types::PREVIEW_FORMAT_*`, pinned by test 49).
const RGB24: u8 = 0;
const GREY: u8 = 1;
const YUYV: u8 = 2;
const NV12: u8 = 3;
const MJPEG: u8 = 4;
const EMPTY: u8 = 255;

fn settings(width: CameraWidth, quality: u8) -> JpegSettings {
    JpegSettings { width, quality }
}

fn full() -> JpegSettings {
    settings(CameraWidth::Full, 70)
}

/// Deterministic xorshift noise (no RNG dependency, identical on every run).
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut s = seed | 1;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 32) as u8
        })
        .collect()
}

/// A smooth gradient frame of `bpp` bytes per pixel.
fn gradient(w: usize, h: usize, bpp: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * bpp);
    for y in 0..h {
        for x in 0..w {
            for c in 0..bpp {
                out.push(((x * 3 + y * 5 + c * 40) % 256) as u8);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Minimal JPEG marker walker (structure only, no decoding)
// ---------------------------------------------------------------------------------------

/// One marker segment: marker byte (the `xx` of `FF xx`) and its payload (after the length).
struct Segment {
    marker: u8,
    payload: Vec<u8>,
}

/// Walks the header segments of a baseline JPEG up to and including SOS. Panics on any
/// structural error (a malformed output fails the test).
fn segments(jpeg: &[u8]) -> Vec<Segment> {
    assert!(jpeg.len() >= 4, "jpeg too short: {}", jpeg.len());
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "SOI first");
    let mut out = Vec::new();
    let mut i = 2;
    loop {
        assert_eq!(jpeg[i], 0xFF, "marker expected at {i}");
        let marker = jpeg[i + 1];
        let len = usize::from(u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]));
        assert!(len >= 2, "segment length at {i}");
        let payload = jpeg[i + 4..i + 2 + len].to_vec();
        out.push(Segment { marker, payload });
        i += 2 + len;
        if marker == 0xDA {
            break;
        }
    }
    out
}

/// `(height, width, [(component id, sampling byte)])` of the single SOF0 segment; asserts
/// that the image is baseline (no SOF2) and ends with EOI.
fn sof0(jpeg: &[u8]) -> (u16, u16, Vec<(u8, u8)>) {
    assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "EOI last");
    let segs = segments(jpeg);
    assert!(
        segs.iter().all(|s| s.marker != 0xC2),
        "progressive SOF2 must never be emitted"
    );
    let frames: Vec<&Segment> = segs.iter().filter(|s| s.marker == 0xC0).collect();
    assert_eq!(frames.len(), 1, "exactly one baseline SOF0");
    let p = &frames[0].payload;
    assert_eq!(p[0], 8, "8-bit precision");
    let height = u16::from_be_bytes([p[1], p[2]]);
    let width = u16::from_be_bytes([p[3], p[4]]);
    let n = usize::from(p[5]);
    let comps = (0..n).map(|k| (p[6 + 3 * k], p[7 + 3 * k])).collect();
    (height, width, comps)
}

/// Concatenated payloads of every DQT segment.
fn dqt(jpeg: &[u8]) -> Vec<u8> {
    segments(jpeg)
        .into_iter()
        .filter(|s| s.marker == 0xDB)
        .flat_map(|s| s.payload)
        .collect()
}

// ---------------------------------------------------------------------------------------
// Reference 2x2 box average (round half up), spec §6.3
// ---------------------------------------------------------------------------------------

fn avg4(a: u8, b: u8, c: u8, d: u8) -> u8 {
    ((u32::from(a) + u32::from(b) + u32::from(c) + u32::from(d) + 2) / 4) as u8
}

fn avg2(a: u8, b: u8) -> u8 {
    ((u32::from(a) + u32::from(b) + 1) / 2) as u8
}

/// Reference half-size of a packed frame of `bpp` bytes per pixel (Grey 1, RGB 3).
fn half_packed(src: &[u8], w: usize, h: usize, bpp: usize) -> Vec<u8> {
    let (ow, oh) = (w / 2, h / 2);
    let mut out = Vec::with_capacity(ow * oh * bpp);
    for oy in 0..oh {
        for ox in 0..ow {
            for c in 0..bpp {
                let at = |x: usize, y: usize| src[(y * w + x) * bpp + c];
                out.push(avg4(
                    at(2 * ox, 2 * oy),
                    at(2 * ox + 1, 2 * oy),
                    at(2 * ox, 2 * oy + 1),
                    at(2 * ox + 1, 2 * oy + 1),
                ));
            }
        }
    }
    out
}

/// Reference full-size YUYV → YCbCr 4:4:4 shuffle.
fn yuyv_full(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for pair in 0..w / 2 {
            let base = (y * w + pair * 2) * 2;
            let (y0, u, y1, v) = (src[base], src[base + 1], src[base + 2], src[base + 3]);
            out.extend_from_slice(&[y0, u, v, y1, u, v]);
        }
    }
    out
}

/// Reference half-size YUYV: one output pixel per pixel pair of two rows.
fn yuyv_half(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (ow, oh) = (w / 2, h / 2);
    let mut out = Vec::with_capacity(ow * oh * 3);
    for oy in 0..oh {
        for ox in 0..ow {
            let b0 = ((2 * oy) * w + 2 * ox) * 2;
            let b1 = ((2 * oy + 1) * w + 2 * ox) * 2;
            let luma = avg4(src[b0], src[b0 + 2], src[b1], src[b1 + 2]);
            let u = avg2(src[b0 + 1], src[b1 + 1]);
            let v = avg2(src[b0 + 3], src[b1 + 3]);
            out.extend_from_slice(&[luma, u, v]);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Test 1
// ---------------------------------------------------------------------------------------

/// Test 1 (RLC11): a 64x48 Grey frame encodes to a baseline JFIF: `FF D8` … `FF D9`, one
/// SOF0 with height 48, width 64 and one component; never progressive.
#[test]
fn test_rlc_grey_frame_encodes_to_baseline_jpeg() {
    let data = gradient(64, 48, 1);
    let frame = encode_preview_jpeg(GREY, 64, 48, &data, full()).expect("grey encodes");
    let bytes: &[u8] = &frame.bytes;
    assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    let (h, w, comps) = sof0(bytes);
    assert_eq!((h, w), (48, 64));
    assert_eq!(comps.len(), 1, "grey is a single-component JPEG");
    assert!(bytes.len() <= MAX_CAMERA_JPEG_BYTES);
}

// ---------------------------------------------------------------------------------------
// Test 2
// ---------------------------------------------------------------------------------------

/// Test 2 (RLC11): YUYV pixel pairs `Y0 U Y1 V` become `(Y0,U,V),(Y1,U,V)` (byte shuffle,
/// `PixelKind::Ycbcr`); the encoded SOF0 has 3 components sampled 0x22 / 0x11 / 0x11
/// (4:2:0).
#[test]
fn test_rlc_yuyv_conversion_shuffles_chroma() {
    let (w, h) = (16usize, 16usize);
    let mut data = Vec::with_capacity(w * h * 2);
    for i in 0..(w * h / 2) {
        let i = i as u8;
        // Y0, U, Y1, V with distinct, recognisable values.
        data.extend_from_slice(&[
            i,
            100u8.wrapping_add(i),
            i.wrapping_add(1),
            200u8.wrapping_add(i),
        ]);
    }
    let (pixels, ow, oh, kind) =
        convert_frame(YUYV, w as u32, h as u32, &data, CameraWidth::Full).expect("yuyv converts");
    assert_eq!((ow, oh, kind), (16, 16, PixelKind::Ycbcr));
    assert_eq!(pixels.len(), w * h * 3);
    assert_eq!(
        &pixels[..6],
        &[0, 100, 200, 1, 100, 200],
        "first pair shuffled"
    );
    assert_eq!(
        &pixels[6..12],
        &[1, 101, 201, 2, 101, 201],
        "second pair shuffled"
    );
    assert_eq!(&pixels[..], &yuyv_full(&data, w, h)[..]);

    let frame = encode_preview_jpeg(YUYV, w as u32, h as u32, &data, full()).expect("encodes");
    let (eh, ew, comps) = sof0(&frame.bytes);
    assert_eq!((eh, ew), (16, 16));
    let sampling: Vec<u8> = comps.iter().map(|c| c.1).collect();
    assert_eq!(sampling, vec![0x22, 0x11, 0x11], "4:2:0 sampling");
}

// ---------------------------------------------------------------------------------------
// Test 3
// ---------------------------------------------------------------------------------------

/// Test 3 (RLC11): RGB24 converts by byte copy (`PixelKind::Rgb`) and encodes to a
/// 3-component baseline JPEG with the exact source dimensions.
#[test]
fn test_rlc_rgb24_frame_encodes() {
    let data = gradient(48, 32, 3);
    let (pixels, ow, oh, kind) =
        convert_frame(RGB24, 48, 32, &data, CameraWidth::Full).expect("rgb converts");
    assert_eq!((ow, oh, kind), (48, 32, PixelKind::Rgb));
    assert_eq!(&pixels[..], &data[..], "RGB24 is a byte copy");
    let frame = encode_preview_jpeg(RGB24, 48, 32, &data, full()).expect("rgb encodes");
    let (h, w, comps) = sof0(&frame.bytes);
    assert_eq!((h, w), (32, 48));
    assert_eq!(comps.len(), 3);
    // A Grey frame of the same geometry stays single-component (kind is not guessed).
    let (_, _, _, kind) = convert_frame(GREY, 48, 32, &data[..48 * 32], CameraWidth::Full).unwrap();
    assert_eq!(kind, PixelKind::Luma);
}

// ---------------------------------------------------------------------------------------
// Test 4
// ---------------------------------------------------------------------------------------

/// Test 4 (RLC9, F6): `Half` is a 2x2 box average with round-half-up on Grey, RGB24 and YUYV
/// (`(U_row0 + U_row1 + 1) / 2` for chroma); odd sources drop their last column/row;
/// `Half` on a source at most 320 wide is unchanged; `Full` never scales.
#[test]
fn test_rlc_half_width_box_average() {
    // Hand-checked rounding: (0+0+1+1+2)/4 = 1 (half up, truncation would give 0);
    // (1+2+3+5+2)/4 = 3; (1+1+1+2+2)/4 = 1.
    let mut grey = vec![0u8; 640 * 480];
    grey[0] = 0;
    grey[1] = 0;
    grey[640] = 1;
    grey[641] = 1;
    grey[2] = 1;
    grey[3] = 2;
    grey[642] = 3;
    grey[643] = 5;
    grey[4] = 1;
    grey[5] = 1;
    grey[644] = 1;
    grey[645] = 2;
    let (out, ow, oh, kind) = convert_frame(GREY, 640, 480, &grey, CameraWidth::Half).unwrap();
    assert_eq!((ow, oh, kind), (320, 240, PixelKind::Luma));
    assert_eq!(&out[..3], &[1, 3, 1], "round half up");

    // Whole-frame equality with the reference on noisy content, every format.
    let g = noise(640 * 480, 7);
    let (out, ow, oh, _) = convert_frame(GREY, 640, 480, &g, CameraWidth::Half).unwrap();
    assert_eq!((ow, oh), (320, 240));
    assert_eq!(&out[..], &half_packed(&g, 640, 480, 1)[..]);

    let rgb = noise(640 * 480 * 3, 11);
    let (out, ow, oh, kind) = convert_frame(RGB24, 640, 480, &rgb, CameraWidth::Half).unwrap();
    assert_eq!((ow, oh, kind), (320, 240, PixelKind::Rgb));
    assert_eq!(&out[..], &half_packed(&rgb, 640, 480, 3)[..]);

    let yuyv = noise(640 * 480 * 2, 13);
    let (out, ow, oh, kind) = convert_frame(YUYV, 640, 480, &yuyv, CameraWidth::Half).unwrap();
    assert_eq!((ow, oh, kind), (320, 240, PixelKind::Ycbcr));
    assert_eq!(&out[..], &yuyv_half(&yuyv, 640, 480)[..]);

    // Odd sources: the last odd column and/or row is dropped.
    let g = noise(533 * 401, 17);
    let (out, ow, oh, _) = convert_frame(GREY, 533, 401, &g, CameraWidth::Half).unwrap();
    assert_eq!((ow, oh), (266, 200));
    assert_eq!(&out[..], &half_packed(&g, 533, 401, 1)[..]);

    let rgb = noise(533 * 400 * 3, 19);
    let (out, ow, oh, _) = convert_frame(RGB24, 533, 400, &rgb, CameraWidth::Half).unwrap();
    assert_eq!((ow, oh), (266, 200));
    assert_eq!(&out[..], &half_packed(&rgb, 533, 400, 3)[..]);

    let yuyv = noise(534 * 401 * 2, 23);
    let (out, ow, oh, _) = convert_frame(YUYV, 534, 401, &yuyv, CameraWidth::Half).unwrap();
    assert_eq!((ow, oh), (267, 200));
    assert_eq!(&out[..], &yuyv_half(&yuyv, 534, 401)[..]);

    // Half on a source at most CAMERA_HALF_WIDTH wide: unchanged.
    let g = noise(320 * 240, 29);
    let (out, ow, oh, _) = convert_frame(GREY, 320, 240, &g, CameraWidth::Half).unwrap();
    assert_eq!((ow, oh), (320, 240));
    assert_eq!(&out[..], &g[..]);
    let rgb = noise(100 * 81 * 3, 31);
    let (out, ow, oh, _) = convert_frame(RGB24, 100, 81, &rgb, CameraWidth::Half).unwrap();
    assert_eq!(
        (ow, oh),
        (100, 81),
        "odd height kept when no downscale happens"
    );
    assert_eq!(&out[..], &rgb[..]);

    // Full never scales, even at the maximum source size.
    let g = noise(640 * 480, 37);
    let (out, ow, oh, _) = convert_frame(GREY, 640, 480, &g, CameraWidth::Full).unwrap();
    assert_eq!((ow, oh), (640, 480));
    assert_eq!(&out[..], &g[..]);

    // The encoded half-width JPEG carries the halved size.
    let frame = encode_preview_jpeg(GREY, 640, 480, &g, settings(CameraWidth::Half, 70)).unwrap();
    assert_eq!(sof0(&frame.bytes).1, CAMERA_HALF_WIDTH as u16);
    assert_eq!(CAMERA_HALF_WIDTH * 2, CAMERA_FULL_WIDTH);
}

// ---------------------------------------------------------------------------------------
// Test 5
// ---------------------------------------------------------------------------------------

/// Test 5 (RLC9, RLC-S6, F6): geometry is validated before any conversion: YUYV needs an
/// even width; Grey and RGB24 accept odd widths and heights; dimensions outside
/// `16..=640` × `16..=480`, a data length off by one and overflowing dimensions are
/// `Geometry`.
#[test]
fn test_rlc_frame_geometry_bounds() {
    let geometry = Err(FrameError::Geometry);
    let conv = |format: u8, w: u32, h: u32, data: &[u8]| {
        convert_frame(format, w, h, data, CameraWidth::Full).map(|(_, ow, oh, k)| (ow, oh, k))
    };

    // YUYV with an odd width.
    assert_eq!(conv(YUYV, 33, 16, &vec![0; 33 * 16 * 2]), geometry);
    assert_eq!(conv(YUYV, 533, 400, &vec![0; 533 * 400 * 2]), geometry);
    // Grey and RGB24 with odd width and odd height are fine.
    assert_eq!(
        conv(GREY, 533, 401, &vec![0; 533 * 401]),
        Ok((533, 401, PixelKind::Luma))
    );
    assert_eq!(
        conv(RGB24, 533, 401, &vec![0; 533 * 401 * 3]),
        Ok((533, 401, PixelKind::Rgb))
    );
    // YUYV with an even width and an odd height is fine.
    assert_eq!(
        conv(YUYV, 534, 401, &vec![0; 534 * 401 * 2]),
        Ok((534, 401, PixelKind::Ycbcr))
    );

    // Bounds of every dimension, every format.
    let min = CAMERA_MIN_SOURCE_DIM;
    let (max_w, max_h) = (CAMERA_MAX_SOURCE_WIDTH, CAMERA_MAX_SOURCE_HEIGHT);
    for (format, bpp) in [(GREY, 1usize), (RGB24, 3), (YUYV, 2)] {
        let ok = |w: u32, h: u32| conv(format, w, h, &vec![0; w as usize * h as usize * bpp]);
        assert!(ok(min, min).is_ok(), "format {format}: minimum accepted");
        assert!(
            ok(max_w, max_h).is_ok(),
            "format {format}: maximum accepted"
        );
        assert_eq!(ok(min - 2, min), geometry, "format {format}: width 14");
        assert_eq!(ok(min, min - 1), geometry, "format {format}: height 15");
        assert_eq!(ok(max_w + 2, max_h), geometry, "format {format}: width 642");
        assert_eq!(
            ok(max_w, max_h + 1),
            geometry,
            "format {format}: height 481"
        );
        assert_eq!(ok(0, 0), geometry, "format {format}: zero");
        // Exact data length only.
        let exact = 64 * 48 * bpp;
        assert!(conv(format, 64, 48, &vec![0; exact]).is_ok());
        assert_eq!(conv(format, 64, 48, &vec![0; exact - 1]), geometry);
        assert_eq!(conv(format, 64, 48, &vec![0; exact + 1]), geometry);
        assert_eq!(conv(format, 64, 48, &[]), geometry);
        // Overflowing dimensions never wrap into a small product.
        assert_eq!(conv(format, u32::MAX, u32::MAX, &[0; 16]), geometry);
        assert_eq!(conv(format, u32::MAX, 2, &[0; 16]), geometry);
        assert_eq!(conv(format, 1 << 31, 2, &[]), geometry);
    }
    // Width 15 is below the minimum; 17x17 Grey (odd) is accepted; 641 is above the maximum.
    assert!(conv(GREY, 15, 16, &vec![0; 15 * 16]).is_err(), "width 15");
    assert!(conv(GREY, 17, 17, &vec![0; 17 * 17]).is_ok());
    assert!(
        conv(GREY, 641, 480, &vec![0; 641 * 480]).is_err(),
        "width 641"
    );

    // encode_preview_jpeg applies the same validation.
    assert!(matches!(
        encode_preview_jpeg(GREY, 64, 48, &vec![0; 64 * 48 - 1], full()),
        Err(FrameError::Geometry)
    ));
    assert!(matches!(
        encode_preview_jpeg(YUYV, 33, 16, &vec![0; 33 * 16 * 2], full()),
        Err(FrameError::Geometry)
    ));
}

// ---------------------------------------------------------------------------------------
// Test 6
// ---------------------------------------------------------------------------------------

/// Test 6 (RLC11, RLC-S8): NV12, MJPEG, unknown codes and the empty marker 255 are never
/// converted: `UnsupportedFormat`, whatever the data.
#[test]
fn test_rlc_unsupported_formats_refused() {
    let data = vec![0u8; 64 * 48 * 3];
    for format in [NV12, MJPEG, 5, 6, 100, 254, EMPTY] {
        assert_eq!(
            convert_frame(format, 64, 48, &data, CameraWidth::Full).map(|(_, w, h, k)| (w, h, k)),
            Err(FrameError::UnsupportedFormat),
            "format {format}"
        );
        assert!(
            matches!(
                encode_preview_jpeg(format, 64, 48, &data, full()),
                Err(FrameError::UnsupportedFormat)
            ),
            "format {format}"
        );
        // A plausible NV12 length (w*h*3/2) or an empty buffer changes nothing.
        assert!(matches!(
            encode_preview_jpeg(format, 64, 48, &data[..64 * 48 * 3 / 2], full()),
            Err(FrameError::UnsupportedFormat)
        ));
        assert!(matches!(
            encode_preview_jpeg(format, 64, 48, &[], full()),
            Err(FrameError::UnsupportedFormat)
        ));
    }
}

// ---------------------------------------------------------------------------------------
// Test 7
// ---------------------------------------------------------------------------------------

/// Test 7 (RLC-S6, RLC13): the JPEG sink is preallocated to `MAX_CAMERA_JPEG_BYTES`, accepts
/// writes up to its capacity, refuses one byte more with `WriteZero` and never grows; an
/// encoding that cannot fit its sink is `TooLarge` (test-capacity sink, `#[doc(hidden)]`
/// hooks `JpegSink::with_capacity_for_tests` and `encode_preview_jpeg_into`).
#[test]
fn test_rlc_jpeg_sink_never_grows() {
    let mut sink = JpegSink::new();
    assert_eq!(sink.capacity(), MAX_CAMERA_JPEG_BYTES);
    let chunk = vec![0xA5u8; 4096];
    let mut written = 0usize;
    while written + chunk.len() <= MAX_CAMERA_JPEG_BYTES {
        sink.write_all(&chunk).expect("within capacity");
        written += chunk.len();
    }
    let rest = MAX_CAMERA_JPEG_BYTES - written;
    sink.write_all(&chunk[..rest])
        .expect("exactly up to capacity");
    let err = sink
        .write_all(&[0u8])
        .expect_err("one byte beyond capacity");
    assert_eq!(err.kind(), ErrorKind::WriteZero);
    let err = sink.write(&[1u8, 2, 3]);
    assert!(
        matches!(&err, Ok(0)) || matches!(&err, Err(e) if e.kind() == ErrorKind::WriteZero),
        "a full sink accepts nothing: {err:?}"
    );
    assert_eq!(
        sink.capacity(),
        MAX_CAMERA_JPEG_BYTES,
        "capacity never grows"
    );
    let frame = sink.into_frame();
    assert_eq!(
        frame.bytes.len(),
        MAX_CAMERA_JPEG_BYTES,
        "nothing beyond capacity kept"
    );

    // A small test sink: an encoding larger than it is TooLarge, never truncated output.
    let data = noise(64 * 48 * 3, 41);
    let small = JpegSink::with_capacity_for_tests(256);
    assert_eq!(small.capacity(), 256);
    assert!(matches!(
        encode_preview_jpeg_into(RGB24, 64, 48, &data, full(), small),
        Err(FrameError::TooLarge)
    ));
    // The same frame fits the production sink and is a complete JPEG.
    let ok = encode_preview_jpeg_into(RGB24, 64, 48, &data, full(), JpegSink::new()).unwrap();
    assert!(ok.bytes.len() > 256);
    let (h, w, _) = sof0(&ok.bytes);
    assert_eq!((h, w), (48, 64));
    // The largest, noisiest accepted frame at the highest quality fits the bound.
    let worst = noise(640 * 480 * 3, 43);
    let big = encode_preview_jpeg(RGB24, 640, 480, &worst, settings(CameraWidth::Full, 85))
        .expect("worst case fits MAX_CAMERA_JPEG_BYTES");
    assert!(big.bytes.len() <= MAX_CAMERA_JPEG_BYTES);
}

// ---------------------------------------------------------------------------------------
// Test 8
// ---------------------------------------------------------------------------------------

/// Test 8 (RLC11): the configured quality reaches the encoder: the same noisy frame at 50
/// and 85 has different quantization tables and the q50 output is shorter.
#[test]
fn test_rlc_quality_changes_quantization() {
    let data = noise(64 * 48 * 3, 47);
    let q50 = encode_preview_jpeg(RGB24, 64, 48, &data, settings(CameraWidth::Full, 50)).unwrap();
    let q85 = encode_preview_jpeg(RGB24, 64, 48, &data, settings(CameraWidth::Full, 85)).unwrap();
    assert_ne!(dqt(&q50.bytes), dqt(&q85.bytes), "DQT differs with quality");
    assert!(
        q50.bytes.len() < q85.bytes.len(),
        "q50 {} < q85 {}",
        q50.bytes.len(),
        q85.bytes.len()
    );
    // Deterministic: the same input and settings give the same bytes.
    let again = encode_preview_jpeg(RGB24, 64, 48, &data, settings(CameraWidth::Full, 50)).unwrap();
    assert_eq!(&q50.bytes[..], &again.bytes[..]);
}

// ---------------------------------------------------------------------------------------
// Test 9 (proptest)
// ---------------------------------------------------------------------------------------

/// Data length strategy: either exactly the length the format and geometry require (to
/// exercise the `Ok` path) or an arbitrary bounded length.
fn data_len(format: u8, w: u32, h: u32, arbitrary: usize, exact: bool) -> usize {
    let bpp = match format {
        GREY => 1usize,
        YUYV => 2,
        RGB24 => 3,
        _ => 3,
    };
    if exact {
        (w as usize) * (h as usize) * bpp
    } else {
        arbitrary
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    /// Test 9 (RLC11, RLC-S6): `convert_frame` never panics on arbitrary format, dimensions
    /// and data length; an `Ok` output is exactly `out_w * out_h * bpp` bytes and at most
    /// `CAMERA_MAX_SCRATCH_BYTES`.
    #[test]
    fn test_rlc_convert_never_panics(
        format in prop_oneof![Just(GREY), Just(YUYV), Just(RGB24), any::<u8>()],
        w in 0u32..=700,
        h in 0u32..=700,
        arbitrary in 0usize..=(2 * 1024 * 1024),
        exact in any::<bool>(),
        half in any::<bool>(),
        fill in any::<u8>(),
    ) {
        let len = data_len(format, w, h, arbitrary, exact).min(2 * 1024 * 1024);
        let data = vec![fill; len];
        let mode = if half { CameraWidth::Half } else { CameraWidth::Full };
        if let Ok((pixels, ow, oh, kind)) = convert_frame(format, w, h, &data, mode) {
            let bpp = match kind {
                PixelKind::Luma => 1usize,
                PixelKind::Rgb | PixelKind::Ycbcr => 3,
            };
            prop_assert_eq!(pixels.len(), usize::from(ow) * usize::from(oh) * bpp);
            prop_assert!(pixels.len() <= CAMERA_MAX_SCRATCH_BYTES);
            prop_assert!(ow > 0 && oh > 0);
            prop_assert!(u32::from(ow) <= CAMERA_MAX_SOURCE_WIDTH);
            prop_assert!(u32::from(oh) <= CAMERA_MAX_SOURCE_HEIGHT);
            prop_assert!(format == GREY || format == YUYV || format == RGB24);
        }
    }
}
