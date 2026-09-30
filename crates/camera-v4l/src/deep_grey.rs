//! Deep-greyscale V4L2 formats exposed by infrared sensors (review finding CAM-13, GitHub #195).
//!
//! Many IR modules advertise only `Y8I` (interleaved 8-bit stereo), `Y10`, `Y12` or `Y16`
//! (little-endian 16-bit containers). None of them is a [`PixelFormat`], so such nodes used to
//! be dropped from enumeration. They are now captured on the wire in their native fourcc and
//! normalised to 8-bit [`PixelFormat::Grey`] before a [`crate::Frame`] is published, so the rest
//! of the pipeline (vision, PAD policy, preview) keeps a single greyscale format.

use crate::frame::PixelFormat;
use crate::v4l_impl::{fourcc_to_pixel_format, pixel_format_to_fourcc};
use v4l::FourCC;

/// Deep-greyscale capture formats normalised to 8-bit greyscale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeepGreyFormat {
    /// `Y8I `: interleaved 8-bit left/right greyscale (2 bytes per pixel, left sensor kept).
    Y8i,
    /// `Y10 `: 10-bit greyscale in a little-endian 16-bit container.
    Y10,
    /// `Y12 `: 12-bit greyscale in a little-endian 16-bit container.
    Y12,
    /// `Y16 `: 16-bit little-endian greyscale.
    Y16,
}

/// Selection order when a node exposes several deep-greyscale formats (most bits first).
pub const DEEP_GREY_PRIORITY: [DeepGreyFormat; 4] = [
    DeepGreyFormat::Y16,
    DeepGreyFormat::Y12,
    DeepGreyFormat::Y10,
    DeepGreyFormat::Y8i,
];

impl DeepGreyFormat {
    /// Maps a V4L2 fourcc to a deep-greyscale format.
    pub fn from_fourcc(fourcc: FourCC) -> Option<Self> {
        match &fourcc.repr {
            b"Y8I " => Some(Self::Y8i),
            b"Y10 " => Some(Self::Y10),
            b"Y12 " => Some(Self::Y12),
            b"Y16 " => Some(Self::Y16),
            _ => None,
        }
    }

    /// Returns the V4L2 fourcc of this format.
    pub fn fourcc(self) -> FourCC {
        match self {
            Self::Y8i => FourCC::new(b"Y8I "),
            Self::Y10 => FourCC::new(b"Y10 "),
            Self::Y12 => FourCC::new(b"Y12 "),
            Self::Y16 => FourCC::new(b"Y16 "),
        }
    }

    /// Converts one captured buffer to packed 8-bit greyscale (`width * height` bytes).
    ///
    /// Every format uses 2 bytes per pixel. `stride` is the driver's `bytesperline`; a value
    /// smaller than `2 * width` is replaced by the packed row size. Returns `None` for a zero
    /// dimension, an arithmetic overflow or a buffer shorter than the declared frame, so a
    /// truncated buffer is dropped instead of being published.
    pub fn to_grey8(self, data: &[u8], width: u32, height: u32, stride: u32) -> Option<Vec<u8>> {
        if width == 0 || height == 0 {
            return None;
        }
        let width = usize::try_from(width).ok()?;
        let height = usize::try_from(height).ok()?;
        let row_bytes = width.checked_mul(2)?;
        let stride = usize::try_from(stride).ok()?.max(row_bytes);
        let last_row_start = stride.checked_mul(height.checked_sub(1)?)?;
        let required = last_row_start.checked_add(row_bytes)?;
        if data.len() < required {
            return None;
        }

        let mut out = Vec::with_capacity(width.checked_mul(height)?);
        for row in 0..height {
            let start = stride.checked_mul(row)?;
            let line = data.get(start..start.checked_add(row_bytes)?)?;
            for &[lo, hi] in line.as_chunks::<2>().0 {
                let value = u16::from_le_bytes([lo, hi]);
                let grey = match self {
                    Self::Y8i => lo,
                    Self::Y10 => clamp_u8(value >> 2),
                    Self::Y12 => clamp_u8(value >> 4),
                    Self::Y16 => hi,
                };
                out.push(grey);
            }
        }
        Some(out)
    }
}

fn clamp_u8(value: u16) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

/// Pixel formats a node can deliver to the pipeline, in enumeration order, without duplicates.
///
/// Natively mapped formats come from [`fourcc_to_pixel_format`]; any deep-greyscale fourcc adds
/// [`PixelFormat::Grey`] (delivered after normalisation).
pub fn delivered_formats(fourccs: &[FourCC]) -> Vec<PixelFormat> {
    let mut formats: Vec<PixelFormat> = Vec::new();
    for &fourcc in fourccs {
        let delivered = fourcc_to_pixel_format(fourcc)
            .or_else(|| DeepGreyFormat::from_fourcc(fourcc).map(|_| PixelFormat::Grey));
        if let Some(format) = delivered {
            if !formats.contains(&format) {
                formats.push(format);
            }
        }
    }
    formats
}

/// Format actually requested from the driver with `VIDIOC_S_FMT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureWireFormat {
    /// The negotiated [`PixelFormat`] is streamed as-is.
    Native(PixelFormat),
    /// A deep-greyscale format is streamed and normalised to [`PixelFormat::Grey`].
    DeepGrey(DeepGreyFormat),
}

impl CaptureWireFormat {
    /// Returns the fourcc passed to `VIDIOC_S_FMT`.
    pub fn fourcc(self) -> FourCC {
        match self {
            Self::Native(format) => pixel_format_to_fourcc(format),
            Self::DeepGrey(format) => format.fourcc(),
        }
    }
}

/// Chooses the wire format for a negotiated `target` format.
///
/// A deep-greyscale format is used only when `target` is [`PixelFormat::Grey`] and the node
/// has no native 8-bit greyscale fourcc; otherwise the target is streamed natively.
pub fn select_wire_format(target: PixelFormat, fourccs: &[FourCC]) -> CaptureWireFormat {
    if target != PixelFormat::Grey {
        return CaptureWireFormat::Native(target);
    }
    let has_native_grey = fourccs
        .iter()
        .any(|&f| fourcc_to_pixel_format(f) == Some(PixelFormat::Grey));
    if has_native_grey {
        return CaptureWireFormat::Native(PixelFormat::Grey);
    }
    DEEP_GREY_PRIORITY
        .iter()
        .copied()
        .find(|deep| fourccs.contains(&deep.fourcc()))
        .map_or(
            CaptureWireFormat::Native(PixelFormat::Grey),
            CaptureWireFormat::DeepGrey,
        )
}
