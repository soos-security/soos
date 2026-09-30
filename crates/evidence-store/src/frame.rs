//! Self-describing frame metadata, bounded validation and RGB reconstruction for evidence
//! snapshots (review finding STO-08, GitHub #181).
//!
//! A raw V4L2 buffer cannot be interpreted without its width, height and pixel format. Since
//! record version 2 these travel with the pixels inside the authenticated AES-256-GCM
//! envelope, so a stored snapshot can be decoded without any out-of-band knowledge.

use crate::error::EvidenceStoreError;
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::ops::Deref;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// Current evidence record format version (self-describing frame metadata).
pub const EVIDENCE_RECORD_VERSION: u16 = 2;

/// Version assigned to records written before GitHub #181 (no version field, no metadata).
pub const LEGACY_EVIDENCE_RECORD_VERSION: u16 = 1;

/// Largest accepted frame width or height in pixels.
pub const MAX_EVIDENCE_DIMENSION: u32 = 8192;

/// Largest accepted frame payload in bytes (32 MiB).
pub const MAX_EVIDENCE_IMAGE_BYTES: usize = 32 * 1024 * 1024;

/// Largest evidence file `load_snapshot` reads (65 MiB).
///
/// Legacy records encode the payload as a CBOR array of integers (up to 2 bytes per pixel
/// byte), hence twice [`MAX_EVIDENCE_IMAGE_BYTES`] plus 1 MiB for metadata and the envelope.
pub const MAX_EVIDENCE_FILE_BYTES: u64 = 2 * (MAX_EVIDENCE_IMAGE_BYTES as u64) + 1024 * 1024;

/// Pixel layout of a stored evidence frame, serialized as its V4L2 fourcc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvidencePixelFormat {
    /// 8-bit grayscale, 1 byte per pixel (`GREY`).
    #[serde(rename = "GREY")]
    Gray8,
    /// Packed RGB 8:8:8, 3 bytes per pixel (`RGB3`).
    #[serde(rename = "RGB3")]
    Rgb24,
    /// Packed YUV 4:2:2 `Y0 U Y1 V`, 2 bytes per pixel, even width (`YUYV`).
    #[serde(rename = "YUYV")]
    Yuyv,
    /// Bi-planar YUV 4:2:0, 1.5 bytes per pixel, even width and height (`NV12`).
    #[serde(rename = "NV12")]
    Nv12,
    /// Motion-JPEG compressed frame, variable size (`MJPG`).
    #[serde(rename = "MJPG")]
    Mjpeg,
}

impl EvidencePixelFormat {
    /// Returns the V4L2 fourcc identifier of the format.
    pub const fn fourcc(self) -> &'static str {
        match self {
            Self::Gray8 => "GREY",
            Self::Rgb24 => "RGB3",
            Self::Yuyv => "YUYV",
            Self::Nv12 => "NV12",
            Self::Mjpeg => "MJPG",
        }
    }

    /// Expected payload length for `width x height`, or `None` for compressed formats and
    /// on arithmetic overflow.
    pub fn expected_len(self, width: u32, height: u32) -> Option<usize> {
        let pixels = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?;
        match self {
            Self::Gray8 => Some(pixels),
            Self::Rgb24 => pixels.checked_mul(3),
            Self::Yuyv => pixels.checked_mul(2),
            Self::Nv12 => pixels.checked_add(pixels.checked_div(2)?),
            Self::Mjpeg => None,
        }
    }
}

/// Capture metadata stored with every frame snapshot (record version 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameMetadata {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Pixel layout of the payload.
    pub pixel_format: EvidencePixelFormat,
    /// Monotonic capture timestamp of the frame in nanoseconds (`CLOCK_MONOTONIC`).
    pub captured_at_mono_ns: u64,
    /// Capture sequence number reported by the camera.
    pub sequence: u64,
}

impl FrameMetadata {
    /// Checks the dimensions against the bounds and the payload length against the format.
    ///
    /// # Errors
    ///
    /// [`EvidenceStoreError::InvalidFrame`] on zero or oversized dimensions, an odd width
    /// (YUYV, NV12) or height (NV12), an oversized or empty payload, or a payload whose
    /// length differs from `width x height` in the declared format.
    pub fn validate(&self, data_len: usize) -> Result<(), EvidenceStoreError> {
        let dims_ok = (1..=MAX_EVIDENCE_DIMENSION).contains(&self.width)
            && (1..=MAX_EVIDENCE_DIMENSION).contains(&self.height);
        if !dims_ok {
            return Err(EvidenceStoreError::InvalidFrame(format!(
                "dimensions {}x{} outside 1..={MAX_EVIDENCE_DIMENSION}",
                self.width, self.height
            )));
        }
        let odd_width = !self.width.is_multiple_of(2);
        let odd_height = !self.height.is_multiple_of(2);
        match self.pixel_format {
            EvidencePixelFormat::Yuyv if odd_width => {
                return Err(EvidenceStoreError::InvalidFrame(
                    "YUYV frames require an even width".to_string(),
                ));
            }
            EvidencePixelFormat::Nv12 if odd_width || odd_height => {
                return Err(EvidenceStoreError::InvalidFrame(
                    "NV12 frames require an even width and height".to_string(),
                ));
            }
            _ => {}
        }
        check_payload_len(data_len)?;
        match self.pixel_format.expected_len(self.width, self.height) {
            Some(expected) if expected != data_len => {
                Err(EvidenceStoreError::InvalidFrame(format!(
                    "{} {}x{} frame needs {expected} bytes, got {data_len}",
                    self.pixel_format.fourcc(),
                    self.width,
                    self.height
                )))
            }
            _ => Ok(()),
        }
    }
}

/// Rejects empty payloads and payloads above [`MAX_EVIDENCE_IMAGE_BYTES`].
pub(crate) fn check_payload_len(data_len: usize) -> Result<(), EvidenceStoreError> {
    if data_len == 0 {
        return Err(EvidenceStoreError::InvalidFrame(
            "empty frame payload".to_string(),
        ));
    }
    if data_len > MAX_EVIDENCE_IMAGE_BYTES {
        return Err(EvidenceStoreError::InvalidFrame(format!(
            "frame payload of {data_len} bytes exceeds {MAX_EVIDENCE_IMAGE_BYTES} bytes"
        )));
    }
    Ok(())
}

/// A captured frame handed to [`crate::EvidenceStore::store_frame_snapshot`].
#[derive(Clone)]
pub struct EvidenceFrame<'a> {
    /// Capture metadata describing `data`.
    pub metadata: FrameMetadata,
    /// Raw payload exactly as captured (never logged).
    pub data: &'a [u8],
}

impl fmt::Debug for EvidenceFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvidenceFrame")
            .field("metadata", &self.metadata)
            .field("data_len", &self.data.len())
            .finish()
    }
}

/// Frame payload bytes, zeroized on drop and redacted from `Debug`.
///
/// Serialized as a CBOR byte string; a CBOR array of integers (the encoding of the legacy
/// `Vec<u8>` field) is still accepted when decoding.
#[derive(Clone, Default, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct FrameBytes(Vec<u8>);

impl FrameBytes {
    /// Wraps `bytes`.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Returns the payload as a slice.
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Deref for FrameBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for FrameBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FrameBytes([REDACTED; {} bytes])", self.0.len())
    }
}

impl PartialEq<Vec<u8>> for FrameBytes {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.0 == *other
    }
}

impl PartialEq<[u8]> for FrameBytes {
    fn eq(&self, other: &[u8]) -> bool {
        self.0 == other
    }
}

impl PartialEq<&[u8]> for FrameBytes {
    fn eq(&self, other: &&[u8]) -> bool {
        self.0 == *other
    }
}

impl<const N: usize> PartialEq<[u8; N]> for FrameBytes {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.0 == other
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for FrameBytes {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self.0 == *other
    }
}

impl Serialize for FrameBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

struct FrameBytesVisitor;

impl<'de> Visitor<'de> for FrameBytesVisitor {
    type Value = FrameBytes;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a byte string or byte array of at most {MAX_EVIDENCE_IMAGE_BYTES} bytes"
        )
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<FrameBytes, E> {
        if v.len() > MAX_EVIDENCE_IMAGE_BYTES {
            return Err(E::custom("frame payload exceeds the evidence bound"));
        }
        Ok(FrameBytes(v.to_vec()))
    }

    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<FrameBytes, E> {
        let bytes = FrameBytes(v);
        if bytes.0.len() > MAX_EVIDENCE_IMAGE_BYTES {
            return Err(E::custom("frame payload exceeds the evidence bound"));
        }
        Ok(bytes)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<FrameBytes, A::Error> {
        let hint = seq.size_hint().unwrap_or(0).min(MAX_EVIDENCE_IMAGE_BYTES);
        let mut bytes = FrameBytes(Vec::with_capacity(hint));
        while let Some(byte) = seq.next_element::<u8>()? {
            if bytes.0.len() >= MAX_EVIDENCE_IMAGE_BYTES {
                return Err(de::Error::custom(
                    "frame payload exceeds the evidence bound",
                ));
            }
            bytes.0.push(byte);
        }
        Ok(bytes)
    }
}

impl<'de> Deserialize<'de> for FrameBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_byte_buf(FrameBytesVisitor)
    }
}

/// Converts a validated frame payload to packed RGB 8:8:8.
///
/// # Errors
///
/// [`EvidenceStoreError::InvalidFrame`] for compressed formats (MJPEG is stored verbatim and
/// must be decoded by an external tool) and for payloads inconsistent with `meta`.
pub(crate) fn to_rgb24(
    meta: &FrameMetadata,
    data: &[u8],
) -> Result<Zeroizing<Vec<u8>>, EvidenceStoreError> {
    meta.validate(data.len())?;
    let pixels = EvidencePixelFormat::Rgb24
        .expected_len(meta.width, meta.height)
        .ok_or_else(|| EvidenceStoreError::InvalidFrame("frame size overflow".to_string()))?;
    let mut rgb = Zeroizing::new(Vec::with_capacity(pixels));
    match meta.pixel_format {
        EvidencePixelFormat::Rgb24 => rgb.extend_from_slice(data),
        EvidencePixelFormat::Gray8 => {
            for &luma in data {
                rgb.extend_from_slice(&[luma, luma, luma]);
            }
        }
        EvidencePixelFormat::Yuyv => {
            let (quads, _) = data.as_chunks::<4>();
            for &[y0, u, y1, v] in quads {
                rgb.extend_from_slice(&yuv_to_rgb(y0, u, v));
                rgb.extend_from_slice(&yuv_to_rgb(y1, u, v));
            }
        }
        EvidencePixelFormat::Nv12 => nv12_to_rgb(meta, data, &mut rgb)?,
        EvidencePixelFormat::Mjpeg => {
            return Err(EvidenceStoreError::InvalidFrame(
                "MJPEG payloads are stored verbatim and are not decoded here".to_string(),
            ));
        }
    }
    Ok(rgb)
}

fn nv12_to_rgb(
    meta: &FrameMetadata,
    data: &[u8],
    rgb: &mut Vec<u8>,
) -> Result<(), EvidenceStoreError> {
    let overflow = || EvidenceStoreError::InvalidFrame("NV12 plane overflow".to_string());
    let width = usize::try_from(meta.width).map_err(|_| overflow())?;
    let luma_len = EvidencePixelFormat::Gray8
        .expected_len(meta.width, meta.height)
        .ok_or_else(overflow)?;
    let (luma, chroma) = data.split_at_checked(luma_len).ok_or_else(overflow)?;
    for (row_index, row) in luma.chunks_exact(width).enumerate() {
        let chroma_row_start = (row_index / 2).checked_mul(width).ok_or_else(overflow)?;
        let chroma_row = chroma.get(chroma_row_start..).ok_or_else(overflow)?;
        let (pairs, _) = row.as_chunks::<2>();
        let (uvs, _) = chroma_row.as_chunks::<2>();
        for (&[y0, y1], &[u, v]) in pairs.iter().zip(uvs) {
            rgb.extend_from_slice(&yuv_to_rgb(y0, u, v));
            rgb.extend_from_slice(&yuv_to_rgb(y1, u, v));
        }
    }
    Ok(())
}

/// ITU-R BT.601 YUV to RGB conversion in 8.8 fixed point.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "8-bit inputs widened to i32 keep every intermediate far below i32::MAX"
)]
fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let y = i32::from(y);
    let u = i32::from(u) - 128;
    let v = i32::from(v) - 128;
    let clamp = |c: i32| u8::try_from(c.clamp(0, 255)).unwrap_or(u8::MAX);
    [
        clamp(y + ((359 * v) >> 8)),
        clamp(y - ((88 * u + 183 * v) >> 8)),
        clamp(y + ((454 * u) >> 8)),
    ]
}
